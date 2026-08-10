//! Finds cointegrated (mean-reverting) pairs among Alpaca's tradable crypto/USD symbols,
//! and backtests a long-only relative-value strategy on the best ones found.
//!
//! Alpaca crypto is spot-only - there's no shorting - so this is NOT a market-neutral
//! pairs trade in the classical sense (long one leg, short the other). It's a long-only
//! approximation: when the spread between two cointegrated symbols stretches unusually far
//! (symbol A cheap relative to symbol B, given their historical relationship), buy A only,
//! betting the relationship reverts. This keeps directional market exposure that a true
//! pairs trade wouldn't have - a real limitation worth remembering when reading results.
//!
//! Cointegration is tested via OLS hedge ratio + empirical mean-reversion half-life
//! (`rust_autohedge::services::indicators::mean_reversion_half_life`), not a formal
//! Engle-Granger/ADF test with critical-value tables - see that function's doc comment for
//! why. Treat "cointegrated" here as "empirically mean-reverting," a related but looser
//! claim.
//!
//! Usage: `cargo run --bin pairs --release -- [--days N] [--top N] [--asset-class crypto|stocks]`

use chrono::{Duration as ChronoDuration, Utc};
use rust_autohedge::backtest_support::{fetch_all_bars, liquid_stock_universe, AssetClass};
use rust_autohedge::config::{AppConfig, FeeConfig};
use rust_autohedge::data::alpaca::{AlpacaClient, CryptoBar};
use rust_autohedge::services::indicators::{mean_reversion_half_life, ols_fit, ols_residuals, sma, stddev};
use serde::Serialize;
use std::collections::HashMap;

const DEFAULT_LOOKBACK_DAYS: i64 = 60;
const TIMEFRAME: &str = "1Min";
// Reject pairs whose spread reverts implausibly fast (numerical noise, not a real tradable
// relationship) or so slowly a position would need to be held for days-to-weeks.
const MIN_HALF_LIFE_BARS: f64 = 5.0;
const MAX_HALF_LIFE_BARS: f64 = 2000.0;
const ZSCORE_WINDOW: usize = 60; // bars used to compute the spread's rolling mean/stddev
const DEFAULT_ENTRY_Z: f64 = 2.0;
const DEFAULT_EXIT_Z: f64 = 0.5;
const DEFAULT_STOP_Z: f64 = 4.5;

fn days_from_args() -> i64 {
    std::env::args()
        .skip_while(|a| a != "--days")
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_LOOKBACK_DAYS)
}

