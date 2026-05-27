//! 共享测试基础设施。
//!
//! 提供一个 [`TestApp`] 工厂，用真实 testcontainers MongoDB + 手写
//! [`TestLlmGenerator`] 拼出与生产同形的 [`AppState`]，便于集成测试聚焦
//! 业务逻辑而无需关心环境差异。
//!
//! 由于 testcontainers 需要 Docker，使用本模块的集成测试一般标记为
//! `#[ignore]`，由 `cargo test -- --ignored` 单独执行。

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mongodb::bson::{doc, oid::ObjectId};
use serde_json::Value;
use testcontainers::ContainerAsync;
use testcontainers_modules::mongo::Mongo;

use wechatagent::config::AppConfig;
use wechatagent::db::Database;
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{ChatUsage, LlmGenerator, LlmJsonResult};
use wechatagent::mcp::McpClient;
use wechatagent::prompts;
use wechatagent::routes::AppState;

/// 手写 LLM 生成器，用预先排队好的响应满足后续调用。
///
/// 用 `Mutex<Vec<_>>` 保留按入队顺序消费的语义，方便集成测试在调用前
/// `push_response`，然后断言 `calls()` 反映实际触达的次数。
#[derive(Default)]
pub struct TestLlmGenerator {
    pub responses: Arc<Mutex<Vec<LlmJsonResult>>>,
    pub call_count: Arc<AtomicUsize>,
}

impl TestLlmGenerator {
    /// 入队下一次 `generate_json*` 调用要返回的 JSON。
    pub fn push_response(&self, value: Value) {
        let result = LlmJsonResult {
            value,
            usage: ChatUsage::default(),
            latency_ms: 0,
            model: "test-model".to_string(),
            retry_count: 0,
        };
        self.responses.lock().expect("test llm queue").push(result);
    }

    /// 入队一条带 usage 信息的响应，用于断言成本统计。
    pub fn push_response_with_usage(&self, value: Value, usage: ChatUsage) {
        let result = LlmJsonResult {
            value,
            usage,
            latency_ms: 0,
            model: "test-model".to_string(),
            retry_count: 0,
        };
        self.responses.lock().expect("test llm queue").push(result);
    }

    /// 当前已被消费的调用次数。
    pub fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    fn pop_or_error(&self) -> AppResult<LlmJsonResult> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let mut queue = self.responses.lock().expect("test llm queue");
        if queue.is_empty() {
            return Err(AppError::External(
                "TestLlmGenerator: 没有预排队的响应".to_string(),
            ));
        }
        Ok(queue.remove(0))
    }
}

#[async_trait]
impl LlmGenerator for TestLlmGenerator {
    async fn generate_json(&self, _system: &str, _user: &str) -> AppResult<Value> {
        Ok(self.pop_or_error()?.value)
    }

    async fn generate_json_with_usage(
        &self,
        _system: &str,
        _user: &str,
    ) -> AppResult<LlmJsonResult> {
        self.pop_or_error()
    }
}

/// 启动好测试环境的 wrapper：持有 [`AppState`] 与底层容器句柄，
/// 容器在 `TestApp` drop 时自动清理。
pub struct TestApp {
    pub state: AppState,
    pub llm: Arc<TestLlmGenerator>,
    _container: ContainerAsync<Mongo>,
}

