//! PEAD (post-earnings-announcement-drift) backtest: the one signal this project's entire
//! strategy search couldn't test until now, since Alpaca has no earnings-estimate/surprise
//! data. Uses Finnhub (`config.yaml`'s `finnhub:` section) for EPS actual/estimate, and
//! enters a position after a qualifying surprise, holding for a fixed `drift_days` - the
//! well-documented empirical finding (Ball & Brown 1968, Bernard & Thomas 1989, and a large
//! body of subsequent research) that a stock's price keeps drifting in the direction of an
//! earnings surprise for weeks afterward, rather than fully repricing instantly.
//!
//! ## Data limitation, read before trusting any result from this binary
//! Finnhub's free tier has two earnings endpoints, both capped: `/calendar/earnings`
//! carries the real report date but only returns each symbol's single MOST RECENT report
//! (confirmed by direct testing - requesting years of history returns the same one event
//! regardless of range), which is useless for backtesting. `/stock/earnings` returns up to
//! ~4 quarters of surprise-percent history but only the fiscal PERIOD END date, not the
//! actual report date (real companies report 2-6 weeks after quarter end, with real
//! variance). This binary uses the latter and approximates the report date as period-end
//! plus `--report-lag-days` (default 30) - a genuine approximation, not a measured date.
//! Any PEAD result here should be read with that imprecision in mind; a paid Finnhub tier
//! (or a different data source) would be needed for a dates-exact historical backtest.
//!
//! Usage: `cargo run --bin pead --release -- --starting-cash 500 --days 700
//! --surprise-threshold-pct 5 --drift-days 20 --stop-loss-pct 15 --report-lag-days 30
//! [--allow-short] [--asset-class stocks] [--offset-days N]`

use chrono::{Duration as ChronoDuration, Utc};
use rust_autohedge::backtest_support::{fetch_all_bars, liquid_stock_universe, AssetClass};
use rust_autohedge::config::{AppConfig, FeeConfig};
use rust_autohedge::data::alpaca::AlpacaClient;
use rust_autohedge::data::finnhub::FinnhubClient;
use serde::Serialize;
use std::collections::HashMap;

const DEFAULT_STARTING_CASH: f64 = 500.0;
const DEFAULT_LOOKBACK_DAYS: i64 = 60;
const TIMEFRAME: &str = "1Day";
const DEFAULT_TARGET_BALANCE_PCT: f64 = 0.10; // risk 10% of current equity per earnings event
const DEFAULT_MAX_ORDER_AMOUNT: f64 = 150.0;
const MIN_ORDER_AMOUNT: f64 = 5.0;
/// Finnhub free tier is 60 calls/min - stay comfortably under that across the universe.
const FINNHUB_CALL_DELAY_MS: u64 = 1100;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Direction {
    Long,
    Short,
}

