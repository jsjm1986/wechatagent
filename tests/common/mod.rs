//! 共享测试基础设施。
//!
//! 提供一个 [`TestApp`] 工厂，用真实 testcontainers MongoDB + 手写
//! [`TestLlmGenerator`] 拼出与生产同形的 [`AppState`]，便于集成测试聚焦
//! 业务逻辑而无需关心环境差异。
//!
//! 由于 testcontainers 需要 Docker，使用本模块的集成测试一般标记为
//! `#[ignore]`，由 `cargo test -- --ignored` 单独执行。

#![allow(dead_code)]

pub mod capability_evidence;
pub mod dynamic;
pub mod generalization;
pub mod identity_generator;
pub mod judge;
pub mod redline;
pub mod roleplay_fixtures;
pub mod roleplayer;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use serde_json::Value;
use testcontainers::ContainerAsync;
use testcontainers_modules::mongo::Mongo;

use wechatagent::config::AppConfig;
use wechatagent::db::Database;
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{ChatUsage, LlmJsonResult, LlmProvider};
use wechatagent::mcp::McpClient;
use wechatagent::models::WechatAccount;
use wechatagent::prompts;
use wechatagent::routes::AppState;

pub fn test_account_document(workspace_id: &str, account_id: &str) -> Document {
    let now = DateTime::now();
    mongodb::bson::to_document(&WechatAccount {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        alias: account_id.to_string(),
        display_name: account_id.to_string(),
        app_id: None,
        wxid: None,
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        webhook_secret: None,
        online: true,
        status: Some("active".to_string()),
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: Vec::new(),
        created_at: now,
        updated_at: now,
    })
    .expect("serialize test account")
}

/// Register an account scope required by workers and scoped MCP resolution.
///
/// The upsert is intentionally insert-only: tests that seed account-specific
/// credentials keep them, while worker tests can opt in without weakening the
/// production fail-closed behavior for unknown accounts.
pub async fn ensure_test_account(state: &AppState, workspace_id: &str, account_id: &str) {
    state
        .db
        .accounts()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            doc! { "$setOnInsert": test_account_document(workspace_id, account_id) },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await
        .expect("ensure test account");
}

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
            // A queued mock response models a successful upstream response.
            // Zero tokens are measured zero, not missing usage telemetry.
            usage: ChatUsage {
                usage_known: true,
                ..Default::default()
            },
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

/// 独立语义 ClaimGate 的严格“无需事实证据”响应。
///
/// Gateway 在每轮 Review 后都会再调用一次 `user.review.claim_gate`。集成测试必须显式
/// 排入完整 schema，避免旧的 Reply→Review 两段 fixture 被第三次调用错位消费。
pub fn independent_claim_gate_pass_json() -> Value {
    serde_json::json!({
        "requiresEvidence": false,
        "claimKinds": [],
        "hasCatalogClaims": false,
        "catalogCoverageComplete": true,
        "hasNonCatalogEvidenceClaims": false,
        "catalogClaims": [],
        "reason": "The candidate contains no independently verifiable business claim."
    })
}

/// Independent ClaimGate verdict for a capability statement that must be backed by verified
/// operation knowledge. This fixture is used only where the test has seeded and cited a verified
/// chunk; it must not be used to bypass evidence checks in generic gateway-flow tests.
pub fn independent_claim_gate_verified_knowledge_json() -> Value {
    serde_json::json!({
        "requiresEvidence": true,
        "claimKinds": ["product_capability"],
        "hasCatalogClaims": false,
        "catalogCoverageComplete": true,
        "hasNonCatalogEvidenceClaims": true,
        "catalogClaims": [],
        "reason": "The candidate asserts a product capability backed by cited verified knowledge."
    })
}

#[async_trait]
impl LlmProvider for TestLlmGenerator {
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

