//! Cross-sectional momentum rotation: at each rebalance point, ranks every symbol in the
//! universe by its own trailing return over `lookback_days`, holds the top `top_n` roughly
//! equal-weighted, and only trades to enter new top-N members or exit ones that fell out -
//! existing holdings that remain in the top N are left alone (untouched compounding, no
//! unnecessary turnover/fees).
//!
//! This is a fundamentally different kind of strategy than everything else in this
//! project: every other signal (`backtest.rs`, `pairs.rs`, `portfolio.rs`'s other modes)
//! answers "should I buy/sell THIS symbol right now" from its own short-horizon price
//! history - a time-series question. This answers "which symbols, relative to each other,
//! are outperforming right now" - a cross-sectional question, the basis of the classical
//! momentum-factor literature (Jegadeesh & Titman 1993 and a large body of subsequent
//! research). It also stays invested with close to 100% of capital continuously, unlike
//! every signal-driven mode in `portfolio.rs`, which only deploys a small fraction of cash
//! per trade and is frequently flat - closing the "opportunity cost of being in cash while
//! the market rises" gap that's shown up as the dominant driver of underperformance vs.
//! buy-and-hold throughout this project's backtests so far.
//!
//! Caveat: academic momentum-factor studies typically use 3-12 month lookbacks and monthly
//! rebalancing on decades of data. This backtest is necessarily compressed into whatever
//! window of historical bars is available (weeks, not years) - a noisier, higher-frequency
//! approximation of the real factor, not a faithful replication of the published research.
//!
//! Usage: `cargo run --bin rotation --release -- --starting-cash 500 --days 60 --top-n 8
//! --lookback-days 20 --rebalance-days 5 --stop-loss-pct 10 [--asset-class stocks]
//! [--offset-days N]`

use chrono::{Duration as ChronoDuration, Utc};
use rust_autohedge::backtest_support::{fetch_all_bars, liquid_stock_universe, AssetClass};
use rust_autohedge::config::{AppConfig, FeeConfig};
use rust_autohedge::data::alpaca::{AlpacaClient, CryptoBar};
use serde::Serialize;
use std::collections::HashMap;

const DEFAULT_STARTING_CASH: f64 = 500.0;
const DEFAULT_LOOKBACK_DAYS: i64 = 60;
const TIMEFRAME: &str = "1Day";

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

fn usize_arg(flag: &str, default: usize) -> usize {
    std::env::args()
        .skip_while(|a| a != flag)
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(default)
}

fn asset_class_from_args() -> AssetClass {
    std::env::args()
        .skip_while(|a| a != "--asset-class")
        .nth(1)
        .map(|s| AssetClass::from_arg(&s))
        .unwrap_or(AssetClass::Stocks)
}

struct Holding {
    entry_price: f64,
    qty: f64,
    cost_basis: f64,
}

#[derive(Serialize, Clone)]
struct RebalanceEvent {
    day: String,
    action: &'static str, // "buy" | "sell" | "stop_loss"
    symbol: String,
    price: f64,
    qty: f64,
    pnl: Option<f64>,
}

#[derive(Serialize, Clone)]
struct EquitySnapshot {
    day: String,
    equity: f64,
    holdings: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let asset_class = asset_class_from_args();
    let starting_cash = f64_arg("--starting-cash", DEFAULT_STARTING_CASH);
    let lookback_window_days = i64_arg("--days", DEFAULT_LOOKBACK_DAYS);
    let offset_days = i64_arg("--offset-days", 0);
    let top_n = usize_arg("--top-n", 8);
    let ranking_lookback_days = usize_arg("--lookback-days", 20);
    let rebalance_days = usize_arg("--rebalance-days", 5).max(1);
    let stop_loss_pct = f64_arg("--stop-loss-pct", 10.0);

    let config = AppConfig::load();
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

    let sim_end = Utc::now() - ChronoDuration::days(offset_days);
    let sim_start = sim_end - ChronoDuration::days(lookback_window_days);
    // Fetch extra history before sim_start so the very first rebalance has a full
    // `ranking_lookback_days` trading days of trailing-return data to rank on.
    let fetch_start = sim_start - ChronoDuration::days(ranking_lookback_days as i64 * 3 / 2 + 10);

    println!(
        "Rotation strategy: asset_class={} starting_cash=${:.2} top_n={} ranking_lookback={}d rebalance_every={}d stop_loss={:.1}%",
        asset_class.label(), starting_cash, top_n, ranking_lookback_days, rebalance_days, stop_loss_pct
    );
    println!("window: {} -> {}", sim_start.to_rfc3339(), sim_end.to_rfc3339());
    println!();

    let bars_by_symbol = fetch_all_bars(
        &client, asset_class, &symbols, TIMEFRAME, &fetch_start.to_rfc3339(), &sim_end.to_rfc3339(),
    ).await;
    println!();

