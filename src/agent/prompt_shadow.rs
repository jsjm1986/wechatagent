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

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, to_document, Document};
use mongodb::options::FindOptions;
use sha2::{Digest, Sha256};

use crate::error::AppResult;
use crate::evolution::revision::{content_sha256, parse_prompt_revision};
use crate::models::{
    AgentTask, Contact, ConversationMessage, OperatingMemory, OperationDomainConfig,
    OperationKnowledgeChunk, OperationPlaybook, Proposal,
};
use crate::routes::AppState;

use super::budget::{RunBudget, RUN_BUDGET};
use super::budget::{ShadowEvaluationSnapshot, SHADOW_EVALUATION_SNAPSHOT};
use super::decision::{
    decide_reply_with_promote, load_operation_playbook_for_contact,
    load_user_operation_domain_config_for_contact, PromptOverride,
};
use super::gateway::{load_context_messages, load_pending_tasks};
use super::guards::{
    initial_operation_state_key, normalize_decision_runtime, normalize_decision_state,
    planner_from_decision,
};
use super::knowledge_router::{
    load_operation_knowledge, route_operation_knowledge_read_only, route_used_knowledge_ids,
    select_operation_knowledge_chunks,
};
use super::memory::{
    effective_memory_card_for_contact, load_operating_memory_read_only, next_memory_card_version,
};
use super::review::{effective_review_mode, review_decision};
use super::run_envelope::SOURCE_KIND_INBOUND_MESSAGE;
use super::runtime::{resolve_thresholds, UserRuntimeParameters};
use super::shadow_finalize::finalize_shadow_decision;
use super::sufficiency::PromptTier;
use super::types::{KnowledgeRouteResult, RunPlannerResult};

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
    /// Production-equivalent terminal status after claim/finalize/state-policy gates.
    pub original_final_review_status: Option<String>,
    pub new_final_review_status: Option<String>,
    pub new_review_risks: Vec<String>,
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
            original_final_review_status: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
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
        .find_one(
            prompt_shadow_source_run_filter(
                &proposal.workspace_id,
                &proposal.account_id,
                source_run_id,
            ),
            None,
        )
        .await?
    {
        Some(log) => log,
        None => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "source_run_not_found",
            ))
        }
    };

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
    // 4. contact 从源 run 反查（workspace + account + wxid 三键定位）。
    let contact_wxid = match original.contact_wxid.clone() {
        Some(wxid) => wxid,
        None => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "contact_unavailable",
            ))
        }
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
        None => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "contact_unavailable",
            ))
        }
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
        .find_one(
            prompt_shadow_source_message_filter(
                &proposal.workspace_id,
                &proposal.account_id,
                &contact_wxid,
                &inbound_id,
            ),
            None,
        )
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

    // 5.5. 精确加载 proposal 评估时冻结的模板。历史行可继续用于 Shadow，
    // 但 token 的 id/version/content hash 任一不符都 fail-closed。
    let base_token = match proposal.base_revision.as_deref() {
        Some(token) => token,
        None => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "proposal_missing_base_revision",
            ))
        }
    };
    let parsed_revision = match parse_prompt_revision(base_token) {
        Some(revision) => revision,
        None => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "prompt_base_revision_invalid",
            ))
        }
    };
    let frozen_template = match state
        .db
        .prompt_templates()
        .find_one(
            doc! {
                "_id": parsed_revision.template_id,
                "workspace_id": &proposal.workspace_id,
                "prompt_key": &target_prompt_key,
                "version": parsed_revision.version,
            },
            None,
        )
        .await?
    {
        Some(template) if content_sha256(&template.content) == parsed_revision.content_sha256 => {
            template
        }
        _ => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "prompt_base_revision_unavailable",
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
    let active_profile =
        super::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id).await?;
    runtime.apply_active_profile(&active_profile);
    let active_products = if active_profile.transaction_facts_enabled {
        super::entitlements::load_active_products(&state.db, &contact.workspace_id).await
    } else {
        Vec::new()
    };
    // Populate the process cache once before entering the immutable pair. Reply
    // branches skip TTL refresh while this snapshot scope is installed.
    let live_taxonomy_cache = crate::agent::taxonomy::global_taxonomy_cache(&state.db);
    live_taxonomy_cache
        .find_or_load_read_only(&state.db)
        .await?;
    let taxonomy_cache = Arc::new(live_taxonomy_cache.snapshot_copy());
    let evaluation_snapshot = Arc::new(ShadowEvaluationSnapshot {
        active_profile,
        active_products,
        evaluated_at: mongodb::bson::DateTime::now(),
        taxonomy_cache,
    });

    // Pin one provider generation for the complete pair. Tests without a
    // registry keep using the injected provider already stored in AppState.
    let mut pinned_state = state.clone();
    if let Some(registry) = state.llm_registry.as_ref() {
        let snapshot = registry.snapshot(&proposal.workspace_id).await?;
        pinned_state.llm = Arc::new(snapshot);
        pinned_state.llm_registry = None;
    }

    SHADOW_EVALUATION_SNAPSHOT
        .scope(
            evaluation_snapshot,
            shadow_replay_inner(
                &pinned_state,
                contact,
                inbound,
                domain_config,
                runtime,
                target_prompt_key,
                append_snippet,
                frozen_template.content,
                source_run_id,
            ),
        )
        .await
}

