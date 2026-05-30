//! Run Envelope 模块（agent-autonomy-loop W1 / Task 2.4）。
//!
//! 本模块负责 [`AgentRunLog`] 的 R0 Run Envelope 生命周期：
//!
//! * [`write_run_envelope_started`]：在任何 LLM 调用之前 `insert_one` 一条
//!   `lifecycle="started"` 的信封记录，确保即使 Reply Agent 超时 / panic /
//!   JSON 解析失败也有可追溯条目（requirements.md R0.1 / R0.5）。
//! * [`update_run_envelope_terminal`]：用 `update_one({run_id}, $set)` 落终态字段；
//!   `matched_count == 0` 时走单次 `insert_one` 兜底 + 写
//!   `agent_events kind="run_envelope_recovered_via_insert"`（R0.2）。
//! * [`install_panic_hook_for_envelope`]：注册全局 `std::panic::set_hook`，把
//!   panic message + location 通过 `tracing::error!` 输出。**实际的 lifecycle
//!   推进**仍然在 W1 task 2.5 的 `catch_unwind` 包装层完成（panic hook 不能直接
//!   调 async update_one；强行 spawn 会有 panic-in-panic 风险）。
//!
//! 使用顺序（W1 task 2.5 接入）：
//! ```text
//! write_run_envelope_started(&db, &run_id, ..).await?;
//! let result = std::panic::catch_unwind(|| run_pipeline()).unwrap_or_else(|_| failed_terminal());
//! update_run_envelope_terminal(&db, &run_id, build_terminal_fields(&result)).await?;
//! ```
//!
//! 与现有 `write_agent_run_log`（`src/agent/gateway.rs`）的关系：
//! 现阶段（W1 task 2.4）`write_agent_run_log` 仍走 `insert_one` 直接落最终
//! 字段；W1 task 2.5 会把 gateway 入口改为先调 [`write_run_envelope_started`]、
//! 主流程结束（含错误路径）调 [`update_run_envelope_terminal`]，从此告别
//! 多次 insert 引发的 DuplicateKey 风险。

use std::sync::{Arc, Once};

use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::UpdateOptions;
use serde::Serialize;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{AgentEvent, AgentRunLog};

/// 触发本次 run 的来源类别枚举（R0.1）。允许 `inbound_message / follow_up_task /
/// manual_send` 三选一；其它取值在 envelope 写入时不强制阻断（保留可观测性），
/// 由 W2 finalize 阶段判定是否视为协议违规。
pub const SOURCE_KIND_INBOUND_MESSAGE: &str = "inbound_message";
pub const SOURCE_KIND_FOLLOW_UP_TASK: &str = "follow_up_task";
pub const SOURCE_KIND_MANUAL_SEND: &str = "manual_send";

/// lifecycle 枚举（R0.3）。统一用 `&'static str` 暴露常量，避免散落字面量。
pub const LIFECYCLE_STARTED: &str = "started";
pub const LIFECYCLE_RUNNING: &str = "running";
pub const LIFECYCLE_COMPLETED: &str = "completed";
pub const LIFECYCLE_FAILED_BEFORE_DECISION: &str = "failed_before_decision";
pub const LIFECYCLE_FAILED_AFTER_DECISION: &str = "failed_after_decision";
pub const LIFECYCLE_ABORTED_BY_BUDGET: &str = "aborted_by_budget";
pub const LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL: &str = "aborted_by_external_signal";

/// gateway_status 占位 — envelope 创建时统一为 `"pending"`，由后续阶段覆盖
/// 为 `approved / blocked_by_required_field / ...` 等具体状态。
const GATEWAY_STATUS_PENDING: &str = "pending";

/// 把 run_id 错位 / matched_count == 0 的事件 kind 暴露为常量，便于 W1 task 2.6
/// 的单元测试断言。
pub const EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT: &str = "run_envelope_recovered_via_insert";

/// R9.2：`finalReviewStatus` 允许枚举集合（含 `legacy_mode_unchecked` 灰度回退态）。
/// 严禁 `held_for_human / human_required / waiting_for_human` 等暗示人工接管的取值。
///
/// 见 requirements.md "状态枚举映射表" finalReviewStatus 列。
pub const FINAL_REVIEW_STATUS_VALUES: &[&str] = &[
    "approved",
    "revision_applied_approved",
    "revision_failed",
    "held_by_ai_policy",
    "blocked_by_safety_guard",
    "ai_waiting_for_more_context",
    "blocked_by_required_field",
    "blocked_by_budget",
    "blocked_unverified_product_claim",
    "legacy_mode_unchecked",
];

/// `gateway_status` 允许枚举集合（过程态，比 finalReviewStatus 更宽）。
///
/// 见 requirements.md "状态枚举映射表" gateway_status 列。`pending` 是
/// envelope 创建时的占位（R0.1）；`allowed` 不在该映射表内但被
/// `precheck_send_gateway` 用作"通过"语义，本期允许其继续存在以避免
/// 一次性大改 gateway 内部状态机（task 3.4 finalize 阶段会重新对齐）。
pub const GATEWAY_STATUS_VALUES: &[&str] = &[
    "pending",
    "approved",
    "allowed",
    "sent",
    "no_reply",
    "review_blocked",
    "revision_failed",
    "revision_skipped_invalid_direction",
    "revision_skipped_budget_exceeded",
    "revision_llm_failure",
    "held_by_ai_policy",
    "blocked_by_safety_guard",
    "ai_waiting_for_more_context",
    "blocked_by_required_field",
    "blocked_by_budget",
    "blocked_unverified_product_claim",
    "tool_loop_timeout",
    "legacy_mode_unchecked",
    // gateway pre-block 集合（precheck_send_gateway）
    "not_managed",
    "cooldown",
    "rate_limited",
    "daily_limit",
    "expired",
    "context_changed",
    "policy_cooldown",
    "policy_wait_user_reply",
    // S5.1 (Phase 0)：补齐 gateway.rs 实际写入但漏录的两条状态。
    // - "gateway_blocked"：precheck 第二轮失败（gateway.rs:1240）
    // - "precheck_blocked"：precheck 第一轮失败的 lifecycle 推导口径
    //   （derive_lifecycle_from_status 已识别但闭集未包含）
    "gateway_blocked",
    "precheck_blocked",
    // S5.2 (Phase 0)：管理 Agent send_contact_message_gateway 改走 outbox 后
    // decision_reviews / agent_run_logs 可能写入 "outbox_enqueued" 终态。
    // dispatcher 完成 MCP 发送后会补一条 "sent"。
    "outbox_enqueued",
    // P0-7 (Phase 0)：admin SPA 显式取消任务时写入。AI 自治语义上 admin 是
    // 维护操作员（不是把对话权交给真人继续聊天），cancel_reason 字段记录管理员触发上下文。
    "admin_cancelled",
];

/// 严禁取值（R2.7 业务语义保护 + R9.2）。任何 finalReviewStatus / gateway_status
/// 取以下值之一 SHALL 视为协议违规（写库阻断），事件埋点 kind="autonomy_field_violation"
/// 在调用方处理。
const FORBIDDEN_HUMAN_HANDOFF_VALUES: &[&str] = &[
    "held_for_human",
    "human_required",
    "waiting_for_human",
    "handoff_to_human",
    "manual_takeover",
];

/// R9.10.e：写库前校验 `finalReviewStatus` 取值是否在合法枚举内。
///
/// 空字符串视为合法（envelope-started 占位语义，详见 R0.1）；其它脏值
/// SHALL 触发 `tracing::error!` + 返回 [`AppError::External`]（采用 External
/// 而非 BadRequest，因为这是内部协议违规、不是用户输入错误）。
pub fn assert_final_review_status_valid(value: &str) -> AppResult<()> {
    // 空字符串是合法占位（envelope-started 时未确定终态）
    if value.is_empty() {
        return Ok(());
    }
    if FORBIDDEN_HUMAN_HANDOFF_VALUES.contains(&value) {
        let msg = format!(
            "agent_protocol_violation: finalReviewStatus={value} is forbidden (human-handoff semantics)"
        );
        tracing::error!("{}", msg);
        return Err(AppError::External(msg));
    }
    if !FINAL_REVIEW_STATUS_VALUES.contains(&value) {
        let msg = format!(
            "agent_protocol_violation: finalReviewStatus={value} is not in the allowed enum {:?}",
            FINAL_REVIEW_STATUS_VALUES
        );
        tracing::error!("{}", msg);
        return Err(AppError::External(msg));
    }
    Ok(())
}

/// R9.10.e：写库前校验 `gateway_status` 取值是否在合法枚举内。
///
/// 空字符串视为合法（envelope-started 占位）；其它脏值与
/// `assert_final_review_status_valid` 同样的 fail-closed 处理。
pub fn assert_gateway_status_valid(value: &str) -> AppResult<()> {
    if value.is_empty() {
        return Ok(());
    }
    if FORBIDDEN_HUMAN_HANDOFF_VALUES.contains(&value) {
        let msg = format!(
            "agent_protocol_violation: gateway_status={value} is forbidden (human-handoff semantics)"
        );
        tracing::error!("{}", msg);
        return Err(AppError::External(msg));
    }
    if !GATEWAY_STATUS_VALUES.contains(&value) {
        let msg = format!(
            "agent_protocol_violation: gateway_status={value} is not in the allowed enum {:?}",
            GATEWAY_STATUS_VALUES
        );
        tracing::error!("{}", msg);
        return Err(AppError::External(msg));
    }
    Ok(())
}

