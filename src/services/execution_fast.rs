use crate::agents::{execution::ExecutionAgent, Agent};
use crate::bus::EventBus;
use crate::config::AppConfig;
use crate::data::store::MarketStore;
use crate::events::{Event, ExecutionReport, OrderRequest};
use crate::exchange::{
    traits::TradingApi,
    types::{
        OrderType as ExOrderType, PlaceOrderRequest as ExPlaceOrderRequest, Side as ExSide,
        TimeInForce as ExTimeInForce,
    },
};
use crate::llm::LLMQueue;
use crate::services::execution_utils::{
    aggressive_limit_price, compute_order_sizing, AccountCache, RateLimiter,
};
use crate::services::position_monitor::{PendingOrder, PositionInfo, PositionTracker};
use std::sync::Arc;
use tracing::{error, info, warn};

/// High-performance execution engine optimized for frequent small trades.
pub struct ExecutionEngine {
    event_bus: EventBus,
    exchange: Arc<dyn TradingApi>,
    market_store: MarketStore,
    llm: LLMQueue,
    config: AppConfig,
    tracker: PositionTracker,
    account_cache: AccountCache,
    rate_limiter: RateLimiter,
}

#[derive(serde::Deserialize)]
struct ExecutionOutput {
    action: String,
    qty: f64,
    order_type: String,
}

// MicroTradeConfig is now defined in config.rs

impl ExecutionEngine {
    pub fn new(
        event_bus: EventBus,
        exchange: Arc<dyn TradingApi>,
        market_store: MarketStore,
        llm: LLMQueue,
        config: AppConfig,
        tracker: PositionTracker,
    ) -> Self {
        let micro_config = &config.micro_trade;

        Self {
            event_bus,
            exchange: exchange.clone(),
            market_store,
            llm,
            config: config.clone(),
            tracker,
            account_cache: AccountCache::new(exchange, micro_config.account_cache_secs),
            rate_limiter: RateLimiter::new(micro_config.min_order_interval_ms),
        }
    }

    pub async fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let exchange = self.exchange.clone();
        let store = self.market_store.clone();
        let llm = self.llm.clone();
        let bus = self.event_bus.clone();
        let config = self.config.clone();
        let tracker = self.tracker.clone();
        let account_cache = self.account_cache.clone();
        let rate_limiter = self.rate_limiter.clone();

