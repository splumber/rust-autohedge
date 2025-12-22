use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct MarketStore {
    pub historical_bars: Arc<Mutex<HashMap<String, VecDeque<Value>>>>,
    pub historical_trades: Arc<Mutex<HashMap<String, VecDeque<Value>>>>,
    pub historical_quotes: Arc<Mutex<HashMap<String, VecDeque<Value>>>>,
    pub news: Arc<Mutex<Vec<Value>>>,
    pub limit: usize,
}

impl MarketStore {
    pub fn new(limit: usize) -> Self {
        Self {
            historical_bars: Arc::new(Mutex::new(HashMap::new())),
            historical_trades: Arc::new(Mutex::new(HashMap::new())),
            historical_quotes: Arc::new(Mutex::new(HashMap::new())),
            news: Arc::new(Mutex::new(Vec::new())),
            limit,
        }
    }

    pub fn update_bar(&self, symbol: String, bar: Value) {
        let mut bars_map = self.historical_bars.lock().unwrap();
        let queue = bars_map.entry(symbol).or_insert_with(VecDeque::new);
        
        if queue.len() >= self.limit {
            queue.pop_front();
        }
        queue.push_back(bar);
    }

    pub fn update_trade(&self, symbol: String, trade: Value) {
        let mut trades_map = self.historical_trades.lock().unwrap();
        let queue = trades_map.entry(symbol).or_insert_with(VecDeque::new);
        
        if queue.len() >= self.limit {
            queue.pop_front();
        }
        queue.push_back(trade);
    }

    pub fn update_quote(&self, symbol: String, quote: Value) {
        let mut quotes_map = self.historical_quotes.lock().unwrap();
        let queue = quotes_map.entry(symbol).or_insert_with(VecDeque::new);
        
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
    
    pub fn get_latest_bar(&self, symbol: &str) -> Option<Value> {
        let bars_map = self.historical_bars.lock().unwrap();
        bars_map.get(symbol).and_then(|q| q.back()).cloned()
    }

    pub fn get_bar_history(&self, symbol: &str) -> Vec<Value> {
        let bars_map = self.historical_bars.lock().unwrap();
        if let Some(queue) = bars_map.get(symbol) {
            queue.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    // Alias for compatibility if needed, but prefer specific names now
    pub fn get_history(&self, symbol: &str) -> Vec<Value> {
        self.get_bar_history(symbol)
    }

    pub fn get_trade_history(&self, symbol: &str) -> Vec<Value> {
        let trades_map = self.historical_trades.lock().unwrap();
        if let Some(queue) = trades_map.get(symbol) {
            queue.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_quote_history(&self, symbol: &str) -> Vec<Value> {
        let quotes_map = self.historical_quotes.lock().unwrap();
        if let Some(queue) = quotes_map.get(symbol) {
            queue.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_latest_quote(&self, symbol: &str) -> Option<Value> {
        let quotes_map = self.historical_quotes.lock().unwrap();
        quotes_map.get(symbol).and_then(|q| q.back()).cloned()
    }
    
    pub fn get_latest_news(&self) -> Vec<Value> {
         let news = self.news.lock().unwrap();
         news.clone()
    }
}
