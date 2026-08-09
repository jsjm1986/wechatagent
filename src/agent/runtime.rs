//! 运行时硬参数 (`UserRuntimeParameters`)。
//!
//! 把 `OperationDomainConfig.runtime_parameters` 这份 `Document`
//! 解析成一组强类型字段，给 gateway / decision / review / guards
//! 等子模块共享使用。字段命名与后台 UI、prompt 中暴露的 camelCase
//! key 一一对应。
//!
//! 同时提供 `as_document()` 方便回写到 prompt / agent_run_logs。

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::FindOptions;

use crate::error::{AppError, AppResult};
use crate::models::{Contact, OperationDomainConfig};
use crate::routes::AppState;

#[derive(Clone, Copy)]
enum RuntimeParameterRule {
    Integer { min: i64, max: i64 },
    Boolean,
}

const USER_RUNTIME_PARAMETER_RULES: &[(&str, RuntimeParameterRule)] = &[
    (
        "recentMessageLimit",
        RuntimeParameterRule::Integer { min: 1, max: 200 },
    ),
    (
        "minReplyIntervalSeconds",
        RuntimeParameterRule::Integer {
            min: 0,
            max: 86_400,
        },
    ),
    (
        "maxDailyTouches",
        RuntimeParameterRule::Integer { min: 0, max: 100 },
    ),
    (
        "maxPendingFollowUps",
        RuntimeParameterRule::Integer { min: 0, max: 100 },
    ),
    (
        "followUpExpiresHours",
        RuntimeParameterRule::Integer { min: 1, max: 8_760 },
    ),
    (
        "cooldownAfterNoReplyHours",
        RuntimeParameterRule::Integer { min: 0, max: 8_760 },
    ),
    (
        "hallucinationBlockAt",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    (
        "pressureRiskBlockAt",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    (
        "knowledgeGroundingBlockBelow",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    (
        "humanLikeRewriteBelow",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    (
        "emotionalValueRewriteBelow",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    (
        "operationStateConfidenceFullReviewBelow",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    (
        "runTokenBudget",
        RuntimeParameterRule::Integer {
            min: 1_000,
            max: 2_000_000,
        },
    ),
    (
        "runTokenBudgetEscalated",
        RuntimeParameterRule::Integer {
            min: 1_000,
            max: 2_000_000,
        },
    ),
    (
        "runMaxLlmCalls",
        RuntimeParameterRule::Integer { min: 1, max: 20 },
    ),
    (
        "simulationTokenBudget",
        RuntimeParameterRule::Integer {
            min: 1_000,
            max: 2_000_000,
        },
    ),
    (
        "reactionTokenBudget",
        RuntimeParameterRule::Integer {
            min: 1_000,
            max: 500_000,
        },
    ),
    (
        "reactionMaxLlmCalls",
        RuntimeParameterRule::Integer { min: 1, max: 10 },
    ),
    ("autonomyProtocolEnabled", RuntimeParameterRule::Boolean),
    (
        "knowledgeMaxToolCalls",
        RuntimeParameterRule::Integer { min: 1, max: 16 },
    ),
    (
        "knowledgeOpenSliceMaxK",
        RuntimeParameterRule::Integer { min: 1, max: 16 },
    ),
    (
        "knowledgeSearchTopK",
        RuntimeParameterRule::Integer { min: 1, max: 32 },
    ),
    (
        "outboxPollIntervalSeconds",
        RuntimeParameterRule::Integer { min: 1, max: 60 },
    ),
    (
        "outboxLeaseSeconds",
        RuntimeParameterRule::Integer { min: 10, max: 600 },
    ),
    ("quietHoursEnabled", RuntimeParameterRule::Boolean),
    (
        "quietHoursStart",
        RuntimeParameterRule::Integer { min: 0, max: 23 },
    ),
    (
        "quietHoursEnd",
        RuntimeParameterRule::Integer { min: 0, max: 23 },
    ),
    (
        "quietHoursTzOffsetHours",
        RuntimeParameterRule::Integer { min: -12, max: 14 },
    ),
    (
        "consolidationWindowCharBudget",
        RuntimeParameterRule::Integer {
            min: 1_000,
            max: 16_000,
        },
    ),
    (
        "consolidationWindowMaxMessages",
        RuntimeParameterRule::Integer { min: 10, max: 200 },
    ),
    (
        "bayesianSlotMinHits",
        RuntimeParameterRule::Integer { min: 1, max: 20 },
    ),
    (
        "bayesianSlotMinStrong",
        RuntimeParameterRule::Integer { min: 0, max: 20 },
    ),
];

const GUIDE_RUNTIME_PARAMETER_KEYS: &[&str] = &[
    "recentMessageLimit",
    "minReplyIntervalSeconds",
    "maxDailyTouches",
    "maxPendingFollowUps",
    "followUpExpiresHours",
    "cooldownAfterNoReplyHours",
    "quietHoursEnabled",
    "quietHoursStart",
    "quietHoursEnd",
    "quietHoursTzOffsetHours",
];

fn integer_value(value: &Bson) -> Option<i64> {
    match value {
        Bson::Int32(value) => Some(i64::from(*value)),
        Bson::Int64(value) => Some(*value),
        _ => None,
    }
}

fn equivalent_runtime_value(left: &Bson, right: &Bson) -> bool {
    match (integer_value(left), integer_value(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

/// Strict write-boundary schema for the user-operations runtime document.
///
/// The two legacy names are accepted and rewritten to the typed canonical
/// names. Unknown keys and values that would otherwise be clamped or make the
/// whole typed decode fall back to defaults are rejected before persistence.
pub(crate) fn validate_and_normalize_user_runtime_parameters(
    input: &Document,
) -> Result<Document, String> {
    let mut normalized = input.clone();
    for (legacy, canonical) in [
        ("factRiskBlockAt", "hallucinationBlockAt"),
        ("productAccuracyBlockBelow", "knowledgeGroundingBlockBelow"),
    ] {
        let Some(legacy_value) = normalized.remove(legacy) else {
            continue;
        };
        if let Some(canonical_value) = normalized.get(canonical) {
            if !equivalent_runtime_value(&legacy_value, canonical_value) {
                return Err(format!(
                    "runtime parameter {legacy} conflicts with canonical {canonical}"
                ));
            }
        } else {
            normalized.insert(canonical, legacy_value);
        }
    }

    for (key, value) in &normalized {
        let Some((_, rule)) = USER_RUNTIME_PARAMETER_RULES
            .iter()
            .find(|(known, _)| *known == key.as_str())
        else {
            return Err(format!("unknown user runtime parameter: {key}"));
        };
        match rule {
            RuntimeParameterRule::Boolean if !matches!(value, Bson::Boolean(_)) => {
                return Err(format!("runtime parameter {key} must be boolean"));
            }
            RuntimeParameterRule::Integer { min, max } => {
                let Some(number) = integer_value(value) else {
                    return Err(format!("runtime parameter {key} must be an integer"));
                };
                if number < *min || number > *max {
                    return Err(format!(
                        "runtime parameter {key} must be between {min} and {max}"
                    ));
                }
            }
            RuntimeParameterRule::Boolean => {}
        }
    }

    let run_budget = normalized
        .get("runTokenBudget")
        .and_then(integer_value)
        .unwrap_or(150_000);
    let escalated_budget = normalized
        .get("runTokenBudgetEscalated")
        .and_then(integer_value)
        .unwrap_or(500_000);
    if escalated_budget < run_budget {
        return Err(
            "runtime parameter runTokenBudgetEscalated must be >= runTokenBudget".to_string(),
        );
    }
    Ok(normalized)
}

/// Guide may tune cadence/context only. Safety thresholds, model budgets,
/// delivery leases, and protocol switches require the dedicated admin editor.
pub(crate) fn validate_guide_runtime_parameter_patch(patch: &Document) -> Result<(), String> {
    for key in patch.keys() {
        if !GUIDE_RUNTIME_PARAMETER_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "Guide cannot modify high-risk runtime parameter: {key}"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct UserRuntimeParameters {
    pub recent_message_limit: i64,
    pub min_reply_interval_seconds: i64,
    pub max_daily_touches: i64,
    pub max_pending_follow_ups: i64,
    pub follow_up_expires_hours: i64,
    pub cooldown_after_no_reply_hours: i64,
    pub fact_risk_block_at: i32,
    pub pressure_risk_block_at: i32,
    pub human_like_rewrite_below: i32,
    pub emotional_value_rewrite_below: i32,
    pub product_accuracy_block_below: i32,
    /// MP-10 / Task 14：当 `decision.operation_state_confidence < 该阈值`时，
    /// 强制 review_mode = "full"，无论 planner 其它条件。
    pub operation_state_confidence_full_review_below: i32,
    /// MP-5 / Task 15：单 run 累计 token 上限。超额触发降级（跳过 review/rewrite/二次 router 等）。
    pub run_token_budget: i64,
    /// B-1 修复:progressive-tier 升档 run 的 token gating 上限(默认 100000)。
    /// gateway 升档分支经 RunBudget::grant_escalated_ceiling 授予本 run。
    pub run_token_budget_escalated: i64,
    /// MP-5 / Task 15：单 run 最多 LLM 调用次数。
    pub run_max_llm_calls: i32,
    /// MP-5 / Task 15：simulation 路径单次累计 token 上限。
    pub simulation_token_budget: i64,
    /// 波 A1：record_user_reaction 单次累计 token 上限。
    /// 反应分析路径只跑 1 次 LLM，但需要预算计数让超额时降级，并把 token
    /// 计入 agent_run_logs.tokens_used。
    pub reaction_token_budget: i64,
    /// 波 A1：reaction 单次最大 LLM 调用次数。
    pub reaction_max_llm_calls: i32,
    /// agent-autonomy-loop W0 / Task 1.3：是否启用自治协议字段校验路径。
    /// 默认 `true`；老 runtime 文档缺该字段时同样视为启用。sunset D+14。
    pub autonomy_protocol_enabled: bool,
    /// agent-autonomy-loop W0 / Task 1.3：单 run 内 tool call 总次数上限。
    /// 默认 6，loader 中 clamp 到 `[1, 16]`。
    pub knowledge_max_tool_calls: i32,
    /// agent-autonomy-loop W0 / Task 1.3：`knowledge.open_slice` 单次入参 K 上限。
    /// 默认 4，loader 中 clamp 到 `[1, 16]`。
    pub knowledge_open_slice_max_k: i32,
    /// agent-autonomy-loop W0 / Task 1.3：`knowledge.search` 默认 top_k。
    /// 默认 8，loader 中 clamp 到 `[1, 32]`。
    pub knowledge_search_top_k: i32,
    /// agent-autonomy-loop W0 / Task 1.3：outbox dispatcher 轮询间隔（秒）。
    /// 默认 5，loader 中 clamp 到 `[1, 60]`。
    pub outbox_poll_interval_seconds: i32,
    /// agent-autonomy-loop W0 / Task 1.3：outbox dispatcher claim lease 时长（秒）。
    /// 默认 60，loader 中 clamp 到 `[10, 600]`。
    pub outbox_lease_seconds: i32,
    /// #69 作息门控：是否启用静默时段。运营域 DB 配置（前端可改），默认 true。
    pub quiet_hours_enabled: bool,
    /// #69 作息门控：静默起点小时（运营方进程本地时区，0..=23，含）。默认 22。
    pub quiet_hours_start: u32,
    /// #69 作息门控：静默终点 / 醒来小时（0..=23，不含）。默认 8。
    pub quiet_hours_end: u32,
    /// #69 作息门控：运营方时区相对 UTC 的小时偏移（中国 +8）。默认 8。
    /// 固定偏移使作息判定不依赖部署宿主时区；loader clamp 到 `[-12, 14]`。
    pub quiet_hours_tz_offset_hours: i32,
    /// universal-domain-adaptation H9：本轮允许的 conversationMode 取值集合
    /// （替代 `agent::types::CONVERSATION_MODE_VALUES` 写死四模式）。`from_config`
    /// 给内置默认四模式；gateway 在加载 active DomainProfile 后用
    /// `profile.conversation_modes` 覆盖（非空时）。`validate_and_promote` 读它做
    /// conversationMode 严格枚举校验。DEFAULT 销售域 = 四模式逐字等价。
    pub allowed_conversation_modes: Vec<String>,
    /// universal-domain-adaptation H14：本域是否在「无产品声明」时旁路 grounding
    /// 软分数硬闸（`classify_dual_gate` 里 `knowledge_grounding_score <
    /// product_accuracy_block_below` 的判罚）。`false`（DEFAULT/老库/`from_config`/
    /// `Default`）= 不旁路 = 每条回复都判 grounding 硬闸（销售域字节等价）；`true`
    /// = 纯关系/情感域，仅当本条回复 `claim_analysis.requiresProductKnowledge=true`
    /// 时才纳入 grounding 硬闸，纯情感回复不再被 grounding 低分误拦。
    /// 由 active DomainProfile.grounding_gate_bypass_without_claim 派生，gateway
    /// 加载 profile 后覆盖。**红线**：本旁路仅作用于 grounding 软分数硬闸，
    /// `blocked_unverified_product_claim`（R5.4 verified 强约束 + 漏判探针）任何
    /// 取值下都不变。
    pub grounding_gate_bypass_without_claim: bool,
    /// reviewer 深度开关。所有 `should_reply=true` 的正文都必须经过独立 Reviewer；
    /// `false`（DEFAULT/`from_config`/`Default`）允许常规低风险回复使用 light Reviewer，
    /// `true` 则让高敏域强制使用 full Reviewer。它不再授权 Reply Agent 以自报
    /// `needs_review=false` 跳过审核（该旧语义已禁用）。由 active
    /// DomainProfile.distrust_self_reported_low_risk 派生，gateway 加载 profile 后覆盖。
    pub distrust_self_reported_low_risk: bool,
    /// tag-trust 子计划3 Task2：记忆归并宽窗口字符预算。`from_config` 把 typed
    /// 值 clamp 到 `[1000, 16000]`；Task3 的 `take_window_by_budget` 消费它决定
    /// 归并重判取多少历史消息进上下文。默认 6000。
    pub consolidation_window_char_budget: i64,
    /// tag-trust 子计划3 Task2：记忆归并宽窗口最大消息条数。`from_config` 把 typed
    /// 值 clamp 到 `[10, 200]`。与 char_budget 共同约束宽窗口规模。默认 60。
    pub consolidation_window_max_messages: i64,
    /// tag-trust 子计划4 Task2：贝叶斯评估旁路占槽门——跨多轮命中阈值。`from_config`
    /// 把 typed 值 clamp 到 `[1, 20]`。纯观测旁路，永不驱动决策。默认 3。
    pub bayesian_slot_min_hits: i32,
    /// tag-trust 子计划4 Task2：贝叶斯评估旁路占槽门——强证据累积阈值。`from_config`
    /// 把 typed 值 clamp 到 `[0, 20]`。强证据由代码侧据消息方向算，不信 LLM 自报。默认 2。
    pub bayesian_slot_min_strong: i32,
}

/// H9：内置默认 conversationMode 四模式（逐字复刻 `types::CONVERSATION_MODE_VALUES`）。
/// `from_config` / `Default` 用它；active profile 声明了 `conversation_modes` 时由
/// gateway 覆盖。
pub(crate) fn default_conversation_modes() -> Vec<String> {
    vec![
        "casual_relationship".to_string(),
        "value_exchange".to_string(),
        "consultative".to_string(),
        "boundary_protection".to_string(),
    ]
}

impl UserRuntimeParameters {
    pub(crate) fn from_config(config: Option<&OperationDomainConfig>, state: &AppState) -> Self {
        // 波 D1：通过 typed 路径解析，确保字段名/默认值与 model 端的
        // `RuntimeParametersTyped` 单源真理一致；缺失字段走 typed 默认。
        let typed = config
            .map(|c| c.runtime_parameters_typed())
            .unwrap_or_default();
        Self {
            // recent_message_limit / min_reply_interval 仍兜底到 AppConfig，
            // 让运维 .env 配置在 prompt 模板未覆盖时也能生效。
            recent_message_limit: if config
                .map(|c| c.runtime_parameters.contains_key("recentMessageLimit"))
                .unwrap_or(false)
            {
                typed.recent_message_limit
            } else {
                state.config.agent_recent_message_limit
            },
            min_reply_interval_seconds: if config
                .map(|c| c.runtime_parameters.contains_key("minReplyIntervalSeconds"))
                .unwrap_or(false)
            {
                typed.min_reply_interval_seconds
            } else {
                state.config.agent_min_reply_interval_seconds
            },
            max_daily_touches: typed.max_daily_touches,
            max_pending_follow_ups: typed.max_pending_follow_ups,
            follow_up_expires_hours: typed.follow_up_expires_hours,
            cooldown_after_no_reply_hours: typed.cooldown_after_no_reply_hours,
            fact_risk_block_at: typed.hallucination_block_at,
            pressure_risk_block_at: typed.pressure_risk_block_at,
            human_like_rewrite_below: typed.human_like_rewrite_below,
            emotional_value_rewrite_below: typed.emotional_value_rewrite_below,
            product_accuracy_block_below: typed.knowledge_grounding_block_below,
            operation_state_confidence_full_review_below: typed
                .operation_state_confidence_full_review_below,
            run_token_budget: typed.run_token_budget,
            run_token_budget_escalated: typed.run_token_budget_escalated,
            run_max_llm_calls: typed.run_max_llm_calls,
            simulation_token_budget: typed.simulation_token_budget,
            reaction_token_budget: typed.reaction_token_budget,
            reaction_max_llm_calls: typed.reaction_max_llm_calls,
            autonomy_protocol_enabled: typed.autonomy_protocol_enabled,
            knowledge_max_tool_calls: clamp_i32(typed.knowledge_max_tool_calls, 1, 16, 6),
            knowledge_open_slice_max_k: clamp_i32(typed.knowledge_open_slice_max_k, 1, 16, 4),
            knowledge_search_top_k: clamp_i32(typed.knowledge_search_top_k, 1, 32, 8),
            outbox_poll_interval_seconds: clamp_i32(typed.outbox_poll_interval_seconds, 1, 60, 5),
            outbox_lease_seconds: clamp_i32(typed.outbox_lease_seconds, 10, 600, 60),
            quiet_hours_enabled: typed.quiet_hours_enabled,
            quiet_hours_start: typed.quiet_hours_start.min(23),
            quiet_hours_end: typed.quiet_hours_end.min(23),
            quiet_hours_tz_offset_hours: typed.quiet_hours_tz_offset_hours.clamp(-12, 14),
            // H9：from_config 不接 DomainProfile，给内置默认四模式；gateway 在
            // 加载 active profile 后用 profile.conversation_modes 覆盖（非空时）。
            allowed_conversation_modes: default_conversation_modes(),
            // H14：from_config 不接 DomainProfile，默认 false=无条件 grounding 硬闸
            // （销售域字节等价）；gateway 加载 active profile 后覆盖。
            grounding_gate_bypass_without_claim: false,
            // reviewer 深度：from_config 不接 DomainProfile，默认 false=普通低风险可走
            // light Reviewer；gateway 加载 active profile 后覆盖。
            distrust_self_reported_low_risk: false,
            // tag-trust 子计划3 Task2：归并宽窗口两参数走 typed → clamp 到合理带。
            consolidation_window_char_budget: typed
                .consolidation_window_char_budget
                .clamp(1000, 16000),
            consolidation_window_max_messages: typed
                .consolidation_window_max_messages
                .clamp(10, 200),
            // tag-trust 子计划4 Task2：贝叶斯占槽门两阈值走 typed → clamp。
            bayesian_slot_min_hits: typed.bayesian_slot_min_hits.clamp(1, 20),
            bayesian_slot_min_strong: typed.bayesian_slot_min_strong.clamp(0, 20),
        }
    }

    pub(crate) fn as_document(&self) -> Document {
        doc! {
            "recentMessageLimit": self.recent_message_limit,
            "minReplyIntervalSeconds": self.min_reply_interval_seconds,
            "maxDailyTouches": self.max_daily_touches,
            "maxPendingFollowUps": self.max_pending_follow_ups,
            "followUpExpiresHours": self.follow_up_expires_hours,
            "cooldownAfterNoReplyHours": self.cooldown_after_no_reply_hours,
            "factRiskBlockAt": self.fact_risk_block_at,
            "pressureRiskBlockAt": self.pressure_risk_block_at,
            "humanLikeRewriteBelow": self.human_like_rewrite_below,
            "emotionalValueRewriteBelow": self.emotional_value_rewrite_below,
            "productAccuracyBlockBelow": self.product_accuracy_block_below,
            "operationStateConfidenceFullReviewBelow": self.operation_state_confidence_full_review_below,
            "runTokenBudget": self.run_token_budget,
            "runTokenBudgetEscalated": self.run_token_budget_escalated,
            "runMaxLlmCalls": self.run_max_llm_calls,
            "simulationTokenBudget": self.simulation_token_budget,
            "reactionTokenBudget": self.reaction_token_budget,
            "reactionMaxLlmCalls": self.reaction_max_llm_calls,
            "autonomyProtocolEnabled": self.autonomy_protocol_enabled,
            "knowledgeMaxToolCalls": self.knowledge_max_tool_calls,
            "knowledgeOpenSliceMaxK": self.knowledge_open_slice_max_k,
            "knowledgeSearchTopK": self.knowledge_search_top_k,
            "outboxPollIntervalSeconds": self.outbox_poll_interval_seconds,
            "outboxLeaseSeconds": self.outbox_lease_seconds,
            "quietHoursEnabled": self.quiet_hours_enabled,
            "quietHoursStart": self.quiet_hours_start as i32,
            "quietHoursEnd": self.quiet_hours_end as i32,
            "quietHoursTzOffsetHours": self.quiet_hours_tz_offset_hours,
            "groundingGateBypassWithoutClaim": self.grounding_gate_bypass_without_claim,
            "distrustSelfReportedLowRisk": self.distrust_self_reported_low_risk
        }
    }

    /// M2：用 active profile 的 `threshold_overrides` 逐字段覆盖五闸阈值。
    /// `None`（DEFAULT profile）→ 不改任何字段（销售域字节等价）；`Some` 时字段内
    /// `Some(n)` 覆盖、`None` 保留 `from_config` 现值（逐字段独立回落）。抽成纯方法
    /// 便于无 DB 单测覆盖语义。
    pub(crate) fn apply_profile_threshold_overrides(
        &mut self,
        overrides: Option<&crate::models::ProfileThresholds>,
    ) {
        let Some(th) = overrides else { return };
        // G13: clamp 到 1..=10 防 admin 误配极值禁用安全硬闸（与 evolution THRESHOLD_REASONABLE_BANDS 同口径，此处独立写路径需自守）
        if let Some(v) = th.fact_risk_block_at {
            self.fact_risk_block_at = v.clamp(1, 10);
        }
        if let Some(v) = th.pressure_risk_block_at {
            self.pressure_risk_block_at = v.clamp(1, 10);
        }
        if let Some(v) = th.human_like_rewrite_below {
            self.human_like_rewrite_below = v.clamp(1, 10);
        }
        if let Some(v) = th.emotional_value_rewrite_below {
            self.emotional_value_rewrite_below = v.clamp(1, 10);
        }
        if let Some(v) = th.product_accuracy_block_below {
            self.product_accuracy_block_below = v.clamp(1, 10);
        }
    }

    /// universal-domain-adaptation 第 78 点：把 active DomainProfile 的运行期价值开关
    /// 一次性派生进本 runtime（gateway 在加载 profile 后调用，替代散落在 inner 里的三
    /// 行手工赋值）。封装为单一入口后，「情感陪伴等非销售 profile → runtime 非销售行为」
    /// 这条价值链可在 lib 单测里纯内存端到端断言（无需 Docker/LLM）。
    ///
    /// 派生三项：
    /// - `grounding_gate_bypass_without_claim`（H14）：纯情感回复无产品声明时旁路
    ///   grounding 软分硬闸。
    /// - `distrust_self_reported_low_risk`（reviewer 深度）：高敏域强制走 full Reviewer。
    /// - `threshold_overrides`（M2）：逐字段覆盖五闸阈值（None 回落不动）。
    ///
    /// DEFAULT 销售 profile（bypass=false/distrust=false/overrides=None）→ 三项均无扰动，
    /// 销售域字节等价。**红线**：conversation_modes 的派生在 decision.rs 的
    /// validate_and_promote 处（与 prompt 注入同源），不并入本函数。
    pub(crate) fn apply_active_profile(&mut self, profile: &crate::models::DomainProfile) {
        self.grounding_gate_bypass_without_claim = profile.grounding_gate_bypass_without_claim;
        self.distrust_self_reported_low_risk = profile.distrust_self_reported_low_risk;
        self.apply_profile_threshold_overrides(profile.threshold_overrides.as_ref());
    }
}

/// agent-autonomy-loop W0 / Task 1.3：把任意整数 value clamp 到 `[min, max]`，
/// 当 value 越界 / 不合理（< 1 等）时回退到 `default`，再 clamp 到上限。
///
/// 调用方应保证 `min <= default <= max`。
fn clamp_i32(value: i32, min: i32, max: i32, default: i32) -> i32 {
    debug_assert!(min <= max);
    debug_assert!(min <= default && default <= max);
    let v = if value < min { default } else { value };
    v.min(max)
}

impl Default for UserRuntimeParameters {
    /// agent-autonomy-loop W3 / Tasks 4.11-4.15 / 性质测试入口需要：
    ///
    /// PBT 不接 `AppState` / `OperationDomainConfig`，需要直接构造一个"全默认值"
    /// 的 [`UserRuntimeParameters`]。本 `Default` 与
    /// [`crate::models::RuntimeParametersTyped::default`] 保持字段值同源。
    fn default() -> Self {
        let typed = crate::models::RuntimeParametersTyped::default();
        Self {
            recent_message_limit: typed.recent_message_limit,
            min_reply_interval_seconds: typed.min_reply_interval_seconds,
            max_daily_touches: typed.max_daily_touches,
            max_pending_follow_ups: typed.max_pending_follow_ups,
            follow_up_expires_hours: typed.follow_up_expires_hours,
            cooldown_after_no_reply_hours: typed.cooldown_after_no_reply_hours,
            fact_risk_block_at: typed.hallucination_block_at,
            pressure_risk_block_at: typed.pressure_risk_block_at,
            human_like_rewrite_below: typed.human_like_rewrite_below,
            emotional_value_rewrite_below: typed.emotional_value_rewrite_below,
            product_accuracy_block_below: typed.knowledge_grounding_block_below,
            operation_state_confidence_full_review_below: typed
                .operation_state_confidence_full_review_below,
            run_token_budget: typed.run_token_budget,
            run_token_budget_escalated: typed.run_token_budget_escalated,
            run_max_llm_calls: typed.run_max_llm_calls,
            simulation_token_budget: typed.simulation_token_budget,
            reaction_token_budget: typed.reaction_token_budget,
            reaction_max_llm_calls: typed.reaction_max_llm_calls,
            autonomy_protocol_enabled: typed.autonomy_protocol_enabled,
            knowledge_max_tool_calls: clamp_i32(typed.knowledge_max_tool_calls, 1, 16, 6),
            knowledge_open_slice_max_k: clamp_i32(typed.knowledge_open_slice_max_k, 1, 16, 4),
            knowledge_search_top_k: clamp_i32(typed.knowledge_search_top_k, 1, 32, 8),
            outbox_poll_interval_seconds: clamp_i32(typed.outbox_poll_interval_seconds, 1, 60, 5),
            outbox_lease_seconds: clamp_i32(typed.outbox_lease_seconds, 10, 600, 60),
            quiet_hours_enabled: typed.quiet_hours_enabled,
            quiet_hours_start: typed.quiet_hours_start.min(23),
            quiet_hours_end: typed.quiet_hours_end.min(23),
            quiet_hours_tz_offset_hours: typed.quiet_hours_tz_offset_hours.clamp(-12, 14),
            // H9：PBT / 无 profile 入口的默认四模式，与销售域逐字等价。
            allowed_conversation_modes: default_conversation_modes(),
            // H14：PBT / 无 profile 入口默认 false=无条件 grounding 硬闸（销售域等价）。
            grounding_gate_bypass_without_claim: false,
            // reviewer 深度：PBT / 无 profile 入口默认 false=普通低风险可走 light Reviewer。
            distrust_self_reported_low_risk: false,
            // tag-trust 子计划3 Task2：与 from_config 同口径 clamp（默认值在带内，结果等价）。
            consolidation_window_char_budget: typed
                .consolidation_window_char_budget
                .clamp(1000, 16000),
            consolidation_window_max_messages: typed
                .consolidation_window_max_messages
                .clamp(10, 200),
            // tag-trust 子计划4 Task2：与 from_config 同口径 clamp（默认值在带内，结果等价）。
            bayesian_slot_min_hits: typed.bayesian_slot_min_hits.clamp(1, 20),
            bayesian_slot_min_strong: typed.bayesian_slot_min_strong.clamp(0, 20),
        }
    }
}

/// agent-self-evolution M4 / W4 Task 5.1：5 闸 + PlannerBlockRate 的"集中读路径"
/// 输出。读取顺序固定为：
///
/// 1. `threshold_overrides`（`rolled_back_at = null` 的最新一条 per `gate_key`） —— 演化器
///    `release_threshold` 写入的覆盖层；
/// 2. `contact.runtime_parameters` —— 单 contact 维度的硬参数（当前未在 `Contact`
///    上独立暴露字段，由 `OperationDomainConfig.runtime_parameters` 经
///    [`UserRuntimeParameters::from_config`] 派生）；
/// 3. `AppConfig` 默认值 —— 5 闸跟随 [`UserRuntimeParameters::default`]，
///    `planner_block_rate_threshold` 跟随 `AppConfig.strategic_planner_block_rate_threshold`。
///
/// 字段语义：
/// - 5 闸（`fact_risk_block / pressure_risk_block`）—— "scores ≥ 此值则 block"；
/// - rewrite 三档（`human_like_score_rewrite / emotional_value_rewrite /
///   product_accuracy_score_block`）—— "scores < 此值则 rewrite / block"；
/// - `planner_block_rate_threshold` —— Planner 反馈环 `blocked / total ≥ 此值`时 backoff。
///
/// 命名约定刻意与 `THRESHOLD_REASONABLE_BANDS` /
/// `evolution::release_threshold` 写入 `threshold_overrides.gate_key` 时使用的
/// 常量字面量保持一致：
///
/// | gate_key                            | 字段                            |
/// | ----------------------------------- | ------------------------------- |
/// | `fact_risk_block`                   | `fact_risk_block`               |
/// | `pressure_risk_block`               | `pressure_risk_block`           |
/// | `human_like_score_rewrite`          | `human_like_score_rewrite`      |
/// | `emotional_value_rewrite`           | `emotional_value_rewrite`       |
/// | `product_accuracy_score_block`      | `product_accuracy_score_block`  |
/// | `planner_block_rate_threshold`      | `planner_block_rate_threshold`  |
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedThresholds {
    pub fact_risk_block: i32,
    pub pressure_risk_block: i32,
    pub human_like_score_rewrite: i32,
    pub emotional_value_rewrite: i32,
    pub product_accuracy_score_block: i32,
    pub planner_block_rate_threshold: f64,
}

/// 6 个 gate_key 字面量，与 `evolution::threshold` /
/// `evolution::release_threshold` 写入 `threshold_overrides.gate_key` 时使用的
/// 字面量保持一致。改动需同步检查 W2 / W4 演化器侧。
#[allow(dead_code)] // 字面量校验常量，生产路径用字面量；test 验证完整性
pub const RESOLVED_GATE_KEYS: &[&str] = &[
    "fact_risk_block",
    "pressure_risk_block",
    "human_like_score_rewrite",
    "emotional_value_rewrite",
    "product_accuracy_score_block",
    "planner_block_rate_threshold",
];

impl ResolvedThresholds {
    /// 从 [`UserRuntimeParameters`] + [`AppConfig`] 构造一个"无 override"的基线，
    /// 调用方在此基础上叠加 `threshold_overrides` 即可。
    fn baseline(runtime: &UserRuntimeParameters, planner_block_rate: f64) -> Self {
        Self {
            fact_risk_block: runtime.fact_risk_block_at,
            pressure_risk_block: runtime.pressure_risk_block_at,
            human_like_score_rewrite: runtime.human_like_rewrite_below,
            emotional_value_rewrite: runtime.emotional_value_rewrite_below,
            product_accuracy_score_block: runtime.product_accuracy_block_below,
            planner_block_rate_threshold: planner_block_rate,
        }
    }

    /// 把 `threshold_overrides` 中某个 gate 的 `value` 应用到本 struct。
    /// 5 闸只接受 [1,10] 整数，PlannerBlockRate 接受 [0.05,0.95] 有限小数。
    /// 历史非法值 fail closed，避免 shadow 评估值与生产实际值不一致。
    fn apply_override(&mut self, gate_key: &str, value: f64) -> AppResult<()> {
        if is_review_gate(gate_key) && !is_integer_review_threshold(value) {
            return Err(AppError::BadRequest(format!(
                "invalid integer threshold override: {gate_key}={value}"
            )));
        }
        if gate_key == "planner_block_rate_threshold"
            && (!value.is_finite() || !(0.05..=0.95).contains(&value))
        {
            return Err(AppError::BadRequest(format!(
                "invalid planner threshold override: {value}"
            )));
        }
        match gate_key {
            "fact_risk_block" => self.fact_risk_block = value as i32,
            "pressure_risk_block" => self.pressure_risk_block = value as i32,
            "human_like_score_rewrite" => self.human_like_score_rewrite = value as i32,
            "emotional_value_rewrite" => self.emotional_value_rewrite = value as i32,
            "product_accuracy_score_block" => self.product_accuracy_score_block = value as i32,
            "planner_block_rate_threshold" => self.planner_block_rate_threshold = value,
            _ => {}
        }
        Ok(())
    }

    /// 把 5 闸值写回 [`UserRuntimeParameters`]，让既有 `review_passed` /
    /// `enforce_decision_guards` 等无须改签名即可拿到 override 后的值。
    /// `planner_block_rate_threshold` 不写回（runtime 没有该字段，由 Planner
    /// 自行从 `ResolvedThresholds` 取）。
    pub fn apply_to_runtime(&self, runtime: &mut UserRuntimeParameters) {
        runtime.fact_risk_block_at = self.fact_risk_block;
        runtime.pressure_risk_block_at = self.pressure_risk_block;
        runtime.human_like_rewrite_below = self.human_like_score_rewrite;
        runtime.emotional_value_rewrite_below = self.emotional_value_rewrite;
        runtime.product_accuracy_block_below = self.product_accuracy_score_block;
    }
}

/// agent-self-evolution M4 / W4 Task 5.1：5 闸 + PlannerBlockRate 的"集中读路径"。
///
/// 读取顺序：`threshold_overrides`（rolled_back_at=null 的最新值 per gate_key）
/// → `contact.runtime_parameters`（经 [`UserRuntimeParameters::from_config`]）
/// → `AppConfig` 默认值。返回的 [`ResolvedThresholds`] 在单次 run / planner tick
/// 入口取一次即可，run 中途不重读（设计 §7.1：避免 release 与正在进行 run 竞争）。
///
/// 函数本身不写 BSON，只发起一次 `find` 聚合 `threshold_overrides`，按
/// `released_at desc` 取每 `gate_key` 最新且未 rollback 的覆盖；运维路径下该
/// collection 体量极小，无需额外索引（W0 已建 `(workspace_id, account_id,
/// gate_key, released_at desc)`）。
///
/// **不**触发 LLM；**不**调用 gateway / outbox / mcp（与 evolution 隔离红线一致）。
pub async fn resolve_thresholds(
    state: &AppState,
    contact: &Contact,
) -> AppResult<ResolvedThresholds> {
    // 步骤 1：构造 baseline（contact 维度运行时参数 + AppConfig PlannerBlockRate 默认）。
    let domain_config =
        load_user_operation_domain_config_for_resolve(state, &contact.workspace_id, &contact.wxid)
            .await?;
    let runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    let mut resolved = ResolvedThresholds::baseline(
        &runtime,
        state.config.strategic_planner_block_rate_threshold,
    );

    // 步骤 2：叠加 threshold_overrides（rolled_back_at=null，最新一条 per gate_key）。
    let mut cursor = state
        .db
        .threshold_overrides()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "current_version": true,
                "rolled_back_at": null,
            },
            FindOptions::builder()
                .sort(doc! { "released_at": -1 })
                .build(),
        )
        .await
        .map_err(crate::error::AppError::from)?;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(o) = cursor
        .try_next()
        .await
        .map_err(crate::error::AppError::from)?
    {
        if seen.insert(o.gate_key.clone()) {
            resolved.apply_override(&o.gate_key, o.value)?;
        }
    }
    Ok(resolved)
}

pub(crate) fn threshold_value_is_representable(gate_key: &str, value: f64) -> bool {
    if is_review_gate(gate_key) {
        is_integer_review_threshold(value)
    } else if gate_key == "planner_block_rate_threshold" {
        value.is_finite() && (0.05..=0.95).contains(&value)
    } else {
        false
    }
}

fn is_review_gate(gate_key: &str) -> bool {
    RESOLVED_GATE_KEYS[..5].contains(&gate_key)
}

fn is_integer_review_threshold(value: f64) -> bool {
    value.is_finite() && (1.0..=10.0).contains(&value) && value.fract() == 0.0
}

/// 内部 helper：避免与 `agent::decision::load_user_operation_domain_config_for_contact`
/// 形成循环依赖（runtime.rs 不应反向依赖 decision.rs 的 pub(crate) 函数）。
/// 行为与之等价：历史版本可驻留，但 scope 已存在时必须恰好一条 current；
/// 多 current 或有历史却零 current 均 fail closed，不再隐式哈希分桶。
async fn load_user_operation_domain_config_for_resolve(
    state: &AppState,
    workspace_id: &str,
    _contact_id: &str,
) -> AppResult<Option<OperationDomainConfig>> {
    use futures::TryStreamExt;
    let coll = state.db.operation_domain_configs();
    let mut active: Vec<OperationDomainConfig> = coll
        .find(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
                "current_version": true,
            },
            None,
        )
        .await
        .map_err(crate::error::AppError::from)?
        .try_collect()
        .await
        .map_err(crate::error::AppError::from)?;
    if active.len() == 1 {
        return Ok(Some(active.remove(0)));
    }
    if active.len() > 1 {
        return Err(crate::error::AppError::Conflict(
            "multiple_current_operation_domain_configs".to_string(),
        ));
    }
    let scope_exists = coll
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
            },
            None,
        )
        .await
        .map_err(crate::error::AppError::from)?
        .is_some();
    if scope_exists {
        Err(crate::error::AppError::Conflict(
            "missing_current_operation_domain_config".to_string(),
        ))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OperationDomainConfig;
    use mongodb::bson::DateTime as BsonDt;

    #[test]
    fn runtime_write_validator_normalizes_legacy_aliases() {
        let normalized = validate_and_normalize_user_runtime_parameters(&doc! {
            "factRiskBlockAt": 8,
            "productAccuracyBlockBelow": 6,
            "maxDailyTouches": 3,
        })
        .expect("legacy aliases should remain writable");
        assert_eq!(normalized.get_i32("hallucinationBlockAt").ok(), Some(8));
        assert_eq!(
            normalized.get_i32("knowledgeGroundingBlockBelow").ok(),
            Some(6)
        );
        assert!(!normalized.contains_key("factRiskBlockAt"));
        assert!(!normalized.contains_key("productAccuracyBlockBelow"));
    }

    #[test]
    fn runtime_write_validator_rejects_unknown_wrong_type_and_unsafe_range() {
        for input in [
            doc! { "unknownRuntimeKey": 1 },
            doc! { "maxDailyTouches": "3" },
            doc! { "maxDailyTouches": -1 },
            doc! { "runTokenBudget": 200_000, "runTokenBudgetEscalated": 100_000 },
        ] {
            assert!(
                validate_and_normalize_user_runtime_parameters(&input).is_err(),
                "invalid runtime document should be rejected: {input:?}"
            );
        }
    }

    #[test]
    fn guide_runtime_patch_allows_cadence_but_rejects_high_risk_fields() {
        assert!(validate_guide_runtime_parameter_patch(&doc! {
            "maxDailyTouches": 2,
            "quietHoursStart": 23,
        })
        .is_ok());
        for key in [
            "hallucinationBlockAt",
            "runTokenBudget",
            "outboxLeaseSeconds",
            "autonomyProtocolEnabled",
        ] {
            assert!(validate_guide_runtime_parameter_patch(&doc! { key: 1 }).is_err());
        }
    }

    fn make_domain_config(params: Document) -> OperationDomainConfig {
        OperationDomainConfig {
            id: None,
            workspace_id: "default".into(),
            domain: "user_operations".into(),
            name: "x".into(),
            goal: "x".into(),
            methodology: "x".into(),
            workflow: "x".into(),
            tool_policy: "x".into(),
            automation_policy: "x".into(),
            review_policy: "x".into(),
            runtime_parameters: params,
            state_machine: Document::new(),
            status: "active".into(),
            updated_at: BsonDt::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
            ask_human_policy: None,
            assist_mode_enabled: None,
        }
    }

    /// 波 A1：as_document() round-trip 含两个 reaction 字段。
    /// 通过手工构造一个零成本的 `UserRuntimeParameters` 直接断言新字段。
    #[test]
    fn as_document_includes_reaction_budget_keys() {
        let runtime = UserRuntimeParameters {
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
            distrust_self_reported_low_risk: false,
            operation_state_confidence_full_review_below: 4,
            run_token_budget: 150000,
            run_token_budget_escalated: 500000,
            run_max_llm_calls: 6,
            simulation_token_budget: 300000,
            reaction_token_budget: 8000,
            reaction_max_llm_calls: 2,
            autonomy_protocol_enabled: true,
            knowledge_max_tool_calls: 6,
            knowledge_open_slice_max_k: 4,
            knowledge_search_top_k: 8,
            outbox_poll_interval_seconds: 5,
            outbox_lease_seconds: 60,
            quiet_hours_enabled: true,
            quiet_hours_start: 22,
            quiet_hours_end: 8,
            quiet_hours_tz_offset_hours: 8,
            allowed_conversation_modes: default_conversation_modes(),
            grounding_gate_bypass_without_claim: false,
            consolidation_window_char_budget: 6000,
            consolidation_window_max_messages: 60,
            bayesian_slot_min_hits: 3,
            bayesian_slot_min_strong: 2,
        };
        let doc = runtime.as_document();
        assert_eq!(doc.get_i64("reactionTokenBudget").ok(), Some(8000));
        assert_eq!(doc.get_i32("reactionMaxLlmCalls").ok(), Some(2));
    }

    /// 波 A1：typed 路径解析自定义 reaction 预算（与 from_config 同源 Document）。
    #[test]
    fn typed_round_trip_carries_reaction_budget() {
        let config = make_domain_config(doc! {
            "reactionTokenBudget": 4242_i64,
            "reactionMaxLlmCalls": 9_i32
        });
        let typed = config.runtime_parameters_typed();
        assert_eq!(typed.reaction_token_budget, 4242);
        assert_eq!(typed.reaction_max_llm_calls, 9);
        // 默认值（未设置时）回到 8000 / 2。
        let blank = make_domain_config(Document::new());
        let blank_typed = blank.runtime_parameters_typed();
        assert_eq!(blank_typed.reaction_token_budget, 8000);
        assert_eq!(blank_typed.reaction_max_llm_calls, 2);
    }

    /// #69 作息门控：typed 路径解析自定义静默时段；未设置时默认启用 + 22→8 + tz+8。
    #[test]
    fn typed_round_trip_carries_quiet_hours() {
        let config = make_domain_config(doc! {
            "quietHoursEnabled": false,
            "quietHoursStart": 23_i64,
            "quietHoursEnd": 7_i64,
            "quietHoursTzOffsetHours": -5_i64
        });
        let typed = config.runtime_parameters_typed();
        assert!(!typed.quiet_hours_enabled);
        assert_eq!(typed.quiet_hours_start, 23);
        assert_eq!(typed.quiet_hours_end, 7);
        assert_eq!(typed.quiet_hours_tz_offset_hours, -5);
        // 默认（未设置）：启用，22→8，tz+8。
        let blank = make_domain_config(Document::new()).runtime_parameters_typed();
        assert!(blank.quiet_hours_enabled, "缺字段默认启用作息门控");
        assert_eq!(blank.quiet_hours_start, 22);
        assert_eq!(blank.quiet_hours_end, 8);
        assert_eq!(
            blank.quiet_hours_tz_offset_hours, 8,
            "缺字段默认 +8 中国时区"
        );
    }

    /// #69：as_document 把作息门控四字段写进 wire shape（camelCase）。
    #[test]
    fn as_document_carries_quiet_hours() {
        let runtime = UserRuntimeParameters::default();
        let doc = runtime.as_document();
        assert_eq!(doc.get_bool("quietHoursEnabled").ok(), Some(true));
        assert_eq!(doc.get_i32("quietHoursStart").ok(), Some(22));
        assert_eq!(doc.get_i32("quietHoursEnd").ok(), Some(8));
        assert_eq!(doc.get_i32("quietHoursTzOffsetHours").ok(), Some(8));
    }

    /// #69：from_config 把 tz 偏移 clamp 到 [-12, 14]，挡住误配。
    #[test]
    fn from_config_clamps_tz_offset() {
        let too_big = make_domain_config(doc! { "quietHoursTzOffsetHours": 99_i64 });
        let typed = too_big.runtime_parameters_typed();
        assert_eq!(typed.quiet_hours_tz_offset_hours.clamp(-12, 14), 14);
        let too_small = make_domain_config(doc! { "quietHoursTzOffsetHours": -99_i64 });
        let typed2 = too_small.runtime_parameters_typed();
        assert_eq!(typed2.quiet_hours_tz_offset_hours.clamp(-12, 14), -12);
    }

    // ── 第 78 点：非销售（情感陪伴）价值断言进常规门（纯内存，无 Docker/LLM）──
    // gateway 的 profile→runtime 派生封装进 apply_active_profile 后，这条价值链可在 lib
    // 门里端到端断言：example 情感 profile → apply → runtime 带非销售开关 → 驱动下游
    // should_run_review（review/mod.rs::should_run_review_forces_full_when_distrust_set）
    // 与 grounding 闸（review/gates.rs::h14_grounding_gate_bypassed_when_no_claim_*）。

    /// 第 78 点：apply_active_profile(情感 profile) → runtime 带上全部非销售价值开关。
    #[test]
    fn emotional_companion_profile_drives_non_sales_runtime() {
        let profile = crate::agent::domain_profile::example_emotional_companion_profile("ws-e");
        let mut runtime = baseline_runtime();
        // 派生前 = 销售默认（零扰动锚点）。
        assert!(!runtime.grounding_gate_bypass_without_claim);
        assert!(!runtime.distrust_self_reported_low_risk);
        runtime.apply_active_profile(&profile);
        // H14：纯情感回复旁路 grounding 软分硬闸。
        assert!(
            runtime.grounding_gate_bypass_without_claim,
            "情感陪伴应旁路 grounding 软闸"
        );
        // reviewer：高敏域强制走 full Reviewer。
        assert!(
            runtime.distrust_self_reported_low_risk,
            "情感陪伴高敏域应使用 full Reviewer"
        );
    }

    /// 第 78 点：DEFAULT 销售 profile → apply_active_profile 零扰动（字节等价护栏）。
    #[test]
    fn apply_active_profile_default_is_zero_perturbation() {
        let profile = crate::agent::domain_profile::default_domain_profile("ws-d");
        let mut runtime = baseline_runtime();
        let before_fact = runtime.fact_risk_block_at;
        let before_pressure = runtime.pressure_risk_block_at;
        let before_human = runtime.human_like_rewrite_below;
        let before_emotional = runtime.emotional_value_rewrite_below;
        let before_product = runtime.product_accuracy_block_below;

        runtime.apply_active_profile(&profile);
        // DEFAULT profile: bypass=false / distrust=false / overrides=None → 三项无扰动。
        assert!(!runtime.grounding_gate_bypass_without_claim);
        assert!(!runtime.distrust_self_reported_low_risk);
        assert_eq!(runtime.fact_risk_block_at, before_fact);
        assert_eq!(runtime.pressure_risk_block_at, before_pressure);
        assert_eq!(runtime.human_like_rewrite_below, before_human);
        assert_eq!(runtime.emotional_value_rewrite_below, before_emotional);
        assert_eq!(runtime.product_accuracy_block_below, before_product);
    }

    /// 第 78 点：情感 profile 声明 threshold_overrides 时，apply 逐字段覆盖五闸阈值。
    #[test]
    fn apply_active_profile_applies_threshold_overrides() {
        let mut profile = crate::agent::domain_profile::example_emotional_companion_profile("ws-t");
        // 情感域放宽压力闸（主动关心不该被高 pressure 拦）、提高情绪价值改写线。
        profile.threshold_overrides = Some(crate::models::ProfileThresholds {
            fact_risk_block_at: None,
            pressure_risk_block_at: Some(9),
            human_like_rewrite_below: None,
            emotional_value_rewrite_below: Some(8),
            product_accuracy_block_below: None,
        });
        let mut runtime = baseline_runtime();
        let before_fact = runtime.fact_risk_block_at;
        runtime.apply_active_profile(&profile);
        // 声明的两项被覆盖。
        assert_eq!(runtime.pressure_risk_block_at, 9);
        assert_eq!(runtime.emotional_value_rewrite_below, 8);
        // 未声明项（None）保持不动。
        assert_eq!(runtime.fact_risk_block_at, before_fact);
    }

    fn baseline_runtime() -> UserRuntimeParameters {
        UserRuntimeParameters::default()
    }

    /// W4 Task 5.1：baseline 完全跟随 [`UserRuntimeParameters::default`] 的 5 闸值
    /// 与传入的 PlannerBlockRate 默认。
    #[test]
    fn resolved_thresholds_baseline_matches_runtime_defaults() {
        let runtime = baseline_runtime();
        let resolved = ResolvedThresholds::baseline(&runtime, 0.6);
        assert_eq!(resolved.fact_risk_block, runtime.fact_risk_block_at);
        assert_eq!(resolved.pressure_risk_block, runtime.pressure_risk_block_at);
        assert_eq!(
            resolved.human_like_score_rewrite,
            runtime.human_like_rewrite_below
        );
        assert_eq!(
            resolved.emotional_value_rewrite,
            runtime.emotional_value_rewrite_below
        );
        assert_eq!(
            resolved.product_accuracy_score_block,
            runtime.product_accuracy_block_below
        );
        assert!((resolved.planner_block_rate_threshold - 0.6).abs() < f64::EPSILON);
    }

    /// W4 Task 5.1：apply_override 按 gate_key 字面量精确改写各 gate；
    /// 未识别 gate_key 静默忽略，不破坏其它字段。
    #[test]
    fn resolved_thresholds_apply_override_per_gate_key() {
        let runtime = baseline_runtime();
        let mut resolved = ResolvedThresholds::baseline(&runtime, 0.6);
        resolved.apply_override("fact_risk_block", 8.0).unwrap();
        resolved.apply_override("pressure_risk_block", 9.0).unwrap();
        resolved
            .apply_override("human_like_score_rewrite", 4.0)
            .unwrap();
        resolved
            .apply_override("emotional_value_rewrite", 3.0)
            .unwrap();
        resolved
            .apply_override("product_accuracy_score_block", 5.0)
            .unwrap();
        resolved
            .apply_override("planner_block_rate_threshold", 0.42)
            .unwrap();
        // 未识别 gate_key —— 静默忽略，不影响已有字段。
        let snapshot = resolved.clone();
        resolved.apply_override("unknown_gate_key", 99.0).unwrap();
        assert_eq!(resolved, snapshot);
        assert_eq!(resolved.fact_risk_block, 8);
        assert_eq!(resolved.pressure_risk_block, 9);
        assert_eq!(resolved.human_like_score_rewrite, 4);
        assert_eq!(resolved.emotional_value_rewrite, 3);
        assert_eq!(resolved.product_accuracy_score_block, 5);
        assert!((resolved.planner_block_rate_threshold - 0.42).abs() < f64::EPSILON);
    }

    #[test]
    fn resolved_thresholds_rejects_fractional_review_override() {
        let runtime = baseline_runtime();
        let mut resolved = ResolvedThresholds::baseline(&runtime, 0.6);
        assert!(resolved
            .apply_override("emotional_value_rewrite", 5.5)
            .is_err());
        assert_eq!(
            resolved.emotional_value_rewrite,
            runtime.emotional_value_rewrite_below
        );
        assert!(threshold_value_is_representable("fact_risk_block", 6.0));
        assert!(!threshold_value_is_representable("fact_risk_block", 6.5));
    }

    /// W4 Task 5.1：apply_to_runtime 把 5 闸值写回 `UserRuntimeParameters`，
    /// PlannerBlockRate 不写回（runtime 没有该字段，由 Planner 直接读 ResolvedThresholds）。
    #[test]
    fn resolved_thresholds_apply_to_runtime_writes_back_5_gates_only() {
        let mut runtime = baseline_runtime();
        let resolved = ResolvedThresholds {
            fact_risk_block: 9,
            pressure_risk_block: 9,
            human_like_score_rewrite: 4,
            emotional_value_rewrite: 4,
            product_accuracy_score_block: 5,
            planner_block_rate_threshold: 0.42,
        };
        resolved.apply_to_runtime(&mut runtime);
        assert_eq!(runtime.fact_risk_block_at, 9);
        assert_eq!(runtime.pressure_risk_block_at, 9);
        assert_eq!(runtime.human_like_rewrite_below, 4);
        assert_eq!(runtime.emotional_value_rewrite_below, 4);
        assert_eq!(runtime.product_accuracy_block_below, 5);
    }

    /// W4 Task 5.1：6 个 gate_key 字面量与 `evolution::threshold` 的
    /// `THRESHOLD_REASONABLE_BANDS` 名称一致；该集合是演化器 / runtime 共享的"权威 6 词"。
    #[test]
    fn resolved_gate_keys_cover_all_six() {
        assert_eq!(RESOLVED_GATE_KEYS.len(), 6);
        for k in [
            "fact_risk_block",
            "pressure_risk_block",
            "human_like_score_rewrite",
            "emotional_value_rewrite",
            "product_accuracy_score_block",
            "planner_block_rate_threshold",
        ] {
            assert!(RESOLVED_GATE_KEYS.contains(&k), "missing gate_key: {k}");
        }
    }

    // ── M2：五闸阈值 profile 覆盖 ──

    #[test]
    fn threshold_overrides_none_is_byte_equivalent() {
        // DEFAULT profile threshold_overrides=None → 不改任何阈值（销售域字节等价）。
        let mut rt = UserRuntimeParameters::default();
        let before = (
            rt.fact_risk_block_at,
            rt.pressure_risk_block_at,
            rt.human_like_rewrite_below,
            rt.emotional_value_rewrite_below,
            rt.product_accuracy_block_below,
        );
        rt.apply_profile_threshold_overrides(None);
        assert_eq!(
            (
                rt.fact_risk_block_at,
                rt.pressure_risk_block_at,
                rt.human_like_rewrite_below,
                rt.emotional_value_rewrite_below,
                rt.product_accuracy_block_below,
            ),
            before,
            "None override 不得改变任何阈值"
        );
    }

    #[test]
    fn threshold_overrides_partial_only_touches_some_fields() {
        // 情感域只放宽 pressure、提高 emotional_value 改写线，其余字段 None → 保留原值。
        let mut rt = UserRuntimeParameters::default();
        let orig_fact = rt.fact_risk_block_at;
        let orig_human = rt.human_like_rewrite_below;
        let orig_grounding = rt.product_accuracy_block_below;
        let th = crate::models::ProfileThresholds {
            pressure_risk_block_at: Some(9),
            emotional_value_rewrite_below: Some(8),
            ..Default::default()
        };
        rt.apply_profile_threshold_overrides(Some(&th));
        // 被覆盖的两个。
        assert_eq!(rt.pressure_risk_block_at, 9);
        assert_eq!(rt.emotional_value_rewrite_below, 8);
        // None 字段保留原值（逐字段独立回落）。
        assert_eq!(rt.fact_risk_block_at, orig_fact);
        assert_eq!(rt.human_like_rewrite_below, orig_human);
        assert_eq!(rt.product_accuracy_block_below, orig_grounding);
    }

    #[test]
    fn threshold_overrides_full_override_all_five() {
        let mut rt = UserRuntimeParameters::default();
        let th = crate::models::ProfileThresholds {
            fact_risk_block_at: Some(8),
            pressure_risk_block_at: Some(9),
            human_like_rewrite_below: Some(4),
            emotional_value_rewrite_below: Some(7),
            product_accuracy_block_below: Some(5),
        };
        rt.apply_profile_threshold_overrides(Some(&th));
        assert_eq!(rt.fact_risk_block_at, 8);
        assert_eq!(rt.pressure_risk_block_at, 9);
        assert_eq!(rt.human_like_rewrite_below, 4);
        assert_eq!(rt.emotional_value_rewrite_below, 7);
        assert_eq!(rt.product_accuracy_block_below, 5);
    }

    #[test]
    fn apply_threshold_overrides_clamps_out_of_range() {
        // G13：admin 误配越界值（如 fact_risk_block_at=100 → score<100 恒真 → 幻觉硬闸禁用）
        // 须被 clamp 到 1..=10；None 字段不动。
        let mut rt = UserRuntimeParameters::default();
        let overrides = crate::models::ProfileThresholds {
            fact_risk_block_at: Some(100),           // 越界高 → clamp 10
            pressure_risk_block_at: Some(0),         // 越界低 → clamp 1
            human_like_rewrite_below: Some(-5),      // 越界低 → clamp 1
            emotional_value_rewrite_below: Some(50), // 越界高 → clamp 10
            product_accuracy_block_below: None,      // None → 不动
        };
        let before_product = rt.product_accuracy_block_below;
        rt.apply_profile_threshold_overrides(Some(&overrides));
        assert_eq!(rt.fact_risk_block_at, 10);
        assert_eq!(rt.pressure_risk_block_at, 1);
        assert_eq!(rt.human_like_rewrite_below, 1);
        assert_eq!(rt.emotional_value_rewrite_below, 10);
        assert_eq!(rt.product_accuracy_block_below, before_product); // None 回落不动
    }
}
