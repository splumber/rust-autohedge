/// Simple symbol normalization helpers.
///
/// Canonical symbol (used internally):
/// - crypto: "BASE/USD" like "BTC/USD" (matches existing .env values)
///
/// Exchange mappings:
/// - Coinbase: "BTC-USD"
/// - Kraken:  "XBT/USD" (Kraken prefers XBT for BTC)

pub fn to_coinbase_product_id(canonical: &str) -> String {
    canonical.replace('/', "-")
}

pub fn to_kraken_pair(canonical: &str) -> String {
    // Basic mapping for BTC
    let s = canonical.replace("BTC/", "XBT/");
    s
}

#[allow(dead_code)]
pub fn to_binance_stream_symbol(canonical: &str) -> String {
    // Binance spot commonly uses e.g. BTCUSDT; for USD-quoted pairs keep BTCUSD.
    canonical.replace('/', "").to_lowercase()
}
