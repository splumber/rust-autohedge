use tracing::{info, error};
use crate::bus::EventBus;
use crate::events::{Event, AnalysisSignal, OrderRequest};
use crate::llm::LLMQueue;
use crate::agents::{Agent, risk::RiskAgent};
use crate::config::AppConfig;
use std::sync::Arc;
use crate::exchange::traits::TradingApi;

pub struct RiskEngine {
    event_bus: EventBus,
    exchange: Arc<dyn TradingApi>,
    llm: LLMQueue,
    config: AppConfig,
}

impl RiskEngine {
    pub fn new(event_bus: EventBus, exchange: Arc<dyn TradingApi>, llm: LLMQueue, config: AppConfig) -> Self {
        Self {
            event_bus,
            exchange,
            llm,
            config,
        }
    }

    pub async fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let exchange_clone = self.exchange.clone();
        let llm_clone = self.llm.clone();
        let bus_clone = self.event_bus.clone();
        let config_clone = self.config.clone();

        tokio::spawn(async move {
            info!("🛡️ Risk Engine Started");
            while let Ok(event) = rx.recv().await {
                if let Event::Signal(signal) = event {
                    let exchange = exchange_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let config = config_clone.clone();

                    tokio::spawn(async move {
                        Self::assess_risk(signal, exchange, llm, bus, config).await;
                    });
                }
            }
        });
    }

    async fn assess_risk(signal: AnalysisSignal, exchange: Arc<dyn TradingApi>, llm: LLMQueue, bus: EventBus, _config: AppConfig) {
        // HFT Fast Path
        if signal.thesis.starts_with("HFT") {
            // Parse TP/SL from market_context "tp=..., sl=..."
            let mut stop_loss = None;
            let mut take_profit = None;
            
            for part in signal.market_context.split(',') {
                let part = part.trim();
                if part.starts_with("tp=") {
                    if let Ok(val) = part["tp=".len()..].parse::<f64>() {
                        take_profit = Some(val);
                    }
                } else if part.starts_with("sl=") {
                    if let Ok(val) = part["sl=".len()..].parse::<f64>() {
                        stop_loss = Some(val);
                    }
                }
            }

            info!("🛡️ [RISK] HFT Fast-Approve: {} (SL: {:?}, TP: {:?})", signal.symbol, stop_loss, take_profit);

            let order_req = OrderRequest {
                 symbol: signal.symbol.clone(),
                 action: signal.signal.clone(),
                 qty: 0.0, // Execution Agent will determine quantity
                 order_type: "hft_buy".to_string(), // Signal for fast execution
                 limit_price: None, 
                 stop_loss,
                 take_profit,
            };
            
            bus.publish(Event::Order(order_req)).ok();
            return;
        }

        // Fetch Account
        let account = match exchange.get_account().await {
            Ok(acc) => acc,
            Err(e) => {
                error!("❌ Risk: Failed to fetch account for {}: {}", signal.symbol, e);
                return;
            }
        };

        let risk_agent = RiskAgent;
        let risk_input = format!(
            "Asset: {}\nAccount Cash: {:?}\nPortfolio Value: {:?}\nThesis: {}\nQuant: N/A", // Simplifying input for now, Strategy signal could include Quant output
            signal.symbol,
            account.cash,
            account.portfolio_value,
            signal.thesis
        );

        let risk_response = match risk_agent.run_high_priority(&risk_input, &llm).await {
             Ok(res) => res,
             Err(e) => {
                 error!("❌ Risk Agent Failed: {}", e);
                 return;
             }
        };

        if !risk_response.to_lowercase().contains("approved") && !risk_response.to_lowercase().contains("true") {
             info!("🛡️ [RISK] Rejected trade for {}: {}", signal.symbol, risk_response);
             return;
        }
        
        // Parse risk response to extract stop_loss and take_profit
        let (stop_loss, take_profit) = Self::parse_risk_parameters(&risk_response);
        
        info!("🛡️ [RISK] Approved: {} (SL: {:?}, TP: {:?})", signal.symbol, stop_loss, take_profit);

        // Publish Order Request with risk parameters
        let order_req = OrderRequest {
             symbol: signal.symbol.clone(),
             action: signal.signal.clone(), // "buy" or "sell"
             qty: 0.0, // Execution Agent will determine quantity
             order_type: "market".to_string(),
             limit_price: None,
             stop_loss,
             take_profit,
        };
        
        bus.publish(Event::Order(order_req)).ok();
    }
    
    fn parse_risk_parameters(risk_response: &str) -> (Option<f64>, Option<f64>) {
        // Try to extract JSON
        let json_str = if let Some(start) = risk_response.find('{') {
            if let Some(end) = risk_response.rfind('}') {
                &risk_response[start..=end]
            } else {
                risk_response
            }
        } else {
            risk_response
        };
        
        // Attempt to parse JSON
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            let stop_loss = json.get("stop_loss")
                .and_then(|v| v.as_f64());
            let take_profit = json.get("take_profit")
                .and_then(|v| v.as_f64());
            
            return (stop_loss, take_profit);
        }
        
        (None, None)
    }
}
