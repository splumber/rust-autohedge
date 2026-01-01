use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    symbols::to_coinbase_product_id,
    traits::{ExchangeResult, TradingApi},
    types::{
        AccountSummary, ExchangeCapabilities, OrderAck, OrderType, PlaceOrderRequest, Position, Side,
        TimeInForce,
    },
};

use crate::config::CoinbaseConfig;

/// Coinbase Advanced Trade adapter.
///
/// NOTE: Proper Coinbase signing (CB-ACCESS-* headers) is required for live trading.
/// This implementation is a compile-safe scaffold and may need signing added before use.
#[derive(Clone)]
pub struct CoinbaseExchange {
    client: Client,
    base_url: String,
    api_key: String,
    api_secret: String,
}

impl CoinbaseExchange {
    pub fn new(config: CoinbaseConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.base_url,
            api_key: config.api_key,
            api_secret: config.secret_key
        }
    }

    fn auth_headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // Placeholder: real implementation must add timestamp + signature.
        req.header("CB-ACCESS-KEY", &self.api_key)
            .header("CB-ACCESS-SECRET", &self.api_secret)
    }
}

#[async_trait]
impl TradingApi for CoinbaseExchange {
    fn name(&self) -> &'static str {
        "coinbase"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        ExchangeCapabilities {
            supports_notional_market_buy: true,
            supports_ws_quotes: false,
            supports_ws_trades: true,
            supports_news: false,
        }
    }

    async fn get_account(&self) -> ExchangeResult<AccountSummary> {
        // Coinbase exposes balances per account.
        Ok(AccountSummary { buying_power: None, cash: None, portfolio_value: None })
    }

    async fn get_positions(&self) -> ExchangeResult<Vec<Position>> {
        // Placeholder
        Ok(vec![])
    }

    async fn get_order(&self, _order_id: &str) -> ExchangeResult<OrderAck> {
        Err("Coinbase get_order not implemented".into())
    }

    async fn cancel_order(&self, _order_id: &str) -> ExchangeResult<()> {
        Err("Coinbase cancel_order not implemented".into())
    }

    async fn cancel_all_orders(&self) -> ExchangeResult<()> {
        Err("Coinbase cancel_all_orders not implemented".into())
    }

    async fn submit_order(&self, order: PlaceOrderRequest) -> ExchangeResult<OrderAck> {
        let endpoint = format!("{}/api/v3/brokerage/orders", self.base_url);

        let side = match order.side { Side::Buy => "BUY", Side::Sell => "SELL" };
        let _tif = match order.time_in_force { TimeInForce::Day => "DAY", TimeInForce::Gtc => "GTC", TimeInForce::Ioc => "IOC" };

        let product_id = to_coinbase_product_id(&order.symbol);

        let body = match order.order_type {
            OrderType::Market => json!({
                "client_order_id": uuid::Uuid::new_v4().to_string(),
                "product_id": product_id,
                "side": side,
                "order_configuration": {
                    "market_market_ioc": {
                        "quote_size": order.notional.map(|n| format!("{:.2}", n)),
                        "base_size": order.qty.map(|q| q.to_string())
                    }
                }
            }),
            OrderType::Limit => json!({
                "client_order_id": uuid::Uuid::new_v4().to_string(),
                "product_id": product_id,
                "side": side,
                "order_configuration": {
                    "limit_limit_gtc": {
                        "base_size": order.qty.map(|q| q.to_string()),
                        "limit_price": "0",
                        "post_only": false
                    }
                }
            }),
        };

        let resp = self.auth_headers(self.client.post(&endpoint)).json(&body).send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Coinbase submit_order failed ({}): {}", status, text).into());
        }

        let raw: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Coinbase submit_order decode failed: {} (body: {})", e, text))?;

        let id = raw
            .pointer("/order_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let status_s = raw
            .pointer("/success")
            .and_then(|v| v.as_bool())
            .map(|b| if b { "accepted" } else { "rejected" })
            .unwrap_or("unknown")
            .to_string();

        Ok(OrderAck { id, status: status_s, raw })
    }

    async fn get_historical_bars(&self, _symbol: &str, _timeframe: &str) -> ExchangeResult<Value> {
        Ok(Value::Null)
    }
}
