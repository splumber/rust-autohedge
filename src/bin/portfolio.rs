//! Simulates a real account: starts with a fixed cash balance, sizes every trade as a
//! fraction of *current cash* (not a fixed notional regardless of account size), and
//! shares one capital pool across every symbol - unlike `backtest.rs`/`pairs.rs`, which
//! give every symbol its own unlimited $100-per-trade budget and just sum the results.
//! That's fine for asking "does this signal have edge at all," but wrong for answering
//! "if I actually funded an account with $X, what would it be worth after N days" -
//! this binary answers the second question.
//!
//! Position sizing: each new trade risks `target_balance_pct` of *cash on hand* (not
//! total equity including open positions' paper value), capped by `max_order_amount` and
//! floored by `min_order_amount`. Using cash rather than mark-to-market equity as the
//! sizing basis means the simulator can never oversize into money it doesn't actually
//! have liquid - the same constraint a real broker enforces via buying power.
//!
//! Every mode below is configured primarily through `config.yaml`'s `strategies:` section
//! (see `docs/STRATEGIES.md` for the full reference) - CLI flags exist for the handful of
//! parameters that make sense to sweep from the command line (mode, capital, date range,
//! leverage, excluded weekdays).
//!
//! Usage: `cargo run --bin portfolio --release -- --mode momentum --starting-cash 500
//! --days 60 [--asset-class stocks] [--offset-days N] [--tp-pct N --sl-pct N]
//! [--leverage 2.0] [--exclude-weekdays mon,fri] [--min-agree 2]`

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Utc, Weekday};
use rust_autohedge::backtest_support::{fetch_all_bars, liquid_stock_universe, AssetClass};
use rust_autohedge::config::{AppConfig, FeeConfig, HftConfig, MlSignalConfig, StrategiesConfig};
use rust_autohedge::data::alpaca::AlpacaClient;
use rust_autohedge::data::alpaca::{CryptoBar, NewsArticle};
use rust_autohedge::services::indicators::{rsi, sma, stddev, vwap as vwap_indicator};
use rust_autohedge::services::ml_signal::{LogisticModel, FEATURE_COUNT as ML_FEATURE_COUNT};
use rust_autohedge::services::sentiment::score_headline;
use rust_autohedge::services::strategy::{
    evaluate_bollinger_signal, evaluate_donchian_breakout_signal, evaluate_hft_signal,
    evaluate_ma_crossover_signal, evaluate_mean_reversion_signal, evaluate_orb_signal,
    evaluate_rsi_signal, evaluate_short_momentum_signal, evaluate_trend_filtered_momentum_signal,
    evaluate_vol_squeeze_signal, evaluate_vwap_signal, HftSignal, SignalDecision,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

const DEFAULT_STARTING_CASH: f64 = 500.0;
const DEFAULT_LOOKBACK_DAYS: i64 = 60;
const TIMEFRAME: &str = "1Min";
const HISTORY_CAP: usize = 64;
const DEFAULT_TARGET_BALANCE_PCT: f64 = 0.05; // risk 5% of current cash per new trade
const MIN_ORDER_AMOUNT: f64 = 5.0;
const DEFAULT_MAX_ORDER_AMOUNT: f64 = 100.0;
const RSI_PERIOD: usize = 14;
const RSI_OVERSOLD: f64 = 20.0;
const BOLLINGER_PERIOD: usize = 20;
const BOLLINGER_Z: f64 = 3.0;
const MA_FAST_PERIOD: usize = 21;
const MA_SLOW_PERIOD: usize = 55;
const DONCHIAN_PERIOD: usize = 50;
const VWAP_PERIOD: usize = 30;
const VWAP_ENTRY_BPS: f64 = 50.0;
const TREND_PERIOD: usize = 50;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Direction {
    Long,
    Short,
}

impl Direction {
    fn label(&self) -> &'static str {
        match self {
            Direction::Long => "long",
            Direction::Short => "short",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Momentum,
    Reversion,
    Rsi,
    Bollinger,
    MaCross,
    Donchian,
    Vwap,
    TrendMomentum,
    /// Enter only when at least `min_agree` of the 8 standalone long-only signals above
    /// agree, instead of picking one signal family up front. New short/relative-value/ML
    /// modes below aren't part of this vote - mixing long and short votes needs its own
    /// aggregation design, out of scope here.
    Combined,
    /// Standalone short strategy (breakdown momentum) - the first mode in this file to
    /// trade the short side as its own strategy rather than a pairs-trading hedge leg.
    ShortMomentum,
    /// Bollinger-bandwidth squeeze followed by an upward breakout.
    VolSqueeze,
    /// Opening-range breakout: buy above the first `orb.range_bars` bars' high each day.
    Orb,
    /// Long/short a single symbol against its sector peer group's average return.
    BasketArb,
    /// Keyword-scored headline sentiment via Alpaca's News API.
    NewsSentiment,
    /// Logistic-regression signal trained on a window strictly before the backtest window.
    MlSignal,
}

const ALL_SINGLE_MODES: [Mode; 8] = [
    Mode::Momentum,
    Mode::Reversion,
    Mode::Rsi,
    Mode::Bollinger,
    Mode::MaCross,
    Mode::Donchian,
    Mode::Vwap,
    Mode::TrendMomentum,
];

impl Mode {
    fn from_args() -> Self {
        match std::env::args().skip_while(|a| a != "--mode").nth(1).as_deref() {
            Some("reversion") => Mode::Reversion,
            Some("rsi") => Mode::Rsi,
            Some("bollinger") => Mode::Bollinger,
            Some("ma_cross") => Mode::MaCross,
            Some("donchian") => Mode::Donchian,
            Some("vwap") => Mode::Vwap,
            Some("trend_momentum") => Mode::TrendMomentum,
            Some("combined") => Mode::Combined,
            Some("short_momentum") => Mode::ShortMomentum,
            Some("vol_squeeze") => Mode::VolSqueeze,
            Some("orb") => Mode::Orb,
            Some("basket_arb") => Mode::BasketArb,
            Some("news_sentiment") => Mode::NewsSentiment,
            Some("ml_signal") => Mode::MlSignal,
            _ => Mode::Momentum,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Mode::Momentum => "momentum",
            Mode::Reversion => "mean-reversion",
            Mode::Rsi => "rsi",
            Mode::Bollinger => "bollinger",
            Mode::MaCross => "ma-cross",
            Mode::Donchian => "donchian",
            Mode::Vwap => "vwap",
            Mode::TrendMomentum => "trend-momentum",
            Mode::Combined => "combined",
            Mode::ShortMomentum => "short-momentum",
            Mode::VolSqueeze => "vol-squeeze",
            Mode::Orb => "orb",
            Mode::BasketArb => "basket-arb",
            Mode::NewsSentiment => "news-sentiment",
            Mode::MlSignal => "ml-signal",
        }
    }
}

fn f64_arg(flag: &str, default: f64) -> f64 {
    std::env::args()
        .skip_while(|a| a != flag)
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(default)
}

fn i64_arg(flag: &str, default: i64) -> i64 {
    std::env::args()
        .skip_while(|a| a != flag)
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(default)
}

fn opt_f64_arg(flag: &str) -> Option<f64> {
    std::env::args()
        .skip_while(|a| a != flag)
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
}

fn string_arg(flag: &str) -> Option<String> {
    std::env::args().skip_while(|a| a != flag).nth(1)
}

fn asset_class_from_args() -> AssetClass {
    std::env::args()
        .skip_while(|a| a != "--asset-class")
        .nth(1)
        .map(|s| AssetClass::from_arg(&s))
        .unwrap_or(AssetClass::Crypto)
}

fn parse_excluded_weekdays(s: &str) -> Vec<Weekday> {
    s.split(',')
        .filter_map(|tok| match tok.trim().to_lowercase().as_str() {
            "mon" | "monday" => Some(Weekday::Mon),
            "tue" | "tuesday" => Some(Weekday::Tue),
            "wed" | "wednesday" => Some(Weekday::Wed),
            "thu" | "thursday" => Some(Weekday::Thu),
            "fri" | "friday" => Some(Weekday::Fri),
            "sat" | "saturday" => Some(Weekday::Sat),
            "sun" | "sunday" => Some(Weekday::Sun),
            _ => None,
        })
        .collect()
}

fn is_excluded_weekday(timestamp: &str, excluded: &[Weekday]) -> bool {
    if excluded.is_empty() {
        return false;
    }
    match DateTime::parse_from_rfc3339(timestamp) {
        Ok(dt) => excluded.contains(&dt.weekday()),
        Err(_) => false,
    }
}

fn days_between(start: &str, end: &str) -> f64 {
    match (
        DateTime::parse_from_rfc3339(start),
        DateTime::parse_from_rfc3339(end),
    ) {
        (Ok(s), Ok(e)) => (e - s).num_seconds() as f64 / 86_400.0,
        _ => 0.0,
    }
}

/// Coarse sector grouping over the project's curated 37-symbol stock universe, used only
/// by `Mode::BasketArb` to build each symbol's peer-group reference basket. Symbols outside
/// this list (crypto, or an unrecognized stock) fall into "other" - basket arb won't find
/// a meaningful peer group for them, which just means it never fires, not a crash.
fn sector_group(symbol: &str) -> &'static str {
    match symbol {
        "AAPL" | "MSFT" | "GOOGL" | "AMZN" | "META" | "NVDA" | "TSLA" | "AMD" | "AVGO" | "CRM" => {
            "mega_cap_tech"
        }
        "JPM" | "BAC" | "GS" | "MS" | "V" | "MA" => "finance",
        "JNJ" | "UNH" | "PFE" | "ABBV" => "healthcare",
        "WMT" | "COST" | "HD" | "MCD" | "NKE" | "DIS" | "PG" | "KO" | "PEP" => "consumer",
        "XOM" | "CVX" | "CAT" | "BA" => "industrial_energy",
        "SPY" | "QQQ" | "DIA" | "IWM" => "index_etf",
        _ => "other",
    }
}

/// Feature vector for `Mode::MlSignal`, shared between training (`train_ml_model`) and
/// live prediction in the backtest loop, so the two can never compute features differently.
fn ml_features(closes: &[f64], volumes: &[f64]) -> Option<[f64; ML_FEATURE_COUNT]> {
    if closes.len() < 51 || closes.len() != volumes.len() {
        return None;
    }
    let mid = *closes.last().unwrap();
    if mid <= 0.0 {
        return None;
    }
    let r = rsi(closes, 14)?;
    let lookback = 10.min(closes.len() - 1);
    let past = closes[closes.len() - 1 - lookback];
    if past <= 0.0 {
        return None;
    }
    let momentum_bps = (mid - past) / past * 10_000.0;
    let vw = vwap_indicator(closes, volumes, 30)?;
    if vw <= 0.0 {
        return None;
    }
    let vwap_dev_bps = (mid - vw) / vw * 10_000.0;
    let trend_sma = sma(closes, 50)?;
    if trend_sma <= 0.0 {
        return None;
    }
    let trend_dev_bps = (mid - trend_sma) / trend_sma * 10_000.0;
    let (bmean, bsd) = (sma(closes, 20)?, stddev(closes, 20)?);
    let boll_z = if bsd > 0.0 { (mid - bmean) / bsd } else { 0.0 };

    Some([
        (r - 50.0) / 50.0,
        (momentum_bps / 100.0).clamp(-5.0, 5.0),
        (vwap_dev_bps / 100.0).clamp(-5.0, 5.0),
        (trend_dev_bps / 100.0).clamp(-5.0, 5.0),
        boll_z.clamp(-5.0, 5.0),
    ])
}

/// Trains the ML signal on `bars_by_symbol` (expected to be a window strictly BEFORE the
/// backtest window - the caller is responsible for that split). Label: did price rise by
/// at least `label_threshold_pct` within `label_horizon_bars` bars.
fn train_ml_model(bars_by_symbol: &[(String, Vec<CryptoBar>)], cfg: &MlSignalConfig) -> LogisticModel {
    let mut rows: Vec<([f64; ML_FEATURE_COUNT], f64)> = Vec::new();
    for (_, bars) in bars_by_symbol {
        let mut closes: Vec<f64> = Vec::with_capacity(bars.len());
        let mut volumes: Vec<f64> = Vec::with_capacity(bars.len());
        for (i, bar) in bars.iter().enumerate() {
            if bar.close <= 0.0 {
                continue;
            }
            closes.push(bar.close);
            volumes.push(bar.volume);
            if cfg.row_stride > 1 && i % cfg.row_stride != 0 {
                continue;
            }
            let label_idx = i + cfg.label_horizon_bars;
            if label_idx >= bars.len() {
                continue;
            }
            let Some(features) = ml_features(&closes, &volumes) else {
                continue;
            };
            let future_close = bars[label_idx].close;
            let current_close = bar.close;
            if current_close <= 0.0 {
                continue;
            }
            let future_return_pct = (future_close - current_close) / current_close * 100.0;
            let label = if future_return_pct >= cfg.label_threshold_pct { 1.0 } else { 0.0 };
            rows.push((features, label));
        }
    }
    println!("ML training: {} labeled rows collected", rows.len());
    LogisticModel::train(&rows, cfg.learning_rate, cfg.epochs, 0.001)
}

fn build_news_index(
    news: &[NewsArticle],
    universe: &HashSet<&str>,
) -> HashMap<String, Vec<(DateTime<Utc>, f64)>> {
    let mut index: HashMap<String, Vec<(DateTime<Utc>, f64)>> = HashMap::new();
    for article in news {
        let Ok(published) = DateTime::parse_from_rfc3339(&article.created_at) else {
            continue;
        };
        let published_utc = published.with_timezone(&Utc);
        let score = score_headline(&article.headline, &article.summary);
        for sym in &article.symbols {
            if universe.contains(sym.as_str()) {
                index.entry(sym.clone()).or_default().push((published_utc, score));
            }
        }
    }
    for v in index.values_mut() {
        v.sort_by_key(|(t, _)| *t);
    }
    index
}

/// LLM-scored variant of `build_news_index`: same lookahead-safe indexing, but each
/// article's sentiment comes from the project's local LLM (cached to disk, see
/// `LlmSentimentScorer`) instead of keyword matching - a genuine semantic upgrade, not just
/// a different threshold on the same crude signal. Scoring one article at a time (this
/// hardware doesn't handle concurrent Ollama requests reliably - confirmed by direct
/// testing), so this can take a while on a large, uncached article set; progress is
/// printed periodically and the cache means repeat runs over overlapping date ranges only
/// pay for genuinely new articles.
async fn build_news_index_llm(
    news: &[NewsArticle],
    universe: &HashSet<&str>,
    scorer: &mut rust_autohedge::services::llm_sentiment::LlmSentimentScorer,
) -> HashMap<String, Vec<(DateTime<Utc>, f64)>> {
    let mut index: HashMap<String, Vec<(DateTime<Utc>, f64)>> = HashMap::new();
    let total = news.len();
    for (i, article) in news.iter().enumerate() {
        let Ok(published) = DateTime::parse_from_rfc3339(&article.created_at) else {
            continue;
        };
        let published_utc = published.with_timezone(&Utc);
        let relevant = article.symbols.iter().any(|s| universe.contains(s.as_str()));
        if !relevant {
            continue;
        }
        let score = scorer.score(&article.headline, &article.summary).await;
        for sym in &article.symbols {
            if universe.contains(sym.as_str()) {
                index.entry(sym.clone()).or_default().push((published_utc, score));
            }
        }
        if (i + 1) % 200 == 0 || i + 1 == total {
            println!(
                "  LLM sentiment scoring: {}/{} articles ({} new LLM calls, {} cached total)",
                i + 1, total, scorer.calls_made(), scorer.cache_size()
            );
        }
    }
    scorer.save();
    for v in index.values_mut() {
        v.sort_by_key(|(t, _)| *t);
    }
    index
}

/// Most recent sentiment score for `symbol` published strictly before `now` and within
/// `lookback_hours` - strictly-before is what keeps this lookahead-free in the backtest.
fn news_sentiment_at(
    index: &HashMap<String, Vec<(DateTime<Utc>, f64)>>,
    symbol: &str,
    now: DateTime<Utc>,
    lookback_hours: i64,
) -> Option<f64> {
    let series = index.get(symbol)?;
    let cutoff = now - ChronoDuration::hours(lookback_hours);
    let idx = series.partition_point(|(t, _)| *t < now);
    if idx == 0 {
        return None;
    }
    let (t, score) = series[idx - 1];
    if t >= cutoff {
        Some(score)
    } else {
        None
    }
}

struct SymbolState {
    mids: VecDeque<f64>,
    volumes: VecDeque<f64>,
    orb_current_day: String,
    orb_range_high: Option<f64>,
    orb_bars_seen: usize,
    basket_spread_history: VecDeque<f64>,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            mids: VecDeque::with_capacity(HISTORY_CAP),
            volumes: VecDeque::with_capacity(HISTORY_CAP),
            orb_current_day: String::new(),
            orb_range_high: None,
            orb_bars_seen: 0,
            basket_spread_history: VecDeque::with_capacity(HISTORY_CAP),
        }
    }
}

