use mongodb::bson::{oid::ObjectId, DateTime, Document};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Normal,
    Managed,
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    #[serde(default)]
    pub summary: String,
    #[serde(default, deserialize_with = "string_or_vec")]
    pub interests: Vec<String>,
    #[serde(default)]
    pub communication_style: String,
    #[serde(default)]
    pub operation_goal: String,
}

fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if let Some(items) = value.as_array() {
        return Ok(items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect());
    }
    if let Some(text) = value.as_str() {
        return Ok(text
            .split([',', '，', '\n', ';', '；'])
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect());
    }
    Ok(Vec::new())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WechatAccount {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub alias: String,
    pub display_name: String,
    pub app_id: Option<String>,
    pub wxid: Option<String>,
    pub nick_name: Option<String>,
    pub mcp_base_url: Option<String>,
    pub mcp_api_key: Option<String>,
    pub online: bool,
    pub last_sync_at: DateTime,
    /// Phase D / D4：单 account 在调度窗口内允许承接的并发联系人上限。
    /// 0 表示未配置（视为不参与多账号轮询，落回单 account 行为）。
    #[serde(default)]
    pub capacity: u32,
    /// Phase D / D4：账号 persona 标签（`sales_assistant` / `support` /
    /// `community_ops` 等运营自定义），调度器把同 persona 的 account 视为
    /// 互替；跨 persona 不互替，避免风格漂移。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_tag: Option<String>,
    /// Phase D / D4：账号每日"勿打扰"窗口（小时区间，运营时区）。
    /// 命中任一区间时该账号 round-robin 跳过，不被分配新 contact。
    #[serde(default)]
    pub off_hours: Vec<HourRange>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// 手写 `Debug`：`mcp_api_key` 走 [`crate::secret::mask_secret`] 掩码，避免
/// `tracing::*!(?account, ...)` / panic backtrace 把真值泄漏到日志后端。
impl std::fmt::Debug for WechatAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WechatAccount")
            .field("id", &self.id)
            .field("workspace_id", &self.workspace_id)
            .field("account_id", &self.account_id)
            .field("alias", &self.alias)
            .field("display_name", &self.display_name)
            .field("app_id", &self.app_id)
            .field("wxid", &self.wxid)
            .field("nick_name", &self.nick_name)
            .field("mcp_base_url", &self.mcp_base_url)
            .field(
                "mcp_api_key",
                &self.mcp_api_key.as_deref().map(crate::secret::mask_secret),
            )
            .field("online", &self.online)
            .field("last_sync_at", &self.last_sync_at)
            .field("capacity", &self.capacity)
            .field("persona_tag", &self.persona_tag)
            .field("off_hours", &self.off_hours)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Phase D / D4：账号"勿打扰"小时区间。运营时区下的 [start_hour, end_hour) 半开
/// 区间；跨午夜（如 22-2）合法，调度器内部会拆成两段比较。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HourRange {
    /// 0..=23
    #[serde(default)]
    pub start_hour: u32,
    /// 0..=24（24 表示当日结尾，便于 0..24 表达"全天关停"）
    #[serde(default)]
    pub end_hour: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    pub agent_status: AgentStatus,
    pub human_profile_note: Option<String>,
    /// per-contact 运营人员特别指令（最高优先级 Operator Instruction 层）。
    /// 注入到 user.reply system prompt 末位，覆盖 Soul + Policy 默认行为。
    /// 上限 1000 字符，由 `PUT /api/contacts/:id/custom-agent-instructions` 维护。
    #[serde(default)]
    pub custom_agent_instructions: Option<String>,
    /// universal-domain-adaptation H8：单客户运营范式覆盖（优先级高于
    /// `DomainProfile.operation_mode`，承接「因用户而异」）。`None` → 用 profile 范式。
    /// 例：某老客户「只维护不推进」→ 设 `operation_mode_override.funnel.enabled=false`，
    /// 单独对他关漏斗，不影响同号其他客户。解析见
    /// [`resolve_operation_mode`](crate::planner::resolve_operation_mode)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_mode_override: Option<OperationMode>,
    pub agent_profile: Option<AgentProfile>,
    pub memory_summary: Option<String>,
    pub playbook_id: Option<ObjectId>,
    pub playbook_version: Option<i32>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 业务字段 JSON 容器（由 DomainSchema 在写入时校验）。
    /// 取代旧的销售域硬编码 `customer_stage / intent_level / objection_type`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_attributes: Option<Document>,
    /// `domain_attributes` 上次更新时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_attributes_updated_at: Option<DateTime>,
    /// agent-autonomy-loop M2：结构化承诺列表（cap 8）。
    ///
    /// 取代原 `last_commitment: Option<String>` 自由文本字段，让 Planner
    /// `scan_commitments` 能按 `due_at` 判定 overdue/imminent。
    /// `serde(default)` 保证旧文档反序列化为空 Vec；迁移
    /// `2026_05_008_contact_commitments_reshape` 把旧 `last_commitment`
    /// 一次性转为单元素 `Vec<CommitmentRepr::Structured>` 并 `$unset` 旧字段。
    #[serde(default)]
    pub commitments: Vec<CommitmentRepr>,
    pub follow_up_policy: Option<String>,
    pub operation_state: Option<String>,
    pub operation_state_reason: Option<String>,
    pub operation_state_confidence: Option<i32>,
    pub operation_state_updated_at: Option<DateTime>,
    pub cooldown_until: Option<DateTime>,
    #[serde(default)]
    pub operation_policy: Document,
    #[serde(default)]
    pub profile_attributes: Document,
    pub profile_updated_at: Option<DateTime>,
    pub last_message_at: Option<DateTime>,
    pub last_inbound_at: Option<DateTime>,
    pub last_outbound_at: Option<DateTime>,
    pub last_agent_run_at: Option<DateTime>,
    /// Phase D / D2：上次出站回复的风格指纹（长度桶 / emoji / 标点 / 句式特征
    /// 拼接而成的短串）。reviewer 在新 draft 通过审批前与本字段比对，差异过大时
    /// 触发 single-shot revision，避免单 contact 不同回合风格漂移。
    /// 缺字段时反序列化为 None，向前兼容历史 contact 文档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outbound_style: Option<String>,
    /// Phase D / D1：意图轨迹滑窗（最近 50 项）。`record_user_reaction` 完成后
    /// 追加一条；`knowledge_router` 在装 prompt 时拼接近 N 项（默认 5）。
    /// 缺字段时反序列化为空 Vec，向前兼容历史 contact 文档。
    #[serde(default)]
    pub intent_trajectory: Vec<IntentTrajectoryEntry>,
    /// 自学习采集管道 S5：admin 手动标记的成交事件（正例-only，稀疏 + 延迟）。
    /// 成交（T0 硬事件）不可从 WeChat 文字入站观测，只能由运营人员手动登记；
    /// 本字段是未来 PU-learning / 延迟反馈归因的唯一正例来源。本阶段只采集、
    /// 不参与任何评分或置信反推。缺字段时反序列化为空 Vec，向前兼容历史文档。
    #[serde(default)]
    pub deal_events: Vec<DealEvent>,
    /// Phase E / E3：联系人语种（BCP-47 短形式，如 `zh-CN` / `en-US`）。
    /// `load_prompt_for_contact` 在 prompt_templates 多 locale 并存时按本字段
    /// 选最匹配版本；缺字段时反序列化为 None，由 `contact_locale_or_default`
    /// 回退到 [`DEFAULT_LOCALE`]（`zh-CN`），向前兼容历史 contact 文档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// 自学习采集管道 S5：单条成交事件（admin 手动标记）。
///
/// 设计取舍——成交是 T0 硬事件，但 WeChat 私聊入站只有文字，系统无法自动观测
/// 支付/下单；唯一可信来源是运营人员事后登记。因此该结构刻意只承载"被标记的
/// 客观事实"（标记时间、可选实际发生时间、可选金额），不含任何 LLM 解释或
/// 置信度反推。本阶段仅落库，作为未来归因/PU-learning 的正例。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DealEvent {
    /// admin 在后台点击"标记成交"的时间。
    pub marked_at: DateTime,
    /// 成交实际发生时间（admin 可回填；缺省时下游用 `marked_at` 近似）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<DateTime>,
    /// 成交金额（可选，业务自行决定是否登记）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    /// 金额币种（ISO-4217 短码，如 `CNY`）；与 `amount` 配套。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// 事件来源；本阶段恒 `"manual"`（将来可扩 `"imported"` 等）。
    #[serde(default)]
    pub source: String,
    /// 标记人（admin 用户名/ID），用于审计。
    #[serde(default)]
    pub marked_by: String,
    /// 备注（可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 自学习采集管道 S1–S3：行为信号（append-only 事件日志）。
///
/// 方法论铁律落到结构上的体现：
/// - **观察与解释分层**（Law ③）：本结构只存"系统直接观察到的客观量"
///   （延迟毫秒、字符数、沉默时长、是否重新激活），**不存任何 LLM 解释标签**。
///   解释层另由 `agent_decision_reviews.reaction_analysis` 承载。
/// - **沉默 = 删失**（Law ②）：`signal_type="silence"` 的事件 `censored=true`，
///   下游绝不可当作负反馈喂入任何评分。
/// - **每条信号带元数据**（Law ④）：`source`/`observed_at`/`confidence`/`dedupe_key`。
/// - **幂等**（Law ⑤）：`(workspace_id, dedupe_key)` partial unique index 约束，
///   同一观察重复采集只落一次。
///
/// 本阶段只采集、不消费——任何学习公式都不读它，直到积累到可学样本量。
///
/// 字段一律 snake_case 落库（与 [`ConversationMessage`] 等同库结构一致）：索引
/// `{workspace_id, dedupe_key}` 的 `partialFilterExpression` 按 snake-case 字段名
/// 匹配，若改成 camelCase 会让 partial filter 命中 0 文档 → unique 约束形同虚设、
/// 重复信号全部落库（已被 `behavior_signal_smoke` 集成测试逮到）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BehaviorSignal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub contact_wxid: String,
    /// `"reply_latency"` | `"reply_length"` | `"reactivation"` | `"silence"`。
    pub signal_type: String,
    /// 信号的**事实发生时刻**（`event_time`）——观察到的客观事件何时发生。
    /// 与 [`Self::ingest_time`]（落库时刻）配对：未来训练样本必须按 `event_time`
    /// 切片防 label leakage（成交是延迟事件），而非按写入时刻。语义不改名以避免迁移。
    pub observed_at: DateTime,
    /// 信号来源；系统观察恒 `"system_observed"`。
    pub source: String,
    /// 观察置信度；系统直接观测恒 `1.0`（LLM 解释信号才会 <1.0，不在本表）。
    pub confidence: f64,
    /// 删失标记：`silence` 事件为 `true`，其余为 `false`。删失 ≠ 负例。
    pub censored: bool,
    /// 幂等键。约定：
    /// - reply_latency / reply_length：`"{type}:{wxid}:{inbound_message_id}"`
    /// - reactivation：`"reactivation:{wxid}:{inbound_message_id}"`
    /// - silence：`"silence:{wxid}:{last_outbound_at_ms}"`
    pub dedupe_key: String,

    // ── 按 signal_type 选填的客观量（只存观察，不存解释）──
    /// reply_latency：上一条 outbound → 本条 inbound 的间隔毫秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<i64>,
    /// reply_length：本条 inbound 文本的字符数（`chars().count()`，多字节安全）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_len: Option<i64>,
    /// silence：从哪条 outbound 起算沉默。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silence_since: Option<DateTime>,
    /// silence：已沉默时长（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silence_ms: Option<i64>,
    /// silence：该 outbound 至今是否仍无回复。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unanswered: Option<bool>,
    /// reactivation：久未联系后重新入站的时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactivated_at: Option<DateTime>,
    /// 写入 DB 的时刻（`ingest_time`），与 [`Self::observed_at`]（`event_time`）配对。
    /// P2 双时间戳：训练时按 `observed_at` 切片防 label leakage，`ingest_time` 仅供
    /// 数据工程审计（采集延迟 / 回填识别）。旧文档缺此字段 `#[serde(default)]` 回落 None（R11）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest_time: Option<DateTime>,
}

/// P3 采集健康度：行为信号采集管道的**计数落库**（Data Cascades 教训：
/// best-effort 旁路若只 warn，管道挂了无感知）。
///
/// `_id` 形如 `"{workspace_id}:{date}"`（镜像 [`AgentOutcomeMetric`] 的聚合幂等模式），
/// 每天每 workspace 一行，`$inc` 累加三态计数 + `$set` 时间戳。三指标
/// （Monte Carlo 数据可观测性子集）：
/// - **新鲜度**：`last_success_at`（最近一次成功写入）；
/// - **量**：`persisted`（真正落库条数，对历史基线判跌量）；
/// - **失败率**：`errors` / `dedupe_skipped`（去重撞键 vs 真失败分开计）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorSignalMetric {
    #[serde(rename = "_id")]
    pub id: String,
    pub workspace_id: String,
    pub date: String,
    /// 真正写入 DB 的信号条数（`persist_signal` 返回 `Ok(true)`）。
    #[serde(default)]
    pub persisted: i64,
    /// 撞 dedupe partial unique 索引被幂等跳过的条数（`Ok(false)`）——正常现象，非失败。
    #[serde(default)]
    pub dedupe_skipped: i64,
    /// 真失败条数（`persist_signal` 返回 `Err`）——管道健康度告警依据。
    #[serde(default)]
    pub errors: i64,
    /// 最近一次成功写入时刻（新鲜度指标；长时间不更新 = 采集断流）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<DateTime>,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub message_id: Option<String>,
    #[serde(default)]
    pub dedupe_key: Option<String>,
    pub direction: MessageDirection,
    pub content: String,
    pub raw: Option<Document>,
    pub created_at: DateTime,
}

/// relay 合成消息的哨兵前缀。decision prompt 见到它即进入"转述模式"（见 user.reply.task prompt 的 relay 输入契约）。
pub const PRINCIPAL_RELAY_SENTINEL: &str = "__PRINCIPAL_RELAY__";

