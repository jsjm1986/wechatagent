//! 用户运营 Agent 网关入口与发送链路。
//!
//! 该模块汇集了所有"动手做事"的步骤：
//! - `run_user_operation_gateway` / `_inner`：reply / follow-up / send-once
//!   三种触发统一进入这里；负责构建 `RunBudget` task-local、串联
//!   precheck → decide → router → review → rewrite → send → 写日志的完整链路；
//! - `precheck_send_gateway`、`precheck_operation_policy`：发送前各种频控、
//!   冷却期与运营策略检查；
//! - `send_outbound_message`：实际调 MCP `message_send_text` 并把出站消息
//!   写回 `conversation_messages`，同时把 `last_outbound_at` /
//!   `last_message_at` 用 aggregation pipeline 原子推进；
//! - `apply_agent_updates` / `apply_operating_memory_update`：决策成功后
//!   把画像、tags、operationState、follow-up 任务、operating memory 等
//!   写回 contact / operating_memories / agent_tasks；
//! - `write_decision_review` / `write_agent_run_log` / `write_event_for_account`
//!   等审计写入；
//! - `handle_managed_message` / `handle_follow_up_task`：webhook 入站消息
//!   与 worker 跟进任务的两个外部入口；
//! - `send_contact_message_gateway`：管理 Agent 主动发送的"生产发送网关"。

use std::sync::Arc;

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, to_document, Bson, DateTime, Document};
use mongodb::options::FindOptions;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::mcp;
use crate::models::{
    AgentDecisionReview, AgentEvent, AgentRunLog, AgentStatus, AgentTask, Contact,
    ConversationMessage, MessageDirection, OperationDomainConfig, OperationPlaybook,
};
use crate::prompts;
use crate::routes::AppState;

use super::budget::{current_run_budget, RunBudget, RUN_BUDGET};
use super::decision::{
    decide_reply_with_promote, load_operation_playbook_for_contact,
    load_operation_state_policy_for_contact, load_user_operation_domain_config_for_contact,
};
use super::guards::{
    classify_decision_action, enforce_state_action_policy, normalize_decision_runtime,
    normalize_decision_state, planner_from_decision,
};
use super::knowledge_router::{
    empty_knowledge_route, load_operation_knowledge,
    maybe_emit_unverified_warning, route_operation_knowledge, route_used_knowledge_ids,
    select_operation_knowledge_chunks, write_knowledge_usage_log,
};
use super::memory::{
    effective_memory_card, effective_memory_card_for_contact, load_or_create_operating_memory,
    memory_card_has_signal, next_memory_card_version, schedule_memory_consolidation_task,
    write_memory_candidates,
};
use super::review::{
    decide_revision, derive_revision_failure, effective_review_mode, finalize_review_for_send,
    local_decision_review, review_decision, review_passed, should_run_review, FinalizeOutcome,
    GatewayStatusFinal, PendingFinalizeEvent, RevisionDecision,
};
use super::run_envelope::{
    assert_final_review_status_valid, assert_gateway_status_valid, assert_lifecycle_valid,
    derive_lifecycle_from_status, SOURCE_KIND_FOLLOW_UP_TASK, SOURCE_KIND_INBOUND_MESSAGE,
    SOURCE_KIND_MANUAL_SEND,
};
use super::runtime::UserRuntimeParameters;
use super::types::{
    doc_bool, doc_i64, doc_string, non_empty_option, parse_rfc3339_to_bson, to_bson_array,
    AgentDecision, AgentTrigger, ContactSendResult, DecisionReviewResult, KnowledgeRouteResult,
    ManualContactSend, RunPlannerResult, SendGatewayResult,
};
use super::outbox::{enqueue as outbox_enqueue, EnqueueOutcome, EnqueueRequest};
use super::taxonomy::{
    check_value as taxonomy_check_value, global_taxonomy_cache, upsert_candidate as taxonomy_upsert_candidate,
    TaxonomyMatch,
};

pub async fn handle_managed_message(
    state: &AppState,
    contact: Contact,
    inbound: &ConversationMessage,
) -> AppResult<()> {
    run_user_operation_gateway(state, contact, AgentTrigger::Inbound(inbound), None).await
}

pub async fn handle_follow_up_task(state: &AppState, task: AgentTask) -> AppResult<()> {
    let Some(task_id) = task.id else {
        return Ok(());
    };
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "wxid": &task.contact_wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("follow-up contact not found".to_string()))?;
    run_user_operation_gateway(state, contact, AgentTrigger::FollowUp(&task), Some(task_id)).await
}

pub async fn send_contact_message_gateway(
    state: &AppState,
    contact: Contact,
    request: ManualContactSend,
) -> AppResult<ContactSendResult> {
    if request.content.trim().is_empty() {
        return Err(AppError::BadRequest("content is required".to_string()));
    }
    let content = request.content.trim().to_string();
    let domain_config =
        load_user_operation_domain_config_for_contact(state, &contact.workspace_id, &contact.wxid)
            .await?;
    let mut runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    // M4 W4 Task 5.1：通过 resolve_thresholds 把 threshold_overrides 的最新生效值
    // 写回 runtime，5 闸 block/rewrite 阈值即时反映 release。
    crate::agent::runtime::resolve_thresholds(state, &contact)
        .await?
        .apply_to_runtime(&mut runtime);
    let synthetic_inbound = ConversationMessage {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        message_id: None,
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "后台管理 Agent 请求发送私聊，请按生产发送网关进行频控和审查。".to_string(),
        raw: Some(request.source.clone()),
        created_at: DateTime::now(),
    };
    let trigger = AgentTrigger::Inbound(&synthetic_inbound);
    let run_id = uuid::Uuid::new_v4().to_string();
    let planner = RunPlannerResult {
        risk_level: "high".to_string(),
        context_needs_refresh: true,
        memory_change_importance: 6,
        knowledge_required: true,
        review_mode: "full".to_string(),
        reason: "后台管理 Agent 主动请求发送，需要完整审查".to_string(),
        confidence_override_triggered: false,
        confidence_override_reason: String::new(),
    };
    let precheck = precheck_send_gateway(state, &contact, &trigger, &runtime).await?;
    if !precheck.allowed {
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "send_gateway_blocked",
            &precheck.status,
            &precheck.reason,
            Some(to_document(&precheck).unwrap_or_default()),
        )
        .await?;
        return Err(AppError::BadRequest(precheck.reason));
    }

    let playbook = load_operation_playbook_for_contact(state, &contact).await?;
    let memory = load_or_create_operating_memory(state, &contact).await?;
    let operation_knowledge = load_operation_knowledge(state, &contact).await?;
    let context_messages = load_context_messages(state, &contact, &runtime).await?;
    // task 6.3：边界处把 typed 转为 Document wire shape，下游 prompt 注入
    // 路径不变。
    let context_pack = effective_memory_card_for_contact(&memory, &contact).to_document();
    let knowledge_route = route_operation_knowledge(
        state,
        &contact,
        &synthetic_inbound,
        &context_messages,
        &memory,
        &context_pack,
        &operation_knowledge,
        Some(&run_id),
    )
    .await?;
    let selected_chunks =
        select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);
    let decision = AgentDecision {
        should_reply: true,
        reply_text: content.clone(),
        context_pack_version: Some(next_memory_card_version(&memory)),
        used_knowledge_ids: route_used_knowledge_ids(&knowledge_route),
        next_best_action: doc! {
            "source": "management_agent_send",
            "originalContentLocked": request.original_content_locked,
        },
        ..Default::default()
    };
    let review = review_decision(
        state,
        &contact,
        &synthetic_inbound,
        &decision,
        playbook.as_ref(),
        domain_config.as_ref(),
        &runtime,
        &memory,
        &context_pack,
        &selected_chunks,
        &knowledge_route,
        "full",
        Some(&run_id),
    )
    .await?;
    if !review_passed(&review, &runtime) {
        let blocked_result = SendGatewayResult {
            allowed: false,
            status: "review_blocked".to_string(),
            reason: "Review Agent 拦截本次发送".to_string(),
            policy_blocks: vec!["review_blocked".to_string()],
            run_mode: "live".to_string(),
            message_id: None,
        };
        let review_id = write_decision_review(
            state,
            &contact,
            &synthetic_inbound,
            &decision,
            &review,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &blocked_result,
            &context_pack,
            "blocked",
            &knowledge_route,
            &run_id,
            &planner,
        )
        .await?;
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "blocked_review",
            "blocked",
            "生产发送网关 Review 未通过，已拦截私聊发送",
            Some(review_event_details(&review)),
        )
        .await?;
        return Ok(ContactSendResult {
            sent_content: content,
            message_id: None,
            review_approved: false,
            gateway_status: "review_blocked".to_string(),
            gateway_reason: "Review Agent 拦截本次发送".to_string(),
            decision_review_id: Some(review_id.to_hex()),
        });
    }

    let final_precheck = precheck_send_gateway(state, &contact, &trigger, &runtime).await?;
    if !final_precheck.allowed {
        let review_id = write_decision_review(
            state,
            &contact,
            &synthetic_inbound,
            &decision,
            &review,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &final_precheck,
            &context_pack,
            "gateway_blocked",
            &knowledge_route,
            &run_id,
            &planner,
        )
        .await?;
        return Ok(ContactSendResult {
            sent_content: content,
            message_id: None,
            review_approved: true,
            gateway_status: final_precheck.status,
            gateway_reason: final_precheck.reason,
            decision_review_id: Some(review_id.to_hex()),
        });
    }

    // S5.2 (Phase 0)：原先这里直接调 `send_outbound_message`，绕过 outbox →
    // 失去 R13 幂等键 + 二次安全门保护。改成 enqueue 到 `agent_send_outbox`，
    // dispatcher worker 异步消费 outbox 完成 MCP 发送。返回值的 messageId 在管理 API
    // 同步路径下不再可得（dispatcher 异步），按 R13.2 设计语义返回
    // gateway_status="outbox_enqueued"，调用方据此感知"已交付到发送队列"。
    let pending_result = SendGatewayResult {
        allowed: true,
        status: "outbox_enqueued".to_string(),
        reason: "Review 通过，已入队 outbox 等待 dispatcher 发送".to_string(),
        policy_blocks: Vec::new(),
        run_mode: "live".to_string(),
        message_id: None,
    };
    let review_id = write_decision_review(
        state,
        &contact,
        &synthetic_inbound,
        &decision,
        &review,
        playbook.as_ref(),
        domain_config.as_ref(),
        &runtime,
        &pending_result,
        &context_pack,
        "outbox_enqueued",
        &knowledge_route,
        &run_id,
        &planner,
    )
    .await?;

    let enqueue_req = EnqueueRequest {
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        run_id: run_id.clone(),
        decision_id: Some(review_id),
        // 管理 Agent 主动发送没有真实 inbound message_id，走 SOURCE_KIND_MANUAL_SEND
        // 语义；outbox::enqueue 的 synthetic 兜底会基于 run_id + content_hash 生成
        // idempotency_key，所以重复点 "发送" 不会真重复发。
        source_event_id: String::new(),
        source_kind: SOURCE_KIND_MANUAL_SEND.to_string(),
        content: content.clone(),
        max_attempts: 3,
    };
    match outbox_enqueue(state, enqueue_req).await {
        Ok(EnqueueOutcome::Created { outbox_id, .. }) => {
            tracing::info!(
                %run_id,
                %outbox_id,
                contact_wxid = %contact.wxid,
                "management send enqueued to outbox"
            );
        }
        Ok(EnqueueOutcome::IdempotentSkip { idempotency_key }) => {
            tracing::info!(
                %run_id,
                %idempotency_key,
                contact_wxid = %contact.wxid,
                "management send outbox idempotent skip"
            );
        }
        Err(err) => {
            tracing::error!(?err, %run_id, "management send outbox enqueue failed");
            return Err(err.into());
        }
    }

    write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "management_send",
        "enqueued",
        "生产发送网关已入队 outbox，dispatcher 将异步发送",
        Some(doc! {
            "sentContent": &content,
            "decisionReviewId": review_id.to_hex(),
            "originalContentLocked": request.original_content_locked,
        }),
    )
    .await?;
    Ok(ContactSendResult {
        sent_content: content,
        message_id: None,
        review_approved: true,
        gateway_status: "outbox_enqueued".to_string(),
        gateway_reason: "已入队 outbox，dispatcher 将异步发送".to_string(),
        decision_review_id: Some(review_id.to_hex()),
    })
}

/// S1.1 (Phase 0)：把 [`AgentTrigger`] 派生为 `(source_event_id, source_kind)`，
/// 透传给 `write_agent_run_log_with_finalize` 用于 R0.1 envelope 字段。
///
/// * `Inbound` → message_id（缺失走 `synthetic:` 前缀兜底）+ `inbound_message`
/// * `FollowUp` → task_id.hex + `follow_up_task`
fn trigger_envelope_source(trigger: &AgentTrigger<'_>) -> (String, &'static str) {
    match trigger {
        AgentTrigger::Inbound(message) => {
            let id = message
                .message_id
                .clone()
                .unwrap_or_else(|| format!("synthetic:{}", message.contact_wxid));
            (id, SOURCE_KIND_INBOUND_MESSAGE)
        }
        AgentTrigger::FollowUp(task) => {
            let id = task
                .id
                .map(|oid| oid.to_hex())
                .unwrap_or_else(|| "synthetic:follow_up".to_string());
            (id, SOURCE_KIND_FOLLOW_UP_TASK)
        }
    }
}

pub(crate) async fn run_user_operation_gateway(
    state: &AppState,
    contact: Contact,
    trigger: AgentTrigger<'_>,
    task_id: Option<ObjectId>,
) -> AppResult<()> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let inbound = trigger_message(&contact, &trigger);
    let domain_config =
        load_user_operation_domain_config_for_contact(state, &contact.workspace_id, &contact.wxid)
            .await?;
    let mut runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    // M4 W4 Task 5.1：通过 resolve_thresholds 把 threshold_overrides 的最新生效值
    // 写回 runtime，让 5 闸 block/rewrite 阈值即时反映 release，无需重启进程。
    crate::agent::runtime::resolve_thresholds(state, &contact)
        .await?
        .apply_to_runtime(&mut runtime);

    // MP-5 / Task 15：为本次 run 构建 budget，并通过 task_local 注入。
    // agent-autonomy-loop W3 / Task 4.1：从 runtime_parameters.knowledgeMaxToolCalls
    // 注入 tool_call_budget（loader 已 clamp 到 [1, 16]，默认 6）。
    let budget = Arc::new(RunBudget::new(
        run_id.clone(),
        runtime.run_token_budget,
        runtime.run_max_llm_calls,
        runtime.knowledge_max_tool_calls,
    ));

    RUN_BUDGET
        .scope(
            budget,
            run_user_operation_gateway_inner(
                state,
                contact,
                trigger,
                task_id,
                run_id,
                inbound,
                domain_config,
                runtime,
            ),
        )
        .await
}

