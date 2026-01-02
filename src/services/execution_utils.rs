use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::warn;

use crate::exchange::traits::TradingApi;
use crate::exchange::types::AccountSummary;

/// Cached account balance to reduce API calls.
/// Refreshes every `refresh_interval` or on explicit invalidation.
#[derive(Clone)]
pub struct AccountCache {
    exchange: Arc<dyn TradingApi>,
    cache: Arc<RwLock<CachedAccount>>,
    refresh_interval: Duration,
}

struct CachedAccount {
    summary: Option<AccountSummary>,
    last_fetch: Option<Instant>,
}

impl AccountCache {
    pub fn new(exchange: Arc<dyn TradingApi>, refresh_interval_secs: u64) -> Self {
        Self {
            exchange,
            cache: Arc::new(RwLock::new(CachedAccount {
                summary: None,
                last_fetch: None,
            })),
            refresh_interval: Duration::from_secs(refresh_interval_secs),
        }
    }

    /// Get cached buying power. Refreshes if stale or missing.
    pub async fn buying_power(&self) -> f64 {
        let should_refresh = {
            let cache = self.cache.read().await;
            match cache.last_fetch {
                Some(t) if t.elapsed() < self.refresh_interval => false,
                _ => true,
            }
        };

        if should_refresh {
            self.refresh().await;
        }

        let cache = self.cache.read().await;
        cache
            .summary
            .as_ref()
            .and_then(|s| s.buying_power.or(s.cash))
            .unwrap_or(0.0)
    }

    /// Force refresh (call after successful order to update balance)
    pub async fn invalidate(&self) {
        let mut cache = self.cache.write().await;
        cache.last_fetch = None;
    }

    async fn refresh(&self) {
        match self.exchange.get_account().await {
            Ok(summary) => {
                let mut cache = self.cache.write().await;
                cache.summary = Some(summary);
                cache.last_fetch = Some(Instant::now());
            }
            Err(e) => {
                warn!("[CACHE] Failed to refresh account: {}", e);
            }
        }
    }
}

/// Pre-computed order sizing for fast execution.
#[derive(Clone, Debug)]
pub struct OrderSizing {
    pub qty: f64,
    pub notional: f64,
    pub limit_price: f64,
}

/// Calculate order sizing based on config and available balance.
/// Returns None if order cannot be placed.
pub fn compute_order_sizing(
    price: f64,
    buying_power: f64,
    min_order: f64,
    max_order: f64,
    target_pct_of_balance: f64,
) -> Option<OrderSizing> {
    if price <= 0.0 || buying_power <= 0.0 {
        return None;
    }

    // Target notional = percentage of buying power, clamped to min/max
    let mut notional = buying_power * target_pct_of_balance;

    // Clamp to configured limits
    if notional < min_order {
        notional = min_order;
    }
    if notional > max_order {
        notional = max_order;
    }

    // Safety: don't exceed 95% of buying power (leave room for fees)
    let max_affordable = buying_power * 0.95;
    if notional > max_affordable {
        if max_affordable < min_order {
            return None; // Can't afford minimum order
        }
        notional = max_affordable;
    }

    let qty = notional / price;

    Some(OrderSizing {
        qty,
        notional,
        limit_price: price,
    })
}

/// Aggressive limit price for faster fills.
/// For buys: slightly above mid (toward ask) to improve fill probability.
/// For sells: slightly below mid (toward bid).
pub fn aggressive_limit_price(bid: f64, ask: f64, side: &str, aggression_bps: f64) -> f64 {
    let mid = (bid + ask) / 2.0;
    let offset = mid * (aggression_bps / 10_000.0);

    if side == "buy" {
        // Move toward ask for faster fill
        (mid + offset).min(ask)
    } else {
        // Move toward bid for faster fill
        (mid - offset).max(bid)
    }
}

/// Rate limiter to prevent API abuse.
/// Uses per-symbol tracking so different symbols can trade independently.
#[derive(Clone)]
pub struct RateLimiter {
    last_order_per_symbol: Arc<DashMap<String, Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            last_order_per_symbol: Arc::new(DashMap::new()),
            min_interval: Duration::from_millis(min_interval_ms),
        }
    }

    /// Returns true if order is allowed for this symbol, false if rate limited.
    /// Each symbol has independent rate limiting.
    pub async fn try_acquire(&self, symbol: &str) -> bool {
        let now = Instant::now();

        // Check if this symbol is rate limited
        if let Some(entry) = self.last_order_per_symbol.get(symbol) {
            if entry.elapsed() < self.min_interval {
                return false; // Still in cooldown
            }
        }

        // Update last order time for this symbol
        self.last_order_per_symbol.insert(symbol.to_string(), now);
        true
    }
}
