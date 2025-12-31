//! Binance Spot adapter (REST + WS minimal).

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use super::{
    traits::{ExchangeResult, TradingApi},
    types::{
        AccountSummary, ExchangeCapabilities, OrderAck, OrderType, PlaceOrderRequest, Position,
        Side, TimeInForce,
    },
};

use crate::config::BinanceConfig;

#[derive(Clone)]
pub struct BinanceExchange {
    client: Client,
    base_url: String,
    api_key: String,
    api_secret: String,
}

impl BinanceExchange {
    pub fn new(config: BinanceConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.base_url,
            api_key: config.api_key,
            api_secret: config.secret_key
        }
    }

    fn auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // Proper Binance signing requires HMAC SHA256 query signing.
        // Placeholder header for compile-time wiring.
        req.header("X-MBX-APIKEY", &self.api_key)
            .header("X-MBX-APISECRET", &self.api_secret)
    }
}

#[async_trait]
impl TradingApi for BinanceExchange {
    fn name(&self) -> &'static str { "binance" }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_notional_market_buy: true,
            supports_ws_quotes: true,
            supports_ws_trades: true,
            supports_news: false,
        }
    }

    async fn get_account(&self) -> ExchangeResult<AccountSummary> {
        Ok(AccountSummary { buying_power: None, cash: None, portfolio_value: None })
    }

    async fn get_positions(&self) -> ExchangeResult<Vec<Position>> {
        // Placeholder
        Ok(vec![])
    }

    async fn get_order(&self, _order_id: &str) -> ExchangeResult<OrderAck> {
        Err("Binance get_order not implemented".into())
    }

    async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> {
        Err("Binance cancel_order not implemented".into())
    }

    async fn submit_order(&self, order: PlaceOrderRequest) -> ExchangeResult<OrderAck> {
        // Minimal placeholder. Real Binance endpoint is POST /api/v3/order with signed query.
        let endpoint = format!("{}/api/v3/order", self.base_url);
        let _tif = match order.time_in_force { TimeInForce::Day => "DAY", TimeInForce::Gtc => "GTC" };
        let _side = match order.side { Side::Buy => "BUY", Side::Sell => "SELL" };
        let _type = match order.order_type { OrderType::Market => "MARKET", OrderType::Limit => "LIMIT" };

        let resp = self.auth_headers(self.client.post(&endpoint)).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Binance submit_order failed ({}): {}", status, text).into());
        }
        let raw: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Binance submit_order decode failed: {} (body: {})", e, text))?;

        let id = raw
            .get("orderId")
            .and_then(|v| v.as_i64())
            .map(|i| i.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let status = raw
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(OrderAck { id, status, raw })
    }

    async fn get_historical_bars(&self, _symbol: &str, _timeframe: &str) -> ExchangeResult<Value> {
        Ok(Value::Null)
    }
}
