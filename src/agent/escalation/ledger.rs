//! 决策请示通道——台账 CRUD 层（pending 台账增删查改 / 知识缺口提案 / relay task 入队）。
//! 全部 async + db 访问。

use super::logic::{is_duplicate_key_error, is_pending_dedupe_conflict, short_code_from_seed};
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, ProvenanceSource, RevisionOp, RevisionRequest,
};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, AskHumanPolicy, OperationDomainConfig,
    OperationKnowledgeChunk, PrincipalDecision, PrincipalEscalationProtocol,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
    PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE, PRINCIPAL_CARD_DELIVERY_QUEUED,
    PRINCIPAL_CARD_DELIVERY_SENT, PRINCIPAL_CARD_DELIVERY_UNKNOWN,
    PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED, PRINCIPAL_RELAY_STATE_ENQUEUED,
    PRINCIPAL_RELAY_STATE_PENDING, PRINCIPAL_RELAY_STATE_TERMINAL,
};
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::UpdateOptions;

/// 插入一条 pending 台账。短码碰撞（短码唯一索引报错）时换种子重试至多 5 次。
///
/// 返回 `Ok(Some(entry))` = 成功插入；`Ok(None)` = 同客户同类别已有 pending
/// （并发漏过 `has_pending_for_contact` 预检，被 pending 去重唯一索引兜住），
/// 调用方据此跳过后续推卡（与 `has_pending_for_contact` 命中早返回同效）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_pending_escalation(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    category: &str,
    reason: &str,
    question_for_principal: &str,
    principal_wxid: &str,
    is_generalizable: bool,
    domain_config: &OperationDomainConfig,
    policy: AskHumanPolicy,
    principal_account_id: &str,
    customer_label: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    debug_assert!(
        ALLOWED_ESCALATION_CATEGORY.contains(&category),
        "category 必须在闭集内"
    );
    let now = DateTime::now();
    for attempt in 0..5u32 {
        let seed =
            (now.timestamp_millis() as u64).wrapping_add(attempt as u64 * 2_654_435_761) as u32;
        let short_code = short_code_from_seed(seed);
        let delivery_content = super::logic::render_principal_card(
            &short_code,
            customer_label,
            reason,
            question_for_principal,
        );
        let entry = AgentPrincipalEscalation {
            id: None,
            workspace_id: workspace_id.to_string(),
            account_id: account_id.to_string(),
            contact_wxid: contact_wxid.to_string(),
            short_code: short_code.clone(),
            status: PRINCIPAL_ESCALATION_STATUS_PENDING.to_string(),
            category: category.to_string(),
            reason: reason.to_string(),
            question_for_principal: question_for_principal.to_string(),
            principal_wxid: principal_wxid.to_string(),
            protocol: Some(PrincipalEscalationProtocol {
                domain: domain_config.domain.clone(),
                policy_version: domain_config.version,
                policy: policy.clone(),
                principal_account_id: principal_account_id.to_string(),
                delivery_generation: 1,
                delivery_state: PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE.to_string(),
                delivery_content,
                delivery_outbox_id: None,
                failure_cleanup_completed_at: None,
            }),
            decision: None,
            authorization_expires_at: None,
            is_generalizable,
            knowledge_proposal_emitted: false,
            last_holding_reply_ms: None,
            // Only a confirmed Outbox delivery may set this timestamp.
            last_pushed_at_ms: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
            resolved_via: None,
            relay_state: None,
            relay_task_id: None,
            relay_enqueued_at: None,
            relay_terminal_at: None,
            relay_terminal_reason: None,
        };
        match state
            .db
            .agent_principal_escalations()
            .insert_one(&entry, None)
            .await
        {
            Ok(res) => {
                let mut saved = entry;
                saved.id = res.inserted_id.as_object_id();
                return Ok(Some(saved));
            }
            Err(e) => {
                // pending 去重唯一索引冲突：并发已插入同客户同类别 pending → 静默"已存在"。
                if is_pending_dedupe_conflict(&e) {
                    return Ok(None);
                }
                // 短码唯一索引冲突：换种子重试。
                if is_duplicate_key_error(&e) {
                    continue;
                }
                return Err(e.into());
            }
        }
    }
    Err(AppError::External(
        "短码生成连续碰撞，插入请示台账失败".into(),
    ))
}

/// Materialize one frozen principal-card intent into the existing durable Outbox.
/// The source event is deterministic per escalation generation, so a crash after
/// Outbox insert but before the acknowledgement update converges to the same row.
pub(crate) async fn materialize_principal_card_delivery(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let protocol = entry
        .protocol
        .as_ref()
        .ok_or_else(|| AppError::Conflict("principal escalation protocol missing".to_string()))?;
    if entry.status != PRINCIPAL_ESCALATION_STATUS_PENDING
        || protocol.delivery_state != PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE
    {
        return Ok(());
    }
    activate_awaiting_principal_owner(state, entry).await?;
    let generation = protocol.delivery_generation;
    let source_event_id = format!("principal-card:{}:{generation}", escalation_id.to_hex());
    let outcome = crate::agent::outbox::enqueue(
        state,
        crate::agent::outbox::EnqueueRequest {
            workspace_id: entry.workspace_id.clone(),
            account_id: protocol.principal_account_id.clone(),
            contact_wxid: entry.principal_wxid.clone(),
            run_id: source_event_id.clone(),
            decision_id: None,
            source_event_id,
            source_kind: crate::agent::run_envelope::SOURCE_KIND_PRINCIPAL_ESCALATION.to_string(),
            content: protocol.delivery_content.clone(),
            media_asset_id: None,
            referral_card_id: None,
            max_attempts: 3,
        },
    )
    .await?;
    let outbox_id = match outcome {
        crate::agent::outbox::EnqueueOutcome::Created { outbox_id, .. } => outbox_id,
        crate::agent::outbox::EnqueueOutcome::IdempotentSkip {
            existing_outbox_id, ..
        } => existing_outbox_id,
    };
    state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "_id": escalation_id,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.delivery_generation": generation,
                "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
            },
            doc! { "$set": {
                "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_QUEUED,
                "protocol.delivery_outbox_id": outbox_id,
            } },
            None,
        )
        .await?;
    Ok(())
}

