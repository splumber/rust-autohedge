//! Approximated market-making backtest: instead of crossing the spread on a directional
//! signal (every other strategy in this codebase), this quotes BOTH sides around each
//! bar's reference price and profits from the spread itself when both sides fill, skewing
//! quotes to manage inventory risk as it accumulates.
//!
//! ## IMPORTANT: not realistically executable via a retail Alpaca account
//! Real market making requires exchange queue priority and often maker rebates that only
//! come from exchange membership or direct market access - a retail broker like Alpaca
//! routes your limit orders for execution but gives no such privilege. This backtest
//! approximates fills from 1-minute bar high/low crossing your quote levels, which
//! substantially overstates how often you'd actually get filled ahead of faster,
//! better-positioned market participants in reality (an institutional market maker reacts
//! to quote changes in microseconds, not once per bar, and gets priority at the same price
//! level via exchange membership). **Treat this as an educational illustration of the
//! mechanics, not a deployable strategy candidate** - see `docs/STRATEGIES.md`.
//!
//! Usage: `cargo run --bin market_maker --release -- --asset-class stocks --days 60
//! [--offset-days N]`

use chrono::{Duration as ChronoDuration, Utc};
use rust_autohedge::backtest_support::{fetch_all_bars, liquid_stock_universe, AssetClass};
use rust_autohedge::config::{AppConfig, FeeConfig};
use rust_autohedge::data::alpaca::AlpacaClient;
use serde::Serialize;

const DEFAULT_LOOKBACK_DAYS: i64 = 60;
const TIMEFRAME: &str = "1Min";
/// Fixed $ notional per individual fill (not per symbol total) - keeps inventory changes
/// bounded and comparable across symbols regardless of share price.
const FILL_NOTIONAL: f64 = 20.0;

fn i64_arg(flag: &str, default: i64) -> i64 {
    std::env::args()
        .skip_while(|a| a != flag)
        .nth(1)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(default)
}

fn asset_class_from_args() -> AssetClass {
    std::env::args()
        .skip_while(|a| a != "--asset-class")
        .nth(1)
        .map(|s| AssetClass::from_arg(&s))
        .unwrap_or(AssetClass::Crypto)
}

