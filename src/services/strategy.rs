use crate::agents::{director::DirectorAgent, quant::QuantAgent, Agent};
use crate::bus::EventBus;
use crate::config::{AppConfig, FeeConfig, HftConfig};
use crate::data::store::{MarketStore, Quote};
use crate::events::{AnalysisSignal, Event, MarketEvent};
use crate::llm::LLMQueue;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};

/// Outcome of evaluating a single HFT quote tick against the entry criteria.
#[derive(Debug, Clone, PartialEq)]
pub enum HftSignal {
    /// Not enough mid-price history yet to compute momentum.
    InsufficientHistory,
    /// Spread too wide to trade.
    SpreadTooWide { spread_bps: f64 },
    /// Raw momentum below the coarse pre-filter.
    EdgeTooLow { edge_bps: f64 },
    /// Passed the raw momentum filter but isn't profitable net of spread + fees.
    NetEdgeTooLow {
        edge_bps: f64,
        spread_bps: f64,
        net_edge_bps: f64,
    },
    /// Passed the edge gates, but recent trade activity doesn't confirm the move - likely
    /// quote noise rather than a real move with conviction behind it.
    VolumeTooLow { volume_ratio: f64 },
    /// All gates passed - buy.
    Buy {
        mid: f64,
        spread_bps: f64,
        edge_bps: f64,
        net_edge_bps: f64,
        take_profit: f64,
        stop_loss: f64,
    },
}

/// Ratio of the most recent trade size to the average of the ones before it, or `None`
/// when there isn't enough trade history yet to compute a baseline. `>1` means the latest
/// trade was larger than recent average activity (confirms conviction); `<1` means it was
/// quieter than usual (more likely to be noise).
fn volume_ratio(recent_volumes: &VecDeque<f64>) -> Option<f64> {
    if recent_volumes.len() < 3 {
        return None;
    }
    let current = *recent_volumes.back()?;
    let baseline_count = recent_volumes.len() - 1;
    let baseline_avg: f64 =
        recent_volumes.iter().take(baseline_count).sum::<f64>() / baseline_count as f64;
    if baseline_avg <= 0.0 {
        return None;
    }
    Some(current / baseline_avg)
}

/// Pure entry-decision logic for the HFT strategy: given a quote, recent mid-price history
/// (most recent last, already including the current tick and capped the same way
/// `evaluate_hft` caps it), and recent trade-size history, decides whether to buy. No I/O,
/// no async, no shared state - this is the single source of truth for "would the HFT
/// strategy buy here," called by both the live strategy loop and the backtester so the two
/// can never drift apart.
///
/// Callers are responsible for the invalid-quote check (`bid <= 0.0 || ask <= 0.0 || ask <
/// bid`) and for pushing the current mid/trade-size into their respective histories before
/// calling this - those are bookkeeping concerns, not entry-decision math.
pub fn evaluate_hft_signal(
    bid: f64,
    ask: f64,
    mids: &VecDeque<f64>,
    recent_volumes: &VecDeque<f64>,
    hft: &HftConfig,
    fees: &FeeConfig,
) -> HftSignal {
    let mid = (bid + ask) / 2.0;
    let spread_bps = ((ask - bid) / mid) * 10_000.0;
    if spread_bps > hft.max_spread_bps {
        return HftSignal::SpreadTooWide { spread_bps };
    }

    // Simple momentum edge: compare current mid to mid N steps back.
    let lookback = hft.momentum_lookback.min(mids.len().saturating_sub(1));
    if lookback == 0 {
        return HftSignal::InsufficientHistory;
    }
    let past = mids.get(mids.len() - 1 - lookback).copied().unwrap_or(mid);
    let edge_bps = ((mid - past) / past) * 10_000.0;

    if edge_bps < hft.min_edge_bps {
        return HftSignal::EdgeTooLow { edge_bps };
    }

    // The real profitability gate: edge net of spread and round-trip trading costs.
    // A raw momentum reading above min_edge_bps means nothing if spread + fees exceed
    // it - the position would be underwater from the moment it opens.
    let net_edge_bps = edge_bps - spread_bps - fees.round_trip_fee_bps;
    if net_edge_bps < hft.min_net_edge_bps {
        return HftSignal::NetEdgeTooLow {
            edge_bps,
            spread_bps,
            net_edge_bps,
        };
    }

    // Volume confirmation: require recent trade activity to be at least in line with
    // its own recent average before trusting the momentum reading. Fails open (doesn't
    // block) when there isn't enough trade history yet, so sparse trade data on a
    // low-liquidity symbol can't silently wedge the strategy shut.
    if let Some(ratio) = volume_ratio(recent_volumes) {
        if ratio < hft.min_volume_ratio {
            return HftSignal::VolumeTooLow {
                volume_ratio: ratio,
            };
        }
    }

    let take_profit = mid * (1.0 + hft.take_profit_bps / 10_000.0);
    let stop_loss = mid * (1.0 - hft.stop_loss_bps / 10_000.0);

    HftSignal::Buy {
        mid,
        spread_bps,
        edge_bps,
        net_edge_bps,
        take_profit,
        stop_loss,
    }
}