struct OpenPosition {
    direction: Direction,
    entry_price: f64,
    qty: f64,
    take_profit: f64,
    stop_loss: f64,
    entry_time: String,
    /// Full notional exposure of the position (long: cash paid; short: proceeds received).
    cost_basis: f64,
    /// Amount borrowed on margin to reach `cost_basis` beyond what cash alone funded.
    /// Always 0 for shorts - leverage in this simulator only scales long sizing (see
    /// `docs/STRATEGIES.md` for why shorts are scoped out of the leverage model).
    borrowed: f64,
    /// Best price seen since entry (highest for longs, lowest for shorts) - only tracked
    /// when a trailing stop is active; used to compute the trailing exit level.
    peak_price: f64,
    /// Whether the trailing stop has activated yet (price has moved `trailing_activation_pct`
    /// in the position's favor). Before activation, the position still exits on the fixed
    /// take-profit/stop-loss exactly as without a trailing stop.
    trailing_active: bool,
}

#[derive(Serialize)]
struct ClosedTrade {
    symbol: String,
    direction: &'static str,
    entry_time: String,
    exit_time: String,
    entry_price: f64,
    exit_price: f64,
    qty: f64,
    exit_reason: &'static str,
    pnl: f64,
    cash_after: f64,
}

#[derive(Serialize, Clone)]
struct EquitySnapshot {
    timestamp: String,
    equity: f64,
}

