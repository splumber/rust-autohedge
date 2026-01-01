//! Unit tests for the EventBus - the core pub/sub messaging system.

#[cfg(test)]
mod bus_tests {
    use crate::bus::EventBus;
    use crate::events::{Event, MarketEvent, AnalysisSignal, OrderRequest, ExecutionReport};

    #[tokio::test]
    async fn test_eventbus_new() {
        let bus = EventBus::new(100);
        // Should be able to create a bus without panicking
        let _rx = bus.subscribe();
    }

    #[tokio::test]
    async fn test_eventbus_publish_subscribe() {
        let bus = EventBus::new(100);
        let mut rx = bus.subscribe();

        let event = Event::Market(MarketEvent::Quote {
            symbol: "BTC/USD".to_string(),
            bid: 50000.0,
            ask: 50001.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        });

        // Publish should succeed
        let result = bus.publish(event.clone());
        assert!(result.is_ok());

        // Subscriber should receive the event
        let received = rx.recv().await;
        assert!(received.is_ok());
        
        if let Ok(Event::Market(MarketEvent::Quote { symbol, bid, ask, .. })) = received {
            assert_eq!(symbol, "BTC/USD");
            assert_eq!(bid, 50000.0);
            assert_eq!(ask, 50001.0);
        } else {
            panic!("Expected Market Quote event");
        }
    }

    #[tokio::test]
    async fn test_eventbus_multiple_subscribers() {
        let bus = EventBus::new(100);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let event = Event::Signal(AnalysisSignal {
            symbol: "ETH/USD".to_string(),
            signal: "buy".to_string(),
            confidence: 0.85,
            thesis: "Bullish momentum".to_string(),
            market_context: "tp=3500, sl=3200".to_string(),
        });

        bus.publish(event).unwrap();

        // Both subscribers should receive
        let r1 = rx1.recv().await;
        let r2 = rx2.recv().await;

        assert!(r1.is_ok());
        assert!(r2.is_ok());
    }

    #[tokio::test]
    async fn test_eventbus_order_event() {
        let bus = EventBus::new(100);
        let mut rx = bus.subscribe();

        let order = OrderRequest {
            symbol: "SOL/USD".to_string(),
            action: "buy".to_string(),
            qty: 10.0,
            order_type: "limit".to_string(),
            limit_price: Some(100.0),
            stop_loss: Some(95.0),
            take_profit: Some(110.0),
        };

        bus.publish(Event::Order(order)).unwrap();

        if let Ok(Event::Order(req)) = rx.recv().await {
            assert_eq!(req.symbol, "SOL/USD");
            assert_eq!(req.action, "buy");
            assert_eq!(req.qty, 10.0);
            assert_eq!(req.limit_price, Some(100.0));
            assert_eq!(req.stop_loss, Some(95.0));
            assert_eq!(req.take_profit, Some(110.0));
        } else {
            panic!("Expected Order event");
        }
    }

    #[tokio::test]
    async fn test_eventbus_execution_report() {
        let bus = EventBus::new(100);
        let mut rx = bus.subscribe();

        let report = ExecutionReport {
            symbol: "DOGE/USD".to_string(),
            order_id: "order123".to_string(),
            status: "filled".to_string(),
            side: "buy".to_string(),
            price: Some(0.08),
            qty: Some(1000.0),
        };

        bus.publish(Event::Execution(report)).unwrap();

        if let Ok(Event::Execution(exec)) = rx.recv().await {
            assert_eq!(exec.symbol, "DOGE/USD");
            assert_eq!(exec.order_id, "order123");
            assert_eq!(exec.status, "filled");
            assert_eq!(exec.side, "buy");
            assert_eq!(exec.price, Some(0.08));
            assert_eq!(exec.qty, Some(1000.0));
        } else {
            panic!("Expected Execution event");
        }
    }

    #[tokio::test]
    async fn test_eventbus_trade_event() {
        let bus = EventBus::new(100);
        let mut rx = bus.subscribe();

        let event = Event::Market(MarketEvent::Trade {
            symbol: "XRP/USD".to_string(),
            price: 0.55,
            size: 5000.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        });

        bus.publish(event).unwrap();

        if let Ok(Event::Market(MarketEvent::Trade { symbol, price, size, .. })) = rx.recv().await {
            assert_eq!(symbol, "XRP/USD");
            assert_eq!(price, 0.55);
            assert_eq!(size, 5000.0);
        } else {
            panic!("Expected Market Trade event");
        }
    }

    #[tokio::test]
    async fn test_eventbus_capacity() {
        // Test that bus respects capacity
        let bus = EventBus::new(5);
        let _rx = bus.subscribe(); // Must have at least one subscriber

        // Publish multiple events
        for i in 0..10 {
            let event = Event::Market(MarketEvent::Quote {
                symbol: format!("SYM{}/USD", i),
                bid: i as f64,
                ask: (i + 1) as f64,
                timestamp: "2025-01-01T00:00:00Z".to_string(),
            });
            let _ = bus.publish(event);
        }
        // Should not panic - channel handles overflow by lagging
    }
}

