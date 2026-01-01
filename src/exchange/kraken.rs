use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use super::{
    symbols::to_kraken_pair,
    traits::{ExchangeResult, TradingApi},
    types::{AccountSummary, ExchangeCapabilities, OrderAck, PlaceOrderRequest, Position},
};

use crate::config::KrakenConfig;

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
    pub fn new(config: KrakenConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.base_url,
            api_key: config.api_key,
            api_secret: config.secret_key,
        }
    }

    fn auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // Placeholder: real implementation must add Kraken API-Sign.
        req.header("API-Key", &self.api_key)
            .header("API-Secret", &self.api_secret)
    }
}

#[async_trait]
impl TradingApi for KrakenExchange {
    fn name(&self) -> &'static str {
        "kraken"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_notional_market_buy: false,
            supports_ws_quotes: true,
            supports_ws_trades: true,
            supports_news: false,
        }
    }

    async fn get_account(&self) -> ExchangeResult<AccountSummary> {
        Ok(AccountSummary {
            buying_power: None,
            cash: None,
            portfolio_value: None,
        })
    }

    async fn get_positions(&self) -> ExchangeResult<Vec<Position>> {
        // Placeholder
        Ok(vec![])
    }

    async fn get_order(&self, _order_id: &str) -> ExchangeResult<OrderAck> {
        Err("Kraken get_order not implemented".into())
    }

    async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> {
        Err("Kraken cancel_order not implemented".into())
    }

    async fn cancel_all_orders(&self) -> ExchangeResult<()> {
        Err("Kraken cancel_all_orders not implemented".into())
    }

    async fn submit_order(&self, order: PlaceOrderRequest) -> ExchangeResult<OrderAck> {
        // Kraken private endpoint: /0/private/AddOrder. Requires nonce + signature.
        // We keep a stub request that returns an error if not configured.
        let _pair = to_kraken_pair(&order.symbol);

        let endpoint = format!("{}/0/private/AddOrder", self.base_url);
        let resp = self
            .auth_headers(self.client.post(&endpoint))
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Kraken submit_order failed ({}): {}", status, text).into());
        }
        let raw: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Kraken submit_order decode failed: {} (body: {})", e, text))?;

        Ok(OrderAck {
            id: "unknown".to_string(),
            status: "unknown".to_string(),
            raw,
        })
    }

    async fn get_historical_bars(&self, _symbol: &str, _timeframe: &str) -> ExchangeResult<Value> {
        Ok(Value::Null)
    }
}
