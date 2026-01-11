use crate::bus::EventBus;
use crate::config::AppConfig;
use crate::events::{AnalysisSignal, Event, MarketEvent};
use crate::exchange::traits::TradingApi;
use crate::exchange::types::{
    OrderType as ExOrderType, PlaceOrderRequest as ExPlaceOrderRequest, Side as ExSide,
    TimeInForce as ExTimeInForce,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

#[derive(Clone, Debug)]
pub struct PositionInfo {
    pub symbol: String,
    pub entry_price: f64,
    pub qty: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub entry_time: String,
    pub side: String,                           // "buy" or "sell"
    pub is_closing: bool,                       // New field to prevent double-sells
    pub open_order_id: Option<String>,          // For Take Profit Limit Order
    pub last_recreate_attempt: Option<Instant>, // Track last recreation attempt
    pub recreate_attempts: u32,                 // Count failed recreation attempts
    // Trailing stop fields
    pub highest_price: f64,         // Track highest price for trailing stop
    pub trailing_stop_active: bool, // Is trailing stop activated?
    pub trailing_stop_price: f64,   // Current trailing stop level
}

#[derive(Clone, Debug)]
pub struct PendingOrder {
    pub order_id: String,
    pub symbol: String,
    pub side: String,
    pub limit_price: f64,
    pub qty: f64,
    pub created_at: String,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub last_check_time: Option<std::time::Instant>,
}

#[derive(Clone)]
pub struct PositionTracker {
    positions: Arc<Mutex<HashMap<String, PositionInfo>>>,
    pending_orders: Arc<Mutex<HashMap<String, PendingOrder>>>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: Arc::new(Mutex::new(HashMap::new())),
            pending_orders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn add_pending_order(&self, mut order: PendingOrder) {
        let mut pending = self.pending_orders.lock().unwrap();
        order.last_check_time = Some(std::time::Instant::now());
        info!(
            "📊 [TRACKER] Added pending order: {} {} @ ${:.8}",
            order.side, order.symbol, order.limit_price
        );
        pending.insert(order.order_id.clone(), order);
    }

    pub fn update_pending_order_check_time(&self, order_id: &str) {
        let mut pending = self.pending_orders.lock().unwrap();
        if let Some(order) = pending.get_mut(order_id) {
            order.last_check_time = Some(std::time::Instant::now());
        }
    }

    pub fn remove_pending_order(&self, order_id: &str) -> Option<PendingOrder> {
        let mut pending = self.pending_orders.lock().unwrap();
        pending.remove(order_id)
    }

    pub fn get_all_pending_orders(&self) -> Vec<PendingOrder> {
        let pending = self.pending_orders.lock().unwrap();
        pending.values().cloned().collect()
    }

    pub fn add_position(&self, mut info: PositionInfo) {
        let mut positions = self.positions.lock().unwrap();
        // Ensure is_closing is false initially
        info.is_closing = false;
        info!(
            "📊 [TRACKER] Added position: {} @ ${:.8} (SL: ${:.8}, TP: ${:.8})",
            info.symbol, info.entry_price, info.stop_loss, info.take_profit
        );
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
    pub fn new(
        event_bus: EventBus,
        exchange: Arc<dyn TradingApi>,
        tracker: PositionTracker,
        config: AppConfig,
    ) -> Self {
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
        let config = self.config.clone();

        tokio::spawn(async move {
            info!("👁️  Position Monitor Started (polling every {}s)", interval);

            // Initial sync with exchange positions
            Self::sync_positions(&*exchange, &tracker, &config).await;

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
            info!(
                "👁️  Position Monitor Started (quote-driven exits) | chatter={}",
                config.chatter_level
            );

            // Initial sync with exchange positions
            Self::sync_positions(&*exchange, &tracker, &config).await;

            while let Ok(event) = rx.recv().await {
                let (symbol, current_price) = match event {
                    Event::Market(MarketEvent::Quote { symbol, bid, .. }) => (symbol, bid),
                    Event::Market(MarketEvent::Trade { symbol, price, .. }) => (symbol, price),
                    _ => continue,
                };

                if current_price <= 0.0 {
                    continue;
                }

                // Check Pending Orders
                let pending_orders = tracker.get_all_pending_orders();
                for order in &pending_orders {
                    if order.symbol == symbol {
                        // Check for expiration
                        if let Some(days) = config.defaults.limit_order_expiration_days {
                            if let Ok(created_at) =
                                chrono::DateTime::parse_from_rfc3339(&order.created_at)
                            {
                                let now = chrono::Utc::now();
                                let age = now.signed_duration_since(created_at);
                                if age.num_days() >= days as i64 {
                                    warn!(
                                        "[MONITOR] Order {} expired (age: {} days). Cancelling.",
                                        order.order_id,
                                        age.num_days()
                                    );
                                    if let Err(e) = exchange.cancel_order(&order.order_id).await {
                                        error!(
                                            "Failed to cancel expired order {}: {}",
                                            order.order_id, e
                                        );
                                    }
                                    tracker.remove_pending_order(&order.order_id);
                                    continue;
                                }
                            }
                        }

                        // Rate limit checks: only check every 2 seconds per order
                        if let Some(last_check) = order.last_check_time {
                            if last_check.elapsed() < Duration::from_secs(2) {
                                continue;
                            }
                        }

                        if order.side == "buy" {
                            // Check if filled (Price <= Limit)
                            if current_price <= order.limit_price {
                                tracker.update_pending_order_check_time(&order.order_id);
                                Self::check_pending_buy_order(
                                    &order, &*exchange, &tracker, &config,
                                )
                                .await;
                            }
                        } else if order.side == "sell" {
                            // Take Profit Limit Order
                            // Check if filled (Price >= Limit)
                            if current_price >= order.limit_price {
                                tracker.update_pending_order_check_time(&order.order_id);
                                Self::check_pending_sell_order(&order, &*exchange, &tracker).await;
                            }

                            // Check Stop Loss condition
                            if let Some(sl) = order.stop_loss {
                                if current_price <= sl {
                                    warn!("[MONITOR] Price dropped to ${:.2} (SL ${:.2}). Cancelling Limit Sell and exiting.", current_price, sl);
                                    // Cancel Limit Order
                                    if let Err(e) = exchange.cancel_order(&order.order_id).await {
                                        error!("Failed to cancel order {}: {}", order.order_id, e);
                                    }
                                    tracker.remove_pending_order(&order.order_id);

                                    // Trigger Market Sell (Exit Signal)
                                    let pos_info = PositionInfo {
                                        symbol: order.symbol.clone(),
                                        entry_price: order.limit_price, // Approximate
                                        qty: order.qty,
                                        stop_loss: sl,
                                        take_profit: order.limit_price,
                                        entry_time: order.created_at.clone(),
                                        side: "buy".to_string(),
                                        is_closing: true,
                                        open_order_id: None,
                                        last_recreate_attempt: None,
                                        recreate_attempts: 0,
                                        highest_price: order.limit_price,
                                        trailing_stop_active: false,
                                        trailing_stop_price: sl,
                                    };
                                    Self::generate_exit_signal(
                                        &pos_info,
                                        "stop_loss_limit_cancel",
                                        current_price,
                                        &bus,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                }

                if let Some(position) = tracker.get_position(&symbol) {
                    // Skip if already closing
                    if position.is_closing {
                        continue;
                    }

                    // IMPORTANT: Check if position has an exit order
                    // If open_order_id is None, this position is orphaned!
                    if position.open_order_id.is_none() {
                        // Check if we've exceeded retry attempts
                        if position.recreate_attempts >= 3 {
                            error!(
                                "❌ [MONITOR] Position {} has failed {} recreation attempts - removing from tracker",
                                position.symbol, position.recreate_attempts
                            );
                            tracker.remove_position(&position.symbol);
                            continue;
                        }

                        // Rate limit recreation attempts - only try every 30 seconds
                        if let Some(last_attempt) = position.last_recreate_attempt {
                            let elapsed = last_attempt.elapsed();
                            if elapsed < Duration::from_secs(30) {
                                // Too soon to retry - skip this iteration
                                continue;
                            }
                        }

                        warn!(
                            "🔍 [MONITOR] Detected orphaned position: {} (no exit order, attempt {}/3)",
                            position.symbol, position.recreate_attempts + 1
                        );

                        // Check if there's actually a pending sell order we don't know about
                        let has_pending_sell = pending_orders
                            .iter()
                            .any(|o| o.symbol == position.symbol && o.side == "sell");

                        if !has_pending_sell {
                            warn!(
                                "🚨 [MONITOR] Position {} has NO pending sell order - recreating!",
                                position.symbol
                            );

                            // Update attempt tracking BEFORE trying to recreate
                            let mut updated_pos = position.clone();
                            updated_pos.last_recreate_attempt = Some(Instant::now());
                            updated_pos.recreate_attempts += 1;
                            tracker.add_position(updated_pos.clone());

                            Self::recreate_limit_sell_order(&updated_pos, &*exchange, &tracker)
                                .await;
                            // Skip further checks this iteration to avoid conflicts
                            continue;
                        } else {
                            // Sync: Link the pending order ID to the position
                            if let Some(pending) = pending_orders
                                .iter()
                                .find(|o| o.symbol == position.symbol && o.side == "sell")
                            {
                                let mut updated_pos = position.clone();
                                updated_pos.open_order_id = Some(pending.order_id.clone());
                                tracker.add_position(updated_pos);
                                info!(
                                    "🔗 [MONITOR] Linked position {} to pending order {}",
                                    position.symbol, pending.order_id
                                );
                            }
                        }
                    }

                    // If we have an open Limit Sell (TP), we don't need to check TP here,
                    // but we DO need to check SL (which is handled above if we track it as PendingOrder).
                    // If we have open_order_id, we assume it's being tracked as PendingOrder.
                    if position.open_order_id.is_some() {
                        continue;
                    }

                    let pl_pct =
                        ((current_price - position.entry_price) / position.entry_price) * 100.0;

                    // In verbose mode, log a heartbeat of position evaluation.
                    if config.chatter_level.to_lowercase() == "verbose" {
                        info!("[MONITOR] Check {}: entry={:.8} current={:.8} pl={:.2}% sl={:.8} tp={:.8}",
                              position.symbol, position.entry_price, current_price, pl_pct, position.stop_loss, position.take_profit);
                    }

                    if current_price >= position.take_profit {
                        info!("[MONITOR] SELL trigger (TAKE PROFIT) for {}: entry={:.8} current={:.8} (+{:.2}%) tp={:.8}",
                              position.symbol, position.entry_price, current_price, pl_pct, position.take_profit);
                        Self::generate_exit_signal(&position, "take_profit", current_price, &bus)
                            .await;
                        tracker.mark_closing(&position.symbol); // Mark as closing instead of removing
                        continue;
                    }

                    if current_price <= position.stop_loss {
                        warn!("[MONITOR] SELL trigger (STOP LOSS) for {}: entry={:.8} current={:.8} ({:.2}%) sl={:.8}",
                              position.symbol, position.entry_price, current_price, pl_pct, position.stop_loss);
                        Self::generate_exit_signal(&position, "stop_loss", current_price, &bus)
                            .await;
                        tracker.mark_closing(&position.symbol); // Mark as closing instead of removing
                        continue;
                    }
                }
            }
        });
    }

    async fn sync_positions(
        exchange: &dyn TradingApi,
        tracker: &PositionTracker,
        config: &AppConfig,
    ) {
        info!(
            "🔄 [MONITOR] Syncing positions with exchange {}...",
            exchange.name()
        );

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
                        let (tp_pct, sl_pct) = config.get_symbol_params(&symbol);
                        let stop_loss = avg_entry * (1.0 - sl_pct / 100.0);
                        let take_profit = avg_entry * (1.0 + tp_pct / 100.0);

                        let pos_info = PositionInfo {
                            symbol: symbol.clone(),
                            entry_price: avg_entry,
                            qty,
                            stop_loss,
                            take_profit,
                            entry_time: chrono::Utc::now().to_rfc3339(),
                            side: "buy".to_string(),
                            is_closing: false,
                            open_order_id: None,
                            last_recreate_attempt: None,
                            recreate_attempts: 0,
                            highest_price: avg_entry,
                            trailing_stop_active: false,
                            trailing_stop_price: stop_loss,
                        };

                        tracker.add_position(pos_info.clone());
                        warn!(
                            "⚠️  [MONITOR] Added existing position {} (defaults: SL -{:.2}%, TP +{:.2}%)",
                            symbol, sl_pct, tp_pct
                        );

                        // IMPORTANT: Create exit order for this synced position
                        info!(
                            "🔄 [MONITOR] Creating exit order for synced position {}",
                            symbol
                        );
                        Self::recreate_limit_sell_order(&pos_info, exchange, tracker).await;
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

    async fn check_pending_buy_order(
        order: &PendingOrder,
        exchange: &dyn TradingApi,
        tracker: &PositionTracker,
        config: &AppConfig,
    ) {
        match exchange.get_order(&order.order_id).await {
            Ok(ack) => {
                if ack.status.eq_ignore_ascii_case("filled") {
                    // IMPORTANT: Extract actual filled quantity from order response
                    // This prevents "insufficient balance" errors from quantity mismatches
                    let filled_qty = ack
                        .raw
                        .get("filled_qty")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                        .or_else(|| ack.raw.get("filled_qty").and_then(|v| v.as_f64()))
                        .unwrap_or(order.qty);

                    // Warn if there's a quantity mismatch
                    if (filled_qty - order.qty).abs() > 0.000001 {
                        warn!(
                            "⚠️ [MONITOR] Quantity mismatch for {}: ordered={}, filled={} - using filled qty",
                            order.symbol, order.qty, filled_qty
                        );
                    }

                    info!(
                        "✅ [MONITOR] Pending BUY filled: {} qty={} @ ${:.2}",
                        order.symbol, filled_qty, order.limit_price
                    );
                    tracker.remove_pending_order(&order.order_id);

                    let (tp_pct, sl_pct) = config.get_symbol_params(&order.symbol);
                    // IMPORTANT: Always recalculate TP/SL based on actual fill price
                    // The signal's TP might be stale (calculated from mid at signal time)
                    // which could be LOWER than the aggressive buy limit price
                    let fill_price = order.limit_price;
                    let take_profit_price = fill_price * (1.0 + tp_pct / 100.0);
                    let stop_loss_price = fill_price * (1.0 - sl_pct / 100.0);

                    info!("📊 [MONITOR] Calculating TP/SL from fill price ${:.8}: TP=${:.8} (+{:.2}%), SL=${:.8} (-{:.2}%)",
                          fill_price, take_profit_price, tp_pct, stop_loss_price, sl_pct);

                    // Create Position with ACTUAL filled quantity
                    let mut pos_info = PositionInfo {
                        symbol: order.symbol.clone(),
                        entry_price: fill_price,
                        qty: filled_qty, // Use actual filled qty
                        stop_loss: stop_loss_price,
                        take_profit: take_profit_price,
                        entry_time: chrono::Utc::now().to_rfc3339(),
                        side: "buy".to_string(),
                        is_closing: false,
                        open_order_id: None,
                        last_recreate_attempt: None,
                        recreate_attempts: 0,
                        highest_price: fill_price,
                        trailing_stop_active: false,
                        trailing_stop_price: stop_loss_price,
                    };

                    // Submit Limit Sell (TP) with ACTUAL filled quantity
                    let tp_req = ExPlaceOrderRequest {
                        symbol: order.symbol.clone(),
                        side: ExSide::Sell,
                        order_type: ExOrderType::Limit,
                        qty: Some(filled_qty), // Use actual filled qty
                        notional: None,
                        limit_price: Some(pos_info.take_profit),
                        time_in_force: ExTimeInForce::Gtc, // Crypto usually GTC
                    };

                    info!(
                        "🚀 [MONITOR] Submitting Take Profit Limit Sell for {} @ ${:.2}",
                        order.symbol, pos_info.take_profit
                    );
                    match exchange.submit_order(tp_req).await {
                        Ok(res) => {
                            info!("✅ [MONITOR] TP Limit Sell Placed: {}", res.id);
                            pos_info.open_order_id = Some(res.id.clone());

                            // Add TP to Pending Orders
                            // NOTE: We don't set stop_loss on the sell order itself.
                            // The position is monitored separately for SL conditions.
                            // This prevents the TP sell from being cancelled due to SL.
                            let tp_pending = PendingOrder {
                                order_id: res.id,
                                symbol: order.symbol.clone(),
                                side: "sell".to_string(),
                                limit_price: pos_info.take_profit,
                                qty: filled_qty, // Use actual filled qty
                                created_at: chrono::Utc::now().to_rfc3339(),
                                stop_loss: None, // Don't attach SL to the sell order
                                take_profit: None,
                                last_check_time: None,
                            };
                            tracker.add_pending_order(tp_pending);
                        }
                        Err(e) => {
                            error!("❌ [MONITOR] Failed to place TP Limit Sell: {}", e);
                        }
                    }

                    tracker.add_position(pos_info);
                } else if ack.status.eq_ignore_ascii_case("canceled")
                    || ack.status.eq_ignore_ascii_case("expired")
                {
                    info!(
                        "❌ [MONITOR] Pending BUY canceled/expired: {}",
                        order.symbol
                    );
                    tracker.remove_pending_order(&order.order_id);
                }
            }
            Err(e) => error!("❌ [MONITOR] Failed to check order status: {}", e),
        }
    }

    async fn check_pending_sell_order(
        order: &PendingOrder,
        exchange: &dyn TradingApi,
        tracker: &PositionTracker,
    ) {
        match exchange.get_order(&order.order_id).await {
            Ok(ack) => {
                if ack.status.eq_ignore_ascii_case("filled") {
                    info!(
                        "💰 [MONITOR] Take Profit Limit Sell FILLED: {} @ ${:.2}",
                        order.symbol, order.limit_price
                    );
                    tracker.remove_pending_order(&order.order_id);
                    tracker.remove_position(&order.symbol);
                } else if ack.status.eq_ignore_ascii_case("canceled")
                    || ack.status.eq_ignore_ascii_case("expired")
                {
                    warn!(
                        "⚠️ [MONITOR] TP Limit Sell canceled/expired: {}",
                        order.symbol
                    );
                    tracker.remove_pending_order(&order.order_id);

                    // IMPORTANT: Position is now orphaned without exit order
                    // Clear open_order_id and flag for recreation
                    if let Some(mut pos) = tracker.get_position(&order.symbol) {
                        pos.open_order_id = None;
                        tracker.add_position(pos.clone());

                        warn!(
                            "🔄 [MONITOR] Position {} now without exit order - will recreate",
                            order.symbol
                        );

                        // Recreate limit sell order immediately
                        Self::recreate_limit_sell_order(&pos, exchange, tracker).await;
                    }
                }
            }
            Err(e) => error!("❌ [MONITOR] Failed to check sell order status: {}", e),
        }
    }

    /// Recreate a limit sell order for a position that lost its exit order
    async fn recreate_limit_sell_order(
        position: &PositionInfo,
        exchange: &dyn TradingApi,
        tracker: &PositionTracker,
    ) {
        info!(
            "🔄 [MONITOR] Recreating TP Limit Sell for {} @ ${:.8}",
            position.symbol, position.take_profit
        );

        // IMPORTANT: Verify actual holdings before placing sell order
        // This prevents "insufficient balance" errors from quantity mismatches
        let (actual_qty, position_exists) = match exchange.get_positions().await {
            Ok(positions) => {
                if let Some(pos) = positions.iter().find(|p| p.symbol == position.symbol) {
                    (pos.qty, true)
                } else {
                    // Position not found on exchange - likely already closed
                    warn!(
                        "⚠️ [MONITOR] Position {} not found on exchange during verification - likely already closed",
                        position.symbol
                    );
                    (0.0, false)
                }
            }
            Err(e) => {
                warn!(
                    "⚠️ [MONITOR] Could not verify holdings for {}: {} (using tracked qty)",
                    position.symbol, e
                );
                (position.qty, true) // Assume exists on API error
            }
        };

        // If position doesn't exist on exchange, remove from tracker and return
        if !position_exists {
            tracker.remove_position(&position.symbol);
            info!(
                "🧹 [MONITOR] Cleaned up tracked position {} (not on exchange)",
                position.symbol
            );
            return;
        }

        // If actual quantity differs from tracked, update the position
        let final_qty = if (actual_qty - position.qty).abs() > 0.000001 {
            warn!(
                "⚠️ [MONITOR] Quantity mismatch for {}: tracked={}, actual={} - using actual",
                position.symbol, position.qty, actual_qty
            );

            // Update the tracked position with correct quantity
            let mut corrected_pos = position.clone();
            corrected_pos.qty = actual_qty;
            tracker.add_position(corrected_pos);

            actual_qty
        } else {
            position.qty
        };

        // Safety check: Don't place order if qty is zero or negative
        if final_qty <= 0.0 {
            warn!(
                "⚠️ [MONITOR] Position {} has zero/negative quantity: {} - removing from tracker",
                position.symbol, final_qty
            );
            tracker.remove_position(&position.symbol);
            return;
        }

        let tp_req = ExPlaceOrderRequest {
            symbol: position.symbol.clone(),
            side: ExSide::Sell,
            order_type: ExOrderType::Limit,
            qty: Some(final_qty),
            notional: None,
            limit_price: Some(position.take_profit),
            time_in_force: ExTimeInForce::Gtc,
        };

        match exchange.submit_order(tp_req).await {
            Ok(res) => {
                info!(
                    "✅ [MONITOR] Recreated TP Limit Sell: {} (order: {})",
                    position.symbol, res.id
                );

                // Update position with new order ID
                let mut updated_pos = position.clone();
                updated_pos.open_order_id = Some(res.id.clone());
                tracker.add_position(updated_pos);

                // Track as pending order
                let tp_pending = PendingOrder {
                    order_id: res.id,
                    symbol: position.symbol.clone(),
                    side: "sell".to_string(),
                    limit_price: position.take_profit,
                    qty: final_qty, // Use final_qty, not position.qty
                    created_at: chrono::Utc::now().to_rfc3339(),
                    stop_loss: None,
                    take_profit: None,
                    last_check_time: None,
                };
                tracker.add_pending_order(tp_pending);
            }
            Err(e) => {
                let error_msg = format!("{}", e);

                // Check if this is an insufficient balance error (403 with code 40310000)
                if (error_msg.contains("403") && error_msg.contains("40310000"))
                    || error_msg.contains("insufficient balance")
                {
                    warn!(
                        "⚠️ [MONITOR] Insufficient balance error for {} - verifying actual holdings and retrying",
                        position.symbol
                    );

                    // RETRY: Get fresh holdings directly from exchange
                    match exchange.get_positions().await {
                        Ok(positions) => {
                            if let Some(pos) =
                                positions.iter().find(|p| p.symbol == position.symbol)
                            {
                                let verified_qty = pos.qty;

                                warn!(
                                    "🔄 [MONITOR] Verified holdings for {}: tried={}, actual={} - retrying with actual",
                                    position.symbol, final_qty, verified_qty
                                );

                                // Safety check
                                if verified_qty <= 0.0 {
                                    error!(
                                        "❌ [MONITOR] Verified qty is invalid: {} - cannot retry",
                                        verified_qty
                                    );
                                    return;
                                }

                                // Update position with verified quantity
                                let mut corrected_pos = position.clone();
                                corrected_pos.qty = verified_qty;
                                tracker.add_position(corrected_pos);

                                // Retry with verified quantity
                                let retry_req = ExPlaceOrderRequest {
                                    symbol: position.symbol.clone(),
                                    side: ExSide::Sell,
                                    order_type: ExOrderType::Limit,
                                    qty: Some(verified_qty),
                                    notional: None,
                                    limit_price: Some(position.take_profit),
                                    time_in_force: ExTimeInForce::Gtc,
                                };

                                match exchange.submit_order(retry_req).await {
                                    Ok(retry_res) => {
                                        info!(
                                            "✅ [MONITOR] Retry successful - TP Limit Sell: {} qty={} (order: {})",
                                            position.symbol, verified_qty, retry_res.id
                                        );

                                        // Update position with new order ID
                                        let mut updated_pos = position.clone();
                                        updated_pos.qty = verified_qty;
                                        updated_pos.open_order_id = Some(retry_res.id.clone());
                                        tracker.add_position(updated_pos);

                                        // Track as pending order
                                        let tp_pending = PendingOrder {
                                            order_id: retry_res.id,
                                            symbol: position.symbol.clone(),
                                            side: "sell".to_string(),
                                            limit_price: position.take_profit,
                                            qty: verified_qty,
                                            created_at: chrono::Utc::now().to_rfc3339(),
                                            stop_loss: None,
                                            take_profit: None,
                                            last_check_time: None,
                                        };
                                        tracker.add_pending_order(tp_pending);
                                    }
                                    Err(retry_err) => {
                                        error!(
                                            "❌ [MONITOR] Retry failed for {} with verified qty {}: {}",
                                            position.symbol, verified_qty, retry_err
                                        );
                                    }
                                }
                            } else {
                                warn!(
                                    "⚠️ [MONITOR] Position {} not found in exchange holdings - likely already closed",
                                    position.symbol
                                );

                                // Position doesn't exist on exchange - remove from our tracker
                                tracker.remove_position(&position.symbol);

                                info!(
                                    "🧹 [MONITOR] Cleaned up tracked position {} (not on exchange)",
                                    position.symbol
                                );
                            }
                        }
                        Err(verify_err) => {
                            error!(
                                "❌ [MONITOR] Failed to verify holdings for {}: {}",
                                position.symbol, verify_err
                            );
                        }
                    }
                } else {
                    // Not an insufficient balance error - log and continue
                    error!(
                        "❌ [MONITOR] Failed to recreate TP Limit Sell for {}: {}",
                        position.symbol, e
                    );
                }
            }
        }
    }
}
