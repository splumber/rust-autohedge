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
            while let Ok(event) = rx.recv().await {
                if let Event::Order(req) = event {
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
        });
    }

    async fn execute_order(req: OrderRequest, alpaca: AlpacaClient, llm: LLMQueue, bus: EventBus, config: AppConfig, tracker: PositionTracker) {
        // Handle sell orders directly (from Position Monitor)
        if req.action == "sell" {
            info!("🔻 [EXECUTION] Processing SELL order for {}", req.symbol);
            
            // Get current price for the sell
            let history = alpaca.market_store.get_quote_history(&req.symbol);
            let estimated_price = if let Some(latest) = history.last() {
                latest.get("bp").and_then(|c| c.as_f64()).unwrap_or(0.0)
            } else {
                0.0
            };
            
            if estimated_price == 0.0 {
                error!("❌ [EXECUTION] Cannot estimate price for {}. No market data available.", req.symbol);
                return;
            }
            
            // Get position to determine quantity
            match alpaca.get_positions().await {
                Ok(positions) => {
                    let position = positions.iter().find(|p| {
                        p.get("symbol").and_then(|v| v.as_str()) == Some(&req.symbol)
                    });
                    
                    if let Some(pos) = position {
                        let qty = pos.get("qty")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(0.0);
                        
                        if qty > 0.0 {
                            let time_in_force = if config.trading_mode.to_lowercase() == "crypto" {
                                "gtc".to_string()
                            } else {
                                "day".to_string()
                            };
                            
                            let api_req = crate::data::alpaca::OrderRequest {
                                symbol: req.symbol.clone(),
                                qty,
                                side: "sell".to_string(),
                                type_: "market".to_string(),
                                time_in_force,
                            };
                            
                            info!("🔻 [ORDER] Submitting SELL: {:.8} {} @ ${:.8}", qty, req.symbol, estimated_price);
                            
                            match alpaca.submit_order(api_req).await {
                                Ok(res) => {
                                    info!("✅ [SUCCESS] SELL Order Placed: {:?}", res);
                                    
                                    // Remove from position tracker
                                    tracker.remove_position(&req.symbol);
                                    
                                    let report = ExecutionReport {
                                        symbol: req.symbol,
                                        order_id: "unknown".to_string(),
                                        status: "new".to_string(),
                                        price: Some(estimated_price),
                                        qty: Some(qty),
                                    };
                                    bus.publish(Event::Execution(report)).ok();
                                },
                                Err(e) => error!("❌ [FAILED] SELL Order Submission: {}", e),
                            }
                        } else {
                            error!("❌ [EXECUTION] No quantity found for {} position", req.symbol);
                        }
                    } else {
                        error!("❌ [EXECUTION] No open position found for {}", req.symbol);
                    }
                },
                Err(e) => error!("❌ [EXECUTION] Failed to fetch positions: {}", e),
            }
            return;
        }
        
        // Handle buy orders (original logic with ExecutionAgent)
        let execution_agent = ExecutionAgent;
        let exec_input = format!("Symbol: {}\nRisk Analysis: Approved\nAction: Create Order JSON", req.symbol);
        
        let order_response = match execution_agent.run_high_priority(&exec_input, &llm).await {
            Ok(res) => res,
            Err(e) => {
                error!("❌ Execution Agent Failed: {}", e);
                return;
            }
        };

        info!("🤖 [EXECUTION] Agent Output: {}", order_response);

        let json_str = Self::extract_json(&order_response).unwrap_or(&order_response);

        match serde_json::from_str::<ExecutionOutput>(json_str) {
            Ok(mut order) => {
                if order.action == "buy" || order.action == "sell" {
                    // Hard Risk Limit Check
                    let history = alpaca.market_store.get_quote_history(&req.symbol);
                    let estimated_price = if let Some(latest) = history.last() {
                         latest.get("bp").and_then(|c| c.as_f64()).unwrap_or(0.0) 
                    } else {
                        0.0
                    };

                    if estimated_price == 0.0 {
                        error!("❌ [EXECUTION] Cannot estimate price for {}. No market data available.", req.symbol);
                        return;
                    }

                    let mut estimated_value = order.qty * estimated_price;

                    // Alpaca requires minimum order value (configurable, default $10)
                    if estimated_value < config.min_order_amount {
                        info!("⚠️ [RISK] Order value ${:.2} is below minimum ${:.2}. Adjusting quantity.", estimated_value, config.min_order_amount);
                        order.qty = config.min_order_amount / estimated_price;
                        estimated_value = order.qty * estimated_price;
                        info!("⚠️ [RISK] Quantity increased to {:.8} (value: ${:.2})", order.qty, estimated_value);
                    }

                    if estimated_value > config.max_order_amount {
                         info!("⚠️ [RISK] Order value ${:.2} exceeds limit ${:.2}. Cap set.", estimated_value, config.max_order_amount);
                         order.qty = config.max_order_amount / estimated_price;
                         estimated_value = order.qty * estimated_price;
                         info!("⚠️ [RISK] Quantity reduced to {:.8} (value: ${:.2})", order.qty, estimated_value);
                    }

                    info!("🚀 [ORDER] Submitting: {} {:.8} {} (Est. Value: ${:.2})", order.action, order.qty, req.symbol, estimated_value);

                    // Crypto requires "gtc" (Good Till Canceled), stocks can use "day"
                    let time_in_force = if config.trading_mode.to_lowercase() == "crypto" {
                        "gtc".to_string()
                    } else {
                        "day".to_string()
                    };

                    let api_req = crate::data::alpaca::OrderRequest {
                        symbol: req.symbol.clone(),
                        qty: order.qty,
                        side: order.action.clone(),
                        type_: order.order_type,
                        time_in_force,
                    };
                    
                    match alpaca.submit_order(api_req).await {
                        Ok(res) => {
                            info!("✅ [SUCCESS] Order Placed: {:?}", res);
                            
                            // Store position info for monitoring
                            if order.action == "buy" {
                                let stop_loss = req.stop_loss.unwrap_or(estimated_price * 0.95); // Default -5%
                                let take_profit = req.take_profit.unwrap_or(estimated_price * 1.10); // Default +10%
                                
                                let position_info = PositionInfo {
                                    symbol: req.symbol.clone(),
                                    entry_price: estimated_price,
                                    qty: order.qty,
                                    stop_loss,
                                    take_profit,
                                    entry_time: chrono::Utc::now().to_rfc3339(),
                                    side: "buy".to_string(),
                                };
                                
                                tracker.add_position(position_info);
                            }
                            
                             // Publish Report
                             let report = ExecutionReport {
                                 symbol: req.symbol,
                                 order_id: "unknown".to_string(), // Could parse res
                                 status: "new".to_string(),
                                 price: Some(estimated_price),
                                 qty: Some(order.qty),
                             };
                             bus.publish(Event::Execution(report)).ok();
                        },
                        Err(e) => error!("❌ [FAILED] Order Submission: {}", e),
                    }
                } else {
                     info!("⚠️ [EXECUTION] Invalid action '{}'", order.action);
                }
            },
            Err(e) => {
                error!("❌ [EXECUTION] JSON Parse Error: {}", e);
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
