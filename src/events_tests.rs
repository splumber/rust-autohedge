//! Unit tests for Events - all event types used in the system.

#[cfg(test)]
mod events_tests {
    use crate::events::*;

    // ============= MarketEvent::Quote Tests =============

    #[test]
    fn test_market_event_quote() {
        let event = MarketEvent::Quote {
            symbol: "BTC/USD".to_string(),
            bid: 50000.0,
            ask: 50001.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        if let MarketEvent::Quote { symbol, bid, ask, timestamp } = event {
            assert_eq!(symbol, "BTC/USD");
            assert_eq!(bid, 50000.0);
            assert_eq!(ask, 50001.0);
            assert_eq!(timestamp, "2025-01-01T00:00:00Z");
        } else {
            panic!("Expected Quote event");
        }
    }

    #[test]
    fn test_market_event_quote_spread() {
        let event = MarketEvent::Quote {
            symbol: "ETH/USD".to_string(),
            bid: 3000.0,
            ask: 3001.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        if let MarketEvent::Quote { bid, ask, .. } = event {
            let spread = ask - bid;
            assert!((spread - 1.0).abs() < 0.001);
        }
    }

    #[test]
    fn test_market_event_quote_clone() {
        let event = MarketEvent::Quote {
            symbol: "SOL/USD".to_string(),
            bid: 100.0,
            ask: 100.5,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        let cloned = event.clone();
        if let MarketEvent::Quote { symbol, .. } = cloned {
            assert_eq!(symbol, "SOL/USD");
        }
    }

    // ============= MarketEvent::Trade Tests =============

    #[test]
    fn test_market_event_trade() {
        let event = MarketEvent::Trade {
            symbol: "DOGE/USD".to_string(),
            price: 0.08,
            size: 10000.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        if let MarketEvent::Trade { symbol, price, size, timestamp } = event {
            assert_eq!(symbol, "DOGE/USD");
            assert_eq!(price, 0.08);
            assert_eq!(size, 10000.0);
            assert_eq!(timestamp, "2025-01-01T00:00:00Z");
        } else {
            panic!("Expected Trade event");
        }
    }

    #[test]
    fn test_market_event_trade_notional() {
        let event = MarketEvent::Trade {
            symbol: "XRP/USD".to_string(),
            price: 0.55,
            size: 1000.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        if let MarketEvent::Trade { price, size, .. } = event {
            let notional = price * size;
            assert!((notional - 550.0).abs() < 0.001);
        }
    }

    // ============= AnalysisSignal Tests =============

    #[test]
    fn test_analysis_signal_buy() {
        let signal = AnalysisSignal {
            symbol: "BTC/USD".to_string(),
            signal: "buy".to_string(),
            confidence: 0.85,
            thesis: "Bullish momentum detected".to_string(),
            market_context: "tp=51000, sl=49000".to_string(),
        };

        assert_eq!(signal.symbol, "BTC/USD");
        assert_eq!(signal.signal, "buy");
        assert_eq!(signal.confidence, 0.85);
    }

    #[test]
    fn test_analysis_signal_sell() {
        let signal = AnalysisSignal {
            symbol: "ETH/USD".to_string(),
            signal: "sell".to_string(),
            confidence: 0.75,
            thesis: "Bearish divergence".to_string(),
            market_context: "current_price=3000".to_string(),
        };

        assert_eq!(signal.signal, "sell");
    }

    #[test]
    fn test_analysis_signal_no_trade() {
        let signal = AnalysisSignal {
            symbol: "SOL/USD".to_string(),
            signal: "no_trade".to_string(),
            confidence: 0.0,
            thesis: "Market too volatile".to_string(),
            market_context: "spread_bps=100".to_string(),
        };

        assert_eq!(signal.signal, "no_trade");
        assert_eq!(signal.confidence, 0.0);
    }

    #[test]
    fn test_analysis_signal_hft() {
        let signal = AnalysisSignal {
            symbol: "DOGE/USD".to_string(),
            signal: "buy".to_string(),
            confidence: 1.0,
            thesis: "HFT momentum: edge_bps=15.0, spread_bps=5.0".to_string(),
            market_context: "tp=0.082, sl=0.078".to_string(),
        };

        assert!(signal.thesis.starts_with("HFT"));
        assert!(signal.market_context.contains("tp="));
        assert!(signal.market_context.contains("sl="));
    }

    // ============= OrderRequest Tests =============

    #[test]
    fn test_order_request_market_buy() {
        let order = OrderRequest {
            symbol: "BTC/USD".to_string(),
            action: "buy".to_string(),
            qty: 0.1,
            order_type: "market".to_string(),
            limit_price: None,
            stop_loss: Some(49000.0),
            take_profit: Some(51000.0),
        };

        assert_eq!(order.symbol, "BTC/USD");
        assert_eq!(order.action, "buy");
        assert_eq!(order.order_type, "market");
        assert_eq!(order.limit_price, None);
    }

    #[test]
    fn test_order_request_limit_buy() {
        let order = OrderRequest {
            symbol: "ETH/USD".to_string(),
            action: "buy".to_string(),
            qty: 1.0,
            order_type: "limit".to_string(),
            limit_price: Some(2950.0),
            stop_loss: Some(2850.0),
            take_profit: Some(3100.0),
        };

        assert_eq!(order.order_type, "limit");
        assert_eq!(order.limit_price, Some(2950.0));
    }

    #[test]
    fn test_order_request_sell() {
        let order = OrderRequest {
            symbol: "SOL/USD".to_string(),
            action: "sell".to_string(),
            qty: 10.0,
            order_type: "market".to_string(),
            limit_price: None,
            stop_loss: None,
            take_profit: None,
        };

        assert_eq!(order.action, "sell");
        assert!(order.stop_loss.is_none());
        assert!(order.take_profit.is_none());
    }

    #[test]
    fn test_order_request_hft() {
        let order = OrderRequest {
            symbol: "DOGE/USD".to_string(),
            action: "buy".to_string(),
            qty: 0.0, // Execution will determine
            order_type: "hft_buy".to_string(),
            limit_price: None,
            stop_loss: Some(0.078),
            take_profit: Some(0.082),
        };

        assert_eq!(order.order_type, "hft_buy");
    }

    // ============= ExecutionReport Tests =============

    #[test]
    fn test_execution_report_filled() {
        let report = ExecutionReport {
            symbol: "BTC/USD".to_string(),
            order_id: "order123".to_string(),
            status: "filled".to_string(),
            side: "buy".to_string(),
            price: Some(50000.0),
            qty: Some(0.1),
        };

        assert_eq!(report.status, "filled");
        assert_eq!(report.side, "buy");
        assert_eq!(report.price, Some(50000.0));
        assert_eq!(report.qty, Some(0.1));
    }

    #[test]
    fn test_execution_report_new() {
        let report = ExecutionReport {
            symbol: "ETH/USD".to_string(),
            order_id: "order456".to_string(),
            status: "new".to_string(),
            side: "sell".to_string(),
            price: Some(3000.0),
            qty: Some(1.0),
        };

        assert_eq!(report.status, "new");
    }

    #[test]
    fn test_execution_report_rejected() {
        let report = ExecutionReport {
            symbol: "SOL/USD".to_string(),
            order_id: "order789".to_string(),
            status: "rejected".to_string(),
            side: "buy".to_string(),
            price: None,
            qty: None,
        };

        assert_eq!(report.status, "rejected");
        assert!(report.price.is_none());
        assert!(report.qty.is_none());
    }

    // ============= Event Enum Tests =============

    #[test]
    fn test_event_market() {
        let event = Event::Market(MarketEvent::Quote {
            symbol: "BTC/USD".to_string(),
            bid: 50000.0,
            ask: 50001.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        });

        assert!(matches!(event, Event::Market(_)));
    }

    #[test]
    fn test_event_signal() {
        let event = Event::Signal(AnalysisSignal {
            symbol: "ETH/USD".to_string(),
            signal: "buy".to_string(),
            confidence: 0.9,
            thesis: "Strong momentum".to_string(),
            market_context: "context".to_string(),
        });

        assert!(matches!(event, Event::Signal(_)));
    }

    #[test]
    fn test_event_order() {
        let event = Event::Order(OrderRequest {
            symbol: "SOL/USD".to_string(),
            action: "buy".to_string(),
            qty: 1.0,
            order_type: "limit".to_string(),
            limit_price: Some(100.0),
            stop_loss: None,
            take_profit: None,
        });

        assert!(matches!(event, Event::Order(_)));
    }

    #[test]
    fn test_event_execution() {
        let event = Event::Execution(ExecutionReport {
            symbol: "DOGE/USD".to_string(),
            order_id: "order123".to_string(),
            status: "filled".to_string(),
            side: "buy".to_string(),
            price: Some(0.08),
            qty: Some(10000.0),
        });

        assert!(matches!(event, Event::Execution(_)));
    }

    #[test]
    fn test_event_clone() {
        let event = Event::Market(MarketEvent::Trade {
            symbol: "XRP/USD".to_string(),
            price: 0.55,
            size: 1000.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        });

        let cloned = event.clone();
        if let Event::Market(MarketEvent::Trade { symbol, .. }) = cloned {
            assert_eq!(symbol, "XRP/USD");
        } else {
            panic!("Clone failed");
        }
    }

    #[test]
    fn test_event_debug() {
        let event = Event::Signal(AnalysisSignal {
            symbol: "LTC/USD".to_string(),
            signal: "buy".to_string(),
            confidence: 0.8,
            thesis: "Test".to_string(),
            market_context: "ctx".to_string(),
        });

        let debug = format!("{:?}", event);
        assert!(debug.contains("Signal"));
        assert!(debug.contains("LTC/USD"));
    }
}

