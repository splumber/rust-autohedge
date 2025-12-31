use async_trait::async_trait;
use serde_json::Value;

use crate::data::alpaca::{AlpacaClient, OrderRequest as AlpacaOrderRequest};

use super::{
    traits::{ExchangeResult, TradingApi},
    types::{
        AccountSummary, ExchangeCapabilities, OrderAck, OrderType, PlaceOrderRequest, Position,
        Side, TimeInForce,
    },
};

#[derive(Clone)]
pub struct AlpacaExchange {
    inner: AlpacaClient,
    trading_mode: String,
}

impl AlpacaExchange {
    pub fn new(inner: AlpacaClient, trading_mode: String) -> Self {
        Self { inner, trading_mode }
    }

    pub fn market_store(&self) -> crate::data::store::MarketStore {
        self.inner.market_store.clone()
    }
}

#[async_trait]
impl TradingApi for AlpacaExchange {
    fn name(&self) -> &'static str {
        "alpaca"
    }

    fn capabilities(&self) -> ExchangeCapabilities {
        // Alpaca crypto supports notional market buy in /v2/orders.
        ExchangeCapabilities {
            supports_notional_market_buy: self.trading_mode.eq_ignore_ascii_case("crypto"),
            supports_ws_quotes: true,
            supports_ws_trades: true,
            supports_news: true,
        }
    }

    async fn get_account(&self) -> ExchangeResult<AccountSummary> {
        let a = self.inner.get_account().await?;
        Ok(AccountSummary {
            buying_power: a.buying_power.parse().ok(),
            cash: a.cash.parse().ok(),
            portfolio_value: a.portfolio_value.parse().ok(),
        })
    }

    async fn get_positions(&self) -> ExchangeResult<Vec<Position>> {
        let vals = self.inner.get_positions().await?;
        let mut out = Vec::with_capacity(vals.len());
        for v in vals {
            let symbol = v.get("symbol").and_then(|x| x.as_str()).unwrap_or_default().to_string();
            let qty = v
                .get("qty")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v.get("qty").and_then(|x| x.as_f64()))
                .unwrap_or(0.0);
            let avg_entry_price = v
                .get("avg_entry_price")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v.get("avg_entry_price").and_then(|x| x.as_f64()));
            out.push(Position {
                symbol,
                qty,
                avg_entry_price,
            });
        }
        Ok(out)
    }

    async fn get_order(&self, order_id: &str) -> ExchangeResult<OrderAck> {
        let raw = self.inner.get_order(order_id).await?;
        let id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let status = raw
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(OrderAck { id, status, raw })
    }

    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()> {
        self.inner.cancel_order(order_id).await?;
        Ok(())
    }

    async fn cancel_all_orders(&self) -> ExchangeResult<()> {
        self.inner.cancel_all_orders().await?;
        Ok(())
    }

    async fn submit_order(&self, order: PlaceOrderRequest) -> ExchangeResult<OrderAck> {
        let side = match order.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };

        let type_ = match order.order_type {
            OrderType::Market => "market",
            OrderType::Limit => "limit",
        };

        let time_in_force = match order.time_in_force {
            TimeInForce::Day => "day",
            TimeInForce::Gtc => "gtc",
        };

        let api_req = AlpacaOrderRequest {
            symbol: order.symbol,
            qty: order.qty.map(|q| q.to_string()),
            notional: order.notional.map(|n| n.to_string()),
            side: side.to_string(),
            type_: type_.to_string(),
            time_in_force: time_in_force.to_string(),
            limit_price: order.limit_price.map(|p| p.to_string()),
        };

        let raw: Value = self.inner.submit_order(api_req, &self.trading_mode).await?;
        let id = raw
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let status = raw
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(OrderAck { id, status, raw })
    }

    async fn get_historical_bars(&self, symbol: &str, timeframe: &str) -> ExchangeResult<Value> {
        if self.trading_mode.eq_ignore_ascii_case("crypto") {
            Ok(self.inner.get_crypto_bars(symbol, timeframe).await?)
        } else {
            Ok(self.inner.get_historical_bars(symbol, timeframe).await?)
        }
    }
}
