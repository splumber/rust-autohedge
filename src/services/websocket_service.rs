use std::env;
use futures_util::{stream::SplitSink, StreamExt, SinkExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, MaybeTlsStream, tungstenite::protocol::Message, WebSocketStream};
use serde_json::{Value, json};
use tracing::{info, error, warn};
use crate::data::store::{MarketStore, Trade, Quote, Bar};
use crate::bus::EventBus;
use crate::events::{Event, MarketEvent};

pub struct WebSocketService {
    api_key: String,
    secret_key: String,
    market_store: MarketStore,
    is_crypto: bool,
    symbols: Vec<String>,
    event_bus: EventBus, // CHANGED from Sender<String>
}

impl WebSocketService {
    pub fn new(market_store: MarketStore, symbols: Vec<String>, is_crypto: bool, event_bus: EventBus) -> Self {
        let api_key = env::var("APCA_API_KEY_ID").expect("APCA_API_KEY_ID not set");
        let secret_key = env::var("APCA_API_SECRET_KEY").expect("APCA_API_SECRET_KEY not set");

        if api_key.contains("your-alpaca-key") || secret_key.contains("your-alpaca-secret") {
            error!("CRITICAL: Alpaca keys are still placeholders. Set APCA_API_KEY_ID and APCA_API_SECRET_KEY in .env.");
        }

        Self {
            api_key,
            secret_key,
            market_store,
            is_crypto,
            symbols,
            event_bus,
        }
    }

    pub async fn start(&self) {
        let market_store_clone = self.market_store.clone();
        let api_key = self.api_key.clone();
        let secret_key = self.secret_key.clone();
        let symbols = self.symbols.clone();
        let is_crypto = self.is_crypto;
        let event_bus_clone = self.event_bus.clone();

        // Spawn Market Data Stream
        tokio::spawn(async move {
            let ws_url = if is_crypto {
                "wss://stream.data.alpaca.markets/v1beta3/crypto/us"
            } else {
                "wss://stream.data.alpaca.markets/v2/iex" 
            };
            
            info!("Connecting to Market Data WebSocket: {}", ws_url);
            
            match connect_async(ws_url).await {
                Ok((ws_stream, _)) => {
                    info!("✓ Market WebSocket Connected");
                    let (mut write, mut read) = ws_stream.split();
                    
                    if let Err(e) = Self::authenticate(&mut write, &api_key, &secret_key).await {
                         error!("❌ Market Auth Failed: {}", e);
                         return;
                    }
                    info!("✓ Market Auth Sent");

                    if let Err(e) = Self::subscribe(&mut write, &symbols, is_crypto).await {
                         error!("❌ Market Subscribe Failed: {}", e);
                         return;
                    }
                    info!("✓ Subscribed to: {:?}", symbols);

                    while let Some(msg) = read.next().await {
                         match msg {
                             Ok(Message::Text(text)) => {
                                 Self::process_market_message(&text, &market_store_clone, &event_bus_clone).await;
                             },
                             Ok(Message::Ping(ping)) => {
                                 write.send(Message::Pong(ping)).await.ok();
                             },
                             Err(e) => error!("❌ Market WS Error: {}", e),
                             _ => {}
                         }
                    }
                    warn!("⚠ Market WebSocket Closed");
                },
                Err(e) => error!("❌ Failed to connect to Market WS: {}", e),
            }
        });

        // Spawn News Stream
        let api_key_news = self.api_key.clone();
        let secret_key_news = self.secret_key.clone();
        let market_store_news = self.market_store.clone();

        tokio::spawn(async move {
            let ws_url = "wss://stream.data.alpaca.markets/v1beta1/news";
            info!("Connecting to News WebSocket: {}", ws_url);

            match connect_async(ws_url).await {
                 Ok((ws_stream, _)) => {
                     info!("✓ News WebSocket Connected");
                     let (mut write, mut read) = ws_stream.split();

                     if let Err(e) = Self::authenticate(&mut write, &api_key_news, &secret_key_news).await {
                         error!("❌ News Auth Failed: {}", e);
                         return;
                     } 
                     
                     // Subscribe to all news
                     let sub_msg = json!({ "action": "subscribe", "news": ["*"] });
                     if let Err(e) = write.send(Message::Text(sub_msg.to_string())).await {
                         error!("❌ News Subscribe Failed: {}", e);
                         return;
                     }
                     info!("✓ Subscribed to News");

                     while let Some(msg) = read.next().await {
                         match msg {
                             Ok(Message::Text(text)) => {
                                  Self::process_news_message(&text, &market_store_news).await;
                             },
                             Ok(Message::Ping(ping)) => {
                                 write.send(Message::Pong(ping)).await.ok();
                             },
                             Err(e) => error!("❌ News WS Error: {}", e),
                             _ => {}
                         }
                     }
                     warn!("⚠ News WebSocket Closed");
                 },
                 Err(e) => error!("❌ Failed to connect to News WS: {}", e),
            }
        });
    }

