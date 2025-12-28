use std::sync::Arc;

use crate::{
    config::AppConfig,
    data::alpaca::AlpacaClient,
};

use super::{
    alpaca::AlpacaExchange,
    binance::BinanceExchange,
    coinbase::CoinbaseExchange,
    kraken::KrakenExchange,
    traits::TradingApi,
};

pub fn build_exchange(config: &AppConfig) -> (Arc<dyn TradingApi>, Option<crate::data::store::MarketStore>) {
    let exchange = std::env::var("EXCHANGE").unwrap_or_else(|_| "alpaca".to_string());

    match exchange.to_lowercase().as_str() {
        "alpaca" => {
            let alpaca_client = AlpacaClient::new(config.history_limit);
            let alpaca = AlpacaExchange::new(alpaca_client.clone(), config.trading_mode.clone());
            let store = Some(alpaca.market_store());
            (Arc::new(alpaca), store)
        }
        "binance" => {
            let ex = BinanceExchange::new();
            (Arc::new(ex), None)
        }
        "coinbase" => {
            let ex = CoinbaseExchange::new();
            (Arc::new(ex), None)
        }
        "kraken" => {
            let ex = KrakenExchange::new();
            (Arc::new(ex), None)
        }
        other => {
            panic!("Unknown EXCHANGE='{}' (expected alpaca|binance|coinbase|kraken)", other)
        }
    }
}