#[allow(clippy::too_many_arguments)]
async fn run_user_operation_gateway_inner(
    state: &AppState,
    contact: Contact,
    trigger: AgentTrigger<'_>,
    task_id: Option<ObjectId>,
    run_id: String,
    inbound: ConversationMessage,
    domain_config: Option<OperationDomainConfig>,
    runtime: UserRuntimeParameters,
) -> AppResult<()> {
    // S1.1 (Phase 0)：派生 R0.1 envelope 的 (source_event_id, source_kind)，
    // 在所有终态写入点透传，确保 agent_run_logs 闭集字段非空。
    let (envelope_source_event_id, envelope_source_kind) = trigger_envelope_source(&trigger);
    let envelope_source_kind = envelope_source_kind.to_string();
    let precheck = precheck_send_gateway(state, &contact, &trigger, &runtime).await?;
    if !precheck.allowed {
        if let Some(task_id) = task_id {
            cancel_task(state, task_id, &precheck.status, &precheck.reason).await?;
        }
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "agent_skipped",
            &precheck.status,
            &precheck.reason,
            Some(to_document(&precheck).unwrap_or_default()),
        )
        .await?;
        write_agent_run_log(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            &precheck.status,
            &RunPlannerResult::default(),
            doc! { "refreshed": false, "reason": "precheck_blocked" },
            &KnowledgeRouteResult::default(),
            Document::new(),
            Document::new(),
            to_document(&precheck).unwrap_or_default(),
            None,
            &envelope_source_event_id,
            &envelope_source_kind,
        )
        .await?;
        return Ok(());
    }

    let recent_messages =
        load_recent_messages(state, &contact, runtime.recent_message_limit).await?;
    let pending_tasks = load_pending_tasks(state, &contact).await?;
    let playbook = load_operation_playbook_for_contact(state, &contact).await?;
    let memory = load_or_create_operating_memory(state, &contact).await?;
    let operation_knowledge = load_operation_knowledge(state, &contact).await?;
    // MP-9 / Task 16：知识库切片全部未验证时给出可见告警，避免运营人员困惑。
    let _ = maybe_emit_unverified_warning(state, &contact).await;
    // task 6.3：边界处把 typed 转为 Document wire shape，下游 prompt 注入
    // 路径不变。
    let memory_card = effective_memory_card_for_contact(&memory, &contact).to_document();
    let should_refresh_context = false;
    let context_pack = memory_card;
    let initial_planner = RunPlannerResult {
        risk_level: "medium".to_string(),
        review_mode: "light".to_string(),
        reason: "Reply Agent 内联判断运行链路，普通消息不再前置 Planner".to_string(),
        ..Default::default()
    };
    // ── WB5：永远先跑知识路由（删除原 decision_requires_knowledge short-circuit）───
    //
    // ISSUE-012 根因：旧链路是先让 Reply Agent 在没知识的情况下盲跑一遍、
    // 再据 knowledgeNeed 决定是否打开知识库——第一遍的寒暄态本身就让
    // knowledgeNeed=not_required，知识库永远进不来。
    //
    // 新链路：每轮都先跑 route_operation_knowledge（含硬关键词快路径），
    // Reply Agent 直接拿着真实知识做 single-pass 决策。预算超额时退化成
    // empty_knowledge_route 但不再回退到旧的两段式。成本：每轮 +1 LLM call
    // ≈ +800 tokens / inbound，已经预留在 RunBudget。
    let knowledge_route = if current_run_budget()
        .map(|b| b.is_exceeded())
        .unwrap_or(false)
    {
        if let Some(budget) = current_run_budget() {
            budget.mark_degraded("knowledge_route_skipped_budget_exceeded");
        }
        let mut route = empty_knowledge_route(&initial_planner);
        route.reason = "预算超额：跳过知识路由，沿用空知识做保守决策".to_string();
        route
    } else {
        route_operation_knowledge(
            state,
            &contact,
            &inbound,
            &recent_messages,
            &memory,
            &context_pack,
            &operation_knowledge,
            Some(&run_id),
        )
        .await?
    };
    let selected_chunks =
        select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);
    // agent-autonomy-loop W2 / Task 3.4：把 RawAgentDecision::validate_and_promote
    // 的 promote_risks 从 reply 调用一路 thread 到 finalize_review_for_send，
    // 由 finalize 阶段判定是否触发 R3.5/R3.6 blocked_by_required_field。
    let (mut decision, mut promote_risks) = decide_reply_with_promote(
        state,
        &contact,
        &inbound,
        &recent_messages,
        &pending_tasks,
        playbook.as_ref(),
        domain_config.as_ref(),
        &runtime,
        &memory,
        &context_pack,
        &selected_chunks,
        &knowledge_route,
        None,
        Some(&run_id),
    )
    .await?;
    normalize_decision_state(&mut decision, domain_config.as_ref());
    normalize_decision_runtime(&mut decision, &initial_planner);
    let mut planner = planner_from_decision(&decision, "Reply Agent 单轮决策（知识路由前置）");
    if !knowledge_route.selected_chunk_ids.is_empty() || !knowledge_route.selected_knowledge_ids.is_empty() {
        planner.knowledge_required = true;
        if planner.review_mode.trim().is_empty() {
            planner.review_mode = "full".to_string();
        }
    }
    apply_confidence_override(&mut planner, &decision, &runtime);
    normalize_decision_runtime(&mut decision, &planner);
    decision.context_pack_version = Some(next_memory_card_version(&memory));
    decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
    let _ = &mut promote_risks;
    // MP-5 / Task 15：进入 review 前预算超额则降级到 local。
    // agent-autonomy-loop W2 / Task 3.1：`local_decision_review` 改为接受
    // `&RunBudget`，在三分支前先抢一次 task-local 引用，None 时构造一个
    // 即时态空预算（is_exceeded() == false），以保持 unit 测试 / 非
    // RUN_BUDGET.scope 入口的兼容性。
    let run_budget = current_run_budget();
    let budget_exceeded_for_review = run_budget
        .as_ref()
        .map(|b| b.is_exceeded())
        .unwrap_or(false);
    let local_budget_fallback;
    let local_budget_ref: &RunBudget = match run_budget.as_ref() {
        Some(b) => b.as_ref(),
        None => {
            local_budget_fallback = RunBudget::new(run_id.clone(), i64::MAX, i32::MAX, i32::MAX);
            &local_budget_fallback
        }
    };
    let mut review = if budget_exceeded_for_review {
        if let Some(b) = run_budget.as_ref() {
            b.mark_degraded("review_skipped_budget_exceeded".to_string());
        }
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "run_budget_exceeded",
            "degraded",
            "预算超额：跳过 LLM review，使用 local_decision_review",
            Some(doc! { "stage": "review", "run_id": &run_id }),
        )
        .await?;
        local_decision_review(&decision, local_budget_ref)
    } else if should_run_review(&decision, &planner, &runtime) {
        review_decision(
            state,
            &contact,
            &inbound,
            &decision,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &memory,
            &context_pack,
            &selected_chunks,
            &knowledge_route,
            effective_review_mode(&planner, &decision, &runtime, false),
            Some(&run_id),
        )
        .await?
    } else {
        local_decision_review(&decision, local_budget_ref)
    };
    let mut final_decision = decision;

    if final_decision.should_reply
        && !review_passed(&review, &runtime)
        && !review.needs_revision
    {
        // Phase B / B1：`needs_revision=true` 表示 [`route_dual_gate`] 已经
        // 把当前 review 标为软闸-only 失败，应走 finalize 之后的 single-shot
        // revision 通道（decide_revision Proceed），而不是这里的 rewrite 路径
        // （rewrite_instruction 为空、且会再调一次 review 形成双重 LLM 调用）。
        // 让本分支只接住 hallucination / grounding 硬闸（reviewer 自己也会
        // 在硬闸失败时写非空 rewrite_instruction）。
        // MP-5 / Task 15：rewrite 之前再检查预算；超额时跳过 rewrite，直接走拦截路径。
        let budget_exceeded_for_rewrite = current_run_budget()
            .map(|b| b.is_exceeded())
            .unwrap_or(false);
        if budget_exceeded_for_rewrite {
            if let Some(b) = current_run_budget() {
                b.mark_degraded("rewrite_skipped_budget_exceeded".to_string());
            }
            write_event_for_account(
                state,
                &contact.account_id,
                Some(&contact.wxid),
                "run_budget_exceeded",
                "degraded",
                "预算超额：跳过 rewrite，本次按现有 review 结果决定是否拦截",
                Some(doc! { "stage": "rewrite", "run_id": &run_id }),
            )
            .await?;
        } else {
            write_decision_review(
                state,
                &contact,
                &inbound,
                &final_decision,
                &review,
                playbook.as_ref(),
                domain_config.as_ref(),
                &runtime,
                &precheck,
                &context_pack,
                "rewrite_requested",
                &knowledge_route,
                &run_id,
                &planner,
            )
            .await?;
            let (rewritten, rewrite_promote_risks) = decide_reply_with_promote(
                state,
                &contact,
                &inbound,
                &recent_messages,
                &pending_tasks,
                playbook.as_ref(),
                domain_config.as_ref(),
                &runtime,
                &memory,
                &context_pack,
                &selected_chunks,
                &knowledge_route,
                Some(&review.rewrite_instruction),
                Some(&run_id),
            )
            .await?;
            final_decision = rewritten;
            promote_risks = rewrite_promote_risks;
            normalize_decision_state(&mut final_decision, domain_config.as_ref());
            normalize_decision_runtime(&mut final_decision, &planner);
            final_decision.context_pack_version = Some(next_memory_card_version(&memory));
            final_decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
            review = review_decision(
                state,
                &contact,
                &inbound,
                &final_decision,
                playbook.as_ref(),
                domain_config.as_ref(),
                &runtime,
                &memory,
                &context_pack,
                &selected_chunks,
                &knowledge_route,
                "full",
                Some(&run_id),
            )
            .await?;
        }
    }

    // ── agent-autonomy-loop W2 / Task 3.4：finalize_review_for_send 接入 ──
    //
    // 三分支（budget_exceeded / should_run_review / 默认）的 review 结果在此
    // 统一汇总到 finalize_review_for_send。任一硬安全门触发 SHALL 强制
    // `final_decision.should_reply=false` 且 `final_decision.autonomy_mode="blocked"`，
    // 并产出待写 `agent_events`（由 [`persist_finalize_pending_events`] 持久化）。
    // 任何上游 `approved=true` SHALL NOT 绕过本调用（详见 design.md §4.5 / N3）。
    let outcome = finalize_review_for_send(
        review,
        &mut final_decision,
        &runtime,
        &contact,
        &selected_chunks,
        promote_risks.clone(),
        inbound.content.as_str(),
    );
    let FinalizeOutcome {
        review: finalized_review,
        status: mut finalize_status,
        pending_events,
    } = outcome;
    let mut review = finalized_review;
    persist_finalize_pending_events(state, &contact, &pending_events).await?;

    // ── Phase B / B4：operation_state_policies 终态再扣一道 ──
    //
    // finalize 走 Approved 之后再按当前 operation_state 校验"该状态允许 / 禁止
    // agent 做哪类动作"。命中 forbidden 或 allowlist 收敛模式不含本次 action，
    // 强制把 finalize_status 改成 `held_by_ai_policy`、`should_reply=false`，
    // 落到下面统一的 `!Approved` 拦截分支去写审计 / 取消任务 / 写 run log。
    //
    // 老库无 `operation_state_policies` 行 → `enforce_state_action_policy(None, _)`
    // fallthrough（向前兼容）；该入口不会绕过 outbox / idempotency。
    if matches!(finalize_status, GatewayStatusFinal::Approved) {
        let policy_opt = load_operation_state_policy_for_contact(
            state,
            &contact.workspace_id,
            final_decision.operation_state.as_deref().unwrap_or(""),
            &contact.wxid,
        )
        .await?;
        let action = classify_decision_action(&final_decision);
        if let Err(reason) = enforce_state_action_policy(policy_opt.as_ref(), action) {
            review.approved = false;
            review.final_review_status = "held_by_ai_policy".to_string();
            final_decision.should_reply = false;
            final_decision.autonomy_mode = "blocked".to_string();
            if !review.risks.iter().any(|r| r == "state_action_policy_blocked") {
                review.risks.push("state_action_policy_blocked".to_string());
            }
            finalize_status =
                GatewayStatusFinal::Held("held_by_ai_policy".to_string());
            write_event_for_account(
                state,
                &contact.account_id,
                Some(&contact.wxid),
                "state_action_policy_blocked",
                "blocked",
                &reason,
                Some(doc! {
                    "run_id": &run_id,
                    "action": action,
                    "operation_state": final_decision
                        .operation_state
                        .clone()
                        .unwrap_or_default(),
                    "reason": reason.clone(),
                }),
            )
            .await?;
        }

        // ── Phase A / A3：taxonomy 软闸 ──
        //
        // 校验 final_decision 上 LLM 给出的 customer_stage / intent_level 是否在
        // system_taxonomies 字典里：命中 active → 通过；命中 alias → 改写为
        // canonical id；deprecated → 仅 risks 追加；CandidateNew → upsert 候选
        // 队列供 admin review。任何 IO 故障静默跳过（best-effort），不阻塞 run。
        // 这是 CLAUDE.md 硬规则"unreviewed candidates must not block runs"的实现位。
        let cache = global_taxonomy_cache();
        // TTL 自愈：启动 warm_up 后若长期无 admin 写操作触发 invalidate，
        // 30s 后 find_or_load 自动 reload，防 cache 永远 stale。任何 IO 故障被
        // find_or_load 内部 log 后吞掉。
        cache.find_or_load(&state.db).await;
        let outcome = compute_taxonomy_guard_outcome(
            final_decision.customer_stage.as_deref(),
            final_decision.intent_level.as_deref(),
            &contact.account_id,
            &cache,
        );
        if let Some(canonical) = outcome.customer_stage_rewrite.clone() {
            final_decision.customer_stage = Some(canonical);
        }
        if let Some(canonical) = outcome.intent_level_rewrite.clone() {
            final_decision.intent_level = Some(canonical);
        }
        for risk in &outcome.risks {
            if !review.risks.iter().any(|r| r == risk) {
                review.risks.push(risk.clone());
            }
        }
        for (kind, raw) in &outcome.candidate_writes {
            if let Err(error) = taxonomy_upsert_candidate(
                &state.db,
                &contact.account_id,
                kind,
                raw,
                Some("user-ops decision path"),
                50,
            )
            .await
            {
                tracing::warn!(?error, kind = kind.as_str(), raw = %raw, "taxonomy upsert_candidate failed");
            }
        }
    }

    // ── R2 single-shot revision 控制流 ──
    //
    // 触发条件（design.md §4.5 / R2.3 / R2.4 / R2.8 / R2.9）：
    //   * `outcome.status == Approved`（finalize 未触发任何硬安全门，且
    //     `review.approved && final_decision.should_reply` 已在 finalize 内确认）；
    //   * `outcome.review.needs_revision == true`；
    //   * `outcome.review.should_hold == false`（hold 路径不 revise）；
    //   * `outcome.review.revision_direction.trim()` 非空；
    //   * 当前 RunBudget 未超额；
    //   * 单 run 内 `revision_attempted == false`（最多 1 次重试）。
    //
    // 二次 Reply Agent 调用走 30s timeout 控制；超时 / LLM 错误 → revision_failed。
    // 二次 review 仍 fail（`review_passed=false` 或 finalize 触发硬门）→
    // gateway_status="revision_failed" + should_reply=false。
    let mut revision_applied = false;
    let mut revision_reason = String::new();
    let mut pre_revision_summary: Option<String> = None;
    let mut post_revision_summary: Option<String> = None;
    let budget_exceeded_for_revision = current_run_budget()
        .map(|b| b.is_exceeded())
        .unwrap_or(false);

    // Phase D / D2：如果 finalize Approved 且 reviewer 没要求 revision，再用结构性
    // 风格指纹和 contact.last_outbound_style 比对一次。3/5 轴漂移 → 强制 single-shot
    // revision，向 last_outbound_style 风格靠拢。empty prev 视为首轮，跳过。
    if matches!(finalize_status, GatewayStatusFinal::Approved)
        && !review.needs_revision
        && !review.should_hold
        && final_decision.should_reply
    {
        let prev_style = contact
            .last_outbound_style
            .clone()
            .unwrap_or_default();
        if !prev_style.trim().is_empty() {
            let new_style =
                super::review::extract_outbound_style_fingerprint(&final_decision.reply_text);
            if super::review::style_diverged(&prev_style, &new_style) {
                let direction = format!(
                    "上一轮出站风格指纹为 [{}]，本轮草稿为 [{}]，二者结构差异较大；\
                     请保留本轮内容要点的同时，向上一轮风格靠拢（长度桶 / emoji / \
                     问句感叹密度 / 句末符号 / 段落数 至少 3 个轴对齐）。",
                    prev_style, new_style
                );
                review.needs_revision = true;
                review.revision_direction = direction.clone();
                if !review.risks.iter().any(|r| r == "style_diverged") {
                    review.risks.push("style_diverged".to_string());
                }
                write_event_for_account(
                    state,
                    &contact.account_id,
                    Some(&contact.wxid),
                    "style_consistency_revision_trigger",
                    "info",
                    &format!(
                        "style_diverged: prev={} new={}",
                        prev_style, new_style
                    ),
                    Some(doc! {
                        "run_id": &run_id,
                        "prev_style": &prev_style,
                        "new_style": &new_style,
                    }),
                )
                .await?;
            }
        }
    }

    let revision_decision =
        decide_revision(&finalize_status, &review, budget_exceeded_for_revision);
    match revision_decision {
        RevisionDecision::NotEligible => { /* 不进 R2 块 */ }
        RevisionDecision::Skip { reason, event } => {
            // R2.5 / R2.8：revisionDirection 空 / 预算超额 → 写事件 + 失败终态。
            review.approved = false;
            review.revision_applied = false;
            review.final_review_status = "revision_failed".to_string();
            final_decision.should_reply = false;
            let (reason_str, status) = derive_revision_failure(reason);
            finalize_status = status;
            revision_reason = reason_str;
            let summary = match event {
                "revision_skipped_invalid_direction" => {
                    "Review Agent 要求 revision 但 revisionDirection 为空，跳过本次 revision"
                }
                "revision_skipped_budget_exceeded" => "预算超额：跳过 single-shot revision",
                _ => "single-shot revision 跳过：未知原因",
            };
            write_event_for_account(
                state,
                &contact.account_id,
                Some(&contact.wxid),
                event,
                "blocked",
                summary,
                Some(doc! { "run_id": &run_id }),
            )
            .await?;
        }
        RevisionDecision::Proceed => {
            let revision_direction = review.revision_direction.trim().to_string();
            // R2.3 / R2.10：触发 1 次 revision，把 revisionDirection 透传
            // 给 Reply Agent，30s 超时控制。
            pre_revision_summary = Some(format!(
                "approved={} reply_text_len={} risks={:?} revisionDirection={}",
                review.approved,
                final_decision.reply_text.chars().count(),
                review.risks,
                revision_direction
            ));
            let revision_future = decide_reply_with_promote(
                state,
                &contact,
                &inbound,
                &recent_messages,
                &pending_tasks,
                playbook.as_ref(),
                domain_config.as_ref(),
                &runtime,
                &memory,
                &context_pack,
                &selected_chunks,
                &knowledge_route,
                Some(&revision_direction),
                Some(&run_id),
            );
            match tokio::time::timeout(std::time::Duration::from_secs(30), revision_future).await {
                Ok(Ok((mut revised_decision, revised_promote_risks))) => {
                    normalize_decision_state(&mut revised_decision, domain_config.as_ref());
                    normalize_decision_runtime(&mut revised_decision, &planner);
                    revised_decision.context_pack_version = Some(next_memory_card_version(&memory));
                    revised_decision.used_knowledge_ids =
                        route_used_knowledge_ids(&knowledge_route);

                    let second_review = review_decision(
                        state,
                        &contact,
                        &inbound,
                        &revised_decision,
                        playbook.as_ref(),
                        domain_config.as_ref(),
                        &runtime,
                        &memory,
                        &context_pack,
                        &selected_chunks,
                        &knowledge_route,
                        "full",
                        Some(&run_id),
                    )
                    .await?;

                    final_decision = revised_decision;
                    promote_risks = revised_promote_risks;

                    let second_outcome = finalize_review_for_send(
                        second_review,
                        &mut final_decision,
                        &runtime,
                        &contact,
                        &selected_chunks,
                        promote_risks.clone(),
                        inbound.content.as_str(),
                    );
                    let FinalizeOutcome {
                        review: second_finalized_review,
                        status: second_finalize_status,
                        pending_events: second_pending_events,
                    } = second_outcome;
                    review = second_finalized_review;
                    persist_finalize_pending_events(state, &contact, &second_pending_events)
                        .await?;

                    let second_passed = matches!(
                        second_finalize_status,
                        GatewayStatusFinal::Approved
                    ) && review_passed(&review, &runtime);

                    if second_passed {
                        // R2.3：revision_applied_approved
                        revision_applied = true;
                        review.revision_applied = true;
                        review.final_review_status =
                            "revision_applied_approved".to_string();
                        revision_reason = "revision_applied_approved".to_string();
                        finalize_status = GatewayStatusFinal::Approved;
                        post_revision_summary = Some(format!(
                            "approved=true reply_text_len={} risks={:?}",
                            final_decision.reply_text.chars().count(),
                            review.risks
                        ));
                    } else {
                        // R2.4：第二轮仍 fail → revision_failed
                        revision_applied = true;
                        review.revision_applied = true;
                        review.approved = false;
                        review.final_review_status = "revision_failed".to_string();
                        final_decision.should_reply = false;
                        let (reason_str, fallback_status) =
                            derive_revision_failure("revision_post_review_failed");
                        revision_reason = reason_str;
                        finalize_status = match second_finalize_status {
                            GatewayStatusFinal::Approved => fallback_status,
                            other => other,
                        };
                        post_revision_summary = Some(format!(
                            "approved=false reply_text_len={} risks={:?}",
                            final_decision.reply_text.chars().count(),
                            review.risks
                        ));
                    }
                }
                Ok(Err(err)) => {
                    // R2.11：LLM 不可解析 / 业务错误 → revision_failed
                    review.approved = false;
                    review.revision_applied = false;
                    review.final_review_status = "revision_failed".to_string();
                    final_decision.should_reply = false;
                    revision_applied = false;
                    let (reason_str, status) =
                        derive_revision_failure(&format!("revision_llm_error:{}", err));
                    revision_reason = reason_str;
                    finalize_status = status;
                    write_event_for_account(
                        state,
                        &contact.account_id,
                        Some(&contact.wxid),
                        "revision_llm_failure",
                        "blocked",
                        "Reply Agent revision 调用失败：JSON 解析或下游错误",
                        Some(doc! {
                            "run_id": &run_id,
                            "error": err.to_string(),
                        }),
                    )
                    .await?;
                }
                Err(_) => {
                    // R2.11：30s 超时 → revision_failed
                    review.approved = false;
                    review.revision_applied = false;
                    review.final_review_status = "revision_failed".to_string();
                    final_decision.should_reply = false;
                    revision_applied = false;
                    let (reason_str, status) =
                        derive_revision_failure("revision_llm_timeout_30s");
                    revision_reason = reason_str;
                    finalize_status = status;
                    write_event_for_account(
                        state,
                        &contact.account_id,
                        Some(&contact.wxid),
                        "revision_llm_failure",
                        "blocked",
                        "Reply Agent revision 调用超时（30s）",
                        Some(doc! {
                            "run_id": &run_id,
                            "latency_ms": 30000_i64,
                        }),
                    )
                    .await?;
                }
            }
        }
    }
    let _ = promote_risks; // 后续如需进一步审计可再消费

    // 同步把 finalize 阶段计算好的 final_review_status / revision_applied 字段
    // 写回 review struct，便于审计 / 落库（write_decision_review / write_agent_run_log
    // 都 serialize 这个 review）。
    if review.final_review_status.is_empty() {
        // 兜底：finalize 路径已设置 final_review_status；若空则用 finalize_status 兜底。
        review.final_review_status = finalize_status.final_review_status_str();
    }

    // ISSUE-001 (R12)：FollowUp 路径下，review 阶段（~3s）期间用户可能中途
    // 发新 inbound。原逻辑在此处 review-held 短路返回，导致 cancel_task 的 reason
    // 始终是 "finalize_review_blocked"，掩盖了"用户中途插话"这一真实信号。
    // 这里先用 last_inbound_at vs task.created_at 重算 context_changed，命中则
    // 把 finalize_status 改写为 BlockedSafetyGuard + reason 改写，让 cancel_task
    // / write_event 落库时显式标记 context_changed。
    let context_changed_followup_hit = match &trigger {
        AgentTrigger::FollowUp(task) => {
            let last_inbound_ms = inbound_marker_for_context_check(&contact)
                .map(|d| d.timestamp_millis());
            let task_created_ms = task.created_at.timestamp_millis();
            check_context_changed_followup_pure(last_inbound_ms, task_created_ms)
        }
        _ => false,
    };
    let context_changed_followup_reason: Option<&'static str> = if context_changed_followup_hit {
        Some("用户在跟进任务后已有新消息（review 阶段被覆盖），取消旧跟进")
    } else {
        None
    };

    // finalize 终态决定是否拦截发送：approved 路径继续走原有 send 逻辑；
    // 其它终态（held / blocked_*）一律 fail-closed（不发送、记录审计）。
    if !matches!(finalize_status, GatewayStatusFinal::Approved) {
        let (blocked_status, cancel_reason) =
            if let Some(reason) = context_changed_followup_reason {
                // ISSUE-001 (R12)：context_changed 抢先覆盖 gateway_status；
                // final_review_status 保持 finalize 计算值（10 项枚举内合法），
                // 但通过 review.risks 追加 "follow_up_context_changed" 标签，
                // 让审计 / observability 能看到这一 race 真实信号。
                if !review
                    .risks
                    .iter()
                    .any(|r| r == "follow_up_context_changed")
                {
                    review.risks.push("follow_up_context_changed".to_string());
                }
                ("context_changed".to_string(), reason)
            } else {
                (
                    finalize_status.gateway_status_str(),
                    "finalize_review_blocked",
                )
            };
        write_decision_review(
            state,
            &contact,
            &inbound,
            &final_decision,
            &review,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &precheck,
            &context_pack,
            &blocked_status,
            &knowledge_route,
            &run_id,
            &planner,
        )
        .await?;
        if let Some(task_id) = task_id {
            cancel_task(state, task_id, &blocked_status, cancel_reason).await?;
        }
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "blocked_review",
            &blocked_status,
            cancel_reason,
            Some(review_event_details(&review)),
        )
        .await?;
        write_agent_run_log_with_finalize(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            &blocked_status,
            &planner,
            doc! { "refreshed": should_refresh_context, "version": context_pack.get_i32("version").unwrap_or_default() },
            &knowledge_route,
            to_document(&final_decision).unwrap_or_default(),
            to_document(&review).unwrap_or_default(),
            to_document(&precheck).unwrap_or_default(),
            None,
            FinalizeRunLogFields {
                final_review_status: review.final_review_status.clone(),
                autonomy_mode: final_decision.autonomy_mode.clone(),
                conversation_mode: final_decision.conversation_mode.clone(),
                conversation_mode_reason: final_decision.conversation_mode_reason.clone(),
                revision_applied,
                revision_reason: revision_reason.clone(),
                pre_revision_summary: pre_revision_summary.clone(),
                post_revision_summary: post_revision_summary.clone(),
                self_critique: non_empty_option(&Some(final_decision.self_critique.clone())),
                source_event_id: envelope_source_event_id.clone(),
                source_kind: envelope_source_kind.clone(),
            },
        )
        .await?;
        return Ok(());
    }

    let final_precheck = precheck_send_gateway(state, &contact, &trigger, &runtime).await?;
    if final_decision.should_reply && !final_precheck.allowed {
        if let Some(task_id) = task_id {
            cancel_task(
                state,
                task_id,
                &final_precheck.status,
                &final_precheck.reason,
            )
            .await?;
        }
        write_decision_review(
            state,
            &contact,
            &inbound,
            &final_decision,
            &review,
            playbook.as_ref(),
            domain_config.as_ref(),
            &runtime,
            &final_precheck,
            &context_pack,
            "gateway_blocked",
            &knowledge_route,
            &run_id,
            &planner,
        )
        .await?;
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "gateway_blocked",
            &final_precheck.status,
            &final_precheck.reason,
            Some(to_document(&final_precheck).unwrap_or_default()),
        )
        .await?;
        write_agent_run_log_with_finalize(
            state,
            &contact,
            &run_id,
            trigger.kind(),
            "gateway_blocked",
            &planner,
            doc! { "refreshed": should_refresh_context, "version": context_pack.get_i32("version").unwrap_or_default() },
            &knowledge_route,
            to_document(&final_decision).unwrap_or_default(),
            to_document(&review).unwrap_or_default(),
            to_document(&final_precheck).unwrap_or_default(),
            None,
            FinalizeRunLogFields {
                final_review_status: review.final_review_status.clone(),
                autonomy_mode: final_decision.autonomy_mode.clone(),
                conversation_mode: final_decision.conversation_mode.clone(),
                conversation_mode_reason: final_decision.conversation_mode_reason.clone(),
                revision_applied,
                revision_reason: revision_reason.clone(),
                pre_revision_summary: pre_revision_summary.clone(),
                post_revision_summary: post_revision_summary.clone(),
                self_critique: non_empty_option(&Some(final_decision.self_critique.clone())),
                source_event_id: envelope_source_event_id.clone(),
                source_kind: envelope_source_kind.clone(),
            },
        )
        .await?;
        return Ok(());
    }

    if final_decision.should_reply && !final_decision.reply_text.trim().is_empty() {
        if let Some(task_id) = task_id {
            // W4 / Task 5.5：发送改异步走 outbox，把 task 状态推进为
            // `outbox_enqueued` 而不是 `sent`；真正 `sent` 由 dispatcher 在
            // MCP 成功后更新（dispatcher 反向通道见 5.4）。
            crate::models::assert_agent_task_status_valid("outbox_enqueued");
            state
                .db
                .tasks()
                .update_one(
                    doc! { "_id": task_id },
                    doc! { "$set": { "status": "outbox_enqueued", "gateway_status": "outbox_enqueued", "updated_at": DateTime::now() } },
                    None,
                )
                .await?;
        }
    }

    apply_agent_updates(state, &contact, &final_decision, &runtime).await?;
    apply_operating_memory_update(
        state,
        &contact,
        &memory,
        &final_decision,
        &context_pack,
        should_refresh_context,
        &run_id,
    )
    .await?;
    let decision_review_id = write_decision_review(
        state,
        &contact,
        &inbound,
        &final_decision,
        &review,
        playbook.as_ref(),
        domain_config.as_ref(),
        &runtime,
        &final_precheck,
        &context_pack,
        if final_decision.should_reply {
            "sent"
        } else {
            "no_reply"
        },
        &knowledge_route,
        &run_id,
        &planner,
    )
    .await?;
    write_knowledge_usage_log(
        state,
        &contact,
        &final_decision,
        &review,
        &knowledge_route,
        review_passed(&review, &runtime),
        &run_id,
    )
    .await?;
    if !final_decision.should_reply {
        if let Some(task_id) = task_id {
            cancel_task(state, task_id, "no_reply", "Agent 判断无需触达").await?;
        }
    }
    let details = build_decision_event_details(&final_decision, playbook.as_ref(), &review);
    write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "agent_reply",
        "success",
        if final_decision.should_reply {
            "Agent 已生成回复，已入队 outbox 等待发送"
        } else {
            "Agent 判断无需回复"
        },
        Some(details),
    )
    .await?;
    write_agent_run_log_with_finalize(
        state,
        &contact,
        &run_id,
        trigger.kind(),
        if final_decision.should_reply { "outbox_enqueued" } else { "no_reply" },
        &planner,
        doc! { "refreshed": should_refresh_context, "version": context_pack.get_i32("version").unwrap_or_default() },
        &knowledge_route,
        to_document(&final_decision).unwrap_or_default(),
        to_document(&review).unwrap_or_default(),
        to_document(&final_precheck).unwrap_or_default(),
        None,
        FinalizeRunLogFields {
            final_review_status: review.final_review_status.clone(),
            autonomy_mode: final_decision.autonomy_mode.clone(),
            conversation_mode: final_decision.conversation_mode.clone(),
            conversation_mode_reason: final_decision.conversation_mode_reason.clone(),
            revision_applied,
            revision_reason: revision_reason.clone(),
            pre_revision_summary: pre_revision_summary.clone(),
            post_revision_summary: post_revision_summary.clone(),
            self_critique: non_empty_option(&Some(final_decision.self_critique.clone())),
            source_event_id: envelope_source_event_id.clone(),
            source_kind: envelope_source_kind.clone(),
        },
    )
    .await?;

    // W4 / Task 5.5：决策落地 = outbox 写入。仅在 finalReviewStatus ∈
    // {approved, revision_applied_approved} 且 should_reply=true 时入队；
    // 真正发送由 dispatcher worker 异步抢占（atomic claim + lease）后通过
    // `send_outbound_message` 调 MCP（spec R13 / requirements §F）。
    let final_status = review.final_review_status.as_str();
    let outbox_eligible = final_decision.should_reply
        && !final_decision.reply_text.trim().is_empty()
        && (final_status == "approved" || final_status == "revision_applied_approved");
    if outbox_eligible {
        let source_event_id = match &trigger {
            AgentTrigger::Inbound(msg) => msg.message_id.clone().unwrap_or_default(),
            AgentTrigger::FollowUp(task) => {
                task.id.map(|id| id.to_hex()).unwrap_or_default()
            }
        };
        let enqueue_req = EnqueueRequest {
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            run_id: run_id.clone(),
            decision_id: Some(decision_review_id),
            source_event_id,
            source_kind: trigger.kind().to_string(),
            content: final_decision.reply_text.clone(),
            max_attempts: 3,
        };
        match outbox_enqueue(state, enqueue_req).await {
            Ok(EnqueueOutcome::Created { outbox_id, .. }) => {
                tracing::info!(
                    %run_id,
                    %outbox_id,
                    contact_wxid = %contact.wxid,
                    "outbox enqueued"
                );
                let _ = state
                    .db
                    .agent_run_logs()
                    .update_one(
                        doc! { "run_id": &run_id },
                        doc! { "$set": { "outbox_status": "pending" } },
                        None,
                    )
                    .await;
            }
            Ok(EnqueueOutcome::IdempotentSkip { idempotency_key }) => {
                tracing::info!(
                    %run_id,
                    %idempotency_key,
                    contact_wxid = %contact.wxid,
                    "outbox enqueue idempotent skip"
                );
            }
            Err(err) => {
                tracing::error!(?err, %run_id, "outbox enqueue failed");
                return Err(err.into());
            }
        }
    }
    Ok(())
}

