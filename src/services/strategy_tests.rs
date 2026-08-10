//! Unit tests for the pure HFT entry-decision function `evaluate_hft_signal`.
//!
//! This is the single source of truth shared by the live strategy loop and the
//! backtester, so these tests are the main guardrail against the entry math drifting.

#[cfg(test)]
mod strategy_tests {
    use crate::config::{FeeConfig, HftConfig};
    use crate::services::strategy::{
        evaluate_bollinger_signal, evaluate_donchian_breakout_signal, evaluate_hft_signal,
        evaluate_ma_crossover_signal, evaluate_mean_reversion_signal, evaluate_rsi_signal,
        evaluate_trend_filtered_momentum_signal, evaluate_vwap_signal, HftSignal, SignalDecision,
    };
    use std::collections::VecDeque;

    fn hft_config() -> HftConfig {
        HftConfig {
            evaluate_every_quotes: 1,
            min_edge_bps: 3.0,
            take_profit_bps: 180.0,
            stop_loss_bps: 100.0,
            max_spread_bps: 50.0,
            min_volume_ratio: 0.3,
            use_vwap_filter: false,
            momentum_lookback: 10,
            min_net_edge_bps: 5.0,
        }
    }

    fn fee_config() -> FeeConfig {
        FeeConfig {
            round_trip_fee_bps: 50.0,
        }
    }

    /// No trade-volume history - `volume_ratio()` returns `None` for this, so the volume
    /// gate fails open and doesn't affect tests exercising the other gates.
    fn no_volumes() -> VecDeque<f64> {
        VecDeque::new()
    }

    /// Builds a mids history ending in `current_mid`, with `past_mid` exactly
    /// `lookback` steps behind it - mirrors how `evaluate_hft` pushes then reads back.
    fn mids_with_lookback(past_mid: f64, current_mid: f64, lookback: usize) -> VecDeque<f64> {
        let mut mids = VecDeque::new();
        for _ in 0..lookback {
            mids.push_back(past_mid);
        }
        mids.push_back(current_mid);
        mids
    }

    #[test]
    fn test_insufficient_history_returns_early() {
        let hft = hft_config();
        let fees = fee_config();
        let mids = VecDeque::from([100.0]); // only the current tick, no lookback available

        let result = evaluate_hft_signal(100.0, 100.01, &mids, &no_volumes(), &hft, &fees);
        assert_eq!(result, HftSignal::InsufficientHistory);
    }

    #[test]
    fn test_spread_too_wide_skips_before_momentum() {
        let hft = hft_config();
        let fees = fee_config();
        let mids = mids_with_lookback(100.0, 101.0, 10);

        // 100 bps spread > max_spread_bps of 50
        let result = evaluate_hft_signal(100.0, 101.0, &mids, &no_volumes(), &hft, &fees);
        match result {
            HftSignal::SpreadTooWide { spread_bps } => assert!(spread_bps > hft.max_spread_bps),
            other => panic!("expected SpreadTooWide, got {:?}", other),
        }
    }

    #[test]
    fn test_edge_too_low_when_flat() {
        let hft = hft_config();
        let fees = fee_config();
        // No momentum: past == current mid.
        let mids = mids_with_lookback(100.0, 100.0, 10);

        let result = evaluate_hft_signal(99.995, 100.005, &mids, &no_volumes(), &hft, &fees);
        match result {
            HftSignal::EdgeTooLow { edge_bps } => assert!(edge_bps < hft.min_edge_bps),
            other => panic!("expected EdgeTooLow, got {:?}", other),
        }
    }

    #[test]
    fn test_net_edge_too_low_when_costs_exceed_raw_edge() {
        let hft = hft_config();
        let fees = fee_config();
        // Raw momentum clears min_edge_bps (3) but spread+fees dwarf it -
        // this is exactly the pre-fix failure mode: a "profitable-looking" raw edge
        // that's actually underwater once real trading costs are counted.
        let past = 100.0;
        let current_mid = past * 1.001; // 10 bps raw edge
        let mids = mids_with_lookback(past, current_mid, 10);

        // Construct bid/ask so their midpoint is exactly current_mid, ~40 bps spread.
        let half_spread = current_mid * 0.0020;
        let bid = current_mid - half_spread;
        let ask = current_mid + half_spread;

        let result = evaluate_hft_signal(bid, ask, &mids, &no_volumes(), &hft, &fees);
        match result {
            HftSignal::NetEdgeTooLow { net_edge_bps, .. } => {
                assert!(net_edge_bps < hft.min_net_edge_bps);
                assert!(net_edge_bps < 0.0);
            }
            other => panic!("expected NetEdgeTooLow, got {:?}", other),
        }
    }

