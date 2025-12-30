use async_trait::async_trait;
use serde_json::Value;

use crate::{bus::EventBus, data::store::MarketStore};

use super::types::{
    AccountSummary, ExchangeCapabilities, OrderAck, PlaceOrderRequest, Position,
};

pub type ExchangeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[async_trait]
pub trait TradingApi: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ExchangeCapabilities;

    async fn get_account(&self) -> ExchangeResult<AccountSummary>;
    async fn get_positions(&self) -> ExchangeResult<Vec<Position>>;
    async fn get_order(&self, order_id: &str) -> ExchangeResult<OrderAck>;
    async fn cancel_order(&self, order_id: &str) -> ExchangeResult<()>;
    async fn submit_order(&self, order: PlaceOrderRequest) -> ExchangeResult<OrderAck>;

    /// Optional helper for strategy warmup/backfill.
    async fn get_historical_bars(&self, _symbol: &str, _timeframe: &str) -> ExchangeResult<Value> {
        Ok(Value::Null)
    }
}

#[async_trait]
pub trait MarketDataStream: Send + Sync {
    async fn start(&self, store: MarketStore, symbols: Vec<String>, event_bus: EventBus) -> ExchangeResult<()>;
}