/// Only callable from outbox_dispatcher (W4 / Task 5.4) and the legacy in-line
/// gateway send paths during the W4 transition. Once 5.5 lands the gateway
/// will route exclusively through outbox enqueue and the in-line callers will
/// be removed.
pub(crate) async fn send_outbound_message(
    state: &AppState,
    contact: &Contact,
    content: &str,
    extra_raw: Option<Document>,
) -> AppResult<serde_json::Value> {
    let response = mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_text",
        json!({
            "recipient": contact.wxid,
            "content": content
        }),
    )
    .await?;
    let message_id = response
        .get("newMsgId")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let mut raw = to_document(&response).unwrap_or_default();
    if let Some(extra_raw) = extra_raw {
        raw.insert("wechatagent", Bson::Document(extra_raw));
    }
    let now = DateTime::now();
    state
        .db
        .messages()
        .insert_one(
            ConversationMessage {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                message_id,
                dedupe_key: None,
                direction: MessageDirection::Outbound,
                content: content.to_string(),
                raw: Some(raw),
                created_at: now,
            },
            None,
        )
        .await?;
    // 用 aggregation pipeline 把 last_outbound_at / last_agent_run_at / updated_at
    // 设为 now，并把 last_message_at 设成 max(last_inbound_at, now)，
    // 不改 last_inbound_at（出站不应推进"用户最后一次说话"的时间）。
    // Phase D / D2：同步把本次出站文本的风格指纹写入 last_outbound_style，
    // 供下一轮 reviewer 做 style_diverged 判定。
    let style_fingerprint =
        super::review::extract_outbound_style_fingerprint(content);
    let pipeline: Vec<Document> = vec![doc! {
        "$set": {
            "last_outbound_at": now,
            "last_agent_run_at": now,
            "updated_at": now,
            "last_message_at": {
                "$max": ["$last_inbound_at", now]
            },
            "last_outbound_style": style_fingerprint,
        }
    }];
    state
        .db
        .contacts()
        .update_one(doc! { "_id": contact.id }, pipeline, None)
        .await?;
    Ok(response)
}