struct MergedEvent<'a> {
    timestamp: &'a str,
    symbol: &'a str,
    bar: &'a CryptoBar,
}

fn merge_chronologically(bars_by_symbol: &[(String, Vec<CryptoBar>)]) -> Vec<MergedEvent> {
    let mut events = Vec::new();
    for (symbol, bars) in bars_by_symbol {
        for bar in bars {
            events.push(MergedEvent {
                timestamp: &bar.timestamp,
                symbol,
                bar,
            });
        }
    }
    events.sort_by(|a, b| a.timestamp.cmp(b.timestamp));
    events
}

/// Extra per-event context needed by strategies whose signal depends on state beyond a
/// single symbol's own rolling price/volume window (opening range, sector-basket
/// consensus, news sentiment, or a pre-trained model). Only the field(s) relevant to the
/// active `Mode` are populated per event.
#[derive(Default)]
struct ExtraContext<'a> {
    opening_range_high: Option<f64>,
    basket_spread_z: Option<f64>,
    news_sentiment: Option<f64>,
    ml_model: Option<&'a LogisticModel>,
    ml_features: Option<[f64; ML_FEATURE_COUNT]>,
}

#[allow(clippy::too_many_arguments)]
fn evaluate_entry(
    mode: Mode,
    bid: f64,
    ask: f64,
    state: &SymbolState,
    hft: &HftConfig,
    fees: &FeeConfig,
    tp_pct: f64,
    sl_pct: f64,
    min_agree: usize,
    strategies: &StrategiesConfig,
    extra: &ExtraContext,
) -> Option<Direction> {
    match mode {
        Mode::Momentum => matches!(
            evaluate_hft_signal(bid, ask, &state.mids, &state.volumes, hft, fees),
            HftSignal::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Reversion => matches!(
            evaluate_mean_reversion_signal(bid, ask, &state.mids, &state.volumes, hft, fees),
            HftSignal::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Rsi => matches!(
            evaluate_rsi_signal(bid, ask, &state.mids, RSI_PERIOD, RSI_OVERSOLD, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Bollinger => matches!(
            evaluate_bollinger_signal(bid, ask, &state.mids, BOLLINGER_PERIOD, BOLLINGER_Z, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::MaCross => matches!(
            evaluate_ma_crossover_signal(bid, ask, &state.mids, MA_FAST_PERIOD, MA_SLOW_PERIOD, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Donchian => matches!(
            evaluate_donchian_breakout_signal(bid, ask, &state.mids, DONCHIAN_PERIOD, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Vwap => matches!(
            evaluate_vwap_signal(bid, ask, &state.mids, &state.volumes, VWAP_PERIOD, VWAP_ENTRY_BPS, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::TrendMomentum => matches!(
            evaluate_trend_filtered_momentum_signal(bid, ask, &state.mids, hft.momentum_lookback, hft.min_edge_bps, TREND_PERIOD, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Combined => {
            let votes = ALL_SINGLE_MODES
                .iter()
                .filter(|&&m| {
                    evaluate_entry(m, bid, ask, state, hft, fees, tp_pct, sl_pct, 0, strategies, extra).is_some()
                })
                .count();
            (votes >= min_agree).then_some(Direction::Long)
        }
        Mode::ShortMomentum => matches!(
            evaluate_short_momentum_signal(
                bid, ask, &state.mids, hft.momentum_lookback, hft.min_edge_bps,
                strategies.short_momentum.trend_period, hft.max_spread_bps, tp_pct, sl_pct
            ),
            SignalDecision::Sell { .. }
        )
        .then_some(Direction::Short),
        Mode::VolSqueeze => matches!(
            evaluate_vol_squeeze_signal(
                bid, ask, &state.mids, strategies.vol_squeeze.period, strategies.vol_squeeze.num_sd,
                strategies.vol_squeeze.squeeze_threshold, strategies.vol_squeeze.lookback_period,
                hft.max_spread_bps, tp_pct, sl_pct
            ),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::Orb => matches!(
            evaluate_orb_signal(bid, ask, extra.opening_range_high, hft.max_spread_bps, tp_pct, sl_pct),
            SignalDecision::Buy { .. }
        )
        .then_some(Direction::Long),
        Mode::BasketArb => {
            let z = extra.basket_spread_z?;
            if z <= -strategies.basket_arb.entry_z {
                Some(Direction::Long)
            } else if z >= strategies.basket_arb.entry_z {
                Some(Direction::Short)
            } else {
                None
            }
        }
        Mode::NewsSentiment => {
            let score = extra.news_sentiment?;
            (score >= strategies.news_sentiment.sentiment_threshold).then_some(Direction::Long)
        }
        Mode::MlSignal => {
            let model = extra.ml_model?;
            let features = extra.ml_features?;
            let prob = model.predict_proba(&features);
            (prob >= strategies.ml_signal.predict_threshold).then_some(Direction::Long)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let mode = Mode::from_args();
    let asset_class = asset_class_from_args();
    let starting_cash = f64_arg("--starting-cash", DEFAULT_STARTING_CASH);
    let lookback_days = i64_arg("--days", DEFAULT_LOOKBACK_DAYS);
    let offset_days = i64_arg("--offset-days", 0);
    let min_agree = i64_arg("--min-agree", 2) as usize;
    let fixed_tp_sl = match (opt_f64_arg("--tp-pct"), opt_f64_arg("--sl-pct")) {
        (Some(tp), Some(sl)) => Some((tp, sl)),
        _ => None,
    };
    // Capital utilization: how much of current cash each new trade commits, and the hard
    // dollar cap per trade. Defaults match the original conservative sizing
    // (DEFAULT_TARGET_BALANCE_PCT=5%, DEFAULT_MAX_ORDER_AMOUNT=$100) - raise these via CLI
    // to test whether deploying capital more aggressively (closer to buy-and-hold's ~100%
    // immediate deployment) closes the gap to the passive baseline.
    let target_balance_pct = opt_f64_arg("--position-pct").unwrap_or(DEFAULT_TARGET_BALANCE_PCT);
    let max_order_amount = opt_f64_arg("--max-order").unwrap_or(DEFAULT_MAX_ORDER_AMOUNT);
    // Trailing stop: once a position is up by `trailing_activation_pct`, track its peak
    // price and exit if it pulls back `trailing_distance_pct` from that peak, instead of
    // waiting for the fixed take-profit - lets winners run further in a sustained move.
    // `None` (the default) preserves the original fixed-TP/SL-only behavior exactly.
    let trailing_stop = match (opt_f64_arg("--trailing-activation-pct"), opt_f64_arg("--trailing-distance-pct")) {
        (Some(a), Some(d)) => Some((a, d)),
        _ => None,
    };
    let use_llm_sentiment = std::env::args().any(|a| a == "--llm-sentiment");
    // Pattern Day Trader awareness: FINRA restricts margin accounts with equity below
    // `pdt_equity_threshold` ($25,000 by rule) to 3 day trades (same-symbol open+close
    // within one session) per rolling 5-trading-day window - a 4th trips PDT status and
    // broadly restricts the account. Every backtest before this flag existed implicitly
    // assumed unlimited day-trading frequency, which most of this project's high-turnover
    // modes (momentum alone: ~38 trades/day) are not actually legal to run in a real
    // sub-$25k margin account. Stocks only - PDT doesn't apply to crypto or cash accounts
    // (the latter has its own settlement-delay constraint, not modeled here).
    let enforce_pdt = std::env::args().any(|a| a == "--enforce-pdt");
    let pdt_equity_threshold = opt_f64_arg("--pdt-equity-threshold").unwrap_or(25_000.0);

    let config = AppConfig::load();
    let strategies = config.strategies.clone();
    let leverage_multiplier = opt_f64_arg("--leverage").unwrap_or(strategies.margin.leverage_multiplier).max(1.0);
    let excluded_weekdays = string_arg("--exclude-weekdays")
        .map(|s| parse_excluded_weekdays(&s))
        .unwrap_or_default();

    let client = AlpacaClient::new(config.alpaca.clone(), config.history_limit);
    let fees = match asset_class {
        AssetClass::Crypto => config.fees.clone(),
        AssetClass::Stocks => FeeConfig {
            round_trip_fee_bps: asset_class.default_round_trip_fee_bps(),
        },
    };
    let symbols = match asset_class {
        AssetClass::Crypto => config.symbols.clone(),
        AssetClass::Stocks => config.stock_symbols.clone().unwrap_or_else(liquid_stock_universe),
    };
    let (tp_pct, sl_pct) = fixed_tp_sl.unwrap_or((config.defaults.take_profit_pct, config.defaults.stop_loss_pct));

    let end = Utc::now() - ChronoDuration::days(offset_days);
    let start = end - ChronoDuration::days(lookback_days);
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();

    println!(
        "Portfolio simulation: mode={} asset_class={} starting_cash=${:.2} tp={:.1}% sl={:.1}% leverage={:.1}x {} days ({} -> {})",
        mode.label(), asset_class.label(), starting_cash, tp_pct, sl_pct, leverage_multiplier, lookback_days, start_str, end_str
    );
    println!(
        "position sizing: {:.0}% of current cash per trade, min ${:.0}, max ${:.0}",
        target_balance_pct * 100.0, MIN_ORDER_AMOUNT, max_order_amount
    );
    if let Some((activation, distance)) = trailing_stop {
        println!("trailing stop: activate at +{:.2}%, trail {:.2}% behind peak", activation, distance);
    }
    if !excluded_weekdays.is_empty() {
        println!("excluded weekdays: {:?}", excluded_weekdays);
    }
    println!();

    let bars_by_symbol = fetch_all_bars(&client, asset_class, &symbols, TIMEFRAME, &start_str, &end_str).await;
    let events = merge_chronologically(&bars_by_symbol);
    println!("\n{} merged events across {} symbols", events.len(), bars_by_symbol.len());

    // Mode-specific pre-computation: news index and/or a trained ML model, both fetched
    // from data strictly before/around the backtest window - only done when the active
    // mode actually needs it, since both can be slow over a large symbol universe.
    let news_index: HashMap<String, Vec<(DateTime<Utc>, f64)>> = if mode == Mode::NewsSentiment {
        println!("Fetching news for sentiment signal...");
        let universe: HashSet<&str> = symbols.iter().map(|s| s.as_str()).collect();
        match client.get_news(&symbols, &start_str, &end_str).await {
            Ok(articles) => {
                println!("{} news articles fetched", articles.len());
                if use_llm_sentiment {
                    let base_url = config.llm.base_url.clone().unwrap_or_else(|| "http://localhost:11434/v1".to_string());
                    let mut scorer = rust_autohedge::services::llm_sentiment::LlmSentimentScorer::new(base_url);
                    println!("Scoring articles via local LLM (cached to data/llm_sentiment_cache.json)...");
                    build_news_index_llm(&articles, &universe, &mut scorer).await
                } else {
                    build_news_index(&articles, &universe)
                }
            }
            Err(e) => {
                println!("News fetch failed ({}), news_sentiment mode will find no signal", e);
                HashMap::new()
            }
        }
    } else {
        HashMap::new()
    };

    let ml_model: Option<LogisticModel> = if mode == Mode::MlSignal {
        let train_end = start;
        let train_start = train_end - ChronoDuration::days(strategies.ml_signal.train_days);
        println!(
            "Fetching ML training window ({} -> {}, strictly before the backtest window)...",
            train_start.to_rfc3339(), train_end.to_rfc3339()
        );
        let train_bars = fetch_all_bars(
            &client, asset_class, &symbols, TIMEFRAME,
            &train_start.to_rfc3339(), &train_end.to_rfc3339(),
        ).await;
        Some(train_ml_model(&train_bars, &strategies.ml_signal))
    } else {
        None
    };

    // Sector-peer groups for basket arb, precomputed once from the actual fetched universe.
    let mut group_members: HashMap<&'static str, Vec<String>> = HashMap::new();
    if mode == Mode::BasketArb {
        for (symbol, _) in &bars_by_symbol {
            group_members.entry(sector_group(symbol)).or_default().push(symbol.clone());
        }
    }

    let mut cash = starting_cash;
    let mut states: HashMap<String, SymbolState> = HashMap::new();
    let mut positions: HashMap<String, OpenPosition> = HashMap::new();
    let mut last_price: HashMap<String, f64> = HashMap::new();
    let mut first_close: HashMap<String, f64> = HashMap::new();
    let mut trades: Vec<ClosedTrade> = Vec::new();
    let mut equity_curve: Vec<EquitySnapshot> = Vec::new();
    let mut last_snapshot_day = String::new();
    let mut blocked_new_entries_pdt = 0usize;

    // PDT tracking: the last <=5 distinct trading days seen (the rolling window FINRA
    // counts against), and every date on which a same-symbol open+close ("day trade")
    // actually occurred (duplicates allowed - multiple day trades on one day all count).
    let mut recent_trading_days: VecDeque<String> = VecDeque::with_capacity(6);
    let mut day_trade_log: VecDeque<String> = VecDeque::new();

    for ev in &events {
        let bar = ev.bar;
        let mid = bar.close;
        if mid <= 0.0 {
            continue;
        }
        last_price.insert(ev.symbol.to_string(), mid);
        first_close.entry(ev.symbol.to_string()).or_insert(mid);

        // Manage an existing position in this symbol first (direction-aware TP/SL check).
        if let Some(pos) = positions.get_mut(ev.symbol) {
            // Trailing-stop bookkeeping: track the best price seen, and once the position
            // has moved `activation_pct` in its favor, replace the fixed take-profit with a
            // level trailing `distance_pct` behind the peak - clamped so it's never worse
            // than the original hard stop-loss, even with an unusually wide distance_pct.
            if let Some((activation_pct, distance_pct)) = trailing_stop {
                match pos.direction {
                    Direction::Long => {
                        pos.peak_price = pos.peak_price.max(bar.high);
                        if !pos.trailing_active {
                            let gain_pct = (pos.peak_price - pos.entry_price) / pos.entry_price * 100.0;
                            pos.trailing_active = gain_pct >= activation_pct;
                        }
                        if pos.trailing_active {
                            let trail_level = pos.peak_price * (1.0 - distance_pct / 100.0);
                            pos.take_profit = trail_level.max(pos.stop_loss);
                        }
                    }
                    Direction::Short => {
                        pos.peak_price = pos.peak_price.min(bar.low);
                        if !pos.trailing_active {
                            let gain_pct = (pos.entry_price - pos.peak_price) / pos.entry_price * 100.0;
                            pos.trailing_active = gain_pct >= activation_pct;
                        }
                        if pos.trailing_active {
                            let trail_level = pos.peak_price * (1.0 + distance_pct / 100.0);
                            pos.take_profit = trail_level.min(pos.stop_loss);
                        }
                    }
                }
            }

            let (hit_sl, hit_tp) = match pos.direction {
                Direction::Long => (bar.low <= pos.stop_loss, bar.high >= pos.take_profit),
                Direction::Short => (bar.high >= pos.stop_loss, bar.low <= pos.take_profit),
            };
            let tp_reason = if pos.trailing_active { "trailing_stop" } else { "take_profit" };
            let exit = if hit_sl {
                Some((pos.stop_loss, "stop_loss"))
            } else if hit_tp {
                Some((pos.take_profit, tp_reason))
            } else {
                None
            };
            if let Some((exit_price, reason)) = exit {
                let fee_cost = pos.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
                let holding_days = days_between(&pos.entry_time, ev.timestamp);
                let interest_cost = pos.borrowed * (strategies.margin.annual_margin_rate_pct / 100.0) * (holding_days / 365.0);
                let pnl = match pos.direction {
                    Direction::Long => {
                        let proceeds = exit_price * pos.qty - fee_cost;
                        cash += proceeds - pos.borrowed - interest_cost;
                        proceeds - pos.cost_basis - interest_cost
                    }
                    Direction::Short => {
                        let cost_to_cover = exit_price * pos.qty + fee_cost;
                        cash -= cost_to_cover;
                        pos.cost_basis - cost_to_cover
                    }
                };
                trades.push(ClosedTrade {
                    symbol: ev.symbol.to_string(),
                    direction: pos.direction.label(),
                    entry_time: pos.entry_time.clone(),
                    exit_time: ev.timestamp.to_string(),
                    entry_price: pos.entry_price,
                    exit_price,
                    qty: pos.qty,
                    exit_reason: reason,
                    pnl,
                    cash_after: cash,
                });
                let entry_day = &pos.entry_time[..10.min(pos.entry_time.len())];
                let exit_day = &ev.timestamp[..10.min(ev.timestamp.len())];
                if entry_day == exit_day {
                    day_trade_log.push_back(exit_day.to_string());
                }
                positions.remove(ev.symbol);
            }
        }

        // Bookkeeping (same shape as backtest.rs's per-symbol history).
        let state = states.entry(ev.symbol.to_string()).or_insert_with(SymbolState::new);
        state.mids.push_back(mid);
        while state.mids.len() > HISTORY_CAP {
            state.mids.pop_front();
        }
        state.volumes.push_back(bar.volume);
        while state.volumes.len() > HISTORY_CAP {
            state.volumes.pop_front();
        }

        let day_key = &ev.timestamp[..10.min(ev.timestamp.len())];
        if recent_trading_days.back().map(|d| d.as_str()) != Some(day_key) {
            recent_trading_days.push_back(day_key.to_string());
            while recent_trading_days.len() > 5 {
                recent_trading_days.pop_front();
            }
            // Entries that have rolled out of the 5-day window can never matter again -
            // drop them so `day_trade_log` stays bounded over a long backtest instead of
            // growing for the whole run.
            day_trade_log.retain(|d| recent_trading_days.contains(d));
        }

        // ORB opening-range tracking: reset at each new day, accumulate the high over the
        // first `orb.range_bars` bars, then freeze it for the rest of the day.
        if mode == Mode::Orb {
            if day_key != state.orb_current_day {
                state.orb_current_day = day_key.to_string();
                state.orb_range_high = None;
                state.orb_bars_seen = 0;
            }
            if state.orb_bars_seen < strategies.orb.range_bars {
                state.orb_range_high = Some(state.orb_range_high.map_or(bar.high, |h: f64| h.max(bar.high)));
                state.orb_bars_seen += 1;
            }
        }

        // Basket-arb spread tracking: this symbol's return-from-start vs. its sector
        // peer group's average return-from-start, pushed into a rolling window for
        // z-scoring. Uses `last_price`/`first_close`, which are already maintained
        // globally regardless of mode.
        if mode == Mode::BasketArb {
            if let (Some(&first), Some(members)) = (first_close.get(ev.symbol), group_members.get(sector_group(ev.symbol))) {
                let sym_ret_bps = (mid / first - 1.0) * 10_000.0;
                let mut sum = 0.0;
                let mut n = 0usize;
                for m in members {
                    if m == ev.symbol {
                        continue;
                    }
                    if let (Some(&mfirst), Some(&mlast)) = (first_close.get(m), last_price.get(m)) {
                        if mfirst > 0.0 {
                            sum += (mlast / mfirst - 1.0) * 10_000.0;
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    let basket_ret_bps = sum / n as f64;
                    let spread_bps = sym_ret_bps - basket_ret_bps;
                    state.basket_spread_history.push_back(spread_bps);
                    while state.basket_spread_history.len() > HISTORY_CAP {
                        state.basket_spread_history.pop_front();
                    }
                }
            }
        }

        // Daily equity snapshot (cash + direction-aware mark-to-market of open positions).
        if day_key != last_snapshot_day {
            last_snapshot_day = day_key.to_string();
            let mtm: f64 = positions
                .iter()
                .map(|(sym, p)| {
                    let last = last_price.get(sym).copied().unwrap_or(p.entry_price);
                    match p.direction {
                        // Subtract `borrowed` - cash was only reduced by the margin
                        // portion at entry, so equity must also net out the liability of
                        // the borrowed portion still owed, or a leveraged position's
                        // mark-to-market equity is overstated by exactly that amount.
                        Direction::Long => last * p.qty - p.borrowed,
                        Direction::Short => -(last * p.qty),
                    }
                })
                .sum();
            equity_curve.push(EquitySnapshot {
                timestamp: ev.timestamp.to_string(),
                equity: cash + mtm,
            });
        }

        if positions.contains_key(ev.symbol) {
            continue;
        }
        if is_excluded_weekday(ev.timestamp, &excluded_weekdays) {
            continue;
        }

        let spread_bps = asset_class.assumed_spread_bps(ev.symbol);
        let half_spread = mid * (spread_bps / 20_000.0);
        let bid = mid - half_spread;
        let ask = mid + half_spread;

        let mut extra = ExtraContext::default();
        match mode {
            Mode::Orb => {
                extra.opening_range_high = if state.orb_bars_seen >= strategies.orb.range_bars {
                    state.orb_range_high
                } else {
                    None
                };
            }
            Mode::BasketArb => {
                let series: Vec<f64> = state.basket_spread_history.iter().copied().collect();
                extra.basket_spread_z = match (sma(&series, strategies.basket_arb.z_period), stddev(&series, strategies.basket_arb.z_period)) {
                    (Some(mean), Some(sd)) if sd > 0.0 => series.last().map(|last| (last - mean) / sd),
                    _ => None,
                };
            }
            Mode::NewsSentiment => {
                if let Ok(now) = DateTime::parse_from_rfc3339(ev.timestamp) {
                    extra.news_sentiment = news_sentiment_at(
                        &news_index, ev.symbol, now.with_timezone(&Utc), strategies.news_sentiment.lookback_hours,
                    );
                }
            }
            Mode::MlSignal => {
                let closes: Vec<f64> = state.mids.iter().copied().collect();
                let volumes: Vec<f64> = state.volumes.iter().copied().collect();
                extra.ml_features = ml_features(&closes, &volumes);
                extra.ml_model = ml_model.as_ref();
            }
            _ => {}
        }

        let Some(direction) = evaluate_entry(mode, bid, ask, state, &config.hft, &fees, tp_pct, sl_pct, min_agree, &strategies, &extra) else {
            continue;
        };

        if enforce_pdt && asset_class == AssetClass::Stocks {
            let equity: f64 = cash
                + positions
                    .iter()
                    .map(|(sym, p)| {
                        let last = last_price.get(sym).copied().unwrap_or(p.entry_price);
                        match p.direction {
                            Direction::Long => last * p.qty - p.borrowed,
                            Direction::Short => -(last * p.qty),
                        }
                    })
                    .sum::<f64>();
            if equity < pdt_equity_threshold {
                let day_trades_in_window = day_trade_log
                    .iter()
                    .filter(|d| recent_trading_days.contains(d))
                    .count();
                if day_trades_in_window >= 3 {
                    blocked_new_entries_pdt += 1;
                    continue; // would risk becoming the 4th day trade in the rolling window - refuse the entry entirely, the only way an automated system can guarantee it never trips PDT status
                }
            }
        }

        let leverage = if direction == Direction::Long { leverage_multiplier } else { 1.0 };
        let target_notional = (cash * target_balance_pct * leverage).clamp(0.0, max_order_amount * leverage);
        let margin_used = target_notional / leverage;
        if margin_used < MIN_ORDER_AMOUNT || margin_used > cash {
            continue; // not enough cash to open a meaningfully-sized position
        }
        let borrowed = target_notional - margin_used;

        let entry_price = match direction {
            Direction::Long => ask,
            Direction::Short => bid,
        };
        let qty = target_notional / entry_price;
        let (take_profit, stop_loss) = match direction {
            Direction::Long => (entry_price * (1.0 + tp_pct / 100.0), entry_price * (1.0 - sl_pct / 100.0)),
            Direction::Short => (entry_price * (1.0 - tp_pct / 100.0), entry_price * (1.0 + sl_pct / 100.0)),
        };

        match direction {
            Direction::Long => cash -= margin_used,
            Direction::Short => cash += target_notional,
        }
        positions.insert(
            ev.symbol.to_string(),
            OpenPosition {
                direction,
                entry_price,
                qty,
                take_profit,
                stop_loss,
                entry_time: ev.timestamp.to_string(),
                cost_basis: target_notional,
                borrowed,
                peak_price: entry_price,
                trailing_active: false,
            },
        );
    }

    // Close out any still-open positions at the last known price (mark-to-market exit).
    let last_event_time = events.last().map(|e| e.timestamp).unwrap_or("");
    for (symbol, pos) in &positions {
        let last = last_price.get(symbol).copied().unwrap_or(pos.entry_price);
        let fee_cost = pos.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
        let holding_days = days_between(&pos.entry_time, last_event_time);
        let interest_cost = pos.borrowed * (strategies.margin.annual_margin_rate_pct / 100.0) * (holding_days / 365.0);
        let pnl = match pos.direction {
            Direction::Long => {
                let proceeds = last * pos.qty - fee_cost;
                cash += proceeds - pos.borrowed - interest_cost;
                proceeds - pos.cost_basis - interest_cost
            }
            Direction::Short => {
                let cost_to_cover = last * pos.qty + fee_cost;
                cash -= cost_to_cover;
                pos.cost_basis - cost_to_cover
            }
        };
        trades.push(ClosedTrade {
            symbol: symbol.clone(),
            direction: pos.direction.label(),
            entry_time: pos.entry_time.clone(),
            exit_time: "backtest_end".to_string(),
            entry_price: pos.entry_price,
            exit_price: last,
            qty: pos.qty,
            exit_reason: "still_open_at_backtest_end",
            pnl,
            cash_after: cash,
        });
    }

    let final_value = cash;
    let total_return_pct = (final_value - starting_cash) / starting_cash * 100.0;
    let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
    let win_rate = if !trades.is_empty() { wins as f64 / trades.len() as f64 * 100.0 } else { 0.0 };

    // Buy-and-hold baseline: split starting_cash equally across every symbol that has at
    // least one bar in range, buy at its first close, mark-to-market at its last close.
    let bh_symbols: Vec<&(String, Vec<CryptoBar>)> =
        bars_by_symbol.iter().filter(|(_, bars)| !bars.is_empty()).collect();
    let bh_per_symbol = if bh_symbols.is_empty() { 0.0 } else { starting_cash / bh_symbols.len() as f64 };
    let bh_final_value: f64 = bh_symbols
        .iter()
        .map(|(_, bars)| {
            let first = bars.first().unwrap().close;
            let last = bars.last().unwrap().close;
            if first <= 0.0 { bh_per_symbol } else { bh_per_symbol * (last / first) }
        })
        .sum();
    let bh_return_pct = if starting_cash > 0.0 { (bh_final_value - starting_cash) / starting_cash * 100.0 } else { 0.0 };
    let excess_vs_bh = final_value - bh_final_value;

    println!();
    println!("===== Portfolio Result =====");
    println!("Starting cash:    ${:.2}", starting_cash);
    println!("Ending value:     ${:.2}", final_value);
    println!("Total return:     {:.2}%", total_return_pct);
    println!("Total trades:     {}", trades.len());
    println!("Win rate:         {:.1}%", win_rate);
    if enforce_pdt {
        println!("Entries blocked by PDT limit: {}", blocked_new_entries_pdt);
    }
    println!();
    println!("Buy & hold ({} symbols, equal-weight): ${:.2} ({:.2}%)", bh_symbols.len(), bh_final_value, bh_return_pct);
    println!("Excess vs buy & hold: ${:.2}", excess_vs_bh);

    // Simple monthly equity checkpoints for a readable growth trajectory.
    if !equity_curve.is_empty() {
        println!();
        println!("Equity curve (every ~10th daily snapshot):");
        for (i, snap) in equity_curve.iter().enumerate() {
            if i % 10 == 0 || i == equity_curve.len() - 1 {
                println!("  {}  ${:.2}", &snap.timestamp[..10.min(snap.timestamp.len())], snap.equity);
            }
        }
    }

    std::fs::create_dir_all("data").ok();
    let out_path = format!(
        "data/portfolio_{}_{}_{}_{}d_offset{}.json",
        asset_class.label(), mode.label().replace('-', "_"), starting_cash as i64, lookback_days, offset_days
    );
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "mode": mode.label(),
            "min_agree": if mode == Mode::Combined { Some(min_agree) } else { None },
            "leverage_multiplier": leverage_multiplier,
            "asset_class": asset_class.label(),
            "starting_cash": starting_cash,
            "final_value": final_value,
            "total_return_pct": total_return_pct,
            "total_trades": trades.len(),
            "win_rate_pct": win_rate,
            "pdt_enforced": enforce_pdt,
            "entries_blocked_by_pdt": blocked_new_entries_pdt,
            "buy_and_hold_final_value": bh_final_value,
            "buy_and_hold_return_pct": bh_return_pct,
            "excess_vs_buy_and_hold": excess_vs_bh,
            "equity_curve": equity_curve,
            "trades": trades,
        }))?,
    )?;
    println!();
    println!("Full results written to {}", out_path);

    Ok(())
}
