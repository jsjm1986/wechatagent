//! 用户运营 Agent 顶层模块入口（LP-11 / Task 23 拆分目标）。
//!
//! 本模块原本是一个 5800+ 行的 `src/agent.rs`，在 LP-11 拆分后以
//! `src/agent/mod.rs` + 一组职责明确的子模块代替；行为完全不变，
//! 所有外部调用方（`webhooks` / `tasks` / `routes::*` / `main`）仍通过
//! `crate::agent::xxx` 访问入口函数与类型。
//!
//! 子模块职责：
//! - [`types`]：内部数据契约（[`AgentDecision`]、`DecisionReviewResult`、
//!   [`KnowledgeRouteResult`]、[`AgentTrigger`] 等）与序列化辅助；
//! - [`runtime`]：[`UserRuntimeParameters`] 强类型运行参数；
//! - [`budget`]：[`RunBudget`] task-local 预算计数 (MP-5)；
//! - [`guards`]：决策守卫（状态机 / 字符串级 fact-risk / 知识支撑校验）；
//! - [`memory`]：长期 memoryCard 整理与 consolidator (MP-8)；
//! - [`reaction`]：用户反应分析与 claim 锁 (HP-3)；
//! - [`knowledge_router`]：知识库加载、Knowledge Tool Planner、未验证告警 (MP-9)；
//! - [`decision`]：Reply Agent 主决策与初始画像生成；
//! - [`review`]：Review Agent / 本地评审 / mode 决策 (MP-10)；
//! - [`gateway`]：发送网关、运行链路编排、`run_user_operation_gateway`；
//! - [`simulation`]：Shadow 模式 `simulate_user_dialogue`。
//!
//! 工具函数 [`generate_agent_json`] 仍由 mod.rs 自身持有：因为它需要
//! 访问 [`budget`] 模块的 task-local，而被几乎所有子模块共用，放在 mod.rs
//! 既能避免循环依赖，也能让 LLM 调用计费/缓存/日志的所有逻辑位于一处。

mod budget;
mod chat_tool_loop;
mod decision;
mod decision_taxonomy;
mod entitlements;
pub mod domain;
pub(crate) mod domain_profile;
pub(crate) mod domain_signals;
pub mod escalation;
mod gateway;
mod guards;
pub mod knowledge_agent;
mod knowledge_router;
mod knowledge_tools;
mod memory;
pub(crate) mod prompt_isolation;
pub(crate) mod quiet_hours;
mod tool_loop;
pub(crate) mod outbox;
pub(crate) mod outbox_dispatcher;
mod reaction;
mod review;
pub(crate) mod runtime;
pub mod run_envelope;
mod simulation;
pub(crate) mod taxonomy;
pub(crate) mod types;

use std::{num::NonZeroUsize, sync::LazyLock};

use lru::LruCache;
use mongodb::bson::DateTime;
use parking_lot::Mutex as PlMutex;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::models::LlmCallLog;
use crate::routes::AppState;

pub use self::budget::RunBudget;
pub(crate) use self::budget::{current_run_budget, RUN_BUDGET};

// 入口函数 / 类型重新导出，保持与拆分前 `crate::agent::xxx` 完全一致。
pub use decision::{build_initial_operation_profile, load_operation_playbook_for_contact};
pub(crate) use decision::load_user_operation_domain_config_for_contact;
// H13：onboarding 写侧（routes/contacts、routes/management）取状态机初始态 key +
// 按 workspace 加载 active domain_config（替代写死 "new_contact"）。
pub(crate) use decision::load_user_operation_domain_config;
pub(crate) use decision::initial_operation_state_for_contact;
pub(crate) use guards::initial_operation_state_key;
pub use gateway::{
    handle_follow_up_task, handle_managed_message, handle_managed_message_aggregated,
    send_contact_message_gateway, write_event_for_account,
};
pub use knowledge_router::test_knowledge_route_for_contact;
// Agent-first 渐进式披露入口：`/api/knowledge/ask` 路由直接调用本 agent。
pub use knowledge_agent::{
    answer as knowledge_agent_answer, AnswerRequest as KnowledgeAnswerRequest,
    AnswerResult as KnowledgeAnswerResult, CatalogEntry as KnowledgeCatalogEntry,
    CatalogFilter as KnowledgeCatalogFilter, ChunkFull as KnowledgeChunkFull,
};
// Phase B / B3 / B6：`tests/chunk_type_routing_pbt.rs` 需要直接驱动
// `format_operation_knowledge_for_prompt`，因此对外 re-export。生产路径调用方
// 仍走 `agent/knowledge_router.rs` 内部，不应跨越 mod 边界使用此符号。
// H16-b：`_with_roles` 是生产路径入口（decision / review 传 active profile.chunk_roles），
// 无参 wrapper 保留供 PBT / 无 profile 入口 = DEFAULT 销售四态。
pub use knowledge_router::{
    format_operation_knowledge_for_prompt, format_operation_knowledge_for_prompt_with_roles,
};
pub use memory::{consolidate_contact_memory, handle_memory_consolidation_task};
pub use outbox_dispatcher::run_outbox_dispatcher;