pub(crate) fn trigger_message(
    contact: &Contact,
    trigger: &AgentTrigger<'_>,
) -> ConversationMessage {
    match trigger {
        AgentTrigger::Inbound(message) => (*message).clone(),
        AgentTrigger::FollowUp(task) => ConversationMessage {
            id: None,
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: format!(
                "系统跟进任务到期，请重新判断是否适合主动触达。任务内容：{}",
                task.content
            ),
            raw: Some(doc! {
                "trigger": "follow_up_task",
                "taskId": task.id.map(|id| id.to_hex()).unwrap_or_default()
            }),
            created_at: DateTime::now(),
        },
    }
}

/// 判定跟进任务 `context_changed` 时使用的"用户最后一次说话"时间戳。
///
/// 优先取 `last_inbound_at`（HP-2 拆分后的精确字段），缺失时降级到
/// `last_message_at`（migration 未跑完或老数据兼容），保证拆分前后行为一致。
pub(crate) fn inbound_marker_for_context_check(contact: &Contact) -> Option<DateTime> {
    contact.last_inbound_at.or(contact.last_message_at)
}

pub(crate) async fn precheck_send_gateway(
    state: &AppState,
    contact: &Contact,
    trigger: &AgentTrigger<'_>,
    runtime: &UserRuntimeParameters,
) -> AppResult<SendGatewayResult> {
    if contact.agent_status != AgentStatus::Managed {
        return Ok(blocked("not_managed", "好友未纳入 Agent 运营"));
    }
    if let Some(cooldown_until) = contact.cooldown_until {
        if cooldown_until.timestamp_millis() > DateTime::now().timestamp_millis() {
            return Ok(blocked("cooldown", "用户处于冷却期"));
        }
    }
    if let Some(policy_block) = precheck_operation_policy(state, contact).await? {
        return Ok(policy_block);
    }
    if let Some(last_run) = contact.last_agent_run_at {
        let elapsed = DateTime::now().timestamp_millis() - last_run.timestamp_millis();
        if elapsed < runtime.min_reply_interval_seconds * 1000 {
            return Ok(blocked("rate_limited", "短时间内已触达，跳过本次自动发送"));
        }
    }
    if daily_touch_count(state, contact).await? >= runtime.max_daily_touches {
        return Ok(blocked("daily_limit", "已达到每日触达上限"));
    }
    if let AgentTrigger::FollowUp(task) = trigger {
        if let Some(expires_at) = task.expires_at {
            if expires_at.timestamp_millis() < DateTime::now().timestamp_millis() {
                return Ok(blocked("expired", "跟进任务已过期"));
            }
        }
        // 用 last_inbound_at 判定 context_changed；老数据若 last_inbound_at 还没回填
        // （migration 未跑或回填中），降级使用 last_message_at 兼容。
        if let Some(last_inbound) = inbound_marker_for_context_check(contact) {
            if last_inbound.timestamp_millis() > task.created_at.timestamp_millis() {
                return Ok(blocked(
                    "context_changed",
                    "用户在跟进任务后已有新消息，取消旧跟进",
                ));
            }
        }
    }
    Ok(SendGatewayResult {
        allowed: true,
        status: "allowed".to_string(),
        reason: "发送网关通过".to_string(),
        policy_blocks: Vec::new(),
        run_mode: "live".to_string(),
        message_id: None,
    })
}

async fn precheck_operation_policy(
    state: &AppState,
    contact: &Contact,
) -> AppResult<Option<SendGatewayResult>> {
    if contact.operation_policy.is_empty() {
        return Ok(None);
    }
    if let Some(until) = doc_string(&contact.operation_policy, "cooldownUntil")
        .and_then(|value| DateTime::parse_rfc3339_str(&value).ok())
    {
        if until.timestamp_millis() > DateTime::now().timestamp_millis() {
            return Ok(Some(blocked(
                "policy_cooldown",
                "联系人运营策略要求冷却，暂不主动触达",
            )));
        }
    }
    let consecutive_outbounds = consecutive_outbound_count(state, contact).await?;
    if doc_bool(
        &contact.operation_policy,
        "requireUserReplyBeforeNextOutbound",
    ) && consecutive_outbounds > 0
    {
        return Ok(Some(blocked(
            "policy_wait_user_reply",
            "联系人运营策略要求等用户回复后再触达",
        )));
    }
    let max_outbounds = doc_i64(
        Some(&contact.operation_policy),
        "maxConsecutiveAgentOutbounds",
        -1,
    );
    if max_outbounds >= 0 {
        if consecutive_outbounds >= max_outbounds {
            return Ok(Some(blocked(
                "policy_consecutive_limit",
                "联系人运营策略限制连续主动触达次数",
            )));
        }
    }
    Ok(None)
}

async fn consecutive_outbound_count(state: &AppState, contact: &Contact) -> AppResult<i64> {
    let mut cursor = state
        .db
        .messages()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(20)
                .build(),
        )
        .await?;
    let mut count = 0;
    while let Some(message) = cursor.try_next().await? {
        match message.direction {
            MessageDirection::Outbound => count += 1,
            MessageDirection::Inbound => break,
        }
    }
    Ok(count)
}

