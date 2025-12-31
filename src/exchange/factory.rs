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
    let exchange = &config.exchange;

    match exchange.to_lowercase().as_str() {
        "alpaca" => {
            let alpaca_client = AlpacaClient::new(config.alpaca.clone(), config.history_limit);
            let alpaca = AlpacaExchange::new(alpaca_client.clone(), config.trading_mode.clone());
            let store = Some(alpaca.market_store());
            (Arc::new(alpaca), store)
        }
        "binance" => {
            let config = config.binance.clone().expect("Binance config missing");
            let ex = BinanceExchange::new(config);
            (Arc::new(ex), None)
        }
        "coinbase" => {
            let config = config.coinbase.clone().expect("Coinbase config missing");
            let ex = CoinbaseExchange::new(config);
            (Arc::new(ex), None)
        }
        "kraken" => {
            let config = config.kraken.clone().expect("Kraken config missing");
            let ex = KrakenExchange::new(config);
            (Arc::new(ex), None)
        }
        other => {
            panic!("Unknown EXCHANGE='{}' (expected alpaca|binance|coinbase|kraken)", other)
        }
    }
}
