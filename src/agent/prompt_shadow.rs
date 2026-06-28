//! Prompt shadow 真模型对照（单条源样本）。
//!
//! 对一条历史源 run（`agent_run_logs`），用「原 prompt + critic 追加片段」
//! （[`PromptOverride`]）重新跑一次真实的 Reply + Review 链路，把新旧两侧的
//! 5 闸命中 / 自评 addressed / review 状态打包成 [`PromptShadowSample`]，作为
//! 管理员 release 的对照证据。
//!
//! 与 [`super::simulation`] 共用同一套加载链（playbook / memory / 知识路由 /
//! recent / context_pack），区别有四：
//!   1. 只处理单条源样本（不循环多轮）。
//!   2. inbound 用源 run 关联的真实历史消息，不合成。
//!   3. contact 从源 run 反查，不是入参。
//!   4. `decide_reply` / `review_decision` 末尾传 `Some(&override)`，注入候选片段。
//!
//! 本函数只跑决策 + 评审（纯演练），永不触达发送链 / outbox。

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, to_document, Document};

use crate::error::AppResult;
use crate::models::{ConversationMessage, Proposal};
use crate::routes::AppState;

use super::budget::{RunBudget, RUN_BUDGET};
use super::decision::{
    decide_reply, load_operation_playbook_for_contact,
    load_user_operation_domain_config_for_contact, PromptOverride,
};
use super::gateway::{load_context_messages, load_pending_tasks};
use super::guards::{
    initial_operation_state_key, normalize_decision_runtime, normalize_decision_state,
    planner_from_decision,
};
use super::knowledge_router::{
    empty_knowledge_route, load_operation_knowledge, route_operation_knowledge,
    route_used_knowledge_ids, select_operation_knowledge_chunks,
};
use super::memory::{
    effective_memory_card_for_contact, load_or_create_operating_memory, next_memory_card_version,
};
use super::review::{effective_review_mode, review_decision};
use super::run_envelope::SOURCE_KIND_INBOUND_MESSAGE;
use super::runtime::{resolve_thresholds, UserRuntimeParameters};
use super::types::RunPlannerResult;

/// 单条源样本的新旧对照结果。Task 13（replay.rs）负责把它映射进 `ShadowReplay`
/// 落库——本结构只承载两侧 scores / status / self_critique，不直接写库。
#[allow(dead_code)] // Task 13 (replay.rs) 接入后消费；本 task 只交付结构 + 入口。
pub(crate) struct PromptShadowSample {
    pub source_run_id: ObjectId,
    /// `"completed"` | `"failed"`。
    pub status: String,
    pub failure_reason: Option<String>,
    /// 源 run review.scores（G4 真实原始侧）。
    pub original_scores: Option<Document>,
    /// 用「原 prompt + 追加片段」跑出的 review.scores。
    pub new_scores: Option<Document>,
    pub original_self_critique_addressed: Option<bool>,
    pub new_self_critique_addressed: Option<bool>,
}

impl PromptShadowSample {
    #[allow(dead_code)]
    fn failed(source_run_id: ObjectId, reason: &str) -> Self {
        Self {
            source_run_id,
            status: "failed".to_string(),
            failure_reason: Some(reason.to_string()),
            original_scores: None,
            new_scores: None,
            original_self_critique_addressed: None,
            new_self_critique_addressed: None,
        }
    }
}