        tokio::spawn(async move {
            info!("⚡ Execution Engine Started (High-Performance Mode)");
            info!(
                "[EXECUTION] Exchange: {} | Mode: {} | MinOrder=${:.2} MaxOrder=${:.2}",
                exchange.name(),
                config.trading_mode,
                config.defaults.min_order_amount,
                config.defaults.max_order_amount
            );

            while let Ok(event) = rx.recv().await {
                if let Event::Order(req) = event {
                    // Skip verbose logging for performance
                    if config.chatter_level != "low" {
                        info!(
                            "[EXECUTION] Received: {} {} {}",
                            req.action, req.symbol, req.order_type
                        );
                    }

                    // Clone for async task
                    let exchange = exchange.clone();
                    let store = store.clone();
                    let llm = llm.clone();
                    let bus = bus.clone();
                    let config = config.clone();
                    let tracker = tracker.clone();
                    let account_cache = account_cache.clone();
                    let rate_limiter = rate_limiter.clone();

                    // Spawn non-blocking execution
                    tokio::spawn(async move {
                        Self::execute_fast(
                            req,
                            exchange,
                            store,
                            llm,
                            bus,
                            config,
                            tracker,
                            account_cache,
                            rate_limiter,
                        )
                        .await;
                    });
                }
            }
        });
    }

    /// Fast execution path optimized for HFT and micro-trades.
    async fn execute_fast(
        req: OrderRequest,
        exchange: Arc<dyn TradingApi>,
        store: MarketStore,
        llm: LLMQueue,
        bus: EventBus,
        config: AppConfig,
        tracker: PositionTracker,
        account_cache: AccountCache,
        rate_limiter: RateLimiter,
    ) {
        let is_crypto = config.trading_mode.to_lowercase() == "crypto";
        let micro_config = &config.micro_trade;

        // ========== SELL PATH (Fast) ==========
        if req.action == "sell" {
            Self::execute_sell(&req, &exchange, &store, &tracker, &bus, is_crypto).await;
            return;
        }

        // ========== BUY PATH (Optimized) ==========

        // Rate limit check (don't spam orders)
        if !rate_limiter.try_acquire().await {
            if config.chatter_level == "verbose" {
                warn!("[EXECUTION] Rate limited for {}", req.symbol);
            }
            return;
        }

        // Check if we already have a position (avoid stacking)
        if tracker.has_position(&req.symbol) {
            if config.chatter_level != "low" {
                info!("[EXECUTION] Skip {}: already have position", req.symbol);
            }
            return;
        }

        // Check for pending orders on this symbol
        let pending = tracker.get_all_pending_orders();
        if pending.iter().any(|p| p.symbol == req.symbol) {
            if config.chatter_level != "low" {
                info!("[EXECUTION] Skip {}: pending order exists", req.symbol);
            }
            return;
        }

        // Get latest quote (fast path - no API call)
        let quote = match store.get_latest_quote(&req.symbol) {
            Some(q) if q.bid_price > 0.0 && q.ask_price > 0.0 => q,
            _ => {
                error!("[EXECUTION] No valid quote for {}", req.symbol);
                return;
            }
        };

        // Calculate aggressive limit price for faster fills
        let limit_price = aggressive_limit_price(
            quote.bid_price,
            quote.ask_price,
            "buy",
            micro_config.aggression_bps,
        );

        // Get cached buying power (reduces API calls from every order to every 30s)
        let buying_power = account_cache.buying_power().await;
        if buying_power <= 0.0 {
            error!("[EXECUTION] No buying power available");
            return;
        }

        // Compute optimal order size
        let sizing = match compute_order_sizing(
            limit_price,
            buying_power,
            config.defaults.min_order_amount,
            config.defaults.max_order_amount,
            micro_config.target_balance_pct,
        ) {
            Some(s) => s,
            None => {
                error!(
                    "[EXECUTION] Cannot size order for {} (balance=${:.2})",
                    req.symbol, buying_power
                );
                return;
            }
        };

        // Determine if HFT fast path or LLM path
        let is_hft = req.order_type == "hft_buy" || config.strategy_mode.to_lowercase() == "hft";
        let use_llm_filter = config.micro_trade.use_llm_filter;

        let (action, order_type) = if is_hft && !use_llm_filter {
            // Pure HFT: Skip LLM entirely, use limit order
            ("buy".to_string(), ExOrderType::Limit)
        } else if is_hft && use_llm_filter {
            // HFT with LLM filter: Ask LLM to validate the trade
            match Self::get_llm_validation(&req.symbol, &llm, &config).await {
                Some(approved) if approved => ("buy".to_string(), ExOrderType::Limit),
                _ => {
                    if config.chatter_level != "low" {
                        info!("[EXECUTION] LLM filter rejected trade for {}", req.symbol);
                    }
                    return;
                }
            }
        } else {
            // Full LLM path: Call agent for complete decision
            match Self::get_llm_decision(&req.symbol, &llm).await {
                Some((a, ot)) => (a, ot),
                None => return,
            }
        };

        if action != "buy" {
            if config.chatter_level != "low" {
                info!(
                    "[EXECUTION] Agent decided '{}' for {}, skipping",
                    action, req.symbol
                );
            }
            return;
        }

        // Build order request
        // For crypto: Use configured time-in-force (gtc or ioc)
        // For stocks: Use Day
        let time_in_force = if is_crypto {
            match config
                .micro_trade
                .crypto_time_in_force
                .to_lowercase()
                .as_str()
            {
                "ioc" => ExTimeInForce::Ioc, // Immediate Or Cancel
                _ => ExTimeInForce::Gtc,     // Good Till Canceled (default)
            }
        } else {
            ExTimeInForce::Day // Stocks use Day
        };

        let api_req = ExPlaceOrderRequest {
            symbol: req.symbol.clone(),
            side: ExSide::Buy,
            order_type: order_type.clone(),
            qty: Some(sizing.qty),
            notional: None, // Use qty for limit orders
            time_in_force,
            limit_price: if matches!(order_type, ExOrderType::Limit) {
                Some(limit_price)
            } else {
                None
            },
        };

        if config.chatter_level != "low" {
            info!(
                "[ORDER] {} {} qty={:.6} @ ${:.4} (${:.2})",
                if matches!(order_type, ExOrderType::Limit) {
                    "LIMIT"
                } else {
                    "MARKET"
                },
                req.symbol,
                sizing.qty,
                limit_price,
                sizing.notional
            );
        }

        // Submit order
        match exchange.submit_order(api_req).await {
            Ok(res) => {
                if config.chatter_level != "low" {
                    info!("[SUCCESS] Order {} status={}", res.id, res.status);
                }

                // Invalidate account cache after successful order
                account_cache.invalidate().await;

                // IMPORTANT: Always calculate TP/SL from the actual limit price we're buying at
                // Don't use req.stop_loss/take_profit as those are from signal time (stale mid price)
                let (tp_pct, sl_pct) = config.get_symbol_params(&req.symbol);
                let stop_loss = limit_price * (1.0 - sl_pct / 100.0);
                let take_profit = limit_price * (1.0 + tp_pct / 100.0);

                if config.chatter_level != "low" {
                    info!("[EXECUTION] TP/SL calculated from limit_price ${:.8}: TP=${:.8} (+{:.2}%), SL=${:.8} (-{:.2}%)",
                          limit_price, take_profit, tp_pct, stop_loss, sl_pct);
                }

                // Track as pending order (limit) or position (market)
                if matches!(order_type, ExOrderType::Limit) {
                    let pending = PendingOrder {
                        order_id: res.id.clone(),
                        symbol: req.symbol.clone(),
                        side: "buy".to_string(),
                        limit_price,
                        qty: sizing.qty,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        stop_loss: Some(stop_loss),
                        take_profit: Some(take_profit),
                        last_check_time: None,
                    };
                    tracker.add_pending_order(pending);
                } else {
                    let position = PositionInfo {
                        symbol: req.symbol.clone(),
                        entry_price: limit_price,
                        qty: sizing.qty,
                        stop_loss,
                        take_profit,
                        entry_time: chrono::Utc::now().to_rfc3339(),
                        side: "buy".to_string(),
                        is_closing: false,
                        open_order_id: None,
                    };
                    tracker.add_position(position);
                }

                // Publish execution report
                let report = ExecutionReport {
                    symbol: req.symbol,
                    order_id: res.id,
                    status: res.status,
                    side: "buy".to_string(),
                    price: Some(limit_price),
                    qty: Some(sizing.qty),
                };
                bus.publish(Event::Execution(report)).ok();
            }
            Err(e) => {
                error!("[FAILED] Order for {}: {}", req.symbol, e);
            }
        }
    }

    /// Fast sell execution
    async fn execute_sell(
        req: &OrderRequest,
        exchange: &Arc<dyn TradingApi>,
        store: &MarketStore,
        tracker: &PositionTracker,
        bus: &EventBus,
        is_crypto: bool,
    ) {
        // Get sell price from latest quote
        let price = store
            .get_latest_quote(&req.symbol)
            .map(|q| q.bid_price)
            .unwrap_or(0.0);

        if price <= 0.0 {
            error!("[EXECUTION] No price for SELL {}", req.symbol);
            return;
        }

        // Get quantity from tracker or exchange
        let qty = if let Some(pos) = tracker.get_position(&req.symbol) {
            pos.qty
        } else {
            match exchange.get_positions().await {
                Ok(positions) => positions
                    .into_iter()
                    .find(|p| p.symbol == req.symbol)
                    .map(|p| p.qty)
                    .unwrap_or(0.0),
                Err(_) => 0.0,
            }
        };

        if qty <= 0.0 {
            error!("[EXECUTION] No qty for SELL {}", req.symbol);
            return;
        }

        let time_in_force = if is_crypto {
            ExTimeInForce::Gtc
        } else {
            ExTimeInForce::Day
        };

        let api_req = ExPlaceOrderRequest {
            symbol: req.symbol.clone(),
            qty: Some(qty),
            notional: None,
            side: ExSide::Sell,
            order_type: ExOrderType::Market, // Market sell for immediate exit
            time_in_force,
            limit_price: None,
        };

        info!("[ORDER] SELL {} qty={:.6} @ ${:.4}", req.symbol, qty, price);

        match exchange.submit_order(api_req).await {
            Ok(res) => {
                info!("[SUCCESS] SELL {} id={}", req.symbol, res.id);
                tracker.remove_position(&req.symbol);

                let report = ExecutionReport {
                    symbol: req.symbol.clone(),
                    order_id: res.id,
                    status: res.status,
                    side: "sell".to_string(),
                    price: Some(price),
                    qty: Some(qty),
                };
                bus.publish(Event::Execution(report)).ok();
            }
            Err(e) => error!("[FAILED] SELL {}: {}", req.symbol, e),
        }
    }

    /// Get decision from LLM (slower path)
    async fn get_llm_decision(symbol: &str, llm: &LLMQueue) -> Option<(String, ExOrderType)> {
        let agent = ExecutionAgent;
        let input = format!(
            "Symbol: {}\nRisk Analysis: Approved\nAction: Create Order JSON",
            symbol
        );

        match agent.run_high_priority(&input, llm).await {
            Ok(response) => {
                let json_str = Self::extract_json(&response)?;
                let output: ExecutionOutput = serde_json::from_str(json_str).ok()?;

                let order_type = if output.order_type.to_lowercase() == "limit" {
                    ExOrderType::Limit
                } else {
                    ExOrderType::Market
                };

                Some((output.action, order_type))
            }
            Err(e) => {
                error!("[EXECUTION] LLM failed for {}: {}", symbol, e);
                None
            }
        }
    }

    /// Lightweight LLM validation for HFT trades.
    /// Returns true if the trade should proceed, false to skip.
    /// This is faster than full LLM decision-making as it only asks yes/no.
    async fn get_llm_validation(symbol: &str, llm: &LLMQueue, config: &AppConfig) -> Option<bool> {
        let agent = ExecutionAgent;

        // Create a concise prompt for quick validation
        let input = format!(
            "Quick validation for {} trade.\n\
             Strategy: HFT micro-trade, targeting {}bps profit.\n\
             Current spread acceptable.\n\
             Should we proceed? Reply with just 'yes' or 'no'.",
            symbol, config.hft.take_profit_bps
        );

        match agent.run_high_priority(&input, llm).await {
            Ok(response) => {
                let lower = response.to_lowercase();
                let approved =
                    lower.contains("yes") || lower.contains("proceed") || lower.contains("approve");
                Some(approved)
            }
            Err(e) => {
                warn!(
                    "[EXECUTION] LLM validation failed for {}: {}, defaulting to approve",
                    symbol, e
                );
                Some(true) // On LLM failure, default to allowing the trade
            }
        }
    }

    fn extract_json(text: &str) -> Option<&str> {
        let start = text.find('{')?;
        let end = text.rfind('}')?;
        if start < end {
            Some(&text[start..=end])
        } else {
            None
        }
    }
}