    // Index each symbol's daily closes by day string, and collect the sorted union of
    // trading days actually present within the simulation window (not the pre-window
    // ranking buffer).
    let sim_start_day = sim_start.format("%Y-%m-%d").to_string();
    let mut by_symbol: HashMap<String, Vec<&CryptoBar>> = HashMap::new();
    let mut all_days: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (symbol, bars) in &bars_by_symbol {
        let mut v: Vec<&CryptoBar> = bars.iter().collect();
        v.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        for bar in &v {
            let day = bar.timestamp[..10.min(bar.timestamp.len())].to_string();
            if day.as_str() >= sim_start_day.as_str() {
                all_days.insert(day);
            }
        }
        by_symbol.insert(symbol.clone(), v);
    }
    let sim_days: Vec<String> = all_days.into_iter().collect();
    println!("{} trading days in simulation window across {} symbols", sim_days.len(), by_symbol.len());

    if sim_days.is_empty() {
        println!("No trading days in window - nothing to simulate.");
        return Ok(());
    }

    // For each symbol, build a day -> close lookup (only needs to cover the whole fetched
    // range, since ranking looks back before sim_start too).
    let mut close_by_day: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for (symbol, bars) in &by_symbol {
        let mut m = HashMap::new();
        for bar in bars {
            let day = bar.timestamp[..10.min(bar.timestamp.len())].to_string();
            m.insert(day, bar.close);
        }
        close_by_day.insert(symbol.clone(), m);
    }
    // Flat, sorted list of every day each symbol has a bar for, used to walk back
    // `ranking_lookback_days` trading days (not calendar days) for the trailing return.
    let mut days_by_symbol: HashMap<String, Vec<String>> = HashMap::new();
    for (symbol, bars) in &by_symbol {
        days_by_symbol.insert(symbol.clone(), bars.iter().map(|b| b.timestamp[..10.min(b.timestamp.len())].to_string()).collect());
    }

    let trailing_return = |symbol: &str, as_of_day: &str| -> Option<f64> {
        let days = days_by_symbol.get(symbol)?;
        let idx = days.iter().rposition(|d| d.as_str() <= as_of_day)?;
        if idx < ranking_lookback_days {
            return None;
        }
        let now_close = *close_by_day.get(symbol)?.get(&days[idx])?;
        let past_close = *close_by_day.get(symbol)?.get(&days[idx - ranking_lookback_days])?;
        if past_close <= 0.0 {
            return None;
        }
        Some((now_close - past_close) / past_close)
    };

    let mut cash = starting_cash;
    let mut holdings: HashMap<String, Holding> = HashMap::new();
    let mut events: Vec<RebalanceEvent> = Vec::new();
    let mut equity_curve: Vec<EquitySnapshot> = Vec::new();

    for (day_idx, day) in sim_days.iter().enumerate() {
        // Daily stop-loss check on every current holding, using that day's low if
        // available (falls back to close if a symbol's bar is missing that day).
        let held_symbols: Vec<String> = holdings.keys().cloned().collect();
        for symbol in held_symbols {
            let Some(&price) = close_by_day.get(&symbol).and_then(|m| m.get(day)) else {
                continue;
            };
            let holding = &holdings[&symbol];
            let loss_pct = (holding.entry_price - price) / holding.entry_price * 100.0;
            if loss_pct >= stop_loss_pct {
                let h = holdings.remove(&symbol).unwrap();
                let fee_cost = h.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
                let proceeds = price * h.qty - fee_cost;
                cash += proceeds;
                events.push(RebalanceEvent {
                    day: day.clone(), action: "stop_loss", symbol: symbol.clone(),
                    price, qty: h.qty, pnl: Some(proceeds - h.cost_basis),
                });
            }
        }

        // Rebalance on day 0 and every `rebalance_days` trading days after.
        if day_idx % rebalance_days == 0 {
            let mut ranked: Vec<(String, f64)> = symbols
                .iter()
                .filter_map(|s| trailing_return(s, day).map(|r| (s.clone(), r)))
                .collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            let target: std::collections::HashSet<String> = ranked.iter().take(top_n).map(|(s, _)| s.clone()).collect();

            // Exit anything no longer in the target set.
            let held_symbols: Vec<String> = holdings.keys().cloned().collect();
            for symbol in held_symbols {
                if target.contains(&symbol) {
                    continue;
                }
                let Some(&price) = close_by_day.get(&symbol).and_then(|m| m.get(day)) else {
                    continue;
                };
                let h = holdings.remove(&symbol).unwrap();
                let fee_cost = h.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
                let proceeds = price * h.qty - fee_cost;
                cash += proceeds;
                events.push(RebalanceEvent {
                    day: day.clone(), action: "sell", symbol: symbol.clone(),
                    price, qty: h.qty, pnl: Some(proceeds - h.cost_basis),
                });
            }

            // Equity-based equal-weight target per slot; existing holdings that remain in
            // the target set are left untouched (no forced rebalancing/turnover on them).
            let equity: f64 = cash
                + holdings
                    .iter()
                    .map(|(s, h)| close_by_day.get(s).and_then(|m| m.get(day)).copied().unwrap_or(h.entry_price) * h.qty)
                    .sum::<f64>();
            let per_slot = if top_n > 0 { equity / top_n as f64 } else { 0.0 };

            for symbol in &target {
                if holdings.contains_key(symbol) {
                    continue; // already held, leave it running
                }
                let Some(&price) = close_by_day.get(symbol).and_then(|m| m.get(day)) else {
                    continue;
                };
                if price <= 0.0 {
                    continue;
                }
                let notional = per_slot.min(cash);
                if notional < 1.0 {
                    continue; // not enough cash left to open a meaningful slot
                }
                let qty = notional / price;
                cash -= notional;
                holdings.insert(symbol.clone(), Holding { entry_price: price, qty, cost_basis: notional });
                events.push(RebalanceEvent {
                    day: day.clone(), action: "buy", symbol: symbol.clone(), price, qty, pnl: None,
                });
            }
        }

        let equity: f64 = cash
            + holdings
                .iter()
                .map(|(s, h)| close_by_day.get(s).and_then(|m| m.get(day)).copied().unwrap_or(h.entry_price) * h.qty)
                .sum::<f64>();
        equity_curve.push(EquitySnapshot { day: day.clone(), equity, holdings: holdings.len() });
    }

