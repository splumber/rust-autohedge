//! Unit tests for the reporting module - trade logging and performance tracking.

#[cfg(test)]
mod reporting_tests {
    use crate::services::reporting::*;

    // ============= PerformanceSummary Tests =============

    #[test]
    fn test_performance_summary_default() {
        let summary = PerformanceSummary::default();
        assert_eq!(summary.total_orders, 0);
        assert_eq!(summary.total_exec_reports, 0);
        assert_eq!(summary.buys, 0);
        assert_eq!(summary.sells, 0);
        assert_eq!(summary.filled, 0);
        assert_eq!(summary.rejected, 0);
        assert_eq!(summary.total_notional, 0.0);
        assert_eq!(summary.total_realized_pnl, 0.0);
        assert_eq!(summary.winning_trades, 0);
        assert_eq!(summary.losing_trades, 0);
    }

    #[test]
    fn test_performance_summary_with_data() {
        let mut summary = PerformanceSummary::default();
        summary.total_orders = 100;
        summary.buys = 60;
        summary.sells = 40;
        summary.filled = 95;
        summary.rejected = 5;
        summary.total_notional = 50000.0;
        summary.winning_trades = 30;
        summary.losing_trades = 10;
        summary.total_profit = 500.0;
        summary.total_loss = 200.0;
        summary.total_realized_pnl = 300.0;

        assert_eq!(summary.total_orders, 100);
        assert_eq!(summary.winning_trades + summary.losing_trades, 40);
    }

    #[test]
    fn test_compute_stats_no_trades() {
        let summary = PerformanceSummary::default();
        let stats = summary.compute_stats();

        assert_eq!(stats.total_closed_trades, 0);
        assert_eq!(stats.win_rate_pct, 0.0);
        assert_eq!(stats.trades_per_hour, 0.0);
        assert_eq!(stats.avg_profit_per_trade, 0.0);
        assert_eq!(stats.profit_factor, 0.0);
    }