/// Experimental mean-reversion variant of `evaluate_hft_signal`: instead of buying on
/// upward momentum (betting the move continues), buys on a short-term dip of at least
/// `hft.min_edge_bps` magnitude (betting price reverts back up). Not wired into the live
/// strategy engine - this exists for backtesting the reversion hypothesis against the same
/// historical data and cost model as the momentum strategy, using the same config knobs
/// (spread/fee/volume gates and TP/SL sizing are identical; only the entry direction and
/// what counts as "edge" differ).
pub fn evaluate_mean_reversion_signal(
    bid: f64,
    ask: f64,
    mids: &VecDeque<f64>,
    recent_volumes: &VecDeque<f64>,
    hft: &HftConfig,
    fees: &FeeConfig,
) -> HftSignal {
    let mid = (bid + ask) / 2.0;
    let spread_bps = ((ask - bid) / mid) * 10_000.0;
    if spread_bps > hft.max_spread_bps {
        return HftSignal::SpreadTooWide { spread_bps };
    }

    let lookback = hft.momentum_lookback.min(mids.len().saturating_sub(1));
    if lookback == 0 {
        return HftSignal::InsufficientHistory;
    }
    let past = mids.get(mids.len() - 1 - lookback).copied().unwrap_or(mid);
    let move_bps = ((mid - past) / past) * 10_000.0; // negative = a dip

    // Only buy on a dip of at least min_edge_bps magnitude - the mirror image of the
    // momentum strategy's "only buy on a rise of at least min_edge_bps."
    if move_bps > -hft.min_edge_bps {
        return HftSignal::EdgeTooLow { edge_bps: move_bps };
    }
    let edge_bps = -move_bps; // reframe the dip size as a positive "reversion edge"

    let net_edge_bps = edge_bps - spread_bps - fees.round_trip_fee_bps;
    if net_edge_bps < hft.min_net_edge_bps {
        return HftSignal::NetEdgeTooLow {
            edge_bps,
            spread_bps,
            net_edge_bps,
        };
    }

    if let Some(ratio) = volume_ratio(recent_volumes) {
        if ratio < hft.min_volume_ratio {
            return HftSignal::VolumeTooLow {
                volume_ratio: ratio,
            };
        }
    }

    let take_profit = mid * (1.0 + hft.take_profit_bps / 10_000.0);
    let stop_loss = mid * (1.0 - hft.stop_loss_bps / 10_000.0);

    HftSignal::Buy {
        mid,
        spread_bps,
        edge_bps,
        net_edge_bps,
        take_profit,
        stop_loss,
    }
}

