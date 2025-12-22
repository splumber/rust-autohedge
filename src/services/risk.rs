use tracing::{info, error};
use crate::bus::EventBus;
use crate::events::{Event, AnalysisSignal, OrderRequest};
use crate::data::alpaca::AlpacaClient;
use crate::llm::LLMQueue;
use crate::agents::{Agent, risk::RiskAgent};
use crate::config::AppConfig;

pub struct RiskEngine {
    event_bus: EventBus,
    alpaca: AlpacaClient,
    llm: LLMQueue,
    config: AppConfig,
}

impl RiskEngine {
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
            info!("🛡️ Risk Engine Started");
            while let Ok(event) = rx.recv().await {
                if let Event::Signal(signal) = event {
                    let alpaca = alpaca_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let config = config_clone.clone();

                    tokio::spawn(async move {
                         Self::assess_risk(signal, alpaca, llm, bus, config).await;
                    });
                }
            }
        });
    }

    async fn assess_risk(signal: AnalysisSignal, alpaca: AlpacaClient, llm: LLMQueue, bus: EventBus, _config: AppConfig) {
        // Fetch Account
        let account = match alpaca.get_account().await {
            Ok(acc) => acc,
            Err(e) => {
                error!("❌ Risk: Failed to fetch account for {}: {}", signal.symbol, e);
                return;
            }
        };

        let risk_agent = RiskAgent;
        let risk_input = format!(
            "Asset: {}\nAccount Cash: {}\nPortfolio Value: {}\nThesis: {}\nQuant: N/A", // Simplifying input for now, Strategy signal could include Quant output
            signal.symbol, account.cash, account.portfolio_value, signal.thesis
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
        info!("🛡️ [RISK] Approved: {}", signal.symbol);

        // Publish Order Request (Pre-Execution)
        // Note: The actual quantity calculation usually happens in Execution Agent based on risk parameters.
        // However, our previous flow had Execution Agent decide the quantity.
        // We will stick to the previous flow: Risk approves -> Execution decides content.
        
        let order_req = OrderRequest {
             symbol: signal.symbol,
             action: "decide_in_execution".to_string(), // Execution Agent handles this
             qty: 0.0,
             order_type: "market".to_string(),
             limit_price: None,
        };

        // We need to pass the "Risk Analysis" text to Execution.
        // But our OrderRequest struct is rigid. 
        // Let's modify OrderRequest to optionally carry context or just let Execution run freely?
        // Actually, the previous pipeline passed `risk_response` to Execution.
        // To keep it clean, we should probably add `risk_analysis` to OrderRequest or make a new event.
        // For now, let's assume Execution has enough context or we repurpose fields.
        
        // Wait, the previous logic was:
        // Execution Agent -> Output JSON -> Check Hard Limit -> Submit.
        
        // So Risk Engine here mainly validates "Can we trade?". 
        // The actual sizing logic was done by Execution Agent + Hard Limit Check.
        
        bus.publish(Event::Order(order_req)).ok(); 
        
        // ISSUE: Validating Hard Limits requires knowing Qty, which comes from Execution Agent.
        // So the Hard Limit check must happen IN Execution Engine, not Risk Engine, or we need an intermediate step.
        // I will move Hard Limit check to Execution Engine as it was in `api.rs`.
    }
}