/// Reconcile principal-card Outbox facts back into escalation state and recover
/// interrupted enqueue acknowledgements. Legacy rows without a protocol are ignored.
pub(crate) async fn reconcile_principal_card_deliveries_once(state: &AppState) -> AppResult<u64> {
    use futures::TryStreamExt;
    let mut cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! { "$or": [
                {
                    "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                    "protocol.delivery_state": { "$in": [
                        PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
                        PRINCIPAL_CARD_DELIVERY_QUEUED,
                    ] },
                },
                {
                    "status": PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED,
                    "protocol.failure_cleanup_completed_at": { "$exists": false },
                },
            ] },
            mongodb::options::FindOptions::builder().limit(100).build(),
        )
        .await?;
    let mut changed = 0_u64;
    while let Some(entry) = cursor.try_next().await? {
        let Some(protocol) = entry.protocol.as_ref() else {
            continue;
        };
        if entry.status == PRINCIPAL_ESCALATION_STATUS_PENDING {
            activate_awaiting_principal_owner(state, &entry).await?;
        }
        if entry.status == PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED {
            changed += u64::from(complete_failed_delivery_cleanup(state, &entry).await?);
            continue;
        }
        if protocol.delivery_state == PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE {
            materialize_principal_card_delivery(state, &entry).await?;
            changed += 1;
            continue;
        }
        let Some(outbox_id) = protocol.delivery_outbox_id else {
            continue;
        };
        let Some(outbox) = state
            .db
            .collection_agent_send_outbox()
            .find_one(doc! { "_id": outbox_id }, None)
            .await?
        else {
            continue;
        };
        let (delivery_state, delivered_at, escalation_status) = match outbox.status.as_str() {
            "sent" => (
                PRINCIPAL_CARD_DELIVERY_SENT,
                outbox.sent_at.unwrap_or(outbox.updated_at),
                PRINCIPAL_ESCALATION_STATUS_PENDING,
            ),
            "failed_terminal" | "canceled" => (
                PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
                outbox.updated_at,
                PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED,
            ),
            "delivery_unknown" => (
                PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                outbox.updated_at,
                PRINCIPAL_ESCALATION_STATUS_PENDING,
            ),
            _ => continue,
        };
        let generation = protocol.delivery_generation;
        let mut set = doc! {
            "protocol.delivery_state": delivery_state,
            "status": escalation_status,
        };
        if delivery_state == PRINCIPAL_CARD_DELIVERY_SENT {
            set.insert("last_pushed_at_ms", delivered_at.timestamp_millis());
            set.insert("updated_at", delivered_at);
        }
        let result = state
            .db
            .agent_principal_escalations()
            .update_one(
                doc! {
                    "_id": entry.id,
                    "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                    "protocol.delivery_generation": generation,
                    "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_QUEUED,
                    "protocol.delivery_outbox_id": outbox_id,
                },
                doc! { "$set": set },
                None,
            )
            .await?;
        changed += result.modified_count;
        if result.modified_count == 1
            && escalation_status == PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED
        {
            let _ = complete_failed_delivery_cleanup(state, &entry).await?;
        }
    }
    Ok(changed)
}

fn awaiting_owner_id(escalation_id: mongodb::bson::oid::ObjectId) -> String {
    escalation_id.to_hex()
}

fn awaiting_owner_patch(awaiting: mongodb::bson::Bson, owners: mongodb::bson::Bson) -> Document {
    let mut patch = Document::new();
    patch.insert(crate::models::AWAITING_PRINCIPAL_DECISION_ATTR, awaiting);
    patch.insert(crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR, owners);
    patch
}

fn activate_awaiting_owner_pipeline(owner: &str, now: DateTime) -> Vec<Document> {
    let owners_key = crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR;
    let owners_path = format!("$domain_attributes.{owners_key}");
    let patch = awaiting_owner_patch(
        true.into(),
        doc! { "$setUnion": ["$$owners", [owner]] }.into(),
    );
    vec![doc! { "$set": {
        "domain_attributes": {
            "$let": {
                "vars": {
                    "attrs": { "$cond": [
                        { "$eq": [{ "$type": "$domain_attributes" }, "object"] },
                        "$domain_attributes",
                        {},
                    ] },
                    "owners": { "$cond": [
                        { "$isArray": &owners_path },
                        &owners_path,
                        [],
                    ] },
                },
                "in": { "$mergeObjects": ["$$attrs", patch] },
            },
        },
        "domain_attributes_updated_at": now,
    } }]
}

