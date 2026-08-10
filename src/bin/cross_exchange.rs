//! Tests a genuinely different data source: does a price gap between Alpaca (our only
//! trading venue) and Binance.US (a separate, independent exchange) predict a short-term
//! move on Alpaca? If Alpaca lags a Binance.US move, buying on Alpaca and betting it
//! catches up is a real, executable signal that doesn't require order-book depth or any
//! paid data provider - Binance.US's historical klines are a free, unauthenticated public
//! API.
//!
//! Binance.com (global) is geo-restricted from US IPs; Binance.US is the separate,
//! US-compliant exchange with its own API (`api.binance.us`) and direct USD-quoted pairs
//! (no USDT conversion needed), used here instead.
//!
//! Usage: `cargo run --bin cross_exchange --release -- [--days N]`

use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rust_autohedge::backtest_support::{fetch_all_bars, max_drawdown, AssetClass, Trade};
use rust_autohedge::config::AppConfig;
use rust_autohedge::data::alpaca::{AlpacaClient, CryptoBar};
use serde_json::Value;
use std::collections::HashMap;

const DEFAULT_LOOKBACK_DAYS: i64 = 60;
const TIMEFRAME: &str = "1Min";
const ENTRY_THRESHOLD_BPS: f64 = 30.0; // how far Alpaca must lag Binance.US to trigger a buy
const ZSCORE_WINDOW: usize = 60;

fn days_from_args() -> i64 {
    std::env::args()
        .skip_while(|a| a != "--days")
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_LOOKBACK_DAYS)
}

/// Alpaca canonical "SOL/USD" -> Binance.US "SOLUSD".
fn to_binance_us_symbol(canonical: &str) -> String {
    canonical.replace('/', "")
}

/// Fetches 1-minute klines from Binance.US's free, unauthenticated public API, paginating
/// via startTime until `end_ms` is reached. Returns bars in the same `CryptoBar` shape
/// Alpaca's client uses, so the rest of the backtest machinery doesn't need to know which
/// exchange a bar came from.
async fn fetch_binance_us_klines(
    client: &reqwest::Client,
    symbol: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<CryptoBar>, Box<dyn std::error::Error + Send + Sync>> {
    let mut bars = Vec::new();
    let mut cursor = start_ms;

    loop {
        let url = format!(
            "https://api.binance.us/api/v3/klines?symbol={}&interval=1m&startTime={}&endTime={}&limit=1000",
            symbol, cursor, end_ms
        );
        let resp = client.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Binance.US klines failed ({}): {}", status, body).into());
        }
        let rows: Vec<Value> = serde_json::from_str(&body)?;
        if rows.is_empty() {
            break;
        }
        let mut last_close_time = cursor;
        for row in &rows {
            let open_time_ms = row[0].as_i64().unwrap_or(0);
            let open: f64 = row[1].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let high: f64 = row[2].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let low: f64 = row[3].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let close: f64 = row[4].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let volume: f64 = row[5].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let close_time_ms = row[6].as_i64().unwrap_or(open_time_ms);
            let Some(ts) = DateTime::<Utc>::from_timestamp_millis(open_time_ms) else {
                continue;
            };
            bars.push(CryptoBar {
                timestamp: ts.to_rfc3339_opts(SecondsFormat::Secs, true),
                open,
                high,
                low,
                close,
                volume,
            });
            last_close_time = last_close_time.max(close_time_ms);
        }
        if rows.len() < 1000 || last_close_time >= end_ms {
            break;
        }
        cursor = last_close_time + 1;
    }

    Ok(bars)
}

/// Minute-truncated timestamp key so bars from two different sources/formats align even
/// if their exact string formatting differs (seconds precision, trailing Z, etc.) -
/// assumes both are UTC and start with `YYYY-MM-DDTHH:MM`.
fn minute_key(timestamp: &str) -> &str {
    if timestamp.len() >= 16 {
        &timestamp[..16]
    } else {
        timestamp
    }
}