impl ConversationMessage {
    /// 构造一条"领导已裁决"的合成 inbound，仅用于触发 relay 转述，不落客户可见会话。
    /// 以哨兵前缀开头 + 结构化裁决载荷；decision prompt 据哨兵进入转述模式。
    pub fn synthetic_principal_relay(
        contact: &Contact,
        verdict: &str,
        substance: &str,
        constraints: &[String],
    ) -> Self {
        let constraint_text = if constraints.is_empty() {
            "（无）".to_string()
        } else {
            constraints.join("；")
        };
        let payload = format!(
            "{PRINCIPAL_RELAY_SENTINEL}\nverdict={verdict}\nsubstance={substance}\nconstraints={constraint_text}"
        );
        ConversationMessage {
            id: None,
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: payload,
            raw: None,
            created_at: DateTime::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub kind: String,
    pub run_at: DateTime,
    pub expires_at: Option<DateTime>,
    pub content: String,
    pub status: String,
    pub source_decision_id: Option<ObjectId>,
    #[serde(default)]
    pub review_required: bool,
    #[serde(default)]
    pub attempt_count: i32,
    #[serde(default)]
    pub max_attempts: i32,
    pub next_retry_at: Option<DateTime>,
    pub gateway_status: Option<String>,
    pub cancel_reason: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub claimed_at: Option<DateTime>,
    #[serde(default)]
    pub claim_recovery_count: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// `AgentTask.status` 的封闭枚举（follow_up_tasks 集合）。所有写入路径
/// 在 `$set: { status: ... }` 之前必须经过 [`assert_agent_task_status_valid`]
/// 校验，避免把闭集外脏值写进 DB。
///
/// 历史值清单（来自审计 D1 C-2/3/4）：
/// - `pending / running / retry / failed / cancelled`：reclaim / claim / 重试 / 终态
/// - `sent`：outcome_aggregation / memory_consolidation 完成态
/// - `completed`：保留为 R10 reset 一致 alias
/// - `outbox_enqueued`：gateway 把决策交付给 outbox 后写回的 task 终态
pub const ALLOWED_AGENT_TASK_STATUS: &[&str] = &[
    "pending",
    "running",
    "retry",
    "failed",
    "cancelled",
    "sent",
    "completed",
    "outbox_enqueued",
];

/// 任意 `agent_tasks.status` 写入站点的闭集断言。命中闭集外值时 panic（debug）
/// 或 `tracing::error!` + 拒绝写入（release）。R9.10.e 闭集断言契约的 task 表对应。
#[track_caller]
pub fn assert_agent_task_status_valid(status: &str) {
    if !ALLOWED_AGENT_TASK_STATUS.contains(&status) {
        let msg = format!(
            "agent_tasks.status='{status}' 不在 ALLOWED_AGENT_TASK_STATUS 闭集 {ALLOWED_AGENT_TASK_STATUS:?}"
        );
        debug_assert!(false, "{msg}");
        tracing::error!(target: "agent_protocol_violation", "{msg}");
    }
}

#[cfg(test)]
mod agent_task_status_tests {
    use super::{assert_agent_task_status_valid, ALLOWED_AGENT_TASK_STATUS};

    #[test]
    fn closed_set_covers_all_known_writers() {
        // P0-5：固化审计 D1 C-2/3/4 列出的全部写入面（gateway / tasks / memory /
        // outcome_aggregation / cancel_task）。新增 status 必须先扩闭集。
        for s in [
            "pending",
            "running",
            "retry",
            "failed",
            "cancelled",
            "sent",
            "completed",
            "outbox_enqueued",
        ] {
            assert!(ALLOWED_AGENT_TASK_STATUS.contains(&s), "{s} 缺失");
            assert_agent_task_status_valid(s);
        }
    }

    #[test]
    #[should_panic(expected = "ALLOWED_AGENT_TASK_STATUS")]
    fn unknown_status_panics_in_debug() {
        assert_agent_task_status_valid("queued");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub details: Option<Document>,
    pub created_at: DateTime,
    /// P1-2：可选事件级去重锚点。携带 `dedupe_key` 的事件由
    /// partial unique index（`workspace_id + dedupe_key`，filter
    /// `dedupe_key: { $type: "string" }`）原子约束，避免并发下 TOCTOU 重复写。
    /// 既有调用方不携带此字段时不参与去重，保持向后兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub applied_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCallLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub tool_name: String,
    pub request: Document,
    pub response: Option<Document>,
    pub error: Option<String>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAsset {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub url: Option<String>,
    pub media_id: Option<String>,
    pub usage_scene: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSoul {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub agent_kind: String,
    pub name: String,
    pub content: String,
    pub status: String,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub prompt_key: String,
    pub agent_kind: String,
    pub layer: String,
    pub title: String,
    pub description: Option<String>,
    pub content: String,
    pub status: String,
    pub version: i32,
    pub prompt_pack_version: String,
    pub created_by: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    /// agent-self-evolution M4 / W0 Task 1.5：多版本支持。`true` 表示当前生效版本。
    /// 同 `(workspace_id, prompt_key)` 下应至多有一条 `current_version=true`，由
    /// `release_prompt` 在 mongo 事务内保证。缺字段时反序列化为 `false`，
    /// `2026_05_M4_001_prompt_template_versioned` 迁移把历史唯一一条置 `true`。
    #[serde(default)]
    pub current_version: bool,
    /// agent-self-evolution M4：被替换的上一版本号（rollback 时取回它）。
    /// `release_prompt` 写入新条时设为旧条的 `version`；首次 seed 为 `None`。
    pub previous_version: Option<i32>,
    /// agent-self-evolution M4：写入来源（`"system"` / `"legacy_migration"` /
    /// `"evolution_release"` 等），方便排查谁改的。
    pub seeded_by: Option<String>,
    /// Phase E / E3：模板语种（BCP-47 短形式，如 `zh-CN` / `en-US`）。
    /// `load_prompt_for_contact` 优先选 `(workspace_id, prompt_key, locale)` 三元
    /// 全匹配的 active 版本；未命中时 fallback 到 `DEFAULT_LOCALE` 的版本。
    /// 缺字段时反序列化为 None，由 `template_locale_or_default` 回退到 `zh-CN`，
    /// 与历史模板（无 locale 字段）兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPlaybook {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub name: String,
    pub description: Option<String>,
    pub method_prompt: String,
    pub profile_method: Option<String>,
    pub tag_method: Option<String>,
    pub stage_method: Option<String>,
    pub intent_method: Option<String>,
    pub follow_up_method: Option<String>,
    pub reply_style: Option<String>,
    pub forbidden_rules: Option<String>,
    pub success_criteria: Option<String>,
    pub created_by: String,
    pub is_default: bool,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDomainConfig {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub domain: String,
    pub name: String,
    pub goal: String,
    pub methodology: String,
    pub workflow: String,
    pub tool_policy: String,
    pub automation_policy: String,
    pub review_policy: String,
    #[serde(default)]
    pub runtime_parameters: Document,
    #[serde(default)]
    pub state_machine: Document,
    pub status: String,
    pub updated_at: DateTime,
    /// Phase E / E5-T1：多版本灰度。同 `(workspace_id, domain)` 下 `version` 单调递增；
    /// `m015` 迁移把历史唯一一条 backfill 为 `version=1`。新版本由
    /// `routes::operation_domains::publish_operation_domain_version` 写入并切换 current。
    #[serde(default = "default_version_one")]
    pub version: i32,
    /// Phase E / E5-T1：当前生效版本标记。同 `(workspace_id, domain)` 下应至多有一条
    /// `current_version=true`，由 publish 路径在切换时事务性保证；缺字段反序列化为 false，
    /// 由迁移把历史唯一一条置 true。
    #[serde(default)]
    pub current_version: bool,
    /// Phase E / E5-T1：被替换的上一版本号（rollback 时取回它）。首次 seed 为 None。
    #[serde(default)]
    pub previous_version: Option<i32>,
    /// Phase E / E5-T1：写入来源（`"system"` / `"legacy_migration"` / `"manual"` 等）。
    #[serde(default)]
    pub seeded_by: Option<String>,
    /// 请示通道：接收请示卡的领导 wxid（须是业务号好友）。None=本 workspace 未启用请示通道。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_decider: Option<String>,
    /// 高风险件升级模式："all"=所有被静默 hold 的高风险件都请示真人；
    /// "decision_only"=只升级实质需决策/授权的件。None/缺省 = "decision_only"（保守）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_risk_escalation_mode: Option<String>,
}

fn default_version_one() -> i32 {
    1
}

/// universal-domain-adaptation H8：`OperationMode` 三驱动力 `enabled` 字段的
/// serde 默认值 = `true`（缺字段 / 旧文档 → 驱动力默认开启 = 当前销售域行为）。
fn default_true() -> bool {
    true
}

/// Phase B / B4：`operation_state_policies` collection 行结构。
///
/// 目的：把"该状态允许 / 禁止 agent 做哪类动作"从 `OperationDomainConfig.state_machine`
/// 里抽出来独立维护，让运营人员可以在不动状态机本身的情况下迭代 send 策略。
///
/// 唯一键：`(workspace_id, domain, state_key)`。`enforce_state_action_policy`
/// 在 review 通过后再扣一道：若 policy 命中 forbidden 列表，则 reply 拦截。
///
/// `recommended_pace` 是软提示（如 `"slow"` / `"normal"` / `"hold"`），仅作为
/// 后续 follow-up worker 的节奏建议，不参与硬拦截。
///
/// 兼容性：`status="active"` 才参与拦截；老库无此 collection 时 `enforce_*`
/// 直接 fallthrough（向前兼容，避免 Phase B 引入新边界破坏既有部署）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationStatePolicy {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub domain: String,
    pub state_key: String,
    /// 该 state 下允许的 action 类型。空数组表示"全部允许"（白名单不启用）。
    #[serde(default)]
    pub allowed: Vec<String>,
    /// 该 state 下禁止的 action 类型。命中即拦截。优先级高于 `allowed`。
    #[serde(default)]
    pub forbidden: Vec<String>,
    /// 推荐节奏（软提示）：`"slow" / "normal" / "hold"`。
    #[serde(default)]
    pub recommended_pace: Option<String>,
    /// `"active" / "draft"`。仅 `active` 参与拦截。
    pub status: String,
    pub updated_at: DateTime,
    /// Phase E / E5-T1：多版本灰度。同 `(workspace_id, domain, state_key)` 下 `version`
    /// 单调递增；`m015` 迁移 backfill 为 1。新版本由 admin REST publish 写入并切换 current。
    #[serde(default = "default_version_one")]
    pub version: i32,
    /// Phase E / E5-T1：当前生效版本标记。同 `(workspace_id, domain, state_key)` 下应至多
    /// 有一条 `current_version=true`，由 publish 路径在切换时事务性保证。
    #[serde(default)]
    pub current_version: bool,
    /// Phase E / E5-T1：被替换的上一版本号。
    #[serde(default)]
    pub previous_version: Option<i32>,
    /// Phase E / E5-T1：写入来源标签。
    #[serde(default)]
    pub seeded_by: Option<String>,
}

/// Phase C / C3：`evolution_runtime_flags` collection 行结构。
///
/// 把"演化器开关 + 灰度比例"从 `EVOLUTION_ENABLED` 单一 env 变量抬升为可在
/// 运行时按 workspace 调整的 mongo 文档，让运维不需要重启即可推进
/// 5% → 20% → 50% 的灰度节奏。`enabled=false` 等价于 env 关停态。
///
/// 灰度判定算法：`hash(contact_id) % 100 < rollout_percent`。同一 contact_id
/// 在 rollout_percent 不变时永远落入同一桶，避免回滚抖动；rollout_percent 单调
/// 上调时新增桶覆盖既有桶，已经在桶里的用户不会被踢出。
///
/// `rollout_percent` 取值范围 `[0, 100]`，超出范围时按 [`rollout_percent_clamped`]
/// 钳制；`updated_by` 为审计字段（admin user / system worker），可空。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionRuntimeFlag {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    /// 演化器整体开关。false 等价于 worker 直接 return；env `EVOLUTION_ENABLED=false`
    /// 仍可硬关停（启动期 short-circuit），优先级高。
    pub enabled: bool,
    /// 灰度比例 0..=100。`hash(contact_id) % 100 < rollout_percent` 命中即在桶内。
    #[serde(default)]
    pub rollout_percent: u32,
    /// 审计：上次写入者（admin id / system worker name）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
    /// Phase C / C5：closed-loop 自动 release 开关。`true` 时显著性 + 邻接性
    /// 双过滤通过的 threshold proposal 由 evolution worker 自动 release 而无需
    /// admin 点击；`false`（默认）保持 M4 W4 的 admin 二次确认行为。env
    /// `EVOLUTION_ENABLED=false` 时该字段无意义（worker 不跑）。
    #[serde(default)]
    pub threshold_auto_release_enabled: bool,
    pub updated_at: DateTime,
}

impl EvolutionRuntimeFlag {
    /// 钳制 `rollout_percent` 到 `[0, 100]`，避免脏数据让灰度计算溢出。
    pub fn rollout_percent_clamped(&self) -> u32 {
        self.rollout_percent.min(100)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatingMemory {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    #[serde(default)]
    pub user_understanding: Document,
    #[serde(default)]
    pub relationship_state: Document,
    #[serde(default)]
    pub product_fit: Document,
    #[serde(default)]
    pub next_action: Document,
    #[serde(default)]
    pub context_pack: Document,
    #[serde(default)]
    pub context_pack_version: i32,
    pub context_pack_updated_at: Option<DateTime>,
    #[serde(default)]
    pub memory_card: MemoryCardTyped,
    #[serde(default)]
    pub memory_card_version: i32,
    pub memory_card_updated_at: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCandidate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub run_id: Option<String>,
    pub source: String,
    #[serde(default)]
    pub candidates: Vec<Document>,
    #[serde(default)]
    pub memory_write_score: i32,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationKnowledgeDocument {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub domain: String,
    pub source_type: String,
    pub source_name: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub catalog_summary: Option<String>,
    #[serde(default)]
    pub routing_map: Vec<String>,
    #[serde(default)]
    pub risk_notes: Vec<String>,
    /// 文档级聚合标签：等于其下所有 chunks 的 `product_tags` 去重并集（≤5）。
    #[serde(default)]
    pub product_tags: Vec<String>,
    /// 文档级业务主题（≤3）：所有 chunks 的 `business_topics` 去重并集。
    #[serde(default)]
    pub business_topics: Vec<String>,
    pub raw_content: Option<String>,
    pub content_hash: Option<String>,
    #[serde(default)]
    pub line_index: Vec<Document>,
    #[serde(default)]
    pub section_index: Vec<Document>,
    pub status: String,
    pub version: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,

    // ── catalog 落库 + 增量重写（feature: catalog persistence） ──
    /// catalog_rebuild_worker 写出的 markdown 快照；`/catalog/persisted` O(1) 直读。
    /// 旧文档读出 None；首次 enqueue catalog_rebuild_job 后由 worker 填入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_summary_persisted: Option<String>,
    /// 单调递增版本号；前端 `If-None-Match` 走 304。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationKnowledgeChunk {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub document_id: Option<ObjectId>,
    pub item_id: Option<ObjectId>,
    pub domain: String,
    pub knowledge_type: Option<String>,
    pub business_context: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub body: Option<String>,
    #[serde(default)]
    pub applicable_scenes: Vec<String>,
    #[serde(default)]
    pub not_applicable_scenes: Vec<String>,
    /// 知识标签（≤5）：产品/品牌/解决方案名称。LLM 在 import-preview 时自动抽取，
    /// 后台可手动编辑。
    #[serde(default)]
    pub product_tags: Vec<String>,
    /// 业务主题（≤3）：本 chunk 属于哪个业务议题（产品定位/竞品对比/部署方式 ...）。
    #[serde(default)]
    pub business_topics: Vec<String>,
    pub source_quote: Option<String>,
    #[serde(default)]
    pub source_anchors: Vec<Document>,
    pub integrity_status: Option<String>,
    pub confidence_score: Option<i32>,
    pub status: String,
    pub priority: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,

    // ── knowledge-wiki 方法论字段（前向兼容；旧文档读出来全 None） ──
    /// 9 类 wiki_type 之一（source/entity/concept/comparison/synthesis/methodology/finding/query/thesis）。
    /// 旧文档读出 None；migration `2026_05_W1_001_chunks_wiki_type_default` 把所有缺字段 chunk 默认填 "entity"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiki_type: Option<String>,
    /// 业务字段 JSON 容器：销售域 `customer_stage / objection_type / pressure_level`，
    /// 教培域 `parent_emotion_state / age_segment / subject` 等。由 DomainSchema 校验。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_attributes: Option<Document>,
    /// 写入来源标注：ai/human/rule/imported + provider 别名 + source_quote。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<ChunkProvenance>,
    /// 时效起始；feedback worker 标 stale 时使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<DateTime>,
    /// 时效截止；valid_to < now 时 dynamic_confidence 减 stale_penalty。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<DateTime>,
    /// 已被新版本替代时记录新 chunk_id（≈ wiki redirect）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// 上一版本的 chunk_id；split/merge/rollback 都会维护这条链路。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_version_id: Option<String>,
    /// 关系图（≈ wikilinks）：6 种 relation_kind ∈ {superseded_by/references/requires/contradicts/clarifies/refines}。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_chunks: Option<Vec<RelatedRef>>,
    /// 30 天滑窗使用统计；feedback worker 周期回写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_stats: Option<UsageStats>,
    /// feedback worker 计算：base × 0.6 + hit_rate × 0.4 - stale_penalty，clamp [0,1]。
    /// catalog_persisted 用此字段排序。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_confidence: Option<f64>,
    /// 完整度评估（沿用既有评估口径，可被 feedback worker 维护）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity_score: Option<f64>,
    /// 编辑保护字段清单：patch 试图改这些字段一律 4xx 拒绝。默认 7 项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_fields: Option<Vec<String>>,

    /// Phase B / B3：4 类 **运营用途** 标签（`product_fact / style_template /
    /// negative_example / peer_case`）。与 `wiki_type`（9 类知识形态）正交：
    /// `wiki_type` 描述"它是什么知识"，`chunk_type` 描述"运营时怎么用它"。
    /// `knowledge_router` 按 `chunk_type` 分段拼接到 prompt：
    ///
    /// - `product_fact`：仅 `verified` 状态可用作产品声明背书；
    /// - `style_template`：作为 few-shot 模板供 reply-agent 参考语气；
    /// - `negative_example`：作为 don't-do 示例（来自 reviewer 误判反馈）；
    /// - `peer_case`：作为同行案例 reference（不作产品承诺）。
    ///
    /// R11 兼容：缺省值反序列化时由 [`default_chunk_type`] 填 `"product_fact"`，
    /// 旧文档不破坏。
    #[serde(default = "default_chunk_type")]
    pub chunk_type: String,
}

/// Phase B / B3：[`OperationKnowledgeChunk::chunk_type`] 缺省值。
/// 旧文档反序列化时缺该字段 → 视为 `product_fact`（最保守、走 verified-only 路径）。
pub(crate) fn default_chunk_type() -> String {
    "product_fact".to_string()
}

impl Default for OperationKnowledgeChunk {
    fn default() -> Self {
        Self {
            id: None,
            workspace_id: String::new(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: String::new(),
            knowledge_type: None,
            business_context: None,
            title: String::new(),
            summary: None,
            body: None,
            applicable_scenes: Vec::new(),
            not_applicable_scenes: Vec::new(),
            product_tags: Vec::new(),
            business_topics: Vec::new(),
            source_quote: None,
            source_anchors: Vec::new(),
            integrity_status: None,
            confidence_score: None,
            status: String::new(),
            priority: 0,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
            wiki_type: None,
            domain_attributes: None,
            provenance: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: None,
            usage_stats: None,
            dynamic_confidence: None,
            integrity_score: None,
            locked_fields: None,
            chunk_type: default_chunk_type(),
        }
    }
}

/// chunk 的写入来源标注。
///
/// `source` ∈ {ai, human, rule, imported}；`llm_model_alias` 用 provider_id 别名
/// （由用户在 LLM Provider Configs 自填），**不允许出现具体模型名/品牌名**。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkProvenance {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_doc_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_model_alias: Option<String>,
    pub edited_at: DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_by: Option<String>,
}

/// chunk 之间的关系引用（≈ wikilink）。`kind` 属于 6 种封闭枚举。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedRef {
    pub chunk_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// chunk 30 天滑窗使用统计。feedback worker 每 N 分钟回写一次。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageStats {
    #[serde(default)]
    pub hit_count_30d: u32,
    #[serde(default)]
    pub blocked_count_30d: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_blocked_reason: Option<String>,
}

/// chunk 编辑历史的不可变记录。每次 patch / split / merge / rollback / archive /
/// restore / verify / unverify 都写一行；revisions 表与 chunks 表双写。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRevision {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub chunk_id: String,
    pub revision_id: String,
    /// 操作语义 ∈ {create, patch, split, merge, rollback, archive, restore, verify, unverify}。
    pub op: String,
    /// 字段级 diff。AI 回复的 chat-canvas 永远只返 patch 而非整 chunk。
    #[serde(default)]
    pub patch: Document,
    pub before_hash: String,
    pub after_hash: String,
    /// 写入来源 ∈ {ai, human, rule, imported}。
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

/// 知识库 gap signal：structural lint（orphan/broken_link/no_outlinks）+
/// semantic lint（contradiction/stale/missing_chunk/suggestion/low_confidence）。
///
/// 两阶段 sweep 后 status 流转：pending → auto_resolved | llm_resolved | applied | dismissed。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGapSignal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub signal_id: String,
    pub workspace_id: String,
    /// 8 类信号 kind 之一。
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub affected_chunk_ids: Vec<String>,
    #[serde(default)]
    pub search_queries: Vec<String>,
    /// "warning" | "info"。
    pub severity: String,
    /// "rule" | "llm"。
    pub source: String,
    /// pending / auto_resolved / llm_resolved / applied / dismissed。
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
    pub created_at: DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime>,
}

/// P1-6：自动 ingest 数据源。worker tick 内逐条拉取 → 解析 → 调
/// `ingest_chunked_text` 落 chunks。`kind` 闭集：`"rss"` / `"html"`。
/// `status` 闭集：`"active"`（参与轮询）/ `"failing"`（连续失败 ≥ 3 次,
/// 仍参与但稀释）/ `"disabled"`（≥ 7 天不可达，跳过）。所有写入由 worker 与
/// `routes::knowledge::ingest_sources` CRUD 维护，handler 不接受 `failing` /
/// `disabled` 入参（语义闭集兜底）。AI 永不自动 verify：worker 落 chunk 走
/// `ingest_chunked_text`，全部 `status="draft"` + `integrity_status="needs_review"`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestSource {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub source_id: String,
    pub workspace_id: String,
    /// 闭集：`"rss"` / `"html"`。
    pub kind: String,
    pub url: String,
    /// 计划轮询周期（分钟）；worker 自身 tick 周期独立，本字段是单 source
    /// 的"距离上次拉取至少 N 分钟才再拉"。
    pub schedule_minutes: i64,
    /// 友好名称，前端展示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// 上次 200 / 304 时间。`None` 表示从未成功拉取。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetched_at: Option<DateTime>,
    /// HTTP `ETag` 缓存键；下一轮带 `If-None-Match` 上来。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_etag: Option<String>,
    /// 上次错误（若有），用于前端排障。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// 闭集：`"active"` / `"failing"` / `"disabled"`。
    pub status: String,
    /// 连续失败计数；成功（含 304）后清零。
    #[serde(default)]
    pub failure_streak: i32,
    /// 累计成功拉到 chunk 的次数（不计 304）。
    #[serde(default)]
    pub ingest_count: i64,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// 行业可配 schema：active 一条 / workspace；chunk 写入时校验 `domain_attributes`。
///
/// `alias_dict` 是 `Document`：`{ "客户阶段": "customer_stage", "话术类别": "objection_type" }`，
/// 写入时透明 rewrite 命中 key 为 canonical name。
/// `guard_dsl` 简版仅 `field OP value`，多条 AND 组合，复杂 DSL 留下一轮。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainSchema {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub schema_id: String,
    pub workspace_id: String,
    pub name: String,
    pub version: i32,
    #[serde(default)]
    pub fields: Vec<DomainField>,
    #[serde(default)]
    pub alias_dict: Document,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_dsl: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// `DomainSchema` 的字段定义。`kind` ∈ {string, enum, number, date, reference}。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainField {
    pub name: String,
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
}

/// universal-domain-adaptation Phase 0：行业/产品「总装配单」。
///
/// 让系统对行业**零假设**：一个 `DomainProfile` 声明本行业「参与决策的画像维度
/// + 关联 chunk 字段表 + prompt 片段 + 承诺词表 + completeness 维度」，由引导层 AI
/// 对话生成候选 → 人审 → publish。运行时按 `is_active=true` 加载（每 workspace 一条）；
/// 无 active 时 fallback 到内置 `DEFAULT_PROFILE`（等价当前销售域写死行为，保证零配置
/// 启动与历史一致）。
///
/// 维度的**取值字典**仍存 `system_taxonomies`（按 `kind` 关联，复用 `check_value`
/// 的 alias 归一/候选发现）；本结构只声明「本行业有哪些维度、哪些进决策校验」。
/// 版本灰度 4 字段与 [`TaxonomyEntry`] / `DomainSchema` 对齐（E5-T1）。
///
/// Phase 0 仅落存储 + 加载器，运行时**暂不消费**（并行加载、零行为变化）；
/// 消费解耦在 Phase 1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainProfile {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub profile_id: String,
    pub workspace_id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// 参与/不参与决策的画像维度声明（替代 `decision_taxonomy::TAGGED_FIELDS` const 表）。
    #[serde(default)]
    pub profile_dimensions: Vec<ProfileDimension>,
    /// 关联的 chunk 字段表（引用 `DomainSchema.schema_id`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain_schema_id: Option<String>,
    /// 行业 prompt 片段（注入决策 prompt，替代写死的销售域维度语义文案）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_fragment: Option<String>,
    /// 本行业绝对化承诺词表（替代 `guards.rs` 写死的中文销售词）。
    #[serde(default)]
    pub commitment_markers: CommitmentMarkers,
    /// completeness 审计维度（替代 `catalog.rs` 写死的五维 coverage）。
    #[serde(default)]
    pub coverage_dimensions: Vec<CoverageDimension>,
    /// universal-domain-adaptation H6：声明哪个画像维度驱动 planner 停滞计时
    /// （替代写死的 `customer_stage`）。`None` 时 planner fallback 到内置默认
    /// `customer_stage`（DEFAULT_PROFILE 下即如此，零行为变化）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagnation_dimension: Option<String>,
    /// universal-domain-adaptation H9：本行业允许的 conversationMode 取值集合
    /// （替代 `agent::types::CONVERSATION_MODE_VALUES` 写死的四模式枚举）。空 Vec
    /// 时 `validate_and_promote` fallback 到内置默认四模式（DEFAULT_PROFILE 即声明
    /// 这四个，零行为变化）。情感陪伴等行业可声明 `intimate_companion` 等额外模式。
    /// **红线**：`boundary_protection` 反接管语义无论本集合是否含它都继续由 prompt
    /// 写死守护，本字段只放宽"用哪些模式"，不放宽"反人工接管"。
    #[serde(default)]
    pub conversation_modes: Vec<String>,
    /// universal-domain-adaptation H8：本行业默认运营范式（三驱动力开关 + 阈值）。
    /// `OperationMode::default()` = 三全开 + 阈值 None 回落全局 config（DEFAULT_PROFILE
    /// 即如此，planner 金标零变化）。陪伴/维护型行业可声明 `funnel.enabled=false`。
    /// 单客户覆盖见 [`Contact::operation_mode_override`]。
    #[serde(default)]
    pub operation_mode: OperationMode,
    /// E5-T1 多版本灰度：同 `(workspace_id, profile_id)` 下 `version` 单调递增。
    #[serde(default = "default_version_one")]
    pub version: i32,
    #[serde(default)]
    pub current_version: bool,
    #[serde(default)]
    pub previous_version: Option<i32>,
    /// 写入来源：`generated_by_ai` / `manual` / `default`。
    #[serde(default)]
    pub seeded_by: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// universal-domain-adaptation H8：运营范式 = 声明启用哪些「主动触达驱动力」
/// + 各自阈值。三驱动力对应 planner 三扫描器（funnel→`scan_stage_stagnation`、
/// silence→`scan_silent`、commitment→`scan_commitments`）。
///
/// 全字段 `#[serde(default)]`，**缺省即「沿用全局 config」**——`OperationMode::default()`
/// = 三驱动力 `enabled=true` + 所有阈值 `None`（回落 `AppConfig`），故 DEFAULT_PROFILE
/// 与无 override 的 contact 下 planner 行为与改造前**逐字等价**（金标零变化）。
///
/// 两级声明：`DomainProfile.operation_mode`（行业默认范式）+
/// `Contact.operation_mode_override`（单客户覆盖，承接「因用户而异」，优先级更高）。
/// 解析走 [`resolve_operation_mode`](crate::planner::resolve_operation_mode)：
/// `contact override ?? profile ?? OperationMode::default()`。
///
/// 范式落法：销售型 = 三全开（DEFAULT）；陪伴/情绪型 = `funnel.enabled=false`
/// （不推进阶段、不被 stagnation 催）；关系维护型 = funnel 关、silence+commitment 开。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationMode {
    /// 沙漏推进驱动力（`scan_stage_stagnation`）。陪伴/维护型关掉它。
    #[serde(default)]
    pub funnel: FunnelMode,
    /// 沉默唤醒驱动力（`scan_silent`）。跨范式通用。
    #[serde(default)]
    pub silence: SilenceMode,
    /// 承诺到期驱动力（`scan_commitments`）。跨范式通用。
    #[serde(default)]
    pub commitment: CommitmentMode,
    /// universal-domain-adaptation H19：作息门控覆盖。情感陪伴「晚上是黄金时段」
    /// 可在此关掉静默时段抑制，让夜间主动/被动发送不被 22→8 压制。
    #[serde(default)]
    pub quiet_hours: QuietHoursMode,
}

impl Default for OperationMode {
    fn default() -> Self {
        Self {
            funnel: FunnelMode::default(),
            silence: SilenceMode::default(),
            commitment: CommitmentMode::default(),
            quiet_hours: QuietHoursMode::default(),
        }
    }
}

/// H8 漏斗推进驱动力。`enabled=false` → `scan_stage_stagnation` 对该 contact 短路。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunnelMode {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 停滞推进阈值（天）。`None` → 回落 `strategic_planner_stage_stagnation_threshold_days`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stagnation_threshold_days: Option<i64>,
}

impl Default for FunnelMode {
    fn default() -> Self {
        Self { enabled: true, stagnation_threshold_days: None }
    }
}

/// H8 沉默唤醒驱动力。`enabled=false` → `scan_silent` 对该 contact 短路。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SilenceMode {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 静默唤醒阈值（小时）。`None` → 回落 `strategic_planner_silent_threshold_hours`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_hours: Option<i64>,
}

impl Default for SilenceMode {
    fn default() -> Self {
        Self { enabled: true, threshold_hours: None }
    }
}

/// H8 承诺到期驱动力。`enabled=false` → `scan_commitments` 对该 contact 短路。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommitmentMode {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 承诺临近窗口（小时）。`None` → 回落
    /// `strategic_planner_commitment_imminent_window_hours`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imminent_window_hours: Option<i64>,
}

impl Default for CommitmentMode {
    fn default() -> Self {
        Self { enabled: true, imminent_window_hours: None }
    }
}

/// H19 作息门控覆盖。`enabled_override`：
/// - `None`（默认）→ 沿用全局 `runtime.quiet_hours_enabled`（DEFAULT 逐字等价）；
/// - `Some(false)` → 本 contact/范式**关闭**静默时段抑制（情感陪伴夜间黄金时段不被压制）；
/// - `Some(true)` → 强制开启（即便全局关）。
///
/// 仅覆盖「是否启用静默」；起止小时 / 时区偏移继续走全局 runtime（避免在 contact
/// 上重复一套作息参数；本阶段需求只是「陪伴型整段关掉作息门」）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuietHoursMode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_override: Option<bool>,
}

