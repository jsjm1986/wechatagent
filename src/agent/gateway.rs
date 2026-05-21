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
    load_user_operation_domain_config,
};
use super::guards::{
    decision_requires_knowledge, enforce_decision_taxonomy_guards, load_product_claim_markers,
    normalize_decision_runtime, normalize_decision_state, planner_from_decision,
};
use super::knowledge_router::{
    empty_knowledge_route, load_operation_knowledge, maybe_emit_unverified_warning,
    route_operation_knowledge, route_used_knowledge_ids, select_operation_knowledge,
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
use super::run_envelope::{assert_final_review_status_valid, assert_gateway_status_valid};
use super::runtime::UserRuntimeParameters;
use super::types::{
    doc_bool, doc_i64, doc_string, non_empty_option, parse_rfc3339_to_bson, to_bson_array,
    AgentDecision, AgentTrigger, ContactSendResult, DecisionReviewResult, KnowledgeRouteResult,
    ManualContactSend, RunPlannerResult, SendGatewayResult,
};
use super::outbox::{enqueue as outbox_enqueue, EnqueueOutcome, EnqueueRequest};

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
    let domain_config = load_user_operation_domain_config(state, &contact.workspace_id).await?;
    let runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
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
    let selected_knowledge =
        select_operation_knowledge(&operation_knowledge.items, &knowledge_route);
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
        &selected_knowledge,
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

    let response = send_outbound_message(
        state,
        &contact,
        &content,
        Some(doc! {
            "source": "management_agent_send",
            "managementSource": request.source,
            "originalContentLocked": request.original_content_locked,
        }),
    )
    .await?;
    let message_id = response
        .get("newMsgId")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let sent_result = SendGatewayResult {
        allowed: true,
        status: "sent".to_string(),
        reason: "发送成功".to_string(),
        policy_blocks: Vec::new(),
        run_mode: "live".to_string(),
        message_id: message_id.clone(),
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
        &sent_result,
        &context_pack,
        "sent",
        &knowledge_route,
        &run_id,
        &planner,
    )
    .await?;
    write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "management_send",
        "success",
        "生产发送网关已发送私聊消息",
        Some(doc! {
            "sentContent": &content,
            "messageId": message_id.clone(),
            "decisionReviewId": review_id.to_hex(),
            "originalContentLocked": request.original_content_locked,
        }),
    )
    .await?;
    Ok(ContactSendResult {
        sent_content: content,
        message_id,
        review_approved: true,
        gateway_status: "sent".to_string(),
        gateway_reason: "发送成功".to_string(),
        decision_review_id: Some(review_id.to_hex()),
    })
}

