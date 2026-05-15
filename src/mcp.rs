use mongodb::bson::{to_document, DateTime};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    error::{AppError, AppResult},
    models::McpCallLog,
    routes::AppState,
};

#[derive(Clone)]
pub struct McpClient {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl McpClient {
    pub fn new(base_url: String, api_key: String) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            client: reqwest::Client::builder().build()?,
        })
    }

    pub async fn call_tool<A: Serialize>(&self, tool_name: &str, arguments: A) -> AppResult<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": uuid::Uuid::new_v4().to_string(),
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });

        let response = self
            .client
            .post(format!("{}/mcp", self.base_url))
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        let body: Value = response.json().await?;
        if !status.is_success() {
            return Err(AppError::External(format!("MCP HTTP {status}: {body}")));
        }
        if let Some(error) = body.get("error") {
            return Err(AppError::External(format!(
                "MCP tool {tool_name} failed: {error}"
            )));
        }
        Ok(body
            .get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .unwrap_or(Value::Null))
    }
}

pub async fn logged_call<A: Serialize>(
    state: &AppState,
    tool_name: &str,
    arguments: A,
) -> AppResult<Value> {
    let request_doc = to_document(&serde_json::to_value(&arguments)?)?;
    let result = state.mcp.call_tool(tool_name, arguments).await;
    let (response, error) = match &result {
        Ok(value) => (to_document(value).ok(), None),
        Err(err) => (None, Some(err.to_string())),
    };
    let _ = state
        .db
        .mcp_logs()
        .insert_one(
            McpCallLog {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: state.config.default_account_id.clone(),
                tool_name: tool_name.to_string(),
                request: request_doc,
                response,
                error,
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
    result
}