fn offset_days_from_args() -> i64 {
    std::env::args()
        .skip_while(|a| a != "--offset-days")
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

fn top_n_from_args() -> usize {
    std::env::args()
        .skip_while(|a| a != "--top")
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(15)
}

fn asset_class_from_args() -> AssetClass {
    std::env::args()
        .skip_while(|a| a != "--asset-class")
        .nth(1)
        .map(|s| AssetClass::from_arg(&s))
        .unwrap_or(AssetClass::Crypto)
}

fn f64_flag(flag: &str, default: f64) -> f64 {
    std::env::args()
        .skip_while(|a| a != flag)
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(default)
}

/// Aligns two symbols' bars by timestamp (only keeping timestamps present in both), then
/// returns their close-price series in matching order. Crypto trades continuously so gaps
/// should be rare, but this guards against any that exist rather than assuming index `i`
/// in one series lines up with index `i` in the other.
fn align_by_timestamp(a: &[CryptoBar], b: &[CryptoBar]) -> (Vec<f64>, Vec<f64>, Vec<String>) {
    let b_by_ts: HashMap<&str, f64> = b.iter().map(|bar| (bar.timestamp.as_str(), bar.close)).collect();
    let mut xa = Vec::new();
    let mut xb = Vec::new();
    let mut ts = Vec::new();
    for bar in a {
        if let Some(&close_b) = b_by_ts.get(bar.timestamp.as_str()) {
            xa.push(bar.close);
            xb.push(close_b);
            ts.push(bar.timestamp.clone());
        }
    }
    (xa, xb, ts)
}

struct PairCandidate {
    symbol_a: String,
    symbol_b: String,
    alpha: f64,
    beta: f64,
    half_life_bars: f64,
    residual_stddev: f64,
    aligned_len: usize,
}

fn find_cointegrated_pairs(bars_by_symbol: &[(String, Vec<CryptoBar>)]) -> Vec<PairCandidate> {
    let mut candidates = Vec::new();
    for i in 0..bars_by_symbol.len() {
        for j in 0..bars_by_symbol.len() {
            if i == j {
                continue;
            }
            let (symbol_a, bars_a) = &bars_by_symbol[i];
            let (symbol_b, bars_b) = &bars_by_symbol[j];
            let (xa, xb, _ts) = align_by_timestamp(bars_a, bars_b);
            if xa.len() < 200 {
                continue; // not enough overlapping history to trust a fit
            }
            // Fit on log prices, not raw prices: crypto pairs span many orders of
            // magnitude (PEPE ~$0.00001 vs BTC ~$100k+), and a raw-price OLS fit across
            // that range produces numerically degenerate coefficients (observed: beta in
            // the billions) that pass every check but mean nothing economically. Log
            // prices put every symbol on a comparable scale and make the spread a
            // relative/percentage mispricing, which is also the economically right thing
            // to trade.
            let log_xa: Vec<f64> = xa.iter().map(|v| v.ln()).collect();
            let log_xb: Vec<f64> = xb.iter().map(|v| v.ln()).collect();
            let Some((alpha, beta)) = ols_fit(&log_xb, &log_xa) else {
                continue; // fits ln(A) = alpha + beta*ln(B)
            };
            let residuals = ols_residuals(&log_xb, &log_xa, alpha, beta);
            let Some(half_life) = mean_reversion_half_life(&residuals) else {
                continue;
            };
            if !(MIN_HALF_LIFE_BARS..=MAX_HALF_LIFE_BARS).contains(&half_life) {
                continue;
            }
            let Some(resid_sd) = stddev(&residuals, residuals.len()) else {
                continue;
            };
            candidates.push(PairCandidate {
                symbol_a: symbol_a.clone(),
                symbol_b: symbol_b.clone(),
                alpha,
                beta,
                half_life_bars: half_life,
                residual_stddev: resid_sd,
                aligned_len: xa.len(),
            });
        }
    }
    candidates
}

const MAX_HOLD_BARS: usize = 2000; // safety exit so a trade can't hang open indefinitely

#[derive(Debug, Clone, Serialize)]
struct PairTrade {
    pair: String,
    entry_time: String,
    exit_time: String,
    direction: &'static str, // "long_a_short_b" or "short_a_long_b"
    long_symbol: String,
    long_entry: f64,
    long_exit: f64,
    long_qty: f64,
    short_symbol: String,
    short_entry: f64,
    short_exit: f64,
    short_qty: f64,
    exit_reason: &'static str,
    pnl: f64,
}

struct OpenPairPosition {
    direction: &'static str,
    long_symbol: String,
    long_entry: f64,
    long_qty: f64,
    short_symbol: String,
    short_entry: f64,
    short_qty: f64,
    entry_time: String,
    entry_index: usize,
}

/// True market-neutral pairs trade: when the spread between A and B stretches
/// `ENTRY_Z` rolling-stddev from its mean, go long the cheap leg and short the expensive
/// leg in equal dollar amounts, betting the *relationship* reverts - not either leg's
/// absolute price. Unlike the old long-only approximation, this trades BOTH directions
/// (A cheap vs B, or A expensive vs B) and its return doesn't depend on which way the
/// broader market moves, since a market-wide move should lift or drop both legs together
/// and roughly cancel in the long+short PnL.
///
/// Exit is spread-based (z-score reverting toward 0, or diverging further as a stop),
/// not a fixed price target on either leg - a fixed-price TP/SL doesn't make sense once
/// there are two legs whose prices can move independently.
fn simulate_market_neutral_pair(
    symbol_a: &str,
    symbol_b: &str,
    bars_a: &[CryptoBar],
    bars_b: &[CryptoBar],
    alpha: f64,
    beta: f64,
    fees: &FeeConfig,
    leg_notional: f64,
    asset_class: AssetClass,
    entry_z: f64,
    exit_z: f64,
    stop_z: f64,
) -> Vec<PairTrade> {
    let spread_a = asset_class.assumed_spread_bps(symbol_a);
    let spread_b = asset_class.assumed_spread_bps(symbol_b);
    let (closes_a, closes_b, timestamps) = align_by_timestamp(bars_a, bars_b);

    let mut residuals = Vec::with_capacity(closes_a.len());
    for i in 0..closes_a.len() {
        residuals.push(closes_a[i].ln() - (alpha + beta * closes_b[i].ln()));
    }

    let mut trades = Vec::new();
    let mut open: Option<OpenPairPosition> = None;

    for i in 0..residuals.len() {
        if i < ZSCORE_WINDOW {
            continue;
        }
        let window = &residuals[i - ZSCORE_WINDOW..=i];
        let Some(mean) = sma(window, ZSCORE_WINDOW) else {
            continue;
        };
        let Some(sd) = stddev(window, ZSCORE_WINDOW) else {
            continue;
        };
        if sd <= 0.0 {
            continue;
        }
        let z = (residuals[i] - mean) / sd;

        if let Some(pos) = &open {
            let held = i - pos.entry_index;
            let reverted = match pos.direction {
                "long_a_short_b" => z >= -exit_z,
                _ => z <= exit_z,
            };
            let diverged = match pos.direction {
                "long_a_short_b" => z <= -stop_z,
                _ => z >= stop_z,
            };
            let exit_reason = if reverted {
                Some("reverted")
            } else if diverged {
                Some("diverged_stop")
            } else if held >= MAX_HOLD_BARS {
                Some("max_hold_timeout")
            } else {
                None
            };

            if let Some(reason) = exit_reason {
                // Which raw close/spread belongs to the long vs short leg depends on
                // direction - "short_a_long_b" means B is the long leg, A is the short
                // leg, the opposite of "long_a_short_b". Using closes_a/spread_a
                // unconditionally for "long" here (as an earlier version of this function
                // did) silently priced the wrong symbol's exit whenever direction was
                // short_a_long_b, producing wildly wrong PnL.
                let (long_close, long_spread, short_close, short_spread) =
                    if pos.direction == "long_a_short_b" {
                        (closes_a[i], spread_a, closes_b[i], spread_b)
                    } else {
                        (closes_b[i], spread_b, closes_a[i], spread_a)
                    };
                let long_exit = long_close * (1.0 - long_spread / 20_000.0); // sell: cross toward bid
                let short_exit = short_close * (1.0 + short_spread / 20_000.0); // cover: cross toward ask
                let long_fee = pos.long_entry * pos.long_qty * (fees.round_trip_fee_bps / 10_000.0);
                let short_fee = pos.short_entry * pos.short_qty * (fees.round_trip_fee_bps / 10_000.0);
                let long_pnl = (long_exit - pos.long_entry) * pos.long_qty - long_fee;
                let short_pnl = (pos.short_entry - short_exit) * pos.short_qty - short_fee;
                trades.push(PairTrade {
                    pair: format!("{}/vs/{}", symbol_a, symbol_b),
                    entry_time: pos.entry_time.clone(),
                    exit_time: timestamps[i].clone(),
                    direction: pos.direction,
                    long_symbol: pos.long_symbol.clone(),
                    long_entry: pos.long_entry,
                    long_exit,
                    long_qty: pos.long_qty,
                    short_symbol: pos.short_symbol.clone(),
                    short_entry: pos.short_entry,
                    short_exit,
                    short_qty: pos.short_qty,
                    exit_reason: reason,
                    pnl: long_pnl + short_pnl,
                });
                open = None;
            }
        }

        if open.is_some() {
            continue;
        }

        let direction = if z <= -entry_z {
            "long_a_short_b" // A cheap relative to B
        } else if z >= entry_z {
            "short_a_long_b" // A expensive relative to B
        } else {
            continue;
        };

        let (long_symbol, long_price_raw, long_spread, short_symbol, short_price_raw, short_spread) =
            if direction == "long_a_short_b" {
                (symbol_a, closes_a[i], spread_a, symbol_b, closes_b[i], spread_b)
            } else {
                (symbol_b, closes_b[i], spread_b, symbol_a, closes_a[i], spread_a)
            };

        let long_entry = long_price_raw * (1.0 + long_spread / 20_000.0);
        let short_entry = short_price_raw * (1.0 - short_spread / 20_000.0);
        open = Some(OpenPairPosition {
            direction,
            long_symbol: long_symbol.to_string(),
            long_entry,
            long_qty: leg_notional / long_entry,
            short_symbol: short_symbol.to_string(),
            short_entry,
            short_qty: leg_notional / short_entry,
            entry_time: timestamps[i].clone(),
            entry_index: i,
        });
    }

    trades
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let lookback_days = days_from_args();
    let top_n = top_n_from_args();
    let asset_class = asset_class_from_args();
    let entry_z = f64_flag("--entry-z", DEFAULT_ENTRY_Z);
    let exit_z = f64_flag("--exit-z", DEFAULT_EXIT_Z);
    let stop_z = f64_flag("--stop-z", DEFAULT_STOP_Z);
    let sweep = std::env::args().any(|a| a == "--sweep");
    let config = AppConfig::load();
    let client = AlpacaClient::new(config.alpaca.clone(), config.history_limit);
    let fees = match asset_class {
        AssetClass::Crypto => config.fees.clone(),
        AssetClass::Stocks => FeeConfig {
            round_trip_fee_bps: asset_class.default_round_trip_fee_bps(),
        },
    };

    let offset_days = offset_days_from_args();
    let end = Utc::now() - ChronoDuration::days(offset_days);
    let start = end - ChronoDuration::days(lookback_days);
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();

    let symbols: Vec<String> = match asset_class {
        AssetClass::Crypto => {
            println!("Fetching tradable crypto assets from Alpaca...");
            let assets = client.get_assets(Some("crypto".to_string())).await?;
            let mut symbols: Vec<String> = assets
                .iter()
                .filter(|a| a.get("tradable").and_then(|t| t.as_bool()).unwrap_or(false))
                .filter_map(|a| a.get("symbol").and_then(|s| s.as_str()))
                .filter(|s| s.ends_with("/USD") && !s.starts_with("USD")) // drop USD-quoted stablecoins as symbols themselves
                // TRUMP/USD dominated an earlier run's "most cointegrated" ranking almost
                // completely (13 of top 15, including a nonsensical beta=184902 fit against
                // BONK/USD) - a signature of a thinly-traded/newly-listed asset whose own noisy
                // price behavior produces spurious fits against nearly everything, not genuine
                // pairwise cointegration. Excluded to see whether the ranking holds without it.
                .filter(|s| *s != "TRUMP/USD")
                .map(String::from)
                .collect();
            symbols.sort();
            symbols.dedup();
            symbols
        }
        AssetClass::Stocks => config.stock_symbols.clone().unwrap_or_else(liquid_stock_universe),
    };
    println!(
        "Testing {} tradable {} symbols",
        symbols.len(),
        asset_class.label()
    );
    println!();

    let bars_by_symbol =
        fetch_all_bars(&client, asset_class, &symbols, TIMEFRAME, &start_str, &end_str).await;
    println!();
    println!(
        "Testing {} possible ordered pairs for cointegration (half-life {}-{} bars)...",
        bars_by_symbol.len() * bars_by_symbol.len().saturating_sub(1),
        MIN_HALF_LIFE_BARS,
        MAX_HALF_LIFE_BARS
    );

    let mut candidates = find_cointegrated_pairs(&bars_by_symbol);
    candidates.sort_by(|a, b| a.half_life_bars.partial_cmp(&b.half_life_bars).unwrap());

    println!();
    println!("===== Top {} Cointegrated Pairs (by shortest half-life) =====", top_n);
    println!(
        "{:<10} {:<10} {:>10} {:>10} {:>14} {:>12} {:>8}",
        "A", "B", "alpha", "beta", "half_life(m)", "resid_sd", "n"
    );
    for c in candidates.iter().take(top_n) {
        println!(
            "{:<10} {:<10} {:>10.4} {:>10.4} {:>14.1} {:>12.6} {:>8}",
            c.symbol_a, c.symbol_b, c.alpha, c.beta, c.half_life_bars, c.residual_stddev, c.aligned_len
        );
    }
    println!();
    println!(
        "{} candidate pairs found out of {} tested.",
        candidates.len(),
        bars_by_symbol.len() * bars_by_symbol.len().saturating_sub(1)
    );

    if candidates.is_empty() {
        return Ok(());
    }

    let bars_map: HashMap<&String, &Vec<CryptoBar>> =
        bars_by_symbol.iter().map(|(s, b)| (s, b)).collect();

    if sweep {
        println!();
        println!("===== Entry/Exit Z-Score Sweep (top {} pairs, stop_z={:.1}) =====", top_n.min(candidates.len()), stop_z);
        println!(
            "{:>8} {:>8} {:>8} {:>10} {:>12} {:>12} {:>14}",
            "EntryZ", "ExitZ", "Trades", "WinRate%", "TotalPnL$", "AvgPnL$", "Verdict"
        );

        let entry_candidates = [1.5, 2.0, 2.5, 3.0];
        let exit_candidates = [0.0, 0.25, 0.5, 0.75, 1.0];
        let mut best: Option<(f64, f64, f64)> = None; // (entry_z, exit_z, total_pnl)

        for &ez in &entry_candidates {
            for &xz in &exit_candidates {
                let mut sweep_trades = Vec::new();
                for c in candidates.iter().take(top_n) {
                    let (Some(bars_a), Some(bars_b)) =
                        (bars_map.get(&c.symbol_a), bars_map.get(&c.symbol_b))
                    else {
                        continue;
                    };
                    sweep_trades.extend(simulate_market_neutral_pair(
                        &c.symbol_a, &c.symbol_b, bars_a, bars_b, c.alpha, c.beta,
                        &fees, config.defaults.max_order_amount, asset_class, ez, xz, stop_z,
                    ));
                }
                let n = sweep_trades.len();
                let wins = sweep_trades.iter().filter(|t| t.pnl > 0.0).count();
                let total_pnl: f64 = sweep_trades.iter().map(|t| t.pnl).sum();
                let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };
                let avg_pnl = if n > 0 { total_pnl / n as f64 } else { 0.0 };
                let verdict = if total_pnl > 0.0 { "PROFITABLE" } else { "" };
                println!(
                    "{:>8.2} {:>8.2} {:>8} {:>10.1} {:>12.2} {:>12.4} {:>14}",
                    ez, xz, n, win_rate, total_pnl, avg_pnl, verdict
                );
                if best.is_none() || total_pnl > best.unwrap().2 {
                    best = Some((ez, xz, total_pnl));
                }
            }
        }

        if let Some((ez, xz, pnl)) = best {
            println!();
            println!("Best combo: entry_z={:.2} exit_z={:.2} stop_z={:.1} -> total_pnl=${:.2}", ez, xz, stop_z, pnl);
        }

        return Ok(());
    }

    // Backtest the true market-neutral (long+short) strategy on the top pairs found.
    println!();
    println!("===== Backtest: Market-Neutral Long/Short (top {} pairs, entry_z={:.2} exit_z={:.2} stop_z={:.1}) =====", top_n.min(candidates.len()), entry_z, exit_z, stop_z);
    println!(
        "{:<10} {:<10} {:>8} {:>10} {:>12} {:>12}",
        "A", "vs B", "Trades", "WinRate%", "TotalPnL$", "AvgPnL$"
    );

    let mut all_trades: Vec<PairTrade> = Vec::new();
    for c in candidates.iter().take(top_n) {
        let (Some(bars_a), Some(bars_b)) =
            (bars_map.get(&c.symbol_a), bars_map.get(&c.symbol_b))
        else {
            continue;
        };
        let trades = simulate_market_neutral_pair(
            &c.symbol_a,
            &c.symbol_b,
            bars_a,
            bars_b,
            c.alpha,
            c.beta,
            &fees,
            config.defaults.max_order_amount,
            asset_class,
            entry_z,
            exit_z,
            stop_z,
        );
        let n = trades.len();
        let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
        let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };
        let avg_pnl = if n > 0 { total_pnl / n as f64 } else { 0.0 };
        println!(
            "{:<10} {:<10} {:>8} {:>10.1} {:>12.2} {:>12.4}",
            c.symbol_a, c.symbol_b, n, win_rate, total_pnl, avg_pnl
        );
        all_trades.extend(trades);
    }

    let total_trades = all_trades.len();
    let total_pnl: f64 = all_trades.iter().map(|t| t.pnl).sum();
    let total_wins = all_trades.iter().filter(|t| t.pnl > 0.0).count();
    let overall_win_rate = if total_trades > 0 {
        total_wins as f64 / total_trades as f64 * 100.0
    } else {
        0.0
    };
    let dd = {
        let mut cumulative = 0.0;
        let mut peak = 0.0;
        let mut max_dd = 0.0;
        for t in &all_trades {
            cumulative += t.pnl;
            if cumulative > peak {
                peak = cumulative;
            }
            let drawdown = peak - cumulative;
            if drawdown > max_dd {
                max_dd = drawdown;
            }
        }
        max_dd
    };

    println!();
    println!("===== Portfolio Summary (across top {} pairs) =====", top_n.min(candidates.len()));
    println!("Total trades:     {}", total_trades);
    println!("Overall win rate: {:.1}%", overall_win_rate);
    println!("Total PnL:        ${:.2}", total_pnl);
    println!(
        "Avg PnL/trade:    ${:.4}",
        if total_trades > 0 { total_pnl / total_trades as f64 } else { 0.0 }
    );
    println!("Max drawdown:     ${:.2}", dd);

    std::fs::create_dir_all("data").ok();
    let report = serde_json::json!({
        "asset_class": asset_class.label(),
        "lookback_days": lookback_days,
        "pairs_tested": bars_by_symbol.len() * bars_by_symbol.len().saturating_sub(1),
        "cointegrated_candidates_found": candidates.len(),
        "top_pairs": candidates.iter().take(top_n).map(|c| serde_json::json!({
            "symbol_a": c.symbol_a,
            "symbol_b": c.symbol_b,
            "alpha": c.alpha,
            "beta": c.beta,
            "half_life_bars": c.half_life_bars,
            "residual_stddev": c.residual_stddev,
        })).collect::<Vec<_>>(),
        "portfolio": {
            "total_trades": total_trades,
            "overall_win_rate_pct": overall_win_rate,
            "total_pnl": total_pnl,
            "max_drawdown": dd,
        },
        "trades": all_trades,
    });
    let out_path = format!("data/pairs_results_{}.json", asset_class.label());
    std::fs::write(&out_path, serde_json::to_string_pretty(&report)?)?;
    println!();
    println!("Full results written to {}", out_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: &str, close: f64) -> CryptoBar {
        CryptoBar {
            timestamp: t.to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 100.0,
        }
    }

    fn ts(i: usize) -> String {
        format!("2026-01-01T{:02}:{:02}:00Z", i / 60, i % 60)
    }

    /// Regression test for the exit-leg mixup bug: exit prices were always computed from
    /// symbol A's/B's close regardless of which one was actually the long vs short leg,
    /// so a "short_a_long_b" trade priced its exits backwards - producing PnL in the tens
    /// of thousands of dollars on $100-notional-per-leg trades instead of a few dollars.
    #[test]
    fn test_market_neutral_short_a_long_b_attributes_pnl_to_correct_leg() {
        // Baseline: A and B track together so the rolling residual window is tight around
        // zero (needed before any z-score is even computable).
        let mut bars_a = Vec::new();
        let mut bars_b = Vec::new();
        for i in 0..90 {
            bars_a.push(bar(&ts(i), 100.0));
            bars_b.push(bar(&ts(i), 100.0));
        }
        // A jumps well above B - A becomes expensive relative to B, should trigger a
        // short_a_long_b entry (short the now-expensive leg, long the cheap one).
        for i in 90..95 {
            bars_a.push(bar(&ts(i), 115.0));
            bars_b.push(bar(&ts(i), 100.0));
        }
        // A reverts back to B's level - the spread should revert and close the trade.
        for i in 95..115 {
            bars_a.push(bar(&ts(i), 100.0));
            bars_b.push(bar(&ts(i), 100.0));
        }

        let fees = FeeConfig {
            round_trip_fee_bps: 2.0,
        };
        let trades = simulate_market_neutral_pair(
            "A",
            "B",
            &bars_a,
            &bars_b,
            0.0,
            1.0,
            &fees,
            100.0,
            AssetClass::Stocks,
            DEFAULT_ENTRY_Z,
            DEFAULT_EXIT_Z,
            DEFAULT_STOP_Z,
        );

        assert!(!trades.is_empty(), "expected at least one trade to trigger");
        let t = &trades[0];
        assert_eq!(t.direction, "short_a_long_b");
        // The long leg must be B, not A - exactly the mapping the bug got backwards.
        assert_eq!(t.long_symbol, "B");
        assert_eq!(t.short_symbol, "A");
        // Sanity bound: PnL on a $100-notional-per-leg trade must be a small multiple of
        // the notional, not tens of thousands of dollars (what the bug produced).
        assert!(
            t.pnl.abs() < 100.0,
            "PnL magnitude {} is implausible for $100/leg notional - likely the exit-leg mixup bug",
            t.pnl
        );
        // A fell back to exactly B's entry level (a full round-trip with no net price
        // capture once the position closes flat), so the only real PnL driver here is
        // fee/spread drag - expect a small loss, not the near-zero-magnitude sanity bound
        // already checked above being violated in the other direction (a large win would
        // be just as suspicious as a large loss for a round trip that captured no net move).
        assert!(
            t.pnl < 0.0 && t.pnl > -5.0,
            "expected a small fee-driven loss on a flat round trip, got pnl={}",
            t.pnl
        );
    }
}
