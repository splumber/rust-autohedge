//! Pure technical-indicator math, shared by every signal in `strategy.rs` and the
//! backtester. No I/O, no config, no state - just numbers in, numbers out, so each
//! indicator can be trusted independently of any particular strategy's use of it.

/// Simple moving average of the last `period` values in `series` (`series` ordered
/// oldest-to-newest). `None` if there isn't enough history yet.
pub fn sma(series: &[f64], period: usize) -> Option<f64> {
    if period == 0 || series.len() < period {
        return None;
    }
    let window = &series[series.len() - period..];
    Some(window.iter().sum::<f64>() / period as f64)
}

/// Population standard deviation of the last `period` values in `series`.
pub fn stddev(series: &[f64], period: usize) -> Option<f64> {
    if period == 0 || series.len() < period {
        return None;
    }
    let mean = sma(series, period)?;
    let window = &series[series.len() - period..];
    let variance = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / period as f64;
    Some(variance.sqrt())
}

/// Wilder's RSI over the last `period` price changes (needs `period + 1` prices).
/// Returns a value in `[0, 100]`; `None` if there isn't enough history.
pub fn rsi(series: &[f64], period: usize) -> Option<f64> {
    if period == 0 || series.len() < period + 1 {
        return None;
    }
    let window = &series[series.len() - period - 1..];
    let mut gain_sum = 0.0;
    let mut loss_sum = 0.0;
    for pair in window.windows(2) {
        let change = pair[1] - pair[0];
        if change > 0.0 {
            gain_sum += change;
        } else {
            loss_sum += -change;
        }
    }
    let avg_gain = gain_sum / period as f64;
    let avg_loss = loss_sum / period as f64;
    if avg_loss == 0.0 {
        return Some(100.0);
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - (100.0 / (1.0 + rs)))
}

/// Highest value in the last `period` entries of `series`, **excluding** the most recent
/// entry - i.e. "the high to beat" for a breakout-above check on the current price.
pub fn prior_period_high(series: &[f64], period: usize) -> Option<f64> {
    if period == 0 || series.len() < period + 1 {
        return None;
    }
    let end = series.len() - 1; // exclude current
    let start = end - period;
    series[start..end].iter().cloned().fold(None, |acc, v| {
        Some(acc.map_or(v, |m: f64| m.max(v)))
    })
}

/// Lowest value in the last `period` entries of `series`, excluding the most recent entry.
pub fn prior_period_low(series: &[f64], period: usize) -> Option<f64> {
    if period == 0 || series.len() < period + 1 {
        return None;
    }
    let end = series.len() - 1;
    let start = end - period;
    series[start..end].iter().cloned().fold(None, |acc, v| {
        Some(acc.map_or(v, |m: f64| m.min(v)))
    })
}

/// Volume-weighted average price over the last `period` (price, volume) pairs -
/// `sum(price_i * volume_i) / sum(volume_i)`. `None` if there isn't enough history or
/// total volume in the window is zero.
pub fn vwap(prices: &[f64], volumes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || prices.len() != volumes.len() || prices.len() < period {
        return None;
    }
    let start = prices.len() - period;
    let mut pv_sum = 0.0;
    let mut v_sum = 0.0;
    for i in start..prices.len() {
        pv_sum += prices[i] * volumes[i];
        v_sum += volumes[i];
    }
    if v_sum <= 0.0 {
        return None;
    }
    Some(pv_sum / v_sum)
}

/// Ordinary least squares for `y = alpha + beta * x + residual`. Returns `(alpha, beta)`.
/// `None` if the series are mismatched, too short, or `x` has zero variance.
pub fn ols_fit(x: &[f64], y: &[f64]) -> Option<(f64, f64)> {
    if x.len() != y.len() || x.len() < 2 {
        return None;
    }
    let n = x.len() as f64;
    let mean_x = x.iter().sum::<f64>() / n;
    let mean_y = y.iter().sum::<f64>() / n;
    let mut cov_xy = 0.0;
    let mut var_x = 0.0;
    for i in 0..x.len() {
        cov_xy += (x[i] - mean_x) * (y[i] - mean_y);
        var_x += (x[i] - mean_x).powi(2);
    }
    if var_x <= 0.0 {
        return None;
    }
    let beta = cov_xy / var_x;
    let alpha = mean_y - beta * mean_x;
    Some((alpha, beta))
}