    /// 模拟一个 supports_vision 的 provider：忽略图片字节，从同一队列取下一条
    /// 预排响应。vision_safety_gate 集成测试靠这个让 import-apply-image 走通
    /// generate_json_with_image 路径（默认实现会报 vision_not_supported）。
    async fn generate_json_with_image(
        &self,
        _system: &str,
        _user: &str,
        _image_base64: &str,
        _mime: &str,
    ) -> AppResult<Value> {
        Ok(self.pop_or_error()?.value)
    }
}

/// 启动好测试环境的 wrapper：持有 [`AppState`] 与底层容器句柄，
/// 容器在 `TestApp` drop 时自动清理。
pub struct TestApp {
    pub state: AppState,
    pub llm: Arc<TestLlmGenerator>,
    _container: Option<ContainerAsync<Mongo>>,
    external_mongo: bool,
}

impl TestApp {
    /// 启动一个新的 testcontainers MongoDB（standalone）+ AppState。
    ///
    /// 每次调用都用独立 database 名（带 UUID），互不干扰。
    pub async fn start() -> Self {
        Self::start_inner(false).await
    }

    /// 启动一个 replica-set 模式的 MongoDB + AppState。
    ///
    /// 多文档事务（`start_transaction` / `commit_with_session`）只能在 replica set
    /// 上提交——standalone mongod 会报 "does not support retryable writes"、无法 commit。
    /// 仅走事务路径的集成测试（如 `evolution::release::rollback_prompt`）需要它；
    /// 其余测试用启动更快的 standalone [`start`]。
    pub async fn start_repl_set() -> Self {
        Self::start_inner(true).await
    }