    #[test]
    fn test_compute_stats_with_trades() {
        let mut summary = PerformanceSummary::default();
        summary.winning_trades = 7;
        summary.losing_trades = 3;
        summary.total_profit = 700.0;
        summary.total_loss = 300.0;
        summary.total_realized_pnl = 400.0;

        let stats = summary.compute_stats();

        assert_eq!(stats.total_closed_trades, 10);
        assert!((stats.win_rate_pct - 70.0).abs() < 0.01);
        assert!((stats.profit_factor - 2.333).abs() < 0.01);
        assert!((stats.avg_profit_per_trade - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_stats_all_wins() {
        let mut summary = PerformanceSummary::default();
        summary.winning_trades = 10;
        summary.losing_trades = 0;
        summary.total_profit = 1000.0;
        summary.total_loss = 0.0;
        summary.total_realized_pnl = 1000.0;

        let stats = summary.compute_stats();

        assert_eq!(stats.win_rate_pct, 100.0);
        assert!(stats.profit_factor.is_infinite()); // No losses
    }

    #[test]
    fn test_compute_stats_all_losses() {
        let mut summary = PerformanceSummary::default();
        summary.winning_trades = 0;
        summary.losing_trades = 10;
        summary.total_profit = 0.0;
        summary.total_loss = 500.0;
        summary.total_realized_pnl = -500.0;

        let stats = summary.compute_stats();

        assert_eq!(stats.win_rate_pct, 0.0);
        assert_eq!(stats.profit_factor, 0.0);
    }

    #[test]
    fn test_compute_stats_with_start_time() {
        let mut summary = PerformanceSummary::default();
        // Set start time to 30 minutes ago
        let start = chrono::Utc::now() - chrono::Duration::minutes(30);
        summary.start_time = Some(start.to_rfc3339());
        summary.winning_trades = 6;
        summary.losing_trades = 4;

        let stats = summary.compute_stats();

        // Should have ~30 minutes runtime
        assert!(stats.runtime_minutes >= 29.0 && stats.runtime_minutes <= 31.0);
        // 10 trades in 0.5 hours = 20 trades/hour
        assert!(stats.trades_per_hour >= 19.0 && stats.trades_per_hour <= 21.0);
    }

    // ============= ClosedTrade Tests =============

    #[test]
    fn test_closed_trade_profit() {
        let trade = ClosedTrade {
            symbol: "BTC/USD".to_string(),
            buy_time: "2025-01-01T00:00:00Z".to_string(),
            sell_time: "2025-01-01T01:00:00Z".to_string(),
            buy_price: 50000.0,
            sell_price: 51000.0,
            qty: 0.1,
            pnl: 100.0,  // (51000 - 50000) * 0.1
            pnl_percent: 2.0,
        };

        assert_eq!(trade.pnl, 100.0);
        assert_eq!(trade.pnl_percent, 2.0);
    }

    #[test]
    fn test_closed_trade_loss() {
        let trade = ClosedTrade {
            symbol: "ETH/USD".to_string(),
            buy_time: "2025-01-01T00:00:00Z".to_string(),
            sell_time: "2025-01-01T01:00:00Z".to_string(),
            buy_price: 3000.0,
            sell_price: 2900.0,
            qty: 1.0,
            pnl: -100.0,
            pnl_percent: -3.33,
        };

        assert!(trade.pnl < 0.0);
        assert!(trade.pnl_percent < 0.0);
    }

    // ============= OpenPosition Tests =============

    #[test]
    fn test_open_position() {
        let pos = OpenPosition {
            symbol: "SOL/USD".to_string(),
            buy_time: "2025-01-01T00:00:00Z".to_string(),
            buy_price: 100.0,
            qty: 10.0,
        };

        assert_eq!(pos.symbol, "SOL/USD");
        assert_eq!(pos.buy_price, 100.0);
        assert_eq!(pos.qty, 10.0);
    }

    // ============= TradeLogEntry Tests =============

    #[test]
    fn test_trade_log_entry_buy() {
        let entry = TradeLogEntry {
            ts: "2025-01-01T00:00:00Z".to_string(),
            symbol: "DOGE/USD".to_string(),
            action: "buy".to_string(),
            order_id: "order123".to_string(),
            status: "filled".to_string(),
            qty: Some(10000.0),
            price: Some(0.08),
            notional: Some(800.0),
            notes: Some("HFT entry".to_string()),
        };

        assert_eq!(entry.action, "buy");
        assert_eq!(entry.status, "filled");
        assert_eq!(entry.notional, Some(800.0));
    }

    #[test]
    fn test_trade_log_entry_sell() {
        let entry = TradeLogEntry {
            ts: "2025-01-01T00:00:00Z".to_string(),
            symbol: "XRP/USD".to_string(),
            action: "sell".to_string(),
            order_id: "order456".to_string(),
            status: "new".to_string(),
            qty: Some(1000.0),
            price: Some(0.55),
            notional: Some(550.0),
            notes: None,
        };

        assert_eq!(entry.action, "sell");
        assert!(entry.notes.is_none());
    }

    #[test]
    fn test_trade_log_entry_rejected() {
        let entry = TradeLogEntry {
            ts: "2025-01-01T00:00:00Z".to_string(),
            symbol: "LTC/USD".to_string(),
            action: "buy".to_string(),
            order_id: "order789".to_string(),
            status: "rejected".to_string(),
            qty: None,
            price: None,
            notional: None,
            notes: Some("Insufficient funds".to_string()),
        };

        assert_eq!(entry.status, "rejected");
        assert!(entry.qty.is_none());
    }

    // ============= Serialization Tests =============

    #[test]
    fn test_performance_summary_serialization() {
        let mut summary = PerformanceSummary::default();
        summary.total_orders = 50;
        summary.buys = 30;
        summary.sells = 20;

        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"total_orders\":50"));
        assert!(json.contains("\"buys\":30"));
    }

    #[test]
    fn test_performance_summary_deserialization() {
        let json = r#"{
            "start_time": null,
            "total_orders": 100,
            "total_exec_reports": 95,
            "buys": 50,
            "sells": 45,
            "filled": 90,
            "rejected": 5,
            "total_notional": 10000.0,
            "per_symbol": {},
            "history": {},
            "open_positions": {},
            "total_realized_pnl": 500.0,
            "winning_trades": 30,
            "losing_trades": 15,
            "total_profit": 800.0,
            "total_loss": 300.0
        }"#;

        let summary: PerformanceSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.total_orders, 100);
        assert_eq!(summary.winning_trades, 30);
    }

    #[test]
    fn test_closed_trade_serialization() {
        let trade = ClosedTrade {
            symbol: "BTC/USD".to_string(),
            buy_time: "2025-01-01T00:00:00Z".to_string(),
            sell_time: "2025-01-01T01:00:00Z".to_string(),
            buy_price: 50000.0,
            sell_price: 51000.0,
            qty: 0.1,
            pnl: 100.0,
            pnl_percent: 2.0,
        };

        let json = serde_json::to_string(&trade).unwrap();
        assert!(json.contains("BTC/USD"));
        assert!(json.contains("51000"));
    }

    // ============= ComputedStats Tests =============

    #[test]
    fn test_computed_stats_struct() {
        let stats = ComputedStats {
            runtime_minutes: 120.0,
            trades_per_hour: 25.0,
            win_rate_pct: 60.0,
            avg_profit_per_trade: 5.0,
            profit_factor: 1.5,
            total_closed_trades: 50,
            open_position_count: 3,
        };

        assert_eq!(stats.runtime_minutes, 120.0);
        assert_eq!(stats.total_closed_trades, 50);
        assert_eq!(stats.open_position_count, 3);
    }

    // ============= Per-Symbol Tracking Tests =============

    #[test]
    fn test_per_symbol_counts() {
        let mut summary = PerformanceSummary::default();
        *summary.per_symbol.entry("BTC/USD".to_string()).or_insert(0) += 5;
        *summary.per_symbol.entry("ETH/USD".to_string()).or_insert(0) += 3;
        *summary.per_symbol.entry("BTC/USD".to_string()).or_insert(0) += 2;

        assert_eq!(summary.per_symbol.get("BTC/USD"), Some(&7));
        assert_eq!(summary.per_symbol.get("ETH/USD"), Some(&3));
    }

    #[test]
    fn test_history_tracking() {
        let mut summary = PerformanceSummary::default();
        
        let trade1 = ClosedTrade {
            symbol: "SOL/USD".to_string(),
            buy_time: "2025-01-01T00:00:00Z".to_string(),
            sell_time: "2025-01-01T01:00:00Z".to_string(),
            buy_price: 100.0,
            sell_price: 101.0,
            qty: 1.0,
            pnl: 1.0,
            pnl_percent: 1.0,
        };

        summary.history.entry("SOL/USD".to_string()).or_default().push(trade1);

        assert_eq!(summary.history.get("SOL/USD").unwrap().len(), 1);
    }

    #[test]
    fn test_open_positions_tracking() {
        let mut summary = PerformanceSummary::default();
        
        summary.open_positions.insert(
            "DOT/USD".to_string(),
            OpenPosition {
                symbol: "DOT/USD".to_string(),
                buy_time: "2025-01-01T00:00:00Z".to_string(),
                buy_price: 5.0,
                qty: 100.0,
            },
        );

        let stats = summary.compute_stats();
        assert_eq!(stats.open_position_count, 1);
    }
}

