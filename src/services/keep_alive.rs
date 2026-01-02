//! Lightweight keep-alive service to prevent free-tier hosting from sleeping
//! Pings the service periodically to maintain activity

use reqwest::Client;
use std::time::Duration;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};

pub struct KeepAliveService {
    base_url: String,
    client: Client,
}

impl KeepAliveService {
    /// Create a new keep-alive service
    ///
    /// # Arguments
    /// * `base_url` - The base URL of your service (e.g., "http://localhost:3000" or "https://myapp.railway.app")
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("Failed to create HTTP client for keep-alive"),
        }
    }

    /// Start the keep-alive cron job
    ///
    /// Pings the service every 10 seconds to prevent free-tier scaling down
    /// Most free hosting services scale down after 5-30 minutes of inactivity
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let scheduler = JobScheduler::new().await?;

        let url = self.base_url.clone();
        let client = self.client.clone();

        // Ping every 10 seconds (cron: "*/10 * * * * *")
        // This aggressively keeps the service alive on free tiers
        let job = Job::new_async("*/10 * * * * *", move |_uuid, _l| {
            let url = url.clone();
            let client = client.clone();

            Box::pin(async move {
                match Self::ping_service(&url, &client).await {
                    Ok(_) => info!("✅ [KEEP-ALIVE] Service pinged successfully"),
                    Err(e) => warn!("⚠️ [KEEP-ALIVE] Ping failed: {}", e),
                }
            })
        })?;

        scheduler.add(job).await?;
        scheduler.start().await?;

        info!(
            "🔔 [KEEP-ALIVE] Cron job started - pinging every 10 seconds at {}",
            self.base_url
        );

        // Keep scheduler alive in background
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        Ok(())
    }

    /// Perform a lightweight ping to the service
    async fn ping_service(
        base_url: &str,
        client: &Client,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Try health endpoint first, fall back to stats if not available
        let endpoints = vec![
            format!("{}/health", base_url),
            format!("{}/stats", base_url),
            format!("{}/", base_url),
        ];

        let mut last_error = None;

        for endpoint in endpoints {
            match client.get(&endpoint).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        info!(
                            "🏓 [KEEP-ALIVE] Pinged {} - Status: {}",
                            endpoint,
                            response.status()
                        );
                        return Ok(());
                    } else {
                        warn!(
                            "⚠️ [KEEP-ALIVE] {} returned {}",
                            endpoint,
                            response.status()
                        );
                        last_error = Some(format!("Non-success status: {}", response.status()));
                    }
                }
                Err(e) => {
                    last_error = Some(format!("Request failed: {}", e));
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| "All endpoints failed".to_string())
            .into())
    }

    /// Start with a custom cron schedule
    ///
    /// # Arguments
    /// * `cron_expression` - Cron expression (e.g., "*/10 * * * * *" for every 10 seconds)
    ///
    /// # Examples
    /// ```
    /// // Every 10 seconds (default)
    /// service.start_with_schedule("*/10 * * * * *").await?;
    ///
    /// // Every 30 seconds
    /// service.start_with_schedule("*/30 * * * * *").await?;
    ///
    /// // Every minute
    /// service.start_with_schedule("0 * * * * *").await?;
    /// ```
    pub async fn start_with_schedule(
        &self,
        cron_expression: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let scheduler = JobScheduler::new().await?;

        let url = self.base_url.clone();
        let client = self.client.clone();

        let job = Job::new_async(cron_expression, move |_uuid, _l| {
            let url = url.clone();
            let client = client.clone();

            Box::pin(async move {
                match Self::ping_service(&url, &client).await {
                    Ok(_) => info!("✅ [KEEP-ALIVE] Service pinged successfully"),
                    Err(e) => warn!("⚠️ [KEEP-ALIVE] Ping failed: {}", e),
                }
            })
        })?;

        scheduler.add(job).await?;
        scheduler.start().await?;

        info!(
            "🔔 [KEEP-ALIVE] Custom cron job started with schedule: {}",
            cron_expression
        );

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_keep_alive_creation() {
        let service = KeepAliveService::new("http://localhost:3000".to_string());
        assert_eq!(service.base_url, "http://localhost:3000");
    }

    #[tokio::test]
    async fn test_ping_localhost() {
        // This test will fail if no local server is running, which is expected
        let client = Client::new();
        let result = KeepAliveService::ping_service("http://localhost:3000", &client).await;
        // We just verify it doesn't panic
        let _ = result;
    }
}