    async fn authenticate(write: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, key: &str, secret: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let auth_msg = json!({
            "action": "auth",
            "key": key,
            "secret": secret
        });
        write.send(Message::Text(auth_msg.to_string())).await?;
        Ok(())
    }

    async fn subscribe(write: &mut SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, symbols: &[String], is_crypto: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let sub_msg = if is_crypto {
            json!({ 
                "action": "subscribe", 
                "quotes": symbols,
                "trades": symbols 
            })
        } else {
            json!({ "action": "subscribe", "bars": symbols })
        };
        write.send(Message::Text(sub_msg.to_string())).await?;
        Ok(())
    }

    async fn process_market_message(text: &str, store: &MarketStore, event_bus: &EventBus) {
        if let Ok(val) = serde_json::from_str::<Value>(text) {
             if let Some(arr) = val.as_array() {
                 for item in arr {
                     if let Some(t) = item.get("T").and_then(|v| v.as_str()) {
                         match t {
                             "b" => { // Bar
                                 if let Some(s) = item.get("S").and_then(|v| v.as_str()) {
                                     let open = item.get("o").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let high = item.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let low = item.get("l").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let close = item.get("c").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let volume = item.get("v").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let timestamp = item.get("t").and_then(|t| t.as_str()).unwrap_or("").to_string();

                                     let bar = Bar {
                                         symbol: s.to_string(),
                                         open,
                                         high,
                                         low,
                                         close,
                                         volume,
                                         timestamp,
                                     };
                                     store.update_bar(s.to_string(), bar);
                                     
                                     info!("📊 Bar: {} Close: ${:.2}", s, close);
                                 }
                             },
                             "t" => { // Trade
                                 if let Some(s) = item.get("S").and_then(|v| v.as_str()) {
                                     let price = item.get("p").and_then(|p| p.as_f64()).unwrap_or(0.0);
                                     let size = item.get("s").and_then(|sz| sz.as_f64()).unwrap_or(0.0);
                                     let timestamp = item.get("t").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                     let id = item.get("i").and_then(|i| i.as_u64());

                                     let trade = Trade {
                                         symbol: s.to_string(),
                                         price,
                                         size,
                                         timestamp: timestamp.clone(),
                                         id,
                                     };
                                     store.update_trade(s.to_string(), trade);
                                     
                                     info!("🤝 Trade: {} Price: ${:.8} Size: {:.4}", s, price, size);
                                     
                                     let event = MarketEvent::Trade { 
                                         symbol: s.to_string(), 
                                         price, 
                                         size, 
                                         timestamp, 
                                     };
                                     event_bus.publish(Event::Market(event)).ok();
                                 }
                             },
                             "q" => { // Quote
                                 if let Some(s) = item.get("S").and_then(|v| v.as_str()) {
                                     let bid = item.get("bp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let ask = item.get("ap").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let bid_size = item.get("bs").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let ask_size = item.get("as").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                     let timestamp = item.get("t").and_then(|t| t.as_str()).unwrap_or("").to_string();

                                     let quote = Quote {
                                         symbol: s.to_string(),
                                         bid_price: bid,
                                         ask_price: ask,
                                         bid_size,
                                         ask_size,
                                         timestamp: timestamp.clone(),
                                     };
                                     store.update_quote(s.to_string(), quote);
                                     
                                     info!("📊 Quote: {} Bid: ${:.8} Ask: ${:.8}", s, bid, ask);
                                     
                                     let event = MarketEvent::Quote { 
                                         symbol: s.to_string(), 
                                         bid, 
                                         ask, 
                                         timestamp, 
                                     };
                                     event_bus.publish(Event::Market(event)).ok();
                                 }
                             },
                             "success" => info!("✅ WS Success: {:?}", item.get("msg")),
                             "subscription" => info!("✅ WS Subscribed: {:?}", item),
                             "error" => error!("❌ WS Error: {:?}", item),
                             _ => {}
                         }
                     }
                 }
             } else {
                 // Single message fallback
                 info!("ℹ WS Message: {}", text);
             }
        } else {
             warn!("⚠ Failed to parse WS message: {}", text);
        }
    }

    async fn process_news_message(text: &str, store: &MarketStore) {
        if let Ok(val) = serde_json::from_str::<Value>(text) {
            if let Some(arr) = val.as_array() {
                for item in arr {
                     if let Some(t) = item.get("T").and_then(|v| v.as_str()) {
                         match t {
                             "n" => { // News
                                 store.add_news(item.clone());
                                 let headline = item.get("headline").and_then(|h| h.as_str()).unwrap_or("No Headline");
                                 info!("📰 News: {}", headline);
                             },
                             "success" => info!("✅ News WS Success"),
                             _ => {}
                         }
                     }
                }
            }
        }
    }
}
