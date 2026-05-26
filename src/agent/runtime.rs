//! 运行时硬参数 (`UserRuntimeParameters`)。
//!
//! 把 `OperationDomainConfig.runtime_parameters` 这份 `Document`
//! 解析成一组强类型字段，给 gateway / decision / review / guards
//! 等子模块共享使用。字段命名与后台 UI、prompt 中暴露的 camelCase
//! key 一一对应。
//!
//! 同时提供 `as_document()` 方便回写到 prompt / agent_run_logs。

use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use mongodb::options::FindOptions;

use crate::error::AppResult;
use crate::models::{Contact, OperationDomainConfig};
use crate::routes::AppState;

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
    /// agent-autonomy-loop W0 / Task 1.3：知识路由模式，
    /// 仅允许 `auto_tool_loop`（默认）或 `classic_router`（灰度回退）。
    /// 非法 / 空字符串在 loader 中回退到 `auto_tool_loop`。sunset D+14。
    pub knowledge_routing_mode: String,
    /// agent-autonomy-loop W0 / Task 1.3：`reply_with_tools_loop` 的最大轮数。
    /// 默认 3，loader 中 clamp 到 `[1, 5]`。
    pub knowledge_max_tool_loops: i32,
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
            pressure_risk_block_at: 7,
            human_like_rewrite_below: typed.human_like_rewrite_below,
            emotional_value_rewrite_below: typed.emotional_value_rewrite_below,
            product_accuracy_block_below: typed.knowledge_grounding_block_below,
            operation_state_confidence_full_review_below: typed
                .operation_state_confidence_full_review_below,
            run_token_budget: typed.run_token_budget,
            run_max_llm_calls: typed.run_max_llm_calls,
            simulation_token_budget: typed.simulation_token_budget,
            reaction_token_budget: typed.reaction_token_budget,
            reaction_max_llm_calls: typed.reaction_max_llm_calls,
            autonomy_protocol_enabled: typed.autonomy_protocol_enabled,
            knowledge_routing_mode: clamp_knowledge_routing_mode(&typed.knowledge_routing_mode),
            knowledge_max_tool_loops: clamp_i32(typed.knowledge_max_tool_loops, 1, 5, 3),
            knowledge_max_tool_calls: clamp_i32(typed.knowledge_max_tool_calls, 1, 16, 6),
            knowledge_open_slice_max_k: clamp_i32(typed.knowledge_open_slice_max_k, 1, 16, 4),
            knowledge_search_top_k: clamp_i32(typed.knowledge_search_top_k, 1, 32, 8),
            outbox_poll_interval_seconds: clamp_i32(typed.outbox_poll_interval_seconds, 1, 60, 5),
            outbox_lease_seconds: clamp_i32(typed.outbox_lease_seconds, 10, 600, 60),
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
            "runMaxLlmCalls": self.run_max_llm_calls,
            "simulationTokenBudget": self.simulation_token_budget,
            "reactionTokenBudget": self.reaction_token_budget,
            "reactionMaxLlmCalls": self.reaction_max_llm_calls,
            "autonomyProtocolEnabled": self.autonomy_protocol_enabled,
            "knowledgeRoutingMode": self.knowledge_routing_mode.clone(),
            "knowledgeMaxToolLoops": self.knowledge_max_tool_loops,
            "knowledgeMaxToolCalls": self.knowledge_max_tool_calls,
            "knowledgeOpenSliceMaxK": self.knowledge_open_slice_max_k,
            "knowledgeSearchTopK": self.knowledge_search_top_k,
            "outboxPollIntervalSeconds": self.outbox_poll_interval_seconds,
            "outboxLeaseSeconds": self.outbox_lease_seconds
        }
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

