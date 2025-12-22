use tracing::{info, error};
use crate::bus::EventBus;
use crate::events::{Event, OrderRequest, ExecutionReport}; // OrderRequest here acts as a "Trigger"
use crate::data::alpaca::AlpacaClient;
use crate::llm::LLMQueue;
use crate::agents::{Agent, execution::ExecutionAgent};
use crate::config::AppConfig;

pub struct ExecutionEngine {
    event_bus: EventBus,
    alpaca: AlpacaClient,
    llm: LLMQueue,
    config: AppConfig,
}

#[derive(serde::Deserialize)]
struct ExecutionOutput {
    action: String,
    qty: f64,
    order_type: String,
}

impl ExecutionEngine {
    pub fn new(event_bus: EventBus, alpaca: AlpacaClient, llm: LLMQueue, config: AppConfig) -> Self {
        Self {
            event_bus,
            alpaca,
            llm,
            config,
        }
    }

    pub async fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let alpaca_clone = self.alpaca.clone();
        let llm_clone = self.llm.clone();
        let bus_clone = self.event_bus.clone();
        let config_clone = self.config.clone();

        tokio::spawn(async move {
            info!("⚡ Execution Engine Started");
            while let Ok(event) = rx.recv().await {
                if let Event::Order(req) = event {
                    // This "OrderRequest" is essentially a "Risk Approved" trigger.
                    // We need to generate the JSON using ExecutionAgent.
                    
                    let alpaca = alpaca_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let config = config_clone.clone();

                    tokio::spawn(async move {
                         Self::execute_order(req, alpaca, llm, bus, config).await;
                    });
                }
            }
        });
    }

    async fn execute_order(req: OrderRequest, alpaca: AlpacaClient, llm: LLMQueue, bus: EventBus, config: AppConfig) {
        // Run Execution Agent
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

                    let estimated_value = order.qty * estimated_price;
                
                    if estimated_value > config.max_order_amount {
                         info!("⚠️ [RISK] Order value ${:.2} exceeds limit ${:.2}. Cap set.", estimated_value, config.max_order_amount);
                         if estimated_price > 0.0 {
                             order.qty = config.max_order_amount / estimated_price;
                             info!("⚠️ [RISK] Quantity reduced to {:.4}", order.qty);
                         }
                    }

                    info!("🚀 [ORDER] Submitting: {} {} {}", order.action, order.qty, req.symbol);

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
                             // Publish Report
                             let report = ExecutionReport {
                                 symbol: req.symbol,
                                 order_id: "unknown".to_string(), // Could parse res
                                 status: "new".to_string(),
                                 price: None,
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
