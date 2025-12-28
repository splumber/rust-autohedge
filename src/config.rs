use std::env;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub trading_mode: String,
    pub symbols: Vec<String>,
    pub history_limit: usize,
    pub warmup_count: usize,
    pub min_order_amount: f64,
    pub max_order_amount: f64,
    pub llm_queue_size: usize,
    pub llm_max_concurrent: usize,
    pub no_trade_cooldown_quotes: usize,

    // Strategy selection
    pub strategy_mode: String, // "llm" | "hft" | "hybrid"

    // HFT configuration knobs
    pub hft_evaluate_every_quotes: usize,
    pub hft_min_edge_bps: f64,
    pub hft_take_profit_bps: f64,
    pub hft_stop_loss_bps: f64,
    pub hft_max_spread_bps: f64,

    // Position monitoring
    pub exit_on_quotes: bool,

    // Hybrid mode: LLM gating for HFT
    pub hybrid_gate_refresh_quotes: usize,
    pub hybrid_no_trade_cooldown_quotes: usize,

    /// Controls how chatty runtime logging is: "low" | "normal" | "verbose"
    pub chatter_level: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let trading_mode = env::var("TRADING_MODE").unwrap_or_else(|_| "stocks".to_string());

        let is_crypto = trading_mode.to_lowercase() == "crypto";
        let default_symbol = if is_crypto { "BTC/USD" } else { "AAPL" };

        let symbols_env = env::var("TRADING_SYMBOLS").unwrap_or_else(|_| default_symbol.to_string());
        let symbols: Vec<String> = symbols_env.split(',').map(|s| s.trim().to_string()).collect();

        let history_limit = env::var("MARKET_HISTORY_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let warmup_count = env::var("WARMUP_MIN_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let llm_queue_size = env::var("LLM_QUEUE_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let llm_max_concurrent = env::var("LLM_MAX_CONCURRENT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);

        let min_order_amount = env::var("MIN_ORDER_AMOUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0); // Alpaca minimum is $10

        let max_order_amount = env::var("MAX_ORDER_AMOUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100.0);

        let no_trade_cooldown_quotes = env::var("NO_TRADE_COOLDOWN_QUOTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let strategy_mode = env::var("STRATEGY_MODE").unwrap_or_else(|_| "llm".to_string());

        let hft_evaluate_every_quotes = env::var("HFT_EVALUATE_EVERY_QUOTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let hft_min_edge_bps = env::var("HFT_MIN_EDGE_BPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(15.0);

        let hft_take_profit_bps = env::var("HFT_TAKE_PROFIT_BPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100.0); // 1.00%

        let hft_stop_loss_bps = env::var("HFT_STOP_LOSS_BPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50.0); // 0.50%

        let hft_max_spread_bps = env::var("HFT_MAX_SPREAD_BPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(25.0);

        let exit_on_quotes = env::var("EXIT_ON_QUOTES")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);

        let hybrid_gate_refresh_quotes = env::var("HYBRID_GATE_REFRESH_QUOTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);

        let hybrid_no_trade_cooldown_quotes = env::var("HYBRID_NO_TRADE_COOLDOWN_QUOTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);

        let chatter_level = env::var("CHATTER_LEVEL").unwrap_or_else(|_| "normal".to_string());

        Self {
            trading_mode,
            symbols,
            history_limit,
            warmup_count,
            min_order_amount,
            max_order_amount,
            llm_queue_size,
            llm_max_concurrent,
            no_trade_cooldown_quotes,
            strategy_mode,
            hft_evaluate_every_quotes,
            hft_min_edge_bps,
            hft_take_profit_bps,
            hft_stop_loss_bps,
            hft_max_spread_bps,
            exit_on_quotes,
            hybrid_gate_refresh_quotes,
            hybrid_no_trade_cooldown_quotes,
            chatter_level,
        }
    }
}