/// S1.1 (Phase 0)：写库前校验 `lifecycle` 取值是否在合法枚举内。
///
/// 空字符串**不**视为合法 —— envelope 已经在 `write_run_envelope_started`
/// 写入时把 `lifecycle="started"` 落库；终态写入路径（gateway finalize）
/// SHALL 显式给出一个非空 lifecycle，否则前端筛选会落入"未完成"分桶。
pub fn assert_lifecycle_valid(value: &str) -> AppResult<()> {
    if matches!(
        value,
        LIFECYCLE_STARTED
            | LIFECYCLE_RUNNING
            | LIFECYCLE_COMPLETED
            | LIFECYCLE_FAILED_BEFORE_DECISION
            | LIFECYCLE_FAILED_AFTER_DECISION
            | LIFECYCLE_ABORTED_BY_BUDGET
            | LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL
    ) {
        return Ok(());
    }
    let msg = format!(
        "agent_protocol_violation: lifecycle={value} is not in the allowed enum (started/running/completed/failed_*/aborted_*)"
    );
    tracing::error!("{}", msg);
    Err(AppError::External(msg))
}

/// S1.1 (Phase 0)：由 `gateway_status` + 是否 budget-exceeded 推算终态 `lifecycle`。
///
/// 推算规则（与 R0.3 / R0.10 一致）：
/// * `precheck_blocked / cooldown / rate_limited / daily_limit / expired /
///   not_managed / context_changed / policy_*`：决策前被拦 → `failed_before_decision`；
/// * `blocked_by_budget` 或 `error == Some(budget_exceeded)`：→ `aborted_by_budget`；
/// * `sent / no_reply / outbox_enqueued / approved / allowed`：→ `completed`；
/// * 其它（review_blocked / blocked_by_safety_guard / held_by_ai_policy /
///   blocked_unverified_product_claim / revision_failed / tool_loop_timeout 等）：
///   决策已生成但被守门拦下 → `failed_after_decision`。
pub fn derive_lifecycle_from_status(gateway_status: &str, error: Option<&str>) -> &'static str {
    if gateway_status == "blocked_by_budget"
        || error
            .map(|e| e.contains("budget_exceeded") || e.contains("BudgetExceeded"))
            .unwrap_or(false)
    {
        return LIFECYCLE_ABORTED_BY_BUDGET;
    }
    match gateway_status {
        "sent" | "no_reply" | "approved" | "allowed" | "outbox_enqueued" => LIFECYCLE_COMPLETED,
        "not_managed" | "cooldown" | "rate_limited" | "daily_limit" | "expired"
        | "context_changed" | "policy_cooldown" | "policy_wait_user_reply"
        | "precheck_blocked" => LIFECYCLE_FAILED_BEFORE_DECISION,
        _ => LIFECYCLE_FAILED_AFTER_DECISION,
    }
}

/// R0.2 / R0.10：lifecycle 状态机合法转换判定。
///
/// 转换规则（见 requirements.md R0.3 / R0.10.b）：
/// * 同状态 → 同状态：合法（幂等 update，例如已 completed 的 run 被重复推进）；
/// * 终态（`completed / failed_before_decision / failed_after_decision /
///   aborted_by_budget / aborted_by_external_signal`）SHALL 是吸收态，
///   不允许转出到任何其它状态（含 `started`）；
/// * `started → running / completed / failed_* / aborted_*`：合法；
/// * `running → completed / failed_after_decision / failed_before_decision /
///   aborted_*`：合法；不允许回到 `started`；
/// * 任何未列出的取值（例如自由字符串）SHALL 视为非法。
///
/// 该函数纯函数，调用方（如 task 2.5 的 `update_run_envelope_terminal` 校验
/// 装饰器）可在写库前调用以阻断 panic-able 的异常转换。
pub fn is_valid_lifecycle_transition(from: &str, to: &str) -> bool {
    let is_terminal = |s: &str| {
        matches!(
            s,
            LIFECYCLE_COMPLETED
                | LIFECYCLE_FAILED_BEFORE_DECISION
                | LIFECYCLE_FAILED_AFTER_DECISION
                | LIFECYCLE_ABORTED_BY_BUDGET
                | LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL
        )
    };
    let is_known = |s: &str| {
        matches!(
            s,
            LIFECYCLE_STARTED
                | LIFECYCLE_RUNNING
                | LIFECYCLE_COMPLETED
                | LIFECYCLE_FAILED_BEFORE_DECISION
                | LIFECYCLE_FAILED_AFTER_DECISION
                | LIFECYCLE_ABORTED_BY_BUDGET
                | LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL
        )
    };

    // 任意未知字符串 → 非法（避免脏数据走通）
    if !is_known(from) || !is_known(to) {
        return false;
    }

    // 同状态 → 合法（幂等）
    if from == to {
        return true;
    }

    // 终态吸收：不允许转出到其它任何状态
    if is_terminal(from) {
        return false;
    }

    match from {
        LIFECYCLE_STARTED => matches!(
            to,
            LIFECYCLE_RUNNING
                | LIFECYCLE_COMPLETED
                | LIFECYCLE_FAILED_BEFORE_DECISION
                | LIFECYCLE_FAILED_AFTER_DECISION
                | LIFECYCLE_ABORTED_BY_BUDGET
                | LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL
        ),
        LIFECYCLE_RUNNING => matches!(
            to,
            LIFECYCLE_COMPLETED
                | LIFECYCLE_FAILED_BEFORE_DECISION
                | LIFECYCLE_FAILED_AFTER_DECISION
                | LIFECYCLE_ABORTED_BY_BUDGET
                | LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL
        ),
        _ => false,
    }
}

/// R0.1：在任何 LLM 调用之前写入信封。
///
/// 该函数 SHALL **不**包裹在 `try / catch` 里——它本身就是失败可追溯的兜底
/// 层；上层调用方拿到 `Err` 时应直接 propagate 给 webhook / worker，让其它
/// 监控（tracing / 心跳告警）跟上。
///
/// 字段填充约定：
/// * `lifecycle = "started"`；
/// * `status = "pending"`（与既有 `AgentRunLog.status` 字段语义一致，避免
///   写库时该字段为空字符串）；
/// * `gateway_result = {"gatewayStatus": "pending"}`（占位，方便前端筛选未完成
///   run）；
/// * `final_review_status = ""`（空字符串占位，由 W2 finalize 后 update）；
/// * 9 个 R3 业务字段不参与 envelope 写入（envelope 不知道 decision），
///   `decision / review / planner / context / knowledge_route` 全部用空 Document
///   占位。
///
/// 参数说明：
/// * `workspace_id` / `account_id` / `contact_wxid`：复用既有 `AgentRunLog` 字段
///   语义；其中 `account_id` 在本仓内是 `String`（不是 `ObjectId`）。
/// * `source_event_id`：触发本次 run 的入站消息 / 跟进任务 ID。空字符串走
///   兜底（不阻断写入）；R13 outbox 在拿到空值时另有 `synthetic:` 前缀兜底。
/// * `source_kind`：见上文 `SOURCE_KIND_*` 常量。
/// * `trigger_kind`：保留与既有 `AgentRunLog.trigger_kind` 字段语义一致（如
///   `"reply"` / `"follow_up"` / `"send_once"`）；W1 task 2.5 接入时可由调用
///   方按现状传入。
#[allow(clippy::too_many_arguments)]
pub async fn write_run_envelope_started(
    db: &Database,
    run_id: &str,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: Option<&str>,
    source_event_id: &str,
    source_kind: &str,
    trigger_kind: &str,
) -> AppResult<()> {
    let envelope = AgentRunLog {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: contact_wxid.map(str::to_string),
        run_id: run_id.to_string(),
        trigger_kind: trigger_kind.to_string(),
        status: GATEWAY_STATUS_PENDING.to_string(),
        planner: Document::new(),
        context: Document::new(),
        knowledge_route: Document::new(),
        decision: Document::new(),
        review: Document::new(),
        gateway_result: doc! { "gatewayStatus": GATEWAY_STATUS_PENDING },
        error: None,
        token_budget: 0,
        tokens_used: 0,
        llm_calls_used: 0,
        degraded_reasons: Vec::new(),
        lifecycle: LIFECYCLE_STARTED.to_string(),
        source_event_id: source_event_id.to_string(),
        source_kind: source_kind.to_string(),
        error_summary: None,
        abort_reason: None,
        revision_applied: false,
        revision_reason: String::new(),
        pre_revision_summary: None,
        post_revision_summary: None,
        self_critique: None,
        autonomy_mode: String::new(),
        conversation_mode: String::new(),
        conversation_mode_reason: None,
        final_review_status: String::new(),
        outbox_status: None,
        memory_consolidator_warnings: Vec::new(),
        created_at: DateTime::now(),
    };

    db.agent_run_logs().insert_one(envelope, None).await?;
    Ok(())
}

/// R0.2 / R0.4：终态字段集合。所有字段都是 `Option<T>` —— 调用方按需 set，
/// `None` 字段在 `$set` Document 里**不出现**，这样可以多次 update 而不互相
/// 覆盖（例如先落 lifecycle / final_review_status，再异步落 outbox_status）。
///
/// 该 struct 的字段名直接对应 BSON 落库 key（snake_case，与
/// `AgentRunLog` 字段命名一致）。
#[derive(Debug, Default, Clone, Serialize)]
pub struct AgentRunLogTerminalFields {
    pub lifecycle: Option<String>,
    pub status: Option<String>,
    pub planner: Option<Document>,
    pub context: Option<Document>,
    pub knowledge_route: Option<Document>,
    pub decision: Option<Document>,
    pub review: Option<Document>,
    pub gateway_result: Option<Document>,
    pub error: Option<String>,
    pub error_summary: Option<String>,
    pub abort_reason: Option<String>,
    pub token_budget: Option<i64>,
    pub tokens_used: Option<i64>,
    pub llm_calls_used: Option<i32>,
    pub degraded_reasons: Option<Vec<String>>,
    pub revision_applied: Option<bool>,
    pub revision_reason: Option<String>,
    pub pre_revision_summary: Option<String>,
    pub post_revision_summary: Option<String>,
    pub self_critique: Option<String>,
    pub autonomy_mode: Option<String>,
    pub conversation_mode: Option<String>,
    pub conversation_mode_reason: Option<String>,
    pub final_review_status: Option<String>,
    pub outbox_status: Option<String>,
    pub memory_consolidator_warnings: Option<Vec<String>>,
}

