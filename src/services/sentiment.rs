//! Lightweight keyword-based headline sentiment scoring, used by the news-sentiment
//! backtest signal (`portfolio --mode news_sentiment`). This is a deliberately crude
//! substitute for real NLP/LLM-based sentiment: it exists so the sentiment hypothesis can
//! be backtested cheaply over hundreds of thousands of bars, something scoring every
//! headline through the project's local LLM (see `src/llm.rs`) would be far too slow for.
//! If the signal shows real backtested edge, upgrading to LLM-scored sentiment for live
//! trading (where only a handful of fresh headlines need scoring per symbol per day, not
//! hundreds of thousands of backtest rows) would be the natural next step - see
//! `docs/STRATEGIES.md`.

const POSITIVE_WORDS: &[&str] = &[
    "beat", "beats", "beating", "surge", "surges", "soar", "soars", "rally", "rallies",
    "upgrade", "upgraded", "record", "strong", "growth", "profit", "profits", "outperform",
    "bullish", "gain", "gains", "jump", "jumps", "rise", "rises", "raised", "exceeds",
    "exceeded", "tops", "boost", "boosts", "wins", "win", "expansion", "breakthrough",
];

const NEGATIVE_WORDS: &[&str] = &[
    "miss", "misses", "missed", "plunge", "plunges", "downgrade", "downgraded", "weak",
    "loss", "losses", "cuts", "cut", "recall", "lawsuit", "bearish", "falls", "fall",
    "drops", "drop", "decline", "declines", "warns", "warning", "below", "disappointing",
    "fraud", "layoffs", "layoff", "bankruptcy", "investigation", "probe", "sued", "slump",
];

/// Scores a headline (and optional summary) as roughly `[-1.0, 1.0]`: count of positive
/// keywords minus negative keywords, normalized by total keyword hits found. Returns `0.0`
/// (neutral) for text with no keyword matches at all, rather than `None` - a headline with
/// no sentiment-bearing words is a legitimate "no signal here," not a failure.
pub fn score_headline(headline: &str, summary: &str) -> f64 {
    let text = format!("{} {}", headline, summary).to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let mut pos = 0i32;
    let mut neg = 0i32;
    for w in &words {
        if POSITIVE_WORDS.contains(w) {
            pos += 1;
        }
        if NEGATIVE_WORDS.contains(w) {
            neg += 1;
        }
    }
    let total = pos + neg;
    if total == 0 {
        return 0.0;
    }
    (pos - neg) as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positive_headline_scores_positive() {
        let score = score_headline("Company beats earnings, shares surge on strong growth", "");
        assert!(score > 0.0);
    }

    #[test]
    fn test_negative_headline_scores_negative() {
        let score = score_headline("Company misses estimates, shares plunge amid lawsuit", "");
        assert!(score < 0.0);
    }

    #[test]
    fn test_neutral_headline_is_zero() {
        let score = score_headline("Company announces annual shareholder meeting date", "");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_mixed_headline_scores_between() {
        let score = score_headline("Company beats revenue guidance but misses on outlook", "");
        assert_eq!(score, 0.0); // one positive ("beats"), one negative ("misses") -> cancels out
    }

    #[test]
    fn test_case_insensitive() {
        let a = score_headline("SURGE SURGE SURGE", "");
        let b = score_headline("surge surge surge", "");
        assert_eq!(a, b);
        assert!(a > 0.0);
    }
}
