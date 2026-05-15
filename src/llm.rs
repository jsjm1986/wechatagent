use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct LlmClient {
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

impl LlmClient {
    pub fn new(base_url: String, api_key: String, model: String) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            client: reqwest::Client::builder().build()?,
        })
    }

    pub async fn generate_json(&self, system: &str, user: &str) -> AppResult<Value> {
        let body = json!({
            "model": self.model,
            "temperature": 0.2,
            "messages": [
                ChatMessage { role: "system", content: system },
                ChatMessage { role: "user", content: user }
            ]
        });

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(AppError::External(format!("LLM HTTP {status}: {text}")));
        }

        let parsed: ChatCompletionResponse = serde_json::from_str(&text)?;
        let content = parsed
            .choices
            .first()
            .map(|choice| choice.message.content.as_str())
            .ok_or_else(|| AppError::External("LLM returned no choices".to_string()))?;
        parse_json_content(content)
    }
}

fn parse_json_content(content: &str) -> AppResult<Value> {
    let trimmed = content.trim();
    let json_text = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    serde_json::from_str(json_text).map_err(AppError::from)
}