/// 对一条源 run 跑「原 prompt + 候选追加片段」的真模型对照。
///
/// 取不到源 run / contact / inbound 消息 / proposal 缺 key/snippet / 预算超额 →
/// 返回 `status="failed"` 的 sample（不抛错）；真正的 DB / LLM 故障才向上抛
/// `AppResult` 的 Err。
#[allow(dead_code)] // Task 13 (replay.rs prompt 分派) 接入后消费。
pub(crate) async fn shadow_replay_prompt_one(
    state: &AppState,
    proposal: &Proposal,
    source_run_id: ObjectId,
) -> AppResult<PromptShadowSample> {
    // 1. 反查源 run。
    let original = match state
        .db
        .agent_run_logs()
        .find_one(doc! { "_id": source_run_id }, None)
        .await?
    {
        Some(log) => log,
        None => return Ok(PromptShadowSample::failed(source_run_id, "source_run_not_found")),
    };

    // 2. 原始侧（G4 真实基线）：源 run review.scores + selfCritiqueAddressed。
    //    review 文档以 camelCase 落库（见 gateway `to_document(&review)`），故
    //    self_critique_addressed → `selfCritiqueAddressed`，scores → `scores`。
    let original_scores = original.review.get_document("scores").ok().cloned();
    let original_self_critique_addressed =
        original.review.get_bool("selfCritiqueAddressed").ok();

    // 3. 候选片段必须齐全（target_prompt_key + diff_snippet）。
    let (target_prompt_key, append_snippet) = match (
        proposal.proposed_template_key.clone(),
        proposal.diff_snippet.clone(),
    ) {
        (Some(key), Some(snippet)) => (key, snippet),
        _ => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "proposal_missing_key_or_snippet",
            ))
        }
    };
    let prompt_override = PromptOverride {
        target_prompt_key,
        append_snippet,
    };

    // 4. contact 从源 run 反查（workspace + account + wxid 三键定位）。
    let contact_wxid = match original.contact_wxid.clone() {
        Some(wxid) => wxid,
        None => return Ok(PromptShadowSample::failed(source_run_id, "contact_unavailable")),
    };
    let contact = match state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &original.workspace_id,
                "account_id": &original.account_id,
                "wxid": &contact_wxid,
            },
            None,
        )
        .await?
    {
        Some(c) => c,
        None => return Ok(PromptShadowSample::failed(source_run_id, "contact_unavailable")),
    };

    // 5. inbound 用源 run 关联的真实消息，不合成。源 inbound id 落在 AgentRunLog
    //    顶层 `source_event_id`（= R0 envelope 的 `message.message_id`，见
    //    gateway `trigger_envelope_source`）——gateway 从不往 `context` 写
    //    inboundMessageId，故必须读顶层字段。prompt shadow 只针对 inbound run：
    //    follow_up 的 source_event_id 是 task hex，查 messages 必 miss——这里靠
    //    source_kind 显式短路，避免无意义的一次 DB 查询。空串 / `synthetic:`
    //    前缀表示无真实 message id（兜底合成 id）→ 同记 `source_message_unavailable`。
    if original.source_kind != SOURCE_KIND_INBOUND_MESSAGE {
        return Ok(PromptShadowSample::failed(
            source_run_id,
            "source_message_unavailable",
        ));
    }
    let inbound_id = original.source_event_id.trim().to_string();
    if inbound_id.is_empty() || inbound_id.starts_with("synthetic:") {
        return Ok(PromptShadowSample::failed(
            source_run_id,
            "source_message_unavailable",
        ));
    }
    let inbound = match state
        .db
        .messages()
        .find_one(doc! { "message_id": &inbound_id }, None)
        .await?
    {
        Some(msg) => msg,
        None => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "source_message_unavailable",
            ))
        }
    };

    // 6. runtime（含 resolve_thresholds，与生产 review 共享同组阈值）。
    let domain_config =
        load_user_operation_domain_config_for_contact(state, &contact.workspace_id, &contact.wxid)
            .await?;
    let mut runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    resolve_thresholds(state, &contact)
        .await?
        .apply_to_runtime(&mut runtime);

    // 7. 预算（同 simulation：RunBudget + RUN_BUDGET.scope 包裹）。
    let run_id = uuid::Uuid::new_v4().to_string();
    let budget = Arc::new(RunBudget::new(
        run_id.clone(),
        runtime.simulation_token_budget,
        runtime.run_max_llm_calls,
        runtime.knowledge_max_tool_calls,
    ));
    RUN_BUDGET
        .scope(
            budget.clone(),
            shadow_replay_inner(
                state,
                contact,
                inbound,
                domain_config,
                runtime,
                run_id,
                budget.clone(),
                prompt_override,
                source_run_id,
                original_scores,
                original_self_critique_addressed,
            ),
        )
        .await
}

