use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

#[derive(Clone, Debug, Deserialize)]
pub struct Defaults {
    pub take_profit_pct: f64,
    pub stop_loss_pct: f64,
    pub min_order_amount: f64,
    pub max_order_amount: f64,
    pub limit_order_expiration_days: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SymbolConfig {
    pub take_profit_pct: Option<f64>,
    pub stop_loss_pct: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HftConfig {
    pub evaluate_every_quotes: usize,
    pub min_edge_bps: f64,
    pub take_profit_bps: f64,
    pub stop_loss_bps: f64,
    pub max_spread_bps: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HybridConfig {
    pub gate_refresh_quotes: usize,
    pub no_trade_cooldown_quotes: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LlmConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub model: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AlpacaConfig {
    pub api_key: String,
    pub secret_key: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BinanceConfig {
    pub api_key: String,
    pub secret_key: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CoinbaseConfig {
    pub api_key: String,
    pub secret_key: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KrakenConfig {
    pub api_key: String,
    pub secret_key: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    pub trading_mode: String,
    pub exchange: String, // "alpaca", "binance", etc.
    pub symbols: Vec<String>,

    pub defaults: Defaults,
    pub symbol_overrides: Option<HashMap<String, SymbolConfig>>,

    pub history_limit: usize,
    pub warmup_count: usize,
    pub llm_queue_size: usize,
    pub llm_max_concurrent: usize,
    pub no_trade_cooldown_quotes: usize,
    pub strategy_mode: String,
    pub chatter_level: String,

    pub hft: HftConfig,
    pub hybrid: HybridConfig,
    pub llm: LlmConfig,
    pub alpaca: AlpacaConfig,
    pub binance: Option<BinanceConfig>,
    pub coinbase: Option<CoinbaseConfig>,
    pub kraken: Option<KrakenConfig>,

    pub exit_on_quotes: bool,
}

impl AppConfig {

    pub fn load() -> Self {
        let config_path = "config.yaml";
        let content = fs::read_to_string(config_path).expect("Failed to read config.yaml");

        // Strip BOM if present
        let content = content.strip_prefix("\u{feff}").unwrap_or(&content);

        let config: AppConfig = serde_yaml::from_str(content).expect("Failed to parse config.yaml");
        config
    }

    // Helper to get effective TP/SL for a symbol
    pub fn get_symbol_params(&self, symbol: &str) -> (f64, f64) {
        let mut tp = self.defaults.take_profit_pct;
        let mut sl = self.defaults.stop_loss_pct;

        if let Some(overrides) = &self.symbol_overrides {
            if let Some(sc) = overrides.get(symbol) {
                if let Some(v) = sc.take_profit_pct { tp = v; }
                if let Some(v) = sc.stop_loss_pct { sl = v; }
            }
        }
        (tp, sl)
    }
}