    // Liquidate everything at the last available price.
    let last_day = sim_days.last().unwrap().clone();
    for (symbol, h) in holdings.drain() {
        let price = close_by_day.get(&symbol).and_then(|m| m.get(&last_day)).copied().unwrap_or(h.entry_price);
        let fee_cost = h.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
        let proceeds = price * h.qty - fee_cost;
        cash += proceeds;
        events.push(RebalanceEvent {
            day: last_day.clone(), action: "sell", symbol: symbol.clone(),
            price, qty: h.qty, pnl: Some(proceeds - h.cost_basis),
        });
    }

    let final_value = cash;
    let total_return_pct = (final_value - starting_cash) / starting_cash * 100.0;

    // Buy-and-hold baseline: equal-weight all symbols with data, first close in the
    // simulation window to last close.
    let bh_symbols: Vec<&String> = symbols.iter().filter(|s| {
        close_by_day.get(*s).map(|m| m.keys().any(|d| d.as_str() >= sim_start_day.as_str())).unwrap_or(false)
    }).collect();
    let bh_per_symbol = if bh_symbols.is_empty() { 0.0 } else { starting_cash / bh_symbols.len() as f64 };
    let bh_final_value: f64 = bh_symbols
        .iter()
        .map(|s| {
            let m = &close_by_day[*s];
            let first = sim_days.iter().find_map(|d| m.get(d)).copied().unwrap_or(0.0);
            let last = sim_days.iter().rev().find_map(|d| m.get(d)).copied().unwrap_or(0.0);
            if first <= 0.0 { bh_per_symbol } else { bh_per_symbol * (last / first) }
        })
        .sum();
    let bh_return_pct = (bh_final_value - starting_cash) / starting_cash * 100.0;
    let excess_vs_bh = final_value - bh_final_value;

    println!();
    println!("===== Rotation Strategy Result =====");
    println!("Starting cash:    ${:.2}", starting_cash);
    println!("Ending value:     ${:.2}", final_value);
    println!("Total return:     {:.2}%", total_return_pct);
    println!("Rebalance events: {}", events.len());
    println!();
    println!("Buy & hold ({} symbols, equal-weight): ${:.2} ({:.2}%)", bh_symbols.len(), bh_final_value, bh_return_pct);
    println!("Excess vs buy & hold: ${:.2}", excess_vs_bh);
    println!();
    println!("Equity curve (every ~5th day):");
    for (i, snap) in equity_curve.iter().enumerate() {
        if i % 5 == 0 || i == equity_curve.len() - 1 {
            println!("  {}  ${:.2}  ({} holdings)", snap.day, snap.equity, snap.holdings);
        }
    }

    std::fs::create_dir_all("data").ok();
    let out_path = format!(
        "data/rotation_{}_{}_{}d_top{}_lb{}_rb{}.json",
        asset_class.label(), starting_cash as i64, lookback_window_days, top_n, ranking_lookback_days, rebalance_days
    );
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "asset_class": asset_class.label(),
            "starting_cash": starting_cash,
            "top_n": top_n,
            "ranking_lookback_days": ranking_lookback_days,
            "rebalance_days": rebalance_days,
            "stop_loss_pct": stop_loss_pct,
            "final_value": final_value,
            "total_return_pct": total_return_pct,
            "buy_and_hold_final_value": bh_final_value,
            "buy_and_hold_return_pct": bh_return_pct,
            "excess_vs_buy_and_hold": excess_vs_bh,
            "equity_curve": equity_curve,
            "events": events,
        }))?,
    )?;
    println!();
    println!("Full results written to {}", out_path);

    Ok(())
}