impl Default for QuietHoursMode {
    fn default() -> Self {
        Self { enabled_override: None }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDimension {
    pub kind: String,
    pub display_name: String,
    /// 是否进 Reply Agent 决策的 taxonomy 校验（对应旧 `TAGGED_FIELDS` 成员）。
    #[serde(default)]
    pub participates_in_decision: bool,
    /// 注入 prompt 的语义说明（如「就诊阶段：初诊/复诊/方案确认/已治疗」）。
    #[serde(default)]
    pub description: String,
}

/// 绝对化承诺词表，按 `commitment_claim_class` 分两类（替代 `guards.rs` 写死词表）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommitmentMarkers {
    /// 产品效果类（如销售域「成功率/见效/回款」，医疗域「根治率」，教培域「保过」）。
    #[serde(default)]
    pub product_effect: Vec<String>,
    /// 纯语气类（如「保证/一定能/绝对」）。
    #[serde(default)]
    pub tone_only: Vec<String>,
}

/// completeness 审计的一个 coverage 维度（替代 `catalog.rs` 写死的五维）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageDimension {
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub required: bool,
}

/// catalog 重建队列：`apply_chunk_revision` 写完即 enqueue；catalog_rebuild_worker
/// 每 200ms 取一批 status=queued 落库 `documents.catalog_summary_persisted`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRebuildJob {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub job_id: String,
    pub workspace_id: String,
    pub document_id: ObjectId,
    pub queued_at: DateTime,
    /// queued / running / done / failed。
    pub status: String,
    #[serde(default)]
    pub attempts: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeUsageLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub run_id: String,
    #[serde(default)]
    pub knowledge_ids: Vec<ObjectId>,
    #[serde(default)]
    pub route_result: Document,
    pub reply_text: Option<String>,
    pub review_approved: bool,
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub tool_trace: Vec<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeChatTurn {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub session_id: String,
    /// 会话内单调递增的 turn 序号，从 1 开始；user / assistant 各占一个序号。
    pub turn_index: i32,
    /// "user" | "assistant"
    pub role: String,
    /// assistant 的 intent 分类；user 一般为空。
    pub intent: Option<String>,
    /// 自然语言内容；user 的输入或 assistant 的 naturalReply。
    pub content: String,
    /// 引用的切片 / 知识包，attachments 仅 ≤ 1 条。
    #[serde(default)]
    pub attachments: Vec<Document>,
    /// assistant 起草的 chunk / pack patch，未应用前是预览。
    pub patch: Option<Document>,
    /// assistant 提示运营仍缺哪些字段。
    #[serde(default)]
    pub missing_fields: Vec<String>,
    /// assistant 提出的追问列表（≤ 3 条）。
    #[serde(default)]
    pub followup_questions: Vec<Document>,
    /// "pending" | "applied" | "discarded"
    pub status: String,
    /// 本轮 LLM 大致 token 消耗（用 BudgetSnapshot.tokens 累计）。
    pub tokens_used: i64,
    pub prompt_key: Option<String>,
    /// knowledge-digest-workstation 扩展：
    /// `task_progress` / `task_summary` / `tool_call_log` / `null`（向后兼容）。
    /// 由 `KnowledgeTaskWorker` / tool_loop 写入，旧 turn 缺该字段视为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// knowledge-digest-workstation 扩展：tool 调用日志，仅 `kind="tool_call_log"`
    /// 或 assistant 发起 tool-calling 时写入。
    /// 形如 `[{name, params, result, latency_ms, tokens}]`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<Document>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDecisionReview {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub run_id: Option<String>,
    pub inbound_message_id: Option<String>,
    pub reply_text: Option<String>,
    pub approved: bool,
    #[serde(default)]
    pub scores: Document,
    #[serde(default)]
    pub formula_breakdown: Document,
    #[serde(default)]
    pub risks: Vec<String>,
    pub rewrite_instruction: Option<String>,
    pub review_summary: Option<String>,
    pub playbook_id: Option<ObjectId>,
    pub playbook_version: Option<i32>,
    #[serde(default)]
    pub used_knowledge_ids: Vec<ObjectId>,
    #[serde(default)]
    pub prompt_versions: Document,
    pub operation_state: Option<String>,
    #[serde(default)]
    pub next_best_action: Document,
    #[serde(default)]
    pub context_pack_snapshot: Document,
    #[serde(default)]
    pub domain_config_snapshot: Document,
    #[serde(default)]
    pub runtime_parameters_snapshot: Document,
    #[serde(default)]
    pub send_gateway_result: Document,
    pub outcome_status: Option<String>,
    #[serde(default)]
    pub reaction_analysis: Document,
    #[serde(default)]
    pub reaction_claimed_at: Option<DateTime>,
    /// Phase C / C1: reviewer 误判信号（reviewer 判断与用户实际反应不一致时记录）。
    /// 取值：`approved_but_user_negative` / `blocked_but_user_positive` / None。
    /// 由 `record_user_reaction_inner` 在 reaction_analysis 写入后计算并 $set 同步落库，
    /// 供 feedback_worker 周期汇总 reviewer_stats，C2 分支据此挑选 negative_example 候选。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer_misjudge_signal: Option<String>,
    pub status: String,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: Option<String>,
    pub run_id: String,
    pub trigger_kind: String,
    pub status: String,
    #[serde(default)]
    pub planner: Document,
    #[serde(default)]
    pub context: Document,
    #[serde(default)]
    pub knowledge_route: Document,
    #[serde(default)]
    pub decision: Document,
    #[serde(default)]
    pub review: Document,
    #[serde(default)]
    pub gateway_result: Document,
    pub error: Option<String>,
    /// MP-5 / Task 15：单次 run 的 token 预算。
    #[serde(default)]
    pub token_budget: i64,
    /// MP-5 / Task 15：本次 run 累计消耗 token。
    #[serde(default)]
    pub tokens_used: i64,
    /// MP-5 / Task 15：本次 run 实际 LLM 调用次数。
    #[serde(default)]
    pub llm_calls_used: i32,
    /// MP-5 / Task 15：本次 run 触发的降级原因列表（"review_skipped_budget_exceeded" 等）。
    #[serde(default)]
    pub degraded_reasons: Vec<String>,
    // ── agent-autonomy-loop W1 (Task 2.4) ──
    //
    // R0 Run Envelope + R9 自治审计字段。所有新增字段使用 `#[serde(default)]`
    // 以保持向后兼容：升级前已经写入的 `agent_run_logs` 文档不含这些字段，
    // 反序列化时取默认值（空字符串 / None / 空 Vec / false），不会破坏现有
    // 测试 / 历史回放。
    //
    // BSON 字段名沿用结构体的 snake_case（与 W0 task 1.2 在 `src/db/indexes.rs`
    // 创建、W6 task 7.1 修订后的 `(account_id, lifecycle, created_at)` /
    // `(account_id, final_review_status, created_at)` /
    // `(account_id, autonomy_mode, created_at)` 复合索引一致）。
    /// R0 Run Envelope：lifecycle ∈ `started / running / completed /
    /// failed_before_decision / failed_after_decision / aborted_by_budget /
    /// aborted_by_external_signal`。新建 envelope 时由
    /// [`crate::agent::run_envelope::write_run_envelope_started`] 显式置为
    /// `"started"`。
    #[serde(default)]
    pub lifecycle: String,
    /// R0 Run Envelope：触发本次 run 的入站消息或跟进任务 ID（`inbound_message.message_id`
    /// 或 `task_id`），是 R13 outbox `idempotency_key` 的核心成分。
    #[serde(default)]
    pub source_event_id: String,
    /// R0 Run Envelope：触发来源类别。允许枚举 `inbound_message / follow_up_task /
    /// manual_send`（具体校验在 W2 finalize 阶段）。
    #[serde(default)]
    pub source_kind: String,
    /// R0 Run Envelope：错误简要（≤ 1024 chars），由 envelope panic / failure
    /// 路径在 `lifecycle ∈ {failed_before_decision, failed_after_decision}` 时填写。
    #[serde(default)]
    pub error_summary: Option<String>,
    /// R0 Run Envelope：取消原因（≤ 256 chars），lifecycle =
    /// `aborted_by_external_signal` 时填写（如 `user_reaction_stop_requested`）。
    #[serde(default)]
    pub abort_reason: Option<String>,
    /// R9：是否触发了 single-shot revision。
    #[serde(default)]
    pub revision_applied: bool,
    /// R9：触发 revision 的原因摘要（≤ 1024 chars）。
    #[serde(default)]
    pub revision_reason: String,
    /// R9：revision 之前的 decision 文案摘要（≤ 2048 chars）。
    #[serde(default)]
    pub pre_revision_summary: Option<String>,
    /// R9：revision 之后的 decision 文案摘要（≤ 2048 chars）。
    #[serde(default)]
    pub post_revision_summary: Option<String>,
    /// R9：Reply Agent 自我批判（≤ 2048 chars）。
    #[serde(default)]
    pub self_critique: Option<String>,
    /// R3 / R9：自治控制位 ∈ `auto / assisted / blocked`。
    #[serde(default)]
    pub autonomy_mode: String,
    /// 对话模式（R-prompt-v3）：四模式人格切换 ∈
    /// `casual_relationship / value_exchange / consultative / boundary_protection`。
    /// 详见 docs/conversation-mode-design.md。
    #[serde(default)]
    pub conversation_mode: String,
    /// `conversation_mode` 选定原因（如 `customer_stage:proposal_evaluation` 等）。
    /// Optional：旧 run log 不带本字段。
    #[serde(default)]
    pub conversation_mode_reason: Option<String>,
    /// R9：最终归档状态（前端 horizon 聚合用）。允许枚举详见
    /// requirements.md "状态枚举映射表" finalReviewStatus 列。
    #[serde(default)]
    pub final_review_status: String,
    /// R13：与 outbox 的反向关联状态（dispatcher worker 在状态推进时
    /// `update_one(agent_run_logs, $set)` 反写）。允许 `pending / in_flight /
    /// sent / failed_terminal / canceled`。
    #[serde(default)]
    pub outbox_status: Option<String>,
    /// R7：memory consolidator 在 `apply_consolidator_deprecations` 中产出的
    /// warnings（如 `deprecated_fact_id_not_found:<id>` /
    /// `superseded_by_id_not_found:<id>:<sup>` 等）。
    #[serde(default)]
    pub memory_consolidator_warnings: Vec<String>,
    pub created_at: DateTime,
}

// ── agent-autonomy-loop W0 (Task 1.1) ──
//
// 三个新增 collection 的占位 struct（`agent_send_outbox / system_taxonomies /
// taxonomy_candidates`）。字段定义参照 design.md §3.2 / §3.3 / §3.4，最终字段
// 与 serde 命名约定将在 W3 / W4 波次的具体业务 task 中落定（含 idempotency_key
// 计算、字典 alias 命中、worker lease 字段语义等）。
//
// 设计注记（保持与既有代码一致的写法）：
// - `account_id` 在本仓既有数据模型中均为 `String`（不是 `ObjectId`），故占位
//   struct 沿用 `String`；如 W4 outbox dispatcher 需要 `ObjectId` 可届时调整。
// - 主键统一沿用 `pub id: Option<ObjectId>` + `#[serde(rename = "_id", skip_serializing_if = "Option::is_none")]`。
// - 不强制 camelCase rename；与现有大多数 collection（如 `AgentRunLog`）保持一致。

/// agent-autonomy-loop W0：`agent_send_outbox` 集合占位结构。
///
/// 用于 Reply Agent → review → MCP 发送之间的可靠链路（持久化 / 幂等 / 重试 /
/// 取消）。具体语义详见 design.md §3.2 与 requirements.md R13；本期 W0 仅用作
/// `Database::collection_agent_send_outbox` 的类型绑定，最终字段约束（如
/// `idempotency_key` 唯一索引、`status` 枚举值 `pending|in_flight|sent|failed_terminal|canceled`）
/// 在 W4 task 5.x 中落地。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    pub run_id: String,
    pub decision_id: Option<ObjectId>,
    pub source_event_id: String,
    pub source_kind: String,
    pub content: String,
    pub content_hash: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub attempt: i32,
    #[serde(default)]
    pub max_attempts: i32,
    pub status: String,
    pub cancel_reason: Option<String>,
    pub last_error: Option<String>,
    pub next_retry_at: Option<DateTime>,
    pub worker_id: Option<String>,
    pub locked_until: Option<DateTime>,
    /// 崩溃恢复标记：`reclaim_expired_leases` 把一条 `in_flight`（lease 过期）
    /// 改回 `pending` 时置 true。说明上一个 worker 抢占后在写 `sent` 前消失
    /// （OOM / 部署 / panic），它**可能已把消息送达 MCP/微信**。dispatcher 重发
    /// 前据此对这条（且仅这条）跑 `mcp_already_succeeded` post-hoc 核对，命中即
    /// 标 sent 不重发——避免给客户发重复消息。`#[serde(default)]` 兼容旧文档。
    #[serde(default)]
    pub reclaimed_in_flight: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub sent_at: Option<DateTime>,
}

/// agent-autonomy-loop W0：`system_taxonomies` 集合占位结构。
///
/// 双层标签的"严格字典"层：`customer_stage / intent_level / objection_type` 等
/// 维度由 `(scope, kind, value.id)` 唯一标识。详见 design.md §3.3 与
/// requirements.md R8；W3 task 4.6 / 4.7 / 4.9 落实加载 / alias 命中 / seed 迁移。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyEntry {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    /// `"global"` 或 `account_id` 字符串。
    pub scope: String,
    /// 维度名（如 `"customer_stage"` / `"intent_level"` / `"objection_type"`）。
    pub kind: String,
    pub value: TaxonomyValue,
    pub updated_at: DateTime,
    /// Phase E / E5-T1：多版本灰度。同 `(scope, kind, value.id)` 下 `version` 单调递增；
    /// `m015` 迁移 backfill 为 1。多个 active 版本可共存（rollout 阶段），
    /// 但 `current_version=true` 至多一条。
    #[serde(default = "default_version_one")]
    pub version: i32,
    /// Phase E / E5-T1：当前生效版本标记。
    #[serde(default)]
    pub current_version: bool,
    /// Phase E / E5-T1：被替换的上一版本号。
    #[serde(default)]
    pub previous_version: Option<i32>,
    /// Phase E / E5-T1：写入来源标签。
    #[serde(default)]
    pub seeded_by: Option<String>,
}

/// agent-autonomy-loop W0：`TaxonomyEntry.value` 内嵌结构。字段名采用 camelCase
/// 以便与 design.md 中"value.id"等索引路径一致（详见 R8.1 唯一索引）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxonomyValue {
    /// 字典 key（如 `"first_contact"`），不是 BSON `_id`。
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// `"active"` | `"deprecated"`。
    pub status: String,
    /// universal-domain-adaptation H6：该取值的跟进优先级权重。planner 排序读它替代
    /// 写死的 `stage_priority_weight` / `intent_level_weight` match 分支。`None` 时
    /// planner fallback 到内置默认（保持旧库零行为变化）。
    #[serde(default)]
    pub priority_weight: Option<i32>,
    /// universal-domain-adaptation H6：是否终态（成交后维护 / 冷却 / 沉默等）。planner
    /// stagnation 段读它替代写死的 `TERMINAL_STAGES` 常量。旧库默认 `false`。
    #[serde(default)]
    pub is_terminal: bool,
}