pub(crate) fn blocked(status: &str, reason: &str) -> SendGatewayResult {
    SendGatewayResult {
        allowed: false,
        status: status.to_string(),
        reason: reason.to_string(),
        policy_blocks: vec![status.to_string()],
        run_mode: "live".to_string(),
        message_id: None,
    }
}

async fn daily_touch_count(state: &AppState, contact: &Contact) -> AppResult<i64> {
    let since = DateTime::from_millis(DateTime::now().timestamp_millis() - 24 * 60 * 60 * 1000);
    state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "direction": "outbound",
                "created_at": { "$gte": since }
            },
            None,
        )
        .await
        .map(|count| count as i64)
        .map_err(AppError::from)
}

async fn cancel_task(
    state: &AppState,
    task_id: ObjectId,
    status: &str,
    reason: &str,
) -> AppResult<()> {
    crate::models::assert_agent_task_status_valid("cancelled");
    state
        .db
        .tasks()
        .update_one(
            doc! { "_id": task_id },
            doc! {
                "$set": {
                    "status": "cancelled",
                    "gateway_status": status,
                    "cancel_reason": reason,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(())
}

/// P2-4：判定 operation_state 是否发生有效迁移。
///
/// * 返回 `Some((prior_normalized, next_normalized))` 表示需要写一条
///   `agent.operation_state_transitioned` stage event；
/// * 返回 `None` 表示无变化（同状态 / 新状态空 / 仅大小写空白差异）。
///
/// `prior` 取自 contact 当前 doc，`next` 取自 LLM 决策 (`decision.operation_state`)。
/// 二者均做 `trim` 归一化；prior 缺失视为空串。
pub(crate) fn detect_state_transition<'a>(
    prior: Option<&'a str>,
    next: Option<&'a str>,
) -> Option<(String, String)> {
    let next_norm = next.map(str::trim).unwrap_or("");
    if next_norm.is_empty() {
        return None;
    }
    let prior_norm = prior.map(str::trim).unwrap_or("");
    if prior_norm == next_norm {
        return None;
    }
    Some((prior_norm.to_string(), next_norm.to_string()))
}

/// 画像/标签/记忆**写侧抖动**的纯观测报告（第一轮：体检量化，不改写库逻辑）。
///
/// 背景：`apply_agent_updates` 对 contact 画像是"present 即整体覆盖"——`tags` 用
/// LLM 单轮输出整体替换累积标签集、`memory_summary` 朴素 append、stage/intent 直接
/// 覆盖，全程无置信门 / 无滞后 / 无"已建立画像 vs 单轮弱信号"对比。这会让一句弱信号
/// （如"在吗"）就推翻长期累积的高置信画像。本结构只**量化**这种抖动严重度，供 CI
/// 真模型多轮跑积累数据后决定下一轮是否升级结构化 TagEntry + 置信门 / union / cap。
///
/// 纯审计：不参与任何写库 / 发送决策，仅用于 `agent.profile_churn_observed` 事件与
/// 单测，定位仿 [`detect_state_transition`]（纯函数、可单测、零副作用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileChurnReport {
    /// new 有、old 无的标签数。
    pub tags_added: usize,
    /// old 有、new 无的标签数——整体覆盖导致的"丢标签"风险，本轮重点量化。
    pub tags_removed: usize,
    /// 标签净变化（new.len - old.len），负数表示净丢失。
    pub tags_net: i64,
    /// stage 翻转：old 与 new 都非空且不同 → (old, new)。old 空（首次画像）不算翻转。
    pub stage_flipped: Option<(String, String)>,
    /// intent 翻转：同 stage 语义。
    pub intent_flipped: Option<(String, String)>,
    /// append 前 summary 长度（字符数）。
    pub summary_len_before: usize,
    /// append 后 summary 长度（字符数）——量化无界增长。
    pub summary_len_after: usize,
    /// 是否值得记审计：丢标签 OR stage 翻转 OR intent 翻转 OR summary 超软水位。
    /// 用于事件噪声门——无抖动时不发，仿 operation_state 迁移事件的同状态不发。
    pub notable: bool,
}

/// summary append 后超过此字符数即视为"无界增长"信号之一，计入 `notable`。
const PROFILE_SUMMARY_SOFT_CAP: usize = 2000;

/// 计算单轮自动回复对已建立画像造成的抖动（纯函数，无 IO）。
///
/// 入参语义与 `apply_agent_updates` 的写侧保持一致：
/// * `old_*` 取自 contact 当前 doc；
/// * `new_tags` 仅在 `decision.tags` 非空时才与 old 比对（与"非空才写"对齐）；空时
///   视作"本轮未给标签"，不计 added/removed；
/// * `old_stage`/`new_stage`、`old_intent`/`new_intent` 经 trim 归一，空当作未知；
/// * `appended_update` = `decision.memory_update`（已 trim 非空才进来），summary
///   长度按"existing + 换行 + update"（与 L1864-1870 的 append 一致）估算。
pub(crate) fn compute_profile_churn(
    old_tags: &[String],
    new_tags: &[String],
    old_stage: Option<&str>,
    new_stage: Option<&str>,
    old_intent: Option<&str>,
    new_intent: Option<&str>,
    old_summary: Option<&str>,
    appended_update: &str,
) -> ProfileChurnReport {
    // 标签比对仅在本轮确实给了标签时计入（new 空 = 未更新，不算丢标签）。
    let (tags_added, tags_removed, tags_net) = if new_tags.is_empty() {
        (0usize, 0usize, 0i64)
    } else {
        let added = new_tags
            .iter()
            .filter(|t| !old_tags.iter().any(|o| o == *t))
            .count();
        let removed = old_tags
            .iter()
            .filter(|o| !new_tags.iter().any(|t| t == *o))
            .count();
        let net = new_tags.len() as i64 - old_tags.len() as i64;
        (added, removed, net)
    };

    let stage_flipped = flip_of(old_stage, new_stage);
    let intent_flipped = flip_of(old_intent, new_intent);

    let summary_len_before = old_summary.map(str::len).unwrap_or(0);
    let trimmed_update = appended_update.trim();
    let summary_len_after = if trimmed_update.is_empty() {
        summary_len_before
    } else if summary_len_before == 0 {
        trimmed_update.len()
    } else {
        // 与写侧 `format!("{}\n{}", existing, update)` 一致：+1 为换行符。
        summary_len_before + 1 + trimmed_update.len()
    };

    let notable = tags_removed > 0
        || stage_flipped.is_some()
        || intent_flipped.is_some()
        || summary_len_after > PROFILE_SUMMARY_SOFT_CAP;

    ProfileChurnReport {
        tags_added,
        tags_removed,
        tags_net,
        stage_flipped,
        intent_flipped,
        summary_len_before,
        summary_len_after,
        notable,
    }
}

/// 翻转判定：old 与 new 都非空（trim 后）且不同 → Some((old, new))；否则 None。
/// old 空表示首次建立该维度，不算翻转。
fn flip_of(old: Option<&str>, new: Option<&str>) -> Option<(String, String)> {
    let old_norm = old.map(str::trim).unwrap_or("");
    let new_norm = new.map(str::trim).unwrap_or("");
    if old_norm.is_empty() || new_norm.is_empty() || old_norm == new_norm {
        None
    } else {
        Some((old_norm.to_string(), new_norm.to_string()))
    }
}

/// 逐消息自动回复路径的标签累积上限。union 后超过此数时，本轮新增的溢出标签暂不并入
/// （保留已累积画像），真正的裁剪 / 去重 / 冲突消解交给有版本锁的 memory consolidation
/// 路径，而非每条消息的写侧——逐消息路径保守优先。
const TAGS_PER_MESSAGE_CAP: usize = 16;

/// 标签写侧 union + cap（纯函数，无 IO）：把本轮 `new` 标签**只增不减**地并入已累积
/// `old`，去重保序，封顶 `cap`。取代旧的整体覆盖写法——
/// [[cautious-profiling]] 红线的**结构层**防御：即使 LLM 单轮 `decision.tags` 漏掉已
/// 累积标签或贴情景标签，结构层也保证长期画像不被一句弱信号抹平。
///
/// 语义：
/// * 先保留全部 `old`（累积画像优先），再追加 `new` 中 old 未含的标签；
/// * 封顶时保留靠前（累积）标签、丢弃溢出新增——宁可不更新，不要误抹；删除 / 替换交给
///   consolidation（有版本锁 / 去重 / 冲突追踪），不在逐消息路径做。
///
/// 定位仿 [`compute_profile_churn`]：纯函数、可确定性单测、零副作用。注意 churn 探针仍按
/// `decision.tags` **原始单轮意图**量化（不看 merge 结果），故结构层修复后 churn 仍能独立
/// 反映 LLM 单轮是否还想丢标签 / 贴情景标签。
pub(crate) fn merge_tags_union_capped(old: &[String], new: &[String], cap: usize) -> Vec<String> {
    let mut merged: Vec<String> = Vec::with_capacity(old.len() + new.len());
    for tag in old.iter().chain(new.iter()) {
        if !merged.iter().any(|m| m == tag) {
            merged.push(tag.clone());
        }
    }
    if merged.len() > cap {
        merged.truncate(cap);
    }
    merged
}

/// 逐消息短期记忆（memory_summary）的保留行数上限。超过时丢弃最旧的行——记忆偏好"保新"，
/// 与 [`merge_tags_union_capped`] 的"保已累积"方向相反：标签是长期画像资产（误删代价高，保旧），
/// 短期 memory_summary 是滚动上下文（旧行已被 consolidation 吸收进 memoryCard，保新更有信息量）。
const MEMORY_SUMMARY_MAX_LINES: usize = 12;
/// memory_summary 字节软上限。封顶时从最旧行开始整行丢弃直到落到上限内，避免逐字符截断切碎多字节中文。
const MEMORY_SUMMARY_MAX_BYTES: usize = 1200;

/// 短期记忆写侧去重 + cap（纯函数，无 IO）：把本轮 `update` 追加到 `existing`，
/// **按整行去重**（已存在的行不重复追加）并按行数 / 字节双重封顶（超限丢最旧行）。
/// 取代旧的 naive `format!("{existing}\n{update}")` 无界 append——
/// [[cautious-profiling]] 第 3 点的**结构层**修复：即使 consolidation 长时间不介入，
/// 逐消息路径自身也不再无界增长 / 不再堆叠重复行。
///
/// 语义：
/// * `update` 自身可能多行；逐行追加，已在结果中出现过的行（trim 后逐字节相等）跳过；
/// * 追加后若超过 `max_lines` 或 `max_bytes`，从**最旧**行开始整行丢弃（保新）直到两上限都满足；
/// * 真正的语义压缩 / 冲突消解仍交给有版本锁的 memory consolidation（memoryCard 路径），
///   逐消息路径只做"防无界 + 防重复"的保守封顶。
pub(crate) fn merge_memory_summary_dedup_capped(
    existing: &str,
    update: &str,
    max_lines: usize,
    max_bytes: usize,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in existing.lines().chain(update.lines()) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !lines.iter().any(|existing_line| existing_line == trimmed) {
            lines.push(trimmed.to_string());
        }
    }
    while lines.len() > max_lines {
        lines.remove(0);
    }
    while lines.len() > 1 && lines.iter().map(|l| l.len() + 1).sum::<usize>() > max_bytes {
        lines.remove(0);
    }
    lines.join("\n")
}

