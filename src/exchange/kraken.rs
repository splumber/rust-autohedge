use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::env;

use super::{
    symbols::to_kraken_pair,
    traits::{ExchangeResult, TradingApi},
    types::{AccountSummary, ExchangeCapabilities, OrderAck, PlaceOrderRequest, Position},
};

/// Kraken Spot adapter.
///
/// NOTE: Proper Kraken authentication (API-Key + API-Sign) is required for private endpoints.
/// This implementation is a compile-safe scaffold.
#[derive(Clone)]
pub struct KrakenExchange {
    client: Client,
    base_url: String,
    api_key: String,
    api_secret: String,
}

impl KrakenExchange {
    pub fn new() -> Self {
        let base_url = env::var("KRAKEN_API_BASE_URL").unwrap_or_else(|_| "https://api.kraken.com".to_string());
        let api_key = env::var("KRAKEN_API_KEY").unwrap_or_default();
        let api_secret = env::var("KRAKEN_API_SECRET").unwrap_or_default();
        Self { client: Client::new(), base_url, api_key, api_secret }
    }

    fn auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // Placeholder: real implementation must add Kraken API-Sign.
        req.header("API-Key", &self.api_key)
            .header("API-Secret", &self.api_secret)
    }
}

#[async_trait]
impl TradingApi for KrakenExchange {
    fn name(&self) -> &'static str { "kraken" }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_notional_market_buy: false,
            supports_ws_quotes: true,
            supports_ws_trades: true,
            supports_news: false,
        }
    }

    async fn get_account(&self) -> ExchangeResult<AccountSummary> {
        Ok(AccountSummary { buying_power: None, cash: None, portfolio_value: None })
    }

    async fn get_positions(&self) -> ExchangeResult<Vec<Position>> {
        Ok(vec![])
    }

    async fn submit_order(&self, order: PlaceOrderRequest) -> ExchangeResult<OrderAck> {
        // Kraken private endpoint: /0/private/AddOrder. Requires nonce + signature.
        // We keep a stub request that returns an error if not configured.
        let _pair = to_kraken_pair(&order.symbol);

        let endpoint = format!("{}/0/private/AddOrder", self.base_url);
        let resp = self.auth_headers(self.client.post(&endpoint)).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Kraken submit_order failed ({}): {}", status, text).into());
        }
        let raw: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Kraken submit_order decode failed: {} (body: {})", e, text))?;

        Ok(OrderAck { id: "unknown".to_string(), status: "unknown".to_string(), raw })
    }

    async fn get_historical_bars(&self, _symbol: &str, _timeframe: &str) -> ExchangeResult<Value> {
        Ok(Value::Null)
    }
}