// W4 / Task 5.8（R13.10）：暴露 dispatcher 内部 helper 给
// `tests/outbox_integration.rs` 集成测试驱动；不应在生产代码中绕过 `tick`
// / `process_entry` 直接调用这些 helper。
pub use outbox_dispatcher::{
    atomic_claim_pending, cancel_entry, process_entry, reclaim_expired_leases,
    schedule_retry_or_terminal, second_safety_gate,
};
// outbox 公共 API（enqueue + 取消通道 + 类型）的对外重导出，集成测试需要。
pub use outbox::{
    cancel_for_contact_on_user_reaction, enqueue, EnqueueOutcome, EnqueueRequest, OutboxStatus,
};
pub use reaction::{cap_intent_trajectory, record_user_reaction};
pub use simulation::simulate_user_dialogue;
pub use types::{
    AgentDecision, ContactSendResult, FollowUpDecision, GeneratedOperationProfile,
    KnowledgeRouteResult, ManualContactSend, RunPlannerResult, UserOperationSimulationTurn,
};

// 跨模块仍需访问的内部辅助（routes::shared 用 memory_card 相关函数）。
pub(crate) use memory::{effective_memory_card_for_contact, memory_card_has_signal};
// knowledge-digest-workstation Phase 5：知识库 chat 用的运营长期偏好记忆
// 读写入口；与 contact memory_card 物理隔离。
pub(crate) use memory::{load_operator_memory, record_operator_memory};

// knowledge-digest-workstation Phase 5：chat 多轮工具循环 + 7 个 chat 工具
// 派发器（含 4 个 chat-only async tool）。仅供 routes::knowledge 内部使用，
// 永不进 user-ops gateway。
pub(crate) use chat_tool_loop::{chat_reply_with_tools_loop, ChatReplyFn, ChatToolLoopError};
pub(crate) use knowledge_tools::{AnchorMatchFn, ALLOWED_CHAT_TOOL_NAMES};

// Task 24：测试可用的 PBT 入口（pure functions，无副作用）。
pub use guards::check_state_transition;
pub use memory::{compact_memory_card_with_dimensions, compact_memory_card_with_previous};

// agent-autonomy-loop W3 / Tasks 4.11-4.15：性质测试 P1-P7 入口。
//
// 这些函数原本是 `pub(crate)`（仅 crate 内部使用）。在 W3 阶段需要被
// `tests/autonomy_protocol_pbt.rs` 这个独立 crate 的测试文件直接调用，因此重
// 导出为 `pub`。语义不变，仅可见性变化。
pub use review::{finalize_review_for_send, local_decision_review, FinalizeOutcome, GatewayStatusFinal, PendingFinalizeEvent};
// Phase B / B6：把 `review_passed` 暴露到 crate 边界，让 PBT 文件
// (`tests/human_like_threshold_pbt.rs` / `tests/pressure_risk_threshold_pbt.rs`)
// 直接断言"双闸阈值穿越是否拦截"——契约性测试的最小暴露面。
pub use review::review_passed;
pub use runtime::UserRuntimeParameters;
pub use runtime::{resolve_thresholds, ResolvedThresholds};
pub use types::{DecisionReviewResult, RawAgentDecision, ReviewScores};

// agent-autonomy-loop W3 / Task 4.14：P4 PBT 已随销售域守卫一起删除（2026-05-25
// 知识库清理），ProductClaimMarkers / default_product_claim_markers 不再公开。

// agent-autonomy-loop W3 / Task 4.11：让 PBT 通过 `wechatagent::agent::taxonomy`
// 直接访问 cache 构造 helper 与命中分支枚举。
pub use taxonomy::{taxonomy_cache_for_tests, TaxonomyCache};

// Phase A / A3：启动期预热入口；main.rs 在 ensure_indexes 后调用。
pub use taxonomy::init_global_taxonomy_cache;

// universal-domain-adaptation 1G-c：active DomainProfile 进程级缓存预热入口；
// main.rs 在 taxonomy 预热之后调用。
pub use domain_profile::init_global_domain_profile_cache;
// H17：记忆维度通用化的两个 pub 渲染/seed 函数，供 tests/ 端到端验证情感 profile。
pub use domain_profile::{default_memory_dimensions, render_memory_candidate_types_guidance};
// roleplay-fuzz P0：集成测试 seed/读回 active DomainProfile + 失效进程级缓存所需入口。
pub use domain_profile::{
    default_domain_profile, invalidate_global_domain_profile_cache, load_active_domain_profile,
};