fn remove_awaiting_owner_pipeline(owner: &str, now: DateTime) -> Vec<Document> {
    let owners_key = crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR;
    let owners_path = format!("$domain_attributes.{owners_key}");
    let patch = awaiting_owner_patch(
        doc! { "$gt": [{ "$size": "$$remaining" }, 0] }.into(),
        "$$remaining".into(),
    );
    vec![doc! { "$set": {
        "domain_attributes": {
            "$let": {
                "vars": {
                    "attrs": { "$cond": [
                        { "$eq": [{ "$type": "$domain_attributes" }, "object"] },
                        "$domain_attributes",
                        {},
                    ] },
                    "owners": { "$cond": [
                        { "$isArray": &owners_path },
                        &owners_path,
                        [],
                    ] },
                },
                "in": {
                    "$let": {
                        "vars": {
                            "remaining": { "$filter": {
                                "input": "$$owners",
                                "as": "candidate",
                                "cond": { "$ne": ["$$candidate", owner] },
                            } },
                        },
                        "in": { "$mergeObjects": ["$$attrs", patch] },
                    },
                },
            },
        },
        "domain_attributes_updated_at": now,
    } }]
}

async fn activate_awaiting_principal_owner(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let owner = awaiting_owner_id(escalation_id);
    let result = state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid,
            },
            activate_awaiting_owner_pipeline(&owner, DateTime::now()),
            None,
        )
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "principal_escalation_contact_missing".to_string(),
        ));
    }
    Ok(())
}

async fn remove_awaiting_principal_owner(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let owner = awaiting_owner_id(escalation_id);
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid,
            },
            remove_awaiting_owner_pipeline(&owner, DateTime::now()),
            None,
        )
        .await?;
    Ok(())
}

/// Mark a resolved principal relay as terminal, then release only this
/// escalation's awaiting ownership. Missing `relay_state` is accepted only
/// through this explicit task-bound path for rolling-upgrade compatibility;
/// background reconciliation still never guesses or replays legacy rows.
pub(crate) async fn terminalize_principal_relay(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    reason: &str,
) -> AppResult<bool> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let now = DateTime::now();
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! {
                "_id": escalation_id,
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "contact_wxid": &entry.contact_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "$or": [
                    { "relay_state": { "$in": [
                        PRINCIPAL_RELAY_STATE_PENDING,
                        PRINCIPAL_RELAY_STATE_ENQUEUED,
                    ] } },
                    { "relay_state": { "$exists": false } },
                ],
            },
            doc! { "$set": {
                "relay_state": PRINCIPAL_RELAY_STATE_TERMINAL,
                "relay_terminal_at": now,
                "relay_terminal_reason": reason,
                "updated_at": now,
            } },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    let terminal = if let Some(updated) = updated {
        updated
    } else {
        state
            .db
            .agent_principal_escalations()
            .find_one(
                doc! {
                    "_id": escalation_id,
                    "workspace_id": &entry.workspace_id,
                    "account_id": &entry.account_id,
                    "contact_wxid": &entry.contact_wxid,
                    "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                    "relay_state": PRINCIPAL_RELAY_STATE_TERMINAL,
                },
                None,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("principal_relay_terminal_state_changed".into()))?
    };
    remove_awaiting_principal_owner(state, &terminal).await?;
    Ok(terminal.relay_terminal_reason.as_deref() == Some(reason))
}

/// Resolve the escalation explicitly bound to a relay task and terminalize it.
/// The deterministic new-protocol task id equals the escalation id; the short
/// code fallback is retained for already-running legacy tasks only.
pub(crate) async fn terminalize_principal_relay_for_task(
    state: &AppState,
    task: &AgentTask,
    reason: &str,
) -> AppResult<bool> {
    let mut identity = doc! {
        "workspace_id": &task.workspace_id,
        "account_id": &task.account_id,
        "contact_wxid": &task.contact_wxid,
        "short_code": task.content.trim(),
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
    };
    if let Some(task_id) = task.id {
        identity.insert(
            "$or",
            vec![
                doc! { "_id": task_id },
                doc! { "relay_task_id": task_id },
                doc! {
                    "relay_task_id": { "$exists": false },
                    "relay_state": { "$exists": false },
                },
            ],
        );
    }
    let entry = state
        .db
        .agent_principal_escalations()
        .find_one(identity, None)
        .await?
        .ok_or_else(|| AppError::Conflict("principal_relay_escalation_not_found".into()))?;
    terminalize_principal_relay(state, &entry, reason).await
}

/// Reconcile the coarse contact awaiting marker after a terminal card failure.
/// Cleanup acknowledgement is written last, so an interrupted pass is retried.
async fn complete_failed_delivery_cleanup(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<bool> {
    remove_awaiting_principal_owner(state, entry).await?;
    let acknowledged = state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "_id": entry.id,
                "status": PRINCIPAL_ESCALATION_STATUS_DELIVERY_FAILED,
                "protocol.failure_cleanup_completed_at": { "$exists": false },
            },
            doc! { "$set": {
                "protocol.failure_cleanup_completed_at": DateTime::now(),
            } },
            None,
        )
        .await?;
    Ok(acknowledged.modified_count == 1)
}