struct AlignedPoint {
    timestamp: String,
    alpaca_close: f64,
    alpaca_high: f64,
    alpaca_low: f64,
    binance_close: f64,
}

fn align_by_minute(alpaca_bars: &[CryptoBar], binance_bars: &[CryptoBar]) -> Vec<AlignedPoint> {
    let binance_by_minute: HashMap<&str, f64> = binance_bars
        .iter()
        .map(|b| (minute_key(&b.timestamp), b.close))
        .collect();
    alpaca_bars
        .iter()
        .filter_map(|a| {
            binance_by_minute
                .get(minute_key(&a.timestamp))
                .map(|&binance_close| AlignedPoint {
                    timestamp: a.timestamp.clone(),
                    alpaca_close: a.close,
                    alpaca_high: a.high,
                    alpaca_low: a.low,
                    binance_close,
                })
        })
        .collect()
}

fn backtest_symbol(
    symbol: &str,
    points: &[AlignedPoint],
    fees: &rust_autohedge::config::FeeConfig,
    trade_notional: f64,
    tp_pct: f64,
    sl_pct: f64,
) -> (Vec<Trade>, Vec<f64>) {
    let spread_bps_assumed = AssetClass::Crypto.assumed_spread_bps(symbol);
    let mut trades = Vec::new();
    let mut open: Option<(f64, f64, f64, f64, String)> = None;
    let mut cross_spreads = Vec::with_capacity(points.len());

    for (i, p) in points.iter().enumerate() {
        let cross_spread_bps = if p.binance_close > 0.0 {
            (p.alpaca_close - p.binance_close) / p.binance_close * 10_000.0
        } else {
            0.0
        };
        cross_spreads.push(cross_spread_bps);

        if let Some((entry_price, qty, tp, sl, ref entry_time)) = open {
            let hit_sl = p.alpaca_low <= sl;
            let hit_tp = p.alpaca_high >= tp;
            let exit = if hit_sl {
                Some((sl, "stop_loss"))
            } else if hit_tp {
                Some((tp, "take_profit"))
            } else {
                None
            };
            if let Some((exit_price, reason)) = exit {
                let fee_cost = entry_price * qty * (fees.round_trip_fee_bps / 10_000.0);
                let pnl = (exit_price - entry_price) * qty - fee_cost;
                trades.push(Trade {
                    symbol: symbol.to_string(),
                    entry_time: entry_time.clone(),
                    entry_price,
                    exit_time: p.timestamp.clone(),
                    exit_price,
                    qty,
                    exit_reason: reason,
                    pnl,
                });
                open = None;
            }
        }

        if open.is_some() || i < ZSCORE_WINDOW {
            continue;
        }

        // Alpaca is unusually cheap relative to Binance.US - bet it catches up.
        if cross_spread_bps > -ENTRY_THRESHOLD_BPS {
            continue;
        }

        let half_spread = p.alpaca_close * (spread_bps_assumed / 20_000.0);
        let entry_price = p.alpaca_close + half_spread;
        let qty = trade_notional / entry_price;
        let tp = entry_price * (1.0 + tp_pct / 100.0);
        let sl = entry_price * (1.0 - sl_pct / 100.0);
        open = Some((entry_price, qty, tp, sl, p.timestamp.clone()));
    }

    (trades, cross_spreads)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let lookback_days = days_from_args();
    let config = AppConfig::load();
    let alpaca_client = AlpacaClient::new(config.alpaca.clone(), config.history_limit);
    let http_client = reqwest::Client::new();

    let end = Utc::now();
    let start = end - ChronoDuration::days(lookback_days);
    let start_str = start.to_rfc3339();
    let end_str = end.to_rfc3339();
    let start_ms = start.timestamp_millis();
    let end_ms = end.timestamp_millis();

    println!(
        "Cross-exchange backtest (Alpaca vs Binance.US), {} symbols, {} days",
        config.symbols.len(),
        lookback_days
    );
    println!("entry_threshold_bps={} (Alpaca must lag Binance.US by this much to trigger a buy)", ENTRY_THRESHOLD_BPS);
    println!();

    println!("-- Alpaca --");
    let alpaca_bars = fetch_all_bars(
        &alpaca_client,
        AssetClass::Crypto,
        &config.symbols,
        TIMEFRAME,
        &start_str,
        &end_str,
    )
    .await;

    println!();
    println!("-- Binance.US --");
    let mut binance_bars: Vec<(String, Vec<CryptoBar>)> = Vec::new();
    for symbol in &config.symbols {
        let binance_symbol = to_binance_us_symbol(symbol);
        print!("Fetching {} ({}) ... ", symbol, binance_symbol);
        match fetch_binance_us_klines(&http_client, &binance_symbol, start_ms, end_ms).await {
            Ok(bars) => {
                println!("{} bars", bars.len());
                if !bars.is_empty() {
                    binance_bars.push((symbol.clone(), bars));
                }
            }
            Err(e) => println!("FAILED: {}", e),
        }
    }

    let binance_map: HashMap<&String, &Vec<CryptoBar>> =
        binance_bars.iter().map(|(s, b)| (s, b)).collect();

    println!();
    println!(
        "{:<10} {:>10} {:>12} {:>12} {:>8} {:>10} {:>12} {:>12}",
        "Symbol", "AlignedN", "AvgSpread", "MaxAbsSpr", "Trades", "WinRate%", "TotalPnL$", "AvgPnL$"
    );

    let mut all_trades = Vec::new();
    for (symbol, alpaca) in &alpaca_bars {
        let Some(binance) = binance_map.get(symbol) else {
            continue;
        };
        let points = align_by_minute(alpaca, binance);
        if points.len() < 200 {
            println!("{:<10} insufficient aligned data ({} points)", symbol, points.len());
            continue;
        }
        let (tp_pct, sl_pct) = config.get_symbol_params(symbol);
        let (trades, spreads) = backtest_symbol(
            symbol,
            &points,
            &config.fees,
            config.defaults.max_order_amount,
            tp_pct,
            sl_pct,
        );

        let avg_spread = spreads.iter().sum::<f64>() / spreads.len() as f64;
        let max_abs_spread = spreads.iter().cloned().fold(0.0_f64, |m, v| m.max(v.abs()));
        let n = trades.len();
        let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
        let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
        let win_rate = if n > 0 { wins as f64 / n as f64 * 100.0 } else { 0.0 };
        let avg_pnl = if n > 0 { total_pnl / n as f64 } else { 0.0 };

        println!(
            "{:<10} {:>10} {:>12.2} {:>12.2} {:>8} {:>10.1} {:>12.2} {:>12.4}",
            symbol, points.len(), avg_spread, max_abs_spread, n, win_rate, total_pnl, avg_pnl
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
    let dd = max_drawdown(&all_trades);

    println!();
    println!("===== Portfolio Summary =====");
    println!("Total trades:     {}", total_trades);
    println!("Overall win rate: {:.1}%", overall_win_rate);
    println!("Total PnL:        ${:.2}", total_pnl);
    println!(
        "Avg PnL/trade:    ${:.4}",
        if total_trades > 0 { total_pnl / total_trades as f64 } else { 0.0 }
    );
    println!("Max drawdown:     ${:.2}", dd);

    std::fs::create_dir_all("data").ok();
    std::fs::write(
        "data/cross_exchange_results.json",
        serde_json::to_string_pretty(&serde_json::json!({
            "lookback_days": lookback_days,
            "entry_threshold_bps": ENTRY_THRESHOLD_BPS,
            "portfolio": {
                "total_trades": total_trades,
                "overall_win_rate_pct": overall_win_rate,
                "total_pnl": total_pnl,
                "max_drawdown": dd,
            },
            "trades": all_trades,
        }))?,
    )?;
    println!();
    println!("Full results written to data/cross_exchange_results.json");

    Ok(())
}
