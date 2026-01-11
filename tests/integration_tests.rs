//! Integration tests for the trading system.
//! These tests verify that components work together correctly.

use rust_autohedge::bus::EventBus;
use rust_autohedge::data::store::{MarketStore, Quote};
use rust_autohedge::events::{AnalysisSignal, Event, ExecutionReport, MarketEvent, OrderRequest};
use rust_autohedge::services::execution_utils::{aggressive_limit_price, compute_order_sizing};
use rust_autohedge::services::position_monitor::{PendingOrder, PositionInfo, PositionTracker};

/// Test the complete flow from market data to signal generation
#[tokio::test]
async fn test_market_data_to_signal_flow() {
    let bus = EventBus::new(100);
    let store = MarketStore::new(100);

    // Simulate receiving market data
    let quote = Quote {
        symbol: "BTC/USD".to_string(),
        bid_price: 50000.0,
        ask_price: 50001.0,
        bid_size: 1.0,
        ask_size: 1.0,
        timestamp: "2025-01-01T00:00:00Z".to_string(),
    };

    store.update_quote("BTC/USD".to_string(), quote.clone());

    // Publish market event
    let event = Event::Market(MarketEvent::Quote {
        symbol: "BTC/USD".to_string(),
        bid: 50000.0,
        ask: 50001.0,
        timestamp: "2025-01-01T00:00:00Z".to_string(),
    });

    let mut rx = bus.subscribe();
    bus.publish(event).unwrap();

    // Verify event received
    let received = rx.recv().await.unwrap();
    assert!(matches!(received, Event::Market(MarketEvent::Quote { .. })));

    // Verify store has data
    let latest = store.get_latest_quote("BTC/USD").unwrap();
    assert_eq!(latest.bid_price, 50000.0);
}

/// Test signal to order flow
#[tokio::test]
async fn test_signal_to_order_flow() {
    let bus = EventBus::new(100);
    let mut rx = bus.subscribe();

    // Create analysis signal
    let signal = AnalysisSignal {
        symbol: "ETH/USD".to_string(),
        signal: "buy".to_string(),
        confidence: 0.9,
        thesis: "HFT momentum: edge_bps=15.0".to_string(),
        market_context: "tp=3100.0, sl=2900.0".to_string(),
    };

    bus.publish(Event::Signal(signal)).unwrap();

    // Verify signal received
    if let Ok(Event::Signal(sig)) = rx.recv().await {
        assert_eq!(sig.symbol, "ETH/USD");
        assert_eq!(sig.signal, "buy");

        // Parse TP/SL from market_context
        assert!(sig.market_context.contains("tp="));
        assert!(sig.market_context.contains("sl="));
    }
}

/// Test order to execution flow
#[tokio::test]
async fn test_order_to_execution_flow() {
    let bus = EventBus::new(100);
    let mut rx = bus.subscribe();

    // Create order request
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

    // Verify order received
    if let Ok(Event::Order(ord)) = rx.recv().await {
        assert_eq!(ord.symbol, "SOL/USD");
        assert_eq!(ord.action, "buy");
        assert_eq!(ord.limit_price, Some(100.0));
    }

    // Simulate execution report
    let report = ExecutionReport {
        symbol: "SOL/USD".to_string(),
        order_id: "order123".to_string(),
        status: "filled".to_string(),
        side: "buy".to_string(),
        price: Some(100.0),
        qty: Some(10.0),
    };

    bus.publish(Event::Execution(report)).unwrap();

    if let Ok(Event::Execution(exec)) = rx.recv().await {
        assert_eq!(exec.status, "filled");
    }
}