/// agent-autonomy-loop W0：`taxonomy_candidates` 集合占位结构。
///
/// 双层标签的"候选"层：Reply Agent 输出但不在 `system_taxonomies` 中的取值落入
/// 此集合，由后台审核后并入正式字典；候选状态 SHALL NOT 阻塞 Reply Agent 运行。
/// 详见 design.md §3.4 与 requirements.md R8.4；W3 task 4.6 / 4.7 / 4.8 落实
/// upsert / approve / reject 业务逻辑。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyCandidate {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub scope: String,
    pub kind: String,
    pub raw_value: String,
    pub evidence: Option<String>,
    #[serde(default)]
    pub confidence: i32,
    pub first_seen_at: DateTime,
    pub last_seen_at: DateTime,
    #[serde(default)]
    pub occurrences: i32,
    /// `"pending"` | `"approved"` | `"rejected"`。
    pub status: String,
    pub reviewed_at: Option<DateTime>,
    pub reviewed_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallLog {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub contact_wxid: Option<String>,
    pub run_id: Option<String>,
    pub prompt_key: String,
    pub model: String,
    pub status: String,
    pub latency_ms: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_cache_hit_tokens: i64,
    pub prompt_cache_miss_tokens: i64,
    pub error: Option<String>,
    /// HP-4 / Task 11：本次调用前发生的重试次数（0 表示一次成功，max_retries-1 表示走完所有重试才成功/失败）。
    #[serde(default)]
    pub retry_count: i32,
    /// HP-4 / Task 11：调用最终状态。`success | failed | json_error | cache_hit`。
    #[serde(default)]
    pub final_status: Option<String>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOperationGuidePreview {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_id: ObjectId,
    pub contact_wxid: String,
    pub instruction: String,
    pub mode: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub impact_scope: String,
    #[serde(default)]
    pub scope_reason: String,
    #[serde(default)]
    pub readable_changes: Vec<String>,
    #[serde(default)]
    pub health_scores: Document,
    #[serde(default)]
    pub suggested_changes: Document,
    #[serde(default)]
    pub risk_warnings: Vec<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagementAgentSession {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub title: String,
    /// S-20 / Task 19：dry-run 模式默认值。`true` 时所有非 read 工具调用
    /// 只回放计划不实际执行；可由单条消息请求覆盖。
    #[serde(default)]
    pub dry_run: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagementAgentMessage {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub session_id: ObjectId,
    pub role: String,
    pub content: String,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCommandRun {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub session_id: ObjectId,
    pub operator_message: String,
    pub status: String,
    pub plan: Option<Document>,
    pub summary: String,
    pub error: Option<String>,
    #[serde(default)]
    pub prompt_versions: Document,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCall {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub command_run_id: ObjectId,
    pub tool_name: String,
    pub arguments: Document,
    pub status: String,
    pub response: Option<Document>,
    pub error: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// S-19 / Task 17：长 horizon 用户运营 outcome 指标。
///
/// `_id` 形如 `"{account_id}:{horizon}:{date}"`，便于幂等聚合（重跑覆盖同 _id）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutcomeMetric {
    #[serde(rename = "_id")]
    pub id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub horizon: String,
    pub date: String,
    /// 波 A2：outbound 总数为 0 时为 `None`（"无数据"），不再静默写 0。
    #[serde(default)]
    pub reply_rate: Option<f64>,
    /// 波 A2：managed contact 数为 0 时为 `None`。
    #[serde(default)]
    pub conversation_depth: Option<f64>,
    /// 波 A2：AI 自暂缓后由 AI 自身澄清恢复继续的比例。当前 events 中尚无
    /// 对应事件源，统一写 `None` 表示"指标暂不可用"，避免前端误判为"零成
    /// 功率"。后续接入事件源后改为实际比例。
    /// 注：旧字段 `human_handoff_success_rate` 已退役（违反全自治产品定位），
    /// 使用 `serde(alias)` 兼容历史 BSON 文档读取，写入用新字段名。
    #[serde(default, alias = "human_handoff_success_rate")]
    pub ai_hold_cleared_rate: Option<f64>,
    /// 波 A2：review 总数为 0 时为 `None`。
    #[serde(default)]
    pub agent_block_rate: Option<f64>,
    #[serde(default)]
    pub daily_run_count: i64,
    #[serde(default)]
    pub daily_run_token_total: i64,
    pub created_at: DateTime,
}

/// S-18 / Task 18：公式遵守度评测场景。
///
/// 用于对比模型 `formula_breakdown` / `review.scores` 与人工标注 ground_truth
/// 的偏差，跟踪四大公式（Trust/ConversionReadiness/EmotionalValue/NextBestActionScore）
/// 的遵守程度。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationScenario {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub scenario_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub account_id: Option<String>,
    #[serde(default)]
    pub contact_seed: Document,
    #[serde(default)]
    pub inbound_messages: Vec<String>,
    #[serde(default)]
    pub ground_truth: Document,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_evaluation_scenario_status")]
    pub status: String,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

fn default_evaluation_scenario_status() -> String {
    "active".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnableAgentRequest {
    pub human_profile_note: String,
    pub playbook_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileNoteRequest {
    pub human_profile_note: String,
}

/// 运营人员特别指令（最高优先级 Operator Instruction 层）写入请求体。
/// `instructions` 上限 1000 字符，由 `PUT /api/contacts/:id/custom-agent-instructions`
/// 路由维护。空字符串表示清空（落库为 null）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAgentInstructionsRequest {
    #[serde(default)]
    pub instructions: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchImportRequest {
    pub query: String,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
}

/// 波 A3：把 `search` 返回的候选导入本地 contacts 集合。两种入参互斥：
/// - `query`：等价于先 search 再 import（兼容老调用方）。
/// - `candidates`：直接消费前端拿到的候选数组。
#[derive(Debug, Deserialize)]
pub struct ImportContactsRequest {
    /// 沿用旧"搜索-导入"语义，方便客户端一步完成。
    #[serde(default)]
    pub query: Option<String>,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
    /// 来自前端 search 的候选项原样数组；每项若含 `.contact` 子对象会被解开。
    #[serde(default)]
    pub candidates: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ContactQuery {
    pub status: Option<String>,
    pub q: Option<String>,
    #[serde(rename = "accountId")]
    pub account_id: Option<String>,
    pub limit: Option<i64>,
    pub skip: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiCommitment {
    pub id: String,
    pub text: String,
    pub due_at: Option<String>,
    pub created_at: Option<String>,
}

impl From<&CommitmentRepr> for ApiCommitment {
    fn from(repr: &CommitmentRepr) -> Self {
        match repr {
            CommitmentRepr::Plain(text) => Self {
                id: String::new(),
                text: text.clone(),
                due_at: None,
                created_at: None,
            },
            CommitmentRepr::Structured(entry) => Self {
                id: entry.id.clone(),
                text: entry.text.clone(),
                due_at: dt_to_string(entry.due_at.unwrap_or(DateTime::from_millis(0)))
                    .filter(|_| entry.due_at.is_some()),
                created_at: dt_to_string(entry.created_at),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiContact {
    pub id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    pub agent_status: AgentStatus,
    pub human_profile_note: Option<String>,
    pub custom_agent_instructions: Option<String>,
    pub agent_profile: Option<AgentProfile>,
    pub memory_summary: Option<String>,
    pub playbook_id: Option<String>,
    pub playbook_version: Option<i32>,
    pub tags: Vec<String>,
    pub domain_attributes: Option<Document>,
    pub domain_attributes_updated_at: Option<String>,
    pub commitments: Vec<ApiCommitment>,
    pub follow_up_policy: Option<String>,
    pub operation_state: Option<String>,
    pub operation_state_reason: Option<String>,
    pub operation_state_confidence: Option<i32>,
    pub operation_state_updated_at: Option<String>,
    pub cooldown_until: Option<String>,
    pub operation_policy: Document,
    pub profile_attributes: Document,
    pub profile_updated_at: Option<String>,
    pub last_message_at: Option<String>,
    pub last_inbound_at: Option<String>,
    pub last_outbound_at: Option<String>,
    pub last_agent_run_at: Option<String>,
    pub updated_at: String,
}

impl From<Contact> for ApiContact {
    fn from(contact: Contact) -> Self {
        Self {
            id: contact.id.map(|id| id.to_hex()).unwrap_or_default(),
            workspace_id: contact.workspace_id,
            account_id: contact.account_id,
            wxid: contact.wxid,
            nickname: contact.nickname,
            remark: contact.remark,
            alias: contact.alias,
            agent_status: contact.agent_status,
            human_profile_note: contact.human_profile_note,
            custom_agent_instructions: contact.custom_agent_instructions,
            agent_profile: contact.agent_profile,
            memory_summary: contact.memory_summary,
            playbook_id: contact.playbook_id.map(|id| id.to_hex()),
            playbook_version: contact.playbook_version,
            tags: contact.tags,
            domain_attributes: contact.domain_attributes,
            domain_attributes_updated_at: contact
                .domain_attributes_updated_at
                .and_then(dt_to_string),
            commitments: contact.commitments.iter().map(ApiCommitment::from).collect(),
            follow_up_policy: contact.follow_up_policy,
            operation_state: contact.operation_state,
            operation_state_reason: contact.operation_state_reason,
            operation_state_confidence: contact.operation_state_confidence,
            operation_state_updated_at: contact.operation_state_updated_at.and_then(dt_to_string),
            cooldown_until: contact.cooldown_until.and_then(dt_to_string),
            operation_policy: contact.operation_policy,
            profile_attributes: contact.profile_attributes,
            profile_updated_at: contact.profile_updated_at.and_then(dt_to_string),
            last_message_at: contact.last_message_at.and_then(dt_to_string),
            last_inbound_at: contact.last_inbound_at.and_then(dt_to_string),
            last_outbound_at: contact.last_outbound_at.and_then(dt_to_string),
            last_agent_run_at: contact.last_agent_run_at.and_then(dt_to_string),
            updated_at: dt_to_string(contact.updated_at).unwrap_or_default(),
        }
    }
}

pub fn dt_to_string(dt: DateTime) -> Option<String> {
    dt.try_to_rfc3339_string().ok()
}

/// 请示台账状态闭集。pending=已推送领导待回；resolved=真人已裁决并已起 relay。
pub const PRINCIPAL_ESCALATION_STATUS_PENDING: &str = "pending";
pub const PRINCIPAL_ESCALATION_STATUS_RESOLVED: &str = "resolved";
pub const ALLOWED_PRINCIPAL_ESCALATION_STATUS: &[&str] = &[
    PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED,
];

/// 请示触发的三类边界（实质驱动）。
pub const ESCALATION_CATEGORY_OUT_OF_SCOPE: &str = "out_of_scope_decision";
pub const ESCALATION_CATEGORY_HIGH_RISK_GATED: &str = "high_risk_gated";
pub const ESCALATION_CATEGORY_STUCK: &str = "stuck_or_undelivered";
pub const ALLOWED_ESCALATION_CATEGORY: &[&str] = &[
    ESCALATION_CATEGORY_OUT_OF_SCOPE,
    ESCALATION_CATEGORY_HIGH_RISK_GATED,
    ESCALATION_CATEGORY_STUCK,
];

/// contact.domain_attributes 上的布尔标记 key：该客户有一个 pending 请示、正在等待领导决策。
/// admin 看板据此显示「等待中」；等待期 pre-check 据此识别。统一占位模型下这只是可观测标记，
/// 不是 hold category——触发请示的 run 本身是 Approved，占位已正常发出。
pub const AWAITING_PRINCIPAL_DECISION_ATTR: &str = "awaiting_principal_decision";

/// 真人裁决口径闭集。
pub const PRINCIPAL_VERDICT_APPROVED: &str = "approved";
pub const PRINCIPAL_VERDICT_REJECTED: &str = "rejected";
pub const PRINCIPAL_VERDICT_CONDITIONAL: &str = "conditional";
pub const PRINCIPAL_VERDICT_DEFERRED: &str = "deferred";
pub const PRINCIPAL_VERDICT_DELEGATED_BACK: &str = "delegated_back";
pub const ALLOWED_PRINCIPAL_VERDICT: &[&str] = &[
    PRINCIPAL_VERDICT_APPROVED,
    PRINCIPAL_VERDICT_REJECTED,
    PRINCIPAL_VERDICT_CONDITIONAL,
    PRINCIPAL_VERDICT_DEFERRED,
    PRINCIPAL_VERDICT_DELEGATED_BACK,
];

/// 决策 Agent 在 decision 阶段 emit 的请示意图（内嵌进 AgentDecision）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EscalationRequest {
    /// 是否需要请示真人。LLM 漏给该字段时安全回落 false（不请示），与 AgentDecision 的 LLM-容错惯例一致。
    #[serde(default)]
    pub needed: bool,
    /// 三类之一，见 ALLOWED_ESCALATION_CATEGORY。
    #[serde(default)]
    pub category: Option<String>,
    /// 卡点原因（给真人看）。
    #[serde(default)]
    pub reason: Option<String>,
    /// 向真人提的问题。
    #[serde(default)]
    pub question_for_principal: Option<String>,
    /// 客户同一条消息里"非越权、可自主答"的部分（等待期分答用）。
    #[serde(default)]
    pub self_serviceable_part: Option<String>,
    /// agent 自判该决策是否可泛化（决定是否发知识缺口提案）。
    #[serde(default)]
    pub is_generalizable: bool,
}

/// 真人自然语言裁决经 LLM 解读后的结构。绝不原话转发给客户。
/// 注意：不加 rename_all="camelCase"——本结构会被持久化进 snake_case 的 AgentPrincipalEscalation
/// 台账（decision 字段），保持 snake_case 让台账文档键统一、避免 decision.authorization_window_hours
/// 查询/索引时静默 miss。Task 14 的 interpret prompt 须输出 snake_case 键。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrincipalDecision {
    /// 裁决口径，见 ALLOWED_PRINCIPAL_VERDICT。
    pub verdict: String,
    /// 决策实质（如"同意 8 折"），AI 口吻转述的事实源。
    pub substance: String,
    /// 附带约束（如"本周内付款"）。
    #[serde(default)]
    pub constraints: Vec<String>,
    /// 授权有效时长（小时）。**领导说了算**：领导明确说了期限（"这个价就今天有效"="约 24"、
    /// "这周内都行"=本周剩余小时数）才填；领导没提期限 → None（= 授权不设过期窗，长期有效）。
    /// 由 interpret LLM 自判填充；Task 19 据此算 authorization_expires_at。
    #[serde(default)]
    pub authorization_window_hours: Option<f64>,
}

/// 请示台账行（MongoDB collection `agent_principal_escalations`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPrincipalEscalation {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub contact_wxid: String,
    /// 人类可读短码，如 "E1A2"。全局唯一。
    pub short_code: String,
    /// pending / resolved，见 ALLOWED_PRINCIPAL_ESCALATION_STATUS。
    pub status: String,
    /// 三类触发之一，见 ALLOWED_ESCALATION_CATEGORY。
    pub category: String,
    /// 卡点原因。
    pub reason: String,
    /// 向真人提的问题。
    pub question_for_principal: String,
    /// 推给领导的 wxid（= 该 workspace 的 principal_decider）。
    pub principal_wxid: String,
    /// resolved 时填：真人裁决解读结果。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<PrincipalDecision>,
    /// resolved 时填：授权过期时间（过期后该条授权不可再用，但条目仍 resolved）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expires_at: Option<DateTime>,
    /// agent 在 emit escalation 时自判：该决策是否可泛化成通用知识（决定 relay 后是否发知识缺口提案）。
    #[serde(default)]
    pub is_generalizable: bool,
    /// 是否已据此发过知识缺口提案（防重复）。
    #[serde(default)]
    pub knowledge_proposal_emitted: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime>,
}

// LP-12 / Task 21：核心 Document 字段的强类型版本。
//
// **设计决策**：本次迭代仅引入 *新增* 强类型 struct 与转换辅助，**不**强制替换
// 既有 Document 调用点（避免触发大面积改动）。后续小迭代里业务代码逐步
// 迁移到强类型上，最终再彻底删除 Document 字段。
//
// 所有 struct 都用 `#[serde(rename_all = "camelCase")]` + `#[serde(default)]`，
// 既保留 wire 格式又能向后兼容老数据缺字段。

mod typed {
    use super::*;
    use mongodb::bson;

    /// LP-12 / Task 21：`OperationDomainConfig.runtime_parameters` 的强类型版本。
    ///
    /// 字段与 [`crate::agent::UserRuntimeParameters`] 通过 doc_i32/doc_i64 读取的
    /// key 完全对齐；运行时仍走 Document，但后台编辑/校验/迁移可以借助这个 struct
    /// 获得 compiler 帮助。
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RuntimeParametersTyped {
        #[serde(default = "defaults::recent_message_limit")]
        pub recent_message_limit: i64,
        #[serde(default = "defaults::min_reply_interval_seconds")]
        pub min_reply_interval_seconds: i64,
        #[serde(default = "defaults::max_daily_touches")]
        pub max_daily_touches: i64,
        #[serde(default = "defaults::max_pending_follow_ups")]
        pub max_pending_follow_ups: i64,
        #[serde(default = "defaults::follow_up_expires_hours")]
        pub follow_up_expires_hours: i64,
        #[serde(default = "defaults::cooldown_after_no_reply_hours")]
        pub cooldown_after_no_reply_hours: i64,
        #[serde(default = "defaults::hallucination_block_at")]
        pub hallucination_block_at: i32,
        /// universal-domain-adaptation C1：压力风险 block 阈值（`PressureRisk ≥ 此值`
        /// 则 block）。默认 7。此前是五闸里**唯一**写死在 `UserRuntimeParameters`
        /// （= 7）而不走 typed 配置的阈值；typed 化后 H9 情感/陪伴场景可经运营域配置
        /// 放宽（如调到 9，允许更主动的情感推进而不被压力门拦）。DEFAULT = 7 逐字等价。
        #[serde(default = "defaults::pressure_risk_block_at")]
        pub pressure_risk_block_at: i32,
        #[serde(default = "defaults::knowledge_grounding_block_below")]
        pub knowledge_grounding_block_below: i32,
        #[serde(default = "defaults::human_like_rewrite_below")]
        pub human_like_rewrite_below: i32,
        #[serde(default = "defaults::emotional_value_rewrite_below")]
        pub emotional_value_rewrite_below: i32,
        #[serde(default = "defaults::operation_state_confidence_full_review_below")]
        pub operation_state_confidence_full_review_below: i32,
        #[serde(default = "defaults::run_token_budget")]
        pub run_token_budget: i64,
        #[serde(default = "defaults::run_max_llm_calls")]
        pub run_max_llm_calls: i32,
        #[serde(default = "defaults::simulation_token_budget")]
        pub simulation_token_budget: i64,
        /// 波 A1：reaction 路径单 run token 上限。
        #[serde(default = "defaults::reaction_token_budget")]
        pub reaction_token_budget: i64,
        /// 波 A1：reaction 路径单 run 最多 LLM 调用次数。
        #[serde(default = "defaults::reaction_max_llm_calls")]
        pub reaction_max_llm_calls: i32,
        /// agent-autonomy-loop W0 / Task 1.3：是否启用自治协议字段校验路径。
        /// sunset D+14；老 runtime 文档缺该字段时走 `true`（启用）。
        #[serde(default = "defaults::autonomy_protocol_enabled")]
        pub autonomy_protocol_enabled: bool,
        /// agent-autonomy-loop W0 / Task 1.3：知识路由模式。
        /// 取值 `auto_tool_loop`（默认）或 `classic_router`（灰度回退，sunset D+14）。
        #[serde(default = "defaults::knowledge_routing_mode")]
        pub knowledge_routing_mode: String,
        /// agent-autonomy-loop W0 / Task 1.3：`reply_with_tools_loop` 的最大轮数。
        /// 默认 3，loader 中 clamp 到 [1, 5]。
        #[serde(default = "defaults::knowledge_max_tool_loops")]
        pub knowledge_max_tool_loops: i32,
        /// agent-autonomy-loop W0 / Task 1.3：单 run 内 tool call 总次数上限。
        /// 默认 6，loader 中 clamp 到 [1, 16]。
        #[serde(default = "defaults::knowledge_max_tool_calls")]
        pub knowledge_max_tool_calls: i32,
        /// agent-autonomy-loop W0 / Task 1.3：`knowledge.open_slice` 单次入参 K 上限。
        /// 默认 4，loader 中 clamp 到 [1, 16]。
        #[serde(default = "defaults::knowledge_open_slice_max_k")]
        pub knowledge_open_slice_max_k: i32,
        /// agent-autonomy-loop W0 / Task 1.3：`knowledge.search` 默认 top_k。
        /// 默认 8，loader 中 clamp 到 [1, 32]。
        #[serde(default = "defaults::knowledge_search_top_k")]
        pub knowledge_search_top_k: i32,
        /// agent-autonomy-loop W0 / Task 1.3：outbox dispatcher 轮询间隔（秒）。
        /// 默认 5，loader 中 clamp 到 [1, 60]。
        #[serde(default = "defaults::outbox_poll_interval_seconds")]
        pub outbox_poll_interval_seconds: i32,
        /// agent-autonomy-loop W0 / Task 1.3：outbox dispatcher claim lease 时长（秒）。
        /// 默认 60，loader 中 clamp 到 [10, 600]。
        #[serde(default = "defaults::outbox_lease_seconds")]
        pub outbox_lease_seconds: i32,
        /// #69 作息门控：是否启用静默时段。默认 **true**（运营域配置，前端可改）。
        /// 开启后客户在静默时段来消息不立即回，排到醒来时段一次性回复；主动发送
        /// （planner/follow_up）静默时段到点则重排到醒来时刻。relay 转述豁免。
        #[serde(default = "defaults::quiet_hours_enabled")]
        pub quiet_hours_enabled: bool,
        /// #69 作息门控：静默起点小时（运营方进程本地时区，0..=23，含）。默认 22。
        #[serde(default = "defaults::quiet_hours_start")]
        pub quiet_hours_start: u32,
        /// #69 作息门控：静默终点 / 醒来小时（0..=23，不含）。默认 8。
        /// `start == end` 退化为永不静默；`start > end`（如 22→8）表示跨午夜。
        #[serde(default = "defaults::quiet_hours_end")]
        pub quiet_hours_end: u32,
        /// #69 作息门控：运营方时区相对 UTC 的小时偏移（如中国 +8）。默认 8。
        /// 用固定偏移而非 `chrono::Local`，使作息判定**不依赖部署宿主时区**。
        #[serde(default = "defaults::quiet_hours_tz_offset_hours")]
        pub quiet_hours_tz_offset_hours: i32,
    }

    impl Default for RuntimeParametersTyped {
        fn default() -> Self {
            Self {
                recent_message_limit: defaults::recent_message_limit(),
                min_reply_interval_seconds: defaults::min_reply_interval_seconds(),
                max_daily_touches: defaults::max_daily_touches(),
                max_pending_follow_ups: defaults::max_pending_follow_ups(),
                follow_up_expires_hours: defaults::follow_up_expires_hours(),
                cooldown_after_no_reply_hours: defaults::cooldown_after_no_reply_hours(),
                hallucination_block_at: defaults::hallucination_block_at(),
                pressure_risk_block_at: defaults::pressure_risk_block_at(),
                knowledge_grounding_block_below: defaults::knowledge_grounding_block_below(),
                human_like_rewrite_below: defaults::human_like_rewrite_below(),
                emotional_value_rewrite_below: defaults::emotional_value_rewrite_below(),
                operation_state_confidence_full_review_below:
                    defaults::operation_state_confidence_full_review_below(),
                run_token_budget: defaults::run_token_budget(),
                run_max_llm_calls: defaults::run_max_llm_calls(),
                simulation_token_budget: defaults::simulation_token_budget(),
                reaction_token_budget: defaults::reaction_token_budget(),
                reaction_max_llm_calls: defaults::reaction_max_llm_calls(),
                autonomy_protocol_enabled: defaults::autonomy_protocol_enabled(),
                knowledge_routing_mode: defaults::knowledge_routing_mode(),
                knowledge_max_tool_loops: defaults::knowledge_max_tool_loops(),
                knowledge_max_tool_calls: defaults::knowledge_max_tool_calls(),
                knowledge_open_slice_max_k: defaults::knowledge_open_slice_max_k(),
                knowledge_search_top_k: defaults::knowledge_search_top_k(),
                outbox_poll_interval_seconds: defaults::outbox_poll_interval_seconds(),
                outbox_lease_seconds: defaults::outbox_lease_seconds(),
                quiet_hours_enabled: defaults::quiet_hours_enabled(),
                quiet_hours_start: defaults::quiet_hours_start(),
                quiet_hours_end: defaults::quiet_hours_end(),
                quiet_hours_tz_offset_hours: defaults::quiet_hours_tz_offset_hours(),
            }
        }
    }

    impl From<RuntimeParametersTyped> for Document {
        fn from(p: RuntimeParametersTyped) -> Self {
            bson::to_document(&p).expect("RuntimeParametersTyped serializable")
        }
    }

    pub mod defaults {
        pub fn recent_message_limit() -> i64 {
            12
        }
        pub fn min_reply_interval_seconds() -> i64 {
            20
        }
        pub fn max_daily_touches() -> i64 {
            3
        }
        pub fn max_pending_follow_ups() -> i64 {
            3
        }
        pub fn follow_up_expires_hours() -> i64 {
            48
        }
        pub fn cooldown_after_no_reply_hours() -> i64 {
            24
        }
        pub fn hallucination_block_at() -> i32 {
            6
        }
        pub fn pressure_risk_block_at() -> i32 {
            7
        }
        pub fn knowledge_grounding_block_below() -> i32 {
            7
        }
        pub fn human_like_rewrite_below() -> i32 {
            6
        }
        pub fn emotional_value_rewrite_below() -> i32 {
            6
        }
        pub fn operation_state_confidence_full_review_below() -> i32 {
            4
        }
        pub fn run_token_budget() -> i64 {
            30000
        }
        pub fn run_max_llm_calls() -> i32 {
            6
        }
        pub fn simulation_token_budget() -> i64 {
            60000
        }
        pub fn reaction_token_budget() -> i64 {
            8000
        }
        pub fn reaction_max_llm_calls() -> i32 {
            2
        }
        // ── agent-autonomy-loop W0 / Task 1.3 新增默认值 ──
        pub fn autonomy_protocol_enabled() -> bool {
            true
        }
        pub fn knowledge_routing_mode() -> String {
            "auto_tool_loop".to_string()
        }
        pub fn knowledge_max_tool_loops() -> i32 {
            3
        }
        pub fn knowledge_max_tool_calls() -> i32 {
            6
        }
        pub fn knowledge_open_slice_max_k() -> i32 {
            4
        }
        pub fn knowledge_search_top_k() -> i32 {
            8
        }
        pub fn outbox_poll_interval_seconds() -> i32 {
            5
        }
        pub fn outbox_lease_seconds() -> i32 {
            60
        }
        // ── #69 作息门控默认值 ──
        pub fn quiet_hours_enabled() -> bool {
            true
        }
        pub fn quiet_hours_start() -> u32 {
            22
        }
        pub fn quiet_hours_end() -> u32 {
            8
        }
        pub fn quiet_hours_tz_offset_hours() -> i32 {
            8
        }
    }

    /// LP-12 / Task 21 / agent-autonomy-loop W5 task 6.1：MemoryCard 的强类型版本。
    ///
    /// 本结构是 [`super::OperatingMemory::memory_card`] 的 wire schema，作为整层
    /// 替换 `Document` 的"边界类型"。设计要点（task 6.1）：
    ///
    /// * `core_facts` / `recent_facts` / `deprecated_facts` 三类事实集合，
    ///   各自有写入侧 cap（6 / 10 / 20，由 `compact_memory_card_typed`
    ///   或 consolidator 在写入前强制）。事实条目类型 [`MemoryFactRepr`] 是
    ///   `#[serde(untagged)]` 包装：当前历史数据形态以 `String`（`Plain`）为主，
    ///   task 6.2 引入完整 [`MemoryFact`] 结构后会通过 `Structured(MemoryFact)`
    ///   分支落地，保证升级后的 round-trip 兼容性。
    /// * `coreProfile` / `relationshipState` 等 free-form 子文档由 `extra`
    ///   通过 `#[serde(flatten)]` 兜底承接（历史 wire shape 的 free-form
    ///   sub-document 形态）。曾经 typed 出 `core_profile` / `relationship_state`
    ///   两个字段会与 `extra` 同名键冲突，序列化产生重复 BSON 键导致下一次反序
    ///   `Kind: duplicate field 'coreProfile'`。修复后只保留 `extra` 一份。
    /// * `extra: Document` 通过 `#[serde(flatten)]` 兜底，承接所有未在本结构
    ///   显式声明的顶层字段（`coreProfile / relationshipState / preferences /
    ///   doNotDo / commitments / objections / openLoops / recentEpisodeSummary /
    ///   conflicts / source / version / coreFacts 旧字符串数组` 等），以保证：
    ///   1) 历史 BSON 文档反序列化不丢字段；
    ///   2) Document 版 helper（如 `compact_memory_card_with_previous`）
    ///      仍可在 `to_document()` 之后无缝消费整张卡。
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct MemoryCardTyped {
        #[serde(default)]
        pub core_facts: Vec<MemoryFactRepr>,
        #[serde(default)]
        pub recent_facts: Vec<MemoryFactRepr>,
        #[serde(default)]
        pub deprecated_facts: Vec<MemoryFactRepr>,
        /// `#[serde(flatten)]` catch-all：承接所有未在上述字段显式声明的顶层
        /// 字段，避免历史数据丢失（如 `coreProfile / relationshipState /
        /// preferences / doNotDo / commitments / objections / openLoops /
        /// recentEpisodeSummary / conflicts / source / version`），并允许
        /// 这些 free-form 子结构持续以 Document 形态共存。
        #[serde(flatten, default)]
        pub extra: Document,
    }

    /// agent-autonomy-loop W5 task 6.1：事实条目的反序列化容器。
    ///
    /// 当前历史数据中 `coreFacts / recentFacts` 几乎全部是 `Vec<String>`
    /// 形态；W5 task 6.2 会引入完整 [`MemoryFact`] 结构（含 id / text /
    /// confidence / importance / ...）。`#[serde(untagged)]` 让两种形态都能
    /// 反序列化进同一字段，避免现网数据迁移期间被 schema 锁死。
    ///
    /// task 6.1 仅需保证：
    /// * `Plain(String)` 分支可承接老数据；
    /// * `Structured(MemoryFact)` 分支以"占位 shell"（仅 `text` + 可选 `id`）
    ///   形式存在，task 6.2 再补齐字段。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(untagged)]
    pub enum MemoryFactRepr {
        /// 历史 / 简单形态：单个字符串即 fact text。
        Plain(String),
        /// 结构化形态：task 6.2 完整化（id / confidence / importance / ...）。
        /// 当前只保留 `text` 与可选 `id`，作为 forward-compat 占位。
        Structured(MemoryFact),
    }

    impl MemoryFactRepr {
        /// 取出 fact 的文本表示（无论 Plain / Structured 都能工作）。
        pub fn as_text(&self) -> &str {
            match self {
                MemoryFactRepr::Plain(s) => s.as_str(),
                MemoryFactRepr::Structured(fact) => fact.text.as_str(),
            }
        }
    }

    impl From<String> for MemoryFactRepr {
        fn from(s: String) -> Self {
            MemoryFactRepr::Plain(s)
        }
    }

    impl<'a> From<&'a str> for MemoryFactRepr {
        fn from(s: &'a str) -> Self {
            MemoryFactRepr::Plain(s.to_string())
        }
    }

    /// task 6.2：`MemoryFact` 的 `created_at / updated_at` 字段在反序列化期
    /// 缺失时的兜底 default。`bson::DateTime` 没有原生 `Default` 实现，因此
    /// 这里固定回退 Unix epoch；正常写入路径请用 `DateTime::now()`。
    fn default_epoch_dt() -> DateTime {
        DateTime::from_millis(0)
    }

    /// agent-autonomy-loop W5 task 6.2：完整 [`MemoryFact`] 强类型结构。
    ///
    /// 字段对照 design.md §3.5 的契约：
    ///
    /// | 字段                  | 类型                | 约束                                                        |
    /// |-----------------------|---------------------|-------------------------------------------------------------|
    /// | `id`                  | `String`            | UUIDv4，必填，作为 consolidator 合并 / 弃用 / 冲突的稳定锚 |
    /// | `text`                | `String`            | 1..=500 chars                                               |
    /// | `evidence`            | `Option<String>`    | ≤ 1000 chars                                                |
    /// | `confidence`          | `i32`               | 0..=10（Plain → 默认 7）                                    |
    /// | `importance`          | `i32`               | 0..=10（Plain → 默认 5）                                    |
    /// | `may_expire`          | `bool`              | —                                                           |
    /// | `deprecated_at`       | `Option<DateTime>`  | task 6.4 `apply_consolidator_deprecations` 写入             |
    /// | `deprecation_reason`  | `Option<String>`    | ≤ 200 chars                                                 |
    /// | `source_message_ids`  | `Vec<ObjectId>`     | ≤ 5（写入侧 cap）                                           |
    /// | `source_run_id`       | `Option<String>`    | 关联触发本条 fact 的 run_id（用于 trace）                    |
    /// | `created_at`          | `DateTime`          | first-seen 时间戳                                            |
    /// | `updated_at`          | `DateTime`          | 末次写入时间戳                                              |
    ///
    /// **稳定 id（task 6.2 / 6.4）**：所有迁移 / 反序列化 / consolidator
    /// 合并路径 SHALL 通过 `id` 字段（UUIDv4 字符串）锚定 fact，禁止用 `text`
    /// 作为身份 key（同义改写场景下 text 会变化）。Plain 升级路径生成 fresh
    /// UUID（见 [`From<MemoryFactRepr>`]），避免历史 `Vec<String>` 数据被多次
    /// 升级到不同 UUID 而无法匹配。
    ///
    /// **bounds 校验**：[`MemoryFact::validate`] 提供运行时长度 / 范围检查；
    /// task 6.4 的 `apply_consolidator_deprecations` 与 W2 校验链调用前会
    /// 执行该函数，违规 fact 将被 drop + warning。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct MemoryFact {
        /// UUIDv4 字符串。空字符串视为"老数据未升级"，由 task 6.6 迁移补 id；
        /// `From<MemoryFactRepr::Plain>` 路径直接生成 fresh UUID。
        #[serde(default)]
        pub id: String,
        /// fact 文本。1..=500 chars（[`MemoryFact::validate`] 校验）。
        #[serde(default)]
        pub text: String,
        /// 证据 / quote。≤ 1000 chars。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub evidence: Option<String>,
        /// 置信度 0..=10。Plain → 默认 7（design.md §3.5）。
        #[serde(default)]
        pub confidence: i32,
        /// 重要性 0..=10。Plain → 默认 5（design.md §3.5）。
        #[serde(default)]
        pub importance: i32,
        /// 是否易过期（提示 consolidator 优先抽查）。
        #[serde(default)]
        pub may_expire: bool,
        /// 弃用时间。`Some` 表示该 fact 已被移到 `deprecated_facts` 集合。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub deprecated_at: Option<DateTime>,
        /// 弃用理由。≤ 200 chars。仅在 `deprecated_at == Some` 时有意义。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub deprecation_reason: Option<String>,
        /// 触发本条 fact 的入站消息 id 列表。≤ 5（写入侧 cap）。
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub source_message_ids: Vec<ObjectId>,
        /// 触发本条 fact 的 run_id。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub source_run_id: Option<String>,
        /// 创建时间。
        #[serde(default = "default_epoch_dt")]
        pub created_at: DateTime,
        /// 末次更新时间。
        #[serde(default = "default_epoch_dt")]
        pub updated_at: DateTime,
        /// 兜底 `extra` 承接未识别的额外字段（前向兼容老数据 / 未来扩展）。
        #[serde(flatten, default)]
        pub extra: Document,
    }

    impl Default for MemoryFact {
        /// `bson::DateTime` 没有 `Default` 实现，因此 `MemoryFact` 需要手写
        /// `Default`：所有时间戳字段统一回退到 `DateTime::from_millis(0)`
        /// （Unix epoch），其余字段沿用零值 / 空集合。这个 default 仅用于
        /// 极少数"占位"场景（测试 / `Document` 兜底反序列化失败时），生产
        /// 路径请用 [`MemoryFact::from_plain_text`] 直接构造。
        fn default() -> Self {
            Self {
                id: String::new(),
                text: String::new(),
                evidence: None,
                confidence: 0,
                importance: 0,
                may_expire: false,
                deprecated_at: None,
                deprecation_reason: None,
                source_message_ids: Vec::new(),
                source_run_id: None,
                created_at: DateTime::from_millis(0),
                updated_at: DateTime::from_millis(0),
                extra: Document::new(),
            }
        }
    }

    impl MemoryFact {
        /// task 6.2：从 `Plain(text)` 升级时调用的工厂；生成 fresh UUIDv4 + 默认
        /// confidence=7 / importance=5（design.md §3.5）。
        pub fn from_plain_text(text: String) -> Self {
            let now = DateTime::now();
            Self {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                evidence: None,
                confidence: 7,
                importance: 5,
                may_expire: false,
                deprecated_at: None,
                deprecation_reason: None,
                source_message_ids: Vec::new(),
                source_run_id: None,
                created_at: now,
                updated_at: now,
                extra: Document::new(),
            }
        }

        /// task 6.2：bounds 校验。返回违规列表（空表示 OK）；调用方按违规
        /// 严重程度决定 drop fact 或追加 warning。具体 bound 见 struct
        /// doc-comment 表格。
        ///
        /// 字符长度统一按 Unicode `char` 计（与 R1 / R3 文本约束一致）。
        pub fn validate(&self) -> Vec<String> {
            let mut errors = Vec::new();
            let text_len = self.text.chars().count();
            if text_len == 0 {
                errors.push("memory_fact_text_empty".to_string());
            } else if text_len > 500 {
                errors.push(format!("memory_fact_text_over_500:{}", text_len));
            }
            if let Some(ev) = &self.evidence {
                let ev_len = ev.chars().count();
                if ev_len > 1000 {
                    errors.push(format!("memory_fact_evidence_over_1000:{}", ev_len));
                }
            }
            if !(0..=10).contains(&self.confidence) {
                errors.push(format!("memory_fact_confidence_out_of_range:{}", self.confidence));
            }
            if !(0..=10).contains(&self.importance) {
                errors.push(format!("memory_fact_importance_out_of_range:{}", self.importance));
            }
            if let Some(reason) = &self.deprecation_reason {
                let reason_len = reason.chars().count();
                if reason_len > 200 {
                    errors.push(format!(
                        "memory_fact_deprecation_reason_over_200:{}",
                        reason_len
                    ));
                }
            }
            if self.source_message_ids.len() > 5 {
                errors.push(format!(
                    "memory_fact_source_message_ids_over_5:{}",
                    self.source_message_ids.len()
                ));
            }
            errors
        }
    }

    /// task 6.2：`MemoryFactRepr → MemoryFact` 的反序列化兼容转换。
    ///
    /// * `Plain(text)`：生成 **fresh UUIDv4**（关键约束）+ 默认 confidence=7 /
    ///   importance=5 / created_at=updated_at=now。fresh UUID 是为了避免
    ///   老 `Vec<String>` 数据每次升级被分配不同 id 而导致 consolidator 合并失真。
    /// * `Structured(fact)`：透传（id 由迁移 task 6.6 或 consolidator 直接写入）。
    impl From<MemoryFactRepr> for MemoryFact {
        fn from(repr: MemoryFactRepr) -> Self {
            match repr {
                MemoryFactRepr::Plain(text) => MemoryFact::from_plain_text(text),
                MemoryFactRepr::Structured(fact) => fact,
            }
        }
    }

    impl MemoryCardTyped {
        /// 整层 typed 后，Document 版 helper 仍是合法消费者。本方法把 typed
        /// 结构序列化成 BSON `Document`，便于在迁移期把"读路径 typed +
        /// 写路径 Document helper"两边粘合。失败（极端情况下不可序列化）
        /// 时返回空 `Document`，避免 panic 把整个 run 拖垮。
        pub fn to_document(&self) -> Document {
            bson::to_document(self).unwrap_or_default()
        }

        /// 把任意 `Document`（含历史老数据）反序列化为 typed；不可识别字段
        /// 落入 `extra`，整体 round-trip 不丢字段。失败回退 `Default`。
        pub fn from_document(doc: &Document) -> Self {
            bson::from_document::<MemoryCardTyped>(doc.clone()).unwrap_or_default()
        }

        /// 与 `Document::is_empty` 语义对齐：当且仅当所有 typed 字段都空
        /// 且 `extra` 也空时返回 true，便于既有 `if !memory.memory_card.is_empty()`
        /// 调用方零改动迁移。
        pub fn is_empty(&self) -> bool {
            self.core_facts.is_empty()
                && self.recent_facts.is_empty()
                && self.deprecated_facts.is_empty()
                && self.extra.is_empty()
        }

        /// agent-autonomy-loop W5 / Task 6.7：检查 typed 三组事实数组中是否
        /// 存在 [`MemoryFactRepr::Plain`] 形态。返回 true 表示 caller（多为
        /// consolidator / simulation 种子）传入了老 `Vec<String>` 输入，需要
        /// 触发 `memory_facts_auto_upgraded` 警告并升级为结构化形态。
        pub fn has_plain_facts(&self) -> bool {
            self.core_facts
                .iter()
                .chain(self.recent_facts.iter())
                .chain(self.deprecated_facts.iter())
                .any(|fact| matches!(fact, MemoryFactRepr::Plain(_)))
        }

        /// agent-autonomy-loop W5 / Task 6.7：把 typed 三组事实数组中所有
        /// [`MemoryFactRepr::Plain`] 升级为 [`MemoryFactRepr::Structured`]，
        /// 字段走 [`MemoryFact::from_plain_text`] 默认值（fresh UUIDv4 +
        /// confidence=7 / importance=5）。返回升级条数；调用方据此决定是否
        /// 在审计 / 响应 body 中追加 `memory_facts_auto_upgraded` warning。
        pub fn auto_upgrade_plain_facts(&mut self) -> usize {
            let mut upgraded = 0usize;
            for repr in self
                .core_facts
                .iter_mut()
                .chain(self.recent_facts.iter_mut())
                .chain(self.deprecated_facts.iter_mut())
            {
                if let MemoryFactRepr::Plain(text) = repr {
                    let promoted = MemoryFact::from_plain_text(std::mem::take(text));
                    *repr = MemoryFactRepr::Structured(promoted);
                    upgraded += 1;
                }
            }
            upgraded
        }
    }

    /// agent-autonomy-loop M2：`Contact.commitments` 元素的反序列化容器。
    ///
    /// 与 [`MemoryFactRepr`] 相同的 `#[serde(untagged)]` 兼容套路：历史数据
    /// （`extra.commitments: Vec<String>` 或 M1 期之前 `Contact.last_commitment:
    /// Option<String>` 经迁移成的单元素 Vec）落入 `Plain`，新写入数据落入
    /// `Structured(CommitmentEntry)`，承诺到期扫描器 (`planner::scan_commitments`)
    /// 对前者跳过、对后者按 `due_at` 判定 overdue/imminent。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(untagged)]
    pub enum CommitmentRepr {
        /// 旧字符串形态（无 due_at）。Planner 不会对其 emit follow_up，等下次
        /// memoryCard rewrite 由 Reply Agent 重塑为 `Structured` 形态。
        Plain(String),
        /// 新结构化形态：含稳定 id / due_at / created_at。
        Structured(CommitmentEntry),
    }

    impl CommitmentRepr {
        /// 取出承诺文本，无论 Plain / Structured。
        pub fn text(&self) -> &str {
            match self {
                CommitmentRepr::Plain(s) => s.as_str(),
                CommitmentRepr::Structured(entry) => entry.text.as_str(),
            }
        }

        /// 取 `due_at`；`Plain` 无该信息，固定返回 `None`。
        pub fn due_at(&self) -> Option<DateTime> {
            match self {
                CommitmentRepr::Plain(_) => None,
                CommitmentRepr::Structured(entry) => entry.due_at,
            }
        }

        /// 取 commitment 的稳定 id；`Plain` 无 id，固定返回空串（调用方据此跳过）。
        pub fn id(&self) -> &str {
            match self {
                CommitmentRepr::Plain(_) => "",
                CommitmentRepr::Structured(entry) => entry.id.as_str(),
            }
        }
    }

    impl From<String> for CommitmentRepr {
        fn from(s: String) -> Self {
            CommitmentRepr::Plain(s)
        }
    }

    impl<'a> From<&'a str> for CommitmentRepr {
        fn from(s: &'a str) -> Self {
            CommitmentRepr::Plain(s.to_string())
        }
    }

    /// agent-autonomy-loop M2：结构化承诺条目。
    ///
    /// 字段对照：
    /// - `id`: UUIDv4 字符串，作为 Planner emit 幂等键（`agent_events.details.commitment_id`）。
    /// - `text`: 承诺文本；1..=500 chars（[`CommitmentEntry::validate`] 校验）。
    /// - `due_at`: 截止时间；可选（旧 `last_commitment` 字符串无此信息）。
    /// - `created_at`: 创建时间，迁移时回退到 `Contact.updated_at`。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct CommitmentEntry {
        #[serde(default)]
        pub id: String,
        #[serde(default)]
        pub text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub due_at: Option<DateTime>,
        #[serde(default = "default_epoch_dt")]
        pub created_at: DateTime,
        #[serde(flatten, default)]
        pub extra: Document,
    }

    impl CommitmentEntry {
        /// 从纯文本构造一条新承诺：fresh UUIDv4 + `created_at = now()`。
        pub fn from_plain_text(text: String) -> Self {
            Self {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                due_at: None,
                created_at: DateTime::now(),
                extra: Document::new(),
            }
        }

        /// bounds 校验，返回违规列表（空表示 OK）。
        pub fn validate(&self) -> Vec<String> {
            let mut errors = Vec::new();
            let text_len = self.text.chars().count();
            if text_len == 0 {
                errors.push("commitment_text_empty".to_string());
            } else if text_len > 500 {
                errors.push(format!("commitment_text_over_500:{}", text_len));
            }
            if self.id.is_empty() {
                errors.push("commitment_id_empty".to_string());
            }
            errors
        }
    }

    impl From<CommitmentRepr> for CommitmentEntry {
        fn from(repr: CommitmentRepr) -> Self {
            match repr {
                CommitmentRepr::Plain(text) => CommitmentEntry::from_plain_text(text),
                CommitmentRepr::Structured(entry) => entry,
            }
        }
    }

    /// Phase D / D1：intent 轨迹元素。每次 `record_user_reaction` 完成后追加一条，
    /// 上限 50 项滑窗（最早条目滚出）。`turn_index` 是该 contact 的回合序号
    /// （从已有 `conversation_messages` 行数估算或调用方递增）；`intent` 是
    /// reaction LLM 给出的归一化 outcomeStatus（`user_replied_buying_signal`
    /// / `user_replied_objection` / ...）；`objection_type` 当前从 reaction
    /// `objection_type` 字段提取，若 reaction agent 未填写则为 `None`。
    ///
    /// 反序列化兼容：缺字段全部 `#[serde(default)]`；老 contact 文档无该字段时
    /// 在 `Contact` 上以 `intent_trajectory: Vec<>` 默认空 Vec。
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct IntentTrajectoryEntry {
        #[serde(default)]
        pub turn_index: i32,
        #[serde(default)]
        pub intent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub objection_type: Option<String>,
        #[serde(default = "default_epoch_dt")]
        pub recorded_at: DateTime,
    }

    impl IntentTrajectoryEntry {
        /// 上限滑窗：保留最近 50 项，最早条目滚出。
        pub const MAX_ITEMS: usize = 50;
    }

    /// 波 D1：`OperationDomainConfig.state_machine` 的强类型版本。
    ///
    /// 与运行时 `crate::agent::guards::check_state_transition` 消费的字段对齐。
    /// 字段全部 `#[serde(default)]`，老文档缺字段时走默认值（避免破坏既有数据）。
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct OperationStateMachineTyped {
        #[serde(default)]
        pub states: Vec<OperationStateTyped>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct OperationStateTyped {
        #[serde(default)]
        pub key: String,
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub goal: String,
        #[serde(default)]
        pub allowed_actions: Vec<String>,
        #[serde(default)]
        pub allowed_from: Vec<String>,
        #[serde(default)]
        pub allow_from_any: bool,
        #[serde(default)]
        pub advance_signals: Vec<String>,
        #[serde(default)]
        pub cooldown_signals: Vec<String>,
        #[serde(default)]
        pub risk_rules: Vec<String>,
        #[serde(default)]
        pub success_criteria: Vec<String>,
    }

    impl From<OperationStateMachineTyped> for Document {
        fn from(machine: OperationStateMachineTyped) -> Self {
            bson::to_document(&machine).expect("OperationStateMachineTyped serializable")
        }
    }
}

pub use typed::{
    CommitmentEntry, CommitmentRepr, IntentTrajectoryEntry, MemoryCardTyped, MemoryFact,
    MemoryFactRepr, OperationStateMachineTyped, OperationStateTyped, RuntimeParametersTyped,
};

impl OperationDomainConfig {
    /// LP-12 / Task 21：把 `runtime_parameters` Document 转成强类型；
    /// 缺失字段走 default。出错（极端情况下不可序列化）时返回默认值。
    pub fn runtime_parameters_typed(&self) -> RuntimeParametersTyped {
        let bson = mongodb::bson::Bson::Document(self.runtime_parameters.clone());
        mongodb::bson::from_bson(bson).unwrap_or_default()
    }

    /// 波 D1：把 `state_machine` Document 转成强类型，便于运行时 guard 与
    /// 后台校验复用同一份 schema。缺失/损坏字段走 default。
    pub fn state_machine_typed(&self) -> OperationStateMachineTyped {
        let bson = mongodb::bson::Bson::Document(self.state_machine.clone());
        mongodb::bson::from_bson(bson).unwrap_or_default()
    }
}

// ──────────────────────────────────────────────────────────────────────────
// agent-self-evolution W0 (Task 1.1)：5 个新 collection 的占位结构。
//
// 业务字段在 W2/W3/W4 落地（threshold/prompt 候选、shadow replay、release
// 路径），这里仅给出 BSON 往返所需的最小字段集合，保证 `Database::experiments()`
// 等 typed accessor 编译通过且 `models_smoke` 单测可往返。
// ──────────────────────────────────────────────────────────────────────────

/// agent-self-evolution W0：`experiments` 信封（一次 tick 一条）。
/// Requirements 1.3 / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Experiment {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub experiment_id: String,
    pub workspace_id: String,
    pub account_id: String,
    /// `"collecting"` | `"evaluating"` | `"awaiting_admin"` | `"released"` | `"aborted"`。
    pub status: String,
    pub window_hours: i32,
    pub started_at: DateTime,
    pub updated_at: DateTime,
    pub finished_at: Option<DateTime>,
    #[serde(default)]
    pub cohort_threshold_run_ids: Vec<ObjectId>,
    #[serde(default)]
    pub cohort_prompt_run_ids: Vec<ObjectId>,
    #[serde(default)]
    pub budget_used_tokens: i64,
    #[serde(default)]
    pub budget_used_calls: i32,
    #[serde(default)]
    pub proposals_count: i32,
    #[serde(default)]
    pub proposals_eligible_count: i32,
}

/// agent-self-evolution W0：`proposals`（threshold 或 prompt 候选）。
/// Requirements 3.x / 4.x / 5.x / 6.x / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub experiment_id: String,
    pub workspace_id: String,
    pub account_id: String,
    /// `"threshold"` | `"prompt"`。
    pub proposal_kind: String,
    /// `"pending_eval"` | `"evaluating"` | `"eligible_for_release"`
    /// | `"rejected_below_threshold"` | `"released"` | `"rolled_back"`。
    pub status: String,
    /// threshold 类专用（如 `"fact_risk_block"` / `"planner_block_rate_threshold"`）。
    pub gate_key: Option<String>,
    /// threshold 类：当前生效值（写入时记快照）。
    pub current_value: Option<f64>,
    /// threshold 类：候选值。
    pub proposed_value: Option<f64>,
    /// threshold 候选生成的命中率统计 / cohort 备注（自由 Document）。
    #[serde(default)]
    pub cohort_notes: Document,
    /// prompt 类：模板 key（如 `"reply_agent_main"`）。
    pub proposed_template_key: Option<String>,
    /// prompt 类：要修订的 section（`"soul" | "system_contract" | "policy" | "operator_instruction"`）。
    pub proposed_section: Option<String>,
    /// prompt 类：Critic LLM 输出的 diff 摘要 / 摘要文本。
    pub diff_summary: Option<String>,
    pub diff_snippet: Option<String>,
    pub critic_reasoning: Option<String>,
    #[serde(default)]
    pub expected_improvement_on: Vec<String>,
    pub risk_note: Option<String>,
    /// release 路径回滚专用：被替换的旧 prompt 版本号（rollback 取回它）。
    pub previous_prompt_version: Option<String>,
    /// 显著性测试结果（design.md §4.6）。
    #[serde(default)]
    pub eval_metrics: Document,
    #[serde(default)]
    pub eval_replays_completed: i32,
    #[serde(default)]
    pub eval_replays_failed: i32,
    pub significance_passed: Option<bool>,
    pub failure_reason: Option<String>,
    pub released_at: Option<DateTime>,
    pub released_by: Option<String>,
    pub rolled_back_at: Option<DateTime>,
    pub rolled_back_by: Option<String>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// agent-self-evolution W0：`shadow_replays`（每条 proposal × source_run_id 一条）。
/// Requirements 5.x / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReplay {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub proposal_id: ObjectId,
    pub experiment_id: String,
    pub workspace_id: String,
    pub account_id: String,
    pub source_run_id: ObjectId,
    /// `"completed"` | `"failed"`。
    pub status: String,
    pub failure_reason: Option<String>,
    pub original_final_review_status: Option<String>,
    pub new_final_review_status: Option<String>,
    #[serde(default)]
    pub new_review_risks: Vec<String>,
    pub new_token_cost: Option<i64>,
    /// 5 闸命中布尔向量（fact_risk / pressure_risk / human_like / emotional / product_accuracy / planner_block_rate）。
    #[serde(default)]
    pub new_5gate_hit: Document,
    pub new_self_critique_addressed: Option<bool>,
    /// 与原 reply 的近似度（0.0~1.0）；W3 task 4.1 写 0.0 占位。
    #[serde(default)]
    pub similarity_to_original_text: f64,
    pub started_at: DateTime,
    pub finished_at: Option<DateTime>,
}

/// agent-self-evolution W0：`threshold_overrides`（按 gate_key 维度的发布覆盖层）。
/// Requirements 6.x / 8.1。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdOverride {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub gate_key: String,
    pub value: f64,
    pub source_proposal_id: ObjectId,
    pub released_at: DateTime,
    pub released_by: String,
    pub rolled_back_at: Option<DateTime>,
    pub rolled_back_by: Option<String>,
}

/// Phase C / C5：`threshold_overrides_audit` 不可变审计表。
///
/// 每次 release / rollback / auto-release 都写一条；与 `threshold_overrides`
/// 区别：threshold_overrides 是"当前生效层"（rollback 把 rolled_back_at 写出来
/// 即等价于失效），audit 表是"完整变更日志"，永不更新只追加，方便事后追因
/// "为什么 fact_risk_block 在 2026-04-12 从 6.0 跳到 5.5"。
///
/// `decided_by` 取值：
///   - `"admin:<id>"`：UI 点 RELEASE/ROLLBACK 触发；
///   - `"evolution_auto"`：closed-loop 自动 release（C5 后续启用，先把字段挂上）；
///   - `"evolution_release"` / `"evolution_rollback"`：worker 走自动通道时的细分。
///
/// `action` 枚举：`"released"` / `"rolled_back"` / `"auto_released"`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdOverrideAudit {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub gate_key: String,
    pub action: String,
    pub previous_value: Option<f64>,
    pub new_value: Option<f64>,
    pub source_proposal_id: ObjectId,
    pub decided_by: String,
    pub decided_at: DateTime,
    /// 触发本次变更时的 cohort 命中率（如可得）；纯审计字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_rate_observed: Option<f64>,
    /// 触发本次变更时的显著性指标（来自 proposal.eval_metrics 的快照）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub significance_metrics: Option<mongodb::bson::Document>,
}

/// agent-self-evolution W0：`post_release_reviews`（W4 Task 5.6 +24h 对比窗口评测）。
/// Requirements 9.7。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostReleaseReview {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub proposal_id: ObjectId,
    pub workspace_id: String,
    pub account_id: String,
    /// `"threshold"` | `"prompt"`。
    pub proposal_kind: String,
    pub released_at: DateTime,
    pub scheduled_at: DateTime,
    #[serde(default)]
    pub completed: bool,
    pub actual_send_success_rate_delta: Option<f64>,
    #[serde(default)]
    pub actual_5gate_hit_delta: Document,
    pub completed_at: Option<DateTime>,
}

// ── Knowledge Digest Workstation ──
//
// 完整 schema 注释见 `docs/data-and-api.md` 知识库日报工作站节。
// 严格不引入"接管 / 人工"语义；状态机用 AI 内部语言。

/// 单张日报卡片（嵌入 `KnowledgeDailyReport.cards` 与 `KnowledgeChatTask.cards`
/// 快照）。`severity` / `kind` / `suggested_action` 是封闭枚举，写库前必须经
/// 后端校验，不允许 LLM 输出未知值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDigestCard {
    /// 持久 id：前端勾选 / dismiss / single-card 派工都按这个引用。
    pub card_id: ObjectId,
    /// `chunk_missing_field` / `chunk_low_hit_rate` / `chunk_caused_block` /
    /// `pack_outdated` / `evolution_pending` / `evolution_released` / `freeform`
    pub kind: String,
    /// ≤ 60 字。后端在写库前截断，超长丢弃整张卡片。
    pub title: String,
    /// ≤ 200 字。同上截断规则。
    pub summary: String,
    /// `[{kind: "chunk"|"pack"|"item"|"run"|"evolution_proposal", id}]`；
    /// 写库前做外键存在性校验，不存在的 ref 整张卡片丢弃。
    #[serde(default)]
    pub target_refs: Vec<Document>,
    /// `fix_chunk` / `add_chunk` / `retag` / `review_evolution` / `dismiss` /
    /// `freeform`；前端按此值映射快捷动作按钮。
    pub suggested_action: String,
    /// `info` / `warn` / `critical`；R2.5 排序优先 critical。
    pub severity: String,
    /// 可选指标：命中率 / 拦截次数 / 缺字段数。
    /// `{name, value, threshold}`；前端展示成 metricChip。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<Document>,
}

/// 当日日报（每天 09:00 由 `KnowledgeDigestWorker` 合成；
/// `(account_id, report_date)` 复合 unique 索引保证一天一份）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDailyReport {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    /// 当日日期，`YYYY-MM-DD`，运营时区。
    pub report_date: String,
    pub generated_at: DateTime,
    /// `worker` / `manual`（手动重算）。
    pub generated_by: String,
    /// `ok` / `partial` / `failed`。封闭枚举。
    pub status: String,
    /// 失败时的错误分类，与 `AppError::LlmUnavailable.kind` 同源；
    /// 成功时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    /// `{tokens_used, llm_calls}`，来自 `RunBudgetSnapshot`。
    #[serde(default)]
    pub budget_snapshot: Document,
    #[serde(default)]
    pub cards: Vec<KnowledgeDigestCard>,
    /// 运营点过"忽略"的卡片 id，画布据此灰显或隐藏。
    #[serde(default)]
    pub dismissed_card_ids: Vec<ObjectId>,
    /// 本次合成用到的 prompt 版本号；查问题用。
    #[serde(default)]
    pub prompt_versions: Document,
}

/// `KnowledgeChatTask.status` 的封闭枚举。任何 DB 写入若不属于该集合应被拒绝。
/// 历史值 `"finished"` 已重命名为 `"completed"`（P2-12）。
pub const ALLOWED_TASK_STATUS: &[&str] =
    &["pending", "running", "completed", "failed", "cancelled"];

/// 长任务（运营在 chat 派工 ≥ 3 cards 或预估 LLM call > 6 时生成；
/// 由 `KnowledgeTaskWorker` 30s 轮询、按 sessionId 串行执行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeChatTask {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub session_id: String,
    pub operator_id: Option<String>,
    /// 任务起源 cards 快照（运营勾的那批；不动 `knowledge_daily_reports`）。
    #[serde(default)]
    pub cards: Vec<KnowledgeDigestCard>,
    /// `[{cardId, action, targetChunkId?, hint?}]`，由
    /// `knowledge.digest.dispatch` LLM 拆出，后端校验后落库。
    #[serde(default)]
    pub planned_steps: Vec<Document>,
    /// `[{cardId, action, chunkId?, error?}]`，worker 每完成一步追加。
    #[serde(default)]
    pub completed_steps: Vec<Document>,
    /// `pending` / `running` / `completed` / `failed` / `cancelled`。封闭枚举，
    /// 见 [`ALLOWED_TASK_STATUS`]。历史值 `"finished"` 已被 P2-12 重命名为
    /// `"completed"`：与 outbox / evolution 的 `"completed"` / `"failed"` 终态语义对齐，
    /// 避免一个项目里 task 终态用三种近义词（finished / completed / done）。
    pub status: String,
    /// 失败时的错误分类。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    pub created_at: DateTime,
    pub started_at: Option<DateTime>,
    pub finished_at: Option<DateTime>,
}

/// 运营偏好记忆（与 `contacts.memory_card` / `agents.soul.memory` 物理隔离）。
/// 设计动机见 `docs/agent-policy.md` 知识库日报工作站节 R5。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeOperatorMemory {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub operator_id: String,
    /// `preference` / `rejection` / `context`。封闭枚举。
    pub kind: String,
    pub content: String,
    pub created_at: DateTime,
    pub last_used_at: DateTime,
    pub expires_at: Option<DateTime>,
}

