//! Backtests the live HFT entry logic (`evaluate_hft_signal`, the exact function the
//! deployed strategy uses) against historical Alpaca crypto bars, to get a profitability
//! signal without waiting on live paper-trading data to accumulate.
//!
//! ## Known approximations (read before trusting the numbers)
//! - Alpaca's historical crypto bars are OHLCV only - no historical bid/ask. Spread is
//!   approximated with a fixed per-symbol assumption (see `assumed_spread_bps`), not
//!   measured. Real spreads spike during volatility and on thin books (SHIB/DOGE/PEPE
//!   especially) in ways this can't capture.
//! - The live volume-confirmation gate compares a trade's size to the recent average
//!   trade size; here it compares each bar's aggregate volume to the recent average bar
//!   volume instead. Same shape of check, different unit, since only bar-level volume is
//!   available historically.
//! - One bar = one evaluation tick. The live strategy re-evaluates on every quote
//!   (potentially many per second); this only sees 1-minute resolution, so it will miss
//!   momentum bursts that happen and reverse within a bar.
//! - Only one open lot per symbol is simulated (position stacking is not modeled).
//! - Fixed notional per trade (`defaults.max_order_amount`), not compounding equity.
//! - When a bar's range touches both take-profit and stop-loss, stop-loss is assumed to
//!   fill first (the conservative assumption, per this project's own convention of never
//!   crediting the best case when the fill order is unknown).
//!
//! Treat this as a first-pass sanity check on the net-edge gate and TP/SL sizing, not a
//! guarantee of live performance.

use chrono::{Duration as ChronoDuration, Utc};
use rust_autohedge::backtest_support::{
    fetch_all_bars, liquid_stock_universe, max_drawdown, AssetClass, Trade,
};
use rust_autohedge::config::{AppConfig, FeeConfig, HftConfig};
use rust_autohedge::data::alpaca::{AlpacaClient, CryptoBar};
use rust_autohedge::services::strategy::{
    evaluate_bollinger_signal, evaluate_donchian_breakout_signal, evaluate_hft_signal,
    evaluate_ma_crossover_signal, evaluate_mean_reversion_signal, evaluate_rsi_signal, HftSignal,
    SignalDecision,
};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

const DEFAULT_LOOKBACK_DAYS: i64 = 14;
const DEFAULT_TIMEFRAME: &str = "1Min";
// Must exceed the longest indicator period tested below (MA_SLOW_PERIOD=55) or that
// signal silently gets zero history and never fires - matches the live strategy's cap of
// 30 only when signals with longer lookbacks aren't in play.
const HISTORY_CAP: usize = 64;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Buy on upward momentum, betting the move continues (the live strategy).
    Momentum,
    /// Buy on a short-term dip, betting price reverts back up.
    Reversion,
    /// Buy when RSI shows oversold.
    Rsi,
    /// Buy when price is N stddev below its own moving average.
    Bollinger,
    /// Buy on a fast/slow moving-average golden cross.
    MaCross,
    /// Buy on a breakout above the prior N-period high.
    Donchian,
    /// Cross-symbol: buy whichever symbol is lagging furthest behind the basket's average
    /// return, betting it catches up. Genuinely different information from any
    /// single-symbol signal above - needs all symbols' data jointly, not just one.
    RelStrengthLaggard,
    /// Cross-symbol: buy whichever symbol is leading furthest ahead of the basket's
    /// average return, betting relative strength persists.
    RelStrengthLeader,
}

impl Mode {
    fn from_args() -> Self {
        let mode_arg = std::env::args()
            .skip_while(|a| a != "--mode")
            .nth(1)
            .unwrap_or_default();
        match mode_arg.as_str() {
            "reversion" => Mode::Reversion,
            "rsi" => Mode::Rsi,
            "bollinger" => Mode::Bollinger,
            "ma_cross" => Mode::MaCross,
            "donchian" => Mode::Donchian,
            "rel_strength_laggard" => Mode::RelStrengthLaggard,
            "rel_strength_leader" => Mode::RelStrengthLeader,
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
            Mode::RelStrengthLaggard => "rel-strength-laggard",
            Mode::RelStrengthLeader => "rel-strength-leader",
        }
    }

    fn is_cross_symbol(&self) -> bool {
        matches!(self, Mode::RelStrengthLaggard | Mode::RelStrengthLeader)
    }
}