/// Test position tracking flow
#[test]
fn test_position_tracking_flow() {
    let tracker = PositionTracker::new();

    // Add pending order (buy)
    let pending_order = PendingOrder {
        order_id: "order123".to_string(),
        symbol: "DOGE/USD".to_string(),
        side: "buy".to_string(),
        limit_price: 0.08,
        qty: 10000.0,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        stop_loss: Some(0.075),
        take_profit: Some(0.085),
        last_check_time: None,
    };

    tracker.add_pending_order(pending_order);
    assert_eq!(tracker.get_all_pending_orders().len(), 1);

    // Simulate order fill - convert to position
    tracker.remove_pending_order("order123");

    let position = PositionInfo {
        symbol: "DOGE/USD".to_string(),
        entry_price: 0.08,
        qty: 10000.0,
        stop_loss: 0.075,
        take_profit: 0.085,
        entry_time: "2025-01-01T00:00:00Z".to_string(),
        side: "buy".to_string(),
        is_closing: false,
        open_order_id: None,
        last_recreate_attempt: None,
        recreate_attempts: 0,
        highest_price: 0.08,
        trailing_stop_active: false,
        trailing_stop_price: 0.075,
    };

    tracker.add_position(position);
    assert!(tracker.has_position("DOGE/USD"));
    assert_eq!(tracker.get_all_pending_orders().len(), 0);
}

/// Test order sizing with position tracker
#[test]
fn test_order_sizing_integration() {
    let tracker = PositionTracker::new();

    // Get latest quote (simulated)
    let bid = 100.0;
    let ask = 100.1;
    let mid = (bid + ask) / 2.0;

    // Calculate aggressive limit price
    let limit_price = aggressive_limit_price(bid, ask, "buy", 10.0);
    assert!(limit_price > mid);
    assert!(limit_price <= ask);

    // Calculate order sizing
    let sizing = compute_order_sizing(
        limit_price,
        10000.0, // buying power
        10.0,    // min order
        100.0,   // max order
        0.05,    // 5% of balance
    )
    .unwrap();

    assert!(sizing.notional >= 10.0);
    assert!(sizing.notional <= 100.0);

    // Create position after fill
    let position = PositionInfo {
        symbol: "TEST/USD".to_string(),
        entry_price: limit_price,
        qty: sizing.qty,
        stop_loss: limit_price * 0.99,
        take_profit: limit_price * 1.01,
        entry_time: "2025-01-01T00:00:00Z".to_string(),
        side: "buy".to_string(),
        is_closing: false,
        open_order_id: None,
        last_recreate_attempt: None,
        recreate_attempts: 0,
        highest_price: limit_price,
        trailing_stop_active: false,
        trailing_stop_price: limit_price * 0.99,
    };

    tracker.add_position(position);
    assert!(tracker.has_position("TEST/USD"));
}

/// Test multiple symbol handling
#[tokio::test]
async fn test_multi_symbol_flow() {
    let bus = EventBus::new(100);
    let store = MarketStore::new(100);
    let tracker = PositionTracker::new();

    let symbols = vec!["BTC/USD", "ETH/USD", "SOL/USD", "DOGE/USD"];

    // Add quotes for each symbol
    for (i, symbol) in symbols.iter().enumerate() {
        let quote = Quote {
            symbol: symbol.to_string(),
            bid_price: (i + 1) as f64 * 1000.0,
            ask_price: (i + 1) as f64 * 1000.0 + 1.0,
            bid_size: 1.0,
            ask_size: 1.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };
        store.update_quote(symbol.to_string(), quote);
    }

    // Add positions for some symbols
    for symbol in &["BTC/USD", "ETH/USD"] {
        let pos = PositionInfo {
            symbol: symbol.to_string(),
            entry_price: 1000.0,
            qty: 1.0,
            stop_loss: 950.0,
            take_profit: 1050.0,
            entry_time: "2025-01-01T00:00:00Z".to_string(),
            side: "buy".to_string(),
            is_closing: false,
            open_order_id: None,
            last_recreate_attempt: None,
            recreate_attempts: 0,
            highest_price: 1000.0,
            trailing_stop_active: false,
            trailing_stop_price: 950.0,
        };
        tracker.add_position(pos);
    }

    // Verify state
    assert_eq!(store.get_quote_history("BTC/USD").len(), 1);
    assert_eq!(store.get_quote_history("SOL/USD").len(), 1);
    assert!(tracker.has_position("BTC/USD"));
    assert!(tracker.has_position("ETH/USD"));
    assert!(!tracker.has_position("SOL/USD"));
    assert!(!tracker.has_position("DOGE/USD"));
}

