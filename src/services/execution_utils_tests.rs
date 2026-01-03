//! Unit tests for execution utilities - order sizing, aggressive pricing, rate limiting.

#[cfg(test)]
mod execution_utils_tests {
    use crate::services::execution_utils::*;

    // ============= Order Sizing Tests =============

    #[test]
    fn test_compute_order_sizing_basic() {
        let result = compute_order_sizing(
            100.0,   // price
            10000.0, // buying_power
            10.0,    // min_order
            100.0,   // max_order
            0.05,    // target 5% of balance
        );

        assert!(result.is_some());
        let sizing = result.unwrap();
        assert_eq!(sizing.notional, 100.0); // 5% of 10000 = 500, clamped to max 100
        assert_eq!(sizing.qty, 1.0); // 100 / 100 = 1
        assert_eq!(sizing.limit_price, 100.0);
    }

    #[test]
    fn test_compute_order_sizing_min_order() {
        let result = compute_order_sizing(
            100.0, // price
            100.0, // buying_power (small)
            10.0,  // min_order
            100.0, // max_order
            0.05,  // target 5% = $5, but min is $10
        );

        assert!(result.is_some());
        let sizing = result.unwrap();
        assert_eq!(sizing.notional, 10.0); // Bumped up to min
    }

    #[test]
    fn test_compute_order_sizing_max_order() {
        let result = compute_order_sizing(
            100.0,    // price
            100000.0, // buying_power (large)
            10.0,     // min_order
            100.0,    // max_order
            0.10,     // target 10% = $10000, clamped to max $100
        );

        assert!(result.is_some());
        let sizing = result.unwrap();
        assert_eq!(sizing.notional, 100.0); // Clamped to max
    }

    #[test]
    fn test_compute_order_sizing_95_percent_cap() {
        // Test that we don't exceed 95% of buying power
        let result = compute_order_sizing(
            100.0, // price
            50.0,  // buying_power (only $50)
            10.0,  // min_order
            100.0, // max_order
            0.50,  // target 50% = $25, but max affordable is $47.50 (95%)
        );

        assert!(result.is_some());
        let sizing = result.unwrap();
        assert_eq!(sizing.notional, 25.0); // 50% of $50 = $25
    }

