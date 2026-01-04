//! Custom error types for the trading system
//!
//! Provides structured, typed errors instead of generic Box<dyn Error>

use thiserror::Error;

/// Top-level trading system errors
#[derive(Error, Debug)]
pub enum TradingError {
    #[error("Insufficient balance for {symbol}: requested {requested}, available {available}")]
    InsufficientBalance {
        symbol: String,
        requested: f64,
        available: f64,
    },

    #[error("Rate limited for {symbol} (cooldown: {cooldown_ms}ms)")]
    RateLimited { symbol: String, cooldown_ms: u64 },

    #[error("Position not found: {symbol}")]
    PositionNotFound { symbol: String },

    #[error("Invalid quantity {qty} for {symbol}")]
    InvalidQuantity { symbol: String, qty: f64 },

    #[error("Invalid price {price} for {symbol}")]
    InvalidPrice { symbol: String, price: f64 },

    #[error("Order not found: {order_id}")]
    OrderNotFound { order_id: String },

    #[error("Pending order already exists for {symbol}")]
    PendingOrderExists { symbol: String },

    #[error("Exchange API error: {0}")]
    Exchange(#[from] ExchangeError),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Parse error: {0}")]
    Parse(String),
}

/// Exchange-specific errors
#[derive(Error, Debug)]
pub enum ExchangeError {
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Order rejected: {reason}")]
    OrderRejected { reason: String },

    #[error("Authentication failed: {reason}")]
    AuthFailed { reason: String },

    #[error("Invalid symbol: {symbol}")]
    InvalidSymbol { symbol: String },

    #[error("Market closed for {symbol}")]
    MarketClosed { symbol: String },

    #[error("Order size too small: {symbol} (min: {min})")]
    OrderTooSmall { symbol: String, min: f64 },

    #[error("Order size too large: {symbol} (max: {max})")]
    OrderTooLarge { symbol: String, max: f64 },

    #[error("WebSocket error: {0}")]
    WebSocket(String),

    #[error("Deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),
}

/// Position tracker errors
#[derive(Error, Debug)]
pub enum TrackerError {
    #[error("Position not found: {symbol}")]
    PositionNotFound { symbol: String },

    #[error("Order not found: {order_id}")]
    OrderNotFound { order_id: String },

    #[error("Position already exists: {symbol}")]
    PositionExists { symbol: String },
}

/// Strategy-related errors
#[derive(Error, Debug)]
pub enum StrategyError {
    #[error("Invalid quote: bid={bid}, ask={ask}")]
    InvalidQuote { bid: f64, ask: f64 },

    #[error("Spread too wide for {symbol}: {spread_bps} bps (max: {max_spread_bps})")]
    SpreadTooWide {
        symbol: String,
        spread_bps: f64,
        max_spread_bps: f64,
    },

    #[error("Insufficient edge for {symbol}: {edge_bps} bps (min: {min_edge_bps})")]
    InsufficientEdge {
        symbol: String,
        edge_bps: f64,
        min_edge_bps: f64,
    },

    #[error("Not enough data for {symbol}: have {count}, need {required}")]
    InsufficientData {
        symbol: String,
        count: usize,
        required: usize,
    },
}

/// Conversion helpers for legacy code
impl From<Box<dyn std::error::Error + Send + Sync>> for TradingError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        TradingError::Parse(err.to_string())
    }
}

impl From<String> for TradingError {
    fn from(err: String) -> Self {
        TradingError::Config(err)
    }
}

impl From<&str> for TradingError {
    fn from(err: &str) -> Self {
        TradingError::Config(err.to_string())
    }
}

/// Helper to check if an error is insufficient balance
pub fn is_insufficient_balance_error(error: &str) -> bool {
    (error.contains("403") && error.contains("40310000")) || error.contains("insufficient balance")
}

/// Helper to parse insufficient balance error details
pub fn parse_insufficient_balance(error: &str) -> Option<(String, f64, f64)> {
    // Parse error message to extract symbol, requested, and available amounts
    // Format: "insufficient balance for SYMBOL (requested: X, available: Y)"

    if !is_insufficient_balance_error(error) {
        return None;
    }

    // This is a simplified parser - in production, use regex or proper JSON parsing
    let symbol = error
        .split("balance for ")
        .nth(1)?
        .split(" (")
        .next()?
        .trim()
        .to_string();

    // Extract numbers from error message
    // This would need more robust parsing in production

    Some((symbol, 0.0, 0.0))
}
