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
    /// Raw momentum threshold in bps. Kept as a coarse pre-filter, but entry is actually
    /// gated on `min_net_edge_bps` below, which nets out spread and fees.
    pub min_edge_bps: f64,
    pub take_profit_bps: f64,
    pub stop_loss_bps: f64,
    pub max_spread_bps: f64,
    /// Minimum volume ratio vs moving average (filter low liquidity periods)
    #[serde(default = "default_volume_ratio")]
    pub min_volume_ratio: f64,
    /// Use VWAP for trend confirmation
    #[serde(default)]
    pub use_vwap_filter: bool,
    /// Lookback window for momentum calculation
    #[serde(default = "default_momentum_lookback")]
    pub momentum_lookback: usize,
    /// Minimum edge in bps *after* subtracting spread and round-trip fees
    /// (`edge_bps - spread_bps - fees.round_trip_fee_bps`). This is the real
    /// profitability gate - raw `min_edge_bps` alone ignores trading costs entirely.
    #[serde(default = "default_min_net_edge_bps")]
    pub min_net_edge_bps: f64,
}

fn default_volume_ratio() -> f64 {
    0.5
}

fn default_momentum_lookback() -> usize {
    20
}

fn default_min_net_edge_bps() -> f64 {
    5.0
}

/// Trading cost assumptions used to compute net edge. Alpaca's crypto taker fee is tiered
/// by 30-day trailing volume - the default here is a conservative worst-tier estimate.
/// Confirm your actual tier in the Alpaca dashboard and tune this down if it's lower;
/// erring conservative (too high) only costs skipped marginal trades, not losses.
#[derive(Clone, Debug, Deserialize)]
pub struct FeeConfig {
    #[serde(default = "default_round_trip_fee_bps")]
    pub round_trip_fee_bps: f64,
}

fn default_round_trip_fee_bps() -> f64 {
    50.0
}