fn prompt_shadow_source_run_filter(
    workspace_id: &str,
    account_id: &str,
    source_run_id: ObjectId,
) -> Document {
    doc! {
        "_id": source_run_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
    }
}

fn prompt_shadow_source_message_filter(
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    message_id: &str,
) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_wxid": contact_wxid,
        "message_id": message_id,
    }
}

struct PreparedPromptShadow {
    playbook: Option<OperationPlaybook>,
    memory: OperatingMemory,
    pending_tasks: Vec<AgentTask>,
    recent: Vec<ConversationMessage>,
    context_pack: Document,
    initial_planner: RunPlannerResult,
    knowledge_route: KnowledgeRouteResult,
    selected_chunks: Vec<OperationKnowledgeChunk>,
}

struct PromptBranchEvidence {
    scores: Option<Document>,
    final_status: String,
    review_risks: Vec<String>,
    self_critique_addressed: Option<bool>,
}

enum PromptBranchFailure {
    BudgetExceeded,
    TargetNotApplied,
}

fn new_shadow_budget(runtime: &UserRuntimeParameters, run_id: &str) -> Arc<RunBudget> {
    Arc::new(
        RunBudget::new(
            run_id,
            runtime.simulation_token_budget,
            runtime.run_max_llm_calls,
            runtime.knowledge_max_tool_calls,
        )
        .with_run_mode("shadow"),
    )
}

#[allow(clippy::too_many_arguments)]
async fn shadow_replay_inner(
    state: &AppState,
    contact: Contact,
    inbound: ConversationMessage,
    domain_config: Option<OperationDomainConfig>,
    runtime: UserRuntimeParameters,
    target_prompt_key: String,
    append_snippet: String,
    frozen_base_content: String,
    source_run_id: ObjectId,
) -> AppResult<PromptShadowSample> {
    let dependency_fingerprint = shadow_dependency_fingerprint(state, &contact).await?;
    let prepare_run_id = format!("shadow-prepare-{}", uuid::Uuid::new_v4());
    let prepare_budget = new_shadow_budget(&runtime, &prepare_run_id);
    let prepared = RUN_BUDGET
        .scope(
            prepare_budget.clone(),
            prepare_prompt_shadow(
                state,
                &contact,
                &inbound,
                domain_config.as_ref(),
                &runtime,
                &prepare_run_id,
                prepare_budget.clone(),
            ),
        )
        .await?;
    let Some(prepared) = prepared else {
        return Ok(PromptShadowSample::failed(source_run_id, "budget_exceeded"));
    };
    if shadow_dependency_fingerprint(state, &contact).await? != dependency_fingerprint {
        return Ok(PromptShadowSample::failed(
            source_run_id,
            "shadow_dependencies_changed",
        ));
    }

    let baseline_override = PromptOverride::new(
        target_prompt_key.clone(),
        String::new(),
        frozen_base_content.clone(),
    );
    let baseline_run_id = format!("shadow-baseline-{}", uuid::Uuid::new_v4());
    let baseline_budget = new_shadow_budget(&runtime, &baseline_run_id);
    let baseline = RUN_BUDGET
        .scope(
            baseline_budget.clone(),
            run_prompt_shadow_branch(
                state,
                &contact,
                &inbound,
                domain_config.as_ref(),
                &runtime,
                &prepared,
                &baseline_run_id,
                baseline_budget.clone(),
                &baseline_override,
            ),
        )
        .await?;
    let baseline = match baseline {
        Ok(value) => value,
        Err(PromptBranchFailure::BudgetExceeded) => {
            return Ok(PromptShadowSample::failed(source_run_id, "budget_exceeded"));
        }
        Err(PromptBranchFailure::TargetNotApplied) => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "prompt_target_not_applied",
            ));
        }
    };
    if shadow_dependency_fingerprint(state, &contact).await? != dependency_fingerprint {
        return Ok(PromptShadowSample::failed(
            source_run_id,
            "shadow_dependencies_changed",
        ));
    }

    let candidate_override =
        PromptOverride::new(target_prompt_key, append_snippet, frozen_base_content);
    let candidate_run_id = format!("shadow-candidate-{}", uuid::Uuid::new_v4());
    let candidate_budget = new_shadow_budget(&runtime, &candidate_run_id);
    let candidate = RUN_BUDGET
        .scope(
            candidate_budget.clone(),
            run_prompt_shadow_branch(
                state,
                &contact,
                &inbound,
                domain_config.as_ref(),
                &runtime,
                &prepared,
                &candidate_run_id,
                candidate_budget.clone(),
                &candidate_override,
            ),
        )
        .await?;
    let candidate = match candidate {
        Ok(value) => value,
        Err(PromptBranchFailure::BudgetExceeded) => {
            return Ok(PromptShadowSample::failed(source_run_id, "budget_exceeded"));
        }
        Err(PromptBranchFailure::TargetNotApplied) => {
            return Ok(PromptShadowSample::failed(
                source_run_id,
                "prompt_target_not_applied",
            ));
        }
    };
    if shadow_dependency_fingerprint(state, &contact).await? != dependency_fingerprint {
        return Ok(PromptShadowSample::failed(
            source_run_id,
            "shadow_dependencies_changed",
        ));
    }

    Ok(PromptShadowSample {
        source_run_id,
        status: "completed".to_string(),
        failure_reason: None,
        original_scores: baseline.scores,
        new_scores: candidate.scores,
        original_final_review_status: Some(baseline.final_status),
        new_final_review_status: Some(candidate.final_status),
        new_review_risks: candidate.review_risks,
        original_self_critique_addressed: baseline.self_critique_addressed,
        new_self_critique_addressed: candidate.self_critique_addressed,
    })
}