const RS_LOOKBACK_BARS: usize = 10; // matches momentum_lookback's default window
const RS_ENTRY_THRESHOLD_BPS: f64 = 100.0; // how far from basket avg return triggers a buy

// Tightened from textbook defaults (RSI 14/30, Bollinger 20/2.0, MA 9/21, Donchian 20):
// a first pass at defaults fired 1400+ trades over 60 days on 1-minute bars (vs 531 for
// mean-reversion), drowning any real signal in fee drag from over-trading. These are
// stricter so the comparison to mean-reversion isn't skewed by raw trade frequency alone.
const RSI_PERIOD: usize = 14;
const RSI_OVERSOLD: f64 = 20.0;
const BOLLINGER_PERIOD: usize = 20;
const BOLLINGER_Z: f64 = 3.0;
const MA_FAST_PERIOD: usize = 21;
const MA_SLOW_PERIOD: usize = 55;
const DONCHIAN_PERIOD: usize = 50;

fn lookback_days_from_args() -> i64 {
    std::env::args()
        .skip_while(|a| a != "--days")
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_LOOKBACK_DAYS)
}

fn timeframe_from_args() -> String {
    std::env::args()
        .skip_while(|a| a != "--timeframe")
        .nth(1)
        .unwrap_or_else(|| DEFAULT_TIMEFRAME.to_string())
}

fn offset_days_from_args() -> i64 {
    std::env::args()
        .skip_while(|a| a != "--offset-days")
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

/// Fixed TP/SL override for out-of-sample validation: apply a specific combo (e.g. one
/// found best on an in-sample window) uniformly across all symbols, instead of
/// config.yaml's per-symbol defaults/overrides. `None` falls back to
/// `config.get_symbol_params` as before.
fn fixed_tp_sl_from_args() -> Option<(f64, f64)> {
    let tp = std::env::args()
        .skip_while(|a| a != "--tp-pct")
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())?;
    let sl = std::env::args()
        .skip_while(|a| a != "--sl-pct")
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())?;
    Some((tp, sl))
}

fn asset_class_from_args() -> AssetClass {
    std::env::args()
        .skip_while(|a| a != "--asset-class")
        .nth(1)
        .map(|s| AssetClass::from_arg(&s))
        .unwrap_or(AssetClass::Crypto)
}

/// When set (via `--combine-dip-bps N`), combines relative-strength with a second,
/// independent requirement: the symbol must also be down at least N bps in absolute
/// terms, not just underperforming the basket. Only meaningful with
/// `--mode rel_strength_laggard`.
fn combine_dip_bps_from_args() -> Option<f64> {
    std::env::args()
        .skip_while(|a| a != "--combine-dip-bps")
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
}

struct OpenPosition {
    entry_time: String,
    entry_price: f64,
    qty: f64,
    take_profit: f64,
    stop_loss: f64,
}

struct SymbolState {
    mids: VecDeque<f64>,
    volumes: VecDeque<f64>,
    quotes_since_eval: usize,
    open: Option<OpenPosition>,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            mids: VecDeque::with_capacity(64),
            volumes: VecDeque::with_capacity(64),
            quotes_since_eval: 0,
            open: None,
        }
    }
}

