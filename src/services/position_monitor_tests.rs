//! Unit tests for PositionTracker - tracking positions and pending orders.

#[cfg(test)]
mod position_tracker_tests {
    use crate::services::position_monitor::{PendingOrder, PositionInfo, PositionTracker};

    // ============= PositionTracker Basic Tests =============

    #[test]
    fn test_position_tracker_new() {
        let tracker = PositionTracker::new();
        assert!(tracker.get_all_positions().is_empty());
        assert!(tracker.get_all_pending_orders().is_empty());
    }

    // ============= Position Tests =============

    #[test]
    fn test_add_position() {
        let tracker = PositionTracker::new();

        let pos = PositionInfo {
            symbol: "BTC/USD".to_string(),
            entry_price: 50000.0,
            qty: 0.1,
            stop_loss: 49000.0,
            take_profit: 51000.0,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
        };

        tracker.add_position(pos);

        assert!(tracker.has_position("BTC/USD"));
        assert!(!tracker.has_position("ETH/USD"));
    }

    #[test]
    fn test_get_position() {
        let tracker = PositionTracker::new();

        let pos = PositionInfo {
            symbol: "ETH/USD".to_string(),
            entry_price: 3000.0,
            qty: 1.0,
            stop_loss: 2900.0,
            take_profit: 3100.0,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: Some("order123".to_string()),
        };

        tracker.add_position(pos);

        let retrieved = tracker.get_position("ETH/USD");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.entry_price, 3000.0);
        assert_eq!(retrieved.qty, 1.0);
        assert_eq!(retrieved.open_order_id, Some("order123".to_string()));
    }

    #[test]
    fn test_get_nonexistent_position() {
        let tracker = PositionTracker::new();
        let pos = tracker.get_position("NONEXISTENT/USD");
        assert!(pos.is_none());
    }

    #[test]
    fn test_remove_position() {
        let tracker = PositionTracker::new();

        let pos = PositionInfo {
            symbol: "SOL/USD".to_string(),
            entry_price: 100.0,
            qty: 10.0,
            stop_loss: 95.0,
            take_profit: 110.0,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
        };

        tracker.add_position(pos);
        assert!(tracker.has_position("SOL/USD"));

        let removed = tracker.remove_position("SOL/USD");
        assert!(removed.is_some());
        assert!(!tracker.has_position("SOL/USD"));
    }

    #[test]
    fn test_remove_nonexistent_position() {
        let tracker = PositionTracker::new();
        let removed = tracker.remove_position("NONEXISTENT/USD");
        assert!(removed.is_none());
    }

    #[test]
    fn test_get_all_positions() {
        let tracker = PositionTracker::new();

        for symbol in &["BTC/USD", "ETH/USD", "SOL/USD"] {
            let pos = PositionInfo {
                symbol: symbol.to_string(),
                entry_price: 100.0,
                qty: 1.0,
                stop_loss: 95.0,
                take_profit: 105.0,
                entry_time: "2025-01-01T00:00:00Z".to_string(),
                side: "buy".to_string(),
                is_closing: false,
                open_order_id: None,
            };
            tracker.add_position(pos);
        }

        let positions = tracker.get_all_positions();
        assert_eq!(positions.len(), 3);
    }

    #[test]
    fn test_mark_closing() {
        let tracker = PositionTracker::new();

        let pos = PositionInfo {
            symbol: "DOGE/USD".to_string(),
            entry_price: 0.08,
            qty: 10000.0,
            stop_loss: 0.07,
            take_profit: 0.09,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
        };

        tracker.add_position(pos);

        // Verify not closing initially
        let before = tracker.get_position("DOGE/USD").unwrap();
        assert!(!before.is_closing);

        // Mark as closing
        tracker.mark_closing("DOGE/USD");

        let after = tracker.get_position("DOGE/USD").unwrap();
        assert!(after.is_closing);
    }

    #[test]
    fn test_position_overwrite() {
        let tracker = PositionTracker::new();

        let pos1 = PositionInfo {
            symbol: "XRP/USD".to_string(),
            entry_price: 0.50,
            qty: 1000.0,
            stop_loss: 0.45,
            take_profit: 0.55,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
        };

        let pos2 = PositionInfo {
            symbol: "XRP/USD".to_string(),
            entry_price: 0.55,
            qty: 2000.0,
            stop_loss: 0.50,
            take_profit: 0.60,
            entry_time: "2025-01-01T01:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
        };

        tracker.add_position(pos1);
        tracker.add_position(pos2);

        // Should have the second position
        let pos = tracker.get_position("XRP/USD").unwrap();
        assert_eq!(pos.entry_price, 0.55);
        assert_eq!(pos.qty, 2000.0);
    }

    // ============= Pending Order Tests =============

    #[test]
    fn test_add_pending_order() {
        let tracker = PositionTracker::new();

        let order = PendingOrder {
            order_id: "order123".to_string(),
            symbol: "BTC/USD".to_string(),
            side: "buy".to_string(),
            limit_price: 50000.0,
            qty: 0.1,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            stop_loss: Some(49000.0),
            take_profit: Some(51000.0),
            last_check_time: None,
        };

        tracker.add_pending_order(order);

        let orders = tracker.get_all_pending_orders();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].order_id, "order123");
    }

    #[test]
    fn test_remove_pending_order() {
        let tracker = PositionTracker::new();

        let order = PendingOrder {
            order_id: "order456".to_string(),
            symbol: "ETH/USD".to_string(),
            side: "sell".to_string(),
            limit_price: 3100.0,
            qty: 1.0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            stop_loss: None,
            take_profit: None,
            last_check_time: None,
        };

        tracker.add_pending_order(order);
        assert_eq!(tracker.get_all_pending_orders().len(), 1);

        let removed = tracker.remove_pending_order("order456");
        assert!(removed.is_some());
        assert_eq!(tracker.get_all_pending_orders().len(), 0);
    }

    #[test]
    fn test_remove_nonexistent_pending_order() {
        let tracker = PositionTracker::new();
        let removed = tracker.remove_pending_order("nonexistent");
        assert!(removed.is_none());
    }

    #[test]
    fn test_multiple_pending_orders() {
        let tracker = PositionTracker::new();

        for i in 0..5 {
            let order = PendingOrder {
                order_id: format!("order{}", i),
                symbol: format!("SYM{}/USD", i),
                side: "buy".to_string(),
                limit_price: 100.0 + i as f64,
                qty: 1.0,
                created_at: "2025-01-01T00:00:00Z".to_string(),
                stop_loss: None,
                take_profit: None,
                last_check_time: None,
            };
            tracker.add_pending_order(order);
        }

        let orders = tracker.get_all_pending_orders();
        assert_eq!(orders.len(), 5);
    }

    #[test]
    fn test_update_pending_order_check_time() {
        let tracker = PositionTracker::new();

        let order = PendingOrder {
            order_id: "order789".to_string(),
            symbol: "SOL/USD".to_string(),
            side: "buy".to_string(),
            limit_price: 100.0,
            qty: 10.0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            stop_loss: None,
            take_profit: None,
            last_check_time: None,
        };

        tracker.add_pending_order(order);

        // Update check time
        tracker.update_pending_order_check_time("order789");

        let orders = tracker.get_all_pending_orders();
        assert!(orders[0].last_check_time.is_some());
    }

    // ============= PositionInfo Struct Tests =============

    #[test]
    fn test_position_info_fields() {
        let pos = PositionInfo {
            symbol: "LTC/USD".to_string(),
            entry_price: 80.0,
            qty: 5.0,
            stop_loss: 75.0,
            take_profit: 88.0,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: true,
            open_order_id: Some("tp_order".to_string()),
        };

        assert_eq!(pos.symbol, "LTC/USD");
        assert_eq!(pos.entry_price, 80.0);
        assert_eq!(pos.stop_loss, 75.0);
        assert_eq!(pos.take_profit, 88.0);
        assert!(pos.is_closing);
    }

    #[test]
    fn test_position_info_clone() {
        let pos = PositionInfo {
            symbol: "DOT/USD".to_string(),
            entry_price: 5.0,
            qty: 100.0,
            stop_loss: 4.5,
            take_profit: 5.5,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
        };

        let cloned = pos.clone();
        assert_eq!(cloned.symbol, "DOT/USD");
        assert_eq!(cloned.qty, 100.0);
    }

    // ============= PendingOrder Struct Tests =============

    #[test]
    fn test_pending_order_fields() {
        let order = PendingOrder {
            order_id: "test_order".to_string(),
            symbol: "SHIB/USD".to_string(),
            side: "sell".to_string(),
            limit_price: 0.00001,
            qty: 1000000.0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            stop_loss: Some(0.000009),
            take_profit: Some(0.000011),
            last_check_time: None,
        };

        assert_eq!(order.order_id, "test_order");
        assert_eq!(order.side, "sell");
        assert_eq!(order.stop_loss, Some(0.000009));
    }

    #[test]
    fn test_pending_order_clone() {
        let order = PendingOrder {
            order_id: "clone_test".to_string(),
            symbol: "ADA/USD".to_string(),
            side: "buy".to_string(),
            limit_price: 0.35,
            qty: 500.0,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            stop_loss: None,
            take_profit: None,
            last_check_time: None,
        };

        let cloned = order.clone();
        assert_eq!(cloned.order_id, "clone_test");
    }

    // ============= Concurrent Access Tests =============

    #[test]
    fn test_concurrent_position_access() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(PositionTracker::new());
        let mut handles = vec![];

        // Spawn threads that add positions
        for i in 0..10 {
            let tracker_clone = Arc::clone(&tracker);
            let handle = thread::spawn(move || {
                let pos = PositionInfo {
                    symbol: format!("SYM{}/USD", i),
                    entry_price: 100.0 + i as f64,
                    qty: 1.0,
                    stop_loss: 95.0,
                    take_profit: 105.0,
                    entry_time: "2025-01-01T00:00:00Z".to_string(),
                    side: "buy".to_string(),
                    is_closing: false,
                    open_order_id: None,
                };
                tracker_clone.add_position(pos);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let positions = tracker.get_all_positions();
        assert_eq!(positions.len(), 10);
    }

    #[test]
    fn test_concurrent_pending_order_access() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(PositionTracker::new());
        let mut handles = vec![];

        for i in 0..10 {
            let tracker_clone = Arc::clone(&tracker);
            let handle = thread::spawn(move || {
                let order = PendingOrder {
                    order_id: format!("order{}", i),
                    symbol: format!("SYM{}/USD", i),
                    side: "buy".to_string(),
                    limit_price: 100.0,
                    qty: 1.0,
                    created_at: "2025-01-01T00:00:00Z".to_string(),
                    stop_loss: None,
                    take_profit: None,
                    last_check_time: None,
                };
                tracker_clone.add_pending_order(order);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let orders = tracker.get_all_pending_orders();
        assert_eq!(orders.len(), 10);
    }
}