impl Default for FeeConfig {
    fn default() -> Self {
        Self {
            round_trip_fee_bps: default_round_trip_fee_bps(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct MicroTradeConfig {
    /// Target % of balance per trade (e.g., 0.05 = 5%)
    pub target_balance_pct: f64,
    /// Aggression in basis points for limit price (higher = closer to market, faster fills)
    pub aggression_bps: f64,
    /// Minimum interval between orders per symbol (ms)
    pub min_order_interval_ms: u64,
    /// Account cache refresh interval (secs)
    pub account_cache_secs: u64,
    /// If true, use LLM to filter/validate HFT signals (slower but potentially smarter)
    #[serde(default)]
    pub use_llm_filter: bool,
    /// If true, use Day time-in-force for limit orders (expire at end of day)
    #[serde(default = "default_true")]
    pub limit_orders_expire_daily: bool,
    /// Time-in-force for crypto limit orders: "gtc" or "ioc"
    /// - gtc: Good Till Canceled (stays open until filled or manually canceled)
    /// - ioc: Immediate Or Cancel (fills immediately or cancels, no partial fills wait)
    #[serde(default = "default_tif")]
    pub crypto_time_in_force: String,
    /// If true, allow multiple positions per symbol (position stacking)
    /// WARNING: This increases risk exposure!
    #[serde(default)]
    pub allow_multiple_positions: bool,
    /// If true, use trailing stop to lock in profits
    #[serde(default)]
    pub use_trailing_stop: bool,
    /// Activate trailing stop after this % profit
    #[serde(default = "default_trailing_activation")]
    pub trailing_stop_activation_pct: f64,
    /// Trail the stop by this % below the highest price reached
    #[serde(default = "default_trailing_distance")]
    pub trailing_stop_distance_pct: f64,
    /// Max concurrent lots per symbol when `allow_multiple_positions` is enabled.
    /// Default of 1 matches pre-stacking behavior (no stacking) even if
    /// `allow_multiple_positions` is mistakenly left on without setting this.
    #[serde(default = "default_max_positions_per_symbol")]
    pub max_positions_per_symbol: u32,
    /// Portfolio-wide cap on distinct symbols with an open position at once.
    #[serde(default = "default_max_open_positions")]
    pub max_open_positions: u32,
}

fn default_trailing_activation() -> f64 {
    0.5
}

fn default_trailing_distance() -> f64 {
    0.25
}

fn default_max_positions_per_symbol() -> u32 {
    1
}

fn default_max_open_positions() -> u32 {
    10
}

fn default_true() -> bool {
    true
}
fn default_tif() -> String {
    "gtc".to_string()
}

impl Default for MicroTradeConfig {
    fn default() -> Self {
        Self {
            target_balance_pct: 0.02,
            aggression_bps: 15.0,
            min_order_interval_ms: 1000,
            account_cache_secs: 30,
            use_llm_filter: false,
            limit_orders_expire_daily: true,
            crypto_time_in_force: "ioc".to_string(),
            allow_multiple_positions: false,
            use_trailing_stop: true,
            trailing_stop_activation_pct: 0.4,
            trailing_stop_distance_pct: 0.2,
            max_positions_per_symbol: default_max_positions_per_symbol(),
            max_open_positions: default_max_open_positions(),
        }
    }
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

/// Finnhub API key for PEAD (post-earnings-announcement-drift) research - the one signal
/// this project's whole strategy search has never been able to test, since Alpaca has no
/// earnings-estimate/surprise data. Free tier: 60 calls/min, no card required. See
/// `docs/STRATEGIES.md` and `src/bin/pead.rs`.
#[derive(Clone, Debug, Deserialize)]
pub struct FinnhubConfig {
    pub api_key: String,
    #[serde(default = "default_finnhub_base_url")]
    pub base_url: String,
}
fn default_finnhub_base_url() -> String {
    "https://finnhub.io/api/v1".to_string()
}

/// Config for `evaluate_short_momentum_signal` (`portfolio --mode short_momentum`): a
/// standalone short strategy, the first in this codebase to trade the short side as its
/// own strategy rather than only as a pairs-trading hedge leg. Requires shorting enabled
/// on the account. See `docs/STRATEGIES.md`.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ShortMomentumConfig {
    /// Longer-horizon trend filter period: price must sit below its own SMA over this many
    /// bars, confirming a genuine downtrend rather than a dip inside an uptrend.
    pub trend_period: usize,
}
impl Default for ShortMomentumConfig {
    fn default() -> Self {
        Self { trend_period: 50 }
    }
}

/// Config for `evaluate_vol_squeeze_signal` (`portfolio --mode vol_squeeze`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct VolSqueezeConfig {
    pub period: usize,
    pub num_sd: f64,
    /// Bollinger bandwidth threshold below which the last `lookback_period` bars count as
    /// having shown a "squeeze" (unusually tight range). Expressed as a fraction of price
    /// (e.g. 0.01 = bands span 1% of price) - tune per how volatile your symbol universe
    /// typically is; a universe of quiet mega-caps needs a tighter threshold than crypto.
    pub squeeze_threshold: f64,
    pub lookback_period: usize,
}
impl Default for VolSqueezeConfig {
    fn default() -> Self {
        Self {
            period: 20,
            num_sd: 2.0,
            squeeze_threshold: 0.01,
            lookback_period: 20,
        }
    }
}

/// Config for `evaluate_orb_signal` (`portfolio --mode orb`): buys a breakout above the
/// high of the first `range_bars` bars of each trading session.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct OrbConfig {
    pub range_bars: usize,
}
impl Default for OrbConfig {
    fn default() -> Self {
        Self { range_bars: 15 }
    }
}

/// Margin-leveraged position sizing, layered on top of any `portfolio` mode via
/// `--leverage`. Models a rough daily interest accrual on the borrowed portion - NOT a
/// real Alpaca margin schedule (confirm your account's actual rate on the dashboard); this
/// is a conservative estimate to avoid a leveraged backtest looking better than it would
/// actually be after real borrowing costs.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct MarginConfig {
    /// 1.0 = no leverage (cash-only, today's default behavior). Up to 4.0 given this
    /// account's confirmed 4x margin approval.
    pub leverage_multiplier: f64,
    /// Annualized interest rate charged on the borrowed portion of a leveraged position.
    pub annual_margin_rate_pct: f64,
}
impl Default for MarginConfig {
    fn default() -> Self {
        Self {
            leverage_multiplier: 1.0,
            annual_margin_rate_pct: 11.5,
        }
    }
}

/// Config for basket/sector-relative-value arbitrage (`portfolio --mode basket_arb`):
/// trades a single symbol long/short against the mean-reversion signal of its own return
/// vs. its sector peer group's average return (see `sector_group` in `portfolio.rs`) -
/// only the single symbol is ever actually traded, not a real basket order.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct BasketArbConfig {
    /// Rolling window (in bars) used to compute the mean/stddev of the symbol-vs-basket
    /// spread for z-scoring.
    pub z_period: usize,
    /// Enter when the spread's z-score is at or beyond +/- this value.
    pub entry_z: f64,
}
impl Default for BasketArbConfig {
    fn default() -> Self {
        Self {
            z_period: 30,
            entry_z: 2.0,
        }
    }
}

/// Config for keyword-based headline sentiment (`portfolio --mode news_sentiment`), using
/// Alpaca's News API (`AlpacaClient::get_news`) - no separate API key/signup needed since
/// it's included with the existing Alpaca market data access.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct NewsSentimentConfig {
    /// Only count a headline as "current" news for this many hours after it was published.
    pub lookback_hours: i64,
    /// Enter long when the keyword sentiment score (range roughly -1..1) is at or above
    /// this value.
    pub sentiment_threshold: f64,
}
impl Default for NewsSentimentConfig {
    fn default() -> Self {
        Self {
            lookback_hours: 24,
            sentiment_threshold: 0.3,
        }
    }
}