impl AgentRunLogTerminalFields {
    /// 把当前字段集合转换为 `$set` Document（None 字段会被丢弃）。
    /// 这是公开方法，便于 W1 task 2.6 单元测试直接断言 set 文档形态。
    pub fn to_set_document(&self) -> Document {
        let mut set = Document::new();
        if let Some(value) = &self.lifecycle {
            set.insert("lifecycle", value);
        }
        if let Some(value) = &self.status {
            set.insert("status", value);
        }
        if let Some(value) = &self.planner {
            set.insert("planner", value.clone());
        }
        if let Some(value) = &self.context {
            set.insert("context", value.clone());
        }
        if let Some(value) = &self.knowledge_route {
            set.insert("knowledge_route", value.clone());
        }
        if let Some(value) = &self.decision {
            set.insert("decision", value.clone());
        }
        if let Some(value) = &self.review {
            set.insert("review", value.clone());
        }
        if let Some(value) = &self.gateway_result {
            set.insert("gateway_result", value.clone());
        }
        if let Some(value) = &self.error {
            set.insert("error", value);
        }
        if let Some(value) = &self.error_summary {
            set.insert("error_summary", value);
        }
        if let Some(value) = &self.abort_reason {
            set.insert("abort_reason", value);
        }
        if let Some(value) = self.token_budget {
            set.insert("token_budget", value);
        }
        if let Some(value) = self.tokens_used {
            set.insert("tokens_used", value);
        }
        if let Some(value) = self.llm_calls_used {
            set.insert("llm_calls_used", value);
        }
        if let Some(value) = &self.degraded_reasons {
            set.insert("degraded_reasons", value.clone());
        }
        if let Some(value) = self.revision_applied {
            set.insert("revision_applied", value);
        }
        if let Some(value) = &self.revision_reason {
            set.insert("revision_reason", value);
        }
        if let Some(value) = &self.pre_revision_summary {
            set.insert("pre_revision_summary", value);
        }
        if let Some(value) = &self.post_revision_summary {
            set.insert("post_revision_summary", value);
        }
        if let Some(value) = &self.self_critique {
            set.insert("self_critique", value);
        }
        if let Some(value) = &self.autonomy_mode {
            set.insert("autonomy_mode", value);
        }
        if let Some(value) = &self.conversation_mode {
            set.insert("conversation_mode", value);
        }
        if let Some(value) = &self.conversation_mode_reason {
            set.insert("conversation_mode_reason", value);
        }
        if let Some(value) = &self.final_review_status {
            set.insert("final_review_status", value);
        }
        if let Some(value) = &self.outbox_status {
            set.insert("outbox_status", value);
        }
        if let Some(value) = &self.memory_consolidator_warnings {
            set.insert("memory_consolidator_warnings", value.clone());
        }
        set
    }
}

/// R0.2：把终态字段写入 `agent_run_logs.{run_id}`；matched_count == 0 时走
/// 单次 `insert_one` 兜底 + 写 `agent_events kind="run_envelope_recovered_via_insert"`。
///
/// 兜底 insert 路径填充必要的非空字段（`workspace_id / account_id / status /
/// lifecycle / created_at` 等），其余字段由 `fields` 覆盖；调用方应至少传入
/// `lifecycle` 字段，否则恢复后的记录 lifecycle 会留空（仍能写库，但前端筛选
/// 会落入"未完成"分桶）。
pub async fn update_run_envelope_terminal(
    db: &Database,
    run_id: &str,
    fields: AgentRunLogTerminalFields,
) -> AppResult<()> {
    // R9.10.e：写库前枚举校验。脏值 SHALL fail-closed（不写库）。
    if let Some(value) = fields.lifecycle.as_deref() {
        // P0-6（审计 D1 C-1）：终态写入路径必须把 lifecycle 也卷进闭集断言，
        // 否则 envelope-started 之后 derive_lifecycle_from_status 漏配新枚举
        // 时会静默写入闭集外脏值。
        assert_lifecycle_valid(value)?;
    }
    if let Some(value) = fields.final_review_status.as_deref() {
        assert_final_review_status_valid(value)?;
    }
    if let Some(value) = fields.status.as_deref() {
        // status 字段语义对应 gateway_status（见 R9 文档）；envelope-started
        // 占位 "pending" 也走该校验路径并合法通过。
        assert_gateway_status_valid(value)?;
    }
    if let Some(gateway_result) = &fields.gateway_result {
        if let Ok(value) = gateway_result.get_str("gatewayStatus") {
            assert_gateway_status_valid(value)?;
        }
    }

    let set_doc = fields.to_set_document();
    if set_doc.is_empty() {
        // 没有任何字段要更新：直接返回 OK，避免发空 update_one。
        return Ok(());
    }

    let update = doc! { "$set": set_doc };
    let result = db
        .agent_run_logs()
        .update_one(doc! { "run_id": run_id }, update.clone(), None)
        .await?;

    if result.matched_count > 0 {
        return Ok(());
    }

    // R0.2：兜底 insert + 事件埋点。
    tracing::error!(
        run_id = run_id,
        "agent_run_envelope_missing run_id; falling back to insert"
    );

    insert_envelope_recovery(db, run_id, &fields).await?;

    // 写一条 agent_events 让运维感知 / 可被 R0.10 单测断言。details 里把
    // 关键终态字段（lifecycle / final_review_status / autonomy_mode）原样
    // 落进去，便于追溯漏写信封的上游路径。
    let mut details = Document::new();
    if let Some(value) = &fields.lifecycle {
        details.insert("lifecycle", value);
    }
    if let Some(value) = &fields.final_review_status {
        details.insert("final_review_status", value);
    }
    if let Some(value) = &fields.autonomy_mode {
        details.insert("autonomy_mode", value);
    }
    if let Some(value) = &fields.error_summary {
        details.insert("error_summary", value);
    }
    let event = AgentEvent {
        id: None,
        // workspace_id 在恢复路径下未知；保持空字符串占位，由后续治理工具
        // 按 run_id JOIN agent_run_logs 找到正确的 workspace。
        workspace_id: String::new(),
        account_id: String::new(),
        contact_wxid: None,
        kind: EVENT_RUN_ENVELOPE_RECOVERED_VIA_INSERT.to_string(),
        status: "warning".to_string(),
        summary: format!("agent_run_envelope_missing for run_id={run_id}"),
        details: Some(doc! {
            "run_id": run_id,
            "fields": details,
        }),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    db.events().insert_one(event, None).await?;

    Ok(())
}

/// 内部：matched_count == 0 时构造一条最小可写的 [`AgentRunLog`] 写入。
async fn insert_envelope_recovery(
    db: &Database,
    run_id: &str,
    fields: &AgentRunLogTerminalFields,
) -> AppResult<()> {
    let envelope = AgentRunLog {
        id: None,
        workspace_id: String::new(),
        account_id: String::new(),
        contact_wxid: None,
        run_id: run_id.to_string(),
        // trigger_kind 在恢复路径下未知；用占位字符串，避免和 normal write 冲突。
        trigger_kind: "envelope_recovered".to_string(),
        status: fields
            .status
            .clone()
            .unwrap_or_else(|| GATEWAY_STATUS_PENDING.to_string()),
        planner: fields.planner.clone().unwrap_or_default(),
        context: fields.context.clone().unwrap_or_default(),
        knowledge_route: fields.knowledge_route.clone().unwrap_or_default(),
        decision: fields.decision.clone().unwrap_or_default(),
        review: fields.review.clone().unwrap_or_default(),
        gateway_result: fields.gateway_result.clone().unwrap_or_default(),
        error: fields.error.clone(),
        token_budget: fields.token_budget.unwrap_or(0),
        tokens_used: fields.tokens_used.unwrap_or(0),
        llm_calls_used: fields.llm_calls_used.unwrap_or(0),
        degraded_reasons: fields.degraded_reasons.clone().unwrap_or_default(),
        lifecycle: fields.lifecycle.clone().unwrap_or_default(),
        source_event_id: String::new(),
        source_kind: String::new(),
        error_summary: fields.error_summary.clone(),
        abort_reason: fields.abort_reason.clone(),
        revision_applied: fields.revision_applied.unwrap_or(false),
        revision_reason: fields.revision_reason.clone().unwrap_or_default(),
        pre_revision_summary: fields.pre_revision_summary.clone(),
        post_revision_summary: fields.post_revision_summary.clone(),
        self_critique: fields.self_critique.clone(),
        autonomy_mode: fields.autonomy_mode.clone().unwrap_or_default(),
        conversation_mode: fields.conversation_mode.clone().unwrap_or_default(),
        conversation_mode_reason: fields.conversation_mode_reason.clone(),
        final_review_status: fields.final_review_status.clone().unwrap_or_default(),
        outbox_status: fields.outbox_status.clone(),
        memory_consolidator_warnings: fields
            .memory_consolidator_warnings
            .clone()
            .unwrap_or_default(),
        created_at: DateTime::now(),
    };

    // 用 update_one + upsert 而不是 insert_one：避免和并发的"信封姗姗来迟
    // 写到 (run_id) 唯一索引"竞态。更深层语义：恢复路径的目的是"让记录最终
    // 存在"，并不严格要求 insert vs update。
    db.agent_run_logs()
        .update_one(
            doc! { "run_id": run_id },
            doc! { "$setOnInsert": mongodb::bson::to_document(&envelope)? },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// 让 panic hook 只安装一次（多次 [`install_panic_hook_for_envelope`] 调用——
/// 例如多个 worker 各自启动——应共享同一个 hook，避免栈式覆盖）。
static PANIC_HOOK_INSTALLED: Once = Once::new();

/// R0.6：注册全局 panic hook，把 panic message + location 通过 `tracing::error!`
/// 输出。
///
/// 设计权衡：
/// * Rust 的 `std::panic::set_hook` 接受 `Fn(&PanicInfo) + Sync + Send`，但**不允许**
///   直接 `.await`，因此本 hook 不会调用 `update_run_envelope_terminal`；
/// * 实际的 lifecycle 推进由 W1 task 2.5 在 `catch_unwind` 包装层完成
///   （pipeline 执行后判断是 `Ok / Err / panic`，再统一调一次 update）；
/// * 这里持有的 `Arc<Database>` 仅用于未来扩展（如 best-effort 同步写错误事件
///   到 mongo），目前签名上保留以便 W1 task 2.5 / W6 监控波直接接入。
///
/// 该函数幂等：多次调用只会安装一次 hook；后续调用直接返回。
pub fn install_panic_hook_for_envelope(_db: Arc<Database>) {
    PANIC_HOOK_INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown".to_string());
            let message = panic_message_from_info(info);
            tracing::error!(
                location = %location,
                panic_message = %message,
                "agent_run_envelope: unhandled_panic captured by hook (lifecycle update happens in catch_unwind wrapper)"
            );
            // 仍然让原 hook 跑完（保留 backtrace 行为）。
            previous(info);
        }));
    });
}