/// 查某 workspace 下某领导 wxid 当前所有 pending 台账（按创建时间升序）。
pub(crate) async fn list_pending_for_principal(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    principal_wxid: &str,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.principal_account_id": account_id,
                "protocol.delivery_state": { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// 该客户是否已有同类别的 pending 请示（去重用：避免等待期重复推卡骚扰领导）。
pub(crate) async fn has_pending_for_contact(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    category: &str,
) -> AppResult<bool> {
    let count = state
        .db
        .agent_principal_escalations()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "contact_wxid": contact_wxid,
                "category": category,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            None,
        )
        .await?;
    Ok(count > 0)
}

/// 把一条 pending 台账标 resolved，写入真人裁决 + 授权过期时间。
pub(crate) async fn resolve_escalation(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
    authorization_expires_at: Option<DateTime>,
    resolved_via: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let now = DateTime::now();
    let decision_bson = mongodb::bson::to_bson(decision)?;
    let mut set = doc! {
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "decision": decision_bson,
        "updated_at": now,
        "resolved_at": now,
        "resolved_via": resolved_via,
        "relay_state": PRINCIPAL_RELAY_STATE_PENDING,
        "relay_task_id": escalation_id,
    };
    if let Some(exp) = authorization_expires_at {
        set.insert("authorization_expires_at", exp);
    }
    let mut filter = doc! {
        "_id": escalation_id,
        "workspace_id": &entry.workspace_id,
        "short_code": &entry.short_code,
        "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
        "principal_wxid": &entry.principal_wxid,
    };
    if let Some(protocol) = entry.protocol.as_ref() {
        filter.insert("protocol.delivery_generation", protocol.delivery_generation);
        if resolved_via == "wechat" {
            filter.insert(
                "protocol.delivery_state",
                doc! { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            );
        }
    }
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            filter,
            doc! { "$set": set },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    if let Some(resolved) = updated.as_ref() {
        materialize_relay_task(state, resolved).await?;
    }
    Ok(updated)
}

/// 真人决策可泛化时，发一条知识缺口提案（draft + needs_review）。
/// 复用现有知识子系统的 draft 契约——绝不自动验证（AI 永不自动验证红线）。
/// 写 workspace 共享域（account_id=None），与既有 chat 补库共享域一致，
/// 保证提案对整个 workspace 召回可见，而非账号私有。
pub(crate) async fn emit_knowledge_gap_proposal(
    state: &AppState,
    escalation: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
) -> AppResult<()> {
    // title 从 substance 提炼（不再用 escalation.reason——同 sediment，reason 是卡点原因/
    // reviewer 质检点评，当知识标题会扭曲召回）；draft 提案加「待审核：」前缀以区分未复核。
    let raw_title = derive_sediment_title(
        state,
        &escalation.workspace_id,
        &escalation.account_id,
        &escalation.contact_wxid,
        &decision.substance,
    )
    .await;
    let title = format!("待审核：{raw_title}");
    let body = format!(
        "源自客户「{}」请示 #{}。\n领导裁决：{}\n约束：{}",
        escalation.contact_wxid,
        escalation.short_code,
        decision.substance,
        if decision.constraints.is_empty() {
            "无".to_string()
        } else {
            decision.constraints.join("；")
        }
    );
    let chunk = OperationKnowledgeChunk {
        workspace_id: escalation.workspace_id.clone(),
        account_id: None, // workspace 共享域（与既有 chat 补库共享域一致）
        status: "draft".to_string(),
        integrity_status: Some("needs_review".to_string()),
        title,
        body: Some(body),
        ..OperationKnowledgeChunk::default()
    };
    state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await?;
    Ok(())
}

/// FNV-1a 64bit 文本 hash——与知识子系统 `stable_text_hash`
/// (routes/knowledge/mod.rs:710，`pub(super)` 跨模块不可见) 保持同一算法。
/// escalation 模块无法调用它，故复制一份纯逻辑，保证自锚定的 quoteHash 与
/// 既有锚点口径完全一致（同输入同 hash）。
fn stable_text_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// 无父文档的 substance 自锚定：quote 即 substance 整段，锚点覆盖整段（start=行首）。
/// 复制 `source_anchor_for_quote`(routes/knowledge/mod.rs:756，`pub(super)` 跨模块不可见)
/// 在 `raw_content == source_quote` 场景下的等价纯逻辑：领导授权的 substance 无外部父文档
/// 可溯源，以 substance 自身为锚源（合法出处 = 领导裁决原话），使其满足 D2 门
/// （source_quote 非空 + source_anchors 非空）。substance trim 后为空 → 返回 None。
fn self_anchor_for_substance(substance: &str) -> Option<Document> {
    let quote = substance.trim();
    if quote.is_empty() {
        return None;
    }
    let start = 0usize;
    let end = quote.len();
    // start=行首=第 1 行；end_line=quote 内换行数 + 1（与 source_anchor_for_quote 同口径：
    // 后者 end_line = raw_content[..end] 的换行数 + 1，本场景 raw_content == quote）。
    let start_line = 1i32;
    let end_line = (quote.bytes().filter(|b| *b == b'\n').count() + 1) as i32;
    Some(doc! {
        "startOffset": start as i32,
        "endOffset": end as i32,
        "startLine": start_line,
        "endLine": end_line,
        "sourceQuote": quote,
        "quoteHash": stable_text_hash(quote),
    })
}

/// 从领导裁决 substance 提炼一个确定性的知识标题兜底：
/// 取首句（截到第一个句末标点 `。！？!?` 或换行之前），再按 chars 限长 40。
/// 空 substance → 固定安全标题（配合 sediment 空 substance 已提前跳过，实际仅有 substance 时被用到）。
/// LLM 提炼失败时回退到本函数，保证 title 永远可读、沉淀永不失败。
// 目前仅被单测消费；Task 3（derive_sediment_title 的 LLM 兜底）/ Task 4
// （sediment 落 title）接线后即成为生产调用点，暂 allow(dead_code) 保持 build 无警告。
#[allow(dead_code)]
pub(crate) fn derive_sediment_title_fallback(substance: &str) -> String {
    let trimmed = substance.trim();
    if trimmed.is_empty() {
        return "领导授权沉淀".to_string();
    }
    // 首句：截到第一个句末标点 / 换行之前。
    let first = trimmed
        .split(|c| matches!(c, '。' | '！' | '？' | '!' | '?' | '\n'))
        .next()
        .unwrap_or(trimmed)
        .trim();
    let first = if first.is_empty() { trimmed } else { first };
    // 按 chars 限长 40（多字节安全），超长截断加省略号。
    let mut chars: Vec<char> = first.chars().collect();
    if chars.len() > 40 {
        chars.truncate(40);
        let mut out: String = chars.into_iter().collect();
        out.push('…');
        out
    } else {
        chars.into_iter().collect()
    }
}

/// 从领导裁决 substance 提炼知识标题：优先 LLM 提炼，任何失败/空结果回退确定性兜底。
/// 绝不因提炼失败让沉淀失败——title 永远可读、非空。
// 目前尚无生产调用点（Task 4 接线 sediment 落 title 时才启用），暂 allow(dead_code)
// 保持 build 无警告。
#[allow(dead_code)]
pub(crate) async fn derive_sediment_title(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_wxid: &str,
    substance: &str,
) -> String {
    let trimmed = substance.trim();
    if trimmed.is_empty() {
        return derive_sediment_title_fallback(substance);
    }
    let system =
        match crate::prompts::load_prompt(&state.db, workspace_id, "escalation.sediment.title")
            .await
        {
            Ok(s) => s,
            Err(_) => return derive_sediment_title_fallback(substance),
        };
    let user = format!("决策实质：{}", trimmed);
    let value = match crate::agent::generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        Some(contact_wxid),
        None,
        "escalation.sediment.title",
        &system,
        &user,
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return derive_sediment_title_fallback(substance),
    };
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if title.is_empty() {
        return derive_sediment_title_fallback(substance);
    }
    // LLM 也可能给超长 title——用兜底同款 chars 限长逻辑收口（40 chars）。
    let capped: String = title.chars().take(40).collect();
    if title.chars().count() > 40 {
        format!("{capped}…")
    } else {
        capped
    }
}

/// B 类沉淀：把领导（真人）经决策请示通道授权的 substance 落为 verified 知识 chunk，
/// 供全体客户复用。
///
/// **红线定性**：验证者是领导（真人）本人，不是 AI 自评——只是把知识库复核前移到领导
/// 裁决当下。source=`PrincipalAuthorized` 归人类权威家族（视同 Human），不落入
/// `apply_chunk_revision` 对 source=Ai 的 draft 强制降级，故可直接带 verified；
/// "AI 永不自动验证"本质未破。
///
/// **两步法**（`apply_chunk_revision` 只能改既有 chunk、不建 chunk，find_one 找不到会 NotFound）：
/// - 步骤①：insert 一条 chunk（status=active + integrity_status=needs_review；domain
///   必填 `user_operations` 否则 knowledge_router 召不回；chunk_type=product_fact；
///   source_quote=substance + 自锚定，满足 D2 门）；
/// - 步骤②：`apply_chunk_revision(op=Verify, source=PrincipalAuthorized)` 把
///   integrity_status 改 verified（verify 语义只动 integrity_status/confidence，不动 status）。
///
/// patch 绝不带锁定字段（verified_at / verified_by / source_anchor）——照搬
/// verify.rs:104-115 的非锁定字段集。substance trim 后为空 → 无从沉淀，直接 Ok 跳过。
pub(crate) async fn sediment_principal_authorized_knowledge(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
    decision: &PrincipalDecision,
) -> AppResult<()> {
    let substance = decision.substance.trim();
    // 空 substance 自锚定为空 → 过不了 D2 门，也无实质可沉淀，跳过（幂等、非错误）。
    let Some(anchor) = self_anchor_for_substance(substance) else {
        return Ok(());
    };

    // 步骤①：建 chunk。参照 emit_knowledge_gap_proposal 的 chunk doc 范式，补齐 D2 门
    // 必需的 domain / chunk_type / source_quote / source_anchors。
    // title 从 substance 提炼（不再用 entry.reason——reason 是给领导看的卡点原因/reviewer
    // 质检点评，当知识标题会扭曲召回打分并污染 decision prompt）；LLM 失败回退确定性兜底。
    let title = derive_sediment_title(
        state,
        &entry.workspace_id,
        &entry.account_id,
        &entry.contact_wxid,
        substance,
    )
    .await;
    let body = format!(
        "源自客户「{}」请示 #{}。\n领导裁决：{}\n约束：{}",
        entry.contact_wxid,
        entry.short_code,
        substance,
        if decision.constraints.is_empty() {
            "无".to_string()
        } else {
            decision.constraints.join("；")
        }
    );
    let chunk = OperationKnowledgeChunk {
        workspace_id: entry.workspace_id.clone(),
        account_id: None, // workspace 共享域（与既有 chat 补库 / 知识提案共享域一致）
        domain: "user_operations".to_string(), // 必填：knowledge_router 按此召回
        chunk_type: "product_fact".to_string(), // 领导授权的产品说法
        status: "active".to_string(),
        integrity_status: Some("needs_review".to_string()),
        title,
        body: Some(body),
        source_quote: Some(substance.to_string()),
        source_anchors: vec![anchor],
        confidence_score: Some(0),
        ..OperationKnowledgeChunk::default()
    };
    let inserted = state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await?;
    let Some(object_id) = inserted.inserted_id.as_object_id() else {
        return Err(AppError::External(
            "沉淀领导授权知识：insert 未返回 ObjectId".into(),
        ));
    };

    // 步骤②：verify。patch 只带非锁定字段（integrity_status / confidence_score）；
    // source=PrincipalAuthorized 标注验证者是领导（真人）；actor=principal_wxid 供审计追溯
    // 是哪位领导授权。绝不带锁定字段 verified_at / verified_by / source_anchor。
    apply_chunk_revision(
        &state.db,
        &entry.workspace_id,
        object_id,
        RevisionRequest {
            op: RevisionOp::Verify,
            source: ProvenanceSource::PrincipalAuthorized,
            patch: doc! {
                "integrity_status": "verified",
                "confidence_score": 100,
            },
            reason: Some(format!("领导授权沉淀（请示 #{}）", entry.short_code)),
            actor: Some(entry.principal_wxid.clone()),
        },
    )
    .await?;
    Ok(())
}

/// 反查：在**入站消息自身所属 workspace** 内，from_wxid 是否是某 domain 的决策人。
/// KD-04：判断 from_wxid 是否为本 workspace 任一 current_version 域配置的决策人
/// （解析后的 decider_chain 成员，含旧 principal_decider 回落）。返回 Some(domain) 表示
/// 是决策人（domain 供调用方观测，webhooks 仅用 is_some 分流）；None 表示非决策人。
/// 从只查旧标量 principal_decider 改为复用 resolve_ask_human_policy——修复推荐配置
/// （只配 decider_chain）下领导回复不被识别的缺陷。
/// 🔒 关键：必须用入站消息自己的 workspace_id 约束查询——否则 A workspace 的领导 wxid
/// 若恰好也是 B workspace 某业务号的好友，B 收到他消息时会被误路由进 A 的请示流（跨域串扰）。
pub(crate) async fn lookup_principal_config(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    from_wxid: &str,
) -> AppResult<Option<String>> {
    use futures::TryStreamExt;
    if let Some(entry) = state
        .db
        .agent_principal_escalations()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": from_wxid,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.principal_account_id": account_id,
                "protocol.delivery_state": { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            },
            None,
        )
        .await?
    {
        return Ok(entry.protocol.map(|protocol| protocol.domain));
    }
    let mut cursor = state
        .db
        .operation_domain_configs()
        .find(
            doc! {
                "workspace_id": workspace_id,
                "current_version": true,
            },
            None,
        )
        .await?;
    while let Some(cfg) = cursor.try_next().await? {
        if crate::agent::escalation::policy::resolve_ask_human_policy(&cfg)
            .decider_chain
            .iter()
            .any(|decider| {
                decider.wxid == from_wxid
                    && decider
                        .account_id
                        .as_deref()
                        .is_none_or(|configured| configured == account_id)
            })
        {
            return Ok(Some(cfg.domain));
        }
    }
    Ok(None)
}

