use std::env;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_host: String,
    pub app_port: u16,
    pub app_base_url: String,
    pub mongodb_uri: String,
    pub mongodb_database: String,
    pub mcp_base_url: String,
    pub mcp_api_key: String,
    pub openai_base_url: String,
    pub openai_api_key: String,
    pub openai_model: String,
    pub default_workspace_id: String,
    pub default_account_id: String,
    pub agent_recent_message_limit: i64,
    pub agent_min_reply_interval_seconds: i64,
    pub task_worker_interval_seconds: u64,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            app_host: env_or("APP_HOST", "0.0.0.0"),
            app_port: env_or("APP_PORT", "8080").parse()?,
            app_base_url: env_or("APP_BASE_URL", "http://localhost:8080"),
            mongodb_uri: env_or("MONGODB_URI", "mongodb://localhost:27017"),
            mongodb_database: env_or("MONGODB_DATABASE", "wechatagent"),
            mcp_base_url: env_or("MCP_BASE_URL", "http://47.108.57.147:3001"),
            mcp_api_key: require_env("MCP_API_KEY")?,
            openai_base_url: env_or("OPENAI_BASE_URL", "https://api.openai.com/v1"),
            openai_api_key: require_env("OPENAI_API_KEY")?,
            openai_model: env_or("OPENAI_MODEL", "gpt-4.1-mini"),
            default_workspace_id: env_or("DEFAULT_WORKSPACE_ID", "default"),
            default_account_id: env_or("DEFAULT_ACCOUNT_ID", "default"),
            agent_recent_message_limit: env_or("AGENT_RECENT_MESSAGE_LIMIT", "12").parse()?,
            agent_min_reply_interval_seconds: env_or("AGENT_MIN_REPLY_INTERVAL_SECONDS", "20")
                .parse()?,
            task_worker_interval_seconds: env_or("TASK_WORKER_INTERVAL_SECONDS", "30").parse()?,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn require_env(key: &str) -> anyhow::Result<String> {
    env::var(key).map_err(|_| anyhow::anyhow!("missing required environment variable {key}"))
}