fn f64_arg(flag: &str, default: f64) -> f64 {
    std::env::args().skip_while(|a| a != flag).nth(1).and_then(|s| s.parse::<f64>().ok()).unwrap_or(default)
}
fn i64_arg(flag: &str, default: i64) -> i64 {
    std::env::args().skip_while(|a| a != flag).nth(1).and_then(|s| s.parse::<i64>().ok()).unwrap_or(default)
}
fn usize_arg(flag: &str, default: usize) -> usize {
    std::env::args().skip_while(|a| a != flag).nth(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(default)
}
fn asset_class_from_args() -> AssetClass {
    std::env::args().skip_while(|a| a != "--asset-class").nth(1).map(|s| AssetClass::from_arg(&s)).unwrap_or(AssetClass::Stocks)
}

struct PendingEntry {
    symbol: String,
    direction: Direction,
    surprise_pct: f64,
}

struct Holding {
    direction: Direction,
    entry_price: f64,
    entry_day_idx: usize,
    qty: f64,
    cost_basis: f64,
}

#[derive(Serialize, Clone)]
struct PeadTrade {
    symbol: String,
    direction: &'static str,
    surprise_pct: f64,
    entry_day: String,
    exit_day: String,
    entry_price: f64,
    exit_price: f64,
    exit_reason: &'static str,
    pnl: f64,
}

#[derive(Serialize, Clone)]
struct EquitySnapshot {
    day: String,
    equity: f64,
    holdings: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder().with_max_level(tracing::Level::WARN).finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let asset_class = asset_class_from_args();
    let starting_cash = f64_arg("--starting-cash", DEFAULT_STARTING_CASH);
    let lookback_window_days = i64_arg("--days", DEFAULT_LOOKBACK_DAYS);
    let offset_days = i64_arg("--offset-days", 0);
    let surprise_threshold_pct = f64_arg("--surprise-threshold-pct", 5.0);
    let drift_days = usize_arg("--drift-days", 20);
    let stop_loss_pct = f64_arg("--stop-loss-pct", 15.0);
    let report_lag_days = i64_arg("--report-lag-days", 30);
    let allow_short = std::env::args().any(|a| a == "--allow-short");
    let target_balance_pct = f64_arg("--position-pct", DEFAULT_TARGET_BALANCE_PCT);
    let max_order_amount = f64_arg("--max-order", DEFAULT_MAX_ORDER_AMOUNT);

    let config = AppConfig::load();
    let Some(finnhub_cfg) = config.finnhub.clone() else {
        eprintln!("No `finnhub:` section in config.yaml - PEAD needs a Finnhub API key. See docs/STRATEGIES.md.");
        std::process::exit(1);
    };
    let finnhub = FinnhubClient::new(finnhub_cfg);
    let client = AlpacaClient::new(config.alpaca.clone(), config.history_limit);
    let fees = match asset_class {
        AssetClass::Crypto => config.fees.clone(),
        AssetClass::Stocks => FeeConfig { round_trip_fee_bps: asset_class.default_round_trip_fee_bps() },
    };
    let symbols = match asset_class {
        AssetClass::Crypto => config.symbols.clone(),
        AssetClass::Stocks => config.stock_symbols.clone().unwrap_or_else(liquid_stock_universe),
    };

    let sim_end = Utc::now() - ChronoDuration::days(offset_days);
    let sim_start = sim_end - ChronoDuration::days(lookback_window_days);
    let sim_start_day = sim_start.format("%Y-%m-%d").to_string();
    let sim_end_day = sim_end.format("%Y-%m-%d").to_string();

    println!(
        "PEAD backtest: asset_class={} starting_cash=${:.2} surprise_threshold={:.1}% drift_days={} stop_loss={:.1}% allow_short={}",
        asset_class.label(), starting_cash, surprise_threshold_pct, drift_days, stop_loss_pct, allow_short
    );
    println!("window: {} -> {}", sim_start_day, sim_end_day);
    println!();

    let bars_by_symbol = fetch_all_bars(&client, asset_class, &symbols, TIMEFRAME, &sim_start.to_rfc3339(), &sim_end.to_rfc3339()).await;
    println!();

    // Daily close/high/low lookups + sorted trading-day list, built BEFORE resolving
    // earnings entry days, so each event can be mapped onto a real trading day (weekend/
    // holiday report dates roll forward, never silently vanish).
    let mut close_by_day: HashMap<String, HashMap<String, f64>> = HashMap::new();
    let mut low_by_day: HashMap<String, HashMap<String, f64>> = HashMap::new();
    let mut all_days: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (symbol, bars) in &bars_by_symbol {
        let mut closes = HashMap::new();
        let mut lows = HashMap::new();
        for bar in bars {
            let day = bar.timestamp[..10.min(bar.timestamp.len())].to_string();
            closes.insert(day.clone(), bar.close);
            lows.insert(day.clone(), bar.low);
            all_days.insert(day);
        }
        close_by_day.insert(symbol.clone(), closes);
        low_by_day.insert(symbol.clone(), lows);
    }
    let sim_days: Vec<String> = all_days.into_iter().collect();
    if sim_days.is_empty() {
        println!("No trading days in window - nothing to simulate.");
        return Ok(());
    }
    println!("{} trading days in simulation window across {} symbols", sim_days.len(), bars_by_symbol.len());

    // First trading day strictly greater than `d` (for after-close/unspecified-timing
    // reports) or the first trading day >= `d` (for before-open reports) - resolves
    // weekend/holiday report dates onto the correct real session instead of dropping them.
    let first_trading_day_at_or_after = |d: &str| -> Option<usize> { sim_days.iter().position(|td| td.as_str() >= d) };

    println!(
        "Fetching earnings history from Finnhub for {} symbols (report dates approximated as period-end + {}d - see module doc)...",
        symbols.len(), report_lag_days
    );
    let mut pending_by_day: HashMap<String, Vec<PendingEntry>> = HashMap::new();
    let mut total_events = 0usize;
    for symbol in &symbols {
        match finnhub.get_earnings_history(symbol).await {
            Ok(events) => {
                for ev in &events {
                    let Some(surprise) = ev.surprise_percent else { continue };
                    let Some(approx_date) = ev.approximate_report_date(report_lag_days) else { continue };
                    if approx_date.as_str() < sim_start_day.as_str() || approx_date.as_str() > sim_end_day.as_str() {
                        continue; // approximated date falls outside the backtest window
                    }
                    total_events += 1;
                    let direction = if surprise >= surprise_threshold_pct {
                        Some(Direction::Long)
                    } else if surprise <= -surprise_threshold_pct && allow_short {
                        Some(Direction::Short)
                    } else {
                        None
                    };
                    let Some(direction) = direction else { continue };
                    // The report date is already an approximation, so there's no reliable
                    // before/after-market timing to key off - always wait for the next
                    // trading day at/after the approximated date, the more conservative
                    // choice (never assumes same-day reaction to an imprecise timestamp).
                    let Some(idx) = first_trading_day_at_or_after(&approx_date) else { continue };
                    let entry_day = sim_days[idx].clone();
                    pending_by_day.entry(entry_day).or_default().push(PendingEntry {
                        symbol: symbol.clone(), direction, surprise_pct: surprise,
                    });
                }
            }
            Err(e) => println!("  {} earnings fetch failed: {}", symbol, e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(FINNHUB_CALL_DELAY_MS)).await;
    }
    let qualifying_events: usize = pending_by_day.values().map(|v| v.len()).sum();
    println!("{} earnings events found, {} cleared the +/-{:.1}% surprise threshold", total_events, qualifying_events, surprise_threshold_pct);
    println!();

    let mut cash = starting_cash;
    let mut holdings: HashMap<String, Holding> = HashMap::new();
    let mut trades: Vec<PeadTrade> = Vec::new();
    let mut equity_curve: Vec<EquitySnapshot> = Vec::new();

    for (day_idx, day) in sim_days.iter().enumerate() {
        // Exit anything that's reached its drift horizon or hit the stop-loss.
        let held_symbols: Vec<String> = holdings.keys().cloned().collect();
        for symbol in held_symbols {
            let Some(&price) = close_by_day.get(&symbol).and_then(|m| m.get(day)) else { continue };
            let h = &holdings[&symbol];
            let days_held = day_idx.saturating_sub(h.entry_day_idx);
            let low = low_by_day.get(&symbol).and_then(|m| m.get(day)).copied().unwrap_or(price);

            let stop_hit = match h.direction {
                Direction::Long => (h.entry_price - low) / h.entry_price * 100.0 >= stop_loss_pct,
                Direction::Short => (price - h.entry_price) / h.entry_price * 100.0 >= stop_loss_pct,
            };
            let horizon_hit = days_held >= drift_days;
            if !stop_hit && !horizon_hit {
                continue;
            }
            let h = holdings.remove(&symbol).unwrap();
            let fee_cost = h.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
            let pnl = match h.direction {
                Direction::Long => {
                    let proceeds = price * h.qty - fee_cost;
                    cash += proceeds;
                    proceeds - h.cost_basis
                }
                Direction::Short => {
                    let cost_to_cover = price * h.qty + fee_cost;
                    cash -= cost_to_cover;
                    h.cost_basis - cost_to_cover
                }
            };
            trades.push(PeadTrade {
                symbol: symbol.clone(),
                direction: if h.direction == Direction::Long { "long" } else { "short" },
                surprise_pct: 0.0, // filled in at entry-time trade record below; exit rows leave 0.0
                entry_day: sim_days[h.entry_day_idx].clone(),
                exit_day: day.clone(),
                entry_price: h.entry_price,
                exit_price: price,
                exit_reason: if stop_hit { "stop_loss" } else { "drift_horizon" },
                pnl,
            });
        }

        // Open any positions scheduled for today (or, for entries dated on a non-trading
        // day - a weekend "bmo" report, for instance - the first trading day at/after it).
        if let Some(entries) = pending_by_day.remove(day) {
            for entry in entries {
                if holdings.contains_key(&entry.symbol) {
                    continue; // already in a position for this symbol, skip overlapping event
                }
                let Some(&price) = close_by_day.get(&entry.symbol).and_then(|m| m.get(day)) else { continue };
                if price <= 0.0 {
                    continue;
                }
                let notional = (cash * target_balance_pct).clamp(0.0, max_order_amount);
                if notional < MIN_ORDER_AMOUNT || notional > cash {
                    continue;
                }
                let qty = notional / price;
                match entry.direction {
                    Direction::Long => cash -= notional,
                    Direction::Short => cash += notional,
                }
                holdings.insert(entry.symbol.clone(), Holding {
                    direction: entry.direction, entry_price: price, entry_day_idx: day_idx, qty, cost_basis: notional,
                });
                trades.push(PeadTrade {
                    symbol: entry.symbol.clone(),
                    direction: if entry.direction == Direction::Long { "long" } else { "short" },
                    surprise_pct: entry.surprise_pct,
                    entry_day: day.clone(),
                    exit_day: String::new(),
                    entry_price: price,
                    exit_price: 0.0,
                    exit_reason: "entry",
                    pnl: 0.0,
                });
            }
        }

        let equity: f64 = cash + holdings.iter().map(|(s, h)| {
            let last = close_by_day.get(s).and_then(|m| m.get(day)).copied().unwrap_or(h.entry_price);
            match h.direction { Direction::Long => last * h.qty, Direction::Short => -(last * h.qty) }
        }).sum::<f64>();
        equity_curve.push(EquitySnapshot { day: day.clone(), equity, holdings: holdings.len() });
    }

    // Liquidate anything still open at the end.
    let last_day = sim_days.last().unwrap().clone();
    for (symbol, h) in holdings.drain() {
        let price = close_by_day.get(&symbol).and_then(|m| m.get(&last_day)).copied().unwrap_or(h.entry_price);
        let fee_cost = h.cost_basis * (fees.round_trip_fee_bps / 10_000.0);
        let pnl = match h.direction {
            Direction::Long => { let proceeds = price * h.qty - fee_cost; cash += proceeds; proceeds - h.cost_basis }
            Direction::Short => { let cost_to_cover = price * h.qty + fee_cost; cash -= cost_to_cover; h.cost_basis - cost_to_cover }
        };
        trades.push(PeadTrade {
            symbol: symbol.clone(),
            direction: if h.direction == Direction::Long { "long" } else { "short" },
            surprise_pct: 0.0,
            entry_day: sim_days[h.entry_day_idx].clone(),
            exit_day: "backtest_end".to_string(),
            entry_price: h.entry_price,
            exit_price: price,
            exit_reason: "still_open_at_backtest_end",
            pnl,
        });
    }

    let final_value = cash;
    let total_return_pct = (final_value - starting_cash) / starting_cash * 100.0;
    let closed: Vec<&PeadTrade> = trades.iter().filter(|t| t.exit_reason != "entry").collect();
    let wins = closed.iter().filter(|t| t.pnl > 0.0).count();
    let win_rate = if !closed.is_empty() { wins as f64 / closed.len() as f64 * 100.0 } else { 0.0 };

    let bh_symbols: Vec<&String> = symbols.iter().filter(|s| close_by_day.get(*s).map(|m| !m.is_empty()).unwrap_or(false)).collect();
    let bh_per_symbol = if bh_symbols.is_empty() { 0.0 } else { starting_cash / bh_symbols.len() as f64 };
    let bh_final_value: f64 = bh_symbols.iter().map(|s| {
        let m = &close_by_day[*s];
        let first = sim_days.iter().find_map(|d| m.get(d)).copied().unwrap_or(0.0);
        let last = sim_days.iter().rev().find_map(|d| m.get(d)).copied().unwrap_or(0.0);
        if first <= 0.0 { bh_per_symbol } else { bh_per_symbol * (last / first) }
    }).sum();
    let bh_return_pct = if starting_cash > 0.0 { (bh_final_value - starting_cash) / starting_cash * 100.0 } else { 0.0 };
    let excess_vs_bh = final_value - bh_final_value;

    println!();
    println!("===== PEAD Result =====");
    println!("Starting cash:    ${:.2}", starting_cash);
    println!("Ending value:     ${:.2}", final_value);
    println!("Total return:     {:.2}%", total_return_pct);
    println!("Closed trades:    {}", closed.len());
    println!("Win rate:         {:.1}%", win_rate);
    println!();
    println!("Buy & hold ({} symbols, equal-weight): ${:.2} ({:.2}%)", bh_symbols.len(), bh_final_value, bh_return_pct);
    println!("Excess vs buy & hold: ${:.2}", excess_vs_bh);

    std::fs::create_dir_all("data").ok();
    let out_path = format!("data/pead_{}_{}_{}d_offset{}_th{}_dd{}.json",
        asset_class.label(), starting_cash as i64, lookback_window_days, offset_days, surprise_threshold_pct as i64, drift_days);
    std::fs::write(&out_path, serde_json::to_string_pretty(&serde_json::json!({
        "asset_class": asset_class.label(), "starting_cash": starting_cash,
        "surprise_threshold_pct": surprise_threshold_pct, "drift_days": drift_days, "stop_loss_pct": stop_loss_pct,
        "allow_short": allow_short, "final_value": final_value, "total_return_pct": total_return_pct,
        "closed_trades": closed.len(), "win_rate_pct": win_rate,
        "buy_and_hold_final_value": bh_final_value, "buy_and_hold_return_pct": bh_return_pct,
        "excess_vs_buy_and_hold": excess_vs_bh, "equity_curve": equity_curve, "trades": trades,
    }))?)?;
    println!();
    println!("Full results written to {}", out_path);

    Ok(())
}