async fn prepare_prompt_shadow(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    run_id: &str,
    budget: Arc<RunBudget>,
) -> AppResult<Option<PreparedPromptShadow>> {
    let playbook = load_operation_playbook_for_contact(state, contact).await?;
    let memory = load_operating_memory_read_only(state, contact).await?;
    let operation_knowledge = load_operation_knowledge(state, contact).await?;
    let pending_tasks = load_pending_tasks(state, contact).await?;
    let mut history = load_context_messages(state, contact, runtime).await?;
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
        contact,
        &initial_operation_state_key(domain_config),
    )
    .to_document();
    let initial_planner = RunPlannerResult {
        risk_level: "medium".to_string(),
        review_mode: "light".to_string(),
        reason: "prompt shadow frozen comparison".to_string(),
        ..Default::default()
    };
    if budget.is_exceeded() {
        return Ok(None);
    }
    let knowledge_route = route_operation_knowledge_read_only(
        state,
        contact,
        inbound,
        &recent,
        &memory,
        &context_pack,
        &operation_knowledge,
        Some(run_id),
    )
    .await?;
    if budget.is_llm_or_token_exhausted() {
        return Ok(None);
    }
    let selected_chunks =
        select_operation_knowledge_chunks(&operation_knowledge.chunks, &knowledge_route);
    Ok(Some(PreparedPromptShadow {
        playbook,
        memory,
        pending_tasks,
        recent,
        context_pack,
        initial_planner,
        knowledge_route,
        selected_chunks,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn run_prompt_shadow_branch(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    domain_config: Option<&OperationDomainConfig>,
    runtime: &UserRuntimeParameters,
    prepared: &PreparedPromptShadow,
    run_id: &str,
    budget: Arc<RunBudget>,
    prompt_override: &PromptOverride,
) -> AppResult<Result<PromptBranchEvidence, PromptBranchFailure>> {
    if budget.is_llm_or_token_exhausted() {
        return Ok(Err(PromptBranchFailure::BudgetExceeded));
    }
    let (mut decision, promote_risks) = decide_reply_with_promote(
        state,
        contact,
        inbound,
        &prepared.recent,
        &prepared.pending_tasks,
        prepared.playbook.as_ref(),
        domain_config,
        runtime,
        &prepared.memory,
        &prepared.context_pack,
        &prepared.selected_chunks,
        &prepared.knowledge_route,
        None,
        Some(run_id),
        Some(prompt_override),
        PromptTier::Full,
        None,
    )
    .await?;
    normalize_decision_state(&mut decision, domain_config);
    normalize_decision_runtime(&mut decision, &prepared.initial_planner);
    let mut planner = planner_from_decision(&decision, "prompt shadow frozen comparison");
    if !prepared.knowledge_route.selected_chunk_ids.is_empty()
        || !prepared.knowledge_route.selected_knowledge_ids.is_empty()
    {
        planner.knowledge_required = true;
        if planner.review_mode.trim().is_empty() {
            planner.review_mode = "full".to_string();
        }
    }
    normalize_decision_runtime(&mut decision, &planner);
    decision.context_pack_version = Some(next_memory_card_version(&prepared.memory));
    decision.used_knowledge_ids = route_used_knowledge_ids(&prepared.knowledge_route);
    if budget.is_llm_or_token_exhausted() {
        return Ok(Err(PromptBranchFailure::BudgetExceeded));
    }
    let review = review_decision(
        state,
        contact,
        inbound,
        &prepared.recent,
        &decision,
        prepared.playbook.as_ref(),
        domain_config,
        runtime,
        &prepared.memory,
        &prepared.context_pack,
        &prepared.selected_chunks,
        &prepared.knowledge_route,
        effective_review_mode(&planner, &decision, runtime, false),
        Some(run_id),
        Some(prompt_override),
        None,
        None,
    )
    .await?;
    if !prompt_override.was_applied() {
        return Ok(Err(PromptBranchFailure::TargetNotApplied));
    }
    let finalized = finalize_shadow_decision(
        state,
        contact,
        inbound,
        decision,
        review,
        runtime,
        &prepared.selected_chunks,
        promote_risks,
        run_id,
    )
    .await?;
    Ok(Ok(PromptBranchEvidence {
        scores: to_document(&finalized.review.scores).ok(),
        final_status: finalized.final_status,
        review_risks: finalized.review.risks.clone(),
        self_critique_addressed: Some(finalized.review.self_critique_addressed),
    }))
}

async fn shadow_dependency_fingerprint(state: &AppState, contact: &Contact) -> AppResult<String> {
    let workspace = &contact.workspace_id;
    let account = &contact.account_id;
    let wxid = &contact.wxid;
    let account_or_global = doc! {
        "workspace_id": workspace,
        "$or": [
            { "account_id": account },
            { "account_id": null },
            { "account_id": { "$exists": false } },
        ],
    };
    let specs = vec![
        (
            "contacts",
            doc! { "workspace_id": workspace, "account_id": account, "wxid": wxid },
        ),
        (
            "conversation_messages",
            doc! { "workspace_id": workspace, "account_id": account, "contact_wxid": wxid },
        ),
        (
            "agent_tasks",
            doc! { "workspace_id": workspace, "account_id": account, "contact_wxid": wxid },
        ),
        (
            "operating_memories",
            doc! { "workspace_id": workspace, "account_id": account, "contact_wxid": wxid },
        ),
        (
            "agent_decision_reviews",
            doc! { "workspace_id": workspace, "account_id": account, "contact_wxid": wxid },
        ),
        (
            "agent_send_ledger",
            doc! { "workspace_id": workspace, "account_id": account, "contact_wxid": wxid },
        ),
        (
            "knowledge_operator_memory",
            doc! { "workspace_id": workspace, "account_id": account, "operator_id": account },
        ),
        (
            "operation_playbooks",
            doc! { "workspace_id": workspace, "account_id": account },
        ),
        (
            "operation_domain_configs",
            doc! { "workspace_id": workspace },
        ),
        (
            "operation_state_policies",
            doc! { "workspace_id": workspace },
        ),
        ("prompt_templates", doc! { "workspace_id": workspace }),
        ("domain_profiles", doc! { "workspace_id": workspace }),
        ("agent_souls", doc! { "workspace_id": workspace }),
        ("products", doc! { "workspace_id": workspace }),
        (
            "system_taxonomies",
            doc! { "workspace_id": workspace, "scope": { "$in": [account, "global"] } },
        ),
        ("content_assets", account_or_global.clone()),
        ("referral_cards", account_or_global.clone()),
        ("operation_knowledge_documents", account_or_global.clone()),
        ("operation_knowledge_chunks", account_or_global),
    ];
    let mut hasher = Sha256::new();
    hasher.update(
        state
            .prompt_pack_version
            .load(std::sync::atomic::Ordering::SeqCst)
            .to_le_bytes(),
    );
    for (name, filter) in specs {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        let mut cursor = state
            .db
            .raw()
            .collection::<Document>(name)
            .find(
                filter,
                FindOptions::builder().sort(doc! { "_id": 1 }).build(),
            )
            .await?;
        while let Some(row) = cursor.try_next().await? {
            let bytes = mongodb::bson::to_vec(&row)?;
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_shadow_source_filters_keep_full_tenant_scope() {
        let source_run_id = ObjectId::new();
        let run = prompt_shadow_source_run_filter("ws-a", "acc-a", source_run_id);
        assert_eq!(run.get_object_id("_id").unwrap(), source_run_id);
        assert_eq!(run.get_str("workspace_id").unwrap(), "ws-a");
        assert_eq!(run.get_str("account_id").unwrap(), "acc-a");

        let message = prompt_shadow_source_message_filter("ws-b", "acc-b", "wxid-b", "msg-1");
        assert_eq!(message.get_str("workspace_id").unwrap(), "ws-b");
        assert_eq!(message.get_str("account_id").unwrap(), "acc-b");
        assert_eq!(message.get_str("contact_wxid").unwrap(), "wxid-b");
        assert_eq!(message.get_str("message_id").unwrap(), "msg-1");
    }
}