#[derive(Serialize)]
struct SymbolResult {
    symbol: String,
    fills: usize,
    buy_fills: usize,
    sell_fills: usize,
    realized_pnl: f64,
    ending_inventory_value: f64,
    inventory_mtm_pnl: f64,
    total_pnl: f64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::WARN)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let asset_class = asset_class_from_args();
    let lookback_days = i64_arg("--days", DEFAULT_LOOKBACK_DAYS);
    let offset_days = i64_arg("--offset-days", 0);

    let config = AppConfig::load();
    let mm_cfg = config.strategies.market_maker.clone();
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

    let end = Utc::now() - ChronoDuration::days(offset_days);
    let start = end - ChronoDuration::days(lookback_days);

    println!("===== Approximated Market-Making Backtest =====");
    println!(
        "IMPORTANT: illustrative only - not realistically executable via a retail Alpaca \
         account (no exchange queue priority or maker rebates). See module doc / \
         docs/STRATEGIES.md before drawing any deployment conclusion from this."
    );
    println!(
        "quote_offset_bps={:.1} inventory_limit=${:.0} skew_factor={:.2} fill_notional=${:.0}",
        mm_cfg.quote_offset_bps, mm_cfg.inventory_limit_usd, mm_cfg.skew_factor, FILL_NOTIONAL
    );
    println!();

    let bars_by_symbol =
        fetch_all_bars(&client, asset_class, &symbols, TIMEFRAME, &start.to_rfc3339(), &end.to_rfc3339()).await;

    let mut results = Vec::new();
    let mut total_pnl = 0.0_f64;

    for (symbol, bars) in &bars_by_symbol {
        // inventory_qty: net shares/units held (positive = long). inventory_cost_basis:
        // signed $ notional paid/received for the current net inventory - used to compute
        // unrealized mark-to-market PnL as (qty * last_price) - cost_basis.
        let mut inventory_qty = 0.0_f64;
        let mut inventory_cost_basis = 0.0_f64;
        let mut realized_pnl = 0.0_f64;
        let mut buy_fills = 0usize;
        let mut sell_fills = 0usize;
        let half_spread_bps = asset_class.assumed_spread_bps(symbol) / 2.0;
        // Approximate half the round-trip fee per single fill (a full round trip is two fills).
        let fee_frac = fees.round_trip_fee_bps / 10_000.0 / 2.0;

        let mut prev_close: Option<f64> = None;
        for bar in bars {
            let reference = prev_close.unwrap_or(bar.open);
            if reference <= 0.0 {
                prev_close = Some(bar.close);
                continue;
            }

            // Skew both quotes based on current inventory: long inventory pushes quotes
            // down (encourage selling, discourage more buying); short inventory pushes up.
            let inventory_ratio = (inventory_cost_basis / mm_cfg.inventory_limit_usd).clamp(-1.0, 1.0);
            let skew_bps = inventory_ratio * mm_cfg.skew_factor * mm_cfg.quote_offset_bps;

            let bid_quote = reference * (1.0 - (mm_cfg.quote_offset_bps + half_spread_bps + skew_bps) / 10_000.0);
            let ask_quote = reference * (1.0 + (mm_cfg.quote_offset_bps + half_spread_bps - skew_bps) / 10_000.0);

            let can_buy = inventory_cost_basis < mm_cfg.inventory_limit_usd;
            let can_sell = inventory_cost_basis > -mm_cfg.inventory_limit_usd;

            if can_buy && bar.low <= bid_quote {
                let qty = FILL_NOTIONAL / bid_quote;
                let cost = FILL_NOTIONAL * (1.0 + fee_frac);
                inventory_qty += qty;
                inventory_cost_basis += FILL_NOTIONAL;
                realized_pnl -= cost;
                buy_fills += 1;
            }
            if can_sell && bar.high >= ask_quote {
                let qty = FILL_NOTIONAL / ask_quote;
                let proceeds = FILL_NOTIONAL * (1.0 - fee_frac);
                inventory_qty -= qty;
                inventory_cost_basis -= FILL_NOTIONAL;
                realized_pnl += proceeds;
                sell_fills += 1;
            }

            prev_close = Some(bar.close);
        }

        let last_price = bars.last().map(|b| b.close).unwrap_or(0.0);
        let ending_inventory_value = inventory_qty * last_price;
        let inventory_mtm_pnl = ending_inventory_value - inventory_cost_basis;
        let total = realized_pnl + inventory_mtm_pnl;
        total_pnl += total;

        results.push(SymbolResult {
            symbol: symbol.clone(),
            fills: buy_fills + sell_fills,
            buy_fills,
            sell_fills,
            realized_pnl,
            ending_inventory_value,
            inventory_mtm_pnl,
            total_pnl: total,
        });
    }

    results.sort_by(|a, b| b.total_pnl.partial_cmp(&a.total_pnl).unwrap());
    println!(
        "{:<8} {:>8} {:>8} {:>8} {:>14} {:>14} {:>14}",
        "Symbol", "Fills", "Buys", "Sells", "RealizedPnL", "InvMtmPnL", "TotalPnL"
    );
    for r in &results {
        println!(
            "{:<8} {:>8} {:>8} {:>8} {:>14.2} {:>14.2} {:>14.2}",
            r.symbol, r.fills, r.buy_fills, r.sell_fills, r.realized_pnl, r.inventory_mtm_pnl, r.total_pnl
        );
    }
    println!();
    println!("Total PnL across {} symbols: ${:.2}", results.len(), total_pnl);
    println!();
    println!(
        "Reminder: approximated/illustrative only - see module doc for why true market \
         making isn't realistically executable via a retail Alpaca account."
    );

    std::fs::create_dir_all("data").ok();
    let out_path = format!(
        "data/market_maker_{}_{}d_offset{}.json",
        asset_class.label(), lookback_days, offset_days
    );
    std::fs::write(
        &out_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "asset_class": asset_class.label(),
            "lookback_days": lookback_days,
            "total_pnl": total_pnl,
            "results": results,
        }))?,
    )?;
    println!("Full results written to {}", out_path);

    Ok(())
}