/// LLM 服务商配置。一个 workspace 可以同时存在多条记录，但只有一条 `is_active=true`
/// 才会被运行时 `LlmRegistry` 加载。`format` 决定底层 HTTP 协议形态：`openai`
/// 走 `POST {base_url}/chat/completions`（兼容 DeepSeek / mimo / Qwen 等），
/// `anthropic` 走 `POST {base_url}/v1/messages`（Anthropic Messages API）。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderConfig {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    /// 业务侧 slug，前端用它定位记录；同 workspace 内唯一。
    pub provider_id: String,
    pub name: String,
    /// 协议形态：`"openai"` | `"anthropic"`。
    pub format: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub retry_base_ms: Option<u64>,
    /// P1-5：是否支持 multimodal vision 输入。`true` 时该 provider 可被指派为
    /// workspace 视觉模型（`is_vision_active`）；缺省 `false`，保持向后兼容。
    #[serde(default)]
    pub supports_vision: bool,
    /// #574：是否被指派为本 workspace 的**专职视觉模型**。一个 workspace 至多
    /// 一条为 `true`。当 active 文字模型本身 `supports_vision=false` 时，
    /// `/import-apply-image` 改用这条记录处理图片；要求其 `supports_vision=true`。
    /// 与 `is_active`（文字主模型）正交：文字主模型与视觉副模型可以是两条不同记录。
    #[serde(default)]
    pub is_vision_active: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

