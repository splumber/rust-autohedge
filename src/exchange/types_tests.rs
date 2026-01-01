//! Unit tests for exchange types and symbol conversion utilities.

#[cfg(test)]
mod types_tests {
    use crate::exchange::types::*;
    use serde_json::json;

    // ============= AccountSummary Tests =============

    #[test]
    fn test_account_summary_full() {
        let summary = AccountSummary {
            buying_power: Some(10000.0),
            cash: Some(5000.0),
            portfolio_value: Some(15000.0),
        };
        assert_eq!(summary.buying_power, Some(10000.0));
        assert_eq!(summary.cash, Some(5000.0));
        assert_eq!(summary.portfolio_value, Some(15000.0));
    }

    #[test]
    fn test_account_summary_partial() {
        let summary = AccountSummary {
            buying_power: None,
            cash: Some(5000.0),
            portfolio_value: None,
        };
        assert_eq!(summary.buying_power, None);
        assert_eq!(summary.cash, Some(5000.0));
    }

    #[test]
    fn test_account_summary_serialization() {
        let summary = AccountSummary {
            buying_power: Some(10000.0),
            cash: Some(5000.0),
            portfolio_value: Some(15000.0),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("buying_power"));
        assert!(json.contains("10000"));
    }

    // ============= Position Tests =============

    #[test]
    fn test_position_creation() {
        let pos = Position {
            symbol: "BTC/USD".to_string(),
            qty: 0.5,
            avg_entry_price: Some(50000.0),
        };
        assert_eq!(pos.symbol, "BTC/USD");
        assert_eq!(pos.qty, 0.5);
        assert_eq!(pos.avg_entry_price, Some(50000.0));
    }

    #[test]
    fn test_position_without_entry_price() {
        let pos = Position {
            symbol: "ETH/USD".to_string(),
            qty: 2.0,
            avg_entry_price: None,
        };
        assert_eq!(pos.avg_entry_price, None);
    }

    // ============= Side Tests =============

    #[test]
    fn test_side_buy() {
        let side = Side::Buy;
        let json = serde_json::to_string(&side).unwrap();
        assert_eq!(json, "\"buy\"");
    }

    #[test]
    fn test_side_sell() {
        let side = Side::Sell;
        let json = serde_json::to_string(&side).unwrap();
        assert_eq!(json, "\"sell\"");
    }

    #[test]
    fn test_side_deserialize() {
        let buy: Side = serde_json::from_str("\"buy\"").unwrap();
        let sell: Side = serde_json::from_str("\"sell\"").unwrap();
        assert!(matches!(buy, Side::Buy));
        assert!(matches!(sell, Side::Sell));
    }

    // ============= OrderType Tests =============

    #[test]
    fn test_order_type_market() {
        let ot = OrderType::Market;
        let json = serde_json::to_string(&ot).unwrap();
        assert_eq!(json, "\"market\"");
    }

    #[test]
    fn test_order_type_limit() {
        let ot = OrderType::Limit;
        let json = serde_json::to_string(&ot).unwrap();
        assert_eq!(json, "\"limit\"");
    }

    // ============= TimeInForce Tests =============

    #[test]
    fn test_tif_day() {
        let tif = TimeInForce::Day;
        let json = serde_json::to_string(&tif).unwrap();
        assert_eq!(json, "\"day\"");
    }

    #[test]
    fn test_tif_gtc() {
        let tif = TimeInForce::Gtc;
        let json = serde_json::to_string(&tif).unwrap();
        assert_eq!(json, "\"gtc\"");
    }

    #[test]
    fn test_tif_ioc() {
        let tif = TimeInForce::Ioc;
        let json = serde_json::to_string(&tif).unwrap();
        assert_eq!(json, "\"ioc\"");
    }

    // ============= PlaceOrderRequest Tests =============

    #[test]
    fn test_place_order_request_market_buy() {
        let req = PlaceOrderRequest {
            symbol: "BTC/USD".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            qty: Some(0.1),
            notional: None,
            limit_price: None,
            time_in_force: TimeInForce::Gtc,
        };
        assert_eq!(req.symbol, "BTC/USD");
        assert!(matches!(req.side, Side::Buy));
        assert!(matches!(req.order_type, OrderType::Market));
        assert_eq!(req.qty, Some(0.1));
        assert_eq!(req.notional, None);
    }