/// Outcome of any indicator-based signal below - deliberately simpler than `HftSignal`
/// since these aren't all comparable along the same "edge_bps/spread_bps/net_edge_bps"
/// axes; each carries its own free-text reasoning for logging/backtest inspection instead.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalDecision {
    NoSignal,
    Buy {
        mid: f64,
        take_profit: f64,
        stop_loss: f64,
        reasoning: String,
    },
    /// Short entry: mirror of `Buy` with inverted risk - `take_profit` sits BELOW `mid`
    /// (price falling is the win condition for a short) and `stop_loss` sits ABOVE `mid`.
    /// Requires an account with shorting enabled (confirmed available on this account -
    /// see `docs/STRATEGIES.md`).
    Sell {
        mid: f64,
        take_profit: f64,
        stop_loss: f64,
        reasoning: String,
    },
}

/// Shared spread/liquidity gate used by every signal below: even a textbook-perfect
/// indicator reading is untradeable if the book is too wide. Returns the mid price when
/// the gate passes. (Unlike `evaluate_hft_signal`, these signals don't duplicate a
/// net-edge-vs-fees pre-check - the backtester already deducts real fees from every
/// trade's PnL regardless of which signal opened it, so cost realism is enforced once,
/// centrally, rather than re-implemented per signal.)
fn passes_spread_gate(bid: f64, ask: f64, max_spread_bps: f64) -> Option<f64> {
    if bid <= 0.0 || ask <= 0.0 || ask < bid {
        return None;
    }
    let mid = (bid + ask) / 2.0;
    let spread_bps = ((ask - bid) / mid) * 10_000.0;
    if spread_bps > max_spread_bps {
        return None;
    }
    Some(mid)
}

/// RSI mean-reversion: buy when RSI drops below `oversold`, betting on a bounce.
/// `closes` must be ordered oldest-to-newest and include the current price as the last
/// element.
pub fn evaluate_rsi_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    period: usize,
    oversold: f64,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();
    let Some(r) = crate::services::indicators::rsi(&closes_vec, period) else {
        return SignalDecision::NoSignal;
    };
    if r > oversold {
        return SignalDecision::NoSignal;
    }
    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!("rsi={:.1} < oversold={:.1}", r, oversold),
    }
}

/// Bollinger-band mean-reversion: buy when price is `entry_z` standard deviations (or
/// more) below its own `period`-length moving average - a volatility-normalized dip,
/// unlike the fixed-bps dip threshold `evaluate_mean_reversion_signal` uses.
pub fn evaluate_bollinger_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    period: usize,
    entry_z: f64,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();
    let (Some(mean), Some(sd)) = (
        crate::services::indicators::sma(&closes_vec, period),
        crate::services::indicators::stddev(&closes_vec, period),
    ) else {
        return SignalDecision::NoSignal;
    };
    if sd <= 0.0 {
        return SignalDecision::NoSignal;
    }
    let z = (mid - mean) / sd;
    if z > -entry_z {
        return SignalDecision::NoSignal;
    }
    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!("bollinger z={:.2} <= -{:.2}", z, entry_z),
    }
}

/// Moving-average crossover (trend-following): buy on a "golden cross," the tick where
/// the fast MA moves from at-or-below the slow MA to above it.
pub fn evaluate_ma_crossover_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    fast_period: usize,
    slow_period: usize,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();
    if closes_vec.len() < slow_period + 1 {
        return SignalDecision::NoSignal;
    }
    let prior = &closes_vec[..closes_vec.len() - 1];
    let (Some(fast_now), Some(slow_now), Some(fast_prev), Some(slow_prev)) = (
        crate::services::indicators::sma(&closes_vec, fast_period),
        crate::services::indicators::sma(&closes_vec, slow_period),
        crate::services::indicators::sma(prior, fast_period),
        crate::services::indicators::sma(prior, slow_period),
    ) else {
        return SignalDecision::NoSignal;
    };
    let crossed_up = fast_prev <= slow_prev && fast_now > slow_now;
    if !crossed_up {
        return SignalDecision::NoSignal;
    }
    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!(
            "golden cross: fast({})={:.8} > slow({})={:.8}",
            fast_period, fast_now, slow_period, slow_now
        ),
    }
}

