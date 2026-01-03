use crate::llm::LLMQueue;
use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::config::AppConfig;
use crate::data::store::MarketStore;
use crate::exchange::traits::{MarketDataStream, TradingApi};
use crate::exchange::ws::WsProvider;
use crate::exchange::{factory::build_exchange, ws::GenericWsStream};
use crate::services::reporting::TradeReporter;

pub struct AppState {
    pub trading_handle: Mutex<Option<JoinHandle<()>>>,
    pub websocket_handle: Mutex<Option<JoinHandle<()>>>,
    pub exchange: Mutex<Option<Arc<dyn TradingApi>>>,
    pub llm: LLMQueue,
    pub config: AppConfig,
}

pub async fn run_server(state: Arc<AppState>) {
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/start", post(start_trading))
        .route("/stop", post(stop_trading))
        .route("/assets", get(get_assets))
        .route("/report", get(get_report))
        .route("/stats", get(get_stats))
        .route("/cancel_all", post(cancel_all_orders))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("API Server listening on port 3000");
    axum::serve(listener, app).await.unwrap();
}

// Lightweight health check endpoint for keep-alive
async fn health_check() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "rust-autohedge"
    }))
}
use axum::extract::Query;

#[derive(serde::Deserialize)]
struct AssetParams {
    class: Option<String>,
}

async fn get_assets(
    State(_state): State<Arc<AppState>>,
    Query(_params): Query<AssetParams>,
) -> impl IntoResponse {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Assets endpoint is exchange-specific; implement via TradingApi extension per exchange.",
    )
        .into_response()
}

async fn get_report(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read the on-disk summary (best-effort) to avoid storing reporter in AppState.
    let path = std::path::PathBuf::from("./data/trade_summary.json");
    match std::fs::read_to_string(&path) {
        Ok(txt) => (axum::http::StatusCode::OK, txt).into_response(),
        Err(_) => (
            axum::http::StatusCode::NOT_FOUND,
            "No report found yet. Start trading first.",
        )
            .into_response(),
    }
}

async fn get_stats(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read the computed stats (smaller, easier to read)
    let path = std::path::PathBuf::from("./data/trade_stats.json");
    match std::fs::read_to_string(&path) {
        Ok(txt) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            txt,
        )
            .into_response(),
        Err(_) => (
            axum::http::StatusCode::NOT_FOUND,
            "No stats found yet. Start trading first.",
        )
            .into_response(),
    }
}