    #[test]
    fn test_compute_order_sizing_cant_afford_min() {
        let result = compute_order_sizing(
            100.0, // price
            5.0,   // buying_power (only $5)
            10.0,  // min_order ($10 minimum)
            100.0, // max_order
            0.50,  // target 50% = $2.50
        );

        // Can't afford minimum order
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_order_sizing_zero_price() {
        let result = compute_order_sizing(
            0.0, // invalid price
            10000.0, 10.0, 100.0, 0.05,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_order_sizing_negative_price() {
        let result = compute_order_sizing(
            -100.0, // invalid price
            10000.0, 10.0, 100.0, 0.05,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_order_sizing_zero_buying_power() {
        let result = compute_order_sizing(
            100.0, 0.0, // no buying power
            10.0, 100.0, 0.05,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_compute_order_sizing_exact_fit() {
        // Notional fits exactly within constraints
        let result = compute_order_sizing(
            50.0,   // price
            1000.0, // buying_power
            50.0,   // min_order
            50.0,   // max_order (same as min)
            0.05,   // target 5% = $50
        );

        assert!(result.is_some());
        let sizing = result.unwrap();
        assert_eq!(sizing.notional, 50.0);
        assert_eq!(sizing.qty, 1.0);
    }

    // ============= Aggressive Limit Price Tests =============

    #[test]
    fn test_aggressive_limit_price_buy() {
        // Buy: should move toward ask
        let price = aggressive_limit_price(100.0, 101.0, "buy", 50.0);
        // Mid = 100.5, offset = 100.5 * 50/10000 = 0.5025
        // Result = 100.5 + 0.5025 = 101.0025, capped at ask (101.0)
        assert!(price > 100.5);
        assert!(price <= 101.0);
    }

    #[test]
    fn test_aggressive_limit_price_sell() {
        // Sell: should move toward bid
        let price = aggressive_limit_price(100.0, 101.0, "sell", 50.0);
        // Mid = 100.5, offset = 100.5 * 50/10000 = 0.5025
        // Result = 100.5 - 0.5025 = 99.9975, floored at bid (100.0)
        assert!(price < 100.5);
        assert!(price >= 100.0);
    }

    #[test]
    fn test_aggressive_limit_price_zero_aggression() {
        // With 0 aggression, should return mid
        let price = aggressive_limit_price(100.0, 102.0, "buy", 0.0);
        assert_eq!(price, 101.0); // Mid price
    }

    #[test]
    fn test_aggressive_limit_price_high_aggression_buy() {
        // Very aggressive buy should cap at ask
        let price = aggressive_limit_price(100.0, 101.0, "buy", 500.0);
        assert_eq!(price, 101.0); // Capped at ask
    }

    #[test]
    fn test_aggressive_limit_price_high_aggression_sell() {
        // Very aggressive sell should floor at bid
        let price = aggressive_limit_price(100.0, 101.0, "sell", 500.0);
        assert_eq!(price, 100.0); // Floored at bid
    }

    #[test]
    fn test_aggressive_limit_price_tight_spread() {
        // Tight spread
        let price = aggressive_limit_price(100.00, 100.01, "buy", 10.0);
        assert!(price >= 100.00);
        assert!(price <= 100.01);
    }

    #[test]
    fn test_aggressive_limit_price_wide_spread() {
        // Wide spread
        let price = aggressive_limit_price(99.0, 101.0, "buy", 10.0);
        // Mid = 100, offset = 100 * 10/10000 = 0.1
        // Result = 100.1
        assert!((price - 100.1).abs() < 0.01);
    }

    // ============= Rate Limiter Tests =============

    #[tokio::test]
    async fn test_rate_limiter_first_call() {
        let limiter = RateLimiter::new(100); // 100ms interval
        let allowed = limiter.try_acquire("BTC/USD").await;
        assert!(allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_immediate_second_call() {
        let limiter = RateLimiter::new(100); // 100ms interval

        let first = limiter.try_acquire("BTC/USD").await;
        let second = limiter.try_acquire("BTC/USD").await;

        assert!(first);
        assert!(!second); // Should be rate limited
    }

    #[tokio::test]
    async fn test_rate_limiter_after_interval() {
        let limiter = RateLimiter::new(50); // 50ms interval

        let first = limiter.try_acquire("BTC/USD").await;
        assert!(first);

        // Wait for interval to pass
        tokio::time::sleep(tokio::time::Duration::from_millis(60)).await;

        let second = limiter.try_acquire("BTC/USD").await;
        assert!(second); // Should be allowed now
    }

    #[tokio::test]
    async fn test_rate_limiter_multiple_requests() {
        let limiter = RateLimiter::new(10); // 10ms interval

        let mut allowed_count = 0;
        let mut denied_count = 0;

        for _ in 0..10 {
            if limiter.try_acquire("BTC/USD").await {
                allowed_count += 1;
            } else {
                denied_count += 1;
            }
        }

        // First should be allowed, rest denied (no delays)
        assert_eq!(allowed_count, 1);
        assert_eq!(denied_count, 9);
    }

    #[tokio::test]
    async fn test_rate_limiter_with_delays() {
        let limiter = RateLimiter::new(20); // 20ms interval

        let first = limiter.try_acquire("BTC/USD").await;
        assert!(first);

        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        let second = limiter.try_acquire("BTC/USD").await;
        assert!(second);

        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        let third = limiter.try_acquire("BTC/USD").await;
        assert!(third);
    }

    #[tokio::test]
    async fn test_rate_limiter_per_symbol() {
        let limiter = RateLimiter::new(100); // 100ms interval

        // Different symbols should not interfere
        let btc1 = limiter.try_acquire("BTC/USD").await;
        let eth1 = limiter.try_acquire("ETH/USD").await;
        let sol1 = limiter.try_acquire("SOL/USD").await;

        assert!(btc1);
        assert!(eth1);
        assert!(sol1);

        // But same symbol should be rate limited
        let btc2 = limiter.try_acquire("BTC/USD").await;
        assert!(!btc2);
    }

    #[tokio::test]
    async fn test_rate_limiter_exact_timing_250ms() {
        let limiter = RateLimiter::new(250); // 250ms interval (production config)

        // First call should succeed
        let first = limiter.try_acquire("TEST/USD").await;
        assert!(first, "First call should be allowed");

        // Immediate second call should fail
        let second = limiter.try_acquire("TEST/USD").await;
        assert!(!second, "Immediate second call should be rate limited");

        // Wait exactly 250ms
        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

        // Third call should now succeed
        let third = limiter.try_acquire("TEST/USD").await;
        assert!(third, "Call after 250ms should be allowed");

        // Fourth immediate call should fail again
        let fourth = limiter.try_acquire("TEST/USD").await;
        assert!(!fourth, "Immediate call after third should be rate limited");
    }

    #[tokio::test]
    async fn test_rate_limiter_slightly_before_interval() {
        let limiter = RateLimiter::new(250); // 250ms interval

        let first = limiter.try_acquire("TIMING/USD").await;
        assert!(first);

        // Wait 240ms (slightly less than 250ms)
        tokio::time::sleep(tokio::time::Duration::from_millis(240)).await;

        let second = limiter.try_acquire("TIMING/USD").await;
        assert!(!second, "Call at 240ms should still be rate limited");

        // Wait remaining 15ms (total 255ms)
        tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;

        let third = limiter.try_acquire("TIMING/USD").await;
        assert!(third, "Call at 255ms should be allowed");
    }

    // ============= OrderSizing Struct Tests =============

    #[test]
    fn test_order_sizing_struct() {
        let sizing = OrderSizing {
            qty: 10.0,
            notional: 1000.0,
            limit_price: 100.0,
        };
        assert_eq!(sizing.qty, 10.0);
        assert_eq!(sizing.notional, 1000.0);
        assert_eq!(sizing.limit_price, 100.0);
    }

    #[test]
    fn test_order_sizing_clone() {
        let sizing = OrderSizing {
            qty: 5.0,
            notional: 500.0,
            limit_price: 100.0,
        };
        let cloned = sizing.clone();
        assert_eq!(cloned.qty, 5.0);
    }

    #[test]
    fn test_order_sizing_debug() {
        let sizing = OrderSizing {
            qty: 1.0,
            notional: 100.0,
            limit_price: 100.0,
        };
        let debug = format!("{:?}", sizing);
        assert!(debug.contains("OrderSizing"));
        assert!(debug.contains("qty"));
    }
}
