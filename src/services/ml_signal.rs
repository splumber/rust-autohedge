//! Minimal logistic regression for the `ml_signal` backtest strategy (`portfolio --mode
//! ml_signal`) - no external ML crate dependency, just enough gradient descent to test
//! whether a linear combination of the project's existing indicators predicts short-term
//! direction better than any single hand-picked threshold rule. Deliberately simple (no
//! cross-validation, no feature selection) - this is a first-pass hypothesis test like
//! every other signal in this codebase, not a production ML pipeline.
//!
//! Training happens once, in `portfolio.rs`, on a window strictly BEFORE the backtest
//! window (`strategies.ml_signal.train_days`), so the model never sees test-window data
//! during training - the same train/test discipline the project's walk-forward validation
//! established for hand-coded signals applies here too.

pub const FEATURE_COUNT: usize = 5;

#[derive(Clone, Debug)]
pub struct LogisticModel {
    pub weights: [f64; FEATURE_COUNT],
    pub bias: f64,
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

impl LogisticModel {
    pub fn predict_proba(&self, features: &[f64; FEATURE_COUNT]) -> f64 {
        let z: f64 = self
            .weights
            .iter()
            .zip(features.iter())
            .map(|(w, f)| w * f)
            .sum::<f64>()
            + self.bias;
        sigmoid(z)
    }

    /// Batch gradient descent on `rows` (feature vector, label in `{0.0, 1.0}`). `l2` is a
    /// small regularization strength to discourage runaway weights on what's ultimately a
    /// noisy, weakly-predictive feature set (price/volume indicators, not fundamentals).
    pub fn train(
        rows: &[([f64; FEATURE_COUNT], f64)],
        learning_rate: f64,
        epochs: usize,
        l2: f64,
    ) -> Self {
        let mut weights = [0.0; FEATURE_COUNT];
        let mut bias = 0.0;
        let n = (rows.len().max(1)) as f64;

        for _ in 0..epochs {
            let mut grad_w = [0.0; FEATURE_COUNT];
            let mut grad_b = 0.0;
            for (features, label) in rows {
                let z: f64 = weights
                    .iter()
                    .zip(features.iter())
                    .map(|(w, f)| w * f)
                    .sum::<f64>()
                    + bias;
                let pred = sigmoid(z);
                let error = pred - label;
                for i in 0..FEATURE_COUNT {
                    grad_w[i] += error * features[i];
                }
                grad_b += error;
            }
            for i in 0..FEATURE_COUNT {
                let reg = l2 * weights[i];
                weights[i] -= learning_rate * (grad_w[i] / n + reg);
            }
            bias -= learning_rate * (grad_b / n);
        }

        Self { weights, bias }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_train_separates_trivially_linear_data() {
        // feature[0] alone determines the label; every other feature is pure noise fixed
        // at 0. A correctly-training model should learn a strongly positive weight[0] and
        // predict high probability for positive feature[0], low for negative.
        let mut rows = Vec::new();
        for i in -20..20 {
            let f0 = i as f64;
            let label = if f0 > 0.0 { 1.0 } else { 0.0 };
            rows.push(([f0, 0.0, 0.0, 0.0, 0.0], label));
        }
        let model = LogisticModel::train(&rows, 0.05, 300, 0.0001);
        assert!(model.predict_proba(&[10.0, 0.0, 0.0, 0.0, 0.0]) > 0.8);
        assert!(model.predict_proba(&[-10.0, 0.0, 0.0, 0.0, 0.0]) < 0.2);
    }

    #[test]
    fn test_train_on_uninformative_features_stays_near_half() {
        // Labels are independent of the (constant) features - the model shouldn't learn
        // any confident direction, i.e. predictions should hover near the base rate.
        let rows: Vec<([f64; FEATURE_COUNT], f64)> = (0..20)
            .map(|i| ([0.0; FEATURE_COUNT], if i % 2 == 0 { 1.0 } else { 0.0 }))
            .collect();
        let model = LogisticModel::train(&rows, 0.05, 100, 0.001);
        let p = model.predict_proba(&[0.0; FEATURE_COUNT]);
        assert!((p - 0.5).abs() < 0.1);
    }
}
