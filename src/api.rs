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
use crate::data::alpaca::AlpacaClient;
use crate::llm::LLMQueue;
use crate::services::websocket_service::WebSocketService;
use tracing::{info, error};
use std::time::Duration;

use crate::config::AppConfig;

pub struct AppState {
    pub trading_handle: Mutex<Option<JoinHandle<()>>>,
    pub alpaca: AlpacaClient,
    pub llm: LLMQueue,
    pub config: AppConfig,
}

pub async fn run_server(state: Arc<AppState>) {
    let app = Router::new()
        .route("/start", post(start_trading))
        .route("/stop", post(stop_trading))
        .route("/assets", get(get_assets))
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
    State(state): State<Arc<AppState>>,
    Query(params): Query<AssetParams>
) -> impl IntoResponse {
    match state.alpaca.get_assets(params.class).await {
        Ok(assets) => Json(assets).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Error fetching assets: {}", e),
        ).into_response(),
    }
}

async fn start_trading(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut handle_lock = state.trading_handle.lock().unwrap();
    
    if handle_lock.is_some() {
        return Json(json!({"status": "already_running"})).into_response();
    }

    let alpaca = state.alpaca.clone();
    let llm = state.llm.clone(); 
    let config = state.config.clone();
    
    let handle = tokio::spawn(async move {
        // Start Streaming
        let trading_mode = config.trading_mode.clone();
        let is_crypto = trading_mode.to_lowercase() == "crypto";
        info!("🔧 Trading Mode: {} (Crypto: {})", trading_mode, is_crypto);
        
        let symbols = config.symbols.clone();

        // Create Event Bus
        let event_bus = crate::bus::EventBus::new(1000);

        info!("Initializing Streaming via WebSocketService for {:?} (Crypto: {})", symbols, is_crypto);
        let ws_service = WebSocketService::new(
            alpaca.market_store.clone(),
            symbols.clone(),
            is_crypto,
            event_bus.clone()
        );
        ws_service.start().await;

        info!("Initializing EDA Services...");

        // Start Strategy Engine
        let strategy_engine = crate::services::strategy::StrategyEngine::new(
            event_bus.clone(),
            alpaca.market_store.clone(),
            llm.clone(),
            config.clone(),
        );
        strategy_engine.start().await;

        // Start Risk Engine
        let risk_engine = crate::services::risk::RiskEngine::new(
            event_bus.clone(),
            alpaca.clone(),
            llm.clone(),
            config.clone(),
        );
        risk_engine.start().await;

        // Start Execution Engine
        let execution_engine = crate::services::execution::ExecutionEngine::new(
            event_bus.clone(),
            alpaca.clone(),
            llm.clone(),
            config.clone(),
        );
        execution_engine.start().await;

        info!("🚀 All EDA Services Started. Trading System Active.");
        
        // Spawn Monitor Loop (Account Balance)
        let alpaca_monitor = alpaca.clone();
        tokio::spawn(async move {
             monitor_loop(alpaca_monitor).await;
        });

        // Keep task alive (prevents immediate exit)
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

async fn monitor_loop(alpaca: AlpacaClient) {
    loop {
        // Fetch Account
        if let Ok(account) = alpaca.get_account().await {
             // Fetch Positions
             let positions = alpaca.get_positions().await.unwrap_or_default();
             
             // Log Account & Holdings Summary
             info!("\n💰 Account Summary\n------------------\nCash: ${}\nPortfolio: ${}\n", account.cash, account.portfolio_value);
             
             if positions.is_empty() {
                  info!("🎒 Current Holdings: None");
             } else {
                  let mut holdings_log = String::from("\n🎒 Current Holdings\n-------------------\n");
                  for p in positions {
                      let symbol = p.get("symbol").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
                      let qty = p.get("qty").and_then(|v| v.as_str()).unwrap_or("0");
                      let price = p.get("current_price").and_then(|v| v.as_str()).unwrap_or("0.00");
                      let pl = p.get("unrealized_pl").and_then(|v| v.as_str()).unwrap_or("0.00");
                      holdings_log.push_str(&format!("- {}: {} shares @ ${} (P/L: ${})\n", symbol, qty, price, pl));
                  }
                  info!("{}", holdings_log);
             }
        } else {
            error!("Monitor: Failed to fetch account.");
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