fn simulate_symbol(
    symbol: &str,
    bars: &[CryptoBar],
    hft: &HftConfig,
    fees: &FeeConfig,
    tp_pct: f64,
    sl_pct: f64,
    trade_notional: f64,
    evaluate_every: usize,
    mode: Mode,
    asset_class: AssetClass,
) -> Vec<Trade> {
    let spread_bps = asset_class.assumed_spread_bps(symbol);
    let mut state = SymbolState::new();
    let mut trades = Vec::new();

    for bar in bars {
        let mid = bar.close;
        if mid <= 0.0 {
            continue;
        }
        let half_spread = mid * (spread_bps / 10_000.0) / 2.0;
        let bid = mid - half_spread;
        let ask = mid + half_spread;

        // ===== Manage an open position first: check SL/TP against this bar's range =====
        if let Some(pos) = &state.open {
            let hit_tp = bar.high >= pos.take_profit;
            let hit_sl = bar.low <= pos.stop_loss;

            let exit = if hit_sl {
                // Conservative: if both are touched in the same bar, assume SL filled first.
                Some((pos.stop_loss, "stop_loss"))
            } else if hit_tp {
                Some((pos.take_profit, "take_profit"))
            } else {
                None
            };

            if let Some((exit_price, reason)) = exit {
                let entry_notional = pos.entry_price * pos.qty;
                let fee_cost = entry_notional * (fees.round_trip_fee_bps / 10_000.0);
                let pnl = (exit_price - pos.entry_price) * pos.qty - fee_cost;

                trades.push(Trade {
                    symbol: symbol.to_string(),
                    entry_time: pos.entry_time.clone(),
                    entry_price: pos.entry_price,
                    exit_time: bar.timestamp.clone(),
                    exit_price,
                    qty: pos.qty,
                    exit_reason: reason,
                    pnl,
                });
                state.open = None;
            }
        }

        // ===== Push this tick into history (same bookkeeping as the live strategy) =====
        state.quotes_since_eval += 1;
        state.mids.push_back(mid);
        while state.mids.len() > HISTORY_CAP {
            state.mids.pop_front();
        }
        // Bar volume stands in for the live strategy's rolling trade-size window - both
        // feed the same volume_ratio() confirmation check in evaluate_hft_signal.
        state.volumes.push_back(bar.volume);
        while state.volumes.len() > HISTORY_CAP {
            state.volumes.pop_front();
        }

        if state.open.is_some() {
            continue; // Already in a position - don't stack (see module doc).
        }
        if state.quotes_since_eval < evaluate_every {
            continue;
        }
        state.quotes_since_eval = 0;

        // ===== Same entry decision the live strategy makes =====
        // simulate_symbol computes its own take_profit/stop_loss from tp_pct/sl_pct below,
        // so all that's needed from any signal here is the buy/no-buy boolean.
        let is_buy = match mode {
            Mode::Momentum => matches!(
                evaluate_hft_signal(bid, ask, &state.mids, &state.volumes, hft, fees),
                HftSignal::Buy { .. }
            ),
            Mode::Reversion => matches!(
                evaluate_mean_reversion_signal(bid, ask, &state.mids, &state.volumes, hft, fees),
                HftSignal::Buy { .. }
            ),
            Mode::Rsi => matches!(
                evaluate_rsi_signal(
                    bid,
                    ask,
                    &state.mids,
                    RSI_PERIOD,
                    RSI_OVERSOLD,
                    hft.max_spread_bps,
                    tp_pct,
                    sl_pct,
                ),
                SignalDecision::Buy { .. }
            ),
            Mode::Bollinger => matches!(
                evaluate_bollinger_signal(
                    bid,
                    ask,
                    &state.mids,
                    BOLLINGER_PERIOD,
                    BOLLINGER_Z,
                    hft.max_spread_bps,
                    tp_pct,
                    sl_pct,
                ),
                SignalDecision::Buy { .. }
            ),
            Mode::MaCross => matches!(
                evaluate_ma_crossover_signal(
                    bid,
                    ask,
                    &state.mids,
                    MA_FAST_PERIOD,
                    MA_SLOW_PERIOD,
                    hft.max_spread_bps,
                    tp_pct,
                    sl_pct,
                ),
                SignalDecision::Buy { .. }
            ),
            Mode::Donchian => matches!(
                evaluate_donchian_breakout_signal(
                    bid,
                    ask,
                    &state.mids,
                    DONCHIAN_PERIOD,
                    hft.max_spread_bps,
                    tp_pct,
                    sl_pct,
                ),
                SignalDecision::Buy { .. }
            ),
            Mode::RelStrengthLaggard | Mode::RelStrengthLeader => {
                unreachable!("cross-symbol modes are dispatched to simulate_relative_strength, never simulate_symbol")
            }
        };
        if is_buy {
            // Simulate an aggressive buy fill at the ask (matches live's bias toward the
            // market side to get filled fast).
            let entry_price = ask;
            let qty = trade_notional / entry_price;
            let take_profit = entry_price * (1.0 + tp_pct / 100.0);
            let stop_loss = entry_price * (1.0 - sl_pct / 100.0);

            state.open = Some(OpenPosition {
                entry_time: bar.timestamp.clone(),
                entry_price,
                qty,
                take_profit,
                stop_loss,
            });
        }
    }

    // Mark-to-market any position still open at the end of the window.
    if let (Some(pos), Some(last_bar)) = (&state.open, bars.last()) {
        let entry_notional = pos.entry_price * pos.qty;
        let fee_cost = entry_notional * (fees.round_trip_fee_bps / 10_000.0);
        let pnl = (last_bar.close - pos.entry_price) * pos.qty - fee_cost;
        trades.push(Trade {
            symbol: symbol.to_string(),
            entry_time: pos.entry_time.clone(),
            entry_price: pos.entry_price,
            exit_time: last_bar.timestamp.clone(),
            exit_price: last_bar.close,
            qty: pos.qty,
            exit_reason: "still_open_at_backtest_end",
            pnl,
        });
    }

    trades
}

