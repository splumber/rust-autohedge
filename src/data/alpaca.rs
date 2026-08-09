use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::error::Error;

use crate::data::store::MarketStore;
// use tracing::{info, error}; // Keep for other logs if needed, but ws logs are gone.
use crate::config::AlpacaConfig;

#[derive(Clone)]
pub struct AlpacaClient {
    client: Client,
    base_url: String,
    api_key: String,
    secret_key: String,
    pub market_store: MarketStore,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Account {
    pub buying_power: String,
    pub cash: String,
    pub portfolio_value: String,
}

/// A single OHLCV bar from Alpaca's historical crypto bars endpoint.
#[derive(Deserialize, Debug, Clone)]
pub struct CryptoBar {
    #[serde(rename = "t")]
    pub timestamp: String,
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
}

#[derive(serde::Serialize, Debug)]
pub struct OrderRequest {
    pub symbol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notional: Option<String>,
    pub side: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub time_in_force: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<String>,
}

impl AlpacaClient {
    pub fn new(config: AlpacaConfig, history_limit: usize) -> Self {
        let api_key = config.api_key;
        let secret_key = config.secret_key;
        let base_url = config.base_url;

        println!("Alpaca Client config: Base URL = {}", base_url);

        Self {
            client: Client::new(),
            base_url,
            api_key,
            secret_key,
            market_store: MarketStore::new(history_limit),
        }
    }

    pub async fn get_account(&self) -> Result<Account, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/account", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Alpaca get_account failed ({}): {}", status, body).into());
        }

        let account: Account = serde_json::from_str(&body)
            .map_err(|e| format!("Alpaca get_account decode failed: {} (body: {})", e, body))?;
        Ok(account)
    }

    pub async fn get_historical_bars(
        &self,
        symbol: &str,
        timeframe: &str,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let url = format!(
            "{}/v2/stocks/{}/bars?timeframe={}&limit=100",
            self.base_url, symbol, timeframe
        );
        let resp = self
            .client
            .get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let data: Value = resp.json().await?;
        Ok(data)
    }

    pub async fn get_assets(
        &self,
        asset_class: Option<String>,
    ) -> Result<Vec<Value>, Box<dyn Error + Send + Sync>> {
        let mut url = format!("{}/v2/assets?status=active", self.base_url);
        if let Some(param) = asset_class {
            url.push_str(&format!("&asset_class={}", param));
        }

        let resp = self
            .client
            .get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Alpaca get_assets failed ({}): {}", status, body).into());
        }

        let assets: Vec<Value> = serde_json::from_str(&body)
            .map_err(|e| format!("Alpaca get_assets decode failed: {} (body: {})", e, body))?;
        Ok(assets)
    }

    pub async fn get_positions(&self) -> Result<Vec<Value>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/positions", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Alpaca get_positions failed ({}): {}", status, body).into());
        }

        let positions: Vec<Value> = serde_json::from_str(&body)
            .map_err(|e| format!("Alpaca get_positions decode failed: {} (body: {})", e, body))?;
        Ok(positions)
    }

    pub async fn get_crypto_bars(
        &self,
        symbol: &str,
        timeframe: &str,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let url = format!(
            "https://data.alpaca.markets/v1beta3/crypto/us/bars?symbols={}&timeframe={}&limit=100",
            symbol, timeframe
        );
        let resp = self
            .client
            .get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let data: Value = resp.json().await?;
        Ok(data)
    }

    /// Fetches every bar for `symbol` between `start` and `end` (RFC3339 timestamps),
    /// following `next_page_token` until Alpaca reports no more pages. Used by the
    /// backtester, which needs a multi-day window rather than the last-100-bars snapshot
    /// `get_crypto_bars` returns.
    pub async fn get_crypto_bars_range(
        &self,
        symbol: &str,
        timeframe: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<CryptoBar>, Box<dyn Error + Send + Sync>> {
        let mut all_bars = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let mut query = vec![
                ("symbols", symbol.to_string()),
                ("timeframe", timeframe.to_string()),
                ("start", start.to_string()),
                ("end", end.to_string()),
                ("limit", "10000".to_string()),
            ];
            if let Some(token) = &page_token {
                query.push(("page_token", token.clone()));
            }

            let resp = self
                .client
                .get("https://data.alpaca.markets/v1beta3/crypto/us/bars")
                .header("APCA-API-KEY-ID", &self.api_key)
                .header("APCA-API-SECRET-KEY", &self.secret_key)
                .query(&query)
                .send()
                .await?;

            let status = resp.status();
            let body = resp.text().await?;
            if !status.is_success() {
                return Err(format!(
                    "Alpaca get_crypto_bars_range failed ({}): {}",
                    status, body
                )
                .into());
            }

            let data: Value = serde_json::from_str(&body).map_err(|e| {
                format!(
                    "Alpaca get_crypto_bars_range decode failed: {} (body: {})",
                    e, body
                )
            })?;

            let bars_for_symbol = data
                .get("bars")
                .and_then(|b| b.get(symbol))
                .and_then(|b| b.as_array())
                .cloned()
                .unwrap_or_default();

            let got_any = !bars_for_symbol.is_empty();
            for bar in bars_for_symbol {
                let parsed: CryptoBar = serde_json::from_value(bar).map_err(|e| {
                    format!("Alpaca get_crypto_bars_range bar decode failed: {}", e)
                })?;
                all_bars.push(parsed);
            }

            page_token = data
                .get("next_page_token")
                .and_then(|t| t.as_str())
                .map(String::from);

            if page_token.is_none() || !got_any {
                break;
            }
        }

        Ok(all_bars)
    }

    pub async fn get_order(&self, order_id: &str) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/orders/{}", self.base_url, order_id);
        let resp = self
            .client
            .get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Alpaca get_order failed ({}): {}", status, body).into());
        }

        let order: Value = serde_json::from_str(&body)
            .map_err(|e| format!("Alpaca get_order decode failed: {} (body: {})", e, body))?;
        Ok(order)
    }

    pub async fn cancel_order(&self, order_id: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/orders/{}", self.base_url, order_id);
        let resp = self
            .client
            .delete(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await?;
            return Err(format!("Alpaca cancel_order failed ({}): {}", status, body).into());
        }
        Ok(())
    }

    pub async fn cancel_all_orders(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/orders", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await?;
            return Err(format!("Alpaca cancel_all_orders failed ({}): {}", status, body).into());
        }
        Ok(())
    }

    pub async fn submit_order(
        &self,
        order: OrderRequest,
        trading_mode: &str,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let is_crypto = trading_mode.eq_ignore_ascii_case("crypto");
        let url = if is_crypto {
            format!("{}/v2/orders", self.base_url)
        } else {
            format!("{}/v2/orders", self.base_url)
        };

        let resp = self
            .client
            .post(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .json(&order)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Failed to place order ({}): {}", status, body).into());
        }

        let data: Value = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to decode order response: {} (body: {})", e, body))?;

        if data.get("id").is_none() {
            return Err(format!("Failed to place order: {:?}", data).into());
        }
        Ok(data)
    }
}