// agent-autonomy-loop W3 / Task 4.5：P7 工具循环性质测试入口。
//
// 直接对外暴露 `reply_with_tools_loop` + `ToolCallRequest` 涉及 `pub(crate)`
// 类型（`RunBudget` / `KnowledgeRuntime`），扩散面太大；本期采用"在 lib
// `cfg(test)` 中实现 P7"的折衷方案，与 P3/P4 走同一模式。
//
// 详见 `src/agent/tool_loop.rs::pbt_tests`。
// task 6.3：`compact_memory_card_typed` 已与 `compact_memory_card_with_previous`
// 合并，仅作向后兼容别名保留；外部测试仍引用，需要 `#[allow(deprecated)]`
// 静默重导出告警，使用方应迁移到 `compact_memory_card_with_previous`。
#[allow(deprecated)]
pub use memory::compact_memory_card_typed;

// 波 C4：销售域 product-claim 标记词缓存已随 guards 重写一并删除（2026-05-25
// 知识库清理），routes 调用方无需再失效缓存。

static LLM_EXACT_CACHE: LazyLock<PlMutex<LruCache<String, Value>>> = LazyLock::new(|| {
    PlMutex::new(LruCache::new(
        NonZeroUsize::new(256).expect("cache capacity must be non-zero"),
    ))
});

/// Agent 公共 LLM JSON 调用入口。所有子模块（decision / review /
/// knowledge_router / memory / reaction 等）都通过它进 LLM，统一处理：
/// - LRU 精确缓存（限定 prompt key 列表）；
/// - 写 `llm_call_logs`（success / cache_hit / failed / json_error）；
/// - 累计当前 run 的 token / 调用次数 (MP-5 task_local budget)；
/// - 错误类型分类（HP-4：JSON 不可重试，HTTP 5xx/429 由 LlmClient 内部退避）。
pub(crate) async fn generate_agent_json(
    state: &AppState,
    account_id: Option<&str>,
    contact_wxid: Option<&str>,
    run_id: Option<&str>,
    prompt_key: &str,
    system: &str,
    user: &str,
) -> AppResult<Value> {
    let started_at = DateTime::now();
    // M4 W4 Task 5.3：把 prompt_pack_version 折进 cache key，让 release_prompt /
    // ensure_prompt_pack_v2 / ensure_evolution_prompt_pack_v1 fetch_add 后旧
    // entry 自动失效，无需 LRU 直接清空。
    let pack_version = state
        .prompt_pack_version
        .load(std::sync::atomic::Ordering::SeqCst);
    let cache_key = llm_exact_cache_key(prompt_key, system, user, pack_version);
    if let Some(key) = cache_key.as_ref() {
        let cached = {
            let mut cache = LLM_EXACT_CACHE.lock();
            cache.get(key).cloned()
        };
        if let Some(value) = cached {
            let _ = state
                .db
                .llm_call_logs()
                .insert_one(
                    LlmCallLog {
                        id: None,
                        workspace_id: state.config.default_workspace_id.clone(),
                        account_id: account_id.map(ToString::to_string),
                        contact_wxid: contact_wxid.map(ToString::to_string),
                        run_id: run_id.map(ToString::to_string),
                        prompt_key: prompt_key.to_string(),
                        model: state.config.openai_model.clone(),
                        status: "cache_hit".to_string(),
                        latency_ms: DateTime::now().timestamp_millis()
                            - started_at.timestamp_millis(),
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                        prompt_cache_hit_tokens: 0,
                        prompt_cache_miss_tokens: 0,
                        error: None,
                        retry_count: 0,
                        final_status: Some("cache_hit".to_string()),
                        created_at: started_at,
                    },
                    None,
                )
                .await;
            return Ok(value);
        }
    }
    match state.llm.generate_json_with_usage(system, user).await {
        Ok(result) => {
            let usage = result.usage.clone();
            let value = result.value;
            let retry_count_i32 = result.retry_count.min(i32::MAX as u32) as i32;
            // MP-5 / Task 15：累计到当前 run 的 budget。
            if let Some(budget) = current_run_budget() {
                budget.record_call(usage.total_tokens);
            }
            if let Some(key) = cache_key {
                LLM_EXACT_CACHE.lock().put(key, value.clone());
            }
            log_llm_call_success(
                state,
                account_id,
                contact_wxid,
                run_id,
                prompt_key,
                result.model,
                result.latency_ms,
                &usage,
                retry_count_i32,
                started_at,
            )
            .await;
            Ok(value)
        }
        Err(error) => {
            log_llm_call_failure(
                state,
                account_id,
                contact_wxid,
                run_id,
                prompt_key,
                &error,
                started_at,
            )
            .await;
            Err(error)
        }
    }
}

