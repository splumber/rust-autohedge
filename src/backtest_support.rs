//! Shared infrastructure for backtest research binaries (`src/bin/backtest.rs`,
//! `src/bin/pairs.rs`, ...): historical data fetching, the common trade record, and
//! portfolio-level stats. Kept in the lib crate so multiple research binaries can reuse it
//! without duplicating fetch/PnL logic that would otherwise drift between copies.
//!
//! ## Crypto vs stocks
//! The actual entry-decision logic (`services::strategy::evaluate_*_signal`,
//! `services::indicators::*`) is already asset-class-agnostic - it operates purely on
//! bid/ask and price-series numbers, with no crypto- or equity-specific assumptions
//! anywhere in it. Everything that genuinely differs between the two asset classes lives
//! here instead: which Alpaca endpoint to fetch bars from, and what fee/spread costs are
//! realistic to assume (crypto's per-trade taker fees and wider spreads vs. stocks'
//! commission-free trading and typically tighter spreads on liquid names). `AssetClass`
//! is the single switch that selects between them; nothing in the strategy layer needs to
//! know which one is active.

use crate::data::alpaca::{AlpacaClient, CryptoBar};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Crypto,
    Stocks,
}

impl AssetClass {
    /// Parses a `--asset-class` CLI value; defaults to `Crypto` for anything unrecognized
    /// so existing crypto-only invocations keep working without needing the flag.
    pub fn from_arg(s: &str) -> Self {
        match s {
            "stock" | "stocks" | "equity" | "equities" => AssetClass::Stocks,
            _ => AssetClass::Crypto,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AssetClass::Crypto => "crypto",
            AssetClass::Stocks => "stocks",
        }
    }

    /// Alpaca's `asset_class` query value for `GET /v2/assets`.
    pub fn alpaca_asset_class_param(&self) -> &'static str {
        match self {
            AssetClass::Crypto => "crypto",
            AssetClass::Stocks => "us_equity",
        }
    }

    /// Conservative default round-trip fee assumption in bps.
    ///
    /// Crypto: mirrors Alpaca's worst-tier crypto taker fee (see `config::FeeConfig`'s
    /// doc comment) - real value depends on 30-day trailing volume tier.
    ///
    /// Stocks: Alpaca stock trading is commission-free. The only real cost is SEC/FINRA
    /// regulatory fees charged on sells only (SEC fee ~$0.0000278/share notional, FINRA
    /// TAF a fraction of a cent per share) - a few hundredths of a bp for retail-size
    /// trades, effectively negligible. 2.0 bps here is a deliberately conservative
    /// round-up buffer, not a measured cost - the honest number is closer to 0.
    pub fn default_round_trip_fee_bps(&self) -> f64 {
        match self {
            AssetClass::Crypto => 50.0,
            AssetClass::Stocks => 2.0,
        }
    }

    /// Rough, labeled-as-estimated liquidity-tier spread in bps - not measured from real
    /// data, since Alpaca's historical bars (both asset classes) carry no bid/ask.
    pub fn assumed_spread_bps(&self, symbol: &str) -> f64 {
        match self {
            AssetClass::Crypto => assumed_crypto_spread_bps(symbol),
            AssetClass::Stocks => assumed_stock_spread_bps(symbol),
        }
    }

    /// Fetches one symbol's historical bars via the endpoint appropriate to this asset
    /// class. Both return the same `CryptoBar` shape (plain OHLCV) - the name is a
    /// historical artifact of crypto support landing first, not a claim about what kind
    /// of instrument the bars describe.
    pub async fn fetch_bars(
        &self,
        client: &AlpacaClient,
        symbol: &str,
        timeframe: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<CryptoBar>, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            AssetClass::Crypto => client.get_crypto_bars_range(symbol, timeframe, start, end).await,
            AssetClass::Stocks => client.get_stock_bars_range(symbol, timeframe, start, end).await,
        }
    }
}