/// Config for the logistic-regression signal (`portfolio --mode ml_signal`): trains a
/// simple model on a window strictly BEFORE the backtest window (never touches test-window
/// data during training, to avoid lookahead bias), then predicts on live features during
/// the backtest itself exactly like any other signal.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct MlSignalConfig {
    /// How many days of data immediately preceding the backtest window to train on.
    pub train_days: i64,
    /// Training label: did price rise by `label_threshold_pct` within this many bars?
    pub label_horizon_bars: usize,
    pub label_threshold_pct: f64,
    pub learning_rate: f64,
    pub epochs: usize,
    /// Predicted probability threshold to trigger a Buy signal at test time.
    pub predict_threshold: f64,
    /// Subsample training rows every Nth bar to bound training time on large universes -
    /// 1 = use every row, higher = faster but noisier gradient estimates.
    pub row_stride: usize,
}
impl Default for MlSignalConfig {
    fn default() -> Self {
        Self {
            train_days: 60,
            label_horizon_bars: 15,
            label_threshold_pct: 0.3,
            learning_rate: 0.1,
            epochs: 150,
            predict_threshold: 0.55,
            row_stride: 3,
        }
    }
}

/// Config for the approximated market-making backtest (`src/bin/market_maker.rs`). See
/// that file's module doc for the important caveat that this strategy is NOT realistically
/// executable via a retail Alpaca account (no exchange queue priority or maker rebates) -
/// the backtest is illustrative/educational, not a deployable strategy candidate.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct MarketMakerConfig {
    /// How far from mid to place each resting quote, in bps (half-spread).
    pub quote_offset_bps: f64,
    /// Max absolute inventory (in $ notional) before quotes skew hard to flatten it.
    pub inventory_limit_usd: f64,
    /// How aggressively quotes skew per unit of inventory imbalance (0 = never skew,
    /// 1 = fully skew toward flattening at the inventory limit).
    pub skew_factor: f64,
}
impl Default for MarketMakerConfig {
    fn default() -> Self {
        Self {
            quote_offset_bps: 5.0,
            inventory_limit_usd: 200.0,
            skew_factor: 0.5,
        }
    }
}

/// Container for every new strategy's tunable parameters added alongside the original
/// signal set - one place to look for "how do I configure strategy X," documented in full
/// in `docs/STRATEGIES.md`. Each sub-config has its own sane defaults, so config.yaml only
/// needs to override what you actually want to change.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct StrategiesConfig {
    pub short_momentum: ShortMomentumConfig,
    pub vol_squeeze: VolSqueezeConfig,
    pub orb: OrbConfig,
    pub margin: MarginConfig,
    pub basket_arb: BasketArbConfig,
    pub news_sentiment: NewsSentimentConfig,
    pub ml_signal: MlSignalConfig,
    pub market_maker: MarketMakerConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    pub trading_mode: String,
    pub exchange: String, // "alpaca", "binance", etc.
    pub symbols: Vec<String>,
    /// Stock symbol universe, used when `trading_mode` is "stocks" instead of `symbols`
    /// (which stays crypto-specific). Kept as the single source of truth for which stocks
    /// the backtest tools (`backtest`, `pairs`, `portfolio`) test against, rather than a
    /// hardcoded list duplicated in code - see `AppConfig::active_symbols`.
    #[serde(default)]
    pub stock_symbols: Option<Vec<String>>,

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
    #[serde(default)]
    pub micro_trade: MicroTradeConfig,
    #[serde(default)]
    pub fees: FeeConfig,
    /// Tunable parameters for the strategies added beyond the original signal set (short
    /// momentum, vol squeeze, ORB, margin leverage, basket arb, news sentiment, ML signal,
    /// market making). See `docs/STRATEGIES.md`.
    #[serde(default)]
    pub strategies: StrategiesConfig,
    pub llm: LlmConfig,
    pub alpaca: AlpacaConfig,
    pub binance: Option<BinanceConfig>,
    pub coinbase: Option<CoinbaseConfig>,
    pub kraken: Option<KrakenConfig>,
    pub finnhub: Option<FinnhubConfig>,

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
                if let Some(v) = sc.take_profit_pct {
                    tp = v;
                }
                if let Some(v) = sc.stop_loss_pct {
                    sl = v;
                }
            }
        }
        (tp, sl)
    }

    /// The symbol universe to trade given `trading_mode`: `stock_symbols` when mode is
    /// "stocks" and that list is populated, `symbols` otherwise (crypto, or stocks mode
    /// with no stock list configured yet).
    pub fn active_symbols(&self) -> &Vec<String> {
        if self.trading_mode.eq_ignore_ascii_case("stocks") {
            if let Some(stocks) = &self.stock_symbols {
                if !stocks.is_empty() {
                    return stocks;
                }
            }
        }
        &self.symbols
    }
}