async fn apply_agent_updates(
    state: &AppState,
    contact: &Contact,
    decision: &AgentDecision,
    runtime: &UserRuntimeParameters,
) -> AppResult<()> {
    let mut set_doc = doc! {
        "updated_at": DateTime::now(),
        "last_agent_run_at": DateTime::now(),
    };

    if let Some(profile) = &decision.profile_update {
        set_doc.insert("agent_profile", to_document(profile)?);
    }
    if !decision.tags.is_empty() {
        // [[cautious-profiling]] 结构层修复（Phase B Round 2）：旧写法 `set_doc.insert("tags",
        // to_bson_array(&decision.tags))` 是**整体覆盖**——LLM 单轮只要给非空 tags 就把累积画像
        // 整列替换，无 union / 无置信门 / 无情景标签隔离。情景压力下 LLM 倾向输出本轮情景化标签
        // （实测对抗压力弧贴 "对抗测试"），直接覆写成持久画像。改为 union + cap：只增不减地并入，
        // 保证一句弱信号无法抹平长期标签。删除 / 替换 / 冲突消解交给有版本锁的 consolidation
        // 路径，逐消息写侧保守优先（宁可不更新，不要误抹）。churn 探针仍按原始 decision.tags 量化。
        let merged = merge_tags_union_capped(&contact.tags, &decision.tags, TAGS_PER_MESSAGE_CAP);
        set_doc.insert("tags", to_bson_array(&merged));
    }
    if let Some(value) = non_empty_option(&decision.customer_stage) {
        // 旧 customer_stage 字段已删除，统一写入 domain_attributes 容器。
        let mut attrs = contact.domain_attributes.clone().unwrap_or_default();
        attrs.insert("customer_stage", value);
        set_doc.insert("domain_attributes", attrs);
        set_doc.insert("domain_attributes_updated_at", DateTime::now());
    }
    if let Some(value) = non_empty_option(&decision.intent_level) {
        let mut attrs = set_doc
            .get_document("domain_attributes")
            .ok()
            .cloned()
            .or_else(|| contact.domain_attributes.clone())
            .unwrap_or_default();
        attrs.insert("intent_level", value);
        set_doc.insert("domain_attributes", attrs);
        set_doc.insert("domain_attributes_updated_at", DateTime::now());
    }
    if let Some(value) = non_empty_option(&decision.last_commitment) {
        // M2：把 LLM 输出的字符串承诺升级为结构化 CommitmentEntry 追加到
        // commitments 数组（cap 8），让 Planner::scan_commitments 在 due_at
        // 到期/快到期时能 emit follow_up。当前 LLM JSON contract 仍输出
        // 单字符串 last_commitment（无 due_at），后续升级 prompt 后可由
        // RawAgentDecision 直接吃 commitments 数组。
        let mut commitments: Vec<crate::models::CommitmentRepr> = contact.commitments.clone();
        let already_present = commitments.iter().any(|c| c.text() == value.as_str());
        if !already_present {
            commitments.push(crate::models::CommitmentRepr::Structured(
                crate::models::CommitmentEntry::from_plain_text(value.clone()),
            ));
            if commitments.len() > 8 {
                let drop = commitments.len() - 8;
                commitments.drain(0..drop);
            }
        }
        let bson_commitments = mongodb::bson::to_bson(&commitments).unwrap_or(mongodb::bson::Bson::Array(Vec::new()));
        set_doc.insert("commitments", bson_commitments);
    }
    if let Some(value) = non_empty_option(&decision.follow_up_policy) {
        set_doc.insert("follow_up_policy", value);
    }
    if let Some(value) = non_empty_option(&decision.operation_state) {
        set_doc.insert("operation_state", value);
        set_doc.insert("operation_state_updated_at", DateTime::now());
    }
    if let Some(value) = non_empty_option(&decision.operation_state_reason) {
        set_doc.insert("operation_state_reason", value);
    }
    if let Some(value) = decision.operation_state_confidence {
        set_doc.insert("operation_state_confidence", value);
    }
    if let Some(value) = decision
        .cooldown_until
        .as_deref()
        .and_then(|value| DateTime::parse_rfc3339_str(value).ok())
    {
        set_doc.insert("cooldown_until", value);
    }
    if !decision.profile_attributes.is_empty() {
        set_doc.insert("profile_attributes", decision.profile_attributes.clone());
    }
    if !decision.tags.is_empty()
        || decision.customer_stage.is_some()
        || decision.intent_level.is_some()
        || decision.last_commitment.is_some()
        || decision.follow_up_policy.is_some()
        || !decision.profile_attributes.is_empty()
    {
        set_doc.insert("profile_updated_at", DateTime::now());
    }
    if !decision.memory_update.trim().is_empty() {
        let existing = contact.memory_summary.clone().unwrap_or_default();
        // [[cautious-profiling]] 第 3 点结构层修复（Phase B Round 3）：旧写法是 naive
        // `format!("{existing}\n{update}")`，无去重无 cap = consolidation 不介入时无界增长且会
        // 堆叠重复行。改为按行去重 + 行数/字节双封顶（保新丢旧），与 tags union+cap 同源的写侧严谨化。
        let merged = merge_memory_summary_dedup_capped(
            &existing,
            &decision.memory_update,
            MEMORY_SUMMARY_MAX_LINES,
            MEMORY_SUMMARY_MAX_BYTES,
        );
        set_doc.insert("memory_summary", merged);
    }

    state
        .db
        .contacts()
        .update_one(doc! { "_id": contact.id }, doc! { "$set": set_doc }, None)
        .await?;

    // 画像写侧抖动观测（第一轮：体检量化，不改写库逻辑）。
    // 用 contact 写库前的现状 vs 本轮 decision 计算 churn，仅在 notable（丢标签 /
    // stage 翻转 / intent 翻转 / summary 超软水位）时写一条 `agent.profile_churn_observed`
    // 审计事件——仿 operation_state 迁移事件的噪声门，无抖动不发避免每条消息刷屏。
    // 纯审计：不改任何写库内容，供 CI 真模型多轮跑积累"一句弱信号推翻已建立画像"的频率。
    let old_stage = contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("customer_stage").ok());
    let old_intent = contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("intent_level").ok());
    let churn = compute_profile_churn(
        &contact.tags,
        &decision.tags,
        old_stage,
        decision.customer_stage.as_deref(),
        old_intent,
        decision.intent_level.as_deref(),
        contact.memory_summary.as_deref(),
        &decision.memory_update,
    );
    if churn.notable {
        let stage_flip = churn
            .stage_flipped
            .as_ref()
            .map(|(o, n)| format!("{o} → {n}"))
            .unwrap_or_default();
        let intent_flip = churn
            .intent_flipped
            .as_ref()
            .map(|(o, n)| format!("{o} → {n}"))
            .unwrap_or_default();
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "agent.profile_churn_observed",
            "observed",
            &format!(
                "profile churn: tags +{}/-{} (net {}), stage[{}] intent[{}], summary {}→{}",
                churn.tags_added,
                churn.tags_removed,
                churn.tags_net,
                stage_flip,
                intent_flip,
                churn.summary_len_before,
                churn.summary_len_after,
            ),
            Some(doc! {
                "tags_added": churn.tags_added as i64,
                "tags_removed": churn.tags_removed as i64,
                "tags_net": churn.tags_net,
                "stage_flip": stage_flip,
                "intent_flip": intent_flip,
                "summary_len_before": churn.summary_len_before as i64,
                "summary_len_after": churn.summary_len_after as i64,
            }),
        )
        .await?;
    }

    // P2-4：operation_state 发生迁移时写一条 stage event，便于 staleness /
    // funnel / dashboard 复盘。同状态或新状态为空时不发，避免噪声。
    if let Some((prior, next)) = detect_state_transition(
        contact.operation_state.as_deref(),
        decision.operation_state.as_deref(),
    ) {
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "agent.operation_state_transitioned",
            "transitioned",
            &format!("operation_state {prior} → {next}"),
            Some(doc! {
                "prior_state": &prior,
                "next_state": &next,
                "reason": decision
                    .operation_state_reason
                    .clone()
                    .unwrap_or_default(),
                "confidence": decision.operation_state_confidence.unwrap_or(0),
            }),
        )
        .await?;
    }

    if let Some(follow_up) = &decision.follow_up {
        if follow_up.needed && !follow_up.content.trim().is_empty() {
            if pending_follow_up_count(state, contact).await? < runtime.max_pending_follow_ups {
                if let Some(run_at) = parse_rfc3339_to_bson(&follow_up.run_at) {
                    let expires_at = DateTime::from_millis(
                        run_at.timestamp_millis()
                            + runtime.follow_up_expires_hours * 60 * 60 * 1000,
                    );
                    state
                        .db
                        .tasks()
                        .insert_one(
                            AgentTask {
                                id: None,
                                workspace_id: contact.workspace_id.clone(),
                                account_id: contact.account_id.clone(),
                                contact_wxid: contact.wxid.clone(),
                                kind: "follow_up".to_string(),
                                run_at,
                                expires_at: Some(expires_at),
                                content: follow_up.content.clone(),
                                status: "pending".to_string(),
                                source_decision_id: None,
                                review_required: true,
                                attempt_count: 0,
                                max_attempts: 3,
                                next_retry_at: None,
                                gateway_status: None,
                                cancel_reason: None,
                                error: None,
                                claimed_at: None,
                                claim_recovery_count: 0,
                                created_at: DateTime::now(),
                                updated_at: DateTime::now(),
                            },
                            None,
                        )
                        .await?;
                }
            }
        }
    }
    Ok(())
}

async fn apply_operating_memory_update(
    state: &AppState,
    contact: &Contact,
    memory: &crate::models::OperatingMemory,
    decision: &AgentDecision,
    context_pack: &Document,
    _context_refreshed: bool,
    run_id: &str,
) -> AppResult<()> {
    write_memory_candidates(state, contact, decision, run_id).await?;
    if decision.operating_memory_update.is_empty() && context_pack.is_empty() {
        return Ok(());
    }
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    if !memory_card_has_signal(&effective_memory_card(memory)) {
        // task 6.3：把 typed memoryCard 在写入边界一次性转为 Document 落库。
        set_doc.insert(
            "memory_card",
            mongodb::bson::to_document(&effective_memory_card_for_contact(memory, contact))
                .unwrap_or_default(),
        );
        set_doc.insert("memory_card_version", next_memory_card_version(memory));
        set_doc.insert("memory_card_updated_at", DateTime::now());
    }
    if decision.consolidation_needed || decision.memory_write_score >= 6 {
        schedule_memory_consolidation_task(state, contact, run_id).await?;
    }
    state
        .db
        .operating_memories()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            doc! { "$set": set_doc },
            None,
        )
        .await?;
    Ok(())
}

pub(crate) fn build_decision_event_details(
    decision: &AgentDecision,
    playbook: Option<&OperationPlaybook>,
    review: &DecisionReviewResult,
) -> Document {
    let mut details = Document::new();
    details.insert("decision", to_document(decision).unwrap_or_default());
    details.insert("review", to_document(review).unwrap_or_default());
    if let Some(playbook) = playbook {
        if let Some(id) = playbook.id {
            details.insert("playbook_id", id.to_hex());
        }
        details.insert("playbook_version", playbook.version);
        details.insert("playbook_name", playbook.name.clone());
    }
    details
}

pub(crate) fn review_event_details(review: &DecisionReviewResult) -> Document {
    to_document(review).unwrap_or_default()
}

pub(crate) fn simulation_gateway_document(gateway: &SendGatewayResult) -> Document {
    let mut doc = to_document(gateway).unwrap_or_default();
    doc.insert("runMode", "shadow");
    doc
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn write_decision_review(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    decision: &AgentDecision,
    review: &DecisionReviewResult,
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    gateway_result: &SendGatewayResult,
    context_pack: &Document,
    status: &str,
    knowledge_route: &KnowledgeRouteResult,
    run_id: &str,
    planner: &RunPlannerResult,
) -> AppResult<ObjectId> {
    let prompt_versions = prompts::prompt_versions(
        &state.db,
        &state.config.default_workspace_id,
        &[
            "user.reply.system",
            "user.reply.policy",
            "user.reply.task",
            "user.knowledge.router",
            "user.review.system",
            "user.review.light.system",
            "user.memory_consolidator.system",
            "user.memory_consolidator.task",
        ],
        Some("user"),
        playbook,
    )
    .await?;
    let result = state
        .db
        .decision_reviews()
        .insert_one(
            AgentDecisionReview {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: Some(contact.wxid.clone()),
                run_id: Some(run_id.to_string()),
                inbound_message_id: inbound.message_id.clone(),
                reply_text: if decision.reply_text.trim().is_empty() {
                    None
                } else {
                    Some(decision.reply_text.clone())
                },
                approved: review_passed(review, runtime),
                scores: to_document(&review.scores).unwrap_or_default(),
                formula_breakdown: review.formula_breakdown.clone(),
                risks: review.risks.clone(),
                rewrite_instruction: non_empty_option(&Some(review.rewrite_instruction.clone())),
                review_summary: non_empty_option(&Some(review.review_summary.clone())),
                playbook_id: playbook.and_then(|item| item.id),
                playbook_version: playbook.map(|item| item.version),
                used_knowledge_ids: decision
                    .used_knowledge_ids
                    .iter()
                    .filter_map(|id| ObjectId::parse_str(id).ok())
                    .collect(),
                prompt_versions,
                operation_state: decision.operation_state.clone(),
                next_best_action: decision.next_best_action.clone(),
                context_pack_snapshot: {
                    let mut snapshot = context_pack.clone();
                    snapshot.insert(
                        "knowledgeRoute",
                        to_document(knowledge_route).unwrap_or_default(),
                    );
                    snapshot.insert("runPlanner", to_document(planner).unwrap_or_default());
                    snapshot
                },
                domain_config_snapshot: domain_config
                    .and_then(|config| to_document(config).ok())
                    .unwrap_or_default(),
                runtime_parameters_snapshot: runtime.as_document(),
                send_gateway_result: to_document(gateway_result).unwrap_or_default(),
                outcome_status: Some("pending".to_string()),
                reaction_analysis: Document::new(),
                reaction_claimed_at: None,
                reviewer_misjudge_signal: None,
                status: status.to_string(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    result
        .inserted_id
        .as_object_id()
        .ok_or_else(|| AppError::External("decision review id missing".to_string()))
}

#[allow(clippy::too_many_arguments)]
async fn write_agent_run_log(
    state: &AppState,
    contact: &Contact,
    run_id: &str,
    trigger_kind: &str,
    status: &str,
    planner: &RunPlannerResult,
    context: Document,
    knowledge_route: &KnowledgeRouteResult,
    decision: Document,
    review: Document,
    gateway_result: Document,
    error: Option<String>,
    source_event_id: &str,
    source_kind: &str,
) -> AppResult<()> {
    write_agent_run_log_with_finalize(
        state,
        contact,
        run_id,
        trigger_kind,
        status,
        planner,
        context,
        knowledge_route,
        decision,
        review,
        gateway_result,
        error,
        FinalizeRunLogFields {
            source_event_id: source_event_id.to_string(),
            source_kind: source_kind.to_string(),
            ..FinalizeRunLogFields::default()
        },
    )
    .await
}

/// agent-autonomy-loop W2 / Task 3.4：`agent_run_logs` 写入终态字段，包含
/// `finalReviewStatus / autonomyMode / revisionApplied / revisionReason /
/// preRevisionSummary / postRevisionSummary / selfCritique`。
///
/// `FinalizeRunLogFields::default()` 时退化为既有 `write_agent_run_log` 行为
/// （这些字段以空字符串 / None / false 形式落库，与 task 2.4 的占位一致）；
/// task 3.4 的 finalize 路径会传入实际值。
///
/// S1.1 (Phase 0)：扩出 `source_event_id / source_kind`，写库前由 `status`
/// 推算 `lifecycle`，全部经过 [`assert_lifecycle_valid`]，杜绝裸 `String::new()`
/// 漏 lifecycle 闭集校验的回归。
#[derive(Debug, Default, Clone)]
struct FinalizeRunLogFields {
    final_review_status: String,
    autonomy_mode: String,
    conversation_mode: String,
    conversation_mode_reason: Option<String>,
    revision_applied: bool,
    revision_reason: String,
    pre_revision_summary: Option<String>,
    post_revision_summary: Option<String>,
    self_critique: Option<String>,
    source_event_id: String,
    source_kind: String,
}

#[allow(clippy::too_many_arguments)]
async fn write_agent_run_log_with_finalize(
    state: &AppState,
    contact: &Contact,
    run_id: &str,
    trigger_kind: &str,
    status: &str,
    planner: &RunPlannerResult,
    context: Document,
    knowledge_route: &KnowledgeRouteResult,
    decision: Document,
    review: Document,
    gateway_result: Document,
    error: Option<String>,
    finalize_fields: FinalizeRunLogFields,
) -> AppResult<()> {
    // R9.10.e：写库前先校验 finalReviewStatus / gateway_status，脏值 fail-closed。
    assert_final_review_status_valid(&finalize_fields.final_review_status)?;
    assert_gateway_status_valid(status)?;

    // S1.1 (Phase 0)：lifecycle 闭集校验。由 `status` + `error` 派生终态
    // lifecycle（与 R0.3 / R0.10 状态机对齐），任何脏值 fail-closed 不写库。
    // 这取代了既有"裸 String::new() 占位"路径——envelope 在 W1 task 2.5 改造完成
    // 之前，本路径是 agent_run_logs 唯一终态写入点，必须保证 lifecycle 永远落非空闭集值。
    let lifecycle =
        derive_lifecycle_from_status(status, error.as_deref()).to_string();
    assert_lifecycle_valid(&lifecycle)?;

    // MP-5 / Task 15：从 task_local 读 budget snapshot，落 agent_run_logs。
    let budget_snapshot = current_run_budget().map(|b| b.snapshot());
    let (token_budget, tokens_used, llm_calls_used, degraded_reasons) = match &budget_snapshot {
        Some(snap) => (
            snap.token_budget,
            snap.tokens_used,
            snap.llm_calls_used,
            snap.degraded_reasons.clone(),
        ),
        None => (0, 0, 0, Vec::new()),
    };
    state
        .db
        .agent_run_logs()
        .insert_one(
            AgentRunLog {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: Some(contact.wxid.clone()),
                run_id: run_id.to_string(),
                trigger_kind: trigger_kind.to_string(),
                status: status.to_string(),
                planner: to_document(planner).unwrap_or_default(),
                context,
                knowledge_route: to_document(knowledge_route).unwrap_or_default(),
                decision,
                review,
                gateway_result,
                error,
                token_budget,
                tokens_used,
                llm_calls_used,
                degraded_reasons,
                // S1.1 (Phase 0)：lifecycle 由 derive_lifecycle_from_status
                // 推算并经 assert_lifecycle_valid 闭集校验；source_event_id /
                // source_kind 由调用方按 trigger 显式传入（FinalizeRunLogFields）。
                // 杜绝旧的裸 String::new() 占位路径。
                lifecycle: lifecycle.clone(),
                source_event_id: finalize_fields.source_event_id.clone(),
                source_kind: finalize_fields.source_kind.clone(),
                error_summary: None,
                abort_reason: None,
                revision_applied: finalize_fields.revision_applied,
                revision_reason: finalize_fields.revision_reason,
                pre_revision_summary: finalize_fields.pre_revision_summary,
                post_revision_summary: finalize_fields.post_revision_summary,
                self_critique: finalize_fields.self_critique,
                autonomy_mode: finalize_fields.autonomy_mode,
                conversation_mode: finalize_fields.conversation_mode,
                conversation_mode_reason: finalize_fields.conversation_mode_reason,
                final_review_status: finalize_fields.final_review_status,
                outbox_status: None,
                memory_consolidator_warnings: Vec::new(),
                created_at: DateTime::now(),
            },
            None,
        )
        .await?;
    Ok(())
}

/// agent-autonomy-loop W2 / Task 3.4：把 `finalize_review_for_send` 产出的待写
/// `agent_events` 列表（[`PendingFinalizeEvent`]）持久化到 `agent_events`。
///
/// finalize 函数被设计为**纯函数**（不持有 `&AppState`，不写库），事件以
/// [`PendingFinalizeEvent`] 形式返回，由本函数集中持久化；这样既保留了
/// finalize 的可测试性（单元测试可断言事件 kind / detail），又避免在
/// `review.rs` 中引入 db 反向依赖。
async fn persist_finalize_pending_events(
    state: &AppState,
    contact: &Contact,
    pending_events: &[PendingFinalizeEvent],
) -> AppResult<()> {
    for event in pending_events {
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            &event.kind,
            &event.status,
            &event.summary,
            Some(event.details.clone()),
        )
        .await?;
    }
    Ok(())
}

pub(crate) fn apply_confidence_override(
    planner: &mut RunPlannerResult,
    decision: &AgentDecision,
    runtime: &UserRuntimeParameters,
) {
    let confidence = decision.operation_state_confidence.unwrap_or(10);
    if confidence >= runtime.operation_state_confidence_full_review_below {
        return;
    }
    planner.review_mode = "full".to_string();
    planner.confidence_override_triggered = true;
    planner.confidence_override_reason = format!(
        "operation_state_confidence={} below threshold {}",
        confidence, runtime.operation_state_confidence_full_review_below
    );
    if !planner.reason.contains(&planner.confidence_override_reason) {
        if planner.reason.trim().is_empty() {
            planner.reason = planner.confidence_override_reason.clone();
        } else {
            planner.reason = format!("{}；{}", planner.reason, planner.confidence_override_reason);
        }
    }
}

pub(crate) async fn load_recent_messages(
    state: &AppState,
    contact: &Contact,
    limit: i64,
) -> AppResult<Vec<ConversationMessage>> {
    let options = FindOptions::builder()
        .sort(doc! { "created_at": -1 })
        .limit(limit)
        .build();
    let mut cursor = state
        .db
        .messages()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            options,
        )
        .await?;
    let mut messages = Vec::new();
    while let Some(message) = cursor.try_next().await? {
        messages.push(message);
    }
    Ok(messages)
}

pub(crate) async fn load_context_messages(
    state: &AppState,
    contact: &Contact,
    runtime: &UserRuntimeParameters,
) -> AppResult<Vec<ConversationMessage>> {
    let limit = (runtime.recent_message_limit * 6).clamp(24, 80);
    load_recent_messages(state, contact, limit).await
}

pub(crate) async fn load_pending_tasks(
    state: &AppState,
    contact: &Contact,
) -> AppResult<Vec<AgentTask>> {
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "status": "pending"
            },
            FindOptions::builder()
                .sort(doc! { "run_at": 1 })
                .limit(5)
                .build(),
        )
        .await?;
    let mut tasks = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        tasks.push(task);
    }
    Ok(tasks)
}

