//! Unit tests for configuration structures and parsing.

#[cfg(test)]
mod config_tests {
    use crate::config::*;

    // ============= MicroTradeConfig Tests =============

    #[test]
    fn test_micro_trade_config_default() {
        let config = MicroTradeConfig::default();

        assert_eq!(config.target_balance_pct, 0.05);
        assert_eq!(config.aggression_bps, 5.0);
        assert_eq!(config.min_order_interval_ms, 500);
        assert_eq!(config.account_cache_secs, 30);
        assert!(!config.use_llm_filter);
        assert!(config.limit_orders_expire_daily);
        assert_eq!(config.crypto_time_in_force, "gtc");
    }

    #[test]
    fn test_micro_trade_config_deserialize() {
        let yaml = r#"
target_balance_pct: 0.10
aggression_bps: 15.0
min_order_interval_ms: 250
account_cache_secs: 15
use_llm_filter: true
limit_orders_expire_daily: false
crypto_time_in_force: "ioc"
"#;
        let config: MicroTradeConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.target_balance_pct, 0.10);
        assert_eq!(config.aggression_bps, 15.0);
        assert_eq!(config.min_order_interval_ms, 250);
        assert!(config.use_llm_filter);
        assert!(!config.limit_orders_expire_daily);
        assert_eq!(config.crypto_time_in_force, "ioc");
    }

    #[test]
    fn test_micro_trade_config_defaults_in_deserialize() {
        // Missing optional fields should use defaults
        let yaml = r#"
target_balance_pct: 0.05
aggression_bps: 5.0
min_order_interval_ms: 500
account_cache_secs: 30
"#;
        let config: MicroTradeConfig = serde_yaml::from_str(yaml).unwrap();

        // These should have defaults
        assert!(!config.use_llm_filter);
        assert!(config.limit_orders_expire_daily);
        assert_eq!(config.crypto_time_in_force, "gtc");
    }

    // ============= Defaults Tests =============

    #[test]
    fn test_defaults_deserialize() {
        let yaml = r#"
take_profit_pct: 1.0
stop_loss_pct: 0.5
min_order_amount: 10.0
max_order_amount: 100.0
limit_order_expiration_days: 1
"#;
        let defaults: Defaults = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(defaults.take_profit_pct, 1.0);
        assert_eq!(defaults.stop_loss_pct, 0.5);
        assert_eq!(defaults.min_order_amount, 10.0);
        assert_eq!(defaults.max_order_amount, 100.0);
        assert_eq!(defaults.limit_order_expiration_days, Some(1));
    }

    #[test]
    fn test_defaults_no_expiration() {
        let yaml = r#"
take_profit_pct: 1.0
stop_loss_pct: 0.5
min_order_amount: 10.0
max_order_amount: 100.0
"#;
        let defaults: Defaults = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(defaults.limit_order_expiration_days, None);
    }

    // ============= SymbolConfig Tests =============

    #[test]
    fn test_symbol_config_full() {
        let yaml = r#"
take_profit_pct: 2.0
stop_loss_pct: 1.0
"#;
        let config: SymbolConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.take_profit_pct, Some(2.0));
        assert_eq!(config.stop_loss_pct, Some(1.0));
    }

    #[test]
    fn test_symbol_config_partial() {
        let yaml = r#"
take_profit_pct: 1.5
"#;
        let config: SymbolConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.take_profit_pct, Some(1.5));
        assert_eq!(config.stop_loss_pct, None);
    }

    // ============= HftConfig Tests =============

    #[test]
    fn test_hft_config_deserialize() {
        let yaml = r#"
evaluate_every_quotes: 5
min_edge_bps: 10.0
take_profit_bps: 50.0
stop_loss_bps: 25.0
max_spread_bps: 30.0
"#;
        let config: HftConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.evaluate_every_quotes, 5);
        assert_eq!(config.min_edge_bps, 10.0);
        assert_eq!(config.take_profit_bps, 50.0);
        assert_eq!(config.stop_loss_bps, 25.0);
        assert_eq!(config.max_spread_bps, 30.0);
    }

    // ============= HybridConfig Tests =============

    #[test]
    fn test_hybrid_config_deserialize() {
        let yaml = r#"
gate_refresh_quotes: 100
no_trade_cooldown_quotes: 50
"#;
        let config: HybridConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.gate_refresh_quotes, 100);
        assert_eq!(config.no_trade_cooldown_quotes, 50);
    }

    // ============= LlmConfig Tests =============

    #[test]
    fn test_llm_config_full() {
        let yaml = r#"
api_key: "sk-test123"
base_url: "https://api.openai.com/v1"
model: "gpt-4"
"#;
        let config: LlmConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.api_key, Some("sk-test123".to_string()));
        assert_eq!(
            config.base_url,
            Some("https://api.openai.com/v1".to_string())
        );
        assert_eq!(config.model, "gpt-4");
    }

    #[test]
    fn test_llm_config_local() {
        let yaml = r#"
api_key: null
base_url: "http://localhost:11434/v1"
model: "llama2"
"#;
        let config: LlmConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.api_key, None);
        assert_eq!(
            config.base_url,
            Some("http://localhost:11434/v1".to_string())
        );
    }

    // ============= Exchange Config Tests =============

    #[test]
    fn test_alpaca_config() {
        let yaml = r#"
api_key: "PKTEST123"
secret_key: "SECRET456"
base_url: "https://paper-api.alpaca.markets"
"#;
        let config: AlpacaConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.api_key, "PKTEST123");
        assert_eq!(config.secret_key, "SECRET456");
        assert_eq!(config.base_url, "https://paper-api.alpaca.markets");
    }

    #[test]
    fn test_binance_config() {
        let yaml = r#"
api_key: "BINANCE_KEY"
secret_key: "BINANCE_SECRET"
base_url: "https://api.binance.com"
"#;
        let config: BinanceConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.api_key, "BINANCE_KEY");
        assert_eq!(config.base_url, "https://api.binance.com");
    }

    #[test]
    fn test_coinbase_config() {
        let yaml = r#"
api_key: "CB_KEY"
secret_key: "CB_SECRET"
base_url: "https://api.coinbase.com"
"#;
        let config: CoinbaseConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.api_key, "CB_KEY");
    }

    #[test]
    fn test_kraken_config() {
        let yaml = r#"
api_key: "KRAKEN_KEY"
secret_key: "KRAKEN_SECRET"
base_url: "https://api.kraken.com"
"#;
        let config: KrakenConfig = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.api_key, "KRAKEN_KEY");
    }

    // ============= get_symbol_params Tests =============

    fn create_test_config() -> AppConfig {
        let yaml = r#"
trading_mode: "crypto"
exchange: "alpaca"
symbols:
  - "BTC/USD"
  - "ETH/USD"
  - "SOL/USD"

defaults:
  take_profit_pct: 1.0
  stop_loss_pct: 0.5
  min_order_amount: 10.0
  max_order_amount: 100.0

symbol_overrides:
  "BTC/USD":
    take_profit_pct: 2.0
    stop_loss_pct: 1.0
  "ETH/USD":
    take_profit_pct: 1.5

history_limit: 50
warmup_count: 50
llm_queue_size: 100
llm_max_concurrent: 3
no_trade_cooldown_quotes: 10
strategy_mode: "hft"
chatter_level: "normal"

hft:
  evaluate_every_quotes: 5
  min_edge_bps: 10.0
  take_profit_bps: 50.0
  stop_loss_bps: 25.0
  max_spread_bps: 30.0

hybrid:
  gate_refresh_quotes: 100
  no_trade_cooldown_quotes: 50

llm:
  api_key: null
  base_url: "http://localhost:11434/v1"
  model: "test-model"

alpaca:
  api_key: "TEST_KEY"
  secret_key: "TEST_SECRET"
  base_url: "https://paper-api.alpaca.markets"

exit_on_quotes: true
"#;
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn test_get_symbol_params_default() {
        let config = create_test_config();

        // SOL/USD has no override, should use defaults
        let (tp, sl) = config.get_symbol_params("SOL/USD");
        assert_eq!(tp, 1.0);
        assert_eq!(sl, 0.5);
    }

    #[test]
    fn test_get_symbol_params_full_override() {
        let config = create_test_config();

        // BTC/USD has both overrides
        let (tp, sl) = config.get_symbol_params("BTC/USD");
        assert_eq!(tp, 2.0);
        assert_eq!(sl, 1.0);
    }

    #[test]
    fn test_get_symbol_params_partial_override() {
        let config = create_test_config();

        // ETH/USD has only TP override
        let (tp, sl) = config.get_symbol_params("ETH/USD");
        assert_eq!(tp, 1.5);
        assert_eq!(sl, 0.5); // Uses default
    }

    #[test]
    fn test_get_symbol_params_unknown_symbol() {
        let config = create_test_config();

        // Unknown symbol should use defaults
        let (tp, sl) = config.get_symbol_params("UNKNOWN/USD");
        assert_eq!(tp, 1.0);
        assert_eq!(sl, 0.5);
    }

    // ============= Full Config Tests =============

    #[test]
    fn test_full_config_deserialize() {
        let config = create_test_config();

        assert_eq!(config.trading_mode, "crypto");
        assert_eq!(config.exchange, "alpaca");
        assert_eq!(config.symbols.len(), 3);
        assert_eq!(config.strategy_mode, "hft");
        assert!(config.exit_on_quotes);
    }

    #[test]
    fn test_config_optional_exchanges() {
        let config = create_test_config();

        // These should be None since not provided
        assert!(config.binance.is_none());
        assert!(config.coinbase.is_none());
        assert!(config.kraken.is_none());
    }

    #[test]
    fn test_config_clone() {
        let config = create_test_config();
        let cloned = config.clone();

        assert_eq!(cloned.trading_mode, "crypto");
        assert_eq!(cloned.symbols, config.symbols);
    }

    #[test]
    fn test_config_debug() {
        let config = create_test_config();
        let debug = format!("{:?}", config);

        assert!(debug.contains("AppConfig"));
        assert!(debug.contains("trading_mode"));
    }

    // ============= BPS to Percent Conversion Tests =============

    #[test]
    fn test_bps_conversion() {
        // 100 bps = 1%
        // 10 bps = 0.1%
        let bps: f64 = 50.0;
        let percent = bps / 100.0; // 0.5%
        assert_eq!(percent, 0.5);

        let price: f64 = 100.0;
        let tp_price = price * (1.0 + bps / 10_000.0);
        assert!((tp_price - 100.5).abs() < 0.001);
    }

    // ============= Defaults Validation =============

    #[test]
    fn test_realistic_defaults() {
        let config = create_test_config();

        // Validate sensible ranges
        assert!(config.defaults.take_profit_pct > 0.0);
        assert!(config.defaults.stop_loss_pct > 0.0);
        assert!(config.defaults.min_order_amount > 0.0);
        assert!(config.defaults.max_order_amount > config.defaults.min_order_amount);
    }

    #[test]
    fn test_hft_config_sensible() {
        let config = create_test_config();

        // TP should be > SL for positive expectancy
        assert!(config.hft.take_profit_bps > config.hft.stop_loss_bps);
        // Spread filter should be reasonable
        assert!(config.hft.max_spread_bps > 0.0);
    }
}