/// Donchian channel breakout (trend/momentum): buy when price closes above the highest
/// close of the prior `period` ticks - a different shape of "momentum" than
/// `evaluate_hft_signal`'s point-to-point comparison (this requires a new extreme, not
/// just net positive drift over the window).
pub fn evaluate_donchian_breakout_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    period: usize,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();
    let Some(prior_high) = crate::services::indicators::prior_period_high(&closes_vec, period)
    else {
        return SignalDecision::NoSignal;
    };
    if mid <= prior_high {
        return SignalDecision::NoSignal;
    }
    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!("breakout above prior {}-period high {:.8}", period, prior_high),
    }
}

/// VWAP mean-reversion: buy when price is `entry_bps` or more below its own rolling
/// volume-weighted average price - a dip weighted toward where the real trading activity
/// happened, unlike `evaluate_mean_reversion_signal`'s plain point-to-point price dip.
pub fn evaluate_vwap_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    volumes: &VecDeque<f64>,
    period: usize,
    entry_bps: f64,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();
    let volumes_vec: Vec<f64> = volumes.iter().copied().collect();
    if closes_vec.len() != volumes_vec.len() {
        return SignalDecision::NoSignal;
    }
    let Some(vw) = crate::services::indicators::vwap(&closes_vec, &volumes_vec, period) else {
        return SignalDecision::NoSignal;
    };
    if vw <= 0.0 {
        return SignalDecision::NoSignal;
    }
    let deviation_bps = (mid - vw) / vw * 10_000.0;
    if deviation_bps > -entry_bps {
        return SignalDecision::NoSignal;
    }
    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!("vwap={:.8}, deviation={:.1}bps", vw, deviation_bps),
    }
}

/// Momentum, but only when a longer-horizon trend filter also agrees: price must be above
/// its own `trend_period`-length moving average, not just showing short-term positive
/// momentum. Standard "trade with the trend" filter (analogous to a 50-period MA filter in
/// classical TA) - tests whether momentum entries perform better when an established
/// uptrend backs them up, versus taking every momentum blip regardless of broader context.
pub fn evaluate_trend_filtered_momentum_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    momentum_lookback: usize,
    min_edge_bps: f64,
    trend_period: usize,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();

    let lookback = momentum_lookback.min(closes_vec.len().saturating_sub(1));
    if lookback == 0 {
        return SignalDecision::NoSignal;
    }
    let past = closes_vec[closes_vec.len() - 1 - lookback];
    if past <= 0.0 {
        return SignalDecision::NoSignal;
    }
    let edge_bps = (mid - past) / past * 10_000.0;
    if edge_bps < min_edge_bps {
        return SignalDecision::NoSignal;
    }

    let Some(trend_sma) = crate::services::indicators::sma(&closes_vec, trend_period) else {
        return SignalDecision::NoSignal;
    };
    if mid <= trend_sma {
        return SignalDecision::NoSignal; // momentum present, but against the broader trend
    }

    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!(
            "edge={:.1}bps, price {:.8} > trend_sma({}) {:.8}",
            edge_bps, mid, trend_period, trend_sma
        ),
    }
}

/// Standalone short strategy (breakdown momentum): the mirror image of
/// `evaluate_trend_filtered_momentum_signal` for the downside - sells short when price has
/// dropped at least `min_edge_bps` over `momentum_lookback` bars AND sits below its own
/// `trend_period`-length moving average (confirms a genuine downtrend, not just a dip inside
/// an uptrend). Every prior signal in this file only ever goes long; this is the first that
/// trades the short side as its own strategy, not just as a pairs-trading hedge leg.
pub fn evaluate_short_momentum_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    momentum_lookback: usize,
    min_edge_bps: f64,
    trend_period: usize,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();

    let lookback = momentum_lookback.min(closes_vec.len().saturating_sub(1));
    if lookback == 0 {
        return SignalDecision::NoSignal;
    }
    let past = closes_vec[closes_vec.len() - 1 - lookback];
    if past <= 0.0 {
        return SignalDecision::NoSignal;
    }
    let edge_bps = (past - mid) / past * 10_000.0; // positive = downward move
    if edge_bps < min_edge_bps {
        return SignalDecision::NoSignal;
    }

    let Some(trend_sma) = crate::services::indicators::sma(&closes_vec, trend_period) else {
        return SignalDecision::NoSignal;
    };
    if mid >= trend_sma {
        return SignalDecision::NoSignal; // breakdown present, but not below the broader trend
    }

    SignalDecision::Sell {
        mid,
        take_profit: mid * (1.0 - tp_pct / 100.0),
        stop_loss: mid * (1.0 + sl_pct / 100.0),
        reasoning: format!(
            "breakdown edge={:.1}bps, price {:.8} < trend_sma({}) {:.8}",
            edge_bps, mid, trend_period, trend_sma
        ),
    }
}

