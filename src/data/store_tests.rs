//! Unit tests for MarketStore - the in-memory market data store.

#[cfg(test)]
mod store_tests {
    use crate::data::store::{Bar, MarketStore, Quote, Trade};

    #[test]
    fn test_market_store_new() {
        let store = MarketStore::new(100);
        assert_eq!(store.limit, 100);
    }

    #[test]
    fn test_update_and_get_quote() {
        let store = MarketStore::new(100);

        let quote = Quote {
            symbol: "BTC/USD".to_string(),
            bid_price: 50000.0,
            ask_price: 50001.0,
            bid_size: 1.5,
            ask_size: 2.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        store.update_quote("BTC/USD".to_string(), quote.clone());

        let latest = store.get_latest_quote("BTC/USD");
        assert!(latest.is_some());
        let latest = latest.unwrap();
        assert_eq!(latest.bid_price, 50000.0);
        assert_eq!(latest.ask_price, 50001.0);
    }

    #[test]
    fn test_quote_history() {
        let store = MarketStore::new(100);

        for i in 0..5 {
            let quote = Quote {
                symbol: "ETH/USD".to_string(),
                bid_price: 3000.0 + i as f64,
                ask_price: 3001.0 + i as f64,
                bid_size: 1.0,
                ask_size: 1.0,
                timestamp: format!("2025-01-01T00:00:0{}Z", i),
            };
            store.update_quote("ETH/USD".to_string(), quote);
        }

        let history = store.get_quote_history("ETH/USD");
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].bid_price, 3000.0);
        assert_eq!(history[4].bid_price, 3004.0);
    }

    #[test]
    fn test_quote_limit_enforcement() {
        let store = MarketStore::new(3); // Small limit

        for i in 0..5 {
            let quote = Quote {
                symbol: "SOL/USD".to_string(),
                bid_price: 100.0 + i as f64,
                ask_price: 101.0 + i as f64,
                bid_size: 1.0,
                ask_size: 1.0,
                timestamp: format!("2025-01-01T00:00:0{}Z", i),
            };
            store.update_quote("SOL/USD".to_string(), quote);
        }

        let history = store.get_quote_history("SOL/USD");
        assert_eq!(history.len(), 3); // Should be capped at limit
                                      // Should have the latest 3
        assert_eq!(history[0].bid_price, 102.0);
        assert_eq!(history[2].bid_price, 104.0);
    }

    #[test]
    fn test_update_and_get_trade() {
        let store = MarketStore::new(100);

        let trade = Trade {
            symbol: "DOGE/USD".to_string(),
            price: 0.08,
            size: 10000.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            id: Some(12345),
        };

        store.update_trade("DOGE/USD".to_string(), trade);

        let history = store.get_trade_history("DOGE/USD");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].price, 0.08);
        assert_eq!(history[0].size, 10000.0);
    }

    #[test]
    fn test_trade_history_limit() {
        let store = MarketStore::new(5);

        for i in 0..10 {
            let trade = Trade {
                symbol: "XRP/USD".to_string(),
                price: 0.5 + (i as f64 * 0.01),
                size: 1000.0,
                timestamp: format!("2025-01-01T00:00:{:02}Z", i),
                id: Some(i as u64),
            };
            store.update_trade("XRP/USD".to_string(), trade);
        }

        let history = store.get_trade_history("XRP/USD");
        assert_eq!(history.len(), 5);
        // Should have trades 5-9 (latest 5)
        assert_eq!(history[0].id, Some(5));
        assert_eq!(history[4].id, Some(9));
    }

    #[test]
    fn test_update_and_get_bar() {
        let store = MarketStore::new(100);

        let bar = Bar {
            symbol: "LTC/USD".to_string(),
            open: 80.0,
            high: 85.0,
            low: 78.0,
            close: 82.0,
            volume: 50000.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        store.update_bar("LTC/USD".to_string(), bar);

        let latest = store.get_latest_bar("LTC/USD");
        assert!(latest.is_some());
        let latest = latest.unwrap();
        assert_eq!(latest.open, 80.0);
        assert_eq!(latest.high, 85.0);
        assert_eq!(latest.close, 82.0);
    }

    #[test]
    fn test_bar_history() {
        let store = MarketStore::new(100);

        for i in 0..3 {
            let bar = Bar {
                symbol: "DOT/USD".to_string(),
                open: 5.0 + i as f64,
                high: 6.0 + i as f64,
                low: 4.0 + i as f64,
                close: 5.5 + i as f64,
                volume: 10000.0,
                timestamp: format!("2025-01-01T0{}:00:00Z", i),
            };
            store.update_bar("DOT/USD".to_string(), bar);
        }

        let history = store.get_bar_history("DOT/USD");
        assert_eq!(history.len(), 3);

        // Also test get_history alias
        let history2 = store.get_history("DOT/USD");
        assert_eq!(history2.len(), 3);
    }

    #[test]
    fn test_multiple_symbols() {
        let store = MarketStore::new(100);

        let quote1 = Quote {
            symbol: "BTC/USD".to_string(),
            bid_price: 50000.0,
            ask_price: 50001.0,
            bid_size: 1.0,
            ask_size: 1.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        let quote2 = Quote {
            symbol: "ETH/USD".to_string(),
            bid_price: 3000.0,
            ask_price: 3001.0,
            bid_size: 2.0,
            ask_size: 2.0,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        };

        store.update_quote("BTC/USD".to_string(), quote1);
        store.update_quote("ETH/USD".to_string(), quote2);

        let btc = store.get_latest_quote("BTC/USD").unwrap();
        let eth = store.get_latest_quote("ETH/USD").unwrap();

        assert_eq!(btc.bid_price, 50000.0);
        assert_eq!(eth.bid_price, 3000.0);
    }

    #[test]
    fn test_nonexistent_symbol() {
        let store = MarketStore::new(100);

        let quote = store.get_latest_quote("NONEXISTENT/USD");
        assert!(quote.is_none());

        let bar = store.get_latest_bar("NONEXISTENT/USD");
        assert!(bar.is_none());

        let history = store.get_quote_history("NONEXISTENT/USD");
        assert!(history.is_empty());
    }

    #[test]
    fn test_add_and_get_news() {
        let store = MarketStore::new(100);

        let news1 = serde_json::json!({
            "headline": "Bitcoin hits new high",
            "source": "CryptoNews"
        });

        let news2 = serde_json::json!({
            "headline": "Ethereum upgrade announced",
            "source": "BlockchainDaily"
        });

        store.add_news(news1);
        store.add_news(news2);

        let news = store.get_latest_news();
        assert_eq!(news.len(), 2);
        assert_eq!(news[0]["headline"], "Bitcoin hits new high");
        assert_eq!(news[1]["headline"], "Ethereum upgrade announced");
    }

    #[test]
    fn test_news_limit_enforcement() {
        let store = MarketStore::new(3);

        for i in 0..5 {
            let news = serde_json::json!({
                "headline": format!("News {}", i),
            });
            store.add_news(news);
        }

        let news = store.get_latest_news();
        assert_eq!(news.len(), 3);
        // Should have news 2, 3, 4 (oldest removed)
        assert_eq!(news[0]["headline"], "News 2");
        assert_eq!(news[2]["headline"], "News 4");
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let store = Arc::new(MarketStore::new(1000));
        let mut handles = vec![];

        // Spawn multiple threads updating quotes
        for i in 0..10 {
            let store_clone = Arc::clone(&store);
            let handle = thread::spawn(move || {
                for j in 0..100 {
                    let quote = Quote {
                        symbol: format!("SYM{}/USD", i),
                        bid_price: j as f64,
                        ask_price: (j + 1) as f64,
                        bid_size: 1.0,
                        ask_size: 1.0,
                        timestamp: format!("2025-01-01T00:00:{:02}Z", j % 60),
                    };
                    store_clone.update_quote(format!("SYM{}/USD", i), quote);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all symbols have data
        for i in 0..10 {
            let history = store.get_quote_history(&format!("SYM{}/USD", i));
            assert_eq!(history.len(), 100);
        }
    }
}