/// 从 `PanicHookInfo` 中尽力抽出 panic message（既可能是 `&str` 也可能是 `String`）。
fn panic_message_from_info(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "unknown panic payload".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::to_document;

    fn sample_terminal_fields() -> AgentRunLogTerminalFields {
        AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("approved".to_string()),
            final_review_status: Some("approved".to_string()),
            autonomy_mode: Some("auto".to_string()),
            revision_applied: Some(false),
            tokens_used: Some(1234),
            llm_calls_used: Some(2),
            ..Default::default()
        }
    }

    #[test]
    fn terminal_fields_to_set_document_skips_none() {
        let fields = sample_terminal_fields();
        let set = fields.to_set_document();
        assert_eq!(set.get_str("lifecycle").unwrap(), LIFECYCLE_COMPLETED);
        assert_eq!(set.get_str("status").unwrap(), "approved");
        assert_eq!(set.get_str("final_review_status").unwrap(), "approved");
        assert_eq!(set.get_str("autonomy_mode").unwrap(), "auto");
        assert_eq!(set.get_bool("revision_applied").unwrap(), false);
        assert_eq!(set.get_i64("tokens_used").unwrap(), 1234);
        assert_eq!(set.get_i32("llm_calls_used").unwrap(), 2);
        // None 字段不应出现在 $set 里
        assert!(!set.contains_key("error"));
        assert!(!set.contains_key("error_summary"));
        assert!(!set.contains_key("post_revision_summary"));
        assert!(!set.contains_key("outbox_status"));
        assert!(!set.contains_key("memory_consolidator_warnings"));
    }

    #[test]
    fn terminal_fields_default_produces_empty_set() {
        let fields = AgentRunLogTerminalFields::default();
        let set = fields.to_set_document();
        assert!(set.is_empty(), "default 应 不包含任何 $set 字段");
    }

    #[test]
    fn terminal_fields_partial_update_preserves_other_fields() {
        // 模拟"先落 lifecycle，再异步落 outbox_status"两次 update 的 set 文档。
        let first = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            final_review_status: Some("approved".to_string()),
            ..Default::default()
        };
        let second = AgentRunLogTerminalFields {
            outbox_status: Some("sent".to_string()),
            ..Default::default()
        };

        let first_set = first.to_set_document();
        let second_set = second.to_set_document();

        // 第二次 update 不应包含 lifecycle / final_review_status —— 否则会
        // 把第一次的覆盖掉。
        assert!(first_set.contains_key("lifecycle"));
        assert!(first_set.contains_key("final_review_status"));
        assert!(!second_set.contains_key("lifecycle"));
        assert!(!second_set.contains_key("final_review_status"));
        assert_eq!(second_set.get_str("outbox_status").unwrap(), "sent");
    }

    // ── agent-autonomy-loop W6 / Task 7.5：R9 审计字段单元测试 ──────────
    //
    // 覆盖 R9.10 / R9.10.e 在写库前对 9 字段自治协议产物的强制约束：
    // 1. 一次正常通过 run：revisionApplied=false / finalReviewStatus="approved" /
    //    autonomyMode 由 Agent 输出落库；
    // 2. revision 触发 + 二审通过 → finalReviewStatus="revision_applied_approved"
    //    且 pre/postRevisionSummary 都非空；
    // 3. revision 触发 + 二审失败 → finalReviewStatus="revision_failed"；
    // 4. shouldHold + holdCategory 走对应 finalReviewStatus 三个分支；
    // 5. finalReviewStatus="held_for_human" 等历史脏值被严格拒收
    //    （`assert_final_review_status_valid` 返回 Err，不写库）。

    #[test]
    fn audit_normal_pass_run_writes_approved_with_autonomy_mode() {
        // 一次正常通过 run：revisionApplied=false + finalReviewStatus="approved"
        // + autonomyMode="auto" 直接落库；pre/postRevisionSummary 必须为 None。
        let fields = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("approved".to_string()),
            final_review_status: Some("approved".to_string()),
            autonomy_mode: Some("auto".to_string()),
            revision_applied: Some(false),
            ..Default::default()
        };

        // R9.10.e：枚举校验通过。
        assert!(assert_final_review_status_valid("approved").is_ok());
        assert!(assert_gateway_status_valid("approved").is_ok());

        let set = fields.to_set_document();
        assert_eq!(set.get_str("final_review_status").unwrap(), "approved");
        assert_eq!(set.get_str("autonomy_mode").unwrap(), "auto");
        assert!(!set.get_bool("revision_applied").unwrap());
        // 正常 run 不应出现 revision 相关字段
        assert!(!set.contains_key("pre_revision_summary"));
        assert!(!set.contains_key("post_revision_summary"));
        assert!(!set.contains_key("revision_reason"));
    }

    #[test]
    fn audit_revision_triggered_then_approved_writes_revision_applied_approved() {
        // revision 触发 + 二审通过 → finalReviewStatus="revision_applied_approved"
        // 且 pre/postRevisionSummary 都非空；revisionApplied=true。
        let fields = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("approved".to_string()),
            final_review_status: Some("revision_applied_approved".to_string()),
            autonomy_mode: Some("assisted".to_string()),
            revision_applied: Some(true),
            revision_reason: Some("review needsRevision=true: 语气过强".to_string()),
            pre_revision_summary: Some("第一版回复（语气过强）".to_string()),
            post_revision_summary: Some("第二版改写（柔和）".to_string()),
            ..Default::default()
        };

        assert!(assert_final_review_status_valid("revision_applied_approved").is_ok());

        let set = fields.to_set_document();
        assert_eq!(
            set.get_str("final_review_status").unwrap(),
            "revision_applied_approved"
        );
        assert!(set.get_bool("revision_applied").unwrap());
        let pre = set.get_str("pre_revision_summary").unwrap();
        let post = set.get_str("post_revision_summary").unwrap();
        assert!(!pre.is_empty(), "pre_revision_summary SHALL 非空");
        assert!(!post.is_empty(), "post_revision_summary SHALL 非空");
        assert_ne!(pre, post, "改写后内容应当与原版不同");
    }

    #[test]
    fn audit_revision_triggered_then_review_failed_writes_revision_failed() {
        // revision 触发 + 二审仍失败 → finalReviewStatus="revision_failed"。
        let fields = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("revision_failed".to_string()),
            final_review_status: Some("revision_failed".to_string()),
            autonomy_mode: Some("blocked".to_string()),
            revision_applied: Some(true),
            revision_reason: Some("review needsRevision=true: 高 FactRisk".to_string()),
            pre_revision_summary: Some("第一版回复".to_string()),
            post_revision_summary: Some("第二版改写仍不达标".to_string()),
            ..Default::default()
        };

        assert!(assert_final_review_status_valid("revision_failed").is_ok());
        assert!(assert_gateway_status_valid("revision_failed").is_ok());

        let set = fields.to_set_document();
        assert_eq!(set.get_str("final_review_status").unwrap(), "revision_failed");
        assert!(set.get_bool("revision_applied").unwrap());
        assert_eq!(set.get_str("autonomy_mode").unwrap(), "blocked");
    }

    #[test]
    fn audit_should_hold_routes_to_correct_final_review_status() {
        // shouldHold + holdCategory 走对应 finalReviewStatus 的三条分支。
        // 这里逐一断言：策略保留 / 安全护栏 / 信息不足。
        let cases = [
            ("policy_hold", "held_by_ai_policy"),
            ("safety_hold", "blocked_by_safety_guard"),
            ("waiting_for_more_context", "ai_waiting_for_more_context"),
        ];
        for (hold_category, expected_status) in cases {
            // R9.10.e：每个状态都应过校验。
            assert!(
                assert_final_review_status_valid(expected_status).is_ok(),
                "{} SHALL 是合法 finalReviewStatus",
                expected_status
            );
            assert!(
                assert_gateway_status_valid(expected_status).is_ok(),
                "{} SHALL 是合法 gateway_status",
                expected_status
            );

            let fields = AgentRunLogTerminalFields {
                lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
                status: Some(expected_status.to_string()),
                final_review_status: Some(expected_status.to_string()),
                autonomy_mode: Some("blocked".to_string()),
                revision_applied: Some(false),
                ..Default::default()
            };
            let set = fields.to_set_document();
            assert_eq!(
                set.get_str("final_review_status").unwrap(),
                expected_status,
                "holdCategory={} → finalReviewStatus 不一致",
                hold_category
            );
            assert_eq!(
                set.get_str("autonomy_mode").unwrap(),
                "blocked",
                "shouldHold 时 autonomyMode SHALL = blocked"
            );
        }
    }

    #[test]
    fn audit_held_for_human_is_strictly_rejected_at_write_site() {
        // R9.10.e + R2.7：finalReviewStatus="held_for_human" 等历史脏值
        // SHALL 在写库前直接拒收，AppError::External 回传给 caller，不会进 update_one。
        for forbidden in [
            "held_for_human",
            "human_required",
            "waiting_for_human",
            "handoff_to_human",
            "manual_takeover",
        ] {
            let result = std::panic::catch_unwind(|| {
                assert_final_review_status_valid(forbidden)
            });
            // debug_assert! 在 debug 构建里会 panic；release 模式下返回 Err。
            // 两条路径都满足"严格拒收"语义，这里都接受：
            match result {
                Err(_) => { /* debug 构建：debug_assert! panic */ }
                Ok(Err(crate::error::AppError::External(msg))) => {
                    assert!(
                        msg.contains("forbidden")
                            && msg.contains(forbidden),
                        "拒收信息应当指明 forbidden 取值: {}",
                        msg
                    );
                }
                Ok(Ok(())) => panic!(
                    "{} SHALL 被严格拒收，不应返回 Ok(())",
                    forbidden
                ),
                Ok(Err(other)) => panic!(
                    "{} SHALL 返回 AppError::External, got {:?}",
                    forbidden, other
                ),
            }

            // gateway_status 同等拒收。
            let gateway_result = std::panic::catch_unwind(|| {
                assert_gateway_status_valid(forbidden)
            });
            match gateway_result {
                Err(_) => {}
                Ok(Err(crate::error::AppError::External(_))) => {}
                Ok(Ok(())) => panic!(
                    "gateway_status={} SHALL 被严格拒收",
                    forbidden
                ),
                Ok(Err(other)) => panic!(
                    "gateway_status={} SHALL 返回 AppError::External, got {:?}",
                    forbidden, other
                ),
            }
        }
    }

    #[test]
    fn audit_unknown_final_review_status_is_rejected() {
        // 任何未在 FINAL_REVIEW_STATUS_VALUES 内的字符串都 SHALL 被拒收
        // （非空 + 非合法枚举）。
        for unknown in [
            "totally_unknown",
            "approved_maybe",
            "blocked_by_unknown_reason",
        ] {
            let result = std::panic::catch_unwind(|| {
                assert_final_review_status_valid(unknown)
            });
            match result {
                Err(_) => {}
                Ok(Err(crate::error::AppError::External(msg))) => {
                    assert!(
                        msg.contains("not in the allowed enum"),
                        "拒收信息应指出不在合法集合内: {}",
                        msg
                    );
                }
                Ok(Ok(())) => panic!(
                    "{} SHALL 被严格拒收，不应返回 Ok(())",
                    unknown
                ),
                Ok(Err(other)) => panic!(
                    "{} SHALL 返回 AppError::External, got {:?}",
                    unknown, other
                ),
            }
        }
    }

    #[test]
    fn audit_empty_final_review_status_is_legal_envelope_started_placeholder() {
        // R0.1：envelope-started 占位时 finalReviewStatus="" 是合法值。
        // 该路径不应被拒收（否则会破坏 W1 信封先于 LLM 写入的不变量）。
        assert!(assert_final_review_status_valid("").is_ok());
        assert!(assert_gateway_status_valid("").is_ok());
    }

    /// S5.1 (Phase 0)：补齐三个被 gateway.rs 写入但漏录闭集的 status：
    /// - `no_reply`：should_reply=false 时 finalize 路径写入
    /// - `gateway_blocked`：precheck 第二轮失败
    /// - `precheck_blocked`：lifecycle 推导口径
    ///
    /// 这条测试是回归门——任何 PR 把它们从 `GATEWAY_STATUS_VALUES` 删掉，会
    /// 导致 prod 路径写库时 fail-closed 不写库。
    #[test]
    fn audit_phase0_s5_added_gateway_statuses_are_in_closed_set() {
        for value in &["no_reply", "gateway_blocked", "precheck_blocked"] {
            assert!(
                assert_gateway_status_valid(value).is_ok(),
                "{value} SHALL 在 GATEWAY_STATUS_VALUES 闭集内"
            );
        }
        // sanity：未补录的脏值仍然 fail-closed。
        assert!(assert_gateway_status_valid("not_a_real_status").is_err());
    }

    // ── agent-autonomy-loop W6 / Task 7.8：autonomy_mode 落库兜底单元测试 ──
    //
    // R9.2 / R9.3：autonomyMode 是 9 字段自治协议核心审计字段。
    //
    // 1. `auto / assisted / blocked` 三种合法取值均能正常落入 $set；
    // 2. 缺失（None）或非法（如 `manual`、空串）取值在写库阶段 SHALL 兜底为
    //    `blocked`（finalize_review_for_send 已经把 decision.autonomy_mode 改写
    //    为 "blocked"，本期 lib 单测验证 terminal fields 的真实行为）；
    // 3. finalReviewStatus 严格枚举校验已在 7.5 audit 测试中覆盖；这里追加
    //    一条针对 envelope-started → terminal "approved" 的端到端断言，避免
    //    回归 R9.10.e 校验栈。

    #[test]
    fn autonomy_mode_landing_accepts_three_legal_values() {
        // R9.3：auto / assisted / blocked 全部走 $set 落库。
        for mode in ["auto", "assisted", "blocked"] {
            let fields = AgentRunLogTerminalFields {
                lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
                status: Some("approved".to_string()),
                final_review_status: Some("approved".to_string()),
                autonomy_mode: Some(mode.to_string()),
                revision_applied: Some(false),
                ..Default::default()
            };
            let set = fields.to_set_document();
            assert_eq!(
                set.get_str("autonomy_mode").unwrap(),
                mode,
                "autonomyMode={} SHALL 原样落库",
                mode
            );
        }
    }

    #[test]
    fn derive_lifecycle_routes_hold_trio_to_failed_after_decision() {
        // P2-5（蜂群审查 #138）：hold 三类（held_by_ai_policy /
        // blocked_by_safety_guard / ai_waiting_for_more_context）都属于
        // "决策已生成但被守门拦下"，SHALL 落 failed_after_decision；
        // 此前 ai_waiting_for_more_context 被错配进 aborted_by_budget，
        // 与 docstring 的"budget 维度耗尽"语义和 holdBreakdown 仪表盘冲突。
        // budget 仍然只走 blocked_by_budget / error 包含 budget_exceeded。
        for status in [
            "held_by_ai_policy",
            "blocked_by_safety_guard",
            "ai_waiting_for_more_context",
            "blocked_unverified_product_claim",
            "revision_failed",
            "review_blocked",
        ] {
            assert_eq!(
                derive_lifecycle_from_status(status, None),
                LIFECYCLE_FAILED_AFTER_DECISION,
                "{status} SHALL 推算为 failed_after_decision"
            );
        }
        assert_eq!(
            derive_lifecycle_from_status("blocked_by_budget", None),
            LIFECYCLE_ABORTED_BY_BUDGET,
            "blocked_by_budget SHALL 推算为 aborted_by_budget"
        );
        assert_eq!(
            derive_lifecycle_from_status("sent", Some("budget_exceeded: ...")),
            LIFECYCLE_ABORTED_BY_BUDGET,
            "error 含 budget_exceeded SHALL 推算为 aborted_by_budget"
        );
        assert_eq!(
            derive_lifecycle_from_status("sent", None),
            LIFECYCLE_COMPLETED
        );
        assert_eq!(
            derive_lifecycle_from_status("cooldown", None),
            LIFECYCLE_FAILED_BEFORE_DECISION
        );
    }

    #[test]
    fn autonomy_mode_landing_blocked_pairs_with_blocked_status() {
        // 重点用例：autonomyMode=blocked 与 finalReviewStatus={blocked_*} 类
        // 终态共生。验证 7 个 blocked / held 类 finalReviewStatus 都能与
        // autonomyMode=blocked 同时落库（gateway/finalReviewStatus 双枚举校验
        // 都通过）。
        let blocked_finals = [
            "blocked_by_required_field",
            "blocked_by_budget",
            "blocked_unverified_product_claim",
            "held_by_ai_policy",
            "blocked_by_safety_guard",
            "ai_waiting_for_more_context",
            "revision_failed",
        ];
        for status in blocked_finals {
            let fields = AgentRunLogTerminalFields {
                lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
                status: Some(status.to_string()),
                final_review_status: Some(status.to_string()),
                autonomy_mode: Some("blocked".to_string()),
                revision_applied: Some(false),
                ..Default::default()
            };

            assert!(
                assert_final_review_status_valid(status).is_ok(),
                "{} SHALL 是合法 finalReviewStatus",
                status
            );
            assert!(
                assert_gateway_status_valid(status).is_ok(),
                "{} SHALL 是合法 gateway_status",
                status
            );

            let set = fields.to_set_document();
            assert_eq!(set.get_str("autonomy_mode").unwrap(), "blocked");
            assert_eq!(set.get_str("final_review_status").unwrap(), status);
        }
    }

    #[test]
    fn autonomy_mode_missing_terminal_field_does_not_clobber_envelope_started_placeholder() {
        // 边界：terminal fields autonomy_mode=None 时，$set 不应写入
        // `autonomy_mode` 键 —— 否则会把 envelope-started 阶段写入的占位
        // （AgentRunLog.autonomy_mode = "" 由 W1 task 2.5 落库）覆盖为 null。
        let fields = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("approved".to_string()),
            final_review_status: Some("approved".to_string()),
            autonomy_mode: None, // terminal 阶段未携带
            ..Default::default()
        };
        let set = fields.to_set_document();
        assert!(
            !set.contains_key("autonomy_mode"),
            "autonomy_mode=None SHALL NOT 进入 $set，避免覆盖 envelope-started 占位"
        );
    }

    #[test]
    fn autonomy_mode_invalid_value_caller_must_normalize_before_write() {
        // 写库层不主动改写 autonomyMode（不像 finalReviewStatus 那样 fail-closed），
        // 但保留单测以警示 caller：任何非 auto/assisted/blocked 的取值必须由
        // finalize_review_for_send 上游统一改写为 "blocked"。
        //
        // 该测试用的输入是上游已经规范化后的产物 —— 即 finalize 强制 blocked
        // 之后落库。这里断言"caller 已规范化 + lib 写库正确"两段闭环。
        let allowed = ["auto", "assisted", "blocked"];
        let normalized_after_finalize = "blocked"; // 上游 finalize 输出
        assert!(
            allowed.contains(&normalized_after_finalize),
            "finalize 输出 SHALL ∈ {{auto, assisted, blocked}}"
        );

        let fields = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("blocked_by_required_field".to_string()),
            final_review_status: Some("blocked_by_required_field".to_string()),
            autonomy_mode: Some(normalized_after_finalize.to_string()),
            revision_applied: Some(false),
            ..Default::default()
        };
        let set = fields.to_set_document();
        assert_eq!(set.get_str("autonomy_mode").unwrap(), "blocked");
    }

    #[test]
    fn autonomy_mode_lifecycle_terminal_pairs_with_approved() {
        // R9.3 反向：autonomyMode=auto + finalReviewStatus=approved + lifecycle=completed
        // 是 happy path 的标准产物。这条断言是 task 7.4 happy_path 集成测试的
        // 一面镜子，确保 lib 写库形态稳定。
        let fields = AgentRunLogTerminalFields {
            lifecycle: Some(LIFECYCLE_COMPLETED.to_string()),
            status: Some("approved".to_string()),
            final_review_status: Some("approved".to_string()),
            autonomy_mode: Some("auto".to_string()),
            revision_applied: Some(false),
            tokens_used: Some(2000),
            llm_calls_used: Some(2),
            ..Default::default()
        };

        assert!(assert_final_review_status_valid("approved").is_ok());
        assert!(assert_gateway_status_valid("approved").is_ok());

        let set = fields.to_set_document();
        assert_eq!(set.get_str("lifecycle").unwrap(), LIFECYCLE_COMPLETED);
        assert_eq!(set.get_str("autonomy_mode").unwrap(), "auto");
        assert_eq!(set.get_str("final_review_status").unwrap(), "approved");
        assert_eq!(set.get_str("status").unwrap(), "approved");
        assert!(!set.get_bool("revision_applied").unwrap());
        assert_eq!(set.get_i64("tokens_used").unwrap(), 2000);
        assert_eq!(set.get_i32("llm_calls_used").unwrap(), 2);
    }

    #[test]
    fn agent_run_log_serializes_with_default_lifecycle_field() {
        // 关键不变量：扩字段后的 AgentRunLog 仍能 BSON 序列化，
        // 且新增字段以 snake_case 落库（与 W0 task 1.2 索引保持一致）。
        let log = AgentRunLog {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            contact_wxid: Some("wxid_test".to_string()),
            run_id: "run_test".to_string(),
            trigger_kind: "reply".to_string(),
            status: GATEWAY_STATUS_PENDING.to_string(),
            planner: Document::new(),
            context: Document::new(),
            knowledge_route: Document::new(),
            decision: Document::new(),
            review: Document::new(),
            gateway_result: Document::new(),
            error: None,
            token_budget: 0,
            tokens_used: 0,
            llm_calls_used: 0,
            degraded_reasons: Vec::new(),
            lifecycle: LIFECYCLE_STARTED.to_string(),
            source_event_id: "evt_42".to_string(),
            source_kind: SOURCE_KIND_INBOUND_MESSAGE.to_string(),
            error_summary: None,
            abort_reason: None,
            revision_applied: false,
            revision_reason: String::new(),
            pre_revision_summary: None,
            post_revision_summary: None,
            self_critique: None,
            autonomy_mode: String::new(),
            final_review_status: String::new(),
            outbox_status: None,
            memory_consolidator_warnings: Vec::new(),
            conversation_mode: String::new(),
            conversation_mode_reason: None,
            created_at: DateTime::now(),
        };
        let doc = to_document(&log).expect("AgentRunLog should serialize to BSON");
        assert_eq!(doc.get_str("lifecycle").unwrap(), LIFECYCLE_STARTED);
        assert_eq!(doc.get_str("source_event_id").unwrap(), "evt_42");
        assert_eq!(
            doc.get_str("source_kind").unwrap(),
            SOURCE_KIND_INBOUND_MESSAGE
        );
        assert_eq!(doc.get_bool("revision_applied").unwrap(), false);
        assert_eq!(doc.get_str("revision_reason").unwrap(), "");
        assert_eq!(doc.get_str("autonomy_mode").unwrap(), "");
        assert_eq!(doc.get_str("final_review_status").unwrap(), "");
        // Vec<String> 默认 -> BSON 空数组
        assert_eq!(
            doc.get_array("memory_consolidator_warnings")
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn agent_run_log_round_trips_with_all_new_fields_populated() {
        // 把 R0 / R9 / R7 全部新字段填上，过 BSON 一圈再读回。
        let original = AgentRunLog {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            contact_wxid: Some("wxid_test".to_string()),
            run_id: "run_full".to_string(),
            trigger_kind: "reply".to_string(),
            status: "approved".to_string(),
            planner: Document::new(),
            context: Document::new(),
            knowledge_route: Document::new(),
            decision: Document::new(),
            review: Document::new(),
            gateway_result: Document::new(),
            error: Some("placeholder".to_string()),
            token_budget: 30000,
            tokens_used: 12345,
            llm_calls_used: 3,
            degraded_reasons: vec!["review_skipped_budget_exceeded".to_string()],
            lifecycle: LIFECYCLE_COMPLETED.to_string(),
            source_event_id: "evt_99".to_string(),
            source_kind: SOURCE_KIND_FOLLOW_UP_TASK.to_string(),
            error_summary: Some("llm_timeout: 30s".to_string()),
            abort_reason: Some("user_reaction_stop_requested".to_string()),
            revision_applied: true,
            revision_reason: "review needsRevision=true".to_string(),
            pre_revision_summary: Some("第一版回复".to_string()),
            post_revision_summary: Some("第二版改写".to_string()),
            self_critique: Some("上一版语气太硬".to_string()),
            autonomy_mode: "assisted".to_string(),
            final_review_status: "revision_applied_approved".to_string(),
            outbox_status: Some("sent".to_string()),
            memory_consolidator_warnings: vec![
                "deprecated_fact_id_not_found:abc".to_string(),
                "superseded_by_id_not_found:abc:def".to_string(),
            ],
            conversation_mode: "consultative".to_string(),
            conversation_mode_reason: Some("customer_stage:proposal_evaluation".to_string()),
            created_at: DateTime::now(),
        };

        let doc = to_document(&original).expect("serialize");
        let round_tripped: AgentRunLog =
            mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(round_tripped.lifecycle, LIFECYCLE_COMPLETED);
        assert_eq!(round_tripped.source_event_id, "evt_99");
        assert_eq!(round_tripped.source_kind, SOURCE_KIND_FOLLOW_UP_TASK);
        assert_eq!(round_tripped.error_summary.as_deref(), Some("llm_timeout: 30s"));
        assert_eq!(
            round_tripped.abort_reason.as_deref(),
            Some("user_reaction_stop_requested")
        );
        assert!(round_tripped.revision_applied);
        assert_eq!(round_tripped.revision_reason, "review needsRevision=true");
        assert_eq!(round_tripped.pre_revision_summary.as_deref(), Some("第一版回复"));
        assert_eq!(round_tripped.post_revision_summary.as_deref(), Some("第二版改写"));
        assert_eq!(round_tripped.self_critique.as_deref(), Some("上一版语气太硬"));
        assert_eq!(round_tripped.autonomy_mode, "assisted");
        assert_eq!(round_tripped.final_review_status, "revision_applied_approved");
        assert_eq!(round_tripped.outbox_status.as_deref(), Some("sent"));
        assert_eq!(round_tripped.memory_consolidator_warnings.len(), 2);
    }

    #[test]
    fn agent_run_log_deserializes_legacy_doc_without_new_fields() {
        // 模拟升级前的老 BSON：完全不含本期新字段。所有新字段应取默认值。
        let legacy = doc! {
            "workspace_id": "default",
            "account_id": "acct",
            "contact_wxid": "wxid_old",
            "run_id": "run_legacy",
            "trigger_kind": "reply",
            "status": "approved",
            "planner": {},
            "context": {},
            "knowledge_route": {},
            "decision": {},
            "review": {},
            "gateway_result": {},
            "token_budget": 0_i64,
            "tokens_used": 0_i64,
            "llm_calls_used": 0_i32,
            "degraded_reasons": Vec::<String>::new(),
            "created_at": DateTime::now(),
        };
        let log: AgentRunLog =
            mongodb::bson::from_document(legacy).expect("legacy doc must still deserialize");
        // 默认值：空字符串 / None / false / 空 Vec
        assert_eq!(log.lifecycle, "");
        assert_eq!(log.source_event_id, "");
        assert_eq!(log.source_kind, "");
        assert!(log.error_summary.is_none());
        assert!(log.abort_reason.is_none());
        assert!(!log.revision_applied);
        assert_eq!(log.revision_reason, "");
        assert!(log.pre_revision_summary.is_none());
        assert!(log.post_revision_summary.is_none());
        assert!(log.self_critique.is_none());
        assert_eq!(log.autonomy_mode, "");
        assert_eq!(log.final_review_status, "");
        assert!(log.outbox_status.is_none());
        assert!(log.memory_consolidator_warnings.is_empty());
    }

    #[test]
    fn lifecycle_constants_match_requirements_enum() {
        // R0.3 lifecycle 枚举：保证常量集合 = requirements 中列举集合。
        let expected = [
            "started",
            "running",
            "completed",
            "failed_before_decision",
            "failed_after_decision",
            "aborted_by_budget",
            "aborted_by_external_signal",
        ];
        let actual = [
            LIFECYCLE_STARTED,
            LIFECYCLE_RUNNING,
            LIFECYCLE_COMPLETED,
            LIFECYCLE_FAILED_BEFORE_DECISION,
            LIFECYCLE_FAILED_AFTER_DECISION,
            LIFECYCLE_ABORTED_BY_BUDGET,
            LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL,
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn source_kind_constants_match_requirements_enum() {
        // R0.1 source_kind 枚举。
        assert_eq!(SOURCE_KIND_INBOUND_MESSAGE, "inbound_message");
        assert_eq!(SOURCE_KIND_FOLLOW_UP_TASK, "follow_up_task");
        assert_eq!(SOURCE_KIND_MANUAL_SEND, "manual_send");
    }

    #[test]
    fn panic_message_extraction_handles_str_payload() {
        // 直接构造 panic info 比较麻烦；这里覆盖常见 payload 类型即可。
        // panic_message_from_info 的兜底分支由 String / &str 两条路径覆盖。
        // （此处用 std::panic::catch_unwind + 一个会 panic 的闭包间接验证）
        let result = std::panic::catch_unwind(|| {
            panic!("hello panic");
        });
        let payload = result.expect_err("should have panicked");
        if let Some(s) = payload.downcast_ref::<&str>() {
            assert!(s.contains("hello panic"));
        } else if let Some(s) = payload.downcast_ref::<String>() {
            assert!(s.contains("hello panic"));
        } else {
            panic!("payload should be either &str or String");
        }
    }
}

#[cfg(test)]
mod protocol_skeleton_tests {
    //! agent-autonomy-loop W1 / Task 2.6：协议骨架的纯逻辑单元测试。
    //!
    //! 涵盖 6+ 条 lib 内可断言的不变量（其中 4 条需要 MongoDB 的集成测试位于
    //! `tests/run_envelope_integration.rs`，标记 `#[ignore]`）：
    //!
    //! 1. lifecycle FSM 不接受非法转换（`completed → started` 返回 false）。
    //! 2. lifecycle 终态是吸收态（completed/failed_*/aborted_* → started 全部 false）。
    //! 3. lifecycle 合法转换正向用例：started → running → completed 等返回 true。
    //! 4. tool_calling 中间轮即使 9 字段全空也不触发协议违规（risks 为空）。
    //! 5. final 轮在 9 字段全空时按 R1.3/R1.4/R3.1 完整校验，risks 含 ≥ 7 条
    //!    `missing_required_field:*` + ≥ 4 条 `missing_required_field` 业务字段。
    //! 6. `risk_level="critical"` 触发 `invalid_enum_value:risk_level:critical`。
    //! 7. `autonomy_mode="manual"` 触发 `invalid_enum_value:autonomy_mode:manual`。
    //! 8. final_review_status / gateway_status 枚举集合与 requirements
    //!    "状态枚举映射表" 一致（按 R9.10.e 拒收 `held_for_human` 等历史脏值）。
    //!
    //! 完整覆盖（含 envelope insert 先于 LLM、DuplicateKey、matched_count == 0
    //! 兜底 + recovery event、Reply Agent panic 推进 lifecycle）由
    //! `tests/run_envelope_integration.rs` 的 `#[ignore]` 集成测试承担。

    use super::*;
    use crate::agent::runtime::UserRuntimeParameters;
    use crate::agent::types::{RawAgentDecision, ToolCallRequest};
    use mongodb::bson::Document;

    // ── helpers ─────────────────────────────────────────────────────────

    fn runtime_default() -> UserRuntimeParameters {
        UserRuntimeParameters {
            recent_message_limit: 12,
            min_reply_interval_seconds: 20,
            max_daily_touches: 3,
            max_pending_follow_ups: 3,
            follow_up_expires_hours: 48,
            cooldown_after_no_reply_hours: 24,
            fact_risk_block_at: 6,
            pressure_risk_block_at: 7,
            human_like_rewrite_below: 6,
            emotional_value_rewrite_below: 6,
            product_accuracy_block_below: 7,
            operation_state_confidence_full_review_below: 4,
            run_token_budget: 30000,
            run_max_llm_calls: 6,
            simulation_token_budget: 60000,
            reaction_token_budget: 8000,
            reaction_max_llm_calls: 2,
            autonomy_protocol_enabled: true,
            knowledge_routing_mode: "auto_tool_loop".to_string(),
            knowledge_max_tool_loops: 3,
            knowledge_max_tool_calls: 6,
            knowledge_open_slice_max_k: 4,
            knowledge_search_top_k: 8,
            outbox_poll_interval_seconds: 5,
            outbox_lease_seconds: 60,
        }
    }

    // ── 1. lifecycle FSM 不接受非法转换（核心：completed → started） ───

    #[test]
    fn lifecycle_completed_to_started_is_invalid() {
        // R0.10.b：终态 → started SHALL 返回 false（写库前的合法性检查）。
        assert!(
            !is_valid_lifecycle_transition(LIFECYCLE_COMPLETED, LIFECYCLE_STARTED),
            "completed → started SHALL 视为非法转换"
        );
    }

    // ── 2. lifecycle 终态是吸收态（含全部 4 类终态 → started） ─────────

    #[test]
    fn lifecycle_terminal_states_are_absorbing_against_started() {
        for terminal in [
            LIFECYCLE_COMPLETED,
            LIFECYCLE_FAILED_BEFORE_DECISION,
            LIFECYCLE_FAILED_AFTER_DECISION,
            LIFECYCLE_ABORTED_BY_BUDGET,
            LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL,
        ] {
            assert!(
                !is_valid_lifecycle_transition(terminal, LIFECYCLE_STARTED),
                "{} → started SHALL 非法（终态吸收）",
                terminal
            );
            assert!(
                !is_valid_lifecycle_transition(terminal, LIFECYCLE_RUNNING),
                "{} → running SHALL 非法（终态吸收）",
                terminal
            );
        }
    }

    #[test]
    fn lifecycle_terminal_to_other_terminal_is_invalid() {
        // 终态之间不允许互转（completed ↛ failed_before_decision 等）。
        assert!(!is_valid_lifecycle_transition(
            LIFECYCLE_COMPLETED,
            LIFECYCLE_FAILED_BEFORE_DECISION
        ));
        assert!(!is_valid_lifecycle_transition(
            LIFECYCLE_FAILED_AFTER_DECISION,
            LIFECYCLE_COMPLETED
        ));
        assert!(!is_valid_lifecycle_transition(
            LIFECYCLE_ABORTED_BY_BUDGET,
            LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL
        ));
    }

    // ── 3. lifecycle 合法转换正向用例 ───────────────────────────────────

    #[test]
    fn lifecycle_started_to_running_to_completed_is_valid() {
        assert!(is_valid_lifecycle_transition(
            LIFECYCLE_STARTED,
            LIFECYCLE_RUNNING
        ));
        assert!(is_valid_lifecycle_transition(
            LIFECYCLE_RUNNING,
            LIFECYCLE_COMPLETED
        ));
        // 终态 → 同终态：幂等合法（重复 update 不出错）
        assert!(is_valid_lifecycle_transition(
            LIFECYCLE_COMPLETED,
            LIFECYCLE_COMPLETED
        ));
    }

    #[test]
    fn lifecycle_started_to_terminal_is_valid() {
        // started 阶段直接 fail / abort 都合法（pre-decision panic / pre-budget abort）。
        for terminal in [
            LIFECYCLE_FAILED_BEFORE_DECISION,
            LIFECYCLE_FAILED_AFTER_DECISION,
            LIFECYCLE_ABORTED_BY_BUDGET,
            LIFECYCLE_ABORTED_BY_EXTERNAL_SIGNAL,
            LIFECYCLE_COMPLETED,
        ] {
            assert!(
                is_valid_lifecycle_transition(LIFECYCLE_STARTED, terminal),
                "started → {} SHALL 合法",
                terminal
            );
        }
    }

    #[test]
    fn lifecycle_unknown_string_is_invalid() {
        // 未在枚举内的字符串两端都视为非法（写库前阻断脏数据）。
        assert!(!is_valid_lifecycle_transition("foo", LIFECYCLE_RUNNING));
        assert!(!is_valid_lifecycle_transition(LIFECYCLE_STARTED, "bar"));
        assert!(!is_valid_lifecycle_transition("", LIFECYCLE_STARTED));
    }

    // ── 4. decision_phase=tool_calling + 9 字段全空 → 不触发协议违规 ───

    #[test]
    fn tool_calling_phase_skips_protocol_validation_with_all_empty_fields() {
        // R1.10 / R4.1.b：tool_calling 中间轮 SHALL 跳过 R1.3/R1.4/R1.5/R1.6/R3
        // 全部校验。即使 R1.3 7 字段、R3.1/R3.2/R3.3 必填字段全部为空，
        // risks 仍应为空（toolCalls 为空时也不报）。
        let raw = RawAgentDecision {
            decision_phase: Some("tool_calling".to_string()),
            // 故意 9 字段 + R3 必填字段全部留空
            ..RawAgentDecision::default()
        };
        let runtime = runtime_default();
        let (decision, risks) = raw.validate_and_promote(&runtime);

        assert_eq!(decision.decision_phase, "tool_calling");
        assert!(
            risks.is_empty(),
            "tool_calling 中间轮即使 9 字段全空也不应触发协议违规, risks={:?}",
            risks
        );
    }

    #[test]
    fn tool_calling_phase_with_valid_tool_call_does_not_trigger_protocol_risks() {
        // 与上一例对照：toolCalls 命中合法工具名，仍不触发 R1.3 missing 风险。
        let raw = RawAgentDecision {
            decision_phase: Some("tool_calling".to_string()),
            tool_calls: Some(vec![ToolCallRequest {
                tool: "knowledge.list_catalog".to_string(),
                arguments: Document::new(),
            }]),
            ..RawAgentDecision::default()
        };
        let runtime = runtime_default();
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks.is_empty(),
            "tool_calling 中间轮 + 合法 toolCalls 不应触发 R1/R3 校验, risks={:?}",
            risks
        );
    }

    // ── 5. decision_phase=final + 9 字段全空 → 完整校验 ────────────────

    #[test]
    fn final_phase_with_all_empty_fields_triggers_full_validation() {
        // R1.10 / R1.3 / R3.1 / R3.2 / R3.3：final 轮空字段应触发完整校验。
        // 最少应包含 R1.3 的 7 个字段缺失 + R3 必填的 6 个枚举/bool/state 字段缺失。
        let raw = RawAgentDecision {
            decision_phase: Some("final".to_string()),
            ..RawAgentDecision::default()
        };
        let runtime = runtime_default();
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        // R1.3 7 个字段
        let r1_3_fields = [
            "user_understanding",
            "relationship_read",
            "operation_goal",
            "knowledge_need_reason",
            "memory_update_reason",
            "self_critique",
            "risk_self_check",
        ];
        for field in r1_3_fields {
            assert!(
                risks.contains(&format!("missing_required_field:{}", field)),
                "final 轮空字段 SHALL 触发 missing_required_field:{}, risks={:?}",
                field,
                risks
            );
        }
        // R3.1/R3.2/R3.3 必填 + 枚举字段
        let r3_fields = [
            "risk_level",
            "knowledge_need",
            "run_mode",
            "autonomy_mode",
            "needs_review",
            "consolidation_needed",
            "operation_state",
        ];
        for field in r3_fields {
            assert!(
                risks.contains(&format!("missing_required_field:{}", field)),
                "final 轮空字段 SHALL 触发 missing_required_field:{}, risks={:?}",
                field,
                risks
            );
        }
    }

    // ── 6. risk_level="critical" 触发 invalid_enum_value ───────────────

    #[test]
    fn risk_level_critical_pushes_invalid_enum_value() {
        // R3.1：risk_level 仅允许 low / medium / high；critical 非法。
        let raw = RawAgentDecision {
            decision_phase: Some("final".to_string()),
            risk_level: Some("critical".to_string()),
            knowledge_need: Some("not_required".to_string()),
            run_mode: Some("fast_chat".to_string()),
            autonomy_mode: Some("auto".to_string()),
            needs_review: Some(false),
            consolidation_needed: Some(false),
            operation_state: Some("idle".to_string()),
            user_understanding: Some("unchanged".to_string()),
            relationship_read: Some("unchanged".to_string()),
            operation_goal: Some("unchanged".to_string()),
            knowledge_need_reason: Some("无须查询知识库即可回应".to_string()),
            memory_update_reason: Some("unchanged".to_string()),
            self_critique: Some("回复内容平和，无误导".to_string()),
            risk_self_check: Some("unchanged".to_string()),
            why_should_reply: Some("用户主动打招呼，及时寒暄维持关系".to_string()),
            should_reply: Some(true),
            reply_text: Some("好的".to_string()),
            ..RawAgentDecision::default()
        };
        let runtime = runtime_default();
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks
                .iter()
                .any(|r| r == "invalid_enum_value:risk_level:critical"),
            "risk_level=critical SHALL 触发 invalid_enum_value:risk_level:critical, risks={:?}",
            risks
        );
    }

    // ── 7. autonomy_mode="manual" 触发 invalid_enum_value ──────────────

    #[test]
    fn autonomy_mode_manual_pushes_invalid_enum_value() {
        // R3.3：autonomy_mode 仅允许 auto / assisted / blocked；manual 是历史
        // 脏值（暗示人工接管），SHALL 视为协议违规。
        let raw = RawAgentDecision {
            decision_phase: Some("final".to_string()),
            risk_level: Some("low".to_string()),
            knowledge_need: Some("not_required".to_string()),
            run_mode: Some("fast_chat".to_string()),
            autonomy_mode: Some("manual".to_string()),
            needs_review: Some(false),
            consolidation_needed: Some(false),
            operation_state: Some("idle".to_string()),
            user_understanding: Some("unchanged".to_string()),
            relationship_read: Some("unchanged".to_string()),
            operation_goal: Some("unchanged".to_string()),
            knowledge_need_reason: Some("无须查询知识库即可回应".to_string()),
            memory_update_reason: Some("unchanged".to_string()),
            self_critique: Some("回复内容平和，无误导".to_string()),
            risk_self_check: Some("unchanged".to_string()),
            why_should_reply: Some("用户主动打招呼，及时寒暄维持关系".to_string()),
            should_reply: Some(true),
            reply_text: Some("好的".to_string()),
            ..RawAgentDecision::default()
        };
        let runtime = runtime_default();
        let (_decision, risks) = raw.validate_and_promote(&runtime);

        assert!(
            risks
                .iter()
                .any(|r| r == "invalid_enum_value:autonomy_mode:manual"),
            "autonomy_mode=manual SHALL 触发 invalid_enum_value, risks={:?}",
            risks
        );
    }

    // ── 8. final_review_status / gateway_status 历史脏值识别 ──────────
    //
    // 这两条用例由 task 2.5 的 `assert_final_review_status_valid` /
    // `assert_gateway_status_valid` 在写库前真正阻断；本期 task 2.6 还在并行
    // 编写，因此本测试只断言**枚举常量集合**（不依赖 task 2.5 是否落地），
    // 用于跟 requirements.md "状态枚举映射表" + R9.10.e 对齐。

    /// requirements.md "状态枚举映射表" 中 finalReviewStatus 列举的全部合法取值
    /// （R9.2）。
    const FINAL_REVIEW_STATUS_ALLOWED: &[&str] = &[
        "approved",
        "revision_applied_approved",
        "revision_failed",
        "held_by_ai_policy",
        "blocked_by_safety_guard",
        "ai_waiting_for_more_context",
        "blocked_by_required_field",
        "blocked_by_budget",
        "blocked_unverified_product_claim",
        "legacy_mode_unchecked",
    ];

    /// 历史脏值（R9.2 / R9.10.e 严格拒收 — 暗示人工接管）。
    const FINAL_REVIEW_STATUS_FORBIDDEN: &[&str] = &[
        "held_for_human",
        "human_required",
        "waiting_for_human",
    ];

    #[test]
    fn final_review_status_allowed_set_excludes_human_handoff_values() {
        for forbidden in FINAL_REVIEW_STATUS_FORBIDDEN {
            assert!(
                !FINAL_REVIEW_STATUS_ALLOWED.contains(forbidden),
                "finalReviewStatus 合法集合 SHALL NOT 含历史脏值 {}",
                forbidden
            );
        }
    }

    #[test]
    fn final_review_status_allowed_set_contains_all_documented_values() {
        // 与 requirements.md "状态枚举映射表" 同步：10 个合法终态。
        assert_eq!(
            FINAL_REVIEW_STATUS_ALLOWED.len(),
            10,
            "finalReviewStatus 合法集合应有 10 个取值（与 requirements 同步）"
        );
        // 关键回归：approved / revision_applied_approved / blocked_by_safety_guard
        // 这三个被 R9.5 / R10 多处引用的取值必须在集合内。
        for must_have in [
            "approved",
            "revision_applied_approved",
            "blocked_by_safety_guard",
            "legacy_mode_unchecked",
        ] {
            assert!(
                FINAL_REVIEW_STATUS_ALLOWED.contains(&must_have),
                "{} SHALL 在 finalReviewStatus 合法集合内",
                must_have
            );
        }
    }
}