    async fn start_inner(repl_set: bool) -> Self {
        use testcontainers::runners::AsyncRunner;

        // best-effort 设置 APP_STARTED_AT；多次调用时 set 失败可忽略，因为
        // OnceCell 一旦填充即不可变。
        let _ = wechatagent::APP_STARTED_AT.set(mongodb::bson::DateTime::now());

        // CI/default remains Testcontainers. Environments with a trusted local mongod but no
        // Docker may opt in explicitly; every TestApp still gets a random isolated database.
        let external_uri = std::env::var("TEST_MONGODB_URI")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let external_mongo = external_uri.is_some();
        let (container, uri) = if let Some(uri) = external_uri {
            (None, uri)
        } else {
            let container = if repl_set {
                Mongo::repl_set()
                    .start()
                    .await
                    .expect("启动 mongo replica set 容器失败")
            } else {
                Mongo::default().start().await.expect("启动 mongo 容器失败")
            };
            let host = container.get_host().await.expect("获取容器 host 失败");
            let port = container
                .get_host_port_ipv4(27017)
                .await
                .expect("获取容器端口失败");
            // 单节点 replica set 必须带 directConnection=true：否则驱动会按 RS 拓扑做
            // server discovery，连到容器内 advertise 的 hostname（宿主不可达）。
            let uri = if repl_set {
                format!("mongodb://{host}:{port}/?directConnection=true")
            } else {
                format!("mongodb://{host}:{port}")
            };
            (Some(container), uri)
        };
        let db_name = format!("wechatagent_test_{}", uuid::Uuid::new_v4().simple());

        let db = Database::connect(&uri, &db_name)
            .await
            .expect("连接测试 mongo 失败");
        wechatagent::db::migrations::run(&db)
            .await
            .expect("运行测试 mongo 迁移失败");
        db.ensure_indexes().await.expect("创建测试 mongo 索引失败");

        // 重新 seed 销售域字典（customer_stage / intent_level / objection_type）。
        // m012_drop_legacy_taxonomy_seed 在非 production 环境会删掉 m006 的这三 kind seed
        // （生产靠 APP_ENV=production 守卫跳过删除、字典保留），测试 DB 跑完全部迁移后这三
        // kind 字典为空。经 validate_dimension_value(Taxonomy) 校验的维度（如 customer_stage）
        // 会因字典 miss 被判 CandidateNew → DropSilently → 键被移除，C2 operation_state 派生
        // 回落 decision.operation_state。复用 m006 的 upsert seed（与生产同源，不抄数据）补回，
        // 让测试 DB 字典状态对齐 production 实例。
        wechatagent::db::migrations::m006_taxonomy_seed::run_step(&db)
            .await
            .expect("重新 seed 销售域 taxonomy 失败");

        let llm: Arc<TestLlmGenerator> = Arc::new(TestLlmGenerator::default());

        let config = test_config(uri, db_name);

        prompts::ensure_prompt_pack_v2(
            &db,
            &config.default_workspace_id,
            &config.default_account_id,
        )
        .await
        .expect("种入默认 prompt pack 失败");

        // 预热进程级 taxonomy 缓存（LazyLock 单例 + 30s TTL），与 main.rs:83 对齐。
        // 上面已 re-seed 销售域字典，这里把缓存对齐到本测试 DB 的字典内容，使后续
        // check_value / validate_dimension_value 能命中（否则缓存可能停留在空字典或
        // 别的测试 DB 状态）。warm_up 内部忽略 TTL 无条件 reload。
        wechatagent::agent::init_global_taxonomy_cache(&db)
            .await
            .expect("warm taxonomy cache");

        // 同理预热**第二个**进程级 TTL 单例：active DomainProfile 缓存（domain_profile.rs
        // 的 GLOBAL_DOMAIN_PROFILE_CACHE，与 taxonomy 缓存同款 LazyLock+30s TTL）。
        // 与 main.rs:86 对齐。不预热的话，本测试若不 seed 自定义 domain_profiles 行，
        // load_active_domain_profile 会命中**前一个测试 seed 的 active profile**（同 binary
        // 内残留），其声明维度可能不含 customer_stage → retain_declared_dimensions 把
        // customer_stage 从 domain_signals 剔除 → C2 operation_state 派生回落
        // decision.operation_state。预热到本测试自己的 DB（无 active 行）→ 回落 DEFAULT
        // profile（声明 customer_stage participates_in_decision=true）→ 维度存活、派生正确。
        wechatagent::agent::init_global_domain_profile_cache(&db)
            .await
            .expect("warm domain profile cache");

        let mcp = McpClient::new(config.mcp_base_url.clone(), config.mcp_api_key.clone())
            .expect("构造测试 mcp client 失败");
        let auth_rate_limiter =
            std::sync::Arc::new(wechatagent::auth::rate_limit::AuthRateLimiter::new(
                config.auth_rate_limit_window_seconds,
                config.auth_rate_limit_client_capacity,
                config.auth_rate_limit_target_capacity,
                config.auth_rate_limit_global_capacity,
            ));

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
            chunk_locks: std::sync::Arc::new(dashmap::DashMap::new()),
            chunk_event_bus: tokio::sync::broadcast::channel(
                wechatagent::routes::chunk_locks::CHUNK_EVENT_CHANNEL_CAPACITY,
            )
            .0,
            jwt_keys: None,
            auth_rate_limiter,
            completeness_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        };
        // M4 W4 Task 5.3：seed 完成后 fetch_add 一次，与 main.rs 行为一致。
        state
            .prompt_pack_version
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        TestApp {
            state,
            llm,
            _container: container,
            external_mongo,
        }
    }

    /// Explicitly remove the random database when TEST_MONGODB_URI is used. Container-backed
    /// databases are removed with their container and need no manual cleanup.
    pub async fn cleanup(self) {
        if self.external_mongo {
            self.state
                .db
                .raw()
                .drop(None)
                .await
                .expect("删除外部 Mongo 隔离测试库失败");
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
        // 账号级发送间隔闸：测试默认关（0/0 → account_send_interval_ms 恒返 0 →
        // process_entry 的 `now - last_sent < 0` 永假 → 永不 defer）。与本文件其它
        // worker「测试默认 disabled / 极小值」约定一致：绝大多数集成测试用同一
        // account_id="default" 连发多条且不 sleep，若启用 1-4s 拟人间隔会把第二条
        // 起的发送 reschedule 成 pending 致断言超时。需要验证间隔闸的测试（见
        // outbox_integration.rs account_pacing_gate_*）自行把 min=max 覆盖为非 0 值。
        account_send_min_interval_ms: 0,
        account_send_max_interval_ms: 0,
        agent_reply_max_segment_chars: 120,
        agent_reply_max_segments: 4,
        // 渐进式三档开关：测试默认 true（与 PROGRESSIVE_TIER_ENABLED 默认一致，走两程循环）。
        progressive_tier_enabled: true,
        message_debounce_window_ms: 4000,
        task_worker_interval_seconds: 30,
        completeness_cache_ttl_seconds: 300,
        llm_timeout_seconds: 5,
        llm_max_retries: 1,
        llm_retry_base_ms: 100,
        task_claim_timeout_seconds: 5,
        import_worker_interval_seconds: 30,
        import_job_claim_timeout_seconds: 5,
        reaction_analysis_claim_timeout_seconds: 5,
        webhook_rate_limit_window_seconds: 60,
        webhook_rate_limit_capacity: 1000,
        strategic_planner_enabled: false,
        strategic_planner_interval_seconds: 600,
        strategic_planner_silent_threshold_hours: 72,
        strategic_planner_daily_emit_cap: 20,
        strategic_planner_commitment_imminent_window_hours: 8,
        strategic_planner_commitment_fallback_due_hours: 72,
        strategic_planner_commitment_emit_dedup_hours: 24,
        strategic_planner_stage_stagnation_threshold_days: 14,
        strategic_planner_stage_stagnation_recent_inbound_hours: 24,
        strategic_planner_calendar_lookahead_days: 1,
        strategic_planner_calendar_daily_cap: 3,
        strategic_planner_calendar_tz_offset_hours: 8,
        strategic_planner_renewal_lookahead_days: 14,
        strategic_planner_renewal_grace_days: 7,
        strategic_planner_renewal_daily_cap: 3,
        strategic_planner_reactivation_dormant_days: 30,
        strategic_planner_reactivation_cadence_days: 30,
        strategic_planner_reactivation_daily_cap: 3,
        value_tier_mid_threshold_cents: 50000,
        value_tier_high_threshold_cents: 300000,
        strategic_planner_block_rate_window_hours: 24,
        strategic_planner_block_rate_min_runs: 3,
        strategic_planner_block_rate_threshold: 0.6,
        strategic_planner_priority_enabled: true,
        cold_contact_worker_enabled: false,
        cold_contact_threshold_hours: 168,
        cold_contact_daily_emit_cap: 5,
        holding_reply_min_interval_hours: 6.0,
        holding_reply_token_budget: 3000,
        account_daily_send_soft_cap: 500,
        campaign_max_audience: 500,
        // ── 自学习采集管道（第一阶段）：测试默认全部 disabled / 极小值 ──
        silence_signal_worker_enabled: false,
        silence_threshold_seconds: 86400,
        silence_signal_interval_seconds: 0,
        silence_signal_daily_cap: 500,
        dynamic_confidence_min_samples: 5,
        dynamic_confidence_real_outcome_enabled: true,
        behavior_signal_metrics_enabled: false,
        knowledge_exploration_enabled: false,
        knowledge_exploration_temperature: 1.0,
        // ── agent-self-evolution M4：测试默认全部 disabled / 极小值 ──
        evolution_enabled: false,
        evolution_tick_seconds: 600,
        evolution_run_token_budget: 60_000,
        evolution_run_max_llm_calls: 30,
        evolution_eval_window_hours: 72,
        evolution_min_replays: 30,
        evolution_min_send_success_delta: 0.05,
        evolution_max_5gate_hit_increase: 0.10,
        evolution_max_safety_regression_rate: 0.0,
        evolution_replay_concurrency: 4,
        evolution_replay_max_fail_rate: 0.30,
        evolution_threshold_release_cooldown_hours: 24,
        evolution_cohort_per_contact_cap: 3,
        evolution_cohort_sample_per_failure_bucket: 10,
        evolution_max_negative_reaction_increase: 0.05,
        evolution_auto_release_enabled: false,
        evolution_auto_release_window_hours: 336,
        evolution_auto_release_per_tick_cap: 1,
        evolution_auto_release_negative_reaction_gate_enabled: false,
        evolution_auto_release_max_negative_reaction_rate: 0.30,
        knowledge_digest_enabled: false,
        knowledge_digest_run_hour: 9,
        knowledge_digest_run_token_budget: 60_000,
        knowledge_digest_run_max_llm_calls: 30,
        knowledge_task_worker_interval_seconds: 0,
        catalog_rebuild_worker_interval_seconds: 0,
        knowledge_feedback_interval_seconds: 0,
        ingest_worker_enabled: false,
        ingest_worker_interval_seconds: 0,
        reviewer_dual_enabled: false,
        reviewer_second_provider_base_url: None,
        reviewer_second_provider_api_key: None,
        reviewer_second_provider_model: None,
        reviewer_second_provider_format: "openai".to_string(),
        session_ttl_hours: 8,
        session_cookie_secure: false,
        bootstrap_admin_username: None,
        bootstrap_admin_password: None,
        system_operator_usernames: Vec::new(),
        auth_rate_limit_window_seconds: 300,
        auth_rate_limit_client_capacity: 1000,
        auth_rate_limit_target_capacity: 1000,
        auth_rate_limit_global_capacity: 1000,
        webhook_verify_signature: false,
        webhook_timestamp_skew_seconds: 300,
        jwt_enabled: false,
        jwt_ttl_minutes: 60,
        jwt_private_key_pem: None,
        jwt_public_key_pem: None,
        media_storage_dir: "./media".to_string(),
        media_max_file_size_mb: 50,
        media_id_cache_ttl_hours: 24,
        wake_jitter_max_seconds: 900,
    }
}