/// Volatility squeeze breakout: buy when Bollinger bandwidth has dropped below
/// `squeeze_threshold` (an unusually tight range - TA folklore treats this as energy
/// building up for a sharp move) at some point within the last `lookback_period` bars, AND
/// price has just broken above its own upper Bollinger band. A different shape of signal
/// than `evaluate_bollinger_signal` (which buys ON a dip below the LOWER band, betting on
/// reversion); this instead buys an upward breakout that follows a period of contraction,
/// betting on continuation.
pub fn evaluate_vol_squeeze_signal(
    bid: f64,
    ask: f64,
    closes: &VecDeque<f64>,
    period: usize,
    num_sd: f64,
    squeeze_threshold: f64,
    lookback_period: usize,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let closes_vec: Vec<f64> = closes.iter().copied().collect();
    if closes_vec.len() < period + lookback_period {
        return SignalDecision::NoSignal;
    }

    let mut had_squeeze = false;
    for i in 0..lookback_period {
        let end = closes_vec.len() - i;
        if end < period {
            break;
        }
        let window = &closes_vec[..end];
        if let Some(bw) =
            crate::services::indicators::bollinger_bandwidth(window, period, num_sd)
        {
            if bw < squeeze_threshold {
                had_squeeze = true;
                break;
            }
        }
    }
    if !had_squeeze {
        return SignalDecision::NoSignal;
    }

    let (Some(mean), Some(sd)) = (
        crate::services::indicators::sma(&closes_vec, period),
        crate::services::indicators::stddev(&closes_vec, period),
    ) else {
        return SignalDecision::NoSignal;
    };
    let upper_band = mean + num_sd * sd;
    if mid <= upper_band {
        return SignalDecision::NoSignal;
    }

    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!(
            "vol squeeze breakout: upper_band={:.8}, mid={:.8}",
            upper_band, mid
        ),
    }
}