async fn start_trading(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut handle_lock = state.trading_handle.lock().unwrap();
    let ws_handle_lock = state.websocket_handle.lock().unwrap();

    if handle_lock.is_some() {
        return Json(json!({"status": "already_running"})).into_response();
    }

    let llm = state.llm.clone();
    let config = state.config.clone();

    // Build exchange synchronously and store in state
    let (exchange, maybe_store) = build_exchange(&config);
    {
        let mut exchange_lock = state.exchange.lock().unwrap();
        *exchange_lock = Some(exchange.clone());
    }

    let handle = tokio::spawn(async move {
        let trading_mode = config.trading_mode.clone();
        let is_crypto = trading_mode.to_lowercase() == "crypto";
        info!("🔧 Trading Mode: {} (Crypto: {})", trading_mode, is_crypto);

        let symbols = config.symbols.clone();

        // Create Event Bus
        let event_bus = crate::bus::EventBus::new(1000);

        // Market store: if exchange doesn't provide one, make a local one.
        let market_store = maybe_store.unwrap_or_else(|| MarketStore::new(config.history_limit));

        // Start Streaming (provider-specific WS)
        let ws_provider = match exchange.name() {
            "alpaca" => {
                let api_key = config.alpaca.api_key.clone();
                let secret = config.alpaca.secret_key.clone();
                GenericWsStream::alpaca(api_key, secret, is_crypto)
            }
            "binance" => {
                let (key, secret) = if let Some(c) = &config.binance {
                    (Some(c.api_key.clone()), Some(c.secret_key.clone()))
                } else {
                    (None, None)
                };
                GenericWsStream::binance(key, secret)
            }
            "coinbase" => {
                let (key, secret) = if let Some(c) = &config.coinbase {
                    (Some(c.api_key.clone()), Some(c.secret_key.clone()))
                } else {
                    (None, None)
                };
                GenericWsStream::coinbase(key, secret)
            }
            "kraken" => {
                let (key, secret) = if let Some(c) = &config.kraken {
                    (Some(c.api_key.clone()), Some(c.secret_key.clone()))
                } else {
                    (None, None)
                };
                GenericWsStream::kraken(key, secret)
            }
            _ => GenericWsStream {
                provider: WsProvider::AlpacaCrypto,
                api_key: None,
                api_secret: None,
            },
        };

        if let Err(e) = ws_provider
            .start(market_store.clone(), symbols.clone(), event_bus.clone())
            .await
        {
            error!("WS start failed: {}", e);
        }

        info!("Initializing EDA Services...");

        // Start Trade Reporter (writes JSONL + summary under ./data)
        let reporter = TradeReporter::new(std::path::PathBuf::from("./data/trades.jsonl"));
        reporter.start(event_bus.clone()).await;

        // Create Position Tracker (shared between Execution and Monitor)
        let position_tracker = crate::services::position_monitor::PositionTracker::new();

        // Start Strategy Engine
        let strategy_engine = crate::services::strategy::StrategyEngine::new(
            event_bus.clone(),
            market_store.clone(),
            llm.clone(),
            config.clone(),
        );
        strategy_engine.start().await;

        // Start Risk Engine
        let risk_engine = crate::services::risk::RiskEngine::new(
            event_bus.clone(),
            exchange.clone(),
            llm.clone(),
            config.clone(),
        );
        risk_engine.start().await;

        // Start Execution Engine (use fast engine for HFT mode)
        if config.strategy_mode.to_lowercase() == "hft" {
            info!("⚡ Using Fast Execution Engine for HFT mode");
            let execution_engine = crate::services::execution_fast::ExecutionEngine::new(
                event_bus.clone(),
                exchange.clone(),
                market_store.clone(),
                llm.clone(),
                config.clone(),
                position_tracker.clone(),
            );
            execution_engine.start().await;
        } else {
            let execution_engine = crate::services::execution::ExecutionEngine::new(
                event_bus.clone(),
                exchange.clone(),
                market_store.clone(),
                llm.clone(),
                config.clone(),
                position_tracker.clone(),
            );
            execution_engine.start().await;
        }

        // Start Position Monitor
        let position_monitor = crate::services::position_monitor::PositionMonitor::new(
            event_bus.clone(),
            exchange.clone(),
            position_tracker.clone(),
            config.clone(),
        );
        position_monitor.start().await;

        info!("🚀 All EDA Services Started. Trading System Active.");

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });

    *handle_lock = Some(handle);

    Json(json!({"status": "started"})).into_response()
}

async fn stop_trading(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut handle_lock = state.trading_handle.lock().unwrap();
    let mut ws_handle_lock = state.websocket_handle.lock().unwrap();

    let mut stopped_something = false;

    // Abort the main trading task (which contains all the spawned services including WS)
    if let Some(handle) = handle_lock.take() {
        info!("Aborting trading task...");
        handle.abort();
        stopped_something = true;
    }

    // Abort WebSocket handle if it exists separately
    if let Some(ws_handle) = ws_handle_lock.take() {
        info!("Aborting WebSocket task...");
        ws_handle.abort();
        stopped_something = true;
    }

    // Clear exchange from state
    {
        let mut exchange_lock = state.exchange.lock().unwrap();
        if exchange_lock.take().is_some() {
            info!("Cleared exchange from state");
        }
    }

    if stopped_something {
        info!("✅ Trading system stopped successfully");
        Json(json!({"status": "stopped"})).into_response()
    } else {
        Json(json!({"status": "not_running"})).into_response()
    }
}

async fn cancel_all_orders(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Attempt to get the exchange from state, or build a temporary one if not initialized
    let exchange = {
        let exchange_lock = state.exchange.lock().unwrap();
        if let Some(ex) = exchange_lock.clone() {
            ex
        } else {
            info!("Exchange not initialized in state, building temporary instance for cancellation...");
            let (ex, _) = build_exchange(&state.config);
            ex
        }
    };

    match exchange.cancel_all_orders().await {
        Ok(_) => {
            Json(json!({"status": "success", "message": "All orders cancelled"})).into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to cancel all orders: {}", e),
        )
            .into_response(),
    }
}