/// agent-autonomy-loop W0 / Task 1.3：把 `knowledgeRoutingMode` 字符串 clamp
/// 到允许的集合 `{auto_tool_loop, classic_router}`，其它值（含空字符串）
/// 回退到默认 `auto_tool_loop`。
fn clamp_knowledge_routing_mode(raw: &str) -> String {
    match raw {
        "auto_tool_loop" | "classic_router" => raw.to_string(),
        _ => "auto_tool_loop".to_string(),
    }
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
            pressure_risk_block_at: 7,
            human_like_rewrite_below: typed.human_like_rewrite_below,
            emotional_value_rewrite_below: typed.emotional_value_rewrite_below,
            product_accuracy_block_below: typed.knowledge_grounding_block_below,
            operation_state_confidence_full_review_below: typed
                .operation_state_confidence_full_review_below,
            run_token_budget: typed.run_token_budget,
            run_max_llm_calls: typed.run_max_llm_calls,
            simulation_token_budget: typed.simulation_token_budget,
            reaction_token_budget: typed.reaction_token_budget,
            reaction_max_llm_calls: typed.reaction_max_llm_calls,
            autonomy_protocol_enabled: typed.autonomy_protocol_enabled,
            knowledge_routing_mode: clamp_knowledge_routing_mode(&typed.knowledge_routing_mode),
            knowledge_max_tool_loops: clamp_i32(typed.knowledge_max_tool_loops, 1, 5, 3),
            knowledge_max_tool_calls: clamp_i32(typed.knowledge_max_tool_calls, 1, 16, 6),
            knowledge_open_slice_max_k: clamp_i32(typed.knowledge_open_slice_max_k, 1, 16, 4),
            knowledge_search_top_k: clamp_i32(typed.knowledge_search_top_k, 1, 32, 8),
            outbox_poll_interval_seconds: clamp_i32(typed.outbox_poll_interval_seconds, 1, 60, 5),
            outbox_lease_seconds: clamp_i32(typed.outbox_lease_seconds, 10, 600, 60),
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
    /// 5 闸值整数化（向下取整），PlannerBlockRate 保持 f64。未识别 gate_key 静默忽略。
    fn apply_override(&mut self, gate_key: &str, value: f64) {
        match gate_key {
            "fact_risk_block" => self.fact_risk_block = value as i32,
            "pressure_risk_block" => self.pressure_risk_block = value as i32,
            "human_like_score_rewrite" => self.human_like_score_rewrite = value as i32,
            "emotional_value_rewrite" => self.emotional_value_rewrite = value as i32,
            "product_accuracy_score_block" => self.product_accuracy_score_block = value as i32,
            "planner_block_rate_threshold" => self.planner_block_rate_threshold = value,
            _ => {}
        }
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
    let domain_config = load_user_operation_domain_config_for_resolve(state, &contact.workspace_id)
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
            resolved.apply_override(&o.gate_key, o.value);
        }
    }
    Ok(resolved)
}

/// 内部 helper：避免与 `agent::decision::load_user_operation_domain_config`
/// 形成循环依赖（runtime.rs 不应反向依赖 decision.rs 的 pub(crate) 函数）。
/// 行为与之等价：按 `(workspace_id, domain="user_operations")` 取一条。
async fn load_user_operation_domain_config_for_resolve(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<Option<OperationDomainConfig>> {
    state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations",
            },
            None,
        )
        .await
        .map_err(crate::error::AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OperationDomainConfig;
    use mongodb::bson::DateTime as BsonDt;

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
            emotional_value_rewrite_below: 5,
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
        assert_eq!(resolved.human_like_score_rewrite, runtime.human_like_rewrite_below);
        assert_eq!(resolved.emotional_value_rewrite, runtime.emotional_value_rewrite_below);
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
        resolved.apply_override("fact_risk_block", 8.0);
        resolved.apply_override("pressure_risk_block", 9.0);
        resolved.apply_override("human_like_score_rewrite", 4.0);
        resolved.apply_override("emotional_value_rewrite", 3.0);
        resolved.apply_override("product_accuracy_score_block", 5.0);
        resolved.apply_override("planner_block_rate_threshold", 0.42);
        // 未识别 gate_key —— 静默忽略，不影响已有字段。
        let snapshot = resolved.clone();
        resolved.apply_override("unknown_gate_key", 99.0);
        assert_eq!(resolved, snapshot);
        assert_eq!(resolved.fact_risk_block, 8);
        assert_eq!(resolved.pressure_risk_block, 9);
        assert_eq!(resolved.human_like_score_rewrite, 4);
        assert_eq!(resolved.emotional_value_rewrite, 3);
        assert_eq!(resolved.product_accuracy_score_block, 5);
        assert!((resolved.planner_block_rate_threshold - 0.42).abs() < f64::EPSILON);
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
}