/// 轮询等待指定 outbox entry 进入终态（sent / failed_terminal / canceled /
/// delivery_unknown）。
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
            if matches!(
                entry.status.as_str(),
                "sent" | "failed_terminal" | "canceled" | "delivery_unknown"
            ) {
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
/// 终态 `sent / failed_terminal / canceled / delivery_unknown` 立即返回；超时 panic
/// 报告最后状态。
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
            if matches!(
                entry.status.as_str(),
                "sent" | "failed_terminal" | "canceled" | "delivery_unknown"
            ) {
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
        chunk_locks: app.state.chunk_locks.clone(),
        chunk_event_bus: app.state.chunk_event_bus.clone(),
        jwt_keys: app.state.jwt_keys.clone(),
        auth_rate_limiter: app.state.auth_rate_limiter.clone(),
        completeness_cache: app.state.completeness_cache.clone(),
    }
}

/// Build an explicitly enabled Evolution release state for tests that exercise
/// the administrative release boundary. The shared TestApp remains disabled by
/// default so unrelated tests cannot accidentally bypass the production gate.
pub async fn evolution_release_state(app: &TestApp, workspace: &str) -> AppState {
    let mut state = app.state.clone();
    state.config.evolution_enabled = true;
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("evolution_runtime_flags")
        .update_one(
            doc! { "workspace_id": workspace },
            doc! {
                "$set": {
                    "workspace_id": workspace,
                    "enabled": true,
                    "rollout_percent": 100_i64,
                    "threshold_auto_release_enabled": false,
                    "updated_by": "integration-test",
                    "updated_at": DateTime::now(),
                }
            },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await
        .expect("enable evolution release test gate");
    state
}

/// 插入一条 `proposal_kind="prompt"`, `status="released"` 的 proposal，供
/// `rollback_prompt` 加载使用。直插 raw collection，避开 Proposal 30+ 字段字面量；
/// 只填 Proposal 反序列化必需的字段 + rollback 实际读取的字段。返回其 `ObjectId`。
pub async fn insert_released_prompt_proposal(
    state: &AppState,
    workspace: &str,
    key: &str,
    prev: &str,
) -> ObjectId {
    let id = ObjectId::new();
    let now = DateTime::now();
    let version = prev.parse::<i32>().expect("previous prompt version");
    let templates = state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("prompt_templates");
    let current = templates
        .find_one(
            doc! {
                "workspace_id": workspace,
                "prompt_key": key,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("read current prompt artifact")
        .expect("current prompt artifact");
    let current_id = current.get_object_id("_id").expect("current prompt _id");
    let current_version = current.get_i32("version").expect("current prompt version");
    let current_content = current.get_str("content").expect("current prompt content");
    let released_revision = wechatagent::evolution::revision::prompt_revision(
        current_id,
        current_version,
        current_content,
    );
    let base_revision = match templates
        .find_one(
            doc! {
                "workspace_id": workspace,
                "prompt_key": key,
                "version": version,
            },
            None,
        )
        .await
        .expect("read prompt rollback baseline")
    {
        Some(base) => wechatagent::evolution::revision::prompt_revision(
            base.get_object_id("_id").expect("baseline prompt _id"),
            base.get_i32("version").expect("baseline prompt version"),
            base.get_str("content").expect("baseline prompt content"),
        ),
        None => wechatagent::evolution::revision::prompt_revision(
            ObjectId::new(),
            version,
            "missing rollback baseline",
        ),
    };
    templates
        .update_one(
            doc! { "_id": current_id, "current_version": true },
            doc! { "$set": { "source_proposal_id": id } },
            None,
        )
        .await
        .expect("bind current prompt artifact to proposal");
    state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("proposals")
        .insert_one(
            doc! {
                "_id": id,
                "experiment_id": "test-exp",
                "workspace_id": workspace,
                "account_id": state.config.default_account_id.clone(),
                "proposal_kind": "prompt",
                "status": "released",
                "proposed_template_key": key,
                "base_revision": base_revision,
                "released_revision": released_revision,
                "previous_prompt_version": prev,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("insert released prompt proposal");
    id
}

/// 同时替换 `llm`（注入真实 [`LlmProvider`]，如 [`wechatagent::llm::LlmClient`]）
/// 与 `mcp_base_url`（指向 wiremock）的 [`AppState`]，复用 [`TestApp`] 已建好的
/// Mongo 容器。
///
/// real-LLM smoke 测试用它：决策/审查链路跑**真实大模型**，但 MCP 永远是桩——
/// 绝不真发微信。`second_reviewer_llm` 保持 `None`（单脑复审），把真模型调用次数
/// 压到最小、也避开双脑分歧分支——首波只验核心链路在真模型下能否跑通。其余字段
/// 沿用 `TestApp`。
pub fn rebuild_app_state_with_real_llm(
    app: &TestApp,
    llm: Arc<dyn LlmProvider>,
    mcp_url: String,
) -> AppState {
    let mut config = app.state.config.clone();
    config.mcp_base_url = mcp_url.clone();
    let mcp = McpClient::new(mcp_url, config.mcp_api_key.clone())
        .expect("rebuild mcp client with overridden url");
    AppState {
        db: app.state.db.clone(),
        mcp,
        llm,
        llm_registry: app.state.llm_registry.clone(),
        config,
        prompt_pack_version: app.state.prompt_pack_version.clone(),
        chat_progress_bus: app.state.chat_progress_bus.clone(),
        second_reviewer_llm: None,
        chunk_locks: app.state.chunk_locks.clone(),
        chunk_event_bus: app.state.chunk_event_bus.clone(),
        jwt_keys: app.state.jwt_keys.clone(),
        auth_rate_limiter: app.state.auth_rate_limiter.clone(),
        completeness_cache: app.state.completeness_cache.clone(),
    }
}
