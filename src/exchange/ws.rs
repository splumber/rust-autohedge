use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

use crate::{
    bus::EventBus,
    data::store::MarketStore,
    events::{Event, MarketEvent},
};

use super::traits::{ExchangeResult, MarketDataStream};

#[derive(Clone)]
pub enum WsProvider {
    AlpacaCrypto,
    AlpacaStocks,
    Binance,
    Coinbase,
    Kraken,
}

#[derive(Clone)]
pub struct GenericWsStream {
    pub provider: WsProvider,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
}

impl GenericWsStream {
    pub fn alpaca(api_key: String, api_secret: String, is_crypto: bool) -> Self {
        Self {
            provider: if is_crypto { WsProvider::AlpacaCrypto } else { WsProvider::AlpacaStocks },
            api_key: Some(api_key),
            api_secret: Some(api_secret),
        }
    }

    pub fn binance() -> Self {
        Self { provider: WsProvider::Binance, api_key: None, api_secret: None }
    }

    pub fn coinbase() -> Self {
        Self { provider: WsProvider::Coinbase, api_key: None, api_secret: None }
    }

    pub fn kraken() -> Self {
        Self { provider: WsProvider::Kraken, api_key: None, api_secret: None }
    }

    fn ws_url(&self) -> &'static str {
        match self.provider {
            WsProvider::AlpacaCrypto => "wss://stream.data.alpaca.markets/v1beta3/crypto/us",
            WsProvider::AlpacaStocks => "wss://stream.data.alpaca.markets/v2/iex",
            WsProvider::Binance => "wss://stream.binance.com:9443/ws",
            WsProvider::Coinbase => "wss://advanced-trade-ws.coinbase.com",
            WsProvider::Kraken => "wss://ws.kraken.com",
        }
    }

    async fn alpaca_auth(write: &mut futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, key: &str, secret: &str) -> ExchangeResult<()> {
        let auth_msg = json!({"action":"auth","key":key,"secret":secret});
        write.send(Message::Text(auth_msg.to_string())).await?;
        Ok(())

    }

    async fn alpaca_subscribe(write: &mut futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, symbols: &[String], is_crypto: bool) -> ExchangeResult<()> {
        let sub = if is_crypto {
            json!({"action":"subscribe","quotes":symbols,"trades":symbols})
        } else {
            json!({"action":"subscribe","bars":symbols})
        };
        write.send(Message::Text(sub.to_string())).await?;
        Ok(())
    }

    async fn binance_subscribe(write: &mut futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, symbols: &[String]) -> ExchangeResult<()> {
        // Binance combined streams need lowercase like "btcusdt@trade" and "btcusdt@bookTicker"
        let mut streams: Vec<String> = Vec::new();
        for s in symbols {
            let stream_sym = s.to_lowercase();
            streams.push(format!("{}@trade", stream_sym));
            streams.push(format!("{}@bookTicker", stream_sym));
        }
        let sub = json!({"method":"SUBSCRIBE","params":streams,"id":1});
        write.send(Message::Text(sub.to_string())).await?;
        Ok(())
    }

    async fn coinbase_subscribe(
        write: &mut futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
        symbols: &[String],
    ) -> ExchangeResult<()> {
        // Subscribe to market_trades channel. Coinbase uses product_ids like "BTC-USD".
        let product_ids: Vec<String> = symbols.iter().map(|s| crate::exchange::symbols::to_coinbase_product_id(s)).collect();
        let sub = json!({"type":"subscribe","product_ids":product_ids,"channel":"market_trades"});
        write.send(Message::Text(sub.to_string())).await?;
        Ok(())
    }

    async fn kraken_subscribe(
        write: &mut futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
        symbols: &[String],
    ) -> ExchangeResult<()> {
        let pairs: Vec<String> = symbols.iter().map(|s| crate::exchange::symbols::to_kraken_pair(s)).collect();
        // Subscribe to trades and ticker.
        let sub_trades = json!({"event":"subscribe","pair":pairs,"subscription": {"name":"trade"}});
        write.send(Message::Text(sub_trades.to_string())).await?;
        let sub_ticker = json!({"event":"subscribe","pair":symbols.iter().map(|s| crate::exchange::symbols::to_kraken_pair(s)).collect::<Vec<_>>(),"subscription": {"name":"ticker"}});
        write.send(Message::Text(sub_ticker.to_string())).await?;
        Ok(())
    }

    async fn process_alpaca(text: &str, store: &MarketStore, bus: &EventBus) {
        if let Ok(val) = serde_json::from_str::<Value>(text) {
            if let Some(arr) = val.as_array() {
                for item in arr {
                    if let Some(t) = item.get("T").and_then(|v| v.as_str()) {
                        match t {
                            "t" => {
                                if let Some(s) = item.get("S").and_then(|v| v.as_str()) {
                                    store.update_trade(s.to_string(), item.clone());
                                    let price = item.get("p").and_then(|p| p.as_f64()).unwrap_or(0.0);
                                    let size = item.get("s").and_then(|sz| sz.as_f64()).unwrap_or(0.0);
                                    let timestamp = item.get("t").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                    bus.publish(Event::Market(MarketEvent::Trade { symbol: s.to_string(), price, size, timestamp, original: item.clone() })).ok();
                                }
                            }
                            "q" => {
                                if let Some(s) = item.get("S").and_then(|v| v.as_str()) {
                                    store.update_quote(s.to_string(), item.clone());
                                    let bid = item.get("bp").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    let ask = item.get("ap").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    let timestamp = item.get("t").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                    bus.publish(Event::Market(MarketEvent::Quote { symbol: s.to_string(), bid, ask, timestamp, original: item.clone() })).ok();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    async fn process_binance(text: &str, store: &MarketStore, bus: &EventBus) {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            // trade event
            if v.get("e").and_then(|x| x.as_str()) == Some("trade") {
                let symbol = v.get("s").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let price = v.get("p").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let size = v.get("q").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let timestamp = v.get("T").and_then(|x| x.as_i64()).map(|t| t.to_string()).unwrap_or_default();
                if !symbol.is_empty() {
                    store.update_trade(symbol.clone(), v.clone());
                    bus.publish(Event::Market(MarketEvent::Trade { symbol, price, size, timestamp, original: v.clone() })).ok();
                }
            }
            // bookTicker event
            if v.get("e").and_then(|x| x.as_str()) == Some("bookTicker") {
                let symbol = v.get("s").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let bid = v.get("b").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let ask = v.get("a").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                let timestamp = v.get("E").and_then(|x| x.as_i64()).map(|t| t.to_string()).unwrap_or_default();
                if !symbol.is_empty() {
                    store.update_quote(symbol.clone(), v.clone());
                    bus.publish(Event::Market(MarketEvent::Quote { symbol, bid, ask, timestamp, original: v.clone() })).ok();
                }
            }
        }
    }

    async fn process_coinbase(text: &str, store: &MarketStore, bus: &EventBus) {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            if v.get("channel").and_then(|c| c.as_str()) == Some("market_trades") {
                if let Some(events) = v.get("events").and_then(|e| e.as_array()) {
                    for ev in events {
                        if let Some(trades) = ev.get("trades").and_then(|t| t.as_array()) {
                            for tr in trades {
                                let product_id = tr.get("product_id").and_then(|x| x.as_str()).unwrap_or("");
                                let symbol = product_id.replace('-', "/");
                                let price = tr.get("price").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                                let size = tr.get("size").and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                                let timestamp = tr.get("time").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                if price > 0.0 {
                                    store.update_trade(symbol.clone(), tr.clone());
                                    bus.publish(Event::Market(MarketEvent::Trade { symbol, price, size, timestamp, original: tr.clone() })).ok();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn process_kraken(text: &str, store: &MarketStore, bus: &EventBus) {
        // Kraken WS uses array messages for data, object messages for system/status.
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            if v.is_array() {
                let arr = v.as_array().unwrap();
                if arr.len() < 3 {
                    return;
                }
                let channel_name = arr.get(arr.len() - 2).and_then(|x| x.as_str()).unwrap_or("");
                let pair = arr.get(arr.len() - 1).and_then(|x| x.as_str()).unwrap_or("");
                let symbol = pair.replace("XBT/", "BTC/");

                if channel_name == "trade" {
                    if let Some(trades) = arr.get(1).and_then(|x| x.as_array()) {
                        for t in trades {
                            if let Some(tarr) = t.as_array() {
                                let price = tarr.get(0).and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                                let size = tarr.get(1).and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                                let timestamp = tarr.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string();
                                if price > 0.0 {
                                    store.update_trade(symbol.clone(), v.clone());
                                    bus.publish(Event::Market(MarketEvent::Trade { symbol: symbol.clone(), price, size, timestamp, original: v.clone() })).ok();
                                }
                            }
                        }
                    }
                }

                if channel_name == "ticker" {
                    // Best effort: pull bid/ask from ticker payload.
                    if let Some(obj) = arr.get(1) {
                        let bid = obj.get("b").and_then(|b| b.get(0)).and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                        let ask = obj.get("a").and_then(|a| a.get(0)).and_then(|x| x.as_str()).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
                        let timestamp = chrono::Utc::now().to_rfc3339();
                        if bid > 0.0 && ask > 0.0 {
                            store.update_quote(symbol.clone(), json!({"bp": bid, "ap": ask, "t": timestamp, "pair": pair}));
                            bus.publish(Event::Market(MarketEvent::Quote { symbol, bid, ask, timestamp, original: v.clone() })).ok();
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl MarketDataStream for GenericWsStream {
    async fn start(&self, store: MarketStore, symbols: Vec<String>, event_bus: EventBus) -> ExchangeResult<()> {
        let ws_url = self.ws_url();
        info!("Connecting to WS: {}", ws_url);

        let (ws_stream, _) = connect_async(ws_url).await.map_err(|e| format!("WS connect failed: {e}"))?;
        let (mut write, mut read) = ws_stream.split();

        let provider = self.provider.clone();

        match provider {
            WsProvider::AlpacaCrypto => {
                let key = self.api_key.clone().unwrap_or_default();
                let secret = self.api_secret.clone().unwrap_or_default();
                Self::alpaca_auth(&mut write, &key, &secret).await?;
                Self::alpaca_subscribe(&mut write, &symbols, true).await?;
            }
            WsProvider::AlpacaStocks => {
                let key = self.api_key.clone().unwrap_or_default();
                let secret = self.api_secret.clone().unwrap_or_default();
                Self::alpaca_auth(&mut write, &key, &secret).await?;
                Self::alpaca_subscribe(&mut write, &symbols, false).await?;
            }
            WsProvider::Binance => {
                Self::binance_subscribe(&mut write, &symbols).await?;
            }
            WsProvider::Coinbase => {
                Self::coinbase_subscribe(&mut write, &symbols).await?;
            }
            WsProvider::Kraken => {
                Self::kraken_subscribe(&mut write, &symbols).await?;
            }
        }

        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => match provider {
                        WsProvider::AlpacaCrypto | WsProvider::AlpacaStocks => Self::process_alpaca(&text, &store, &event_bus).await,
                        WsProvider::Binance => Self::process_binance(&text, &store, &event_bus).await,
                        WsProvider::Coinbase => Self::process_coinbase(&text, &store, &event_bus).await,
                        WsProvider::Kraken => Self::process_kraken(&text, &store, &event_bus).await,
                    },
                    Ok(Message::Ping(p)) => {
                        let _ = write.send(Message::Pong(p)).await;
                    }
                    Err(e) => {
                        error!("WS error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
            warn!("WS loop ended");
        });

        Ok(())
    }
}
