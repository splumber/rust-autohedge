//! LLM-based headline sentiment scoring - a real semantic upgrade over
//! `services::sentiment::score_headline`'s keyword matching, using the project's existing
//! local LLM (Ollama, `config.yaml`'s `llm:` section - no new API key or signup needed).
//!
//! Scoring happens once per unique article (not once per backtest bar - `build_news_index`
//! already only calls the scorer once per article), and results are cached to disk keyed
//! by headline text, so re-running a backtest - or running several walk-forward windows
//! that share overlapping historical news - never re-scores the same article twice.

use crate::llm::LLMClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Small, fast local model confirmed reachable and responsive (~0.4s/call once warm) -
/// used specifically for this high-volume classification task regardless of whichever
/// model `config.yaml`'s `llm.model` names for the live trading LLM pipeline, since that
/// field may point at a larger/slower model not suited to scoring thousands of headlines.
const SENTIMENT_MODEL: &str = "gemma3:4b";

const SYSTEM_PROMPT: &str = "You are a financial news sentiment classifier. Given a headline \
(and optional summary) about a publicly traded company, reply with EXACTLY one word: \
positive, negative, or neutral. Judge from the perspective of a shareholder - does this \
news make the stock more or less likely to rise. No explanation, just the one word.";

#[derive(Serialize, Deserialize, Default)]
struct Cache {
    scores: HashMap<String, f64>,
}

pub struct LlmSentimentScorer {
    client: LLMClient,
    cache_path: PathBuf,
    cache: Cache,
    calls_made: usize,
}

impl LlmSentimentScorer {
    pub fn new(base_url: String) -> Self {
        let client = LLMClient::new("ollama".to_string(), Some(base_url), SENTIMENT_MODEL.to_string());
        let cache_path = PathBuf::from("data/llm_sentiment_cache.json");
        let cache = std::fs::read_to_string(&cache_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { client, cache_path, cache, calls_made: 0 }
    }

    fn cache_key(headline: &str, summary: &str) -> String {
        format!("{}||{}", headline.trim(), summary.trim())
    }

    /// Scores one article, `-1.0` (negative) / `0.0` (neutral, or unparseable) / `1.0`
    /// (positive). Checks the disk cache first; only calls the LLM on a genuine miss.
    pub async fn score(&mut self, headline: &str, summary: &str) -> f64 {
        let key = Self::cache_key(headline, summary);
        if let Some(&score) = self.cache.scores.get(&key) {
            return score;
        }
        let user_input = if summary.trim().is_empty() {
            headline.to_string()
        } else {
            format!("{}\n{}", headline, summary)
        };
        let score = match self.client.chat(SYSTEM_PROMPT, &user_input).await {
            Ok(response) => {
                let lower = response.to_lowercase();
                if lower.contains("positive") {
                    1.0
                } else if lower.contains("negative") {
                    -1.0
                } else {
                    0.0
                }
            }
            Err(_) => 0.0,
        };
        self.calls_made += 1;
        self.cache.scores.insert(key, score);
        // Persist every 50 new scores so a long scoring run isn't lost to a crash/interrupt.
        if self.calls_made % 50 == 0 {
            self.save();
        }
        score
    }

    pub fn save(&self) {
        std::fs::create_dir_all("data").ok();
        if let Ok(json) = serde_json::to_string(&self.cache) {
            let _ = std::fs::write(&self.cache_path, json);
        }
    }

    pub fn calls_made(&self) -> usize {
        self.calls_made
    }

    pub fn cache_size(&self) -> usize {
        self.cache.scores.len()
    }
}
