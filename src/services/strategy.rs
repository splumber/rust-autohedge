use tracing::{info, error, warn};
use crate::bus::EventBus;
use crate::events::{Event, MarketEvent, AnalysisSignal};
use crate::data::store::{MarketStore, Quote};
use crate::llm::LLMQueue;
use crate::agents::{Agent, director::DirectorAgent, quant::QuantAgent};
use crate::config::AppConfig;
use std::collections::VecDeque;
use std::sync::Arc;
use dashmap::DashMap;

#[derive(Clone)]
struct SymbolCooldown {
    quotes_remaining: usize,
}

#[derive(Clone, Default)]
struct HftSymbolState {
    quotes_since_eval: usize,
    last_mid: Option<f64>,
    mids: VecDeque<f64>,
}

#[derive(Clone, Default)]
struct HybridGateState {
    quotes_until_refresh: usize,
    cooldown_quotes_remaining: usize,
    allowed: bool,
    last_reason: Option<String>,
}

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

        // Cooldown tracking for LLM mode: symbol -> quotes_remaining
        let cooldowns: Arc<DashMap<String, SymbolCooldown>> = Arc::new(DashMap::new());

        // Per-symbol state for HFT mode
        let hft_state: Arc<DashMap<String, HftSymbolState>> = Arc::new(DashMap::new());

        // Per-symbol gate state for HYBRID mode
        let hybrid_gate: Arc<DashMap<String, HybridGateState>> = Arc::new(DashMap::new());

        tokio::spawn(async move {
            info!("🧠 Strategy Engine Started (mode: {})", config_clone.strategy_mode);
            while let Ok(event) = rx.recv().await {
                if let Event::Market(market_event) = event {
                    let (symbol, bid, ask) = match &market_event {
                        MarketEvent::Quote { symbol, bid, ask, .. } => (symbol.clone(), *bid, *ask),
                        MarketEvent::Trade { symbol, price, .. } => (symbol.clone(), *price, *price),
                    };

                    let mode = config_clone.strategy_mode.to_lowercase();

                    if mode == "hft" {
                        let bus = bus_clone.clone();
                        let tracker = hft_state.clone();
                        let config = config_clone.clone();
                        tokio::spawn(async move {
                            Self::evaluate_hft(symbol, bid, ask, bus, tracker, config).await;
                        });
                        continue;
                    }

                    if mode == "hybrid" {
                        let bus = bus_clone.clone();
                        let config = config_clone.clone();
                        let store = store_clone.clone();
                        let llm = llm_clone.clone();
                        let hft_tracker = hft_state.clone();
                        let gate = hybrid_gate.clone();
                        tokio::spawn(async move {
                            Self::evaluate_hybrid(symbol, bid, ask, bus, store, llm, hft_tracker, gate, config).await;
                        });
                        continue;
                    }

                    // Default: LLM pipeline ("llm" or anything else)

                    // Check cooldown status
                    if let Some(mut cooldown) = cooldowns.get_mut(&symbol) {
                        if cooldown.quotes_remaining > 0 {
                            cooldown.quotes_remaining -= 1;
                            if cooldown.quotes_remaining == 0 {
                                info!("⏰ [COOLDOWN] {} cooldown expired. Ready for analysis.", symbol);
                                // DashMap doesn't need explicit remove here if we just check > 0
                                // But to clean up memory we can remove.
                                // However, get_mut holds a lock shard.
                                // We can't remove while holding a reference.
                                // We can just set to 0.
                            }
                            // drop(cooldown) happens automatically
                            continue;
                        }
                    }
                    // Cleanup expired cooldowns lazily or just leave them as 0.
                    // Or use remove_if.
                    if let Some(cooldown) = cooldowns.get(&symbol) {
                         if cooldown.quotes_remaining == 0 {
                             cooldowns.remove(&symbol);
                         }
                    }

                    // Warm-up Check
                    let history = store_clone.get_quote_history(&symbol);
                    if history.len() < config_clone.warmup_count {
                        continue;
                    }

                    // Spawn Analysis Task (Parallel)
                    let store = store_clone.clone();
                    let llm = llm_clone.clone();
                    let bus = bus_clone.clone();
                    let symbol_clone = symbol.clone();
                    let cooldowns_clone = cooldowns.clone();
                    let config = config_clone.clone();

                    tokio::spawn(async move {
                        Self::analyze_symbol_llm(symbol_clone, store, llm, bus, cooldowns_clone, config).await;
                    });
                }
            }
            error!("❌ Strategy Engine loop terminated");
        });
    }

    async fn analyze_symbol_llm(
        symbol: String,
        store: MarketStore,
        llm: LLMQueue,
        bus: EventBus,
        cooldowns: Arc<DashMap<String, SymbolCooldown>>,
        config: AppConfig,
    ) {
        // Prepare Data
        let history = store.get_quote_history(&symbol);
        let news = store.get_latest_news();
        let market_data_str = Self::format_quote_history_table(&history);

        // News Summary
        let news_summary = if news.is_empty() {
            "No recent news.".to_string()
        } else {
            let headlines: Vec<String> = news
                .iter()
                .take(5)
                .filter_map(|n| n.get("headline").and_then(|h| h.as_str()).map(|s| s.to_string()))
                .collect();
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
        if lower_resp.contains("no_trade")
            || lower_resp.contains("no trade")
            || (!lower_resp.contains("trade") && !lower_resp.contains("opportunity"))
        {
            // Set cooldown: wait for configured number of quotes before analyzing this symbol again
            cooldowns.insert(
                symbol.clone(),
                SymbolCooldown {
                    quotes_remaining: config.no_trade_cooldown_quotes,
                },
            );

            warn!(
                "🔴 [STRATEGY] No trade opportunity for {}. Cooldown: {} quotes.",
                symbol, config.no_trade_cooldown_quotes
            );
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
            signal: "buy".to_string(),
            confidence: 0.0,
            thesis: director_response,
            market_context: combined_data,
        };

        bus.publish(Event::Signal(signal)).ok();
    }

    async fn evaluate_hft(
        symbol: String,
        bid: f64,
        ask: f64,
        bus: EventBus,
        state: Arc<DashMap<String, HftSymbolState>>,
        config: AppConfig,
    ) {
        if bid <= 0.0 || ask <= 0.0 || ask < bid {
            if config.chatter_level.to_lowercase() == "verbose" {
                warn!("[HFT] Skip {}: invalid quote bid={} ask={}", symbol, bid, ask);
            }
            return;
        }

        let mid = (bid + ask) / 2.0;
        let spread_bps = ((ask - bid) / mid) * 10_000.0;
        if spread_bps > config.hft.max_spread_bps {
            if config.chatter_level.to_lowercase() == "verbose" {
                info!("[HFT] Skip {}: spread_bps={:.2} > max_spread_bps={:.2} (bid={:.8} ask={:.8})",
                      symbol, spread_bps, config.hft.max_spread_bps, bid, ask);
            }
            return;
        }

        let mut entry = state.entry(symbol.clone()).or_insert_with(|| HftSymbolState {
            quotes_since_eval: 0,
            last_mid: None,
            mids: VecDeque::with_capacity(64),
        });

        entry.quotes_since_eval += 1;
        entry.mids.push_back(mid);
        while entry.mids.len() > 30 {
            entry.mids.pop_front();
        }

        if entry.quotes_since_eval < config.hft.evaluate_every_quotes {
            if config.chatter_level.to_lowercase() == "verbose" {
                info!("[HFT] Debounce {}: {}/{} quotes collected (mid={:.8})",
                      symbol, entry.quotes_since_eval, config.hft.evaluate_every_quotes, mid);
            }
            entry.last_mid = Some(mid);
            return;
        }
        entry.quotes_since_eval = 0;

        // Simple momentum edge: compare current mid to mid N steps back.
        let lookback = 10usize.min(entry.mids.len().saturating_sub(1));
        if lookback == 0 {
            if config.chatter_level.to_lowercase() == "verbose" {
                info!("[HFT] Skip {}: insufficient history for lookback", symbol);
            }
            entry.last_mid = Some(mid);
            return;
        }
        let past = entry.mids.get(entry.mids.len() - 1 - lookback).copied().unwrap_or(mid);
        let edge_bps = ((mid - past) / past) * 10_000.0;

        entry.last_mid = Some(mid);
        // drop(entry); // DashMap RefMut is dropped here

        if edge_bps < config.hft.min_edge_bps {
            if config.chatter_level.to_lowercase() == "verbose" {
                info!("[HFT] Skip {}: edge_bps={:.2} < min_edge_bps={:.2} (mid={:.8} past={:.8})",
                      symbol, edge_bps, config.hft.min_edge_bps, mid, past);
            }
            return;
        }

        // If momentum is positive and spread is acceptable, emit a buy signal.
        let tp = mid * (1.0 + config.hft.take_profit_bps / 10_000.0);
        let sl = mid * (1.0 - config.hft.stop_loss_bps / 10_000.0);

        // This is the key "when HFT will buy" log.
        // - In normal: only log on entry.
        // - In verbose: include more details.
        if config.chatter_level.to_lowercase() != "low" {
            info!("[HFT] BUY trigger {}: edge_bps={:.2} >= min_edge_bps={:.2}, spread_bps={:.2} <= max_spread_bps={:.2} | entry(mid)={:.8} tp={:.8} sl={:.8}",
                  symbol, edge_bps, config.hft.min_edge_bps, spread_bps, config.hft.max_spread_bps, mid, tp, sl);
        }

        let thesis = format!(
            "HFT momentum: edge_bps={:.2}, spread_bps={:.2}, mid={:.8}, past={:.8}",
            edge_bps, spread_bps, mid, past
        );

        let signal = AnalysisSignal {
            symbol,
            signal: "buy".to_string(),
            confidence: 1.0,
            thesis: thesis.clone(),
            market_context: format!("tp={:.8}, sl={:.8}", tp, sl),
        };

        bus.publish(Event::Signal(signal)).ok();
    }

    async fn evaluate_hybrid(
        symbol: String,
        bid: f64,
        ask: f64,
        bus: EventBus,
        store: MarketStore,
        llm: LLMQueue,
        hft_state: Arc<DashMap<String, HftSymbolState>>,
        gate: Arc<DashMap<String, HybridGateState>>,
        config: AppConfig,
    ) {
        if bid <= 0.0 || ask <= 0.0 || ask < bid {
            if config.chatter_level.to_lowercase() == "verbose" {
                warn!("[HYBRID] Skip {}: invalid quote bid={} ask={}", symbol, bid, ask);
            }
            return;
        }

        // Gate bookkeeping (quote based)
        let mut should_refresh = false;
        let mut currently_allowed;

        {
            let mut entry = gate.entry(symbol.clone()).or_insert_with(|| HybridGateState {
                quotes_until_refresh: config.hybrid.gate_refresh_quotes,
                cooldown_quotes_remaining: 0,
                allowed: true,
                last_reason: None,
            });

            if entry.cooldown_quotes_remaining > 0 {
                entry.cooldown_quotes_remaining = entry.cooldown_quotes_remaining.saturating_sub(1);
                entry.allowed = false;
            }

            if entry.quotes_until_refresh > 0 {
                entry.quotes_until_refresh = entry.quotes_until_refresh.saturating_sub(1);
            }

            if entry.quotes_until_refresh == 0 && entry.cooldown_quotes_remaining == 0 {
                should_refresh = true;
                entry.quotes_until_refresh = config.hybrid.gate_refresh_quotes;
            }

            currently_allowed = entry.allowed && entry.cooldown_quotes_remaining == 0;

            if !currently_allowed && config.chatter_level.to_lowercase() == "verbose" {
                info!("[HYBRID] Gate closed for {} (cooldown_remaining={}, quotes_until_refresh={})",
                      symbol, entry.cooldown_quotes_remaining, entry.quotes_until_refresh);
            }
        }

        if should_refresh {
            let history = store.get_quote_history(&symbol);
            if history.len() >= config.warmup_count {
                if config.chatter_level.to_lowercase() != "low" {
                    info!("[HYBRID] Refreshing LLM gate for {} (history_len={})", symbol, history.len());
                }

                let combined_data = Self::format_quote_history_table(&history);
                let director = DirectorAgent;
                let director_input = format!("Symbol: {}, Market Context: {}", symbol, combined_data);

                match director.run(&director_input, &llm).await {
                    Ok(resp) => {
                        let lower = resp.to_lowercase();
                        let allowed = !(lower.contains("no_trade")
                            || lower.contains("no trade")
                            || (!lower.contains("trade") && !lower.contains("opportunity")));

                        let mut entry = gate.entry(symbol.clone()).or_default();
                        entry.allowed = allowed;
                        entry.last_reason = Some(resp.clone());

                        if !allowed {
                            entry.cooldown_quotes_remaining = config.hybrid.no_trade_cooldown_quotes;
                            warn!("[HYBRID] Gate CLOSED for {} by director. Cooldown {} quotes.", symbol, config.hybrid.no_trade_cooldown_quotes);
                            if config.chatter_level.to_lowercase() == "verbose" {
                                warn!("[HYBRID] Director response (no_trade) for {}: {}", symbol, resp);
                            }
                        } else {
                            if config.chatter_level.to_lowercase() != "low" {
                                info!("[HYBRID] Gate OPEN for {} by director.", symbol);
                            }
                            if config.chatter_level.to_lowercase() == "verbose" {
                                info!("[HYBRID] Director response (allowed) for {}: {}", symbol, resp);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("[HYBRID] Director gate failed for {}: {} (keeping previous gate)", symbol, e);
                    }
                }
            } else if config.chatter_level.to_lowercase() == "verbose" {
                info!("[HYBRID] Skip gate refresh for {}: warmup not met (history_len={}, warmup={})",
                      symbol, history.len(), config.warmup_count);
            }
        }

        // Re-check gate after potential refresh
        {
            if let Some(s) = gate.get(&symbol) {
                currently_allowed = s.allowed && s.cooldown_quotes_remaining == 0;
            }
        }

        if !currently_allowed {
            return;
        }

        Self::evaluate_hft(symbol, bid, ask, bus, hft_state, config).await;
    }

    fn format_quote_history_table(history: &[Quote]) -> String {
        let mut table = String::from("Recent Quote History (Last 50 Quotes):\nTime | Bid | BidSz | Ask | AskSz\n");
        for quote in history {
            let t = &quote.timestamp;
            let bp = quote.bid_price;
            let bs = quote.bid_size;
            let ap = quote.ask_price;
            let as_ = quote.ask_size;

            let time_short = if t.len() > 11 { &t[11..23] } else { t };
            table.push_str(&format!(
                "{} | {:.8} | {:.8} | {:.8} | {:.8}\n",
                time_short, bp, bs, ap, as_
            ));
        }
        table
    }
}