/// Residual series `y_i - (alpha + beta * x_i)` for a fitted OLS line.
pub fn ols_residuals(x: &[f64], y: &[f64], alpha: f64, beta: f64) -> Vec<f64> {
    x.iter()
        .zip(y.iter())
        .map(|(&xi, &yi)| yi - (alpha + beta * xi))
        .collect()
}

/// Empirical mean-reversion half-life of `series`, via an AR(1) regression of each step's
/// change against its prior level: `delta_t = c + lambda * level_(t-1)`. `None` if the
/// series doesn't show mean reversion (`lambda >= 0`, i.e. trending or a random walk) or
/// there isn't enough data.
///
/// This is a practical, lightweight substitute for a formal cointegration test (Engle-
/// Granger/ADF, which needs simulated critical-value tables not available without an
/// external stats crate): it answers the question a pairs strategy actually cares about -
/// does this spread revert, and how fast - directly from the data, without needing a
/// hypothesis-test p-value. It will accept some spreads a rigorous ADF test would reject
/// (and vice versa); treat it as a tradability heuristic, not a formal proof of
/// cointegration.
pub fn mean_reversion_half_life(series: &[f64]) -> Option<f64> {
    if series.len() < 10 {
        return None;
    }
    let levels = &series[..series.len() - 1];
    let deltas: Vec<f64> = series.windows(2).map(|w| w[1] - w[0]).collect();
    let (_, lambda) = ols_fit(levels, &deltas)?;
    // lambda must be strictly negative (mean-reverting) and not so extreme it implies
    // sub-one-period reversion, which is a sign of a numerically degenerate fit rather
    // than a real, tradable half-life.
    if !(-2.0..0.0).contains(&lambda) {
        return None;
    }
    let half_life = -(2.0_f64.ln()) / (1.0 + lambda).ln();
    if half_life.is_finite() && half_life > 0.0 {
        Some(half_life)
    } else {
        None
    }
}

