//! Application-wide constants and magic numbers
//!
//! This module centralizes all hardcoded values to improve maintainability
//! and make the codebase easier to tune.

use std::time::Duration;

/// Position monitoring constants
pub mod position_monitor {
    use super::*;

    /// Minimum difference to consider two float quantities different (0.0001%)
    /// Used for comparing tracked vs actual quantities
    pub const QTY_EPSILON: f64 = 0.000001;

    /// How often to check pending orders (avoids API spam)
    pub const ORDER_CHECK_INTERVAL: Duration = Duration::from_secs(2);

    /// Maximum retries for failed order placement
    pub const MAX_ORDER_RETRIES: u32 = 3;

    /// Time to wait between retries (exponential backoff base)
    pub const RETRY_BASE_DELAY_MS: u64 = 100;
}

/// Trading and exchange constants
pub mod trading {
    /// Alpaca's insufficient balance error code
    pub const ALPACA_INSUFFICIENT_BALANCE_CODE: &str = "40310000";

    /// Alpaca's rate limit (orders per minute for crypto)
    pub const ALPACA_ORDER_RATE_LIMIT_PER_MINUTE: u32 = 200;

    /// Safety margin for buying power (use 95% not 100%)
    pub const BUYING_POWER_SAFETY_MARGIN: f64 = 0.95;

    /// Basis points in one percent
    pub const BASIS_POINTS_PER_PERCENT: f64 = 100.0;

    /// Basis points in 100%
    pub const BASIS_POINTS_PER_UNIT: f64 = 10_000.0;
}

/// Rate limiting constants
pub mod rate_limit {
    use super::*;

    /// Default rate limit interval (250ms = 4 orders/sec per symbol)
    pub const DEFAULT_INTERVAL_MS: u64 = 250;

    /// Minimum safe interval to respect Alpaca's limits
    pub const MIN_SAFE_INTERVAL_MS: u64 = 200;
}

/// Caching constants
pub mod cache {
    /// Account balance cache TTL (seconds)
    pub const ACCOUNT_CACHE_TTL_SECS: u64 = 15;

    /// Position cache TTL (seconds)
    pub const POSITION_CACHE_TTL_SECS: u64 = 5;

    /// Market data history limit (number of candles/quotes to keep)
    pub const DEFAULT_HISTORY_LIMIT: usize = 50;
}

/// Logging event names for structured logging
pub mod events {
    pub const BUY_ORDER_FILLED: &str = "buy_order_filled";
    pub const SELL_ORDER_FILLED: &str = "sell_order_filled";
    pub const QUANTITY_MISMATCH: &str = "quantity_mismatch";
    pub const POSITION_OPENED: &str = "position_opened";
    pub const POSITION_CLOSED: &str = "position_closed";
    pub const STOP_LOSS_TRIGGERED: &str = "stop_loss_triggered";
    pub const TAKE_PROFIT_TRIGGERED: &str = "take_profit_triggered";
    pub const ORDER_RETRY: &str = "order_retry";
    pub const INSUFFICIENT_BALANCE: &str = "insufficient_balance";
    pub const RATE_LIMITED: &str = "rate_limited";
}