#[allow(clippy::too_many_arguments)]
async fn shadow_replay_inner(
    state: &AppState,
    contact: crate::models::Contact,
    inbound: ConversationMessage,
    domain_config: Option<crate::models::OperationDomainConfig>,
    runtime: UserRuntimeParameters,
    run_id: String,
    budget: Arc<RunBudget>,
    prompt_override: PromptOverride,
    source_run_id: ObjectId,
    original_scores: Option<Document>,
    original_self_critique_addressed: Option<bool>,
) -> AppResult<PromptShadowSample> {
    // 8. 由 contact 实时重建加载链（与 simulation 同款）。
    let playbook = load_operation_playbook_for_contact(state, &contact).await?;
    let memory = load_or_create_operating_memory(state, &contact).await?;
    let operation_knowledge = load_operation_knowledge(state, &contact).await?;
    let pending_tasks = load_pending_tasks(state, &contact).await?;
    let mut history = load_context_messages(state, &contact, &runtime).await?;
    history.reverse();
    let mut recent = history
        .iter()
        .rev()
        .take(runtime.recent_message_limit as usize)
        .cloned()
        .collect::<Vec<_>>();
    recent.reverse();

    let context_pack = effective_memory_card_for_contact(
        &memory,
        &contact,
        &initial_operation_state_key(domain_config.as_ref()),
    )
    .to_document();

    let initial_planner = RunPlannerResult {
        risk_level: "medium".to_string(),
        review_mode: "light".to_string(),
        reason: "Shadow 模式复用真实 Reply Agent 内联路由".to_string(),
        ..Default::default()
    };

    // 9. 知识路由（预算超额 → 空路由保守决策，与 simulation/生产 gateway 对齐）。
    let knowledge_route = if budget.is_exceeded() {
        budget.mark_degraded("prompt_shadow_knowledge_route_skipped_budget_exceeded");
        let mut route = empty_knowledge_route(&initial_planner);
        route.reason = "Shadow 预算超额：跳过知识路由，沿用空知识做保守决策".to_string();
        route
    } else {
        route_operation_knowledge(
            state,
            &contact,
            &inbound,
            &recent,
            &memory,
            &context_pack,
            &operation_knowledge,
            Some(&run_id),
        )
        .await?
    };
    let selected_chunks =
        select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);

    // 预算在跑决策前已耗尽 → 本条无意义，记 failed。
    if budget.is_exceeded() {
        return Ok(PromptShadowSample::failed(source_run_id, "budget_exceeded"));
    }

    // 10. Reply Agent（末尾传 Some(&override) 注入候选片段）。
    let mut decision = decide_reply(
        state,
        &contact,
        &inbound,
        &recent,
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
        Some(&prompt_override),
    )
    .await?;
    normalize_decision_state(&mut decision, domain_config.as_ref());
    normalize_decision_runtime(&mut decision, &initial_planner);
    let mut planner = planner_from_decision(&decision, "Shadow 单轮决策（知识路由前置）");
    if !knowledge_route.selected_chunk_ids.is_empty()
        || !knowledge_route.selected_knowledge_ids.is_empty()
    {
        planner.knowledge_required = true;
        if planner.review_mode.trim().is_empty() {
            planner.review_mode = "full".to_string();
        }
    }
    normalize_decision_runtime(&mut decision, &planner);
    decision.context_pack_version = Some(next_memory_card_version(&memory));
    decision.used_knowledge_ids = route_used_knowledge_ids(&knowledge_route);

    // 预算可能在 decide_reply 中耗尽 → 不再跑 review，记 failed。
    if budget.is_exceeded() {
        return Ok(PromptShadowSample::failed(source_run_id, "budget_exceeded"));
    }

    // 11. 独立 Review Agent（同样传 Some(&override)，覆盖 reply + review 两条链）。
    let review = review_decision(
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
        Some(&prompt_override),
    )
    .await?;

    // 12. 打包新侧对照（scores / self_critique addressed）。
    let new_scores = to_document(&review.scores).ok();

    Ok(PromptShadowSample {
        source_run_id,
        status: "completed".to_string(),
        failure_reason: None,
        original_scores,
        new_scores,
        original_self_critique_addressed,
        new_self_critique_addressed: Some(review.self_critique_addressed),
    })
}