#[derive(Serialize)]
struct SymbolSummary {
    symbol: String,
    trades: usize,
    wins: usize,
    losses: usize,
    win_rate_pct: f64,
    total_pnl: f64,
    avg_pnl: f64,
}

fn summarize(symbol: &str, trades: &[Trade]) -> SymbolSummary {
    let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
    let losses = trades.iter().filter(|t| t.pnl <= 0.0).count();
    let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
    let win_rate_pct = if trades.is_empty() {
        0.0
    } else {
        wins as f64 / trades.len() as f64 * 100.0
    };
    let avg_pnl = if trades.is_empty() {
        0.0
    } else {
        total_pnl / trades.len() as f64
    };
    SymbolSummary {
        symbol: symbol.to_string(),
        trades: trades.len(),
        wins,
        losses,
        win_rate_pct,
        total_pnl,
        avg_pnl,
    }
}

struct MergedEvent {
    timestamp: String,
    symbol: String,
    close: f64,
    high: f64,
    low: f64,
}

/// Merges every symbol's bars into one chronological stream. RFC3339 timestamps sort
/// correctly as plain strings, so no parsing is needed just to order them.
fn merge_bars_chronologically(bars_by_symbol: &[(String, Vec<CryptoBar>)]) -> Vec<MergedEvent> {
    let mut events = Vec::new();
    for (symbol, bars) in bars_by_symbol {
        for bar in bars {
            events.push(MergedEvent {
                timestamp: bar.timestamp.clone(),
                symbol: symbol.clone(),
                close: bar.close,
                high: bar.high,
                low: bar.low,
            });
        }
    }
    events.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    events
}