    #[test]
    fn test_volume_too_low_blocks_even_with_good_edge() {
        let hft = hft_config();
        let fees = fee_config();
        // Strong momentum, tight spread (would otherwise Buy - see
        // test_buy_when_all_gates_clear), but the latest trade is much quieter than
        // recent average activity: not enough conviction behind the move to trust it.
        let past = 100.0;
        let current = past * 1.02;
        let mids = mids_with_lookback(past, current, 10);
        let bid = current - current * 0.0005;
        let ask = current;

        // Baseline trades of size 100, then a much smaller latest trade of size 5.
        let mut volumes = VecDeque::new();
        for _ in 0..5 {
            volumes.push_back(100.0);
        }
        volumes.push_back(5.0);

        let result = evaluate_hft_signal(bid, ask, &mids, &volumes, &hft, &fees);
        match result {
            HftSignal::VolumeTooLow { volume_ratio } => {
                assert!(volume_ratio < hft.min_volume_ratio);
            }
            other => panic!("expected VolumeTooLow, got {:?}", other),
        }
    }

    #[test]
    fn test_buy_when_volume_confirms_edge() {
        let hft = hft_config();
        let fees = fee_config();
        let past = 100.0;
        let current = past * 1.02;
        let mids = mids_with_lookback(past, current, 10);
        let bid = current - current * 0.0005;
        let ask = current;

        // Latest trade is in line with (here, above) recent average activity.
        let mut volumes = VecDeque::new();
        for _ in 0..5 {
            volumes.push_back(100.0);
        }
        volumes.push_back(150.0);

        let result = evaluate_hft_signal(bid, ask, &mids, &volumes, &hft, &fees);
        assert!(matches!(result, HftSignal::Buy { .. }));
    }

    #[test]
    fn test_buy_when_all_gates_clear() {
        let hft = hft_config();
        let fees = fee_config();
        // Strong momentum, tight spread - should clear every gate.
        let past = 100.0;
        let current = past * 1.02; // 200 bps raw edge
        let mids = mids_with_lookback(past, current, 10);

        let bid = current - current * 0.0005; // 5 bps spread
        let ask = current;

        let result = evaluate_hft_signal(bid, ask, &mids, &no_volumes(), &hft, &fees);
        match result {
            HftSignal::Buy {
                mid,
                take_profit,
                stop_loss,
                net_edge_bps,
                ..
            } => {
                assert!(net_edge_bps >= hft.min_net_edge_bps);
                assert!(take_profit > mid);
                assert!(stop_loss < mid);
            }
            other => panic!("expected Buy, got {:?}", other),
        }
    }

    #[test]
    fn test_mean_reversion_buys_on_dip_not_rise() {
        let hft = hft_config();
        let fees = fee_config();

        // A rise should NOT trigger the reversion strategy (that's the momentum strategy's job).
        let past = 100.0;
        let rise = past * 1.02;
        let rise_mids = mids_with_lookback(past, rise, 10);
        let rise_result = evaluate_mean_reversion_signal(
            rise - rise * 0.0005,
            rise,
            &rise_mids,
            &no_volumes(),
            &hft,
            &fees,
        );
        assert!(matches!(rise_result, HftSignal::EdgeTooLow { .. }));

        // A dip of the same magnitude SHOULD trigger it.
        let dip = past * 0.98;
        let dip_mids = mids_with_lookback(past, dip, 10);
        let dip_result = evaluate_mean_reversion_signal(
            dip - dip * 0.0005,
            dip,
            &dip_mids,
            &no_volumes(),
            &hft,
            &fees,
        );
        match dip_result {
            HftSignal::Buy { edge_bps, .. } => assert!(edge_bps > 0.0),
            other => panic!("expected Buy on a dip, got {:?}", other),
        }
    }

    #[test]
    fn test_lookback_respects_momentum_lookback_config() {
        // Regression test for the bug where momentum_lookback was defined in config but
        // never actually read (hardcoded to 10 instead). Build 11 ticks of history: index
        // 0 is a big outlier (90.0), indices 1..10 are flat at 100.0 (last index = 10).
        // - Configured lookback=3 (correct) reads index 10-3=7 -> 100.0 -> flat -> no edge.
        // - The old hardcoded-10 bug would read index 10-10=0 -> 90.0 -> huge apparent edge.
        // If this test ever starts seeing the 90.0 outlier, the hardcoding regressed.
        let mut hft = hft_config();
        hft.momentum_lookback = 3;
        let fees = fee_config();

        let mut mids = VecDeque::new();
        mids.push_back(90.0);
        for _ in 0..10 {
            mids.push_back(100.0);
        }
        assert_eq!(mids.len(), 11);

        let result = evaluate_hft_signal(99.995, 100.005, &mids, &no_volumes(), &hft, &fees);
        match result {
            HftSignal::EdgeTooLow { edge_bps } => assert!((edge_bps).abs() < 1.0),
            other => panic!(
                "expected EdgeTooLow (configured lookback=3 should ignore the 10-tick-old outlier), got {:?}",
                other
            ),
        }
    }