fn assumed_crypto_spread_bps(symbol: &str) -> f64 {
    match symbol {
        "BTC/USD" | "ETH/USD" => 8.0, // most liquid pairs Alpaca offers
        "SOL/USD" | "LTC/USD" | "XRP/USD" | "BCH/USD" => 15.0,
        "DOT/USD" | "AVAX/USD" | "LINK/USD" | "UNI/USD" | "AAVE/USD" => 20.0,
        "DOGE/USD" => 25.0,
        "SHIB/USD" => 40.0,
        "PEPE/USD" => 45.0,
        _ => 30.0, // unlisted/unknown - assume less liquid than a known major
    }
}

/// A curated, deliberately liquid stock universe for backtest research - not exhaustive
/// (Alpaca offers thousands of tradable US equities), sized to keep pairwise cointegration
/// testing tractable (36 symbols here, matching the crypto universe's size, so
/// C(36,2)=1260 ordered pairs stays a reasonable amount of work). Picked for genuine
/// liquidity across sectors, not for any expected trading edge - the point is to test
/// whether the same signals behave differently in a near-zero-fee, tight-spread
/// environment, not to hand-pick winners.
pub fn liquid_stock_universe() -> Vec<String> {
    [
        // Mega-cap tech
        "AAPL", "MSFT", "GOOGL", "AMZN", "META", "NVDA", "TSLA", "AMD", "AVGO", "CRM",
        // Finance
        "JPM", "BAC", "GS", "MS", "V", "MA",
        // Healthcare
        "JNJ", "UNH", "PFE", "ABBV",
        // Consumer
        "WMT", "COST", "HD", "MCD", "NKE", "DIS", "PG", "KO", "PEP",
        // Industrial / energy
        "XOM", "CVX", "CAT", "BA",
        // Broad index ETFs
        "SPY", "QQQ", "DIA", "IWM",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn assumed_stock_spread_bps(symbol: &str) -> f64 {
    match symbol {
        // Mega-cap tech / most liquid single names on the market - typically penny-wide
        // or tighter, often sub-1bp in practice.
        "AAPL" | "MSFT" | "NVDA" | "AMZN" | "GOOGL" | "META" | "TSLA" => 1.0,
        // Highly liquid large caps.
        "AMD" | "AVGO" | "JPM" | "V" | "MA" | "WMT" | "COST" | "HD" | "XOM" | "CVX" => 2.0,
        // The broad index ETFs - among the most liquid instruments that exist.
        "SPY" | "QQQ" | "DIA" | "IWM" => 0.5,
        _ => 5.0, // unlisted/unknown large-ish cap - still tighter than most crypto by default
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Trade {
    pub symbol: String,
    pub entry_time: String,
    pub entry_price: f64,
    pub exit_time: String,
    pub exit_price: f64,
    pub qty: f64,
    pub exit_reason: &'static str,
    pub pnl: f64,
}

pub fn max_drawdown(trades: &[Trade]) -> f64 {
    let mut cumulative = 0.0;
    let mut peak = 0.0;
    let mut max_dd = 0.0;
    for t in trades {
        cumulative += t.pnl;
        if cumulative > peak {
            peak = cumulative;
        }
        let dd = peak - cumulative;
        if dd > max_dd {
            max_dd = dd;
        }
    }
    max_dd
}

/// Fetches historical bars for every symbol in `symbols` via the given asset class's
/// endpoint, skipping (with a logged failure) any symbol whose fetch errors or returns no
/// data.
pub async fn fetch_all_bars(
    client: &AlpacaClient,
    asset_class: AssetClass,
    symbols: &[String],
    timeframe: &str,
    start_str: &str,
    end_str: &str,
) -> Vec<(String, Vec<CryptoBar>)> {
    let mut bars_by_symbol = Vec::new();
    for symbol in symbols {
        print!("Fetching {} ... ", symbol);
        match asset_class.fetch_bars(client, symbol, timeframe, start_str, end_str).await {
            Ok(bars) => {
                println!("{} bars", bars.len());
                if !bars.is_empty() {
                    bars_by_symbol.push((symbol.clone(), bars));
                }
            }
            Err(e) => println!("FAILED: {}", e),
        }
    }
    bars_by_symbol
}
