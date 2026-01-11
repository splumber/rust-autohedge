use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountSummary {
    pub buying_power: Option<f64>,
    pub cash: Option<f64>,
    pub portfolio_value: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub qty: f64,
    pub avg_entry_price: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeInForce {
    Day,
    Gtc,
    Ioc, // Immediate Or Cancel - for crypto limit orders
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaceOrderRequest {
    pub symbol: String,
    pub side: Side,
    pub order_type: OrderType,
    /// Quantity in base units. If notional is set, qty may be None.
    pub qty: Option<f64>,
    /// Notional in quote currency. If qty is set, notional may be None.
    pub notional: Option<f64>,
    pub limit_price: Option<f64>,
    pub time_in_force: TimeInForce,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderAck {
    pub id: String,
    pub status: String,
    pub raw: Value,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct NormalizedQuote {
    pub symbol: String,
    pub bid: f64,
    pub ask: f64,
    pub timestamp: String,
    pub raw: Value,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct NormalizedTrade {
    pub symbol: String,
    pub price: f64,
    pub size: f64,
    pub timestamp: String,
    pub raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExchangeCapabilities {
    pub supports_notional_market_buy: bool,
    pub supports_ws_quotes: bool,
    pub supports_ws_trades: bool,
    pub supports_news: bool,
}
