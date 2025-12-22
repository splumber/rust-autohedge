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
        }
    }
}