/// 手写 `Debug`：`api_key` 走 [`crate::secret::mask_secret`] 掩码，避免
/// `tracing::*!(?cfg, ...)` / panic backtrace 把真值泄漏到日志后端。
impl std::fmt::Debug for LlmProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmProviderConfig")
            .field("id", &self.id)
            .field("workspace_id", &self.workspace_id)
            .field("provider_id", &self.provider_id)
            .field("name", &self.name)
            .field("format", &self.format)
            .field("base_url", &self.base_url)
            .field("api_key", &crate::secret::mask_secret(&self.api_key))
            .field("model", &self.model)
            .field("is_active", &self.is_active)
            .field("timeout_seconds", &self.timeout_seconds)
            .field("max_retries", &self.max_retries)
            .field("retry_base_ms", &self.retry_base_ms)
            .field("supports_vision", &self.supports_vision)
            .field("is_vision_active", &self.is_vision_active)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn runtime_parameters_typed_defaults_apply_when_missing() {
        let p: RuntimeParametersTyped =
            mongodb::bson::from_document(doc! {}).expect("default deserialize");
        assert_eq!(p.recent_message_limit, 12);
        assert_eq!(p.hallucination_block_at, 6);
        // C1：pressure_risk_block_at 缺字段默认 7（DEFAULT 逐字等价旧写死值）。
        assert_eq!(p.pressure_risk_block_at, 7);
        assert_eq!(p.run_token_budget, 30000);
        assert_eq!(p.run_max_llm_calls, 6);
    }

    #[test]
    fn runtime_parameters_typed_reads_existing_values() {
        let doc = doc! {
            "recentMessageLimit": 24,
            "hallucinationBlockAt": 8,
            "pressureRiskBlockAt": 9,
            "runTokenBudget": 50000_i64
        };
        let p: RuntimeParametersTyped = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(p.recent_message_limit, 24);
        assert_eq!(p.hallucination_block_at, 8);
        // C1：H9 情感场景可经运营域配置放宽压力阈值（如 9）。
        assert_eq!(p.pressure_risk_block_at, 9);
        assert_eq!(p.run_token_budget, 50000);
        // 其它字段 fallback 默认值。
        assert_eq!(p.knowledge_grounding_block_below, 7);
    }

    #[test]
    fn runtime_parameters_typed_into_document_roundtrip() {
        let p = RuntimeParametersTyped::default();
        let doc: Document = p.into();
        assert_eq!(doc.get_i64("recentMessageLimit").unwrap(), 12);
        assert_eq!(doc.get_i32("hallucinationBlockAt").unwrap(), 6);
    }

    #[test]
    fn memory_card_typed_default_is_empty_document_relationship() {
        // task 6.1：`coreProfile` / `relationshipState` 现在统一通过 `extra`
        // catch-all 承接（free-form Document 形态），typed 字段已删除以避免
        // serde flatten 同名键冲突写出重复 BSON 键。default 应是空 extra。
        let card: MemoryCardTyped = mongodb::bson::from_document(doc! {}).expect("default");
        assert!(card.core_facts.is_empty());
        assert!(card.recent_facts.is_empty());
        assert!(card.deprecated_facts.is_empty());
        assert!(card.extra.is_empty());
    }

    #[test]
    fn memory_card_typed_round_trips_legacy_string_facts() {
        // task 6.1：历史 `Vec<String>` 形态的 coreFacts/recentFacts 必须能
        // 反序列化进 typed.core_facts / recent_facts（走 MemoryFactRepr::Plain
        // 分支），写回 Document 后字段顺序与原值一致。
        let legacy = doc! {
            "coreFacts": ["fact_a", "fact_b"],
            "recentFacts": ["recent_1"],
            "preferences": ["likes 中文"],
            "version": 3_i32,
        };
        let card: MemoryCardTyped =
            mongodb::bson::from_document(legacy.clone()).expect("legacy deserializes");
        assert_eq!(card.core_facts.len(), 2);
        assert_eq!(card.core_facts[0].as_text(), "fact_a");
        assert_eq!(card.recent_facts[0].as_text(), "recent_1");
        // preferences / version 落入 extra 兜底字段。
        assert!(card.extra.contains_key("preferences"));
        assert_eq!(card.extra.get_i32("version").unwrap(), 3);
        let round_tripped = card.to_document();
        assert_eq!(
            round_tripped.get_array("coreFacts").unwrap().len(),
            2,
            "core_facts round-trip 不丢条目"
        );
    }

    /// task 6.2：`MemoryFact::from_plain_text` 给老 `Vec<String>` 元素
    /// 升级时 SHALL 生成 fresh UUIDv4 + confidence=7 + importance=5。
    /// fresh UUID 是关键：避免同一 Plain 文本两次升级被分配同一 id 而
    /// 在 consolidator 合并时被错误判定为"未变 fact"。
    #[test]
    fn memory_fact_from_plain_text_uses_fresh_uuid_and_default_scores() {
        let f = MemoryFact::from_plain_text("用户偏好微信沟通".to_string());
        assert_eq!(f.text, "用户偏好微信沟通");
        assert_eq!(f.confidence, 7, "design.md §3.5：Plain 默认 confidence=7");
        assert_eq!(f.importance, 5, "design.md §3.5：Plain 默认 importance=5");
        assert!(!f.may_expire);
        assert!(f.evidence.is_none());
        assert!(f.deprecated_at.is_none());
        assert!(f.deprecation_reason.is_none());
        assert!(f.source_message_ids.is_empty());
        assert!(f.source_run_id.is_none());
        // UUID 形态：恰好 36 chars，含 4 个 '-'，且能被 uuid::Uuid::parse_str 接受。
        assert_eq!(f.id.len(), 36, "UUIDv4 字符串长度恒为 36");
        assert_eq!(f.id.matches('-').count(), 4);
        let _ = uuid::Uuid::parse_str(&f.id).expect("id 必须是合法 UUID");

        // fresh：再升级一次同样文本应得到不同 UUID。
        let f2 = MemoryFact::from_plain_text("用户偏好微信沟通".to_string());
        assert_ne!(f.id, f2.id, "同文本两次升级 SHALL 生成不同 UUID");
    }

    /// task 6.2：`From<MemoryFactRepr>` 桥接两种反序列化形态：
    /// `Plain` → fresh-UUID shell，`Structured` → 原样透传。
    #[test]
    fn memory_fact_repr_into_memory_fact_bridges_plain_and_structured() {
        let plain = MemoryFactRepr::Plain("简短事实".to_string());
        let upgraded: MemoryFact = plain.into();
        assert_eq!(upgraded.text, "简短事实");
        assert_eq!(upgraded.confidence, 7);
        assert!(!upgraded.id.is_empty());

        // Structured 透传：同 id / 同 confidence / 同 importance 不被覆盖。
        let preset_id = uuid::Uuid::new_v4().to_string();
        let original = MemoryFact {
            id: preset_id.clone(),
            text: "已结构化的事实".to_string(),
            confidence: 9,
            importance: 8,
            ..MemoryFact::default()
        };
        let structured = MemoryFactRepr::Structured(original.clone());
        let passthrough: MemoryFact = structured.into();
        assert_eq!(passthrough.id, preset_id);
        assert_eq!(passthrough.confidence, 9);
        assert_eq!(passthrough.importance, 8);
        assert_eq!(passthrough.text, "已结构化的事实");
    }

    /// task 6.2：`MemoryFact::validate` 长度 / 范围校验。
    #[test]
    fn memory_fact_validate_enforces_bounds() {
        // 合法 fact：无违规。
        let ok = MemoryFact::from_plain_text("ok".to_string());
        assert!(ok.validate().is_empty());

        // text 空：违规。
        let empty_text = MemoryFact {
            text: String::new(),
            ..MemoryFact::from_plain_text("placeholder".to_string())
        };
        assert!(empty_text
            .validate()
            .iter()
            .any(|e| e == "memory_fact_text_empty"));

        // text 超 500：违规。
        let too_long = MemoryFact::from_plain_text("a".repeat(501));
        assert!(too_long
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_text_over_500:")));

        // confidence 超 10：违规。
        let bad_conf = MemoryFact {
            confidence: 11,
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_conf
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_confidence_out_of_range:")));

        // importance 负值：违规。
        let bad_imp = MemoryFact {
            importance: -1,
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_imp
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_importance_out_of_range:")));

        // evidence 超 1000 / deprecation_reason 超 200 / source_message_ids 超 5。
        let bad_evidence = MemoryFact {
            evidence: Some("e".repeat(1001)),
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_evidence
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_evidence_over_1000:")));

        let bad_reason = MemoryFact {
            deprecation_reason: Some("r".repeat(201)),
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(bad_reason
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_deprecation_reason_over_200:")));

        let too_many_sources = MemoryFact {
            source_message_ids: (0..6).map(|_| ObjectId::new()).collect(),
            ..MemoryFact::from_plain_text("ok".to_string())
        };
        assert!(too_many_sources
            .validate()
            .iter()
            .any(|e| e.starts_with("memory_fact_source_message_ids_over_5:")));
    }

    /// task 6.2：`MemoryFact` 完整结构 BSON round-trip 不丢字段；
    /// 含 evidence / confidence / importance / deprecated_at / source_*。
    #[test]
    fn memory_fact_bson_round_trip_preserves_all_fields() {
        let fact = MemoryFact {
            id: uuid::Uuid::new_v4().to_string(),
            text: "用户期望本周末签约".to_string(),
            evidence: Some("用户原话：周六签".to_string()),
            confidence: 9,
            importance: 8,
            may_expire: true,
            deprecated_at: None,
            deprecation_reason: None,
            source_message_ids: vec![ObjectId::new(), ObjectId::new()],
            source_run_id: Some("run-abc".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_010_000),
            extra: Document::new(),
        };
        let doc = mongodb::bson::to_document(&fact).expect("serialize");
        // camelCase wire shape：sourceMessageIds / sourceRunId / mayExpire / createdAt / updatedAt
        assert_eq!(doc.get_str("text").unwrap(), "用户期望本周末签约");
        assert_eq!(doc.get_str("evidence").unwrap(), "用户原话：周六签");
        assert_eq!(doc.get_i32("confidence").unwrap(), 9);
        assert_eq!(doc.get_i32("importance").unwrap(), 8);
        assert!(doc.get_bool("mayExpire").unwrap());
        assert_eq!(doc.get_array("sourceMessageIds").unwrap().len(), 2);
        assert_eq!(doc.get_str("sourceRunId").unwrap(), "run-abc");
        assert!(doc.contains_key("createdAt"));
        assert!(doc.contains_key("updatedAt"));
        // 反序列化回来全字段一致。
        let parsed: MemoryFact = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed, fact);
    }

    /// task 6.2：`MemoryCardTyped` 中 `Vec<MemoryFactRepr>` 既能承接老
    /// `Vec<String>`，也能承接新结构化形态；untagged enum 在两种 wire shape
    /// 下都不丢字段。
    #[test]
    fn memory_card_typed_accepts_mixed_plain_and_structured_facts() {
        let mixed = doc! {
            "coreFacts": [
                "纯字符串老 fact",
                {
                    "id": "11111111-2222-3333-4444-555555555555",
                    "text": "结构化事实",
                    "confidence": 9_i32,
                    "importance": 7_i32,
                    "mayExpire": false,
                }
            ]
        };
        let card: MemoryCardTyped =
            mongodb::bson::from_document(mixed).expect("mixed coreFacts deserializes");
        assert_eq!(card.core_facts.len(), 2);
        // Plain 分支：as_text 取出原字符串，但 id 仍然空（迁移 task 6.6 会补）。
        match &card.core_facts[0] {
            MemoryFactRepr::Plain(s) => assert_eq!(s, "纯字符串老 fact"),
            MemoryFactRepr::Structured(_) => panic!("first fact 应走 Plain 分支"),
        }
        // Structured 分支：id / confidence / importance 完整保留。
        match &card.core_facts[1] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.id, "11111111-2222-3333-4444-555555555555");
                assert_eq!(f.text, "结构化事实");
                assert_eq!(f.confidence, 9);
                assert_eq!(f.importance, 7);
            }
            MemoryFactRepr::Plain(_) => panic!("second fact 应走 Structured 分支"),
        }
    }

    /// agent-autonomy-loop W5 / Task 6.7：`auto_upgrade_plain_facts` 把所有
    /// Plain 形态升级为 Structured，并返回升级条数；已经是 Structured 的不计。
    #[test]
    fn auto_upgrade_plain_facts_promotes_only_plain_branches() {
        let mut card: MemoryCardTyped = mongodb::bson::from_document(doc! {
            "coreFacts": [
                "纯字符串 A",
                {
                    "id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "text": "已结构化",
                    "confidence": 8_i32,
                    "importance": 6_i32,
                    "mayExpire": false,
                }
            ],
            "recentFacts": ["纯字符串 R1", "纯字符串 R2"],
        })
        .expect("mixed deserialize");

        assert!(card.has_plain_facts(), "升级前应有 Plain 形态");
        let upgraded = card.auto_upgrade_plain_facts();
        assert_eq!(upgraded, 3, "三个 Plain 应当被全部升级");
        assert!(!card.has_plain_facts(), "升级后不应残留 Plain");

        // 升级后 Plain 入口的字段获得默认 confidence=7 / importance=5 + fresh UUID。
        match &card.core_facts[0] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.text, "纯字符串 A");
                assert_eq!(f.confidence, 7);
                assert_eq!(f.importance, 5);
                assert!(!f.id.is_empty(), "Plain 升级须分配 fresh UUIDv4");
            }
            MemoryFactRepr::Plain(_) => panic!("应已升级为 Structured"),
        }
        // 已经 Structured 的条目原样保留：id / confidence / importance 不变。
        match &card.core_facts[1] {
            MemoryFactRepr::Structured(f) => {
                assert_eq!(f.id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
                assert_eq!(f.confidence, 8);
                assert_eq!(f.importance, 6);
            }
            MemoryFactRepr::Plain(_) => panic!("Structured 应原样保留"),
        }
        // 二次调用应返回 0（幂等）。
        assert_eq!(card.auto_upgrade_plain_facts(), 0, "幂等：第二次升级应当 0 条");
    }

    #[test]
    fn operation_domain_config_runtime_parameters_typed_helper() {
        let cfg = OperationDomainConfig {
            id: None,
            workspace_id: "default".to_string(),
            domain: "user_operations".to_string(),
            name: "test".to_string(),
            goal: String::new(),
            methodology: String::new(),
            workflow: String::new(),
            tool_policy: String::new(),
            automation_policy: String::new(),
            review_policy: String::new(),
            runtime_parameters: doc! {
                "recentMessageLimit": 16,
                "runMaxLlmCalls": 4
            },
            state_machine: Document::new(),
            status: "active".to_string(),
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
        };
        let typed = cfg.runtime_parameters_typed();
        assert_eq!(typed.recent_message_limit, 16);
        assert_eq!(typed.run_max_llm_calls, 4);
    }

    /// 波 A2：AgentOutcomeMetric 在缺数据时字段是 None；前端能区分"暂无数据"
    /// 和"零成功率"。
    #[test]
    fn agent_outcome_metric_round_trips_null_fields() {
        let metric = AgentOutcomeMetric {
            id: "default:default:7d:2026-05-18".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            horizon: "7d".to_string(),
            date: "2026-05-18".to_string(),
            reply_rate: None,
            conversation_depth: None,
            ai_hold_cleared_rate: None,
            agent_block_rate: None,
            daily_run_count: 0,
            daily_run_token_total: 0,
            created_at: DateTime::now(),
        };
        // BSON round-trip 保持 None。
        let doc = mongodb::bson::to_document(&metric).expect("serialize metric");
        // 字段使用 snake_case（与原始 struct 字段保持一致；前端 JSON 由
        // `outcome_metric_json` 单独 camelCase 化）。
        assert!(matches!(
            doc.get("reply_rate"),
            Some(mongodb::bson::Bson::Null)
        ));
        assert!(matches!(
            doc.get("ai_hold_cleared_rate"),
            Some(mongodb::bson::Bson::Null)
        ));
        let parsed: AgentOutcomeMetric =
            mongodb::bson::from_document(doc).expect("deserialize metric");
        assert!(parsed.reply_rate.is_none());
        assert!(parsed.ai_hold_cleared_rate.is_none());
        assert!(parsed.agent_block_rate.is_none());
        assert!(parsed.conversation_depth.is_none());
    }

    /// 波 A2：旧 BSON 文档（`replyRate: 0.0` 与 `human_handoff_success_rate`
    /// 字段名）反序列化保留为 `Some(0.0)`，验证 `serde(alias)` 兼容性。
    #[test]
    fn agent_outcome_metric_reads_legacy_zero_value() {
        let legacy = doc! {
            "_id": "default:default:7d:2026-05-17",
            "workspace_id": "default",
            "account_id": "default",
            "horizon": "7d",
            "date": "2026-05-17",
            "reply_rate": 0.0,
            "conversation_depth": 0.0,
            "human_handoff_success_rate": 0.0,
            "agent_block_rate": 0.0,
            "daily_run_count": 0_i64,
            "daily_run_token_total": 0_i64,
            "created_at": DateTime::now(),
        };
        let parsed: AgentOutcomeMetric =
            mongodb::bson::from_document(legacy).expect("deserialize legacy");
        assert_eq!(parsed.reply_rate, Some(0.0));
        assert_eq!(parsed.ai_hold_cleared_rate, Some(0.0));
    }

    /// 波 D1：`OperationStateMachineTyped` 能从默认状态机正确反序列化 +
    /// `state_machine_typed` 辅助方法可用。
    #[test]
    fn state_machine_typed_round_trips_default() {
        use crate::prompts::default_user_operation_state_machine;
        let cfg = OperationDomainConfig {
            id: None,
            workspace_id: "default".to_string(),
            domain: "user_operations".to_string(),
            name: "x".to_string(),
            goal: String::new(),
            methodology: String::new(),
            workflow: String::new(),
            tool_policy: String::new(),
            automation_policy: String::new(),
            review_policy: String::new(),
            runtime_parameters: Document::new(),
            state_machine: default_user_operation_state_machine(),
            status: "active".to_string(),
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
        };
        let typed = cfg.state_machine_typed();
        assert!(!typed.states.is_empty());
        // cooldown state 必须 allowFromAny=true。
        let cooldown = typed
            .states
            .iter()
            .find(|s| s.key == "cooldown")
            .expect("cooldown state");
        assert!(cooldown.allow_from_any);
        // new_contact 在 allowedFrom 里包含自身（自循环显式写入）。
        let new_contact = typed
            .states
            .iter()
            .find(|s| s.key == "new_contact")
            .expect("new_contact state");
        assert!(new_contact.allowed_from.iter().any(|s| s == "new_contact"));
    }

    // ── agent-autonomy-loop W0 (Task 1.6) ──
    //
    // 为 W0 新增的三个 collection 占位 struct（`OutboxEntry / TaxonomyEntry /
    // TaxonomyCandidate`）写最小 BSON 往返 smoke 单元测试。仅验证序列化/反序列化
    // 不丢字段，不依赖 MongoDB 连接。
    //
    // NOTE: `ensure_indexes()` 的真实启动期返回 OK 路径（Requirements 8.1 / 13.1）
    // 由 W0 集成测试在 `tests/` 中通过 testcontainers 覆盖（需要运行中的 Mongo），
    // 此处不在 lib unit test 范围内。

    /// W0 / Task 1.6：`OutboxEntry` BSON 往返保字段不丢（Requirements 13.1）。
    #[test]
    fn outbox_entry_bson_round_trip() {
        let now = DateTime::now();
        let decision_id = ObjectId::new();
        let entry = OutboxEntry {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "acct-1".to_string(),
            contact_wxid: "wxid_test_001".to_string(),
            run_id: "run-uuid-001".to_string(),
            decision_id: Some(decision_id),
            source_event_id: "evt-source-001".to_string(),
            source_kind: "inbound_message".to_string(),
            content: "你好，欢迎咨询".to_string(),
            content_hash: "sha256:abcdef".to_string(),
            idempotency_key: "evt-source-001:wxid_test_001:sha256:abcdef".to_string(),
            attempt: 0,
            max_attempts: 3,
            status: "pending".to_string(),
            cancel_reason: None,
            last_error: None,
            next_retry_at: None,
            worker_id: None,
            locked_until: None,
            reclaimed_in_flight: false,
            created_at: now,
            updated_at: now,
            sent_at: None,
        };

        let doc = mongodb::bson::to_document(&entry).expect("serialize OutboxEntry");
        // `_id` 在 None 时被 skip_serializing_if 忽略。
        assert!(!doc.contains_key("_id"));
        // 关键字段透传不丢。
        assert_eq!(doc.get_str("workspace_id").unwrap(), "default");
        assert_eq!(doc.get_str("account_id").unwrap(), "acct-1");
        assert_eq!(doc.get_str("contact_wxid").unwrap(), "wxid_test_001");
        assert_eq!(doc.get_str("run_id").unwrap(), "run-uuid-001");
        assert_eq!(
            doc.get_str("idempotency_key").unwrap(),
            "evt-source-001:wxid_test_001:sha256:abcdef"
        );
        assert_eq!(doc.get_str("status").unwrap(), "pending");

        let parsed: OutboxEntry =
            mongodb::bson::from_document(doc).expect("deserialize OutboxEntry");
        assert_eq!(parsed.workspace_id, entry.workspace_id);
        assert_eq!(parsed.account_id, entry.account_id);
        assert_eq!(parsed.contact_wxid, entry.contact_wxid);
        assert_eq!(parsed.run_id, entry.run_id);
        assert_eq!(parsed.decision_id, Some(decision_id));
        assert_eq!(parsed.source_event_id, entry.source_event_id);
        assert_eq!(parsed.source_kind, entry.source_kind);
        assert_eq!(parsed.content, entry.content);
        assert_eq!(parsed.idempotency_key, entry.idempotency_key);
        assert_eq!(parsed.status, "pending");
        assert_eq!(parsed.attempt, 0);
        assert_eq!(parsed.max_attempts, 3);
        assert!(parsed.cancel_reason.is_none());
        assert!(parsed.sent_at.is_none());
    }

    /// W0 / Task 1.6：`TaxonomyEntry` BSON 往返；同时验证 `TaxonomyValue` 的
    /// camelCase rename（`display_name` → `displayName`），保证 `(scope, kind,
    /// value.id)` 唯一索引路径与序列化字段一致（Requirements 8.1）。
    #[test]
    fn taxonomy_entry_bson_round_trip() {
        let now = DateTime::now();
        let entry = TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: "first_contact".to_string(),
                display_name: "首次接触".to_string(),
                description: "首次建立联系，尚未深入沟通".to_string(),
                aliases: vec!["新客".to_string(), "first-contact".to_string()],
                status: "active".to_string(),
                priority_weight: None,
                is_terminal: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
        };

        let doc = mongodb::bson::to_document(&entry).expect("serialize TaxonomyEntry");
        assert_eq!(doc.get_str("scope").unwrap(), "global");
        assert_eq!(doc.get_str("kind").unwrap(), "customer_stage");
        // value 是嵌套 Document；其 `id` / `displayName` 字段正确地以 camelCase
        // 写出（`displayName` 而非 `display_name`，与 R8.1 索引路径一致）。
        let value_doc = doc.get_document("value").expect("value document");
        assert_eq!(value_doc.get_str("id").unwrap(), "first_contact");
        assert_eq!(value_doc.get_str("displayName").unwrap(), "首次接触");
        // 反向：snake_case 字段不应出现。
        assert!(!value_doc.contains_key("display_name"));
        // 反向：唯一索引路径用的是 `value.id`，绝不是 `value.value_id`。
        assert!(!value_doc.contains_key("value_id"));
        let aliases = value_doc.get_array("aliases").expect("aliases array");
        assert_eq!(aliases.len(), 2);
        assert_eq!(value_doc.get_str("status").unwrap(), "active");

        let parsed: TaxonomyEntry =
            mongodb::bson::from_document(doc).expect("deserialize TaxonomyEntry");
        assert_eq!(parsed.scope, "global");
        assert_eq!(parsed.kind, "customer_stage");
        assert_eq!(parsed.value.id, "first_contact");
        assert_eq!(parsed.value.display_name, "首次接触");
        assert_eq!(parsed.value.description, "首次建立联系，尚未深入沟通");
        assert_eq!(parsed.value.aliases.len(), 2);
        assert_eq!(parsed.value.aliases[0], "新客");
        assert_eq!(parsed.value.status, "active");
    }

    /// W0 / Task 1.6：`TaxonomyCandidate` BSON 往返保字段不丢（Requirements 8.1）。
    #[test]
    fn taxonomy_candidate_bson_round_trip() {
        let now = DateTime::now();
        let candidate = TaxonomyCandidate {
            id: None,
            scope: "default".to_string(),
            kind: "objection_type".to_string(),
            raw_value: "价格敏感_新词".to_string(),
            evidence: Some("用户多次询问折扣并对比同类产品".to_string()),
            confidence: 7,
            first_seen_at: now,
            last_seen_at: now,
            occurrences: 1,
            status: "pending".to_string(),
            reviewed_at: None,
            reviewed_by: None,
        };

        let doc = mongodb::bson::to_document(&candidate).expect("serialize TaxonomyCandidate");
        assert_eq!(doc.get_str("scope").unwrap(), "default");
        assert_eq!(doc.get_str("kind").unwrap(), "objection_type");
        assert_eq!(doc.get_str("raw_value").unwrap(), "价格敏感_新词");
        assert_eq!(doc.get_str("status").unwrap(), "pending");
        assert_eq!(doc.get_i32("confidence").unwrap(), 7);
        assert_eq!(doc.get_i32("occurrences").unwrap(), 1);

        let parsed: TaxonomyCandidate =
            mongodb::bson::from_document(doc).expect("deserialize TaxonomyCandidate");
        assert_eq!(parsed.scope, candidate.scope);
        assert_eq!(parsed.kind, candidate.kind);
        assert_eq!(parsed.raw_value, candidate.raw_value);
        assert_eq!(
            parsed.evidence.as_deref(),
            Some("用户多次询问折扣并对比同类产品")
        );
        assert_eq!(parsed.confidence, 7);
        assert_eq!(parsed.occurrences, 1);
        assert_eq!(parsed.status, "pending");
        assert!(parsed.reviewed_at.is_none());
        assert!(parsed.reviewed_by.is_none());
    }

    // ── agent-self-evolution W0 (Task 1.6) ──
    //
    // 5 个新 collection 的 BSON 往返 smoke：保证字段命名 / Option / 嵌套 Document
    // 在序列化后能还原；W2/W3/W4 落地业务字段后再补显著性 / release 路径专项测试。

    #[test]
    fn experiment_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let exp = Experiment {
            id: None,
            experiment_id: "exp_2026_05_001".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            status: "collecting".to_string(),
            window_hours: 72,
            started_at: now,
            updated_at: now,
            finished_at: None,
            cohort_threshold_run_ids: vec![mongodb::bson::oid::ObjectId::new()],
            cohort_prompt_run_ids: vec![],
            budget_used_tokens: 0,
            budget_used_calls: 0,
            proposals_count: 0,
            proposals_eligible_count: 0,
        };
        let doc = mongodb::bson::to_document(&exp).expect("serialize Experiment");
        assert_eq!(doc.get_str("experiment_id").unwrap(), "exp_2026_05_001");
        assert_eq!(doc.get_str("status").unwrap(), "collecting");
        assert_eq!(doc.get_i32("window_hours").unwrap(), 72);
        let parsed: Experiment =
            mongodb::bson::from_document(doc).expect("deserialize Experiment");
        assert_eq!(parsed.experiment_id, exp.experiment_id);
        assert_eq!(parsed.cohort_threshold_run_ids.len(), 1);
        assert!(parsed.cohort_prompt_run_ids.is_empty());
        assert!(parsed.finished_at.is_none());
    }

    #[test]
    fn proposal_bson_round_trip_threshold_kind() {
        let now = mongodb::bson::DateTime::now();
        let p = Proposal {
            id: None,
            experiment_id: "exp_001".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "pending_eval".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(7.0),
            cohort_notes: doc! { "hit_rate_observed": 0.42 },
            proposed_template_key: None,
            proposed_section: None,
            diff_summary: None,
            diff_snippet: None,
            critic_reasoning: None,
            expected_improvement_on: vec![],
            risk_note: None,
            previous_prompt_version: None,
            eval_metrics: doc! {},
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: None,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: now,
            updated_at: now,
        };
        let doc = mongodb::bson::to_document(&p).expect("serialize Proposal");
        assert_eq!(doc.get_str("proposal_kind").unwrap(), "threshold");
        assert_eq!(doc.get_str("gate_key").unwrap(), "fact_risk_block");
        assert_eq!(doc.get_f64("current_value").unwrap(), 6.0);
        let cohort = doc.get_document("cohort_notes").unwrap();
        assert_eq!(cohort.get_f64("hit_rate_observed").unwrap(), 0.42);
        let parsed: Proposal =
            mongodb::bson::from_document(doc).expect("deserialize Proposal");
        assert_eq!(parsed.proposed_value, Some(7.0));
        assert!(parsed.proposed_template_key.is_none());
    }

    #[test]
    fn proposal_bson_round_trip_prompt_kind() {
        let now = mongodb::bson::DateTime::now();
        let p = Proposal {
            id: None,
            experiment_id: "exp_002".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            proposal_kind: "prompt".to_string(),
            status: "pending_eval".to_string(),
            gate_key: None,
            current_value: None,
            proposed_value: None,
            cohort_notes: doc! {},
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("强化 product fact-check 兜底语句".to_string()),
            diff_snippet: Some("…在引用产品参数前必须确认 knowledge chunk…".to_string()),
            critic_reasoning: Some("过去 30 条失败 cohort 中 12 条触发 fact_risk_block".to_string()),
            expected_improvement_on: vec!["blocked_unverified_product_claim".to_string()],
            risk_note: None,
            previous_prompt_version: Some("v3".to_string()),
            eval_metrics: doc! {},
            eval_replays_completed: 0,
            eval_replays_failed: 0,
            significance_passed: None,
            failure_reason: None,
            released_at: None,
            released_by: None,
            rolled_back_at: None,
            rolled_back_by: None,
            created_at: now,
            updated_at: now,
        };
        let doc = mongodb::bson::to_document(&p).expect("serialize Proposal");
        assert_eq!(doc.get_str("proposal_kind").unwrap(), "prompt");
        assert_eq!(doc.get_str("proposed_section").unwrap(), "policy");
        let parsed: Proposal =
            mongodb::bson::from_document(doc).expect("deserialize Proposal");
        assert_eq!(parsed.proposed_template_key.as_deref(), Some("reply_agent_main"));
        assert_eq!(parsed.expected_improvement_on.len(), 1);
        assert_eq!(parsed.previous_prompt_version.as_deref(), Some("v3"));
    }

    #[test]
    fn shadow_replay_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let proposal_id = mongodb::bson::oid::ObjectId::new();
        let source_run_id = mongodb::bson::oid::ObjectId::new();
        let r = ShadowReplay {
            id: None,
            proposal_id,
            experiment_id: "exp_003".to_string(),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            source_run_id,
            status: "completed".to_string(),
            failure_reason: None,
            original_final_review_status: Some("blocked_unverified_product_claim".to_string()),
            new_final_review_status: Some("approved".to_string()),
            new_review_risks: vec!["pressure_risk:hard_close".to_string()],
            new_token_cost: Some(1234),
            new_5gate_hit: doc! { "fact_risk": false, "pressure_risk": true },
            new_self_critique_addressed: Some(true),
            similarity_to_original_text: 0.0,
            started_at: now,
            finished_at: Some(now),
        };
        let doc = mongodb::bson::to_document(&r).expect("serialize ShadowReplay");
        assert_eq!(doc.get_str("status").unwrap(), "completed");
        assert_eq!(doc.get_f64("similarity_to_original_text").unwrap(), 0.0);
        let parsed: ShadowReplay =
            mongodb::bson::from_document(doc).expect("deserialize ShadowReplay");
        assert_eq!(parsed.proposal_id, proposal_id);
        assert_eq!(parsed.source_run_id, source_run_id);
        assert_eq!(parsed.new_token_cost, Some(1234));
        assert!(parsed.new_self_critique_addressed.unwrap());
    }

    #[test]
    fn threshold_override_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let proposal_id = mongodb::bson::oid::ObjectId::new();
        let o = ThresholdOverride {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            gate_key: "human_like_score_rewrite".to_string(),
            value: 6.5,
            source_proposal_id: proposal_id,
            released_at: now,
            released_by: "admin@local".to_string(),
            rolled_back_at: None,
            rolled_back_by: None,
        };
        let doc = mongodb::bson::to_document(&o).expect("serialize ThresholdOverride");
        assert_eq!(doc.get_str("gate_key").unwrap(), "human_like_score_rewrite");
        assert_eq!(doc.get_f64("value").unwrap(), 6.5);
        let parsed: ThresholdOverride =
            mongodb::bson::from_document(doc).expect("deserialize ThresholdOverride");
        assert_eq!(parsed.gate_key, "human_like_score_rewrite");
        assert_eq!(parsed.value, 6.5);
        assert_eq!(parsed.source_proposal_id, proposal_id);
        assert!(parsed.rolled_back_at.is_none());
    }

    #[test]
    fn post_release_review_bson_round_trip() {
        let now = mongodb::bson::DateTime::now();
        let proposal_id = mongodb::bson::oid::ObjectId::new();
        let r = PostReleaseReview {
            id: None,
            proposal_id,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            proposal_kind: "threshold".to_string(),
            released_at: now,
            scheduled_at: now,
            completed: false,
            actual_send_success_rate_delta: None,
            actual_5gate_hit_delta: doc! {},
            completed_at: None,
        };
        let doc = mongodb::bson::to_document(&r).expect("serialize PostReleaseReview");
        assert_eq!(doc.get_str("proposal_kind").unwrap(), "threshold");
        assert_eq!(doc.get_bool("completed").unwrap(), false);
        let parsed: PostReleaseReview =
            mongodb::bson::from_document(doc).expect("deserialize PostReleaseReview");
        assert_eq!(parsed.proposal_id, proposal_id);
        assert!(!parsed.completed);
        assert!(parsed.actual_send_success_rate_delta.is_none());
    }

    /// W0 / Task 1.6：M4 演化器迁移 ID 必须按字符串字典序排在 M3 末条之后，
    /// 否则 `migration_ids_are_chronologically_ordered` 测试将失败。
    /// 这里独立断言一次，给后续追加 M4_002 / M5 时一个明确锚点。
    #[test]
    fn m4_migration_id_is_after_2026_05_009() {
        // `0` (0x30) < `M` (0x4D)：`2026_05_009...` < `2026_05_M4_001...`。
        assert!("2026_05_009_contact_customer_stage_updated_at_backfill"
            < "2026_05_M4_001_prompt_template_versioned");
    }

    /// knowledge-wiki Phase A：旧 chunk 文档（无 wiki_type / domain_attributes /
    /// provenance / usage_stats / dynamic_confidence / locked_fields 等新字段）
    /// 必须能被新版 `OperationKnowledgeChunk` 反序列化，且新字段读出为 None。
    /// 这是前向兼容硬约束（CLAUDE.md R11）。
    #[test]
    fn legacy_chunk_doc_deserializes_with_new_fields_none() {
        let now = DateTime::now();
        let raw = doc! {
            "workspace_id": "default",
            "domain": "user_ops",
            "title": "legacy",
            "applicable_scenes": Vec::<String>::new(),
            "not_applicable_scenes": Vec::<String>::new(),
            "safe_claims": Vec::<String>::new(),
            "forbidden_claims": Vec::<String>::new(),
            "evidence_items": Vec::<String>::new(),
            "product_tags": Vec::<String>::new(),
            "business_topics": Vec::<String>::new(),
            "source_anchors": Vec::<Document>::new(),
            "distortion_risks": Vec::<String>::new(),
            "unsupported_claims": Vec::<String>::new(),
            "verified_claims": Vec::<String>::new(),
            "status": "draft",
            "priority": 1,
            "created_at": now,
            "updated_at": now,
        };
        let chunk: OperationKnowledgeChunk =
            mongodb::bson::from_document(raw).expect("legacy chunk deserialize");
        assert!(chunk.wiki_type.is_none());
        assert!(chunk.domain_attributes.is_none());
        assert!(chunk.provenance.is_none());
        assert!(chunk.valid_from.is_none());
        assert!(chunk.valid_to.is_none());
        assert!(chunk.superseded_by.is_none());
        assert!(chunk.previous_version_id.is_none());
        assert!(chunk.related_chunks.is_none());
        assert!(chunk.usage_stats.is_none());
        assert!(chunk.dynamic_confidence.is_none());
        assert!(chunk.integrity_score.is_none());
        assert!(chunk.locked_fields.is_none());
    }

    #[test]
    fn chunk_with_wiki_fields_roundtrip() {
        let now = DateTime::now();
        let prov = ChunkProvenance {
            source: "imported".to_string(),
            source_doc_id: Some("doc_42".to_string()),
            source_quote: Some("销售口径 v3 §3.1 …".to_string()),
            llm_model_alias: Some("provider_alias".to_string()),
            edited_at: now,
            edited_by: None,
        };
        let related = vec![RelatedRef {
            chunk_id: "chk_b".to_string(),
            kind: "references".to_string(),
            note: None,
        }];
        let stats = UsageStats {
            hit_count_30d: 7,
            blocked_count_30d: 1,
            last_used_at: Some(now),
            last_blocked_reason: Some("FactRisk:6".to_string()),
        };
        let chunk = OperationKnowledgeChunk {
            id: None,
            workspace_id: "default".to_string(),
            account_id: None,
            document_id: None,
            item_id: None,
            domain: "user_ops".to_string(),
            knowledge_type: None,
            business_context: None,
            title: "wiki sample".to_string(),
            summary: None,
            body: Some("…".to_string()),
            applicable_scenes: vec![],
            not_applicable_scenes: vec![],
            product_tags: vec![],
            business_topics: vec![],
            source_quote: None,
            source_anchors: vec![],
            integrity_status: Some("verified".to_string()),
            confidence_score: Some(85),
            status: "active".to_string(),
            priority: 1,
            created_at: now,
            updated_at: now,
            wiki_type: Some("methodology".to_string()),
            domain_attributes: Some(doc! { "customer_stage": "decision" }),
            provenance: Some(prov),
            valid_from: Some(now),
            valid_to: None,
            superseded_by: None,
            previous_version_id: None,
            related_chunks: Some(related),
            usage_stats: Some(stats),
            dynamic_confidence: Some(0.74),
            integrity_score: Some(0.92),
            locked_fields: Some(vec![
                "chunk_id".to_string(),
                "wiki_type".to_string(),
                "created_at".to_string(),
            ]),
            chunk_type: "product_fact".to_string(),
        };
        let doc = mongodb::bson::to_document(&chunk).expect("serialize chunk");
        assert_eq!(doc.get_str("wiki_type").unwrap(), "methodology");
        let parsed: OperationKnowledgeChunk =
            mongodb::bson::from_document(doc).expect("deserialize chunk");
        assert_eq!(parsed.wiki_type.as_deref(), Some("methodology"));
        assert_eq!(parsed.dynamic_confidence, Some(0.74));
        let stats = parsed.usage_stats.expect("usage_stats");
        assert_eq!(stats.hit_count_30d, 7);
        assert_eq!(stats.blocked_count_30d, 1);
        let related = parsed.related_chunks.expect("related_chunks");
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].kind, "references");
    }

    #[test]
    fn chunk_revision_roundtrip() {
        let now = DateTime::now();
        let rev = ChunkRevision {
            id: None,
            chunk_id: "chk_x".to_string(),
            revision_id: "rev_chk_x_001".to_string(),
            op: "patch".to_string(),
            patch: doc! { "summary": "new" },
            before_hash: "h_before".to_string(),
            after_hash: "h_after".to_string(),
            source: "ai".to_string(),
            reason: Some("补出处".to_string()),
            created_at: now,
            created_by: None,
        };
        let doc = mongodb::bson::to_document(&rev).expect("serialize ChunkRevision");
        assert_eq!(doc.get_str("op").unwrap(), "patch");
        let parsed: ChunkRevision =
            mongodb::bson::from_document(doc).expect("deserialize ChunkRevision");
        assert_eq!(parsed.revision_id, "rev_chk_x_001");
        assert_eq!(parsed.source, "ai");
    }

    #[test]
    fn knowledge_gap_signal_roundtrip() {
        let now = DateTime::now();
        let sig = KnowledgeGapSignal {
            id: None,
            signal_id: "gap_001".to_string(),
            workspace_id: "default".to_string(),
            kind: "broken_link".to_string(),
            title: "悬挂关系".to_string(),
            description: "chunk 引用已归档的目标".to_string(),
            affected_chunk_ids: vec!["chk_a".to_string(), "chk_b".to_string()],
            search_queries: vec![],
            severity: "warning".to_string(),
            source: "rule".to_string(),
            status: "pending".to_string(),
            resolution_note: None,
            created_at: now,
            resolved_at: None,
        };
        let doc = mongodb::bson::to_document(&sig).expect("serialize");
        let parsed: KnowledgeGapSignal =
            mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed.kind, "broken_link");
        assert_eq!(parsed.affected_chunk_ids.len(), 2);
    }

    #[test]
    fn domain_schema_roundtrip() {
        let now = DateTime::now();
        let s = DomainSchema {
            id: None,
            schema_id: "schema_sales".to_string(),
            workspace_id: "default".to_string(),
            name: "销售域 v1".to_string(),
            version: 1,
            fields: vec![DomainField {
                name: "customer_stage".to_string(),
                label: "客户阶段".to_string(),
                kind: "enum".to_string(),
                required: true,
                allowed_values: Some(vec!["lead".to_string(), "decision".to_string()]),
                alias_of: None,
            }],
            alias_dict: doc! { "客户阶段": "customer_stage" },
            guard_dsl: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        let doc = mongodb::bson::to_document(&s).expect("serialize");
        let parsed: DomainSchema = mongodb::bson::from_document(doc).expect("deserialize");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.fields[0].kind, "enum");
        assert_eq!(parsed.fields[0].required, true);
    }

    #[test]
    fn document_catalog_persisted_default_none() {
        // 旧 document 文档没有 catalog_summary_persisted / catalog_version 字段，
        // 仍可反序列化，新字段读出 None。
        let now = DateTime::now();
        let raw = doc! {
            "workspace_id": "default",
            "domain": "user_ops",
            "source_type": "manual",
            "title": "doc",
            "routing_map": Vec::<String>::new(),
            "risk_notes": Vec::<String>::new(),
            "product_tags": Vec::<String>::new(),
            "business_topics": Vec::<String>::new(),
            "line_index": Vec::<Document>::new(),
            "section_index": Vec::<Document>::new(),
            "status": "active",
            "version": 1,
            "created_at": now,
            "updated_at": now,
        };
        let d: OperationKnowledgeDocument =
            mongodb::bson::from_document(raw).expect("legacy doc deserialize");
        assert!(d.catalog_summary_persisted.is_none());
        assert!(d.catalog_version.is_none());
    }
}

