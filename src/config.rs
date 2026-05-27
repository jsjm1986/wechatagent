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
    pub llm_timeout_seconds: u64,
    pub llm_max_retries: u32,
    pub llm_retry_base_ms: u64,
    /// HP-1 / Task 9：Worker 进程崩溃后，状态卡在 `running` 的任务的回收阈值。
    /// `claimed_at` 早于 `now - 该秒数` 即视为 stale，由下一次 tick 重置回 `retry`。
    pub task_claim_timeout_seconds: u64,
    /// HP-3 / Task 10：reaction analysis claim 锁的超时阈值。
    /// `outcome_status="analyzing"` 但 `reaction_claimed_at` 早于 `now - 该秒数`
    /// 的 review 会被视为分析进程崩溃，允许下次 webhook 重新 claim。
    pub reaction_analysis_claim_timeout_seconds: u64,
    /// LP-14 / Task 20：webhook 限流窗口秒数（默认 60）。
    pub webhook_rate_limit_window_seconds: u32,
    /// LP-14 / Task 20：webhook 限流窗口内最大请求数（默认 30）。
    pub webhook_rate_limit_capacity: u32,
    /// M1 Strategic Planner：是否启用静默跟进扫描器。默认 false——首发关闭，
    /// 通过 `STRATEGIC_PLANNER_ENABLED=true` 显式开启；关闭时 `main.rs` 不会
    /// 启动 planner loop，等价于功能未上线，是回滚开关。
    pub strategic_planner_enabled: bool,
    /// M1 Strategic Planner：扫描循环周期秒数（默认 600 / 10 分钟）。
    pub strategic_planner_interval_seconds: u64,
    /// M1 Strategic Planner：判定 contact "静默" 的阈值小时数。
    /// `last_inbound_at < now - 该小时数` 才会被纳入候选。默认 72。
    pub strategic_planner_silent_threshold_hours: i64,
    /// M1 Strategic Planner：每个 account 当日最多 emit 多少条 follow-up 任务。
    /// 防止扫描器一次性把积压的静默联系人全部 emit 出去。默认 20。
    pub strategic_planner_daily_emit_cap: i64,
    /// M2 Strategic Planner：commitment.due_at 在 [now, now+N] 内视为 imminent，
    /// 触发 `Planner: commitment_imminent` 提前提醒。默认 8 小时。
    pub strategic_planner_commitment_imminent_window_hours: i64,
    /// M2 Strategic Planner：同一 commitment_id 在多少小时内不重复 emit。
    /// Planner 通过 `agent_events` 反查同 commitment_id 历史 emit 实现幂等。默认 24 小时。
    pub strategic_planner_commitment_emit_dedup_hours: i64,
    /// M2 Strategic Planner：customer_stage 多久未变视为停滞。
    /// 默认 14 天，比 silent 阈值 (72h) 长，避免与静默扫描器叠加。
    pub strategic_planner_stage_stagnation_threshold_days: i64,
    /// M2 Strategic Planner：比此值更近的 inbound 跳过 stage_stagnation emit。
    /// 默认 24 小时——避免用户刚说过话还没轮到推进就被 Planner 强催。
    pub strategic_planner_stage_stagnation_recent_inbound_hours: i64,
    /// M3 Strategic Planner：反馈环回看窗口小时数。
    /// 在此窗口内反查该 contact 的 `agent_run_logs.final_review_status`，
    /// 计算 block-rate；默认 24。
    pub strategic_planner_block_rate_window_hours: i64,
    /// M3 Strategic Planner：反馈环最少 run 数门槛。
    /// 窗口内 run 数 < 此值时不参与 backoff 判定（冷启动友好）。默认 3。
    pub strategic_planner_block_rate_min_runs: i64,
    /// M3 Strategic Planner：block-rate 阈值（0.0~1.0）。
    /// `blocked / (blocked + ok) >= 此值` 时当次 emit 跳过，写 backoff 事件。默认 0.6。
    pub strategic_planner_block_rate_threshold: f64,
    /// M3 Strategic Planner：是否启用 commitment / stage_stagnation 段的跨联系人优先级排序。
    /// false 时退化为 M2 自然顺序（Mongo cursor 顺序）。默认 true。
    pub strategic_planner_priority_enabled: bool,

    // ── Phase D / D3：cold contact reactivation ──

    /// Phase D / D3：是否启用冷联系人重激活扫描器（与静默扫描器互补：
    /// 关注 `last_outbound_at` 远早于 now 的 contact，由 peer_case 钩子文案推动）。
    /// 默认 false——首发关闭，通过 `COLD_CONTACT_WORKER_ENABLED=true` 显式开启。
    pub cold_contact_worker_enabled: bool,
    /// Phase D / D3：判定 contact "冷链路" 的阈值小时数。
    /// `last_outbound_at < now - 该小时数` 才会被纳入候选；默认 168（7 天）。
    pub cold_contact_threshold_hours: i64,
    /// Phase D / D3：单 account 当日最多 emit 多少条冷重激活 follow_up；
    /// 与 strategic_planner_daily_emit_cap 解耦，避免拖累常规 follow_up。默认 5。
    pub cold_contact_daily_emit_cap: i64,

    // ── agent-self-evolution M4：演化器（独立 worker） ──
    //
    // 默认全部保守值。`evolution_enabled=false` 是安装态默认；运维需显式
    // 通过 env 打开。所有阈值在 design.md §5 中有明确单位与范围说明。

    /// M4：是否启用 evolutionary worker。默认 false（安装态关停，需运维显式打开）。
    pub evolution_enabled: bool,
    /// M4：演化器主循环间隔秒数。默认 21600（6 小时）——比 strategic planner 长一档。
    pub evolution_tick_seconds: u64,
    /// M4：单次 tick 的 LLM token 预算上限。Critic LLM 触顶后整波 prompt 候选 drop。
    pub evolution_run_token_budget: i64,
    /// M4：单次 tick 的 LLM 调用次数上限。
    pub evolution_run_max_llm_calls: i32,
    /// M4：cohort 选择回看窗口小时数。默认 72。
    pub evolution_eval_window_hours: u32,
    /// M4：cohort 最少 run 数门槛。低于此值整波 cohort 视为空（不产候选）。
    pub evolution_min_replays: usize,
    /// M4：threshold 候选释放门槛——shadow eval 后 send_success_rate 提升必须 ≥ 此值。
    pub evolution_min_send_success_delta: f64,
    /// M4：prompt 候选释放门槛——self_critique_addressed_rate 提升必须 ≥ 此值。
    pub evolution_min_self_critique_delta: f64,
    /// M4：5 闸命中率任一上升不得超过此值（防止 prompt 修订引入新风险）。
    pub evolution_max_5gate_hit_increase: f64,
    /// M4：shadow replay 并发上限（tokio Semaphore 容量）。
    pub evolution_replay_concurrency: usize,
    /// M4：shadow replay 失败率上限——超过此比例直接 reject 候选。
    pub evolution_replay_max_fail_rate: f64,
    /// M4：同 gate_key 上次 release 的 cooldown 小时数；窗口内同 gate 候选直接 reject。
    pub evolution_threshold_release_cooldown_hours: u32,
    /// M4：cohort 内同一 contact_wxid 的 run 上限（去重）。
    pub evolution_cohort_per_contact_cap: usize,
    /// M4：每个 finalReviewStatus 失败桶给 Critic LLM 的样本数。
    pub evolution_cohort_sample_per_failure_bucket: usize,

    // ── Knowledge Digest Workstation ──
    //
    // 默认关停。设计见 `.kiro/specs/knowledge-digest-workstation/`。

    /// 是否启用知识库日报 worker。默认 false（安装态关停）。
    pub knowledge_digest_enabled: bool,
    /// 每天触发合成的小时（运营时区，0..=23）。默认 9。
    pub knowledge_digest_run_hour: u32,
    /// 单次 worker tick 的 LLM token 预算上限。
    pub knowledge_digest_run_token_budget: i64,
    /// 单次 worker tick 的 LLM 调用次数上限。
    pub knowledge_digest_run_max_llm_calls: i32,
    /// `KnowledgeTaskWorker` tick 间隔秒数；0 表示停掉。默认 30。
    pub knowledge_task_worker_interval_seconds: u64,
    /// knowledge-wiki Phase E：catalog rebuild worker tick 间隔秒数；0 表示停掉。默认 3。
    pub catalog_rebuild_worker_interval_seconds: u64,
    /// knowledge-wiki Phase F：feedback worker tick 间隔秒数；0 表示停掉。默认 600（10 分钟）。
    pub knowledge_feedback_interval_seconds: u64,
    /// Phase E / E2：reviewer 双脑并行开关。`true` 时 review_decision 会用第二
    /// provider 并行跑一次评分；与主 reviewer 在 `approved` 或软闸命中上分歧
    /// 即触发 single-shot revision，达到 epistemic diversity。`false`（默认）
    /// 退回单 reviewer，行为与开关引入前完全一致。
    pub reviewer_dual_enabled: bool,
    /// 第二 reviewer provider 的 base_url；`reviewer_dual_enabled=true` 但本字段
    /// 为空时启动序列拒绝（避免静默退化为单 reviewer）。
    pub reviewer_second_provider_base_url: Option<String>,
    /// 第二 reviewer provider 的 api_key。
    pub reviewer_second_provider_api_key: Option<String>,
    /// 第二 reviewer provider 的 model 名（如 `deepseek-chat` / `claude-3-5-haiku`）。
    pub reviewer_second_provider_model: Option<String>,
    /// 第二 reviewer provider 的协议形态：`openai`（默认，覆盖 OpenAI / DeepSeek /
    /// 兼容路径）/ `anthropic`。与 [`crate::llm::LlmFormat`] 同集合。
    pub reviewer_second_provider_format: String,
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
            llm_timeout_seconds: env_or("LLM_TIMEOUT_SECONDS", "45").parse()?,
            llm_max_retries: env_or("LLM_MAX_RETRIES", "5").parse()?,
            llm_retry_base_ms: env_or("LLM_RETRY_BASE_MS", "1500").parse()?,
            task_claim_timeout_seconds: env_or("TASK_CLAIM_TIMEOUT_SECONDS", "300").parse()?,
            reaction_analysis_claim_timeout_seconds: env_or(
                "REACTION_ANALYSIS_CLAIM_TIMEOUT_SECONDS",
                "60",
            )
            .parse()?,
            webhook_rate_limit_window_seconds: env_or("WEBHOOK_RATE_LIMIT_WINDOW_SECONDS", "60")
                .parse()?,
            webhook_rate_limit_capacity: env_or("WEBHOOK_RATE_LIMIT_CAPACITY", "30").parse()?,
            strategic_planner_enabled: parse_bool(&env_or("STRATEGIC_PLANNER_ENABLED", "false")),
            strategic_planner_interval_seconds: env_or("STRATEGIC_PLANNER_INTERVAL_SECONDS", "600")
                .parse()?,
            strategic_planner_silent_threshold_hours: env_or(
                "STRATEGIC_PLANNER_SILENT_THRESHOLD_HOURS",
                "72",
            )
            .parse()?,
            strategic_planner_daily_emit_cap: env_or("STRATEGIC_PLANNER_DAILY_EMIT_CAP", "20")
                .parse()?,
            strategic_planner_commitment_imminent_window_hours: env_or(
                "STRATEGIC_PLANNER_COMMITMENT_IMMINENT_WINDOW_HOURS",
                "8",
            )
            .parse()?,
            strategic_planner_commitment_emit_dedup_hours: env_or(
                "STRATEGIC_PLANNER_COMMITMENT_EMIT_DEDUP_HOURS",
                "24",
            )
            .parse()?,
            strategic_planner_stage_stagnation_threshold_days: env_or(
                "STRATEGIC_PLANNER_STAGE_STAGNATION_THRESHOLD_DAYS",
                "14",
            )
            .parse()?,
            strategic_planner_stage_stagnation_recent_inbound_hours: env_or(
                "STRATEGIC_PLANNER_STAGE_STAGNATION_RECENT_INBOUND_HOURS",
                "24",
            )
            .parse()?,
            strategic_planner_block_rate_window_hours: env_or(
                "STRATEGIC_PLANNER_BLOCK_RATE_WINDOW_HOURS",
                "24",
            )
            .parse()?,
            strategic_planner_block_rate_min_runs: env_or(
                "STRATEGIC_PLANNER_BLOCK_RATE_MIN_RUNS",
                "3",
            )
            .parse()?,
            strategic_planner_block_rate_threshold: env_or(
                "STRATEGIC_PLANNER_BLOCK_RATE_THRESHOLD",
                "0.6",
            )
            .parse()?,
            strategic_planner_priority_enabled: parse_bool(&env_or(
                "STRATEGIC_PLANNER_PRIORITY_ENABLED",
                "true",
            )),
            cold_contact_worker_enabled: parse_bool(&env_or(
                "COLD_CONTACT_WORKER_ENABLED",
                "false",
            )),
            cold_contact_threshold_hours: env_or("COLD_CONTACT_THRESHOLD_HOURS", "168").parse()?,
            cold_contact_daily_emit_cap: env_or("COLD_CONTACT_DAILY_EMIT_CAP", "5").parse()?,
            // ── agent-self-evolution M4 ──
            evolution_enabled: parse_bool(&env_or("EVOLUTION_ENABLED", "false")),
            evolution_tick_seconds: env_or("EVOLUTION_TICK_SECONDS", "21600").parse()?,
            evolution_run_token_budget: env_or("EVOLUTION_RUN_TOKEN_BUDGET", "60000").parse()?,
            evolution_run_max_llm_calls: env_or("EVOLUTION_RUN_MAX_LLM_CALLS", "30").parse()?,
            evolution_eval_window_hours: env_or("EVOLUTION_EVAL_WINDOW_HOURS", "72").parse()?,
            evolution_min_replays: env_or("EVOLUTION_MIN_REPLAYS", "30").parse()?,
            evolution_min_send_success_delta: env_or("EVOLUTION_MIN_SEND_SUCCESS_DELTA", "0.05")
                .parse()?,
            evolution_min_self_critique_delta: env_or("EVOLUTION_MIN_SELF_CRITIQUE_DELTA", "0.10")
                .parse()?,
            evolution_max_5gate_hit_increase: env_or("EVOLUTION_MAX_5GATE_HIT_INCREASE", "0.10")
                .parse()?,
            evolution_replay_concurrency: env_or("EVOLUTION_REPLAY_CONCURRENCY", "4").parse()?,
            evolution_replay_max_fail_rate: env_or("EVOLUTION_REPLAY_MAX_FAIL_RATE", "0.30")
                .parse()?,
            evolution_threshold_release_cooldown_hours: env_or(
                "EVOLUTION_THRESHOLD_RELEASE_COOLDOWN_HOURS",
                "24",
            )
            .parse()?,
            evolution_cohort_per_contact_cap: env_or("EVOLUTION_COHORT_PER_CONTACT_CAP", "3")
                .parse()?,
            evolution_cohort_sample_per_failure_bucket: env_or(
                "EVOLUTION_COHORT_SAMPLE_PER_FAILURE_BUCKET",
                "10",
            )
            .parse()?,
            // ── Knowledge Digest Workstation ──
            knowledge_digest_enabled: parse_bool(&env_or("KNOWLEDGE_DIGEST_ENABLED", "false")),
            knowledge_digest_run_hour: env_or("KNOWLEDGE_DIGEST_RUN_HOUR", "9").parse()?,
            knowledge_digest_run_token_budget: env_or(
                "KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET",
                "24000",
            )
            .parse()?,
            knowledge_digest_run_max_llm_calls: env_or(
                "KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS",
                "8",
            )
            .parse()?,
            knowledge_task_worker_interval_seconds: env_or(
                "KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS",
                "30",
            )
            .parse()?,
            catalog_rebuild_worker_interval_seconds: env_or(
                "CATALOG_REBUILD_WORKER_INTERVAL_SECONDS",
                "3",
            )
            .parse()?,
            knowledge_feedback_interval_seconds: env_or(
                "KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS",
                "600",
            )
            .parse()?,
            reviewer_dual_enabled: parse_bool(&env_or("REVIEWER_DUAL_ENABLED", "false")),
            reviewer_second_provider_base_url: env::var("REVIEWER_SECOND_PROVIDER_BASE_URL").ok(),
            reviewer_second_provider_api_key: env::var("REVIEWER_SECOND_PROVIDER_API_KEY").ok(),
            reviewer_second_provider_model: env::var("REVIEWER_SECOND_PROVIDER_MODEL").ok(),
            reviewer_second_provider_format: env_or("REVIEWER_SECOND_PROVIDER_FORMAT", "openai"),
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn require_env(key: &str) -> anyhow::Result<String> {
    env::var(key).map_err(|_| anyhow::anyhow!("missing required environment variable {key}"))
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