/// Bollinger bandwidth: `(2 * num_sd * stddev) / mean`, a normalized measure of how
/// "squeezed" or "expanded" the bands are relative to price level (so it's comparable
/// across symbols at very different price scales, unlike raw stddev). Low bandwidth means
/// unusually low recent volatility (a "squeeze"), which technical-analysis folklore treats
/// as often preceding a sharp directional move - `evaluate_vol_squeeze_signal` in
/// `strategy.rs` trades that pattern.
pub fn bollinger_bandwidth(series: &[f64], period: usize, num_sd: f64) -> Option<f64> {
    let mean = sma(series, period)?;
    let sd = stddev(series, period)?;
    if mean <= 0.0 {
        return None;
    }
    Some((2.0 * num_sd * sd) / mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sma_basic() {
        let series = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(sma(&series, 3), Some((3.0 + 4.0 + 5.0) / 3.0));
    }

    #[test]
    fn test_sma_insufficient_history() {
        let series = [1.0, 2.0];
        assert_eq!(sma(&series, 5), None);
    }

    #[test]
    fn test_stddev_constant_series_is_zero() {
        let series = [5.0, 5.0, 5.0, 5.0];
        assert_eq!(stddev(&series, 4), Some(0.0));
    }

    #[test]
    fn test_stddev_known_value() {
        // [2, 4, 4, 4, 5, 5, 7, 9] has a textbook population stddev of 2.0.
        let series = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let sd = stddev(&series, series.len()).unwrap();
        assert!((sd - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_rsi_all_gains_is_100() {
        let series = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(rsi(&series, 5), Some(100.0));
    }

    #[test]
    fn test_rsi_all_losses_is_0() {
        let series = [6.0, 5.0, 4.0, 3.0, 2.0, 1.0];
        assert_eq!(rsi(&series, 5), Some(0.0));
    }

    #[test]
    fn test_rsi_flat_series_is_midpoint() {
        // No gains, no losses at all -> avg_loss is 0 -> defined as 100 by convention
        // (matches the common RSI convention: no losses means maximally "strong").
        let series = [5.0, 5.0, 5.0, 5.0];
        assert_eq!(rsi(&series, 3), Some(100.0));
    }

    #[test]
    fn test_rsi_insufficient_history() {
        let series = [1.0, 2.0];
        assert_eq!(rsi(&series, 5), None);
    }

    #[test]
    fn test_prior_period_high_excludes_current() {
        // Last element (100.0) must NOT count toward the "prior" high.
        let series = [1.0, 5.0, 3.0, 2.0, 100.0];
        assert_eq!(prior_period_high(&series, 3), Some(5.0));
    }

    #[test]
    fn test_prior_period_low_excludes_current() {
        let series = [5.0, 1.0, 3.0, 4.0, 0.1];
        assert_eq!(prior_period_low(&series, 3), Some(1.0));
    }

    #[test]
    fn test_ols_fit_recovers_known_line() {
        // y = 2 + 3x exactly, no noise - OLS must recover (2, 3) exactly.
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y: Vec<f64> = x.iter().map(|&xi| 2.0 + 3.0 * xi).collect();
        let (alpha, beta) = ols_fit(&x, &y).unwrap();
        assert!((alpha - 2.0).abs() < 1e-9);
        assert!((beta - 3.0).abs() < 1e-9);
    }

    #[test]
    fn test_ols_fit_zero_variance_x_is_none() {
        let x = [5.0, 5.0, 5.0];
        let y = [1.0, 2.0, 3.0];
        assert_eq!(ols_fit(&x, &y), None);
    }

    #[test]
    fn test_ols_residuals_are_zero_for_perfect_fit() {
        let x = [1.0, 2.0, 3.0];
        let y: Vec<f64> = x.iter().map(|&xi| 2.0 + 3.0 * xi).collect();
        let (alpha, beta) = ols_fit(&x, &y).unwrap();
        let resid = ols_residuals(&x, &y, alpha, beta);
        for r in resid {
            assert!(r.abs() < 1e-9);
        }
    }

    #[test]
    fn test_half_life_detects_mean_reverting_series() {
        // Synthetic AR(1): level_t = level_(t-1) * (1 + lambda), lambda = -0.1, so it
        // decays toward zero - a textbook mean-reverting series.
        let lambda = -0.1;
        let mut series = vec![100.0];
        for _ in 0..50 {
            let prev = *series.last().unwrap();
            series.push(prev * (1.0 + lambda));
        }
        let half_life = mean_reversion_half_life(&series).unwrap();
        let expected = -(2.0_f64.ln()) / (1.0 + lambda).ln();
        assert!((half_life - expected).abs() < 1e-6);
    }

    #[test]
    fn test_half_life_none_for_trending_series() {
        // Monotonically increasing, no reversion at all.
        let series: Vec<f64> = (0..30).map(|i| i as f64).collect();
        assert_eq!(mean_reversion_half_life(&series), None);
    }

    #[test]
    fn test_half_life_none_for_insufficient_data() {
        let series = [1.0, 2.0, 3.0];
        assert_eq!(mean_reversion_half_life(&series), None);
    }

    #[test]
    fn test_vwap_weights_by_volume() {
        // Two prices, one with far more volume - VWAP should sit much closer to it.
        let prices = [100.0, 200.0];
        let volumes = [90.0, 10.0];
        let v = vwap(&prices, &volumes, 2).unwrap();
        // (100*90 + 200*10) / 100 = 110
        assert!((v - 110.0).abs() < 1e-9);
    }

    #[test]
    fn test_vwap_equal_volume_is_simple_average() {
        let prices = [100.0, 200.0];
        let volumes = [1.0, 1.0];
        let v = vwap(&prices, &volumes, 2).unwrap();
        assert!((v - 150.0).abs() < 1e-9);
    }

    #[test]
    fn test_vwap_zero_volume_is_none() {
        let prices = [100.0, 200.0];
        let volumes = [0.0, 0.0];
        assert_eq!(vwap(&prices, &volumes, 2), None);
    }

    #[test]
    fn test_vwap_insufficient_history() {
        let prices = [100.0];
        let volumes = [1.0];
        assert_eq!(vwap(&prices, &volumes, 5), None);
    }

    #[test]
    fn test_bollinger_bandwidth_zero_for_constant_series() {
        let series = [10.0, 10.0, 10.0, 10.0];
        assert_eq!(bollinger_bandwidth(&series, 4, 2.0), Some(0.0));
    }

    #[test]
    fn test_bollinger_bandwidth_scales_with_num_sd() {
        let series = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let bw1 = bollinger_bandwidth(&series, series.len(), 1.0).unwrap();
        let bw2 = bollinger_bandwidth(&series, series.len(), 2.0).unwrap();
        assert!((bw2 - 2.0 * bw1).abs() < 1e-9);
    }

    #[test]
    fn test_bollinger_bandwidth_insufficient_history() {
        let series = [1.0, 2.0];
        assert_eq!(bollinger_bandwidth(&series, 5, 2.0), None);
    }
}