/// Opening-range breakout: buy when price breaks above the high of the first `range_bars`
/// bars of the current trading session. `opening_range_high` is computed by the caller
/// (`None` before the current day's opening range has been fully established yet) since
/// tracking "the first N bars of today" needs day-boundary bookkeeping this file's other
/// pure, state-free signals don't carry - keeping that bookkeeping in the caller (the
/// backtester's per-symbol state) keeps this function itself pure and testable in isolation.
pub fn evaluate_orb_signal(
    bid: f64,
    ask: f64,
    opening_range_high: Option<f64>,
    max_spread_bps: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> SignalDecision {
    let Some(mid) = passes_spread_gate(bid, ask, max_spread_bps) else {
        return SignalDecision::NoSignal;
    };
    let Some(or_high) = opening_range_high else {
        return SignalDecision::NoSignal;
    };
    if mid <= or_high {
        return SignalDecision::NoSignal;
    }
    SignalDecision::Buy {
        mid,
        take_profit: mid * (1.0 + tp_pct / 100.0),
        stop_loss: mid * (1.0 - sl_pct / 100.0),
        reasoning: format!("ORB: broke above opening range high {:.8}", or_high),
    }
}

#[derive(Clone)]
struct SymbolCooldown {
    quotes_remaining: usize,
}

#[derive(Clone, Default)]
struct HftSymbolState {
    quotes_since_eval: usize,
    last_mid: Option<f64>,
    mids: VecDeque<f64>,
    /// Rolling window of recent trade sizes, used to confirm momentum with real activity
    /// (see `evaluate_hft_signal`'s volume gate) rather than trading on quote noise alone.
    recent_trade_sizes: VecDeque<f64>,
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
    pub fn new(
        event_bus: EventBus,
        market_store: MarketStore,
        llm: LLMQueue,
        config: AppConfig,
    ) -> Self {
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
            info!(
                "🧠 Strategy Engine Started (mode: {})",
                config_clone.strategy_mode
            );
            loop {
                let event = match rx.recv().await {
                    Ok(event) => event,
                    Err(RecvError::Lagged(n)) => {
                        warn!("[STRATEGY] Event bus lagged, dropped {} events", n);
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        info!("[STRATEGY] Event bus closed, shutting down");
                        break;
                    }
                };
                if let Event::Market(market_event) = event {
                    let (symbol, bid, ask, trade_size) = match &market_event {
                        MarketEvent::Quote {
                            symbol, bid, ask, ..
                        } => (symbol.clone(), *bid, *ask, None),
                        MarketEvent::Trade {
                            symbol,
                            price,
                            size,
                            ..
                        } => (symbol.clone(), *price, *price, Some(*size)),
                    };

                    let mode = config_clone.strategy_mode.to_lowercase();

                    if mode == "hft" {
                        let bus = bus_clone.clone();
                        let tracker = hft_state.clone();
                        let config = config_clone.clone();
                        tokio::spawn(async move {
                            Self::evaluate_hft(symbol, bid, ask, trade_size, bus, tracker, config)
                                .await;
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
                            Self::evaluate_hybrid(
                                symbol,
                                bid,
                                ask,
                                trade_size,
                                bus,
                                store,
                                llm,
                                hft_tracker,
                                gate,
                                config,
                            )
                            .await;
                        });
                        continue;
                    }

                    // Default: LLM pipeline ("llm" or anything else)

                    // Check cooldown status
                    if let Some(mut cooldown) = cooldowns.get_mut(&symbol) {
                        if cooldown.quotes_remaining > 0 {
                            cooldown.quotes_remaining -= 1;
                            if cooldown.quotes_remaining == 0 {
                                info!(
                                    "⏰ [COOLDOWN] {} cooldown expired. Ready for analysis.",
                                    symbol
                                );
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
                        Self::analyze_symbol_llm(
                            symbol_clone,
                            store,
                            llm,
                            bus,
                            cooldowns_clone,
                            config,
                        )
                        .await;
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
                .filter_map(|n| {
                    n.get("headline")
                        .and_then(|h| h.as_str())
                        .map(|s| s.to_string())
                })
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

        info!(
            "🟢 [STRATEGY] Opportunity found for {}! Running Quant...",
            symbol
        );

        // 2. Quant
        let quant = QuantAgent;
        let quant_input = format!(
            "Thesis: {}\n\nMarket Data:\n{}",
            director_response, combined_data
        );

        let quant_response = match quant.run_high_priority(&quant_input, &llm).await {
            Ok(res) => res,
            Err(e) => {
                error!("❌ Quant Failed for {}: {}", symbol, e);
                return;
            }
        };

        info!(
            "📈 [STRATEGY] Quant Analysis for {}: {}",
            symbol, quant_response
        );

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
        trade_size: Option<f64>,
        bus: EventBus,
        state: Arc<DashMap<String, HftSymbolState>>,
        config: AppConfig,
    ) {
        if bid <= 0.0 || ask <= 0.0 || ask < bid {
            if config.chatter_level.to_lowercase() == "verbose" {
                warn!(
                    "[HFT] Skip {}: invalid quote bid={} ask={}",
                    symbol, bid, ask
                );
            }
            return;
        }

        let mid = (bid + ask) / 2.0;
        let spread_bps = ((ask - bid) / mid) * 10_000.0;
        if spread_bps > config.hft.max_spread_bps {
            if config.chatter_level.to_lowercase() == "verbose" {
                info!(
                    "[HFT] Skip {}: spread_bps={:.2} > max_spread_bps={:.2} (bid={:.8} ask={:.8})",
                    symbol, spread_bps, config.hft.max_spread_bps, bid, ask
                );
            }
            return;
        }

        let mut entry = state
            .entry(symbol.clone())
            .or_insert_with(|| HftSymbolState {
                quotes_since_eval: 0,
                last_mid: None,
                mids: VecDeque::with_capacity(64),
                recent_trade_sizes: VecDeque::with_capacity(32),
            });

        entry.quotes_since_eval += 1;
        entry.mids.push_back(mid);
        while entry.mids.len() > 30 {
            entry.mids.pop_front();
        }
        if let Some(size) = trade_size {
            entry.recent_trade_sizes.push_back(size);
            while entry.recent_trade_sizes.len() > 30 {
                entry.recent_trade_sizes.pop_front();
            }
        }

        if entry.quotes_since_eval < config.hft.evaluate_every_quotes {
            if config.chatter_level.to_lowercase() == "verbose" {
                info!(
                    "[HFT] Debounce {}: {}/{} quotes collected (mid={:.8})",
                    symbol, entry.quotes_since_eval, config.hft.evaluate_every_quotes, mid
                );
            }
            entry.last_mid = Some(mid);
            return;
        }
        entry.quotes_since_eval = 0;
        entry.last_mid = Some(mid);

        let decision = evaluate_hft_signal(
            bid,
            ask,
            &entry.mids,
            &entry.recent_trade_sizes,
            &config.hft,
            &config.fees,
        );
        // drop(entry); // DashMap RefMut is dropped here

        let verbose = config.chatter_level.to_lowercase() == "verbose";
        let (edge_bps, net_edge_bps, take_profit, stop_loss) = match decision {
            HftSignal::InsufficientHistory => {
                if verbose {
                    info!("[HFT] Skip {}: insufficient history for lookback", symbol);
                }
                return;
            }
            HftSignal::SpreadTooWide { spread_bps } => {
                if verbose {
                    info!(
                        "[HFT] Skip {}: spread_bps={:.2} > max_spread_bps={:.2} (bid={:.8} ask={:.8})",
                        symbol, spread_bps, config.hft.max_spread_bps, bid, ask
                    );
                }
                return;
            }
            HftSignal::EdgeTooLow { edge_bps } => {
                if verbose {
                    info!(
                        "[HFT] Skip {}: edge_bps={:.2} < min_edge_bps={:.2} (mid={:.8})",
                        symbol, edge_bps, config.hft.min_edge_bps, mid
                    );
                }
                return;
            }
            HftSignal::NetEdgeTooLow {
                edge_bps,
                spread_bps,
                net_edge_bps,
            } => {
                if verbose {
                    info!(
                        "[HFT] Skip {}: net_edge_bps={:.2} (edge={:.2} - spread={:.2} - fees={:.2}) < min_net_edge_bps={:.2}",
                        symbol, net_edge_bps, edge_bps, spread_bps, config.fees.round_trip_fee_bps, config.hft.min_net_edge_bps
                    );
                }
                return;
            }
            HftSignal::VolumeTooLow { volume_ratio } => {
                if verbose {
                    info!(
                        "[HFT] Skip {}: volume_ratio={:.2} < min_volume_ratio={:.2} (edge looked good but activity doesn't confirm it)",
                        symbol, volume_ratio, config.hft.min_volume_ratio
                    );
                }
                return;
            }
            HftSignal::Buy {
                spread_bps,
                edge_bps,
                net_edge_bps,
                take_profit,
                stop_loss,
                ..
            } => {
                // This is the key "when HFT will buy" log.
                // - In normal: only log on entry.
                // - In verbose: include more details.
                if config.chatter_level.to_lowercase() != "low" {
                    info!("[HFT] BUY trigger {}: edge_bps={:.2} >= min_edge_bps={:.2}, net_edge_bps={:.2} >= min_net_edge_bps={:.2}, spread_bps={:.2} <= max_spread_bps={:.2} | entry(mid)={:.8} tp={:.8} sl={:.8}",
                          symbol, edge_bps, config.hft.min_edge_bps, net_edge_bps, config.hft.min_net_edge_bps, spread_bps, config.hft.max_spread_bps, mid, take_profit, stop_loss);
                }
                (edge_bps, net_edge_bps, take_profit, stop_loss)
            }
        };

        let thesis = format!(
            "HFT momentum: edge_bps={:.2}, net_edge_bps={:.2}, mid={:.8}",
            edge_bps, net_edge_bps, mid
        );

        let signal = AnalysisSignal {
            symbol,
            signal: "buy".to_string(),
            confidence: 1.0,
            thesis: thesis.clone(),
            market_context: format!("tp={:.8}, sl={:.8}", take_profit, stop_loss),
        };

        bus.publish(Event::Signal(signal)).ok();
    }

    async fn evaluate_hybrid(
        symbol: String,
        bid: f64,
        ask: f64,
        trade_size: Option<f64>,
        bus: EventBus,
        store: MarketStore,
        llm: LLMQueue,
        hft_state: Arc<DashMap<String, HftSymbolState>>,
        gate: Arc<DashMap<String, HybridGateState>>,
        config: AppConfig,
    ) {
        if bid <= 0.0 || ask <= 0.0 || ask < bid {
            if config.chatter_level.to_lowercase() == "verbose" {
                warn!(
                    "[HYBRID] Skip {}: invalid quote bid={} ask={}",
                    symbol, bid, ask
                );
            }
            return;
        }

        // Gate bookkeeping (quote based)
        let mut should_refresh = false;
        let mut currently_allowed;

        {
            let mut entry = gate
                .entry(symbol.clone())
                .or_insert_with(|| HybridGateState {
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
                info!(
                    "[HYBRID] Gate closed for {} (cooldown_remaining={}, quotes_until_refresh={})",
                    symbol, entry.cooldown_quotes_remaining, entry.quotes_until_refresh
                );
            }
        }

        if should_refresh {
            let history = store.get_quote_history(&symbol);
            if history.len() >= config.warmup_count {
                if config.chatter_level.to_lowercase() != "low" {
                    info!(
                        "[HYBRID] Refreshing LLM gate for {} (history_len={})",
                        symbol,
                        history.len()
                    );
                }

                let combined_data = Self::format_quote_history_table(&history);
                let director = DirectorAgent;
                let director_input =
                    format!("Symbol: {}, Market Context: {}", symbol, combined_data);

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
                            entry.cooldown_quotes_remaining =
                                config.hybrid.no_trade_cooldown_quotes;
                            warn!(
                                "[HYBRID] Gate CLOSED for {} by director. Cooldown {} quotes.",
                                symbol, config.hybrid.no_trade_cooldown_quotes
                            );
                            if config.chatter_level.to_lowercase() == "verbose" {
                                warn!(
                                    "[HYBRID] Director response (no_trade) for {}: {}",
                                    symbol, resp
                                );
                            }
                        } else {
                            if config.chatter_level.to_lowercase() != "low" {
                                info!("[HYBRID] Gate OPEN for {} by director.", symbol);
                            }
                            if config.chatter_level.to_lowercase() == "verbose" {
                                info!(
                                    "[HYBRID] Director response (allowed) for {}: {}",
                                    symbol, resp
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            "[HYBRID] Director gate failed for {}: {} (keeping previous gate)",
                            symbol, e
                        );
                    }
                }
            } else if config.chatter_level.to_lowercase() == "verbose" {
                info!(
                    "[HYBRID] Skip gate refresh for {}: warmup not met (history_len={}, warmup={})",
                    symbol,
                    history.len(),
                    config.warmup_count
                );
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

        Self::evaluate_hft(symbol, bid, ask, trade_size, bus, hft_state, config).await;
    }

    fn format_quote_history_table(history: &[Quote]) -> String {
        let mut table = String::from(
            "Recent Quote History (Last 50 Quotes):\nTime | Bid | BidSz | Ask | AskSz\n",
        );
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