pub(crate) async fn run_user_operation_gateway(
    state: &AppState,
    contact: Contact,
    trigger: AgentTrigger<'_>,
    task_id: Option<ObjectId>,
) -> AppResult<()> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let inbound = trigger_message(&contact, &trigger);
    let domain_config = load_user_operation_domain_config(state, &contact.workspace_id).await?;
    let runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);

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
    let mut knowledge_route = empty_knowledge_route(&initial_planner);
    let mut selected_knowledge =
        select_operation_knowledge(&operation_knowledge.items, &knowledge_route);
    let mut selected_chunks =
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
        &selected_knowledge,
        &selected_chunks,
        &knowledge_route,
        None,
        Some(&run_id),
    )
    .await?;
    normalize_decision_state(&mut decision, domain_config.as_ref());
    normalize_decision_runtime(&mut decision, &initial_planner);
    let mut planner = planner_from_decision(&decision, "Reply Agent 首轮内联路由");
    apply_confidence_override(&mut planner, &decision, &runtime);
    if decision_requires_knowledge(&decision) {
        if current_run_budget()
            .map(|b| b.is_exceeded())
            .unwrap_or(false)
        {
            if let Some(budget) = current_run_budget() {
                budget.mark_degraded("knowledge_second_pass_skipped_budget_exceeded");
            }
            knowledge_route.reason =
                "预算超额：跳过知识路由和二次回复决策，沿用首轮保守决策".to_string();
            planner.reason = "Reply Agent 首轮需要知识，但预算超额，跳过二次知识路由".to_string();
            planner.knowledge_required = true;
        } else {
            knowledge_route = route_operation_knowledge(
                state,
                &contact,
                &inbound,
                &recent_messages,
                &memory,
                &context_pack,
                &operation_knowledge,
                Some(&run_id),
            )
            .await?;
            selected_knowledge =
                select_operation_knowledge(&operation_knowledge.items, &knowledge_route);
            selected_chunks =
                select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);
            let (decision_v2, promote_risks_v2) = decide_reply_with_promote(
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
                &selected_knowledge,
                &selected_chunks,
                &knowledge_route,
                None,
                Some(&run_id),
            )
            .await?;
            decision = decision_v2;
            promote_risks = promote_risks_v2;
            normalize_decision_state(&mut decision, domain_config.as_ref());
            planner = planner_from_decision(&decision, "Reply Agent 打开知识库后二次路由");
            planner.knowledge_required = true;
            if planner.review_mode.trim().is_empty() {
                planner.review_mode = "full".to_string();
            }
            apply_confidence_override(&mut planner, &decision, &runtime);
        }
    }
    apply_confidence_override(&mut planner, &decision, &runtime);
    normalize_decision_runtime(&mut decision, &planner);
    decision.context_pack_version = Some(next_memory_card_version(&memory));
    decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);
    // ── agent-autonomy-loop W3 / Task 4.7：taxonomy 字典守卫 ──
    //
    // 对 `decision.customer_stage / intent_level` 调 `taxonomy::check_value`，
    // alias 命中改写为 canonical_id；deprecated / candidate_new 仅追加到
    // `promote_risks` 让 finalize_review_for_send 一次性合并到 review.risks。
    // R8.4：候选标记 SHALL NOT 强制 `review.approved=false`。
    crate::agent::taxonomy::ensure_global_cache_loaded(&state.db).await;
    enforce_decision_taxonomy_guards(
        &state.db,
        &crate::agent::taxonomy::global_taxonomy_cache(),
        &contact.account_id,
        &mut decision,
        &mut promote_risks,
    )
    .await;
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
            &selected_knowledge,
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

    if final_decision.should_reply && !review_passed(&review, &runtime) {
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
                &selected_knowledge,
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
                &selected_knowledge,
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
    let markers = load_product_claim_markers(state).await;
    let outcome = finalize_review_for_send(
        review,
        &mut final_decision,
        &runtime,
        &contact,
        &selected_chunks,
        &markers,
        promote_risks.clone(),
    );
    let FinalizeOutcome {
        review: finalized_review,
        status: mut finalize_status,
        pending_events,
    } = outcome;
    let mut review = finalized_review;
    persist_finalize_pending_events(state, &contact, &pending_events).await?;

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
                &selected_knowledge,
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
                        &selected_knowledge,
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
                        &markers,
                        promote_risks.clone(),
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

    // finalize 终态决定是否拦截发送：approved 路径继续走原有 send 逻辑；
    // 其它终态（held / blocked_*）一律 fail-closed（不发送、记录审计）。
    if !matches!(finalize_status, GatewayStatusFinal::Approved) {
        let blocked_status = finalize_status.gateway_status_str();
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
            cancel_task(state, task_id, &blocked_status, "finalize_review_blocked").await?;
        }
        write_event_for_account(
            state,
            &contact.account_id,
            Some(&contact.wxid),
            "blocked_review",
            &blocked_status,
            "finalize_review_for_send 拦截：本次回复未发送",
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
                revision_applied,
                revision_reason: revision_reason.clone(),
                pre_revision_summary: pre_revision_summary.clone(),
                post_revision_summary: post_revision_summary.clone(),
                self_critique: non_empty_option(&Some(final_decision.self_critique.clone())),
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
                revision_applied,
                revision_reason: revision_reason.clone(),
                pre_revision_summary: pre_revision_summary.clone(),
                post_revision_summary: post_revision_summary.clone(),
                self_critique: non_empty_option(&Some(final_decision.self_critique.clone())),
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
        if final_decision.should_reply { "sent" } else { "no_reply" },
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
            revision_applied,
            revision_reason: revision_reason.clone(),
            pre_revision_summary: pre_revision_summary.clone(),
            post_revision_summary: post_revision_summary.clone(),
            self_critique: non_empty_option(&Some(final_decision.self_critique.clone())),
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
    let pipeline: Vec<Document> = vec![doc! {
        "$set": {
            "last_outbound_at": now,
            "last_agent_run_at": now,
            "updated_at": now,
            "last_message_at": {
                "$max": ["$last_inbound_at", now]
            }
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
        set_doc.insert("tags", to_bson_array(&decision.tags));
    }
    if let Some(value) = non_empty_option(&decision.customer_stage) {
        // M2：customer_stage 实际变化时同步刷新 customer_stage_updated_at，
        // 让 Planner::scan_stage_stagnation 能区分"长期停滞"与"近期推进"。
        let prev = contact.customer_stage.as_deref();
        if prev.map(|s| s != value).unwrap_or(true) {
            set_doc.insert("customer_stage", value);
            set_doc.insert("customer_stage_updated_at", DateTime::now());
        } else {
            set_doc.insert("customer_stage", value);
        }
    }
    if let Some(value) = non_empty_option(&decision.intent_level) {
        set_doc.insert("intent_level", value);
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
        let merged = if existing.is_empty() {
            decision.memory_update.clone()
        } else {
            format!("{}\n{}", existing, decision.memory_update)
        };
        set_doc.insert("memory_summary", merged);
    }

    state
        .db
        .contacts()
        .update_one(doc! { "_id": contact.id }, doc! { "$set": set_doc }, None)
        .await?;

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
        FinalizeRunLogFields::default(),
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
#[derive(Debug, Default, Clone)]
struct FinalizeRunLogFields {
    final_review_status: String,
    autonomy_mode: String,
    revision_applied: bool,
    revision_reason: String,
    pre_revision_summary: Option<String>,
    post_revision_summary: Option<String>,
    self_critique: Option<String>,
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
                // ── agent-autonomy-loop W2 (Task 3.4) ──
                //
                // finalize_review_for_send 终态字段。`write_agent_run_log`
                // 调用方走 default，保留旧 trace（lifecycle / source_event_id /
                // source_kind 在 W1 task 2.5 接入 envelope 时写入；
                // task 3.4 仅负责 finalReviewStatus / autonomyMode / revision*
                // 这一组 R9 自治审计字段）。
                lifecycle: String::new(),
                source_event_id: String::new(),
                source_kind: String::new(),
                error_summary: None,
                abort_reason: None,
                revision_applied: finalize_fields.revision_applied,
                revision_reason: finalize_fields.revision_reason,
                pre_revision_summary: finalize_fields.pre_revision_summary,
                post_revision_summary: finalize_fields.post_revision_summary,
                self_critique: finalize_fields.self_critique,
                autonomy_mode: finalize_fields.autonomy_mode,
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
            },
            None,
        )
        .await?;
    Ok(())
}