/// Test TP/SL calculation from entry price
#[test]
fn test_tp_sl_calculation() {
    let entry_price = 100.0;
    let tp_pct = 1.0; // 1%
    let sl_pct = 0.5; // 0.5%

    let take_profit = entry_price * (1.0 + tp_pct / 100.0);
    let stop_loss = entry_price * (1.0 - sl_pct / 100.0);

    assert_eq!(take_profit, 101.0);
    assert_eq!(stop_loss, 99.5);

    // Verify TP > entry > SL
    assert!(take_profit > entry_price);
    assert!(stop_loss < entry_price);
}

/// Test HFT edge calculation
#[test]
fn test_hft_edge_calculation() {
    let past_mid: f64 = 100.0;
    let current_mid: f64 = 100.15;

    // Edge in basis points
    let edge_bps = ((current_mid - past_mid) / past_mid) * 10_000.0;

    assert!((edge_bps - 15.0).abs() < 0.1);

    // Should trigger if edge >= min_edge_bps
    let min_edge_bps = 10.0;
    assert!(edge_bps >= min_edge_bps);
}

/// Test spread calculation
#[test]
fn test_spread_calculation() {
    let bid: f64 = 100.0;
    let ask: f64 = 100.05;
    let mid = (bid + ask) / 2.0;

    let spread_bps = ((ask - bid) / mid) * 10_000.0;

    assert!((spread_bps - 5.0).abs() < 0.1);

    // Should trade if spread <= max_spread_bps
    let max_spread_bps = 10.0;
    assert!(spread_bps <= max_spread_bps);
}

/// Test concurrent event publishing
#[tokio::test]
async fn test_concurrent_event_publishing() {
    use tokio::task;

    let bus = EventBus::new(1000);
    let mut handles = vec![];

    // Spawn multiple tasks publishing events
    for i in 0..10 {
        let bus_clone = bus.clone();
        let handle = task::spawn(async move {
            for j in 0..10 {
                let event = Event::Market(MarketEvent::Quote {
                    symbol: format!("SYM{}/USD", i),
                    bid: (j as f64) * 100.0,
                    ask: (j as f64) * 100.0 + 1.0,
                    timestamp: format!("2025-01-01T00:00:{:02}Z", j),
                });
                let _ = bus_clone.publish(event);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    // Bus should handle all events without panic
}

/// Test position lifecycle
#[test]
fn test_position_lifecycle() {
    let tracker = PositionTracker::new();

    // 1. Create pending buy order
    let order = PendingOrder {
        order_id: "buy123".to_string(),
        symbol: "XRP/USD".to_string(),
        side: "buy".to_string(),
        limit_price: 0.50,
        qty: 1000.0,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        stop_loss: Some(0.48),
        take_profit: Some(0.52),
        last_check_time: None,
    };
    tracker.add_pending_order(order);

    // 2. Order fills, create position
    tracker.remove_pending_order("buy123");
    let position = PositionInfo {
        symbol: "XRP/USD".to_string(),
        entry_price: 0.50,
        qty: 1000.0,
        stop_loss: 0.48,
        take_profit: 0.52,
        entry_time: "2025-01-01T00:00:00Z".to_string(),
        side: "buy".to_string(),
        is_closing: false,
        open_order_id: None,
        last_recreate_attempt: None,
        recreate_attempts: 0,
        highest_price: 0.50,
        trailing_stop_active: false,
        trailing_stop_price: 0.48,
    };
    tracker.add_position(position);

    // 3. Create TP sell order
    let tp_order = PendingOrder {
        order_id: "sell456".to_string(),
        symbol: "XRP/USD".to_string(),
        side: "sell".to_string(),
        limit_price: 0.52,
        qty: 1000.0,
        created_at: "2025-01-01T00:01:00Z".to_string(),
        stop_loss: None,
        take_profit: None,
        last_check_time: None,
    };
    tracker.add_pending_order(tp_order);

    // 4. TP fills, close position
    tracker.remove_pending_order("sell456");
    tracker.remove_position("XRP/USD");

    // Final state: no positions, no orders
    assert!(!tracker.has_position("XRP/USD"));
    assert!(tracker.get_all_pending_orders().is_empty());
}
