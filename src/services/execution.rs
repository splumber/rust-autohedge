use tracing::{info, error};
use crate::bus::EventBus;
use crate::events::{Event, OrderRequest, ExecutionReport};
use crate::data::alpaca::AlpacaClient;
use crate::llm::LLMQueue;
use crate::agents::{Agent, execution::ExecutionAgent};
use crate::config::AppConfig;
use crate::services::position_monitor::{PositionTracker, PositionInfo};

pub struct ExecutionEngine {
    event_bus: EventBus,
    alpaca: AlpacaClient,
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
    pub fn new(event_bus: EventBus, alpaca: AlpacaClient, llm: LLMQueue, config: AppConfig, tracker: PositionTracker) -> Self {
        Self {
            event_bus,
            alpaca,
            llm,
            config,
            tracker,
        }
    }

    pub async fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let alpaca_clone = self.alpaca.clone();
        let llm_clone = self.llm.clone();
        let bus_clone = self.event_bus.clone();
        let config_clone = self.config.clone();
        let tracker_clone = self.tracker.clone();

        tokio::spawn(async move {
            info!("⚡ Execution Engine Started");
            info!("[EXECUTION] Mode: {} | MinOrder=${:.2} MaxOrder=${:.2}", config_clone.trading_mode, config_clone.min_order_amount, config_clone.max_order_amount);
            while let Ok(event) = rx.recv().await {
                if let Event::Order(req) = event {
                    info!("[EXECUTION] Received OrderRequest: symbol={} action={} order_type={} limit_price={:?} sl={:?} tp={:?}",
                          req.symbol, req.action, req.order_type, req.limit_price, req.stop_loss, req.take_profit);

                    let alpaca = alpaca_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let config = config_clone.clone();
                    let tracker = tracker_clone.clone();

                    tokio::spawn(async move {
                        Self::execute_order(req, alpaca, llm, bus, config, tracker).await;
                    });
                }
            }
            info!("[EXECUTION] Event loop ended (channel closed)");
        });
    }

    async fn execute_order(req: OrderRequest, alpaca: AlpacaClient, llm: LLMQueue, bus: EventBus, config: AppConfig, tracker: PositionTracker) {
        let is_crypto = config.trading_mode.to_lowercase() == "crypto";
        info!("[EXECUTION] Begin execute_order: symbol={} action={} (crypto={})", req.symbol, req.action, is_crypto);

        // Handle sell orders directly (from Position Monitor)
        if req.action == "sell" {
            info!("[EXECUTION] SELL path (monitor-triggered) for {}", req.symbol);

            // Get current price for the sell
            let history = alpaca.market_store.get_quote_history(&req.symbol);
            info!("[EXECUTION] Market history size for {}: {}", req.symbol, history.len());

            let estimated_price = if let Some(latest) = history.last() {
                latest.get("bp").and_then(|c| c.as_f64()).unwrap_or(0.0)
            } else {
                0.0
            };

            info!("[EXECUTION] Estimated SELL price for {}: ${:.8}", req.symbol, estimated_price);

            if estimated_price == 0.0 {
                error!("[EXECUTION] Cannot estimate price for {}. No market data available.", req.symbol);
                return;
            }

            // Prefer local tracker qty; fall back to Alpaca positions as a safety net.
            let tracked_qty = tracker.get_position(&req.symbol).map(|p| p.qty);
            info!("[EXECUTION] Tracker qty for {}: {:?}", req.symbol, tracked_qty);

            let qty = if let Some(qty) = tracked_qty {
                qty
            } else {
                info!("[EXECUTION] Tracker missing qty for {}. Fetching positions from Alpaca...", req.symbol);
                match alpaca.get_positions().await {
                    Ok(positions) => {
                        info!("[EXECUTION] Alpaca returned {} positions", positions.len());
                        let position = positions.iter().find(|p| {
                            p.get("symbol").and_then(|v| v.as_str()) == Some(&req.symbol)
                        });

                        if let Some(pos) = position {
                            let qty = pos.get("qty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            info!("[EXECUTION] Alpaca position qty for {}: {:.8}", req.symbol, qty);
                            qty
                        } else {
                            info!("[EXECUTION] No Alpaca position found for {}", req.symbol);
                            0.0
                        }
                    }
                    Err(e) => {
                        error!("[EXECUTION] Failed to fetch positions from Alpaca for sell {}: {}", req.symbol, e);
                        0.0
                    }
                }
            };

            if qty <= 0.0 {
                error!("[EXECUTION] No quantity found for {} position", req.symbol);
                return;
            }

            let time_in_force = if is_crypto { "gtc".to_string() } else { "day".to_string() };
            info!("[EXECUTION] time_in_force for {} sell: {}", req.symbol, time_in_force);

            let api_req = crate::data::alpaca::OrderRequest {
                symbol: req.symbol.clone(),
                qty: Some(qty.to_string()),
                notional: None,
                side: "sell".to_string(),
                type_: "market".to_string(),
                time_in_force,
            };

            info!("[ORDER] Submitting SELL: qty={:.8} symbol={} est_price=${:.8} est_value=${:.2}",
                  qty, req.symbol, estimated_price, qty * estimated_price);

            match alpaca.submit_order(api_req, &config.trading_mode).await {
                Ok(res) => {
                    info!("[SUCCESS] SELL Order Placed: {:?}", res);

                    tracker.remove_position(&req.symbol);

                    let report = ExecutionReport {
                        symbol: req.symbol,
                        order_id: "unknown".to_string(),
                        status: "new".to_string(),
                        price: Some(estimated_price),
                        qty: Some(qty),
                    };
                    info!("[EXECUTION] Publishing ExecutionReport for SELL {}", report.symbol);
                    bus.publish(Event::Execution(report)).ok();
                }
                Err(e) => error!("[FAILED] SELL Order Submission: {}", e),
            }

            return;
        }

        // Handle buy orders (original logic with ExecutionAgent)
        info!("[EXECUTION] BUY path (agent-driven) for {}", req.symbol);

        let execution_agent = ExecutionAgent;
        let exec_input = format!("Symbol: {}\nRisk Analysis: Approved\nAction: Create Order JSON", req.symbol);
        info!("[EXECUTION] Calling ExecutionAgent for {}", req.symbol);

        let order_response = match execution_agent.run_high_priority(&exec_input, &llm).await {
            Ok(res) => res,
            Err(e) => {
                error!("Execution Agent Failed: {}", e);
                return;
            }
        };

        info!("[EXECUTION] Agent Output (raw) for {}: {}", req.symbol, order_response);

        let json_str = Self::extract_json(&order_response).unwrap_or(&order_response);
        info!("[EXECUTION] Agent Output (json_str) for {}: {}", req.symbol, json_str);

        match serde_json::from_str::<ExecutionOutput>(json_str) {
            Ok(mut order) => {
                info!("[EXECUTION] Parsed agent output for {} => action={} qty={:.8} order_type={}",
                      req.symbol, order.action, order.qty, order.order_type);

                if order.action == "buy" || order.action == "sell" {
                    // Hard Risk Limit Check
                    let history = alpaca.market_store.get_quote_history(&req.symbol);
                    info!("[EXECUTION] Market history size for {}: {}", req.symbol, history.len());

                    let estimated_price = if let Some(latest) = history.last() {
                        latest.get("bp").and_then(|c| c.as_f64()).unwrap_or(0.0)
                    } else {
                        0.0
                    };

                    info!("[EXECUTION] Estimated price for {}: ${:.8}", req.symbol, estimated_price);

                    if estimated_price == 0.0 {
                        error!("[EXECUTION] Cannot estimate price for {}. No market data available.", req.symbol);
                        return;
                    }

                    // For stocks, qty-based orders are fine. For crypto, notional orders reliably meet Alpaca's minimum order rules.
                    let is_crypto = config.trading_mode.to_lowercase() == "crypto";

                    // Estimate value from agent qty; tighten to min/max via config.
                    let mut estimated_value = order.qty * estimated_price;
                    info!("[EXECUTION] Initial sizing for {} => qty={:.8} est_value=${:.2}", req.symbol, order.qty, estimated_value);

                    if estimated_value < config.min_order_amount {
                        info!("[RISK] Order value ${:.2} is below minimum ${:.2}. Adjusting.", estimated_value, config.min_order_amount);
                        estimated_value = config.min_order_amount;
                        order.qty = estimated_value / estimated_price;
                        info!("[RISK] Adjusted qty for min order => qty={:.8} est_value=${:.2}", order.qty, estimated_value);
                    }

                    if estimated_value > config.max_order_amount {
                        info!("[RISK] Order value ${:.2} exceeds limit ${:.2}. Capping.", estimated_value, config.max_order_amount);
                        estimated_value = config.max_order_amount;
                        order.qty = estimated_value / estimated_price;
                        info!("[RISK] Adjusted qty for max cap => qty={:.8} est_value=${:.2}", order.qty, estimated_value);
                    }

                    info!("[ORDER] Submitting: action={} qty={:.8} symbol={} est_value=${:.2} order_type={}",
                          order.action, order.qty, req.symbol, estimated_value, order.order_type);

                    let time_in_force = if is_crypto { "gtc".to_string() } else { "day".to_string() };
                    info!("[EXECUTION] time_in_force for {} {}: {}", req.symbol, order.action, time_in_force);

                    // Build API request: crypto uses notional where possible (buy) to satisfy min cost basis.
                    let (qty, notional) = if is_crypto && order.action == "buy" {
                        info!("[EXECUTION] Crypto BUY => using notional=${:.2} (qty omitted)", estimated_value);
                        (None, Some(format!("{:.2}", estimated_value)))
                    } else {
                        info!("[EXECUTION] Using qty={:.8} (notional omitted)", order.qty);
                        (Some(order.qty.to_string()), None)
                    };

                    let api_req = crate::data::alpaca::OrderRequest {
                        symbol: req.symbol.clone(),
                        qty,
                        notional,
                        side: order.action.clone(),
                        type_: order.order_type,
                        time_in_force,
                    };

                    info!("[EXECUTION] Submitting order to Alpaca for {} (side={})", req.symbol, api_req.side);

                    match alpaca.submit_order(api_req, &config.trading_mode).await {
                        Ok(res) => {
                            info!("[SUCCESS] Order Placed: {:?}", res);

                            if order.action == "buy" {
                                let stop_loss = req.stop_loss.unwrap_or(estimated_price * 0.995); // default -0.5%
                                let take_profit = req.take_profit.unwrap_or(estimated_price * 1.01); // default +1%
                                info!("[EXECUTION] Tracking new position for {} => entry=${:.8} qty≈{:.8} sl={:.8} tp={:.8}",
                                      req.symbol, estimated_price, order.qty, stop_loss, take_profit);

                                let position_info = PositionInfo {
                                    symbol: req.symbol.clone(),
                                    entry_price: estimated_price,
                                    // For notional orders, we don't know filled qty yet; keep the computed qty as an estimate.
                                    qty: order.qty,
                                    stop_loss,
                                    take_profit,
                                    entry_time: chrono::Utc::now().to_rfc3339(),
                                    side: "buy".to_string(),
                                    is_closing: false,
                                };

                                tracker.add_position(position_info);
                            }

                            let report = ExecutionReport {
                                symbol: req.symbol,
                                order_id: "unknown".to_string(),
                                status: "new".to_string(),
                                price: Some(estimated_price),
                                qty: Some(order.qty),
                            };
                            info!("[EXECUTION] Publishing ExecutionReport for {}", report.symbol);
                            bus.publish(Event::Execution(report)).ok();
                        },
                        Err(e) => error!("[FAILED] Order Submission: {}", e),
                    }
                } else {
                    info!("[EXECUTION] Invalid action '{}'", order.action);
                }
            },
            Err(e) => {
                error!("[EXECUTION] JSON Parse Error: {}", e);
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