    // ============= RSI signal =============

    #[test]
    fn test_rsi_buys_when_oversold() {
        // Monotonically declining series -> RSI should read low (near 0).
        let closes: VecDeque<f64> = (0..20).map(|i| 100.0 - i as f64).collect();
        let result = evaluate_rsi_signal(99.995, 100.005, &closes, 14, 30.0, 50.0, 2.0, 1.0);
        assert!(matches!(result, SignalDecision::Buy { .. }));
    }

    #[test]
    fn test_rsi_no_signal_when_not_oversold() {
        // Monotonically rising series -> RSI reads high, nowhere near oversold.
        let closes: VecDeque<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
        let result = evaluate_rsi_signal(119.995, 120.005, &closes, 14, 30.0, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    #[test]
    fn test_rsi_no_signal_on_wide_spread() {
        let closes: VecDeque<f64> = (0..20).map(|i| 100.0 - i as f64).collect();
        // 200 bps spread > max_spread_bps of 50, even though RSI is oversold.
        let result = evaluate_rsi_signal(79.0, 81.0, &closes, 14, 30.0, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    // ============= Bollinger signal =============

    #[test]
    fn test_bollinger_buys_on_deep_dip_below_band() {
        // Flat history around 100, then a sharp one-tick drop far below the band.
        let mut closes: VecDeque<f64> = std::iter::repeat(100.0).take(19).collect();
        closes.push_back(90.0);
        let result = evaluate_bollinger_signal(89.995, 90.005, &closes, 20, 2.0, 50.0, 2.0, 1.0);
        match result {
            SignalDecision::Buy { mid, .. } => assert_eq!(mid, 90.0),
            other => panic!("expected Buy, got {:?}", other),
        }
    }

    #[test]
    fn test_bollinger_no_signal_within_band() {
        // A series with genuine variation (alternating +/-1 around 100, stddev = 1.0),
        // then a move that's within 2 stddev of the mean - shouldn't trigger.
        let mut closes: VecDeque<f64> = VecDeque::new();
        for i in 0..20 {
            closes.push_back(if i % 2 == 0 { 101.0 } else { 99.0 });
        }
        *closes.back_mut().unwrap() = 99.0; // last tick stays a normal -1 deviation, not a dip
        let result = evaluate_bollinger_signal(98.995, 99.005, &closes, 20, 2.0, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    #[test]
    fn test_bollinger_no_signal_zero_variance() {
        // Perfectly flat history -> stddev is 0 -> must not divide by zero / false-trigger.
        let closes: VecDeque<f64> = std::iter::repeat(100.0).take(20).collect();
        let result = evaluate_bollinger_signal(99.995, 100.005, &closes, 20, 2.0, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    // ============= MA crossover signal =============

    #[test]
    fn test_ma_crossover_buys_on_golden_cross() {
        // Fast(3)/slow(5) both start flat at 100 (equal), then a sharp final uptick pulls
        // the fast average above the slow average on this tick - a fresh crossover.
        let mut closes: VecDeque<f64> = std::iter::repeat(100.0).take(5).collect();
        closes.push_back(110.0);
        let result = evaluate_ma_crossover_signal(109.995, 110.005, &closes, 3, 5, 50.0, 2.0, 1.0);
        assert!(matches!(result, SignalDecision::Buy { .. }));
    }

    #[test]
    fn test_ma_crossover_no_signal_when_already_crossed() {
        // Fast already above slow on the PRIOR tick too - not a fresh cross this tick.
        let mut closes: VecDeque<f64> = std::iter::repeat(100.0).take(4).collect();
        closes.push_back(110.0);
        closes.push_back(110.5); // fast was already above slow before this tick
        let result =
            evaluate_ma_crossover_signal(110.495, 110.505, &closes, 3, 5, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    #[test]
    fn test_ma_crossover_no_signal_insufficient_history() {
        let closes: VecDeque<f64> = VecDeque::from([100.0, 101.0]);
        let result = evaluate_ma_crossover_signal(100.995, 101.005, &closes, 3, 5, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    // ============= Donchian breakout signal =============

    #[test]
    fn test_donchian_buys_on_new_high() {
        let mut closes: VecDeque<f64> = VecDeque::from([100.0, 102.0, 101.0, 99.0, 100.5]);
        closes.push_back(105.0); // breaks above the prior 5-period high of 102.0
        let result = evaluate_donchian_breakout_signal(104.995, 105.005, &closes, 5, 50.0, 2.0, 1.0);
        match result {
            SignalDecision::Buy { mid, .. } => assert_eq!(mid, 105.0),
            other => panic!("expected Buy, got {:?}", other),
        }
    }

    #[test]
    fn test_donchian_no_signal_without_breakout() {
        let mut closes: VecDeque<f64> = VecDeque::from([100.0, 102.0, 101.0, 99.0, 100.5]);
        closes.push_back(101.5); // below the prior 5-period high of 102.0
        let result = evaluate_donchian_breakout_signal(101.495, 101.505, &closes, 5, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    // ============= VWAP signal =============

    #[test]
    fn test_vwap_buys_below_volume_weighted_average() {
        // Heavy volume at 100 (the "real" price), then a light-volume dip to 95 - VWAP
        // stays near 100, so 95 reads as a meaningful dip below it.
        let mut closes = VecDeque::new();
        let mut volumes = VecDeque::new();
        for _ in 0..10 {
            closes.push_back(100.0);
            volumes.push_back(1000.0);
        }
        closes.push_back(95.0);
        volumes.push_back(10.0);
        let result = evaluate_vwap_signal(94.995, 95.005, &closes, &volumes, 11, 50.0, 50.0, 2.0, 1.0);
        assert!(matches!(result, SignalDecision::Buy { .. }));
    }

    #[test]
    fn test_vwap_no_signal_when_at_vwap() {
        let closes: VecDeque<f64> = std::iter::repeat(100.0).take(11).collect();
        let volumes: VecDeque<f64> = std::iter::repeat(1000.0).take(11).collect();
        let result = evaluate_vwap_signal(99.995, 100.005, &closes, &volumes, 11, 50.0, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    #[test]
    fn test_vwap_no_signal_mismatched_lengths() {
        let closes: VecDeque<f64> = VecDeque::from([100.0, 99.0]);
        let volumes: VecDeque<f64> = VecDeque::from([1.0]);
        let result = evaluate_vwap_signal(98.995, 99.005, &closes, &volumes, 2, 50.0, 50.0, 2.0, 1.0);
        assert_eq!(result, SignalDecision::NoSignal);
    }

    // ============= Trend-filtered momentum signal =============

    #[test]
    fn test_trend_filtered_momentum_buys_when_trend_agrees() {
        // Established uptrend (rising SMA), plus a fresh momentum burst on top of it.
        let mut closes = VecDeque::new();
        for i in 0..30 {
            closes.push_back(100.0 + i as f64 * 0.5); // steady uptrend, ends at 114.5
        }
        closes.push_back(120.0); // momentum burst clears min_edge_bps
        let result = evaluate_trend_filtered_momentum_signal(
            119.995, 120.005, &closes, 10, 3.0, 20, 50.0, 2.0, 1.0,
        );
        assert!(matches!(result, SignalDecision::Buy { .. }));
    }

    #[test]
    fn test_trend_filtered_momentum_blocks_against_downtrend() {
        // A steep 20-tick decline (300 -> 100), then a 10-tick bounce (100 -> 110) that's
        // entirely within the momentum lookback window - clears the raw edge_bps gate
        // (recent move is positive) but the 20-period trend SMA (~125.25, still dominated
        // by the decline) sits well above the current price (110), so this is a bounce
        // inside a downtrend, not a confirmed uptrend, and must be rejected.
        let mut closes = VecDeque::new();
        for i in 0..=20 {
            closes.push_back(300.0 - i as f64 * 10.0); // decline: 300 down to 100
        }
        for i in 1..=10 {
            closes.push_back(100.0 + i as f64); // bounce: 101 up to 110
        }
        let result = evaluate_trend_filtered_momentum_signal(
            109.995, 110.005, &closes, 10, 3.0, 20, 50.0, 2.0, 1.0,
        );
        assert_eq!(result, SignalDecision::NoSignal);
    }

    #[test]
    fn test_trend_filtered_momentum_blocks_without_edge() {
        // Flat series - no momentum at all, regardless of trend. The edge gate must
        // block this before the trend filter is even relevant.
        let closes: VecDeque<f64> = std::iter::repeat(110.0).take(31).collect();
        let result = evaluate_trend_filtered_momentum_signal(
            109.995, 110.005, &closes, 10, 3.0, 20, 50.0, 2.0, 1.0,
        );
        assert_eq!(result, SignalDecision::NoSignal);
    }
}
