//! 用户运营引导路由：自然语言指令转配置预览与确认应用。

use axum::{extract::State, Extension, Json};
use mongodb::bson::{doc, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument, TransactionOptions};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::{ApiContact, UserOperationGuidePreview},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuidePreviewRequest {
    account_id: String,
    contact_id: String,
    instruction: String,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GuideApplyRequest {
    preview_id: String,
}

const GUIDE_APPLY_LEASE_MS: i64 = 5 * 60 * 1000;
const GUIDE_APPLY_PROTOCOL_VERSION: i32 = 2;

fn guide_claim_filter(
    preview_id: mongodb::bson::oid::ObjectId,
    workspace_id: &str,
    stale_before: DateTime,
) -> Document {
    doc! {
        "_id": preview_id,
        "workspace_id": workspace_id,
        "$or": [
            { "status": "pending" },
            {
                "status": "failed",
                "apply_protocol_version": GUIDE_APPLY_PROTOCOL_VERSION,
            },
            {
                "status": "applying",
                "apply_protocol_version": GUIDE_APPLY_PROTOCOL_VERSION,
                "apply_started_at": { "$lt": stale_before },
            }
        ],
    }
}

fn guide_owned_apply_filter(
    preview_id: mongodb::bson::oid::ObjectId,
    workspace_id: &str,
    apply_token: &str,
) -> Document {
    doc! {
        "_id": preview_id,
        "workspace_id": workspace_id,
        "status": "applying",
        "apply_token": apply_token,
    }
}

pub(super) async fn preview_user_operation_guide(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<GuidePreviewRequest>,
) -> AppResult<Json<Value>> {
    if payload.instruction.trim().is_empty() {
        return Err(AppError::BadRequest("instruction is required".to_string()));
    }
    validate_account(&state, &admin.current_workspace, &payload.account_id).await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &payload.contact_id).await?;
    if contact.account_id != payload.account_id {
        return Err(AppError::BadRequest(
            "contact does not belong to account".to_string(),
        ));
    }
    let memory = ensure_operating_memory(&state, &contact).await?;
    let latest_review = latest_decision_review(&state, &contact).await?;
    let playbook = agent::load_operation_playbook_for_contact(&state, &contact).await?;
    let (in_quiet_hours, next_wake_at, quiet_hours_enabled) =
        compute_quiet_hours_view(&state, &contact).await?;
    let health = operation_health_json(
        &contact,
        &memory,
        latest_review.as_ref(),
        in_quiet_hours,
        next_wake_at,
        quiet_hours_enabled,
    );
    // 注入合法值(治 LLM 产越界值的源头):状态机合法态 key + customer_stage/intent_level 字典 canonical。
    let domain_config = agent::load_user_operation_domain_config_for_contact(
        &state,
        &contact.workspace_id,
        &contact.wxid,
    )
    .await?;
    let legal_states: Vec<String> = agent::operation_states(domain_config.as_ref())
        .iter()
        .filter_map(|d| d.get_str("key").ok().map(String::from))
        .collect();
    let cache = agent::taxonomy::global_taxonomy_cache(&state.db);
    cache
        .find_or_load(&state.db, &admin.current_workspace)
        .await; // 冷/过期缓存返回空,先 load(幂等自愈)
    let stage_values = agent::taxonomy::dimension_values_with_labels(
        &admin.current_workspace,
        "customer_stage",
        &contact.account_id,
        cache.as_ref(),
    );
    let intent_values = agent::taxonomy::dimension_values_with_labels(
        &admin.current_workspace,
        "intent_level",
        &contact.account_id,
        cache.as_ref(),
    );
    let system = "你是微信私域用户运营产品里的 AI 引导助手。你的职责不是直接写聊天回复，而是根据运营人员的自然语言指令，生成一份可确认的配置修改预览。必须输出严格 JSON。";
    let user = build_guide_preview_prompt(
        &payload.instruction,
        payload.mode.as_deref().unwrap_or("smart"),
        &contact,
        &memory,
        playbook.as_ref(),
        latest_review.as_ref(),
        &health,
        &legal_states,
        &stage_values,
        &intent_values,
    );
    let generated = agent::generate_agent_json(
        &state,
        Some(&payload.account_id),
        Some(&contact.wxid),
        None,
        "user.guide.preview",
        system,
        &user,
    )
    .await?;
    let summary = json_string_any(&generated, &["summary"])
        .unwrap_or_else(|| "已生成运营优化预览。".to_string());
    let impact_scope = json_string_any(&generated, &["impactScope", "impact_scope"])
        .unwrap_or_else(|| "current_contact".to_string());
    let scope_reason = json_string_any(&generated, &["scopeReason", "scope_reason"])
        .unwrap_or_else(|| "默认只影响当前好友，确认后不会改动其他用户。".to_string());
    let health_scores = json_document_any(&generated, &["healthScores", "health_scores"])
        .unwrap_or_else(|| health_scores_document(&contact, &memory, latest_review.as_ref()));
    let suggested_changes =
        json_document_any(&generated, &["suggestedChanges", "suggested_changes"])
            .unwrap_or_else(Document::new);
    let readable_changes =
        json_string_vec_any(&generated, &["readableChanges", "readable_changes"]);
    let risk_warnings = json_string_vec_any(&generated, &["riskWarnings", "risk_warnings"]);
    let preview = UserOperationGuidePreview {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id,
        contact_id: contact
            .id
            .ok_or_else(|| AppError::External("contact id missing".to_string()))?,
        contact_wxid: contact.wxid,
        instruction: payload.instruction,
        mode: payload.mode.unwrap_or_else(|| "smart".to_string()),
        status: "pending".to_string(),
        summary,
        impact_scope,
        scope_reason,
        readable_changes,
        health_scores,
        suggested_changes,
        risk_warnings,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let result = state
        .db
        .user_operation_guide_previews()
        .insert_one(preview, None)
        .await?;
    let id = result
        .inserted_id
        .as_object_id()
        .ok_or_else(|| AppError::External("guide preview id missing".to_string()))?;
    let stored = state
        .db
        .user_operation_guide_previews()
        .find_one(doc! { "_id": id }, None)
        .await?
        .ok_or_else(|| AppError::External("guide preview missing after insert".to_string()))?;
    Ok(Json(json!({ "item": guide_preview_json(stored) })))
}

pub(super) async fn apply_user_operation_guide(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<GuideApplyRequest>,
) -> AppResult<Json<Value>> {
    let preview_id = parse_object_id(&payload.preview_id)?;
    let previews = state.db.user_operation_guide_previews();
    let apply_token = uuid::Uuid::new_v4().to_string();
    let now = DateTime::now();
    let stale_before = DateTime::from_millis(now.timestamp_millis() - GUIDE_APPLY_LEASE_MS);
    // 原子 lease：pending 或超时 applying 才能被 claim。token 隔离旧 worker；即使旧
    // worker 在 lease 过期后恢复，它的事务 finalize CAS 也会失败并整体回滚。
    let preview = previews
        .find_one_and_update(
            guide_claim_filter(preview_id, &admin.current_workspace, stale_before),
            doc! {
                "$set": {
                    "status": "applying",
                    "apply_protocol_version": GUIDE_APPLY_PROTOCOL_VERSION,
                    "apply_token": &apply_token,
                    "apply_started_at": now,
                    "updated_at": now,
                },
                "$unset": { "apply_error": "" },
            },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let Some(preview) = preview else {
        // 保持跨 workspace 404（不泄漏存在性）；本 workspace 内已被 claim 或已终态则
        // 返回 409，明确告诉调用方这不是可重放操作。
        let existing = previews
            .find_one(
                doc! { "_id": preview_id, "workspace_id": &admin.current_workspace },
                None,
            )
            .await?;
        return match existing {
            Some(item) => Err(AppError::Conflict(format!(
                "guide_preview_not_pending:{}",
                item.status
            ))),
            None => Err(AppError::NotFound("guide preview not found".to_string())),
        };
    };

    let result = apply_claimed_user_operation_guide(
        &state,
        &preview,
        preview_id,
        &payload.preview_id,
        &apply_token,
    )
    .await;
    if let Err(error) = &result {
        // 仅本 lease 的 applying 可转 failed。事务失败已回滚全部业务写；若 token 已被
        // stale reclaim 替换，旧 worker 不得覆盖新执行者状态。
        let error_summary: String = error.to_string().chars().take(500).collect();
        if let Err(mark_error) = previews
            .update_one(
                guide_owned_apply_filter(preview_id, &admin.current_workspace, &apply_token),
                doc! {
                    "$set": {
                        "status": "failed",
                        "apply_error": error_summary,
                        "updated_at": DateTime::now(),
                    }
                },
                None,
            )
            .await
        {
            tracing::error!(
                preview_id = %preview_id,
                ?mark_error,
                "failed to persist guide apply failure state"
            );
        }
    }
    result
}

async fn apply_claimed_user_operation_guide(
    state: &AppState,
    preview: &UserOperationGuidePreview,
    preview_id: mongodb::bson::oid::ObjectId,
    preview_id_text: &str,
    apply_token: &str,
) -> AppResult<Json<Value>> {
    debug_assert_eq!(preview.status, "applying");
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "_id": preview.contact_id,
                "workspace_id": &preview.workspace_id,
                "account_id": &preview.account_id
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found".to_string()))?;
    // 所有校验和 merge 在事务前完成；这里只准备确定的 BSON 更新，不产生 guide 副作用。
    let (contact_set, skipped) =
        prepare_contact_changes(state, &contact, &preview.suggested_changes).await?;
    // appliedFields = suggestedChanges 顶层键 - 被跳过的字段(给前端"已应用 N 项"用)。
    let skipped_names: std::collections::HashSet<&str> =
        skipped.iter().map(|s| s.field.as_str()).collect();
    let applied_fields: Vec<String> = preview
        .suggested_changes
        .keys()
        .filter(|k| !skipped_names.contains(k.as_str()))
        .cloned()
        .collect();
    let skipped_json: Vec<Value> = skipped
        .iter()
        .map(|s| json!({ "field": s.field, "reason": s.reason }))
        .collect();
    // ensure 可能创建基础 memory，但建议 patch 尚未写入；初始化本身幂等且不属于 guide
    // 业务变更。真正的 patch 与其余副作用全部在下方同一事务中。
    let memory_before = ensure_operating_memory(state, &contact).await?;
    let memory_set = prepare_memory_changes(&memory_before, &preview.suggested_changes);
    let playbook_update = prepare_playbook_changes(&contact, &preview.suggested_changes);
    let playbook_expected_version = if let Some((playbook_id, _)) = &playbook_update {
        Some(
            state
                .db
                .operation_playbooks()
                .find_one(
                    doc! {
                        "_id": playbook_id,
                        "workspace_id": &contact.workspace_id,
                        "account_id": &contact.account_id,
                    },
                    None,
                )
                .await?
                .ok_or_else(|| AppError::Conflict("guide_playbook_changed".to_string()))?
                .version,
        )
    } else {
        None
    };
    let domain_update =
        prepare_domain_changes(state, &preview.workspace_id, &preview.suggested_changes).await?;

    let client = state.db.client();
    let mut session = client.start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let transaction_result: AppResult<()> = async {
        if !contact_set.is_empty() {
            let result = state
                .db
                .contacts()
                .update_one_with_session(
                    doc! {
                        "_id": preview.contact_id,
                        "workspace_id": &preview.workspace_id,
                        "account_id": &preview.account_id,
                        "updated_at": contact.updated_at,
                    },
                    doc! { "$set": contact_set },
                    None,
                    &mut session,
                )
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::Conflict("guide_contact_changed".to_string()));
            }
        }
        if !memory_set.is_empty() {
            let result = state
                .db
                .operating_memories()
                .update_one_with_session(
                    doc! {
                        "workspace_id": &contact.workspace_id,
                        "account_id": &contact.account_id,
                        "contact_wxid": &contact.wxid,
                        "updated_at": memory_before.updated_at,
                    },
                    doc! { "$set": memory_set },
                    None,
                    &mut session,
                )
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::Conflict("guide_memory_changed".to_string()));
            }
        }
        if let Some((playbook_id, set_doc)) = playbook_update {
            let result = state
                .db
                .operation_playbooks()
                .update_one_with_session(
                    doc! {
                        "_id": playbook_id,
                        "workspace_id": &contact.workspace_id,
                        "account_id": &contact.account_id,
                        "version": playbook_expected_version.expect("version loaded with update"),
                    },
                    doc! { "$set": set_doc, "$inc": { "version": 1 } },
                    None,
                    &mut session,
                )
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::Conflict("guide_playbook_changed".to_string()));
            }
        }
        if let Some((config_id, runtime, expected_updated_at)) = domain_update {
            let result = state
                .db
                .operation_domain_configs()
                .update_one_with_session(
                    doc! {
                        "_id": config_id,
                        "workspace_id": &preview.workspace_id,
                        "domain": "user_operations",
                        "current_version": true,
                        "updated_at": expected_updated_at,
                    },
                    doc! { "$set": {
                        "runtime_parameters": runtime,
                        "updated_at": DateTime::now(),
                    } },
                    None,
                    &mut session,
                )
                .await?;
            if result.matched_count != 1 {
                return Err(AppError::Conflict(
                    "current user_operations domain config changed during guide apply".to_string(),
                ));
            }
        }
        state
            .db
            .events()
            .insert_one_with_session(
                crate::models::AgentEvent {
                    id: None,
                    workspace_id: preview.workspace_id.clone(),
                    account_id: preview.account_id.clone(),
                    contact_wxid: Some(preview.contact_wxid.clone()),
                    kind: "user_operation_guide_applied".to_string(),
                    status: "succeeded".to_string(),
                    summary: preview.summary.clone(),
                    details: Some(doc! {
                        "previewId": preview_id_text,
                        "instruction": &preview.instruction,
                        "impactScope": &preview.impact_scope,
                        "scopeReason": &preview.scope_reason,
                        "readableChanges": &preview.readable_changes,
                        "suggestedChanges": &preview.suggested_changes,
                        "skippedFields": mongodb::bson::to_bson(&skipped_json).unwrap_or(mongodb::bson::Bson::Array(vec![]))
                    }),
                    created_at: DateTime::now(),
                    dedupe_key: Some(format!("guide_apply:{preview_id}")),
                },
                None,
                &mut session,
            )
            .await?;
        let finalized = state
            .db
            .user_operation_guide_previews()
            .update_one_with_session(
                guide_owned_apply_filter(preview_id, &preview.workspace_id, apply_token),
                doc! {
                    "$set": {
                        "status": "applied",
                        "applied_at": DateTime::now(),
                        "updated_at": DateTime::now(),
                    },
                    "$unset": { "apply_error": "", "apply_token": "" },
                },
                None,
                &mut session,
            )
            .await?;
        if finalized.modified_count != 1 {
            return Err(AppError::Conflict("guide_preview_lease_lost".to_string()));
        }
        Ok(())
    }
    .await;
    if let Err(error) = transaction_result {
        let _ = session.abort_transaction().await;
        return Err(error);
    }
    loop {
        match session.commit_transaction().await {
            Ok(()) => break,
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => return Err(error.into()),
        }
    }

    let updated_contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "_id": preview.contact_id,
                "workspace_id": &preview.workspace_id,
                "account_id": &preview.account_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found after guide apply".to_string()))?;
    let memory = ensure_operating_memory(state, &updated_contact).await?;
    let latest_review = latest_decision_review(state, &updated_contact).await?;
    let (in_quiet_hours, next_wake_at, quiet_hours_enabled) =
        compute_quiet_hours_view(state, &updated_contact).await?;
    let health = operation_health_json(
        &updated_contact,
        &memory,
        latest_review.as_ref(),
        in_quiet_hours,
        next_wake_at,
        quiet_hours_enabled,
    );
    Ok(Json(json!({
        "item": {
            "contact": ApiContact::from(updated_contact),
            "operatingMemory": operating_memory_json(memory),
            "health": health,
            "appliedFields": applied_fields,
            "skippedFields": skipped_json
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::{guide_claim_filter, guide_owned_apply_filter, GUIDE_APPLY_PROTOCOL_VERSION};
    use mongodb::bson::{doc, oid::ObjectId, DateTime};

    #[test]
    fn guide_claim_filter_allows_retry_and_stale_recovery() {
        let id = ObjectId::new();
        let stale_before = DateTime::from_millis(1_700_000_000_000);
        assert_eq!(
            guide_claim_filter(id, "ws-a", stale_before),
            doc! {
                "_id": id,
                "workspace_id": "ws-a",
                "$or": [
                    { "status": "pending" },
                    {
                        "status": "failed",
                        "apply_protocol_version": GUIDE_APPLY_PROTOCOL_VERSION,
                    },
                    {
                        "status": "applying",
                        "apply_protocol_version": GUIDE_APPLY_PROTOCOL_VERSION,
                        "apply_started_at": { "$lt": stale_before },
                    }
                ],
            }
        );
    }

    #[test]
    fn guide_owned_apply_filter_fences_old_worker() {
        let id = ObjectId::new();
        assert_eq!(
            guide_owned_apply_filter(id, "ws-a", "lease-1"),
            doc! {
                "_id": id,
                "workspace_id": "ws-a",
                "status": "applying",
                "apply_token": "lease-1",
            }
        );
    }
}
