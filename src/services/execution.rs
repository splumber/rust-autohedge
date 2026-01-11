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
use crate::services::position_monitor::{PositionInfo, PositionTracker};
use std::sync::Arc;
use tracing::{error, info};

pub struct ExecutionEngine {
    event_bus: EventBus,
    exchange: Arc<dyn TradingApi>,
    market_store: MarketStore,
    llm: LLMQueue,
    config: AppConfig,
    tracker: PositionTracker,
}

#[derive(serde::Deserialize)]
struct ExecutionOutput {
    action: String,
    qty: f64,
    order_type: String,
}

impl ExecutionEngine {
    pub fn new(
        event_bus: EventBus,
        exchange: Arc<dyn TradingApi>,
        market_store: MarketStore,
        llm: LLMQueue,
        config: AppConfig,
        tracker: PositionTracker,
    ) -> Self {
        Self {
            event_bus,
            exchange,
            market_store,
            llm,
            config,
            tracker,
        }
    }

    pub async fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let exchange_clone = self.exchange.clone();
        let store_clone = self.market_store.clone();
        let llm_clone = self.llm.clone();
        let bus_clone = self.event_bus.clone();
        let config_clone = self.config.clone();
        let tracker_clone = self.tracker.clone();

        tokio::spawn(async move {
            info!("⚡ Execution Engine Started");
            info!(
                "[EXECUTION] Exchange: {} | Mode: {} | MinOrder=${:.2} MaxOrder=${:.2}",
                exchange_clone.name(),
                config_clone.trading_mode,
                config_clone.defaults.min_order_amount,
                config_clone.defaults.max_order_amount
            );
            while let Ok(event) = rx.recv().await {
                if let Event::Order(req) = event {
                    info!("[EXECUTION] Received OrderRequest: symbol={} action={} order_type={} limit_price={:?} sl={:?} tp={:?}",
                          req.symbol, req.action, req.order_type, req.limit_price, req.stop_loss, req.take_profit);

                    let exchange = exchange_clone.clone();
                    let store = store_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let config = config_clone.clone();
                    let tracker = tracker_clone.clone();

                    tokio::spawn(async move {
                        Self::execute_order(req, exchange, store, llm, bus, config, tracker).await;
                    });
                }
            }
            info!("[EXECUTION] Event loop ended (channel closed)");
        });
    }

    async fn execute_order(
        req: OrderRequest,
        exchange: Arc<dyn TradingApi>,
        store: MarketStore,
        llm: LLMQueue,
        bus: EventBus,
        config: AppConfig,
        tracker: PositionTracker,
    ) {
        let is_crypto = config.trading_mode.to_lowercase() == "crypto";
        info!(
            "[EXECUTION] Begin execute_order: symbol={} action={} (crypto={})",
            req.symbol, req.action, is_crypto
        );

        // Handle sell orders directly (from Position Monitor)
        if req.action == "sell" {
            info!(
                "[EXECUTION] SELL path (monitor-triggered) for {}",
                req.symbol
            );

            let estimated_price = store
                .get_latest_quote(&req.symbol)
                .map(|q| q.bid_price)
                .unwrap_or(0.0);

            info!(
                "[EXECUTION] Estimated SELL price for {}: ${:.8}",
                req.symbol, estimated_price
            );

            if estimated_price == 0.0 {
                error!(
                    "[EXECUTION] Cannot estimate price for {}. No market data available.",
                    req.symbol
                );
                return;
            }

            // Prefer local tracker qty; fall back to exchange positions as a safety net.
            let tracked_qty = tracker.get_position(&req.symbol).map(|p| p.qty);
            info!(
                "[EXECUTION] Tracker qty for {}: {:?}",
                req.symbol, tracked_qty
            );

            let qty = if let Some(qty) = tracked_qty {
                qty
            } else {
                info!(
                    "[EXECUTION] Tracker missing qty for {}. Fetching positions from exchange...",
                    req.symbol
                );
                match exchange.get_positions().await {
                    Ok(positions) => {
                        let position = positions.into_iter().find(|p| p.symbol == req.symbol);
                        position.map(|p| p.qty).unwrap_or(0.0)
                    }
                    Err(e) => {
                        error!(
                            "[EXECUTION] Failed to fetch positions for sell {}: {}",
                            req.symbol, e
                        );
                        0.0
                    }
                }
            };

            if qty <= 0.0 {
                error!("[EXECUTION] No quantity found for {} position", req.symbol);
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
                order_type: ExOrderType::Market,
                time_in_force,
                limit_price: None,
            };

            info!(
                "[ORDER] Submitting SELL: qty={:.8} symbol={} est_price=${:.8} est_value=${:.2}",
                qty,
                req.symbol,
                estimated_price,
                qty * estimated_price
            );

            match exchange.submit_order(api_req).await {
                Ok(res) => {
                    info!(
                        "[SUCCESS] SELL Order Placed: id={} status={}",
                        res.id, res.status
                    );

                    tracker.remove_position(&req.symbol);

                    let report = ExecutionReport {
                        symbol: req.symbol,
                        order_id: res.id,
                        status: res.status,
                        side: "sell".to_string(),
                        price: Some(estimated_price),
                        qty: Some(qty),
                    };
                    info!(
                        "[EXECUTION] Publishing ExecutionReport for SELL {}",
                        report.symbol
                    );
                    bus.publish(Event::Execution(report)).ok();
                }
                Err(e) => error!("[FAILED] SELL Order Submission: {}", e),
            }

            return;
        }

        // Handle buy orders (original logic with ExecutionAgent) or HFT fast path
        let mut order = if req.order_type == "hft_buy" {
            info!("[EXECUTION] HFT Fast Path for {}", req.symbol);
            ExecutionOutput {
                action: "buy".to_string(),
                qty: 0.0, // Will be sized to min_order_amount by logic below
                order_type: "limit".to_string(),
            }
        } else {
            info!("[EXECUTION] BUY path (agent-driven) for {}", req.symbol);

            let execution_agent = ExecutionAgent;
            let exec_input = format!(
                "Symbol: {}\nRisk Analysis: Approved\nAction: Create Order JSON",
                req.symbol
            );
            info!("[EXECUTION] Calling ExecutionAgent for {}", req.symbol);

            let order_response = match execution_agent.run_high_priority(&exec_input, &llm).await {
                Ok(res) => res,
                Err(e) => {
                    error!("Execution Agent Failed: {}", e);
                    return;
                }
            };

            info!(
                "[EXECUTION] Agent Output (raw) for {}: {}",
                req.symbol, order_response
            );

            let json_str = Self::extract_json(&order_response).unwrap_or(&order_response);
            info!(
                "[EXECUTION] Agent Output (json_str) for {}: {}",
                req.symbol, json_str
            );

            match serde_json::from_str::<ExecutionOutput>(json_str) {
                Ok(o) => o,
                Err(e) => {
                    error!("[EXECUTION] JSON Parse Error: {}", e);
                    return;
                }
            }
        };

        info!(
            "[EXECUTION] Processing order: action={} qty={:.8} order_type={}",
            order.action, order.qty, order.order_type
        );

        if order.action == "buy" || order.action == "sell" {
            let history = store.get_quote_history(&req.symbol);
            let estimated_price = if let Some(latest) = history.last() {
                if order.action == "buy" {
                    latest.ask_price
                } else {
                    latest.bid_price
                }
            } else {
                0.0
            };

            info!(
                "[EXECUTION] Estimated price for {}: ${:.8}",
                req.symbol, estimated_price
            );

            if estimated_price == 0.0 {
                error!(
                    "[EXECUTION] Cannot estimate price for {}. No market data available.",
                    req.symbol
                );
                return;
            }

            // Estimate value from agent qty; tighten to min/max via config.
            let mut estimated_value = order.qty * estimated_price;
            info!(
                "[EXECUTION] Initial sizing for {} => qty={:.8} est_value=${:.2}",
                req.symbol, order.qty, estimated_value
            );

            if estimated_value < config.defaults.min_order_amount {
                info!(
                    "[RISK] Order value ${:.2} is below minimum ${:.2}. Adjusting.",
                    estimated_value, config.defaults.min_order_amount
                );
                estimated_value = config.defaults.min_order_amount;
                order.qty = estimated_value / estimated_price;
                info!(
                    "[RISK] Adjusted qty for min order => qty={:.8} est_value=${:.2}",
                    order.qty, estimated_value
                );
            }

            if estimated_value > config.defaults.max_order_amount {
                info!(
                    "[RISK] Order value ${:.2} exceeds limit ${:.2}. Capping.",
                    estimated_value, config.defaults.max_order_amount
                );
                estimated_value = config.defaults.max_order_amount;
                order.qty = estimated_value / estimated_price;
                info!(
                    "[RISK] Adjusted qty for max cap => qty={:.8} est_value=${:.2}",
                    order.qty, estimated_value
                );
            }

            // Balance Check (Post-Adjustment)
            if order.action == "buy" {
                match exchange.get_account().await {
                    Ok(account) => {
                        let buying_power = account.buying_power.or(account.cash).unwrap_or(0.0);
                        let required_funds = estimated_value; // No buffer here, exact check against value

                        if buying_power < required_funds {
                            let max_affordable = buying_power * 0.99; // 1% buffer for fees
                            if max_affordable < config.defaults.min_order_amount {
                                error!("[EXECUTION] Insufficient funds. Available: ${:.2}, Min Required: ${:.2}", buying_power, config.defaults.min_order_amount);
                                return;
                            }

                            info!("[EXECUTION] Capping order to affordable amount: ${:.2} (Available: ${:.2})", max_affordable, buying_power);
                            estimated_value = max_affordable;
                            order.qty = estimated_value / estimated_price;
                        }
                    }
                    Err(e) => {
                        error!("[EXECUTION] Failed to fetch account balance: {}", e);
                        return;
                    }
                }
            }

            // Force Limit Order for Buy
            let mut order_type_enum = if order.order_type.to_lowercase() == "limit" {
                ExOrderType::Limit
            } else {
                ExOrderType::Market
            };
            if order.action == "buy" {
                order_type_enum = ExOrderType::Limit;
            }

            info!("[ORDER] Submitting: action={} qty={:.8} symbol={} est_value=${:.2} order_type={:?}",
                          order.action, order.qty, req.symbol, estimated_value, order_type_enum);

            let time_in_force = if is_crypto {
                ExTimeInForce::Gtc
            } else {
                ExTimeInForce::Day
            };

            let supports_notional = exchange.capabilities().supports_notional_market_buy;

            // For Limit orders, we usually need Qty, not Notional.
            let (qty, notional) = if is_crypto
                && order.action == "buy"
                && supports_notional
                && matches!(order_type_enum, ExOrderType::Market)
            {
                (None, Some(estimated_value))
            } else {
                (Some(order.qty), None)
            };

            let side = if order.action == "buy" {
                ExSide::Buy
            } else {
                ExSide::Sell
            };

            let limit_price = if matches!(order_type_enum, ExOrderType::Limit) {
                Some(estimated_price)
            } else {
                None
            };

            let api_req = ExPlaceOrderRequest {
                symbol: req.symbol.clone(),
                side,
                order_type: order_type_enum,
                qty,
                notional,
                time_in_force,
                limit_price,
            };

            info!(
                "[EXECUTION] Submitting order to exchange {} for {}",
                exchange.name(),
                req.symbol
            );

            match exchange.submit_order(api_req).await {
                Ok(res) => {
                    info!(
                        "[SUCCESS] Order Placed: id={} status={}",
                        res.id, res.status
                    );

                    if order.action == "buy" {
                        // IMPORTANT: Always calculate TP/SL from actual entry price
                        // Don't use req.stop_loss/take_profit as those may be stale
                        let (tp_pct, sl_pct) = config.get_symbol_params(&req.symbol);
                        let stop_loss = estimated_price * (1.0 - sl_pct / 100.0);
                        let take_profit = estimated_price * (1.0 + tp_pct / 100.0);

                        info!("[EXECUTION] TP/SL from entry ${:.8}: TP=${:.8} (+{:.2}%), SL=${:.8} (-{:.2}%)",
                                      estimated_price, take_profit, tp_pct, stop_loss, sl_pct);

                        if matches!(order_type_enum, ExOrderType::Limit) {
                            let pending = crate::services::position_monitor::PendingOrder {
                                order_id: res.id.clone(),
                                symbol: req.symbol.clone(),
                                side: "buy".to_string(),
                                limit_price: estimated_price,
                                qty: order.qty,
                                created_at: chrono::Utc::now().to_rfc3339(),
                                stop_loss: Some(stop_loss),
                                take_profit: Some(take_profit),
                                last_check_time: None,
                            };
                            tracker.add_pending_order(pending);
                        } else {
                            let position_info = PositionInfo {
                                symbol: req.symbol.clone(),
                                entry_price: estimated_price,
                                qty: order.qty,
                                stop_loss,
                                take_profit,
                                entry_time: chrono::Utc::now().to_rfc3339(),
                                side: "buy".to_string(),
                                is_closing: false,
                                open_order_id: None,
                                last_recreate_attempt: None,
                                recreate_attempts: 0,
                                highest_price: estimated_price,
                                trailing_stop_active: false,
                                trailing_stop_price: stop_loss,
                            };
                            tracker.add_position(position_info);
                        }
                    }

                    let report = ExecutionReport {
                        symbol: req.symbol,
                        order_id: res.id,
                        status: res.status,
                        side: order.action.clone(),
                        price: Some(estimated_price),
                        qty: Some(order.qty),
                    };

                    bus.publish(Event::Execution(report)).ok();
                }
                Err(e) => error!("[FAILED] Order Submission: {}", e),
            }
        } else {
            info!("[EXECUTION] Invalid action '{}'", order.action);
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