/// Cross-symbol relative-strength backtest: at each bar, tracks every symbol's own
/// `RS_LOOKBACK_BARS`-bar return, compares it to the average return across all symbols
/// with a fresh reading at that point, and buys whichever symbol is furthest from that
/// basket average (in the laggard or leader direction, per `buy_laggards`) by at least
/// `RS_ENTRY_THRESHOLD_BPS`. Unlike `simulate_symbol`, this can't run one symbol at a
/// time - the entry decision for any single symbol depends on every other symbol's recent
/// performance, so all symbols' data has to be walked jointly in time order.
fn simulate_relative_strength(
    bars_by_symbol: &[(String, Vec<CryptoBar>)],
    fees: &FeeConfig,
    trade_notional: f64,
    tp_pct: f64,
    sl_pct: f64,
    buy_laggards: bool,
    // Combines a second, independent signal on top of relative strength: additionally
    // require the symbol's OWN return to be below -own_dip_threshold_bps (i.e. genuinely
    // declining in absolute terms, not just underperforming a basket that might itself be
    // flat or rising). `None` disables this and reproduces plain relative-strength.
    require_own_dip_bps: Option<f64>,
    asset_class: AssetClass,
) -> Vec<Trade> {
    let events = merge_bars_chronologically(bars_by_symbol);

    let mut price_history: HashMap<String, VecDeque<f64>> = HashMap::new();
    let mut last_return_bps: HashMap<String, f64> = HashMap::new();
    let mut open_positions: HashMap<String, OpenPosition> = HashMap::new();
    let mut trades = Vec::new();

    for ev in &events {
        // Manage an existing open position for this symbol first, using this bar's range.
        if let Some(pos) = open_positions.get(&ev.symbol) {
            let hit_sl = ev.low <= pos.stop_loss;
            let hit_tp = ev.high >= pos.take_profit;
            let exit = if hit_sl {
                Some((pos.stop_loss, "stop_loss"))
            } else if hit_tp {
                Some((pos.take_profit, "take_profit"))
            } else {
                None
            };
            if let Some((exit_price, reason)) = exit {
                let entry_notional = pos.entry_price * pos.qty;
                let fee_cost = entry_notional * (fees.round_trip_fee_bps / 10_000.0);
                let pnl = (exit_price - pos.entry_price) * pos.qty - fee_cost;
                trades.push(Trade {
                    symbol: ev.symbol.clone(),
                    entry_time: pos.entry_time.clone(),
                    entry_price: pos.entry_price,
                    exit_time: ev.timestamp.clone(),
                    exit_price,
                    qty: pos.qty,
                    exit_reason: reason,
                    pnl,
                });
                open_positions.remove(&ev.symbol);
            }
        }

        let hist = price_history.entry(ev.symbol.clone()).or_default();
        hist.push_back(ev.close);
        while hist.len() > RS_LOOKBACK_BARS + 1 {
            hist.pop_front();
        }
        if hist.len() >= RS_LOOKBACK_BARS + 1 {
            let past = hist[0];
            let current = *hist.back().unwrap();
            if past > 0.0 {
                last_return_bps.insert(ev.symbol.clone(), ((current - past) / past) * 10_000.0);
            }
        }

        if open_positions.contains_key(&ev.symbol) {
            continue;
        }
        // Need several symbols with a fresh return reading for "average" to mean anything.
        if last_return_bps.len() < 3 {
            continue;
        }
        let Some(&this_return) = last_return_bps.get(&ev.symbol) else {
            continue;
        };
        let basket_avg: f64 =
            last_return_bps.values().sum::<f64>() / last_return_bps.len() as f64;
        let relative_bps = this_return - basket_avg;

        let should_buy = if buy_laggards {
            relative_bps <= -RS_ENTRY_THRESHOLD_BPS
        } else {
            relative_bps >= RS_ENTRY_THRESHOLD_BPS
        };
        if !should_buy {
            continue;
        }
        if let Some(own_dip_bps) = require_own_dip_bps {
            if this_return > -own_dip_bps {
                continue; // relative-strength condition met, but not an absolute dip too
            }
        }

        // Simulate an aggressive buy fill using the same per-symbol spread assumption as
        // simulate_symbol, applied as an offset from the close since there's no separate
        // bid/ask here.
        let spread_bps = asset_class.assumed_spread_bps(&ev.symbol);
        let entry_price = ev.close * (1.0 + spread_bps / 20_000.0);
        let qty = trade_notional / entry_price;
        open_positions.insert(
            ev.symbol.clone(),
            OpenPosition {
                entry_time: ev.timestamp.clone(),
                entry_price,
                qty,
                take_profit: entry_price * (1.0 + tp_pct / 100.0),
                stop_loss: entry_price * (1.0 - sl_pct / 100.0),
            },
        );
    }

    // Mark-to-market any positions still open at the end, using each symbol's last price.
    for (symbol, pos) in &open_positions {
        if let Some(last_price) = price_history.get(symbol).and_then(|h| h.back()) {
            let entry_notional = pos.entry_price * pos.qty;
            let fee_cost = entry_notional * (fees.round_trip_fee_bps / 10_000.0);
            let pnl = (last_price - pos.entry_price) * pos.qty - fee_cost;
            trades.push(Trade {
                symbol: symbol.clone(),
                entry_time: pos.entry_time.clone(),
                entry_price: pos.entry_price,
                exit_time: "backtest_end".to_string(),
                exit_price: *last_price,
                qty: pos.qty,
                exit_reason: "still_open_at_backtest_end",
                pnl,
            });
        }
    }

    trades
}

