pub mod queue;

use async_openai::{
    config::OpenAIConfig,
    types::{CreateChatCompletionRequestArgs, ChatCompletionRequestMessage},
    Client,
};
use std::error::Error;

pub use queue::{LLMQueue, Priority};

#[derive(Clone)]
pub struct LLMClient {
    pub client: Client<OpenAIConfig>,
    pub model: String,
}

impl LLMClient {
    pub fn new(api_key: String, base_url: Option<String>, model: String) -> Self {
        let mut config = OpenAIConfig::new().with_api_key(api_key);
        if let Some(url) = base_url {
            config = config.with_api_base(url);
        }
        let client = Client::with_config(config);
        Self { client, model }
    }

    pub async fn chat(&self, system_prompt: &str, user_input: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        use tracing::info;

        info!("🤖 Sending request to LLM (Model: {})...", self.model);
        
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages([
                ChatCompletionRequestMessage::System(async_openai::types::ChatCompletionRequestSystemMessageArgs::default().content(system_prompt).build()?),
                ChatCompletionRequestMessage::User(async_openai::types::ChatCompletionRequestUserMessageArgs::default().content(user_input).build()?),
            ])
            .build()?;

        let response = self.client.chat().create(request).await?;
        
        info!("🤖 LLM Response received.");

        Ok(response.choices[0].message.content.clone().unwrap_or_default())
    }
}