    #[test]
    fn test_place_order_request_limit_sell() {
        let req = PlaceOrderRequest {
            symbol: "ETH/USD".to_string(),
            side: Side::Sell,
            order_type: OrderType::Limit,
            qty: Some(1.0),
            notional: None,
            limit_price: Some(3500.0),
            time_in_force: TimeInForce::Day,
        };
        assert!(matches!(req.side, Side::Sell));
        assert!(matches!(req.order_type, OrderType::Limit));
        assert_eq!(req.limit_price, Some(3500.0));
    }

    #[test]
    fn test_place_order_request_notional() {
        let req = PlaceOrderRequest {
            symbol: "SOL/USD".to_string(),
            side: Side::Buy,
            order_type: OrderType::Market,
            qty: None,
            notional: Some(100.0),
            limit_price: None,
            time_in_force: TimeInForce::Ioc,
        };
        assert_eq!(req.qty, None);
        assert_eq!(req.notional, Some(100.0));
    }

    // ============= OrderAck Tests =============

    #[test]
    fn test_order_ack() {
        let ack = OrderAck {
            id: "order123".to_string(),
            status: "filled".to_string(),
            raw: json!({"filled_qty": 0.1}),
        };
        assert_eq!(ack.id, "order123");
        assert_eq!(ack.status, "filled");
    }

    // ============= ExchangeCapabilities Tests =============

    #[test]
    fn test_exchange_capabilities_full_featured() {
        let caps = ExchangeCapabilities {
            supports_notional_market_buy: true,
            supports_ws_quotes: true,
            supports_ws_trades: true,
            supports_news: true,
        };
        assert!(caps.supports_notional_market_buy);
        assert!(caps.supports_ws_quotes);
        assert!(caps.supports_ws_trades);
        assert!(caps.supports_news);
    }

    #[test]
    fn test_exchange_capabilities_limited() {
        let caps = ExchangeCapabilities {
            supports_notional_market_buy: false,
            supports_ws_quotes: true,
            supports_ws_trades: true,
            supports_news: false,
        };
        assert!(!caps.supports_notional_market_buy);
        assert!(!caps.supports_news);
    }
}

#[cfg(test)]
mod symbols_tests {
    use crate::exchange::symbols::*;

    // ============= Coinbase Symbol Conversion =============

    #[test]
    fn test_to_coinbase_btc() {
        let result = to_coinbase_product_id("BTC/USD");
        assert_eq!(result, "BTC-USD");
    }

    #[test]
    fn test_to_coinbase_eth() {
        let result = to_coinbase_product_id("ETH/USD");
        assert_eq!(result, "ETH-USD");
    }

    #[test]
    fn test_to_coinbase_various() {
        assert_eq!(to_coinbase_product_id("SOL/USD"), "SOL-USD");
        assert_eq!(to_coinbase_product_id("DOGE/USD"), "DOGE-USD");
        assert_eq!(to_coinbase_product_id("XRP/USD"), "XRP-USD");
    }

    // ============= Kraken Symbol Conversion =============

    #[test]
    fn test_to_kraken_btc() {
        let result = to_kraken_pair("BTC/USD");
        assert_eq!(result, "XBT/USD");
    }

    #[test]
    fn test_to_kraken_eth_unchanged() {
        let result = to_kraken_pair("ETH/USD");
        assert_eq!(result, "ETH/USD"); // ETH stays as ETH
    }

    #[test]
    fn test_to_kraken_various() {
        assert_eq!(to_kraken_pair("SOL/USD"), "SOL/USD");
        assert_eq!(to_kraken_pair("DOGE/USD"), "DOGE/USD");
    }

    // ============= Binance Symbol Conversion =============

    #[test]
    fn test_to_binance_btc() {
        let result = to_binance_stream_symbol("BTC/USD");
        assert_eq!(result, "btcusd");
    }

    #[test]
    fn test_to_binance_eth() {
        let result = to_binance_stream_symbol("ETH/USD");
        assert_eq!(result, "ethusd");
    }

    #[test]
    fn test_to_binance_lowercase() {
        // Binance uses lowercase
        let result = to_binance_stream_symbol("DOGE/USD");
        assert_eq!(result, "dogeusd");
        assert!(result.chars().all(|c| c.is_lowercase() || c.is_numeric()));
    }
}
