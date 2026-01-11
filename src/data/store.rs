use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quote {
    #[serde(rename = "S")]
    pub symbol: String,
    #[serde(rename = "bp")]
    pub bid_price: f64,
    #[serde(rename = "ap")]
    pub ask_price: f64,
    #[serde(rename = "bs")]
    pub bid_size: f64,
    #[serde(rename = "as")]
    pub ask_size: f64,
    #[serde(rename = "t")]
    pub timestamp: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trade {
    #[serde(rename = "S")]
    pub symbol: String,
    #[serde(rename = "p")]
    pub price: f64,
    #[serde(rename = "s")]
    pub size: f64,
    #[serde(rename = "t")]
    pub timestamp: String,
    #[serde(rename = "i")]
    pub id: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bar {
    #[serde(rename = "S")]
    pub symbol: String,
    #[serde(rename = "o")]
    pub open: f64,
    #[serde(rename = "h")]
    pub high: f64,
    #[serde(rename = "l")]
    pub low: f64,
    #[serde(rename = "c")]
    pub close: f64,
    #[serde(rename = "v")]
    pub volume: f64,
    #[serde(rename = "t")]
    pub timestamp: String,
}

#[derive(Clone, Debug)]
pub struct MarketStore {
    pub historical_bars: Arc<DashMap<String, VecDeque<Bar>>>,
    pub historical_trades: Arc<DashMap<String, VecDeque<Trade>>>, // Use DashMap for concurrent access
    pub historical_quotes: Arc<DashMap<String, VecDeque<Quote>>>, // Use DashMap for concurrent access
    pub news: Arc<Mutex<Vec<Value>>>,
    pub limit: usize,
}

impl MarketStore {
    pub fn new(limit: usize) -> Self {
        Self {
            historical_bars: Arc::new(DashMap::new()),
            historical_trades: Arc::new(DashMap::new()),
            historical_quotes: Arc::new(DashMap::new()),
            news: Arc::new(Mutex::new(Vec::new())),
            limit,
        }
    }

    pub fn update_bar(&self, symbol: String, bar: Bar) {
        let mut queue = self.historical_bars.entry(symbol).or_default();
        if queue.len() >= self.limit {
            queue.pop_front();
        }
        queue.push_back(bar);
    }

    pub fn update_trade(&self, symbol: String, trade: Trade) {
        let mut queue = self.historical_trades.entry(symbol).or_default();
        if queue.len() >= self.limit {
            queue.pop_front();
        }
        queue.push_back(trade);
    }

    pub fn update_quote(&self, symbol: String, quote: Quote) {
        let mut queue = self.historical_quotes.entry(symbol).or_default();
        if queue.len() >= self.limit {
            queue.pop_front();
        }
        queue.push_back(quote);
    }

    pub fn add_news(&self, news_item: Value) {
        let mut news = self.news.lock().unwrap();
        if news.len() >= self.limit {
            news.remove(0);
        }
        news.push(news_item);
    }

    pub fn get_latest_bar(&self, symbol: &str) -> Option<Bar> {
        self.historical_bars
            .get(symbol)
            .and_then(|q| q.back().cloned())
    }

    pub fn get_bar_history(&self, symbol: &str) -> Vec<Bar> {
        if let Some(queue) = self.historical_bars.get(symbol) {
            queue.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    // Alias for compatibility if needed, but prefer specific names now
    pub fn get_history(&self, symbol: &str) -> Vec<Bar> {
        self.get_bar_history(symbol)
    }

    pub fn get_trade_history(&self, symbol: &str) -> Vec<Trade> {
        if let Some(queue) = self.historical_trades.get(symbol) {
            queue.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_quote_history(&self, symbol: &str) -> Vec<Quote> {
        if let Some(queue) = self.historical_quotes.get(symbol) {
            queue.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_latest_quote(&self, symbol: &str) -> Option<Quote> {
        self.historical_quotes
            .get(symbol)
            .and_then(|q| q.back().cloned())
    }

    pub fn get_latest_news(&self) -> Vec<Value> {
        let news = self.news.lock().unwrap();
        news.clone()
    }
}