/// Idempotently materialize a durable relay intent as an immediately runnable task.
/// The task `_id` equals the escalation `_id`, so a crash or concurrent reconciler
/// can retry the upsert without creating a second relay.
pub(crate) async fn materialize_relay_task(
    state: &AppState,
    entry: &AgentPrincipalEscalation,
) -> AppResult<()> {
    if entry.status != PRINCIPAL_ESCALATION_STATUS_RESOLVED
        || entry.relay_state.as_deref() != Some(PRINCIPAL_RELAY_STATE_PENDING)
    {
        return Err(AppError::Conflict(
            "principal_relay_intent_not_pending".to_string(),
        ));
    }
    let escalation_id = entry
        .id
        .ok_or_else(|| AppError::External("principal escalation missing _id".to_string()))?;
    let task_id = entry
        .relay_task_id
        .ok_or_else(|| AppError::External("principal relay intent missing task id".to_string()))?;
    if task_id != escalation_id {
        return Err(AppError::Conflict(
            "principal_relay_task_identity_mismatch".to_string(),
        ));
    }
    let now = DateTime::now();
    let task = AgentTask {
        id: Some(task_id),
        workspace_id: entry.workspace_id.clone(),
        account_id: entry.account_id.clone(),
        contact_wxid: entry.contact_wxid.clone(),
        kind: "principal_decision_relay".to_string(),
        run_at: now,
        expires_at: None,
        content: entry.short_code.clone(),
        status: "pending".to_string(),
        source_decision_id: None,
        review_required: false,
        attempt_count: 0,
        max_attempts: 3,
        next_retry_at: None,
        gateway_status: None,
        cancel_reason: None,
        error: None,
        claimed_at: None,
        claim_recovery_count: 0,
        created_at: now,
        updated_at: now,
    };
    let mut task_doc = mongodb::bson::to_document(&task)?;
    task_doc.remove("_id");
    state
        .db
        .tasks()
        .update_one(
            doc! {
                "_id": task_id,
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "contact_wxid": &entry.contact_wxid,
                "kind": "principal_decision_relay",
                "content": &entry.short_code,
            },
            doc! { "$setOnInsert": task_doc },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;

    let marked = state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "_id": escalation_id,
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "relay_state": PRINCIPAL_RELAY_STATE_PENDING,
                "relay_task_id": task_id,
            },
            doc! { "$set": {
                "relay_state": PRINCIPAL_RELAY_STATE_ENQUEUED,
                "relay_enqueued_at": now,
                "updated_at": now,
            } },
            None,
        )
        .await?;
    if marked.modified_count == 0 {
        let current = state
            .db
            .agent_principal_escalations()
            .find_one(
                doc! {
                    "_id": escalation_id,
                    "relay_state": PRINCIPAL_RELAY_STATE_ENQUEUED,
                    "relay_task_id": task_id,
                },
                None,
            )
            .await?;
        if current.is_none() {
            return Err(AppError::Conflict(
                "principal_relay_intent_changed".to_string(),
            ));
        }
    }
    Ok(())
}

