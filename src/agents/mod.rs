pub mod director;
pub mod quant;
pub mod risk;
pub mod execution;

use crate::llm::{LLMQueue, Priority};
use std::error::Error;

use tracing::info;

pub trait Agent {
    fn name(&self) -> &str;
    fn system_prompt(&self) -> &str;
    
    /// Run the agent with normal priority (for new analysis)
    async fn run(&self, query: &str, llm: &LLMQueue) -> Result<String, Box<dyn Error + Send + Sync>> {
        self.run_with_priority(query, llm, Priority::Normal).await
    }
    
    /// Run the agent with high priority (for pipeline continuations)
    async fn run_high_priority(&self, query: &str, llm: &LLMQueue) -> Result<String, Box<dyn Error + Send + Sync>> {
        self.run_with_priority(query, llm, Priority::High).await
    }
    
    /// Internal method to run with specified priority
    async fn run_with_priority(&self, query: &str, llm: &LLMQueue, priority: Priority) -> Result<String, Box<dyn Error + Send + Sync>> {
        let priority_str = match priority {
            Priority::High => "HIGH",
            Priority::Normal => "NORMAL",
        };
        info!("🤖 [AGENT] Sending {} priority request to {}...", priority_str, self.name());
        let response = llm.chat(self.system_prompt(), query, priority).await?;
        info!("🤖 [AGENT] Response from {}: {}", self.name(), response);
        Ok(response)
    }
}