async fn pending_follow_up_count(state: &AppState, contact: &Contact) -> AppResult<i64> {
    state
        .db
        .tasks()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "kind": "follow_up",
                "status": "pending"
            },
            None,
        )
        .await
        .map(|count| count as i64)
        .map_err(AppError::from)
}

pub async fn write_event_for_account(
    state: &AppState,
    account_id: &str,
    contact_wxid: Option<&str>,
    kind: &str,
    status: &str,
    summary: &str,
    details: Option<Document>,
) -> AppResult<()> {
    state
        .db
        .events()
        .insert_one(
            AgentEvent {
                id: None,
                workspace_id: state.config.default_workspace_id.clone(),
                account_id: account_id.to_string(),
                contact_wxid: contact_wxid.map(ToString::to_string),
                kind: kind.to_string(),
                status: status.to_string(),
                summary: summary.to_string(),
                details,
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await?;
    Ok(())
}

/// ISSUE-001 (R12)：FollowUp 路径下"用户中途插话"判定纯函数。
///
/// 输入：`last_inbound_ms` = 联系人 last_inbound_at（缺失时 None），
/// `task_created_ms` = AgentTask.created_at；
/// 返回：true 表示在 task 创建后又有新 inbound，应当触发 context_changed。
///
/// 这是抢先在 review-held 短路前覆盖的判定逻辑，用于让 cancel_task /
/// write_event 落库时显式标记 context_changed 而非 finalize_review_blocked。
pub(crate) fn check_context_changed_followup_pure(
    last_inbound_ms: Option<i64>,
    task_created_ms: i64,
) -> bool {
    match last_inbound_ms {
        Some(ms) => ms > task_created_ms,
        None => false,
    }
}

/// Phase A / A3：taxonomy 软闸的纯逻辑——给定 LLM 输出的 customer_stage / intent_level
/// 与 [`TaxonomyCache`]，决定要做的字段改写、要附加的 risks 和要 upsert 的候选。
///
/// gateway 主路径只负责把 outcome 应用到 `final_decision` / `review.risks` 并执行
/// `upsert_candidate` 的 IO；判定本身可以在 lib-level 测，避免靠 #[ignore] 集成测试
/// 来保证"未知值真的进了候选 + 不阻塞 run"的硬契约。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TaxonomyGuardOutcome {
    pub customer_stage_rewrite: Option<String>,
    pub intent_level_rewrite: Option<String>,
    pub risks: Vec<String>,
    /// 待写入 `taxonomy_candidates` 的 `(kind, raw_value)` 对。空 / 仅空格的 raw 已被过滤。
    pub candidate_writes: Vec<(String, String)>,
}

pub(crate) fn compute_taxonomy_guard_outcome(
    customer_stage: Option<&str>,
    intent_level: Option<&str>,
    scope_account_id: &str,
    cache: &super::taxonomy::TaxonomyCache,
) -> TaxonomyGuardOutcome {
    let mut outcome = TaxonomyGuardOutcome::default();
    for (kind, raw_opt) in [("customer_stage", customer_stage), ("intent_level", intent_level)] {
        let Some(raw) = raw_opt.map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        match taxonomy_check_value(kind, raw, scope_account_id, cache) {
            TaxonomyMatch::Active => {}
            TaxonomyMatch::AliasActive(canonical) => {
                if kind == "customer_stage" {
                    outcome.customer_stage_rewrite = Some(canonical);
                } else if kind == "intent_level" {
                    outcome.intent_level_rewrite = Some(canonical);
                }
                outcome.risks.push(format!("taxonomy_alias_rewritten:{kind}"));
            }
            TaxonomyMatch::Deprecated => {
                outcome.risks.push(format!("taxonomy_deprecated_value:{kind}"));
            }
            TaxonomyMatch::CandidateNew => {
                outcome.risks.push(format!("taxonomy_candidate_new:{kind}"));
                outcome
                    .candidate_writes
                    .push((kind.to_string(), raw.to_string()));
            }
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_changed_followup_pure_hits_when_inbound_after_task() {
        let task_created_ms: i64 = 1_000_000;
        let last_inbound_ms = Some(task_created_ms + 5_000);
        assert!(check_context_changed_followup_pure(
            last_inbound_ms,
            task_created_ms
        ));
    }

    #[test]
    fn context_changed_followup_pure_passes_when_inbound_before_task() {
        let task_created_ms: i64 = 1_000_000;
        let last_inbound_ms = Some(task_created_ms - 1_000);
        assert!(!check_context_changed_followup_pure(
            last_inbound_ms,
            task_created_ms
        ));
    }

    #[test]
    fn context_changed_followup_pure_passes_when_no_inbound() {
        let task_created_ms: i64 = 1_000_000;
        assert!(!check_context_changed_followup_pure(None, task_created_ms));
    }

    #[test]
    fn context_changed_followup_pure_passes_on_exact_equality() {
        let task_created_ms: i64 = 1_000_000;
        let last_inbound_ms = Some(task_created_ms);
        // 严格大于：等时刻不算 context_changed，避免对边界 race 过敏
        assert!(!check_context_changed_followup_pure(
            last_inbound_ms,
            task_created_ms
        ));
    }

    #[test]
    fn context_changed_followup_pure_handles_negative_timestamps() {
        // 防御性：极旧时间戳（migration 数据）应仍走 i64 比较语义
        let task_created_ms: i64 = -1;
        let last_inbound_ms = Some(0_i64);
        assert!(check_context_changed_followup_pure(
            last_inbound_ms,
            task_created_ms
        ));
    }

    // ── Phase A / A3 落地验证：taxonomy 软闸 outcome 纯函数契约 ──────────
    //
    // gateway 主路径已经把"决定要做什么 (改写字段 / 追加 risk / upsert 候选)"提为
    // [`compute_taxonomy_guard_outcome`]，IO 留给调用方做。这里把 4 路命中分支 +
    // 空 / 空白输入路径 + customer_stage / intent_level 同时命中混合分支都覆盖一遍，
    // 保证后续重构不会让"未知值不进候选"或"alias 命中却没改写字段"这种契约偷偷失效。

    use super::super::taxonomy::{taxonomy_cache_for_tests, TaxonomyCache};
    use crate::models::{TaxonomyEntry, TaxonomyValue};

    fn entry(scope: &str, kind: &str, id: &str, aliases: &[&str], status: &str) -> TaxonomyEntry {
        TaxonomyEntry {
            id: None,
            scope: scope.to_string(),
            kind: kind.to_string(),
            value: TaxonomyValue {
                id: id.to_string(),
                display_name: id.to_string(),
                description: String::new(),
                aliases: aliases.iter().map(|s| s.to_string()).collect(),
                status: status.to_string(),
            },
            updated_at: mongodb::bson::DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
        }
    }

    fn cache_with(entries: Vec<TaxonomyEntry>) -> TaxonomyCache {
        taxonomy_cache_for_tests(entries)
    }

    #[test]
    fn taxonomy_outcome_empty_when_both_kinds_missing() {
        // 无任何 LLM 维度输出 → outcome 完全为空，不会乱写候选。
        let cache = cache_with(vec![]);
        let out = compute_taxonomy_guard_outcome(None, None, "acct-1", &cache);
        assert!(out.customer_stage_rewrite.is_none());
        assert!(out.intent_level_rewrite.is_none());
        assert!(out.risks.is_empty());
        assert!(out.candidate_writes.is_empty());
    }

    #[test]
    fn taxonomy_outcome_skips_blank_inputs() {
        // 空白字符串 trim 后等同于 None，不应触发 CandidateNew。
        let cache = cache_with(vec![]);
        let out = compute_taxonomy_guard_outcome(Some("   "), Some(""), "acct-1", &cache);
        assert!(out.candidate_writes.is_empty());
        assert!(out.risks.is_empty());
    }

    #[test]
    fn taxonomy_outcome_active_match_is_silent() {
        // 命中 active canonical_id：无改写、无 risk、无候选写入。
        let cache = cache_with(vec![entry(
            "global",
            "customer_stage",
            "first_contact",
            &[],
            "active",
        )]);
        let out =
            compute_taxonomy_guard_outcome(Some("first_contact"), None, "acct-1", &cache);
        assert!(out.customer_stage_rewrite.is_none());
        assert!(out.risks.is_empty());
        assert!(out.candidate_writes.is_empty());
    }

    #[test]
    fn taxonomy_outcome_alias_active_rewrites_field_and_appends_risk() {
        // alias 命中 → 改写为 canonical_id + 追加 taxonomy_alias_rewritten:* risk。
        let cache = cache_with(vec![entry(
            "global",
            "customer_stage",
            "first_contact",
            &["新客", "刚加好友"],
            "active",
        )]);
        let out = compute_taxonomy_guard_outcome(Some("新客"), None, "acct-1", &cache);
        assert_eq!(
            out.customer_stage_rewrite.as_deref(),
            Some("first_contact"),
            "alias 应被重写为 canonical_id"
        );
        assert!(
            out.risks
                .iter()
                .any(|r| r == "taxonomy_alias_rewritten:customer_stage"),
            "应追加 taxonomy_alias_rewritten:customer_stage risk，实际 {:?}",
            out.risks
        );
        assert!(
            out.candidate_writes.is_empty(),
            "alias 命中不应写候选，实际 {:?}",
            out.candidate_writes
        );
    }

    #[test]
    fn taxonomy_outcome_deprecated_only_appends_risk() {
        // deprecated 命中：仅追加 risk，不改写、不写候选。
        let cache = cache_with(vec![entry(
            "global",
            "intent_level",
            "lukewarm",
            &[],
            "deprecated",
        )]);
        let out = compute_taxonomy_guard_outcome(None, Some("lukewarm"), "acct-1", &cache);
        assert!(out.intent_level_rewrite.is_none());
        assert!(out
            .risks
            .iter()
            .any(|r| r == "taxonomy_deprecated_value:intent_level"));
        assert!(out.candidate_writes.is_empty());
    }

    /// CLAUDE.md 硬规则"unreviewed candidates must not block runs"的核心契约：
    /// 完全未知值 → 写候选 + 标 risk，但 review.approved 的判定是 gateway 主路径
    /// 自己做的事，与 outcome 无关；此处只断言 outcome 形状不会"反向阻塞"——没有
    /// 字段说"必须 fail review"。如果未来重构里 outcome 长出 `must_block: bool`
    /// 字段，本测会立刻失效，强制重新审视该硬规则。
    #[test]
    fn taxonomy_outcome_candidate_new_writes_to_queue_without_blocking() {
        let cache = cache_with(vec![entry(
            "global",
            "customer_stage",
            "first_contact",
            &[],
            "active",
        )]);
        let out = compute_taxonomy_guard_outcome(
            Some("完全没听过的阶段"),
            None,
            "acct-1",
            &cache,
        );
        assert!(
            out.risks
                .iter()
                .any(|r| r == "taxonomy_candidate_new:customer_stage"),
            "未知值应附加 taxonomy_candidate_new:* risk"
        );
        assert_eq!(
            out.candidate_writes,
            vec![("customer_stage".to_string(), "完全没听过的阶段".to_string())],
            "未知值必须进 candidate_writes，admin 才能在后台审核"
        );
        assert!(out.customer_stage_rewrite.is_none());
    }

    #[test]
    fn taxonomy_outcome_handles_both_kinds_in_single_pass() {
        // customer_stage 命中 alias，intent_level 完全未知：两个维度独立产出 risk
        // 与 candidate_writes，相互不串扰，保证 user-ops 决策路径上每条 LLM 输出
        // 都被走到。
        let cache = cache_with(vec![entry(
            "global",
            "customer_stage",
            "first_contact",
            &["新客"],
            "active",
        )]);
        let out = compute_taxonomy_guard_outcome(
            Some("新客"),
            Some("never_seen_intent"),
            "acct-1",
            &cache,
        );
        assert_eq!(
            out.customer_stage_rewrite.as_deref(),
            Some("first_contact")
        );
        assert!(out.intent_level_rewrite.is_none());
        let risks: Vec<&str> = out.risks.iter().map(String::as_str).collect();
        assert!(risks.contains(&"taxonomy_alias_rewritten:customer_stage"));
        assert!(risks.contains(&"taxonomy_candidate_new:intent_level"));
        assert_eq!(
            out.candidate_writes,
            vec![(
                "intent_level".to_string(),
                "never_seen_intent".to_string()
            )],
            "只有 intent_level 一个维度该进候选"
        );
    }

    #[test]
    fn taxonomy_outcome_account_scope_overrides_global() {
        // account 私有字典定义了 alias，global 没有：scope_account_id 走 account-first
        // fallback。本测确保 outcome 计算把 scope 透传给 check_value，避免回归到
        // "永远只查 global"。
        let cache = cache_with(vec![
            entry("global", "customer_stage", "first_contact", &[], "active"),
            entry(
                "acct-1",
                "customer_stage",
                "premium_first_contact",
                &["首单 VIP"],
                "active",
            ),
        ]);
        let out =
            compute_taxonomy_guard_outcome(Some("首单 VIP"), None, "acct-1", &cache);
        assert_eq!(
            out.customer_stage_rewrite.as_deref(),
            Some("premium_first_contact"),
            "应命中 account scope 的 alias，而非回落 global"
        );
    }

    /// P2-4：operation_state 同状态 / 缺值 SHALL 不触发 stage event。
    #[test]
    fn detect_state_transition_skips_no_op() {
        assert!(detect_state_transition(None, None).is_none());
        assert!(detect_state_transition(None, Some("")).is_none());
        assert!(detect_state_transition(None, Some("   ")).is_none());
        assert!(detect_state_transition(Some("intro"), Some("intro")).is_none());
        assert!(
            detect_state_transition(Some("  intro  "), Some("intro")).is_none(),
            "trim 后相等也算同状态"
        );
    }

    /// P2-4：从空 / None → 新状态 SHALL 触发首次 stage event。
    #[test]
    fn detect_state_transition_emits_on_first_state() {
        let out = detect_state_transition(None, Some("intro"));
        assert_eq!(out, Some(("".to_string(), "intro".to_string())));
        let out = detect_state_transition(Some(""), Some("intro"));
        assert_eq!(out, Some(("".to_string(), "intro".to_string())));
    }

    /// P2-4：A → B SHALL 触发 stage event；prior 与 next 双双归一化。
    #[test]
    fn detect_state_transition_emits_on_change() {
        let out = detect_state_transition(Some("intro"), Some("qualifying"));
        assert_eq!(out, Some(("intro".to_string(), "qualifying".to_string())));
        let out = detect_state_transition(Some(" intro\n"), Some(" closing "));
        assert_eq!(
            out,
            Some(("intro".to_string(), "closing".to_string())),
            "返回值应是 trim 后字符串"
        );
    }

    // ---- compute_profile_churn 画像写侧抖动探针（纯函数，确定性单测）----

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// ① 整体覆盖丢标签：old=[A,B,C] new=[A] → removed=2、net=-2、notable。
    #[test]
    fn churn_detects_tag_loss_from_full_overwrite() {
        let r = compute_profile_churn(
            &s(&["高LTV老客户", "技术", "理性决策"]),
            &s(&["高LTV老客户"]),
            None,
            None,
            None,
            None,
            None,
            "",
        );
        assert_eq!(r.tags_removed, 2, "整体覆盖丢了 2 个累积标签");
        assert_eq!(r.tags_added, 0);
        assert_eq!(r.tags_net, -2);
        assert!(r.notable, "丢标签必须计入 notable");
    }

    /// ② stage 翻转：old 决策 / new 关注 → flipped、notable。
    #[test]
    fn churn_detects_stage_flip() {
        let r = compute_profile_churn(
            &[],
            &[],
            Some("决策"),
            Some("关注"),
            None,
            None,
            None,
            "",
        );
        assert_eq!(
            r.stage_flipped,
            Some(("决策".to_string(), "关注".to_string()))
        );
        assert!(r.notable);
    }

    /// ③ old 空不算翻转：首次建立 stage 不是 flip，无抖动。
    #[test]
    fn churn_first_time_stage_is_not_flip() {
        let r = compute_profile_churn(
            &[],
            &[],
            None,
            Some("决策"),
            Some(""),
            Some("高"),
            None,
            "",
        );
        assert_eq!(r.stage_flipped, None, "old 空 = 首次画像，不算翻转");
        assert_eq!(r.intent_flipped, None, "old 空串 = 未知，不算翻转");
        assert!(!r.notable);
    }

    /// ④ summary append 长度增长，与写侧 `existing\nupdate` 一致（+1 换行）。
    #[test]
    fn churn_tracks_summary_growth() {
        let r = compute_profile_churn(
            &[],
            &[],
            None,
            None,
            None,
            None,
            Some("abc"),
            "de",
        );
        assert_eq!(r.summary_len_before, 3);
        assert_eq!(r.summary_len_after, 3 + 1 + 2, "existing + 换行 + update");
    }

    /// ④b summary 超软水位 → notable（无界增长信号）。
    #[test]
    fn churn_summary_over_soft_cap_is_notable() {
        let existing = "x".repeat(PROFILE_SUMMARY_SOFT_CAP);
        let r = compute_profile_churn(&[], &[], None, None, None, None, Some(&existing), "y");
        assert!(r.summary_len_after > PROFILE_SUMMARY_SOFT_CAP);
        assert!(r.notable, "summary 超软水位必须计入 notable");
    }

    /// ⑤ 无抖动：稳定标签 + 无翻转 + 短 summary → notable=false（不发事件）。
    #[test]
    fn churn_quiet_when_stable() {
        let r = compute_profile_churn(
            &s(&["技术", "理性决策"]),
            &s(&["技术", "理性决策", "高意向"]),
            Some("决策"),
            Some("决策"),
            Some("高"),
            Some("高"),
            Some("已有简介"),
            "补充一句",
        );
        assert_eq!(r.tags_removed, 0, "纯新增不丢标签");
        assert_eq!(r.tags_added, 1);
        assert_eq!(r.stage_flipped, None);
        assert_eq!(r.intent_flipped, None);
        assert!(!r.notable, "纯新增 + 无翻转 + 短 summary 不算抖动，不发事件");
    }

    /// ⑥ new 空 = 本轮未给标签，不计 added/removed（与"非空才写"对齐）。
    #[test]
    fn churn_empty_new_tags_means_no_update() {
        let r = compute_profile_churn(
            &s(&["技术", "理性决策"]),
            &[],
            None,
            None,
            None,
            None,
            None,
            "",
        );
        assert_eq!(r.tags_removed, 0, "new 空 = 未更新，不算丢标签");
        assert_eq!(r.tags_added, 0);
        assert_eq!(r.tags_net, 0);
        assert!(!r.notable);
    }

    // ---- merge_tags_union_capped 标签写侧结构层防御（纯函数，确定性单测）----

    /// ① 累积画像不被单轮覆盖：old=[A,B,C] new=[A] → 仍含 [A,B,C]（保序，只增不减）。
    /// 这是 [[cautious-profiling]] 结构层红线——一句弱信号无法抹平长期标签。
    #[test]
    fn merge_tags_keeps_accumulated_against_overwrite() {
        let out = merge_tags_union_capped(
            &s(&["高LTV老客户", "技术", "理性决策"]),
            &s(&["高LTV老客户"]),
            TAGS_PER_MESSAGE_CAP,
        );
        assert_eq!(out, s(&["高LTV老客户", "技术", "理性决策"]), "覆盖式单轮不得丢累积标签");
    }

    /// ② 新增标签 union 进来：old=[A,B] new=[C] → [A,B,C]，old 在前保序。
    #[test]
    fn merge_tags_appends_new_in_order() {
        let out = merge_tags_union_capped(&s(&["A", "B"]), &s(&["C"]), TAGS_PER_MESSAGE_CAP);
        assert_eq!(out, s(&["A", "B", "C"]));
    }

    /// ③ 去重：old 与 new 重叠只保留一份，且保 old 的位置。
    #[test]
    fn merge_tags_dedups_overlap() {
        let out = merge_tags_union_capped(&s(&["A", "B"]), &s(&["B", "D"]), TAGS_PER_MESSAGE_CAP);
        assert_eq!(out, s(&["A", "B", "D"]), "重叠去重，保 old 顺序");
    }

    /// ④ 封顶保累积、丢溢出新增：cap=2，old=[A,B] new=[C] → [A,B]（宁可不更新）。
    #[test]
    fn merge_tags_cap_prefers_accumulated() {
        let out = merge_tags_union_capped(&s(&["A", "B"]), &s(&["C"]), 2);
        assert_eq!(out, s(&["A", "B"]), "封顶时丢溢出新增，保累积画像");
    }

    /// ⑤ old 空：首次画像直接吃 new（去重保序），不受影响。
    #[test]
    fn merge_tags_empty_old_takes_new() {
        let out = merge_tags_union_capped(&[], &s(&["A", "A", "B"]), TAGS_PER_MESSAGE_CAP);
        assert_eq!(out, s(&["A", "B"]), "首次画像吃 new 并去重");
    }

    // ── Phase B Round 3：memory_summary 去重 + cap 写侧严谨化（[[cautious-profiling]] 第3点）──

    /// ① 空 existing：首条记忆直接落地（不再 naive concat 出前导换行）。
    #[test]
    fn memory_summary_empty_existing_takes_update() {
        let out = merge_memory_summary_dedup_capped("", "用户咨询五万预算方案", 12, 1200);
        assert_eq!(out, "用户咨询五万预算方案");
    }

    /// ② 正常追加：existing 与 update 各成一行，保序拼接。
    #[test]
    fn memory_summary_appends_new_line() {
        let out = merge_memory_summary_dedup_capped("第一轮要点", "第二轮要点", 12, 1200);
        assert_eq!(out, "第一轮要点\n第二轮要点");
    }

    /// ③ 整行去重：update 重复 existing 已有行时不再堆叠（修旧 naive append 的重复行病灶）。
    #[test]
    fn memory_summary_dedups_repeated_line() {
        let out = merge_memory_summary_dedup_capped("用户否认买意向", "用户否认买意向", 12, 1200);
        assert_eq!(out, "用户否认买意向", "重复行只保留一份");
    }

    /// ④ 行数封顶丢最旧（保新）：max_lines=2，已有 [a,b] + 追加 c → [b,c]。
    #[test]
    fn memory_summary_line_cap_drops_oldest() {
        let out = merge_memory_summary_dedup_capped("a\nb", "c", 2, 1200);
        assert_eq!(out, "b\nc", "超行数上限丢最旧行，保新");
    }

    /// ⑤ 字节封顶丢最旧：三行各远超半 cap，max_bytes 很小 → 只剩最新行（但至少保 1 行）。
    #[test]
    fn memory_summary_byte_cap_drops_oldest_keeps_one() {
        let line = "x".repeat(40);
        let existing = format!("{line}\n{line}");
        let out = merge_memory_summary_dedup_capped(&existing, &line, 12, 50);
        assert_eq!(out, line, "超字节上限丢到只剩最新一行，绝不丢空");
    }

    /// ⑥ 空白行被过滤：update 全是空白不污染结果，existing 原样保留。
    #[test]
    fn memory_summary_skips_blank_lines() {
        let out = merge_memory_summary_dedup_capped("要点A", "   \n\n", 12, 1200);
        assert_eq!(out, "要点A", "空白行不追加");
    }
}
