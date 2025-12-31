use axum::{
    routing::{get, post},
    Router,
    extract::State,
    Json,
    response::IntoResponse,
};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use serde_json::json;
use crate::llm::LLMQueue;
use tracing::{info, error};

use crate::config::AppConfig;
use crate::exchange::{factory::build_exchange, ws::GenericWsStream};
use crate::exchange::ws::WsProvider;
use crate::exchange::traits::{TradingApi, MarketDataStream};
use crate::data::store::MarketStore;
use crate::services::reporting::TradeReporter;

pub struct AppState {
    pub trading_handle: Mutex<Option<JoinHandle<()>>>,
    pub exchange: Mutex<Option<Arc<dyn TradingApi>>>,
    pub llm: LLMQueue,
    pub config: AppConfig,
}

pub async fn run_server(state: Arc<AppState>) {
    let app = Router::new()
        .route("/start", post(start_trading))
        .route("/stop", post(stop_trading))
        .route("/assets", get(get_assets))
        .route("/report", get(get_report))
        .route("/cancel_all", post(cancel_all_orders))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("API Server listening on port 3000");
    axum::serve(listener, app).await.unwrap();
}

use axum::extract::Query;

#[derive(serde::Deserialize)]
struct AssetParams {
    class: Option<String>,
}

async fn get_assets(
    State(_state): State<Arc<AppState>>,
    Query(_params): Query<AssetParams>
) -> impl IntoResponse {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Assets endpoint is exchange-specific; implement via TradingApi extension per exchange.",
    ).into_response()
}

async fn get_report(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read the on-disk summary (best-effort) to avoid storing reporter in AppState.
    let path = std::path::PathBuf::from("./data/trade_summary.json");
    match std::fs::read_to_string(&path) {
        Ok(txt) => (axum::http::StatusCode::OK, txt).into_response(),
        Err(_) => (
            axum::http::StatusCode::NOT_FOUND,
            "No report found yet. Start trading first.",
        ).into_response(),
    }
}

async fn start_trading(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut handle_lock = state.trading_handle.lock().unwrap();

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
            },
            "coinbase" => {
                let (key, secret) = if let Some(c) = &config.coinbase {
                    (Some(c.api_key.clone()), Some(c.secret_key.clone()))
                } else {
                    (None, None)
                };
                GenericWsStream::coinbase(key, secret)
            },
            "kraken" => {
                let (key, secret) = if let Some(c) = &config.kraken {
                    (Some(c.api_key.clone()), Some(c.secret_key.clone()))
                } else {
                    (None, None)
                };
                GenericWsStream::kraken(key, secret)
            },
            _ => GenericWsStream { provider: WsProvider::AlpacaCrypto, api_key: None, api_secret: None },
        };

        if let Err(e) = ws_provider.start(market_store.clone(), symbols.clone(), event_bus.clone()).await {
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

        // Start Execution Engine
        let execution_engine = crate::services::execution::ExecutionEngine::new(
            event_bus.clone(),
            exchange.clone(),
            market_store.clone(),
            llm.clone(),
            config.clone(),
            position_tracker.clone(),
        );
        execution_engine.start().await;

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
    
    if let Some(handle) = handle_lock.take() {
        handle.abort();
        Json(json!({"status": "stopped"})).into_response()
    } else {
        Json(json!({"status": "not_running"})).into_response()
    }
}

async fn cancel_all_orders(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let exchange = {
        let exchange_lock = state.exchange.lock().unwrap();
        exchange_lock.clone()
    };

    if let Some(exchange) = exchange {
        match exchange.cancel_all_orders().await {
            Ok(_) => Json(json!({"status": "success", "message": "All orders cancelled"})).into_response(),
            Err(e) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to cancel all orders: {}", e),
            ).into_response(),
        }
    } else {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "Exchange not initialized. Start trading first.",
        ).into_response()
    }
}
