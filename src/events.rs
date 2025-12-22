use serde_json::Value;

#[derive(Clone, Debug)]
pub enum MarketEvent {
    Quote {
        symbol: String,
        bid: f64,
        ask: f64,
        timestamp: String,
        original: Value,
    },
    Trade {
        symbol: String,
        price: f64,
        size: f64,
        timestamp: String,
        original: Value,
    },
    // We can add Bar later if needed
}

#[derive(Clone, Debug)]
pub struct AnalysisSignal {
    pub symbol: String,
    pub signal: String, // "buy", "sell", "no_trade"
    pub confidence: f64,
    pub thesis: String,
    pub market_context: String, // Snapshot of data used
}

#[derive(Clone, Debug)]
pub struct OrderRequest {
    pub symbol: String,
    pub action: String, // "buy", "sell"
    pub qty: f64,
    pub order_type: String, // "market", "limit"
    pub limit_price: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct ExecutionReport {
    pub symbol: String,
    pub order_id: String,
    pub status: String, // "filled", "new", "rejected"
    pub price: Option<f64>,
    pub qty: Option<f64>,
}

// Global Event Enum
#[derive(Clone, Debug)]
pub enum Event {
    Market(MarketEvent),
    Signal(AnalysisSignal),
    Order(OrderRequest),
    Execution(ExecutionReport),
}
