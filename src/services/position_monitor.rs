use tracing::{info, error, warn};
use crate::bus::EventBus;
use crate::events::{Event, AnalysisSignal, MarketEvent};
use crate::config::AppConfig;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use crate::exchange::traits::TradingApi;

#[derive(Clone, Debug)]
pub struct PositionInfo {
    pub symbol: String,
    pub entry_price: f64,
    pub qty: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub entry_time: String,
    pub side: String, // "buy" or "sell"
    pub is_closing: bool, // New field to prevent double-sells
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

    pub fn add_position(&self, mut info: PositionInfo) {
        let mut positions = self.positions.lock().unwrap();
        // Ensure is_closing is false initially
        info.is_closing = false;
        info!("📊 [TRACKER] Added position: {} @ ${:.8} (SL: ${:.8}, TP: ${:.8})",
              info.symbol, info.entry_price, info.stop_loss, info.take_profit);
        positions.insert(info.symbol.clone(), info);
    }

    pub fn mark_closing(&self, symbol: &str) {
        let mut positions = self.positions.lock().unwrap();
        if let Some(pos) = positions.get_mut(symbol) {
            pos.is_closing = true;
            info!("📊 [TRACKER] Marked position {} as closing", symbol);
        }
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

    /// Best-effort helper used by execution sizing when MarketStore isn't directly available.
    pub fn get_quote_history(&self, _symbol: &str) -> Vec<serde_json::Value> {
        // PositionTracker doesn't own market data; this is overridden at call sites that have store.
        // Returning empty keeps behavior safe.
        vec![]
    }

    pub fn get_last_bid(&self, _symbol: &str) -> Option<f64> {
        None
    }
}

pub struct PositionMonitor {
    event_bus: EventBus,
    exchange: Arc<dyn TradingApi>,
    tracker: PositionTracker,
    check_interval_secs: u64,
    config: AppConfig,
}

impl PositionMonitor {
    pub fn new(event_bus: EventBus, exchange: Arc<dyn TradingApi>, tracker: PositionTracker, config: AppConfig) -> Self {
        Self {
            event_bus,
            exchange,
            tracker,
            check_interval_secs: 10,
            config,
        }
    }

    pub async fn start(&self) {
        if self.config.exit_on_quotes {
            self.start_quote_driven().await;
        } else {
            self.start_polling().await;
        }
    }

    async fn start_polling(&self) {
        let bus = self.event_bus.clone();
        let exchange = self.exchange.clone();
        let tracker = self.tracker.clone();
        let interval = self.check_interval_secs;

        tokio::spawn(async move {
            info!("👁️  Position Monitor Started (polling every {}s)", interval);

            // Initial sync with exchange positions
            Self::sync_positions(&*exchange, &tracker).await;

            loop {
                sleep(Duration::from_secs(interval)).await;

                let tracked_positions = tracker.get_all_positions();
                if tracked_positions.is_empty() {
                    continue;
                }

                // Check each tracked position
                for position in tracked_positions {
                    match Self::check_position(&position, &tracker, &bus).await {
                        Ok(should_exit) => {
                            if should_exit {
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

    async fn start_quote_driven(&self) {
        let bus = self.event_bus.clone();
        let exchange = self.exchange.clone();
        let tracker = self.tracker.clone();
        let mut rx = self.event_bus.subscribe();
        let config = self.config.clone();

        tokio::spawn(async move {
            info!("👁️  Position Monitor Started (quote-driven exits) | chatter={}", config.chatter_level);

            // Initial sync with exchange positions
            Self::sync_positions(&*exchange, &tracker).await;

            while let Ok(event) = rx.recv().await {
                let (symbol, current_price) = match event {
                    Event::Market(MarketEvent::Quote { symbol, bid, .. }) => (symbol, bid),
                    Event::Market(MarketEvent::Trade { symbol, price, .. }) => (symbol, price),
                    _ => continue,
                };

                if current_price <= 0.0 {
                    continue;
                }

                if let Some(position) = tracker.get_position(&symbol) {
                    // Skip if already closing
                    if position.is_closing {
                        continue;
                    }

                    let pl_pct = ((current_price - position.entry_price) / position.entry_price) * 100.0;

                    // In verbose mode, log a heartbeat of position evaluation.
                    if config.chatter_level.to_lowercase() == "verbose" {
                        info!("[MONITOR] Check {}: entry={:.8} current={:.8} pl={:.2}% sl={:.8} tp={:.8}",
                              position.symbol, position.entry_price, current_price, pl_pct, position.stop_loss, position.take_profit);
                    }

                    if current_price >= position.take_profit {
                        info!("[MONITOR] SELL trigger (TAKE PROFIT) for {}: entry={:.8} current={:.8} (+{:.2}%) tp={:.8}",
                              position.symbol, position.entry_price, current_price, pl_pct, position.take_profit);
                        Self::generate_exit_signal(&position, "take_profit", current_price, &bus).await;
                        tracker.mark_closing(&position.symbol); // Mark as closing instead of removing
                        continue;
                    }

                    if current_price <= position.stop_loss {
                        warn!("[MONITOR] SELL trigger (STOP LOSS) for {}: entry={:.8} current={:.8} ({:.2}%) sl={:.8}",
                              position.symbol, position.entry_price, current_price, pl_pct, position.stop_loss);
                        Self::generate_exit_signal(&position, "stop_loss", current_price, &bus).await;
                        tracker.mark_closing(&position.symbol); // Mark as closing instead of removing
                        continue;
                    }
                }
            }
        });
    }

    async fn sync_positions(exchange: &dyn TradingApi, tracker: &PositionTracker) {
        info!("🔄 [MONITOR] Syncing positions with exchange {}...", exchange.name());

        match exchange.get_positions().await {
            Ok(positions) => {
                for pos in positions {
                    let symbol = pos.symbol;
                    if symbol.is_empty() || tracker.has_position(&symbol) {
                        continue;
                    }

                    let avg_entry = pos.avg_entry_price.unwrap_or(0.0);
                    let qty = pos.qty;

                    if avg_entry > 0.0 {
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
                            is_closing: false,
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
        _tracker: &PositionTracker,
        _bus: &EventBus,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // Polling-based exit requires market data access; quote-driven is preferred.
        // Keep polling mode as a no-op for now.
        let _ = position;
        Ok(false)
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
