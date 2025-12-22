use tracing::{info, error, warn};
use crate::bus::EventBus;
use crate::events::{Event, AnalysisSignal};
use crate::data::alpaca::AlpacaClient;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use rand::Rng;

#[derive(Clone, Debug)]
pub struct PositionInfo {
    pub symbol: String,
    pub entry_price: f64,
    pub qty: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub entry_time: String,
    pub side: String, // "buy" or "sell"
}

#[derive(Clone)]
pub struct PositionTracker {
    positions: Arc<Mutex<HashMap<String, PositionInfo>>>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn add_position(&self, info: PositionInfo) {
        let mut positions = self.positions.lock().unwrap();
        info!("📊 [TRACKER] Added position: {} @ ${:.8} (SL: ${:.8}, TP: ${:.8})",
              info.symbol, info.entry_price, info.stop_loss, info.take_profit);
        positions.insert(info.symbol.clone(), info);
    }

    pub fn remove_position(&self, symbol: &str) -> Option<PositionInfo> {
        let mut positions = self.positions.lock().unwrap();
        let removed = positions.remove(symbol);
        if removed.is_some() {
            info!("📊 [TRACKER] Removed position: {}", symbol);
        }
        removed
    }

    pub fn get_position(&self, symbol: &str) -> Option<PositionInfo> {
        let positions = self.positions.lock().unwrap();
        positions.get(symbol).cloned()
    }

    pub fn get_all_positions(&self) -> Vec<PositionInfo> {
        let positions = self.positions.lock().unwrap();
        positions.values().cloned().collect()
    }

    pub fn has_position(&self, symbol: &str) -> bool {
        let positions = self.positions.lock().unwrap();
        positions.contains_key(symbol)
    }
}

pub struct PositionMonitor {
    event_bus: EventBus,
    alpaca: AlpacaClient,
    tracker: PositionTracker,
    check_interval_secs: u64,
}

impl PositionMonitor {
    pub fn new(event_bus: EventBus, alpaca: AlpacaClient, tracker: PositionTracker) -> Self {
        Self {
            event_bus,
            alpaca,
            tracker,
            check_interval_secs: 10, // Check every 10 seconds
        }
    }

    pub async fn start(&self) {
        let bus = self.event_bus.clone();
        let alpaca = self.alpaca.clone();
        let tracker = self.tracker.clone();
        let interval = self.check_interval_secs;

        tokio::spawn(async move {
            info!("👁️  Position Monitor Started (checking every {}s)", interval);

            // Initial sync with Alpaca positions
            Self::sync_positions(&alpaca, &tracker).await;

            loop {
                sleep(Duration::from_secs(interval)).await;

                let tracked_positions = tracker.get_all_positions();
                if tracked_positions.is_empty() {
                    continue;
                }

                // Check each tracked position
                for position in tracked_positions {
                    match Self::check_position(&position, &alpaca, &tracker, &bus).await {
                        Ok(should_exit) => {
                            if should_exit {
                                // Position was closed, remove from tracker
                                tracker.remove_position(&position.symbol);
                            }
                        }
                        Err(e) => {
                            error!("❌ [MONITOR] Error checking {}: {}", position.symbol, e);
                        }
                    }
                }
            }
        });
    }

