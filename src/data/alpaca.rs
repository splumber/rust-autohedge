use std::env;
use std::error::Error;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value; 

use crate::data::store::MarketStore;
// use tracing::{info, error}; // Keep for other logs if needed, but ws logs are gone.


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

impl AlpacaClient {
    pub fn new(history_limit: usize) -> Self {
        let api_key = env::var("APCA_API_KEY_ID").expect("CRITICAL: APCA_API_KEY_ID not set");
        let secret_key = env::var("APCA_API_SECRET_KEY").expect("CRITICAL: APCA_API_SECRET_KEY not set");
        let base_url = env::var("APCA_API_BASE_URL").unwrap_or_else(|_| "https://paper-api.alpaca.markets".to_string());
        
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
        let resp = self.client.get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;
            
        let account: Account = resp.json().await?;
        Ok(account)
    }

    pub async fn get_historical_bars(&self, symbol: &str, timeframe: &str) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/stocks/{}/bars?timeframe={}&limit=100", self.base_url, symbol, timeframe);
        let resp = self.client.get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let data: Value = resp.json().await?;
        Ok(data)
    }

    pub async fn get_assets(&self, asset_class: Option<String>) -> Result<Vec<Value>, Box<dyn Error + Send + Sync>> {
        let mut url = format!("{}/v2/assets?status=active", self.base_url);
        if let Some(param) = asset_class {
            url.push_str(&format!("&asset_class={}", param));
        }

        let resp = self.client.get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;
        
        let assets: Vec<Value> = resp.json().await?;
        Ok(assets)
    }

    pub async fn get_positions(&self) -> Result<Vec<Value>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/positions", self.base_url);
        let resp = self.client.get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;
        
        // Check for success? mostly assuming 200 OK or error
        let positions: Vec<Value> = resp.json().await?;
        Ok(positions)
    }
    
    pub async fn get_crypto_bars(&self, symbol: &str, timeframe: &str) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let url = format!("https://data.alpaca.markets/v1beta3/crypto/us/bars?symbols={}&timeframe={}&limit=100", symbol, timeframe);
         let resp = self.client.get(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .send()
            .await?;

        let data: Value = resp.json().await?;
        Ok(data)
    }


}


#[derive(serde::Serialize, Debug)]
pub struct OrderRequest {
    pub symbol: String,
    pub qty: f64,
    pub side: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub time_in_force: String,
}

impl AlpacaClient {
    // ... logic ...

    pub async fn submit_order(&self, order: OrderRequest) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/v2/orders", self.base_url);
        let resp = self.client.post(&url)
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.secret_key)
            .json(&order)
            .send()
            .await?;
            
        let data: Value = resp.json().await?;
        if data.get("id").is_none() {
             return Err(format!("Failed to place order: {:?}", data).into());
        }
        Ok(data)
    }
}
