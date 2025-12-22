use tracing::{info, error};
use crate::bus::EventBus;
use crate::events::{Event, MarketEvent, AnalysisSignal};
use crate::data::store::MarketStore;
use crate::llm::LLMQueue;
use crate::agents::{Agent, director::DirectorAgent, quant::QuantAgent};
use crate::config::AppConfig;
use serde_json::Value;

pub struct StrategyEngine {
    event_bus: EventBus,
    market_store: MarketStore,
    llm: LLMQueue,
    config: AppConfig,
}

impl StrategyEngine {
    pub fn new(event_bus: EventBus, market_store: MarketStore, llm: LLMQueue, config: AppConfig) -> Self {
        Self {
            event_bus,
            market_store,
            llm,
            config,
        }
    }

    pub async fn start(&self) {
        let mut rx = self.event_bus.subscribe();
        let store_clone = self.market_store.clone();
        let llm_clone = self.llm.clone();
        let bus_clone = self.event_bus.clone();
        let config_clone = self.config.clone();

        tokio::spawn(async move {
            info!("🧠 Strategy Engine Started");
            while let Ok(event) = rx.recv().await {
                if let Event::Market(market_event) = event {
                    let symbol = match &market_event {
                        MarketEvent::Quote { symbol, .. } => symbol.clone(),
                        MarketEvent::Trade { symbol, .. } => symbol.clone(),
                    };

                    // Warm-up Check
                    let history = store_clone.get_quote_history(&symbol);
                    if history.len() < config_clone.warmup_count {
                        continue;
                    }

                    // Throttle/Debounce check could go here (omitted for now)

                    // Spawn Analysis Task (Parallel)
                    let store = store_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let symbol_clone = symbol.clone();

                    tokio::spawn(async move {
                         Self::analyze_symbol(symbol_clone, store, llm, bus).await;
                    });
                }
            }
            error!("❌ Strategy Engine loop terminated");
        });
    }

    async fn analyze_symbol(symbol: String, store: MarketStore, llm: LLMQueue, bus: EventBus) {
        // Prepare Data
        let history = store.get_quote_history(&symbol);
        let news = store.get_latest_news();
        let market_data_str = Self::format_quote_history_table(&history);
        
        // News Summary
        let news_summary = if news.is_empty() {
            "No recent news.".to_string()
        } else {
            let headlines: Vec<String> = news.iter().take(5).filter_map(|n| n.get("headline").and_then(|h| h.as_str()).map(|s| s.to_string())).collect();
            format!("Recent News: {:?}", headlines)
        };

        let combined_data = format!("{}\n{}", market_data_str, news_summary);
        
        // 1. Director
        let director = DirectorAgent;
        let director_input = format!("Symbol: {}, Market Context: {}", symbol, combined_data);
        
        let director_response = match director.run(&director_input, &llm).await {
            Ok(res) => res,
            Err(e) => {
                error!("❌ Director Failed for {}: {}", symbol, e);
                return;
            }
        };

        let lower_resp = director_response.to_lowercase();
        if lower_resp.contains("no_trade") || lower_resp.contains("no trade") || (!lower_resp.contains("trade") && !lower_resp.contains("opportunity")) {
            return;
        }

        info!("🟢 [STRATEGY] Opportunity found for {}! Running Quant...", symbol);

        // 2. Quant
        let quant = QuantAgent;
        let quant_input = format!("Thesis: {}\n\nMarket Data:\n{}", director_response, combined_data);
        
        let quant_response = match quant.run_high_priority(&quant_input, &llm).await {
            Ok(res) => res,
            Err(e) => {
                error!("❌ Quant Failed for {}: {}", symbol, e);
                return;
            }
        };

        info!("📈 [STRATEGY] Quant Analysis for {}: {}", symbol, quant_response);

        // Publish Signal
        let signal = AnalysisSignal {
            symbol: symbol.clone(),
            signal: "buy".to_string(), // Inferred from "Opportunity found"
            confidence: 0.0, // Could parse JSON, but keeping simple for now
            thesis: director_response,
            market_context: combined_data,
        };

        bus.publish(Event::Signal(signal)).ok();
    }

    fn format_quote_history_table(history: &[Value]) -> String {
        let mut table = String::from("Recent Quote History (Last 50 Quotes):\nTime | Bid | BidSz | Ask | AskSz\n");
        for quote in history {
            let t = quote.get("t").and_then(|v| v.as_str()).unwrap_or("?");
            let bp = quote.get("bp").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let bs = quote.get("bs").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ap = quote.get("ap").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let as_ = quote.get("as").and_then(|v| v.as_f64()).unwrap_or(0.0); 
            
            let time_short = if t.len() > 11 { &t[11..23] } else { t }; 
            table.push_str(&format!("{} | {:.8} | {:.8} | {:.8} | {:.8}\n", time_short, bp, bs, ap, as_));
        }
        table
    }
}