    async fn sync_positions(alpaca: &AlpacaClient, tracker: &PositionTracker) {
        info!("🔄 [MONITOR] Syncing positions with Alpaca...");

        match alpaca.get_positions().await {
            Ok(positions) => {
                for pos in positions {
                    let symbol = pos.get("symbol").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if symbol.is_empty() || tracker.has_position(&symbol) {
                        continue;
                    }

                    // Position exists in Alpaca but not tracked - add it with defaults
                    let avg_entry = pos.get("avg_entry_price")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    let qty = pos.get("qty")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);

                    if avg_entry > 0.0 {
                        // Create position info with default stop/take profit (5% and 10%)
                        let stop_loss = avg_entry * 0.95;
                        let take_profit = avg_entry * 1.10;

                        let info = PositionInfo {
                            symbol: symbol.clone(),
                            entry_price: avg_entry,
                            qty,
                            stop_loss,
                            take_profit,
                            entry_time: chrono::Utc::now().to_rfc3339(),
                            side: "buy".to_string(),
                        };

                        tracker.add_position(info);
                        warn!("⚠️  [MONITOR] Added existing position {} (defaults: SL -5%, TP +10%)", symbol);
                    }
                }
                info!("✅ [MONITOR] Position sync complete");
            }
            Err(e) => {
                error!("❌ [MONITOR] Failed to sync positions: {}", e);
            }
        }
    }

    async fn check_position(
        position: &PositionInfo,
        alpaca: &AlpacaClient,
        _tracker: &PositionTracker,
        bus: &EventBus,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // Get current price from market store
        let history = alpaca.market_store.get_quote_history(&position.symbol);

        if history.is_empty() {
            return Ok(false);
        }

        let latest_quote = history.last().unwrap();
        let current_price = latest_quote.get("bp").and_then(|v| v.as_f64()).unwrap_or(0.0);

        if current_price == 0.0 {
            return Ok(false);
        }

        // Calculate P/L
        let pl_pct = ((current_price - position.entry_price) / position.entry_price) * 100.0;

        // Check if position still exists in Alpaca
        match alpaca.get_positions().await {
            Ok(positions) => {
                let still_open = positions.iter().any(|p| {
                    p.get("symbol").and_then(|v| v.as_str()) == Some(&position.symbol)
                });

                if !still_open {
                    info!("📉 [MONITOR] Position {} no longer exists in Alpaca. Removing from tracker.", position.symbol);
                    return Ok(true); // Signal to remove from tracker
                }
            }
            Err(_) => {
                // Continue checking even if API call fails
            }
        }

        // Check Take Profit
        if current_price >= position.take_profit {
            info!("🎯 [MONITOR] TAKE PROFIT HIT for {}!", position.symbol);
            info!("   Entry: ${:.8} → Current: ${:.8} (P/L: +{:.2}%)",
                  position.entry_price, current_price, pl_pct);

            Self::generate_exit_signal(position, "take_profit", current_price, bus).await;
            return Ok(true); // Position will be closed
        }

        // Check Stop Loss
        if current_price <= position.stop_loss {
            warn!("🛑 [MONITOR] STOP LOSS HIT for {}!", position.symbol);
            warn!("   Entry: ${:.8} → Current: ${:.8} (P/L: {:.2}%)",
                  position.entry_price, current_price, pl_pct);

            Self::generate_exit_signal(position, "stop_loss", current_price, bus).await;
            return Ok(true); // Position will be closed
        }

        // Log periodic status (every 10 checks = ~100 seconds)
        let mut rng = rand::thread_rng();
        if rng.gen_range(0..10) == 0 {
            info!("📊 [MONITOR] {} @ ${:.8} (P/L: {:.2}%, SL: ${:.8}, TP: ${:.8})",
                  position.symbol, current_price, pl_pct, position.stop_loss, position.take_profit);
        }

        Ok(false) // Continue monitoring
    }

    async fn generate_exit_signal(
        position: &PositionInfo,
        reason: &str,
        current_price: f64,
        bus: &EventBus,
    ) {
        let pl_pct = ((current_price - position.entry_price) / position.entry_price) * 100.0;

        let thesis = format!(
            "Exit signal for {} due to {}. Entry: ${:.8}, Current: ${:.8}, P/L: {:.2}%",
            position.symbol, reason, position.entry_price, current_price, pl_pct
        );

        let signal = AnalysisSignal {
            symbol: position.symbol.clone(),
            signal: "sell".to_string(),
            confidence: 1.0, // High confidence - triggered by rule
            thesis,
            market_context: format!("Reason: {}", reason),
        };

        match bus.publish(Event::Signal(signal)) {
            Ok(_) => {
                info!("✅ [MONITOR] Exit signal published for {}", position.symbol);
            }
            Err(e) => {
                error!("❌ [MONITOR] Failed to publish exit signal: {}", e);
            }
        }
    }
}