/// Recover new-protocol resolutions whose relay intent was persisted but whose
/// task materialization or acknowledgement was interrupted. Legacy resolved rows
/// without `relay_state` are deliberately ignored.
pub(crate) async fn reconcile_pending_relay_intents_once(state: &AppState) -> AppResult<u64> {
    use futures::TryStreamExt;

    let mut cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
                "relay_state": PRINCIPAL_RELAY_STATE_PENDING,
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "resolved_at": 1, "_id": 1 })
                .limit(100)
                .build(),
        )
        .await?;
    let mut reconciled = 0_u64;
    while let Some(entry) = cursor.try_next().await? {
        match materialize_relay_task(state, &entry).await {
            Ok(()) => reconciled += 1,
            Err(error) => {
                tracing::warn!(
                    short_code = %entry.short_code,
                    error = %error,
                    "principal relay intent reconciliation failed"
                );
            }
        }
    }
    Ok(reconciled)
}

/// 按 workspace + status 列请示台账（admin 收件箱/SLA 看板用），created_at 升序。
pub(crate) async fn list_escalations_by_workspace(
    state: &AppState,
    workspace_id: &str,
    status: &str,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! { "workspace_id": workspace_id, "status": status },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": 1 })
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// Atomically open the next delivery generation for a frozen decider.
/// The previous generation must already be terminal, so no still-runnable card
/// can race the reassignment. Delivery time is written only after Outbox confirms sent.
pub(crate) async fn reassign_escalation(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    expected_principal_wxid: &str,
    expected_generation: i64,
    to_wxid: &str,
    to_account_id: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let now = DateTime::now();
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "principal_wxid": expected_principal_wxid,
                "protocol.delivery_generation": expected_generation,
                "protocol.delivery_state": { "$in": [
                    PRINCIPAL_CARD_DELIVERY_SENT,
                    PRINCIPAL_CARD_DELIVERY_FAILED_TERMINAL,
                    PRINCIPAL_CARD_DELIVERY_UNKNOWN,
                ] },
            },
            doc! {
                "$set": {
                    "principal_wxid": to_wxid,
                    "protocol.principal_account_id": to_account_id,
                    "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_PENDING_ENQUEUE,
                    "updated_at": now,
                },
                "$inc": { "protocol.delivery_generation": 1i64 },
                "$unset": {
                    "protocol.delivery_outbox_id": "",
                    "last_pushed_at_ms": "",
                },
            },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(updated)
}

