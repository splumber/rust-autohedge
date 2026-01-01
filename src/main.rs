mod agents;
mod api;
mod bus;
mod config;
mod data;
mod events;
mod exchange;
mod llm;
pub mod services;

use api::{run_server, AppState};
use config::AppConfig;
use llm::{LLMClient, LLMQueue};
use std::sync::{Arc, Mutex};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Setup Logging
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    info!("Starting AutoHedge Rust...");

    // Load Configuration
    let config = AppConfig::load();
    info!("Loaded Configuration: {:?}", config);

    // Initialize Clients
    info!("Initializing AI Clients...");
    let api_key = config.llm.api_key.clone().unwrap_or_default();
    let base_url = config.llm.base_url.clone();
    if let Some(url) = &base_url {
        info!("Using Custom OpenAI Base URL: {}", url);
    }

    let model = config.llm.model.clone();
    info!("Using LLM Model: {}", model);

    let llm_client = LLMClient::new(api_key, base_url, model);

    // Create LLM Queue with max concurrent requests from config
    info!(
        "📬 Initializing LLM Queue (max concurrent: {}, size: {})...",
        config.llm_max_concurrent, config.llm_queue_size
    );
    let llm_queue = LLMQueue::new(llm_client, config.llm_max_concurrent, config.llm_queue_size);

    // Create App State
    let app_state = Arc::new(AppState {
        trading_handle: Mutex::new(None),
        exchange: Mutex::new(None),
        llm: llm_queue,
        config,
    });

    // Start API Server
    info!("Initializing API Server...");
    run_server(app_state).await;

    Ok(())
}