/// 流式版本的 [`generate_agent_json`]：走 [`LlmProvider::generate_json_streaming`]，
/// 把上游模型**原始 JSON 文本片段**逐段推入 `token_tx`（由调用方在通道下游做
/// 增量答案抽取），返回时给出与 [`generate_agent_json`] 同形的最终 `value`。
///
/// 与非流式版共用 budget 计费与 `llm_call_logs` 写入语义（success / failed /
/// json_error），但**不走 LRU 精确缓存** —— 唯一调用方是 `knowledge.agent`
/// prompt key，本就不在 [`llm_exact_cache_key`] 白名单内，省掉缓存读写不改变行为。
pub(crate) async fn generate_agent_json_streaming(
    state: &AppState,
    account_id: Option<&str>,
    contact_wxid: Option<&str>,
    run_id: Option<&str>,
    prompt_key: &str,
    system: &str,
    user: &str,
    token_tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> AppResult<Value> {
    let started_at = DateTime::now();
    match state
        .llm
        .generate_json_streaming(system, user, token_tx)
        .await
    {
        Ok(result) => {
            let usage = result.usage.clone();
            let value = result.value;
            let retry_count_i32 = result.retry_count.min(i32::MAX as u32) as i32;
            if let Some(budget) = current_run_budget() {
                budget.record_call(usage.total_tokens);
            }
            log_llm_call_success(
                state,
                account_id,
                contact_wxid,
                run_id,
                prompt_key,
                result.model,
                result.latency_ms,
                &usage,
                retry_count_i32,
                started_at,
            )
            .await;
            Ok(value)
        }
        Err(error) => {
            log_llm_call_failure(
                state,
                account_id,
                contact_wxid,
                run_id,
                prompt_key,
                &error,
                started_at,
            )
            .await;
            Err(error)
        }
    }
}

/// 写一条 `success` 的 `llm_call_logs` 行。供 [`generate_agent_json`] 与
/// [`generate_agent_json_streaming`] 共用，`model` 取上游实际返回的模型名。
#[allow(clippy::too_many_arguments)]
async fn log_llm_call_success(
    state: &AppState,
    account_id: Option<&str>,
    contact_wxid: Option<&str>,
    run_id: Option<&str>,
    prompt_key: &str,
    model: String,
    latency_ms: i64,
    usage: &crate::llm::ChatUsage,
    retry_count_i32: i32,
    started_at: DateTime,
) {
    let _ = state
        .db
        .llm_call_logs()
        .insert_one(
            LlmCallLog {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: account_id.map(ToString::to_string),
                contact_wxid: contact_wxid.map(ToString::to_string),
                run_id: run_id.map(ToString::to_string),
                prompt_key: prompt_key.to_string(),
                model,
                status: "success".to_string(),
                latency_ms,
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
                prompt_cache_hit_tokens: usage.prompt_cache_hit_tokens,
                prompt_cache_miss_tokens: usage.prompt_cache_miss_tokens,
                error: None,
                retry_count: retry_count_i32,
                final_status: Some("success".to_string()),
                created_at: started_at,
            },
            None,
        )
        .await;
}

/// 写一条 `failed` 的 `llm_call_logs` 行。HP-4：按错误类型把 `final_status`
/// 区分为 `json_error` / `failed`，model 用配置默认（失败时无上游实际模型名）。
async fn log_llm_call_failure(
    state: &AppState,
    account_id: Option<&str>,
    contact_wxid: Option<&str>,
    run_id: Option<&str>,
    prompt_key: &str,
    error: &AppError,
    started_at: DateTime,
) {
    let final_status = match error {
        AppError::Json(_) => "json_error",
        _ => "failed",
    };
    let _ = state
        .db
        .llm_call_logs()
        .insert_one(
            LlmCallLog {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: account_id.map(ToString::to_string),
                contact_wxid: contact_wxid.map(ToString::to_string),
                run_id: run_id.map(ToString::to_string),
                prompt_key: prompt_key.to_string(),
                model: state.config.openai_model.clone(),
                status: "failed".to_string(),
                latency_ms: DateTime::now().timestamp_millis() - started_at.timestamp_millis(),
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                prompt_cache_hit_tokens: 0,
                prompt_cache_miss_tokens: 0,
                error: Some(error.to_string()),
                retry_count: retry_count_from_error(&error.to_string()),
                final_status: Some(final_status.to_string()),
                created_at: started_at,
            },
            None,
        )
        .await;
}

fn retry_count_from_error(error: &str) -> i32 {
    error
        .split("retry_count=")
        .nth(1)
        .and_then(|tail| {
            tail.chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse::<i32>()
                .ok()
        })
        .unwrap_or(0)
}

fn llm_exact_cache_key(
    prompt_key: &str,
    system: &str,
    user: &str,
    pack_version: u64,
) -> Option<String> {
    if !matches!(
        prompt_key,
        "knowledge.import.preview"
            | "playbook.generator"
            | "playbook.optimizer"
            | "user.guide.preview"
    ) {
        return None;
    }
    Some(format!(
        "{}:{}:{}:{}",
        prompt_key,
        pack_version,
        stable_agent_hash(system),
        stable_agent_hash(user)
    ))
}

fn stable_agent_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::budget::RunBudget;
    use super::gateway::inbound_marker_for_context_check;
    use super::guards::{check_state_transition, normalize_decision_state};
    use super::memory::{compact_memory_card, compact_memory_card_with_previous};
    use super::reaction::reaction_outcome_status;
    use super::review::effective_review_mode;
    use super::runtime::UserRuntimeParameters;
    use super::types::{AgentDecision, RunPlannerResult};
    use crate::models::{AgentStatus, Contact, OperationDomainConfig};
    use mongodb::bson::{doc, DateTime, Document};

    fn runtime() -> UserRuntimeParameters {
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
            // agent-autonomy-loop W0 / Task 1.3：测试默认值与
            // `RuntimeParametersTyped::default()` 保持一致，方便后续 wave 的
            // gating / clamp 行为复用同一组默认。
            autonomy_protocol_enabled: true,
            knowledge_routing_mode: "auto_tool_loop".to_string(),
            knowledge_max_tool_loops: 3,
            knowledge_max_tool_calls: 6,
            knowledge_open_slice_max_k: 4,
            knowledge_search_top_k: 8,
            outbox_poll_interval_seconds: 5,
            outbox_lease_seconds: 60,
            quiet_hours_enabled: true,
            quiet_hours_start: 22,
            quiet_hours_end: 8,
            quiet_hours_tz_offset_hours: 8,
            allowed_conversation_modes: crate::agent::runtime::default_conversation_modes(),
            grounding_gate_bypass_without_claim: false,
        }
    }

    #[test]
    fn reaction_outcome_prefers_model_status() {
        let analysis = doc! {
            "outcomeStatus": "user_replied_continue_exploring",
            "stopRequested": true
        };
        assert_eq!(
            reaction_outcome_status(&analysis),
            "user_replied_continue_exploring"
        );
    }

    #[test]
    fn reaction_outcome_uses_structured_flags_without_keywords() {
        let analysis = doc! { "buyingSignal": true };
        assert_eq!(
            reaction_outcome_status(&analysis),
            "user_replied_buying_signal"
        );
    }

    #[test]
    fn runtime_document_keeps_gateway_parameters() {
        let doc = runtime().as_document();
        assert_eq!(doc.get_i32("factRiskBlockAt").unwrap(), 6);
        assert_eq!(doc.get_i64("maxDailyTouches").unwrap(), 3);
    }

    #[test]
    fn operation_state_name_is_normalized_to_key() {
        let config = OperationDomainConfig {
            id: None,
            workspace_id: "default".to_string(),
            domain: "user_operations".to_string(),
            name: "用户运营".to_string(),
            goal: String::new(),
            methodology: String::new(),
            workflow: String::new(),
            tool_policy: String::new(),
            automation_policy: String::new(),
            review_policy: String::new(),
            runtime_parameters: Document::new(),
            state_machine: doc! { "states": [{ "key": "need_discovery", "name": "需求探索", "allowedFrom": ["need_discovery"] }] },
            status: "active".to_string(),
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
        };
        let mut decision = AgentDecision {
            operation_state: Some("需求探索".to_string()),
            ..Default::default()
        };
        normalize_decision_state(&mut decision, Some(&config));
        assert_eq!(decision.operation_state.as_deref(), Some("need_discovery"));
    }

    fn test_contact_template() -> Contact {
        Contact {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            wxid: "test_wxid".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            agent_status: AgentStatus::Managed,
            human_profile_note: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            tags: Vec::new(),
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            domain_attributes: None,
            domain_attributes_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn inbound_marker_prefers_last_inbound_at_over_last_message_at() {
        // 出站把 last_message_at 推进到 200，但 last_inbound_at 仍停在 100。
        // context_changed 检查必须只看用户实际说话的时间。
        let contact = Contact {
            last_inbound_at: Some(DateTime::from_millis(100)),
            last_message_at: Some(DateTime::from_millis(200)),
            ..test_contact_template()
        };
        let marker = inbound_marker_for_context_check(&contact)
            .expect("should fall through to last_inbound_at");
        assert_eq!(marker.timestamp_millis(), 100);
    }

    #[test]
    fn inbound_marker_falls_back_to_last_message_at_when_inbound_missing() {
        // 老数据 last_inbound_at 还没回填，要兼容地用 last_message_at。
        let contact = Contact {
            last_inbound_at: None,
            last_message_at: Some(DateTime::from_millis(500)),
            ..test_contact_template()
        };
        let marker = inbound_marker_for_context_check(&contact)
            .expect("should fall back to last_message_at");
        assert_eq!(marker.timestamp_millis(), 500);
    }

    #[test]
    fn inbound_marker_returns_none_when_both_missing() {
        let contact = test_contact_template();
        assert!(inbound_marker_for_context_check(&contact).is_none());
    }

    #[test]
    fn inbound_marker_blocks_follow_up_when_inbound_after_task() {
        // 跟进任务在 1000 创建，用户在 1500 又发了一条新消息，
        // 此时 follow-up 应被 context_changed 拦截。
        let contact = Contact {
            last_inbound_at: Some(DateTime::from_millis(1500)),
            last_message_at: Some(DateTime::from_millis(1500)),
            ..test_contact_template()
        };
        let task_created_at_ms = 1000_i64;
        let marker = inbound_marker_for_context_check(&contact).expect("should have a marker");
        assert!(marker.timestamp_millis() > task_created_at_ms);
    }

    #[test]
    fn inbound_marker_allows_follow_up_when_only_outbound_advanced() {
        // 模拟出站后 last_message_at 被推进到 2000（max(inbound, now)），
        // 但 last_inbound_at 仍停留在 500，跟进任务创建于 1000。
        // 用 last_message_at 会误判 context_changed；用 last_inbound_at 不会。
        let contact = Contact {
            last_inbound_at: Some(DateTime::from_millis(500)),
            last_message_at: Some(DateTime::from_millis(2000)),
            ..test_contact_template()
        };
        let task_created_at_ms = 1000_i64;
        let marker = inbound_marker_for_context_check(&contact).expect("should have a marker");
        assert!(
            marker.timestamp_millis() <= task_created_at_ms,
            "agent 自己出站不应触发 context_changed",
        );
    }

    fn typed_card_with_core_facts(facts: &[&str]) -> crate::models::MemoryCardTyped {
        crate::models::MemoryCardTyped {
            core_facts: facts
                .iter()
                .map(|s| crate::models::MemoryFactRepr::Plain(s.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn typed_card_with_recent_facts(facts: &[String]) -> crate::models::MemoryCardTyped {
        crate::models::MemoryCardTyped {
            recent_facts: facts
                .iter()
                .map(|s| crate::models::MemoryFactRepr::Plain(s.clone()))
                .collect(),
            ..Default::default()
        }
    }

    fn fact_texts(facts: &[crate::models::MemoryFactRepr]) -> Vec<String> {
        facts.iter().map(|f| f.as_text().to_string()).collect()
    }

    #[test]
    fn compact_memory_card_caps_core_facts_at_six() {
        // coreFacts 上限 6：超出的尾部应被截留掉。
        let card = typed_card_with_core_facts(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let compacted = compact_memory_card(&card);
        assert_eq!(
            compacted.core_facts.len(),
            6,
            "coreFacts must be capped at 6"
        );
    }

    #[test]
    fn compact_memory_card_caps_recent_facts_at_ten() {
        let recents: Vec<String> = (0..15).map(|i| format!("f{i}")).collect();
        let card = typed_card_with_recent_facts(&recents);
        let compacted = compact_memory_card(&card);
        assert_eq!(
            compacted.recent_facts.len(),
            10,
            "recentFacts must be capped at 10"
        );
    }

    #[test]
    fn compact_memory_card_with_previous_preserves_undiscarded_core_facts() {
        // MP-8 关键不变量：上一版 coreFacts 中未被 discarded 的事实必须在结果里。
        let previous = typed_card_with_core_facts(&["important_fact_1", "important_fact_2"]);
        let incoming = typed_card_with_core_facts(&["new_fact"]);
        let merged = compact_memory_card_with_previous(&incoming, Some(&previous), &[]);
        let cores = fact_texts(&merged.core_facts);
        assert!(cores.contains(&"important_fact_1".to_string()));
        assert!(cores.contains(&"important_fact_2".to_string()));
        assert!(cores.contains(&"new_fact".to_string()));
    }

    #[test]
    fn compact_memory_card_with_previous_drops_explicitly_discarded() {
        // discarded 列出的事实必须被显式移除，不会因合并语义又被带回来。
        let previous = typed_card_with_core_facts(&["old_fact_to_drop", "keep_me"]);
        let incoming = typed_card_with_core_facts(&["new_fact"]);
        let discarded = vec!["old_fact_to_drop".to_string()];
        let merged = compact_memory_card_with_previous(&incoming, Some(&previous), &discarded);
        let cores = fact_texts(&merged.core_facts);
        assert!(!cores.contains(&"old_fact_to_drop".to_string()));
        assert!(cores.contains(&"keep_me".to_string()));
        assert!(cores.contains(&"new_fact".to_string()));
    }

    fn full_state_machine_config() -> OperationDomainConfig {
        OperationDomainConfig {
            id: None,
            workspace_id: "default".to_string(),
            domain: "user_operations".to_string(),
            name: "用户运营".to_string(),
            goal: String::new(),
            methodology: String::new(),
            workflow: String::new(),
            tool_policy: String::new(),
            automation_policy: String::new(),
            review_policy: String::new(),
            runtime_parameters: Document::new(),
            state_machine: crate::prompts::default_user_operation_state_machine(),
            status: "active".to_string(),
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
            principal_decider: None,
            high_risk_escalation_mode: None,
        }
    }

    #[test]
    fn state_transition_allows_legal_path() {
        let config = full_state_machine_config();
        // new_contact -> relationship_building 是合法的。
        assert!(check_state_transition(
            Some(&config),
            Some("new_contact"),
            "relationship_building"
        )
        .is_none());
        // need_discovery -> solution_fit 合法。
        assert!(
            check_state_transition(Some(&config), Some("need_discovery"), "solution_fit").is_none()
        );
        // commitment_followup -> customer_success 合法。
        assert!(check_state_transition(
            Some(&config),
            Some("commitment_followup"),
            "customer_success"
        )
        .is_none());
    }

    #[test]
    fn state_transition_blocks_jump_to_customer_success() {
        // new_contact 不能直接跳到 customer_success（必须经过 commitment_followup）。
        let config = full_state_machine_config();
        let reason = check_state_transition(Some(&config), Some("new_contact"), "customer_success");
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("state_transition_invalid"));
    }

    #[test]
    fn state_transition_cooldown_allow_from_any() {
        let config = full_state_machine_config();
        // 任意状态 -> cooldown 都合法。
        assert!(check_state_transition(Some(&config), Some("new_contact"), "cooldown").is_none());
        assert!(check_state_transition(Some(&config), Some("solution_fit"), "cooldown").is_none());
        assert!(check_state_transition(Some(&config), None, "cooldown").is_none());
    }

    #[test]
    fn state_transition_self_loop_allowed() {
        let config = full_state_machine_config();
        // 同状态不变不应被拦截。
        assert!(
            check_state_transition(Some(&config), Some("solution_fit"), "solution_fit").is_none()
        );
    }

    #[test]
    fn state_transition_empty_from_only_allows_new_contact() {
        let config = full_state_machine_config();
        assert!(check_state_transition(Some(&config), None, "new_contact").is_none());
        assert!(check_state_transition(Some(&config), Some(""), "need_discovery").is_some());
    }

    #[test]
    fn state_transition_skips_when_no_state_machine() {
        // 没有 domain_config 时不强校验。
        assert!(check_state_transition(None, Some("anywhere"), "anywhere_else").is_none());
    }

    fn high_risk_planner() -> RunPlannerResult {
        RunPlannerResult {
            risk_level: "high".to_string(),
            ..Default::default()
        }
    }

    fn low_risk_light_planner() -> RunPlannerResult {
        RunPlannerResult {
            risk_level: "medium".to_string(),
            review_mode: "light".to_string(),
            knowledge_required: false,
            ..Default::default()
        }
    }

    #[test]
    fn effective_review_mode_low_confidence_forces_full() {
        let runtime = runtime();
        let planner = low_risk_light_planner();
        let decision = AgentDecision {
            operation_state_confidence: Some(3),
            ..Default::default()
        };
        // confidence=3 < threshold=4 强制 full。
        assert_eq!(
            effective_review_mode(&planner, &decision, &runtime, false),
            "full"
        );
    }

    #[test]
    fn effective_review_mode_high_confidence_keeps_light() {
        let runtime = runtime();
        let planner = low_risk_light_planner();
        let decision = AgentDecision {
            operation_state_confidence: Some(8),
            ..Default::default()
        };
        assert_eq!(
            effective_review_mode(&planner, &decision, &runtime, false),
            "light"
        );
    }

    #[test]
    fn effective_review_mode_missing_confidence_keeps_light() {
        // None 视作 10（最高），不强制 full。
        let runtime = runtime();
        let planner = low_risk_light_planner();
        let decision = AgentDecision::default();
        assert_eq!(
            effective_review_mode(&planner, &decision, &runtime, false),
            "light"
        );
    }

    #[test]
    fn effective_review_mode_high_risk_overrides() {
        // 高风险不论 confidence 都走 full。
        let runtime = runtime();
        let planner = high_risk_planner();
        let decision = AgentDecision {
            operation_state_confidence: Some(8),
            ..Default::default()
        };
        assert_eq!(
            effective_review_mode(&planner, &decision, &runtime, false),
            "full"
        );
    }

    #[test]
    fn run_budget_record_call_increments() {
        let budget = RunBudget::new("run_t", 1000, 3, i32::MAX);
        budget.record_call(200);
        budget.record_call(300);
        let snap = budget.snapshot();
        assert_eq!(snap.tokens_used, 500);
        assert_eq!(snap.llm_calls_used, 2);
        assert!(!budget.is_exceeded());
    }

    #[test]
    fn run_budget_token_exceeded_marks_exceeded() {
        let budget = RunBudget::new("run_t", 100, 10, i32::MAX);
        budget.record_call(60);
        assert!(!budget.is_exceeded());
        budget.record_call(50);
        assert!(budget.is_exceeded(), "tokens_used 110 >= budget 100");
    }

    #[test]
    fn run_budget_call_count_exceeded_marks_exceeded() {
        let budget = RunBudget::new("run_t", 100000, 2, i32::MAX);
        budget.record_call(1);
        budget.record_call(1);
        assert!(budget.is_exceeded(), "llm_calls_used 2 >= max 2");
    }

    #[test]
    fn run_budget_mark_degraded_records_reason() {
        let budget = RunBudget::new("run_t", 100, 2, i32::MAX);
        budget.mark_degraded("review_skipped_budget_exceeded");
        budget.mark_degraded("rewrite_skipped_budget_exceeded");
        let snap = budget.snapshot();
        assert_eq!(snap.degraded_reasons.len(), 2);
        assert!(snap.degraded_reasons[0].contains("review"));
    }

    /// 波 D2：confidence < 阈值时强制 full review，并把 confidence_override_*
    /// 字段写到 planner，从而 to_document(planner) 后能落进 agent_run_logs.planner。
    #[test]
    fn apply_confidence_override_marks_planner_when_below_threshold() {
        let mut planner = RunPlannerResult {
            risk_level: "medium".to_string(),
            review_mode: "light".to_string(),
            reason: "原因 A".to_string(),
            ..Default::default()
        };
        let decision = AgentDecision {
            operation_state_confidence: Some(2),
            ..Default::default()
        };
        let mut runtime = runtime();
        runtime.operation_state_confidence_full_review_below = 4;
        super::gateway::apply_confidence_override(&mut planner, &decision, &runtime);
        assert_eq!(planner.review_mode, "full");
        assert!(planner.confidence_override_triggered);
        assert!(
            planner
                .confidence_override_reason
                .contains("operation_state_confidence=2"),
            "reason 中应说明触发的 confidence 值，实际：{}",
            planner.confidence_override_reason
        );
        // 原 reason 不丢，新 reason 追加在后面。
        assert!(planner.reason.contains("原因 A"));
        assert!(planner.reason.contains("below threshold 4"));

        // 落进 Document 后的 key 形态（agent_run_logs.planner 实际字段）。
        let planner_doc = mongodb::bson::to_document(&planner).expect("serialize");
        assert_eq!(
            planner_doc.get_bool("confidenceOverrideTriggered").ok(),
            Some(true),
            "agent_run_logs.planner.confidenceOverrideTriggered 必须是 true"
        );
        assert!(planner_doc
            .get_str("confidenceOverrideReason")
            .map(|s| s.contains("operation_state_confidence=2"))
            .unwrap_or(false));
    }

    /// 波 D2：confidence ≥ 阈值时不动 planner（保持 light review）。
    #[test]
    fn apply_confidence_override_no_op_when_above_threshold() {
        let mut planner = RunPlannerResult {
            risk_level: "medium".to_string(),
            review_mode: "light".to_string(),
            reason: "原因 A".to_string(),
            ..Default::default()
        };
        let decision = AgentDecision {
            operation_state_confidence: Some(8),
            ..Default::default()
        };
        let runtime = runtime();
        super::gateway::apply_confidence_override(&mut planner, &decision, &runtime);
        assert_eq!(planner.review_mode, "light");
        assert!(!planner.confidence_override_triggered);
        assert!(planner.confidence_override_reason.is_empty());
    }

    /// 波 D2：confidence 缺失时视为 10（最高），不强制 full。
    #[test]
    fn apply_confidence_override_treats_missing_as_max() {
        let mut planner = RunPlannerResult {
            risk_level: "medium".to_string(),
            review_mode: "light".to_string(),
            ..Default::default()
        };
        let decision = AgentDecision {
            operation_state_confidence: None,
            ..Default::default()
        };
        let runtime = runtime();
        super::gateway::apply_confidence_override(&mut planner, &decision, &runtime);
        assert_eq!(planner.review_mode, "light");
        assert!(!planner.confidence_override_triggered);
    }

    /// M4 W4 Task 5.3：cache key 折入 prompt_pack_version——不同 version
    /// 必须产出不同 key，让 release_prompt 后旧 entry 自动失效。
    #[test]
    fn llm_exact_cache_key_changes_when_prompt_pack_version_bumps() {
        let v0 = super::llm_exact_cache_key("playbook.optimizer", "sys-A", "user-A", 0)
            .expect("whitelisted prompt key produces cache key");
        let v1 = super::llm_exact_cache_key("playbook.optimizer", "sys-A", "user-A", 1)
            .expect("whitelisted prompt key produces cache key");
        assert_ne!(v0, v1, "bumping version must invalidate cache key");
    }

    /// 同 version + 同 system/user → 同 key（cache hit 正确路径）。
    #[test]
    fn llm_exact_cache_key_stable_within_same_version() {
        let a = super::llm_exact_cache_key("playbook.generator", "sys", "user", 7)
            .expect("whitelisted prompt key produces cache key");
        let b = super::llm_exact_cache_key("playbook.generator", "sys", "user", 7)
            .expect("whitelisted prompt key produces cache key");
        assert_eq!(a, b);
    }

    /// 非白名单 prompt_key 永远不进 cache（None），不受 version 影响。
    #[test]
    fn llm_exact_cache_key_returns_none_for_non_whitelisted_prompt_key() {
        assert!(
            super::llm_exact_cache_key("agent.decision", "sys", "user", 0).is_none(),
            "non-whitelisted prompt_key must not enter LRU cache"
        );
    }
}
