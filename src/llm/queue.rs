use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tracing::info;

use super::LLMClient;

/// Priority level for LLM requests
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// High priority: pipeline continuation requests (Quant, Risk, Execution after signal)
    High,
    /// Normal priority: new analysis requests (Director)
    Normal,
}

/// A request to be queued for LLM processing
struct QueuedRequest {
    system_prompt: String,
    user_input: String,
    response_tx: oneshot::Sender<Result<String, String>>,
}

/// LLM Queue that limits concurrent requests and prioritizes pipeline continuations
#[derive(Clone)]
pub struct LLMQueue {
    high_tx: mpsc::Sender<QueuedRequest>,
    normal_tx: mpsc::Sender<QueuedRequest>,
}

impl LLMQueue {
    /// Create a new LLM Queue with the given client and max concurrent requests
    pub fn new(client: LLMClient, max_concurrent: usize, queue_size: usize) -> Self {
        let (high_tx, high_rx) = mpsc::channel::<QueuedRequest>(queue_size);
        let (normal_tx, normal_rx) = mpsc::channel::<QueuedRequest>(queue_size);

        let semaphore = Arc::new(Semaphore::new(max_concurrent));

        // Spawn the queue processor
        tokio::spawn(Self::process_queue(client, semaphore, high_rx, normal_rx));

        Self { high_tx, normal_tx }
    }

    /// Process queued requests, prioritizing high-priority over normal-priority
    async fn process_queue(
        client: LLMClient,
        semaphore: Arc<Semaphore>,
        mut high_rx: mpsc::Receiver<QueuedRequest>,
        mut normal_rx: mpsc::Receiver<QueuedRequest>,
    ) {
        info!(
            "📬 [QUEUE] LLM Queue processor started (max concurrent: {})",
            semaphore.available_permits()
        );

        loop {
            // Prioritize high-priority requests, fall back to normal if none available
            let request = tokio::select! {
                biased;

                Some(req) = high_rx.recv() => {
                    info!("📬 [QUEUE] Processing HIGH priority request");
                    req
                }
                Some(req) = normal_rx.recv() => {
                    info!("📬 [QUEUE] Processing NORMAL priority request");
                    req
                }
                else => {
                    // Both channels closed, exit
                    info!("📬 [QUEUE] All channels closed, shutting down");
                    break;
                }
            };

            // Acquire semaphore permit
            let permit = semaphore.clone().acquire_owned().await;
            if permit.is_err() {
                let _ = request
                    .response_tx
                    .send(Err("Semaphore closed".to_string()));
                continue;
            }
            let permit = permit.unwrap();

            let available = semaphore.available_permits();
            info!("📬 [QUEUE] Acquired permit. {} slots remaining", available);

            // Spawn the actual LLM call
            let client_clone = client.clone();
            tokio::spawn(async move {
                let result = client_clone
                    .chat(&request.system_prompt, &request.user_input)
                    .await
                    .map_err(|e| e.to_string());

                let _ = request.response_tx.send(result);
                drop(permit); // Release permit when done
            });
        }
    }

    /// Send a chat request with the specified priority
    pub async fn chat(
        &self,
        system_prompt: &str,
        user_input: &str,
        priority: Priority,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = QueuedRequest {
            system_prompt: system_prompt.to_string(),
            user_input: user_input.to_string(),
            response_tx,
        };

        // Send to appropriate queue based on priority
        let send_result = match priority {
            Priority::High => self.high_tx.send(request).await,
            Priority::Normal => self.normal_tx.send(request).await,
        };

        if send_result.is_err() {
            return Err("Failed to queue LLM request".into());
        }

        // Wait for response
        match response_rx.await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err("LLM request was cancelled".into()),
        }
    }

    /// Convenience method for normal priority chat
    pub async fn chat_normal(
        &self,
        system_prompt: &str,
        user_input: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.chat(system_prompt, user_input, Priority::Normal).await
    }

    /// Convenience method for high priority chat (pipeline continuations)
    pub async fn chat_high(
        &self,
        system_prompt: &str,
        user_input: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        self.chat(system_prompt, user_input, Priority::High).await
    }
}