/// All new-protocol rows whose current card was confirmed delivered and whose
/// frozen policy has a timeout. Legacy rows are intentionally not guessed.
pub(crate) async fn list_timeout_eligible_escalations(
    state: &AppState,
) -> AppResult<Vec<AgentPrincipalEscalation>> {
    use futures::TryStreamExt;
    let cursor = state
        .db
        .agent_principal_escalations()
        .find(
            doc! {
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
                "protocol.delivery_state": PRINCIPAL_CARD_DELIVERY_SENT,
                "protocol.policy.timeoutHours": { "$type": "number" },
                "last_pushed_at_ms": { "$type": "number" },
            },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "last_pushed_at_ms": 1, "_id": 1 })
                .limit(500)
                .build(),
        )
        .await?;
    Ok(cursor.try_collect().await?)
}

/// 更新链尾安抚话术发送时刻（去重用）。仅 pending 可更新。
pub(crate) async fn touch_last_holding_reply_ms(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    now_ms: i64,
) -> AppResult<()> {
    state
        .db
        .agent_principal_escalations()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            doc! { "$set": { "last_holding_reply_ms": now_ms } },
            None,
        )
        .await?;
    Ok(())
}

/// 统计某决策人当日（since_ms 起）已被推送的请示卡数（骚扰门 daily_push_cap 用）。
/// 以 last_pushed_at_ms（首推+改派刷新）为推送时刻（每条 pending = 一次推卡）。
pub(crate) async fn count_pushes_today(
    state: &AppState,
    workspace_id: &str,
    principal_wxid: &str,
    since_ms: i64,
) -> AppResult<u32> {
    let count = state
        .db
        .agent_principal_escalations()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                // KD-05：用真实最近推送时刻，而非 created_at（改派后 created_at 不刷新会漏计）。
                "last_pushed_at_ms": { "$gte": since_ms },
            },
            None,
        )
        .await?;
    Ok(count as u32)
}