/// Runs `simulate_symbol` for every symbol at one (tp_pct, sl_pct) combo (applied
/// uniformly - ignores config.yaml's per-symbol overrides, since the sweep is asking "is
/// this ratio better in general," not tuning each symbol individually) and returns the
/// pooled trades across all symbols.
fn run_combo(
    bars_by_symbol: &[(String, Vec<CryptoBar>)],
    hft: &HftConfig,
    fees: &FeeConfig,
    trade_notional: f64,
    evaluate_every: usize,
    mode: Mode,
    tp_pct: f64,
    sl_pct: f64,
    asset_class: AssetClass,
) -> Vec<Trade> {
    let mut all_trades = Vec::new();
    for (symbol, bars) in bars_by_symbol {
        let trades = simulate_symbol(
            symbol,
            bars,
            hft,
            fees,
            tp_pct,
            sl_pct,
            trade_notional,
            evaluate_every,
            mode,
            asset_class,
        );
        all_trades.extend(trades);
    }
    all_trades
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let mode = Mode::from_args();
    let lookback_days = lookback_days_from_args();
    let timeframe = timeframe_from_args();
    let asset_class = asset_class_from_args();
    let sweep = std::env::args().any(|a| a == "--sweep");
    let scan = std::env::args().any(|a| a == "--scan");
    let combine_dip_bps = combine_dip_bps_from_args();
    let config = AppConfig::load();
    let client = AlpacaClient::new(config.alpaca.clone(), config.history_limit);

    // Stocks use config.yaml's stock_symbols (falling back to the hardcoded liquid
    // universe if unset) and stock-appropriate fee assumptions instead of config.yaml's
    // crypto-tuned symbols/fees (Alpaca stock trading is commission-free, unlike crypto's
    // ~50bps assumed taker fee - see AssetClass::default_round_trip_fee_bps).
    let symbols = match asset_class {
        AssetClass::Crypto => config.symbols.clone(),
        AssetClass::Stocks => config.stock_symbols.clone().unwrap_or_else(liquid_stock_universe),
    };
    let fees = match asset_class {
        AssetClass::Crypto => config.fees.clone(),
        AssetClass::Stocks => FeeConfig {
            round_trip_fee_bps: asset_class.default_round_trip_fee_bps(),
        },
    };

    // offset_days shifts the whole window back in time without changing its length -
    // e.g. --days 60 --offset-days 60 fetches the 60-day window ending 60 days ago,
    // non-overlapping with a plain --days 60 run. Exists specifically for out-of-sample
    // validation: tune parameters on one window, then check whether they still work on a
    // window the tuning never saw.
    let offset_days = offset_days_from_args();
    let end = Utc::now() - ChronoDuration::days(offset_days);
    let start = end - ChronoDuration::days(lookback_days);
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();

    if scan {
        // Best-known combo from the TP/SL sweep on the original 7 symbols - reused here
        // to ask a different question: does this same strategy have real edge on OTHER
        // Alpaca-tradeable crypto/USD pairs, including majors (BTC/ETH) the live config
        // dropped in favor of meme coins.
        const SCAN_TP_PCT: f64 = 3.0;
        const SCAN_SL_PCT: f64 = 1.5;

        println!("Fetching tradable crypto assets from Alpaca...");
        let assets = client.get_assets(Some("crypto".to_string())).await?;
        let mut symbols: Vec<String> = assets
            .iter()
            .filter(|a| a.get("tradable").and_then(|t| t.as_bool()).unwrap_or(false))
            .filter_map(|a| a.get("symbol").and_then(|s| s.as_str()))
            .filter(|s| s.ends_with("/USD"))
            .map(String::from)
            .collect();
        symbols.sort();
        symbols.dedup();
        println!(
            "Found {} tradable crypto/USD pairs: {:?}",
            symbols.len(),
            symbols
        );
        println!();

        let bars_by_symbol =
            fetch_all_bars(&client, AssetClass::Crypto, &symbols, &timeframe, &start_str, &end_str)
                .await;

        println!();
        println!(
            "===== Symbol Scan (mode={}, timeframe={}, tp={:.1}% sl={:.1}%, {} days) =====",
            mode.label(),
            timeframe,
            SCAN_TP_PCT,
            SCAN_SL_PCT,
            lookback_days
        );
        println!(
            "{:<10} {:>8} {:>10} {:>12} {:>12} {:>10}",
            "Symbol", "Trades", "WinRate%", "TotalPnL$", "AvgPnL$", "InConfig?"
        );

        let mut results: Vec<(String, usize, f64, f64, f64)> = Vec::new(); // symbol, n, win_rate, total_pnl, avg_pnl
        for (symbol, bars) in &bars_by_symbol {
            let trades = simulate_symbol(
                symbol,
                bars,
                &config.hft,
                &config.fees,
                SCAN_TP_PCT,
                SCAN_SL_PCT,
                config.defaults.max_order_amount,
                config.hft.evaluate_every_quotes,
                mode,
                AssetClass::Crypto,
            );
            let n = trades.len();
            let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
            let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
            let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };
            let avg_pnl = if n > 0 { total_pnl / n as f64 } else { 0.0 };
            results.push((symbol.clone(), n, win_rate, total_pnl, avg_pnl));
        }

        results.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
        for (symbol, n, win_rate, total_pnl, avg_pnl) in &results {
            let in_config = if config.symbols.contains(symbol) { "yes" } else { "" };
            let verdict = if *total_pnl > 0.0 { " <-- PROFITABLE" } else { "" };
            println!(
                "{:<10} {:>8} {:>10.1} {:>12.2} {:>12.4} {:>10}{}",
                symbol, n, win_rate, total_pnl, avg_pnl, in_config, verdict
            );
        }

        let profitable: Vec<_> = results.iter().filter(|r| r.3 > 0.0).collect();
        println!();
        println!(
            "{} of {} symbols profitable at this combo.",
            profitable.len(),
            results.len()
        );

        return Ok(());
    }

    println!(
        "Backtesting {} symbols [mode={}] over the last {} days ({} -> {})",
        symbols.len(),
        mode.label(),
        lookback_days,
        start_str,
        end_str
    );
    if let Some(dip_bps) = combine_dip_bps {
        println!(
            "combined condition: also requires own return <= -{:.0} bps (not just relative-strength)",
            dip_bps
        );
    }
    println!(
        "asset_class={} hft: min_edge_bps={} min_net_edge_bps={} max_spread_bps={} fees.round_trip_fee_bps={}",
        asset_class.label(),
        config.hft.min_edge_bps,
        config.hft.min_net_edge_bps,
        config.hft.max_spread_bps,
        fees.round_trip_fee_bps
    );
    println!();

    let bars_by_symbol =
        fetch_all_bars(&client, asset_class, &symbols, &timeframe, &start_str, &end_str).await;

    if sweep {
        println!();
        println!("===== TP/SL Sweep (mode={}) =====", mode.label());
        println!(
            "{:>8} {:>8} {:>8} {:>10} {:>10} {:>12} {:>12} {:>14}",
            "TP%", "SL%", "Trades", "WinRate%", "Breakeven%", "TotalPnL$", "AvgPnL$", "Verdict"
        );

        let tp_candidates = [1.0, 1.2, 1.5, 1.8, 2.0, 2.5, 3.0];
        let sl_candidates = [0.4, 0.5, 0.7, 1.0, 1.3, 1.5];

        let mut best: Option<(f64, f64, f64)> = None; // (tp, sl, total_pnl)

        for &tp_pct in &tp_candidates {
            for &sl_pct in &sl_candidates {
                let trades = if mode.is_cross_symbol() {
                    simulate_relative_strength(
                        &bars_by_symbol,
                        &fees,
                        config.defaults.max_order_amount,
                        tp_pct,
                        sl_pct,
                        mode == Mode::RelStrengthLaggard,
                        combine_dip_bps,
                        asset_class,
                    )
                } else {
                    run_combo(
                        &bars_by_symbol,
                        &config.hft,
                        &fees,
                        config.defaults.max_order_amount,
                        config.hft.evaluate_every_quotes,
                        mode,
                        tp_pct,
                        sl_pct,
                        asset_class,
                    )
                };
                let n = trades.len();
                let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
                let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
                let win_rate = if n > 0 {
                    wins as f64 / n as f64 * 100.0
                } else {
                    0.0
                };
                let breakeven = sl_pct / (sl_pct + tp_pct) * 100.0;
                let avg_pnl = if n > 0 { total_pnl / n as f64 } else { 0.0 };
                let verdict = if total_pnl > 0.0 { "PROFITABLE" } else { "" };

                println!(
                    "{:>8.1} {:>8.1} {:>8} {:>10.1} {:>10.1} {:>12.2} {:>12.4} {:>14}",
                    tp_pct, sl_pct, n, win_rate, breakeven, total_pnl, avg_pnl, verdict
                );

                if best.is_none() || total_pnl > best.unwrap().2 {
                    best = Some((tp_pct, sl_pct, total_pnl));
                }
            }
        }

        if let Some((tp, sl, pnl)) = best {
            println!();
            println!(
                "Best combo: take_profit_pct={:.1} stop_loss_pct={:.1} -> total_pnl=${:.2}",
                tp, sl, pnl
            );
        }

        return Ok(());
    }

    let mut all_trades: Vec<Trade> = Vec::new();
    let mut summaries: Vec<SymbolSummary> = Vec::new();

    if mode.is_cross_symbol() {
        // Needs all symbols' data jointly, not one at a time - see simulate_relative_strength.
        all_trades = simulate_relative_strength(
            &bars_by_symbol,
            &fees,
            config.defaults.max_order_amount,
            config.defaults.take_profit_pct,
            config.defaults.stop_loss_pct,
            mode == Mode::RelStrengthLaggard,
            combine_dip_bps,
            asset_class,
        );
        for (symbol, _) in &bars_by_symbol {
            let symbol_trades: Vec<Trade> =
                all_trades.iter().filter(|t| &t.symbol == symbol).cloned().collect();
            if !symbol_trades.is_empty() {
                summaries.push(summarize(symbol, &symbol_trades));
            }
        }
    } else {
        let fixed_tp_sl = fixed_tp_sl_from_args();
        for (symbol, bars) in &bars_by_symbol {
            let (tp_pct, sl_pct) =
                fixed_tp_sl.unwrap_or_else(|| config.get_symbol_params(symbol));
            let trades = simulate_symbol(
                symbol,
                bars,
                &config.hft,
                &fees,
                tp_pct,
                sl_pct,
                config.defaults.max_order_amount,
                config.hft.evaluate_every_quotes,
                mode,
                asset_class,
            );

            summaries.push(summarize(symbol, &trades));
            all_trades.extend(trades);
        }
    }

    println!();
    println!(
        "{:<10} {:>7} {:>6} {:>6} {:>10} {:>12} {:>12}",
        "Symbol", "Trades", "Wins", "Loss", "WinRate%", "TotalPnL$", "AvgPnL$"
    );
    for s in &summaries {
        println!(
            "{:<10} {:>7} {:>6} {:>6} {:>10.1} {:>12.2} {:>12.4}",
            s.symbol, s.trades, s.wins, s.losses, s.win_rate_pct, s.total_pnl, s.avg_pnl
        );
    }

    let total_trades = all_trades.len();
    let total_pnl: f64 = all_trades.iter().map(|t| t.pnl).sum();
    let total_wins = all_trades.iter().filter(|t| t.pnl > 0.0).count();
    let overall_win_rate = if total_trades > 0 {
        total_wins as f64 / total_trades as f64 * 100.0
    } else {
        0.0
    };
    let dd = max_drawdown(&all_trades);

    println!();
    println!("===== Portfolio Summary =====");
    println!("Total trades:     {}", total_trades);
    println!("Overall win rate: {:.1}%", overall_win_rate);
    println!("Total PnL:        ${:.2}", total_pnl);
    println!(
        "Avg PnL/trade:    ${:.4}",
        if total_trades > 0 {
            total_pnl / total_trades as f64
        } else {
            0.0
        }
    );
    println!("Max drawdown:     ${:.2}", dd);

    // Persist full results for later inspection.
    std::fs::create_dir_all("data").ok();
    let report = serde_json::json!({
        "mode": mode.label(),
        "asset_class": asset_class.label(),
        "lookback_days": lookback_days,
        "start": start_str,
        "end": end_str,
        "config_snapshot": {
            "min_edge_bps": config.hft.min_edge_bps,
            "min_net_edge_bps": config.hft.min_net_edge_bps,
            "max_spread_bps": config.hft.max_spread_bps,
            "round_trip_fee_bps": fees.round_trip_fee_bps,
        },
        "summaries": summaries.iter().map(|s| serde_json::json!({
            "symbol": s.symbol,
            "trades": s.trades,
            "wins": s.wins,
            "losses": s.losses,
            "win_rate_pct": s.win_rate_pct,
            "total_pnl": s.total_pnl,
            "avg_pnl": s.avg_pnl,
        })).collect::<Vec<_>>(),
        "portfolio": {
            "total_trades": total_trades,
            "overall_win_rate_pct": overall_win_rate,
            "total_pnl": total_pnl,
            "max_drawdown": dd,
        },
        "trades": all_trades,
    });
    let out_path = format!(
        "data/backtest_results_{}_{}_{}d_offset{}.json",
        asset_class.label(),
        mode.label().replace('-', "_"),
        lookback_days,
        offset_days
    );
    let out_path = out_path.as_str();
    std::fs::write(out_path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("Full results written to {}", out_path);

    Ok(())
}