#[cfg(test)]
mod principal_escalation_model_tests {
    use super::*;

    #[test]
    fn principal_escalation_status_closed_set_is_self_consistent() {
        assert!(ALLOWED_PRINCIPAL_ESCALATION_STATUS.contains(&PRINCIPAL_ESCALATION_STATUS_PENDING));
        assert!(ALLOWED_PRINCIPAL_ESCALATION_STATUS.contains(&PRINCIPAL_ESCALATION_STATUS_RESOLVED));
        assert_eq!(ALLOWED_PRINCIPAL_ESCALATION_STATUS.len(), 2);
    }

    #[test]
    fn awaiting_principal_decision_attr_key_is_stable() {
        // set（Task 18 apply_agent_updates）与 unset（Task 16 clear_awaiting_principal_state）
        // 必须用同一个 key，否则等待标记清不掉。锁死常量值防回归。
        assert_eq!(AWAITING_PRINCIPAL_DECISION_ATTR, "awaiting_principal_decision");
    }

    #[test]
    fn escalation_category_and_verdict_closed_sets_are_self_consistent() {
        assert_eq!(ALLOWED_ESCALATION_CATEGORY.len(), 3);
        assert_eq!(ALLOWED_PRINCIPAL_VERDICT.len(), 5);
        assert!(ALLOWED_PRINCIPAL_VERDICT.contains(&PRINCIPAL_VERDICT_DELEGATED_BACK));
    }

    #[test]
    fn escalation_request_deserializes_with_defaults() {
        let req: EscalationRequest =
            serde_json::from_str(r#"{"needed": true}"#).expect("should deserialize");
        assert!(req.needed);
        assert_eq!(req.category, None);
        assert!(!req.is_generalizable);
        assert!(req.self_serviceable_part.is_none());
    }

    #[test]
    fn escalation_request_empty_object_defaults_to_not_needed() {
        let req: EscalationRequest =
            serde_json::from_str("{}").expect("empty object should deserialize");
        assert!(!req.needed);
    }
}
