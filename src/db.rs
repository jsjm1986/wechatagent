use mongodb::{
    bson::doc,
    options::{ClientOptions, IndexOptions},
    Client, Collection, Database as MongoDatabase, IndexModel,
};

use crate::models::{
    AgentEvent, AgentTask, Contact, ConversationMessage, McpCallLog, WechatAccount,
};

#[derive(Clone)]
pub struct Database {
    db: MongoDatabase,
}

impl Database {
    pub async fn connect(uri: &str, database: &str) -> anyhow::Result<Self> {
        let mut options = ClientOptions::parse(uri).await?;
        options.app_name = Some("wechatagent".to_string());
        let client = Client::with_options(options)?;
        let db = client.database(database);
        let database = Self { db };
        database.ensure_indexes().await?;
        Ok(database)
    }

    pub fn accounts(&self) -> Collection<WechatAccount> {
        self.db.collection("wechat_accounts")
    }

    pub fn contacts(&self) -> Collection<Contact> {
        self.db.collection("contacts")
    }

    pub fn messages(&self) -> Collection<ConversationMessage> {
        self.db.collection("conversation_messages")
    }

    pub fn tasks(&self) -> Collection<AgentTask> {
        self.db.collection("agent_tasks")
    }

    pub fn events(&self) -> Collection<AgentEvent> {
        self.db.collection("agent_events")
    }

    pub fn mcp_logs(&self) -> Collection<McpCallLog> {
        self.db.collection("mcp_call_logs")
    }

    async fn ensure_indexes(&self) -> anyhow::Result<()> {
        self.accounts()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "workspace_id": 1, "account_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                None,
            )
            .await?;
        self.contacts()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "workspace_id": 1, "account_id": 1, "wxid": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                None,
            )
            .await?;
        self.messages()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                    .build(),
                None,
            )
            .await?;
        self.messages()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "workspace_id": 1, "account_id": 1, "message_id": 1 })
                    .options(IndexOptions::builder().sparse(true).unique(true).build())
                    .build(),
                None,
            )
            .await?;
        self.tasks()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "status": 1, "run_at": 1 })
                    .build(),
                None,
            )
            .await?;
        Ok(())
    }
}
