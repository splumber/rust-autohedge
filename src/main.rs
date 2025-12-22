mod agents;
mod data;
mod llm;
mod api;
mod config;
mod events;
mod bus;
pub mod services;


use config::AppConfig;
use data::alpaca::AlpacaClient;
use llm::{LLMClient, LLMQueue};
use std::env;
use std::sync::{Arc, Mutex};
use tracing::{info, error};
use api::{run_server, AppState};



#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Setup Logging
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    info!("Starting AutoHedge Rust...");

    // Load .env
    match dotenvy::dotenv() {
        Ok(path) => info!("Loaded .env file from: {:?}", path),
        Err(e) => info!("Could not load .env file: {} (Using system env vars or defaults)", e),
    }

    // Verify critical env vars
    let critical_vars = ["OPENAI_API_KEY", "APCA_API_KEY_ID", "APCA_API_SECRET_KEY"];
    for var in critical_vars {
        if env::var(var).is_err() {
            error!("CRITICAL: Environment variable {} is NOT set.", var);
        } else {
            info!("Environment variable {} is present.", var);
        }
    }

    // Load Configuration
    let config = AppConfig::from_env();
    info!("Loaded Configuration: {:?}", config);

    // Initialize Clients
    info!("Initializing AI Clients...");
    let api_key = env::var("OPENAI_API_KEY").unwrap_or_default();
    let base_url = env::var("OPENAI_BASE_URL").ok();
    if let Some(url) = &base_url {
        info!("Using Custom OpenAI Base URL: {}", url);
    }
    
    let model = env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4-turbo-preview".to_string());
    info!("Using LLM Model: {}", model);
    
    let llm_client = LLMClient::new(api_key, base_url, model);
    
    // Create LLM Queue with max concurrent requests from config
    info!("📬 Initializing LLM Queue (max concurrent: {}, size: {})...", config.llm_max_concurrent, config.llm_queue_size);
    let llm_queue = LLMQueue::new(llm_client, config.llm_max_concurrent, config.llm_queue_size);

    info!("Initializing Alpaca Client...");
    let alpaca_client = AlpacaClient::new(config.history_limit);

    // Create App State
    let app_state = Arc::new(AppState {
        trading_handle: Mutex::new(None),
        alpaca: alpaca_client,
        llm: llm_queue,
        config,
    });

    // Start API Server
    info!("Initializing API Server...");
    run_server(app_state).await;

    Ok(())
}