impl TestApp {
    /// 启动一个新的 testcontainers MongoDB + AppState。
    ///
    /// 每次调用都用独立 database 名（带 UUID），互不干扰。
    pub async fn start() -> Self {
        use testcontainers::runners::AsyncRunner;

        // best-effort 设置 APP_STARTED_AT；多次调用时 set 失败可忽略，因为
        // OnceCell 一旦填充即不可变。
        let _ = wechatagent::APP_STARTED_AT.set(mongodb::bson::DateTime::now());

        let container = Mongo::default().start().await.expect("启动 mongo 容器失败");
        let host = container.get_host().await.expect("获取容器 host 失败");
        let port = container
            .get_host_port_ipv4(27017)
            .await
            .expect("获取容器端口失败");
        let uri = format!("mongodb://{host}:{port}");
        let db_name = format!("wechatagent_test_{}", uuid::Uuid::new_v4().simple());

        let db = Database::connect(&uri, &db_name)
            .await
            .expect("连接测试 mongo 失败");
        wechatagent::db::migrations::run(&db)
            .await
            .expect("运行测试 mongo 迁移失败");
        db.ensure_indexes().await.expect("创建测试 mongo 索引失败");

        let llm: Arc<TestLlmGenerator> = Arc::new(TestLlmGenerator::default());

        let config = test_config(uri, db_name);

        prompts::ensure_prompt_pack_v2(
            &db,
            &config.default_workspace_id,
            &config.default_account_id,
        )
        .await
        .expect("种入默认 prompt pack 失败");

        let mcp = McpClient::new(config.mcp_base_url.clone(), config.mcp_api_key.clone())
            .expect("构造测试 mcp client 失败");

        let state = AppState {
            db,
            mcp,
            llm: llm.clone(),
            llm_registry: None,
            config,
            prompt_pack_version: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            chat_progress_bus: std::sync::Arc::new(
                wechatagent::knowledge_task::ChatProgressBus::new(),
            ),
            second_reviewer_llm: None,
        };
        // M4 W4 Task 5.3：seed 完成后 fetch_add 一次，与 main.rs 行为一致。
        state
            .prompt_pack_version
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        TestApp {
            state,
            llm,
            _container: container,
        }
    }
}

fn test_config(mongodb_uri: String, mongodb_database: String) -> AppConfig {
    AppConfig {
        app_host: "127.0.0.1".to_string(),
        app_port: 0,
        app_base_url: "http://localhost".to_string(),
        mongodb_uri,
        mongodb_database,
        mcp_base_url: "http://test-mcp.invalid".to_string(),
        mcp_api_key: "test-mcp-key".to_string(),
        openai_base_url: "http://test-llm.invalid".to_string(),
        openai_api_key: "test-llm-key".to_string(),
        openai_model: "test-model".to_string(),
        default_workspace_id: "default".to_string(),
        default_account_id: "default".to_string(),
        agent_recent_message_limit: 12,
        agent_min_reply_interval_seconds: 20,
        task_worker_interval_seconds: 30,
        llm_timeout_seconds: 5,
        llm_max_retries: 1,
        llm_retry_base_ms: 100,
        task_claim_timeout_seconds: 5,
        reaction_analysis_claim_timeout_seconds: 5,
        webhook_rate_limit_window_seconds: 60,
        webhook_rate_limit_capacity: 1000,
        strategic_planner_enabled: false,
        strategic_planner_interval_seconds: 600,
        strategic_planner_silent_threshold_hours: 72,
        strategic_planner_daily_emit_cap: 20,
        strategic_planner_commitment_imminent_window_hours: 8,
        strategic_planner_commitment_emit_dedup_hours: 24,
        strategic_planner_stage_stagnation_threshold_days: 14,
        strategic_planner_stage_stagnation_recent_inbound_hours: 24,
        strategic_planner_block_rate_window_hours: 24,
        strategic_planner_block_rate_min_runs: 3,
        strategic_planner_block_rate_threshold: 0.6,
        strategic_planner_priority_enabled: true,
        cold_contact_worker_enabled: false,
        cold_contact_threshold_hours: 168,
        cold_contact_daily_emit_cap: 5,
        // ── agent-self-evolution M4：测试默认全部 disabled / 极小值 ──
        evolution_enabled: false,
        evolution_tick_seconds: 600,
        evolution_run_token_budget: 60_000,
        evolution_run_max_llm_calls: 30,
        evolution_eval_window_hours: 72,
        evolution_min_replays: 30,
        evolution_min_send_success_delta: 0.05,
        evolution_min_self_critique_delta: 0.10,
        evolution_max_5gate_hit_increase: 0.10,
        evolution_replay_concurrency: 4,
        evolution_replay_max_fail_rate: 0.30,
        evolution_threshold_release_cooldown_hours: 24,
        evolution_cohort_per_contact_cap: 3,
        evolution_cohort_sample_per_failure_bucket: 10,
        knowledge_digest_enabled: false,
        knowledge_digest_run_hour: 9,
        knowledge_digest_run_token_budget: 60_000,
        knowledge_digest_run_max_llm_calls: 30,
        knowledge_task_worker_interval_seconds: 0,
        catalog_rebuild_worker_interval_seconds: 0,
        knowledge_feedback_interval_seconds: 0,
        reviewer_dual_enabled: false,
        reviewer_second_provider_base_url: None,
        reviewer_second_provider_api_key: None,
        reviewer_second_provider_model: None,
        reviewer_second_provider_format: "openai".to_string(),
    }
}