/// 查某决策人最近一次被推卡的时刻（毫秒）——骚扰门 dedupe_window_hours 用。
/// 以 last_pushed_at_ms（首推+改派刷新）作推送时刻（与 count_pushes_today 同口径）。
/// 无任何台账 → None（首次推卡，dedupe 不拦）。
pub(crate) async fn latest_push_ms(
    state: &AppState,
    workspace_id: &str,
    principal_wxid: &str,
) -> AppResult<Option<i64>> {
    let latest = state
        .db
        .agent_principal_escalations()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "principal_wxid": principal_wxid,
                "last_pushed_at_ms": { "$type": "number" },
            },
            mongodb::options::FindOneOptions::builder()
                // KD-05：按真实最近推送时刻排序取最近一次推卡时刻（改派刷新后才准）。
                .sort(doc! { "last_pushed_at_ms": -1 })
                .build(),
        )
        .await?;
    Ok(latest.and_then(|e| e.last_pushed_at_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awaiting_owner_pipelines_use_wire_field_names() {
        let activate = format!(
            "{:?}",
            activate_awaiting_owner_pipeline("owner-a", DateTime::from_millis(1))
        );
        let remove = format!(
            "{:?}",
            remove_awaiting_owner_pipeline("owner-a", DateTime::from_millis(1))
        );
        for rendered in [&activate, &remove] {
            assert!(rendered.contains(crate::models::AWAITING_PRINCIPAL_DECISION_ATTR));
            assert!(rendered.contains(crate::models::AWAITING_PRINCIPAL_DECISION_IDS_ATTR));
            assert!(!rendered.contains("awaiting_key"));
            assert!(!rendered.contains("owners_key"));
        }
        assert!(activate.contains("$setUnion"));
        assert!(remove.contains("$filter"));
        assert!(remove.contains("$ne"));
    }

    // stable_text_hash / self_anchor_for_substance 是复制自知识子系统的纯逻辑
    // （原 fn 是 pub(super)，escalation 模块跨模块不可见）。这里锁死"与原口径一致"
    // 及"满足 D2 门"两条不变量，防复制版本后续被误改偏离。
    // 完整两步法（insert + apply_chunk_revision）依赖 DB，本地磁盘纪律不跑 testcontainer，
    // 交 Task 5 联调 + 生产验证。

    #[test]
    fn stable_text_hash_matches_knowledge_subsystem_algorithm() {
        // 与 routes/knowledge/mod.rs:710 的 FNV-1a 64bit 同算法：确定性 + 16 位 hex。
        let h1 = stable_text_hash("foo");
        let h2 = stable_text_hash("foo");
        assert_eq!(h1, h2, "同输入必须同 hash");
        assert_eq!(h1.len(), 16, "16 位 hex");
        assert_ne!(h1, stable_text_hash("bar"), "不同输入不同 hash");
    }

    #[test]
    fn self_anchor_empty_substance_returns_none() {
        // 空/纯空白 substance 无从自锚 → None（调用方据此跳过沉淀，过不了 D2 门）。
        assert!(self_anchor_for_substance("").is_none());
        assert!(self_anchor_for_substance("   \n  ").is_none());
    }

    #[test]
    fn self_anchor_covers_full_substance_and_satisfies_d2() {
        let substance = "同意给这位客户 8 折优惠";
        let anchor = self_anchor_for_substance(substance).expect("非空 substance 必产锚点");
        // D2 门要求 source_anchors 非空且能定位来源：quote 即 substance 整段，start=行首。
        assert_eq!(anchor.get_i32("startOffset").unwrap(), 0, "锚点 start=行首");
        assert_eq!(
            anchor.get_i32("endOffset").unwrap() as usize,
            substance.len(),
            "锚点覆盖整段 substance"
        );
        assert_eq!(anchor.get_str("sourceQuote").unwrap(), substance);
        assert_eq!(anchor.get_i32("startLine").unwrap(), 1);
        assert_eq!(
            anchor.get_i32("endLine").unwrap(),
            1,
            "单行 substance endLine=1"
        );
        assert_eq!(
            anchor.get_str("quoteHash").unwrap(),
            stable_text_hash(substance),
            "quoteHash 与 stable_text_hash 口径一致"
        );
    }

    #[test]
    fn self_anchor_trims_before_anchoring() {
        // 前后空白被 trim：quote 为 trim 后文本，offset 从 0 起（自锚源即 quote 自身）。
        let anchor = self_anchor_for_substance("  报价 5000 元  ").expect("非空");
        assert_eq!(anchor.get_str("sourceQuote").unwrap(), "报价 5000 元");
        assert_eq!(anchor.get_i32("startOffset").unwrap(), 0);
        assert_eq!(
            anchor.get_i32("endOffset").unwrap() as usize,
            "报价 5000 元".len()
        );
    }

    #[test]
    fn self_anchor_multiline_end_line_counts_newlines() {
        // 多行 substance：endLine = 换行数 + 1（与 source_anchor_for_quote 同口径）。
        let anchor = self_anchor_for_substance("第一行\n第二行\n第三行").expect("非空");
        assert_eq!(anchor.get_i32("startLine").unwrap(), 1);
        assert_eq!(
            anchor.get_i32("endLine").unwrap(),
            3,
            "两个换行 → endLine=3"
        );
    }

    #[test]
    fn fallback_takes_first_sentence() {
        // 句号截断：只取首句
        let t = derive_sediment_title_fallback("同意给他八折。本周内付款有效。");
        assert_eq!(t, "同意给他八折");
    }

    #[test]
    fn fallback_no_terminator_takes_whole_when_short() {
        let t = derive_sediment_title_fallback("同意八折");
        assert_eq!(t, "同意八折");
    }

    #[test]
    fn fallback_truncates_long_by_chars_not_bytes() {
        // 41 个中文字符（多字节）应截到 40 + 省略号，且不 panic（按 chars 截断）
        let s = "一".repeat(41);
        let t = derive_sediment_title_fallback(&s);
        assert_eq!(t.chars().count(), 41); // 40 + '…'
        assert!(t.ends_with('…'));
    }

    #[test]
    fn fallback_empty_returns_safe_title() {
        assert_eq!(derive_sediment_title_fallback("   "), "领导授权沉淀");
    }

    #[test]
    fn fallback_newline_is_sentence_terminator() {
        let t = derive_sediment_title_fallback("同意八折\n补充说明若干");
        assert_eq!(t, "同意八折");
    }
}