/// 轮询等待指定 outbox entry 进入终态（sent / failed_terminal / canceled）。
///
/// W4 / Task 5.8（R13.10 / requirements.md:549）：集成测试 helper —— dispatcher
/// 异步推进状态机，调用方需要"决策入队 → worker 抢占 → 终态"完整链路结束。
/// 100ms 步长 polling，超过 timeout 则 panic 报告最后状态。
pub async fn wait_for_outbox_processed(
    state: &AppState,
    outbox_id: ObjectId,
    timeout: Duration,
) -> wechatagent::models::OutboxEntry {
    let collection = state.db.collection_agent_send_outbox();
    let start = std::time::Instant::now();
    let mut last_status = String::new();
    while start.elapsed() < timeout {
        let entry = collection
            .find_one(doc! { "_id": outbox_id }, None)
            .await
            .expect("query outbox entry");
        if let Some(entry) = entry {
            last_status = entry.status.clone();
            if matches!(entry.status.as_str(), "sent" | "failed_terminal" | "canceled") {
                return entry;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "wait_for_outbox_processed timed out after {:?}, last status = {:?}",
        timeout, last_status
    );
}

/// 与 [`wait_for_outbox_processed`] 同语义，但按 `run_id` 字符串字段查 outbox 行。
///
/// W6 / Task 7.4（R0.7 / R13.10）：happy_path_run 集成测试在调用 `handle_managed_message`
/// 之后并不掌握 `outbox._id`，只有 `run_id`，因此需要按 `run_id` 字段轮询。命中
/// 终态 `sent / failed_terminal / canceled` 立即返回；超时 panic 报告最后状态。
pub async fn wait_for_outbox_processed_by_run_id(
    state: &AppState,
    run_id: &str,
    timeout: Duration,
) -> wechatagent::models::OutboxEntry {
    let collection = state.db.collection_agent_send_outbox();
    let start = std::time::Instant::now();
    let mut last_status = String::new();
    while start.elapsed() < timeout {
        let entry = collection
            .find_one(doc! { "run_id": run_id }, None)
            .await
            .expect("query outbox entry by run_id");
        if let Some(entry) = entry {
            last_status = entry.status.clone();
            if matches!(entry.status.as_str(), "sent" | "failed_terminal" | "canceled") {
                return entry;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "wait_for_outbox_processed_by_run_id({run_id:?}) timed out after {:?}, last status = {:?}",
        timeout, last_status
    );
}

/// 重新构造一个使用自定义 `mcp_base_url` 的 [`AppState`]，复用 [`TestApp`] 已建好的
/// Mongo 容器与 LLM mock。
///
/// W4 / Task 5.8：dispatcher → MCP 链路要靠真实 HTTP 端点验证 happy path（成功）
/// 与失败重试，因此整个测试在 setup 时把 `mcp_base_url` 替换成 wiremock URL 或
/// 一个被拒绝的端口。
pub fn rebuild_app_state_with_mcp_url(app: &TestApp, mcp_url: String) -> AppState {
    let mut config = app.state.config.clone();
    config.mcp_base_url = mcp_url.clone();
    let mcp = McpClient::new(mcp_url, config.mcp_api_key.clone())
        .expect("rebuild mcp client with overridden url");
    AppState {
        db: app.state.db.clone(),
        mcp,
        llm: app.state.llm.clone(),
        llm_registry: app.state.llm_registry.clone(),
        config,
        prompt_pack_version: app.state.prompt_pack_version.clone(),
        chat_progress_bus: app.state.chat_progress_bus.clone(),
        second_reviewer_llm: app.state.second_reviewer_llm.clone(),
    }
}
