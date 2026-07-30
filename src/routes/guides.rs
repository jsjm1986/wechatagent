//! 用户运营引导路由：自然语言指令转配置预览与确认应用。

use axum::{extract::State, Extension, Json};
use mongodb::bson::{self, doc, Bson, DateTime, Document};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument, TransactionOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::{
        Contact, GuideApplyReceipt, GuideAuthoritativeChange, GuideFrozenPlan, GuideSkippedField,
        OperatingMemory, OperationDomainConfig, OperationPlaybook, UserOperationGuidePreview,
    },
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
    expected_account_id: String,
    expected_contact_id: String,
    candidate_hash: String,
    #[serde(default)]
    confirm_global_impact: bool,
}

const GUIDE_APPLY_LEASE_MS: i64 = 5 * 60 * 1000;
const GUIDE_APPLY_PROTOCOL_VERSION: i32 = 3;
const GUIDE_APPLY_GUARD_FIELD: &str = "_guide_apply_guard";

fn guide_claim_filter(
    preview_id: mongodb::bson::oid::ObjectId,
    workspace_id: &str,
    account_id: &str,
    contact_id: mongodb::bson::oid::ObjectId,
    candidate_hash: &str,
    stale_before: DateTime,
) -> Document {
    doc! {
        "_id": preview_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_id": contact_id,
        "candidate_hash": candidate_hash,
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

fn candidate_hash(
    workspace_id: &str,
    account_id: &str,
    contact_id: mongodb::bson::oid::ObjectId,
    plan: &GuideFrozenPlan,
) -> AppResult<String> {
    let bytes = bson::to_vec(&doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_id": contact_id,
        "frozen_plan": bson::to_bson(plan)?,
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn receipt_json(receipt: &GuideApplyReceipt) -> Value {
    json!({
        "committed": true,
        "previewId": receipt.preview_id.to_hex(),
        "candidateHash": receipt.candidate_hash,
        "committedAt": crate::models::dt_to_string(receipt.committed_at),
        "appliedFields": receipt.applied_fields,
        "skippedFields": receipt.skipped_fields,
        "impactScope": receipt.impact_scope,
    })
}

fn validated_apply_receipt<'a>(
    preview: &'a UserOperationGuidePreview,
    requested_hash: &str,
) -> AppResult<&'a GuideApplyReceipt> {
    let preview_id = preview
        .id
        .ok_or_else(|| AppError::Conflict("guide_preview_missing_id".to_string()))?;
    let plan = preview.frozen_plan.as_ref().ok_or_else(|| {
        AppError::Conflict("guide_preview_legacy_requires_regeneration".to_string())
    })?;
    let stored_hash = preview
        .candidate_hash
        .as_deref()
        .ok_or_else(|| AppError::Conflict("guide_preview_missing_candidate_hash".to_string()))?;
    if stored_hash != requested_hash
        || candidate_hash(
            &preview.workspace_id,
            &preview.account_id,
            preview.contact_id,
            plan,
        )? != stored_hash
    {
        return Err(AppError::Conflict(
            "guide_preview_candidate_hash_mismatch".to_string(),
        ));
    }
    let (authoritative_scope, _, _) = plan_impact(plan);
    if preview.impact_scope != authoritative_scope {
        return Err(AppError::Conflict(
            "guide_preview_scope_mismatch".to_string(),
        ));
    }
    let receipt = preview
        .apply_receipt
        .as_ref()
        .ok_or_else(|| AppError::Conflict("guide_preview_missing_receipt".to_string()))?;
    if receipt.preview_id != preview_id
        || receipt.candidate_hash != stored_hash
        || receipt.impact_scope != authoritative_scope
        || receipt.applied_fields != plan.applied_fields
        || receipt.skipped_fields != plan.skipped_fields
    {
        return Err(AppError::Conflict(
            "guide_preview_receipt_mismatch".to_string(),
        ));
    }
    Ok(receipt)
}

fn optional_string(value: Option<&str>) -> Bson {
    value
        .map(|value| Bson::String(value.to_string()))
        .unwrap_or(Bson::Null)
}

fn contact_value(contact: &Contact, key: &str) -> Bson {
    if let Some(attribute) = key.strip_prefix("domain_attributes.") {
        return contact
            .domain_attributes
            .as_ref()
            .and_then(|values| values.get(attribute))
            .cloned()
            .unwrap_or(Bson::Null);
    }
    match key {
        "human_profile_note" => optional_string(contact.human_profile_note.as_deref()),
        "manual_tags" => bson::to_bson(&contact.manual_tags).unwrap_or(Bson::Array(Vec::new())),
        "follow_up_policy" => optional_string(contact.follow_up_policy.as_deref()),
        "operation_state" => optional_string(contact.operation_state.as_deref()),
        "operation_state_reason" => optional_string(contact.operation_state_reason.as_deref()),
        "operation_policy" => Bson::Document(contact.operation_policy.clone()),
        _ => Bson::Null,
    }
}

fn memory_value(memory: &OperatingMemory, key: &str) -> Bson {
    match key {
        "user_understanding" => Bson::Document(memory.user_understanding.clone()),
        "relationship_state" => Bson::Document(memory.relationship_state.clone()),
        "product_fit" => Bson::Document(memory.product_fit.clone()),
        "next_action" => Bson::Document(memory.next_action.clone()),
        _ => Bson::Null,
    }
}

fn playbook_value(playbook: &OperationPlaybook, key: &str) -> Bson {
    match key {
        "reply_style" => optional_string(playbook.reply_style.as_deref()),
        "follow_up_method" => optional_string(playbook.follow_up_method.as_deref()),
        "forbidden_rules" => optional_string(playbook.forbidden_rules.as_deref()),
        "success_criteria" => optional_string(playbook.success_criteria.as_deref()),
        _ => Bson::Null,
    }
}

fn strip_timestamp_fields(set: &mut Document) -> Vec<String> {
    let fields: Vec<String> = set
        .keys()
        .filter(|key| *key == "updated_at" || key.ends_with("_updated_at"))
        .cloned()
        .collect();
    for field in &fields {
        set.remove(field);
    }
    fields
}

fn retain_effective_changes<F>(
    target: &str,
    set: &mut Document,
    current: F,
    changes: &mut Vec<GuideAuthoritativeChange>,
) where
    F: Fn(&str) -> Bson,
{
    let keys: Vec<String> = set.keys().cloned().collect();
    for key in keys {
        let before = current(&key);
        let after = set.get(&key).cloned().unwrap_or(Bson::Null);
        if before == after {
            set.remove(&key);
        } else {
            changes.push(GuideAuthoritativeChange {
                target: target.to_string(),
                field: key.clone(),
                label: key,
                before,
                after,
            });
        }
    }
}

fn suggested_field_for_contact_key(key: &str) -> &'static str {
    match key {
        "human_profile_note" => "humanProfileNote",
        "manual_tags" => "tags",
        "follow_up_policy" => "followUpPolicy",
        "operation_state" => "operationState",
        "operation_state_reason" => "operationStateReason",
        "operation_policy" => "operationPolicy",
        key if key.ends_with("customer_stage") => "customerStage",
        key if key.ends_with("intent_level") => "intentLevel",
        _ => "contact",
    }
}

async fn freeze_guide_plan(
    state: &AppState,
    contact: &Contact,
    memory: &OperatingMemory,
    playbook: Option<&OperationPlaybook>,
    domain_config: Option<&OperationDomainConfig>,
    suggested_changes: &Document,
) -> AppResult<GuideFrozenPlan> {
    let (mut contact_set, skipped) =
        prepare_contact_changes(state, contact, suggested_changes).await?;
    let mut contact_timestamp_fields = strip_timestamp_fields(&mut contact_set);
    let mut authoritative_changes = Vec::new();
    retain_effective_changes(
        "contact",
        &mut contact_set,
        |key| contact_value(contact, key),
        &mut authoritative_changes,
    );
    if contact_set.is_empty() {
        contact_timestamp_fields.clear();
    }

    let mut memory_set = prepare_memory_changes(memory, suggested_changes);
    let mut memory_timestamp_fields = strip_timestamp_fields(&mut memory_set);
    retain_effective_changes(
        "operatingMemory",
        &mut memory_set,
        |key| memory_value(memory, key),
        &mut authoritative_changes,
    );
    if memory_set.is_empty() {
        memory_timestamp_fields.clear();
    }

    let playbook_id = playbook.and_then(|value| value.id);
    let playbook_version = playbook.map(|value| value.version);
    let mut playbook_set = Document::new();
    let mut playbook_timestamp_fields = Vec::new();
    if let Some((id, mut set)) = prepare_playbook_changes(contact, suggested_changes) {
        set.remove("created_by");
        playbook_timestamp_fields = strip_timestamp_fields(&mut set);
        let bound = playbook
            .filter(|value| value.id == Some(id))
            .ok_or_else(|| AppError::Conflict("guide_playbook_changed".to_string()))?;
        retain_effective_changes(
            "playbook",
            &mut set,
            |key| playbook_value(bound, key),
            &mut authoritative_changes,
        );
        if !set.is_empty() {
            playbook_set = set;
        } else {
            playbook_timestamp_fields.clear();
        }
    }

    let mut domain_config_id = domain_config.and_then(|value| value.id);
    let mut domain_version = domain_config.map(|value| value.version);
    let mut domain_updated_at = domain_config.map(|value| value.updated_at);
    let mut domain_runtime_parameters = None;
    if let Some((id, runtime, updated_at, version)) =
        prepare_domain_changes(state, &contact.workspace_id, suggested_changes).await?
    {
        let current = domain_config
            .filter(|value| value.id == Some(id) && value.version == version)
            .ok_or_else(|| AppError::Conflict("guide_domain_changed".to_string()))?;
        if current.runtime_parameters != runtime {
            let keys: BTreeSet<String> = current
                .runtime_parameters
                .keys()
                .chain(runtime.keys())
                .cloned()
                .collect();
            for key in keys {
                let before = current
                    .runtime_parameters
                    .get(&key)
                    .cloned()
                    .unwrap_or(Bson::Null);
                let after = runtime.get(&key).cloned().unwrap_or(Bson::Null);
                if before != after {
                    authoritative_changes.push(GuideAuthoritativeChange {
                        target: "workspaceRuntime".to_string(),
                        field: key.clone(),
                        label: key,
                        before,
                        after,
                    });
                }
            }
            domain_config_id = Some(id);
            domain_version = Some(version);
            domain_updated_at = Some(updated_at);
            domain_runtime_parameters = Some(runtime);
        }
    }

    let playbook_affected_contacts = if !playbook_set.is_empty() {
        let id =
            playbook_id.ok_or_else(|| AppError::Conflict("guide_playbook_changed".to_string()))?;
        state
            .db
            .contacts()
            .count_documents(
                doc! {
                    "workspace_id": &contact.workspace_id,
                    "account_id": &contact.account_id,
                    "playbook_id": id,
                },
                None,
            )
            .await? as i64
    } else {
        0
    };

    let mut applied = BTreeSet::new();
    for key in contact_set.keys() {
        applied.insert(suggested_field_for_contact_key(key).to_string());
    }
    if !memory_set.is_empty() {
        applied.insert("memory".to_string());
    }
    if !playbook_set.is_empty() {
        applied.insert("playbookPatch".to_string());
    }
    if domain_runtime_parameters.is_some() {
        applied.insert("domainRuntimeParameters".to_string());
    }

    let mut skipped_fields: Vec<GuideSkippedField> =
        skipped.iter().map(GuideSkippedField::from).collect();
    let allowed = [
        "humanProfileNote",
        "tags",
        "customerStage",
        "intentLevel",
        "followUpPolicy",
        "operationState",
        "operationStateReason",
        "operationPolicy",
        "memory",
        "playbookPatch",
        "domainRuntimeParameters",
    ];
    for key in suggested_changes.keys() {
        if !allowed.contains(&key.as_str()) {
            skipped_fields.push(GuideSkippedField {
                field: key.clone(),
                reason: "unsupported guide field".to_string(),
            });
        }
    }
    if suggested_changes.contains_key("playbookPatch") && playbook_set.is_empty() {
        skipped_fields.push(GuideSkippedField {
            field: "playbookPatch".to_string(),
            reason: "no effective change to a bound playbook".to_string(),
        });
    }

    Ok(GuideFrozenPlan {
        contact_updated_at: contact.updated_at,
        memory_updated_at: memory.updated_at,
        playbook_id,
        playbook_version,
        domain_config_id,
        domain_version,
        domain_updated_at,
        contact_set,
        contact_timestamp_fields,
        memory_set,
        memory_timestamp_fields,
        playbook_set,
        playbook_timestamp_fields,
        domain_runtime_parameters,
        applied_fields: applied.into_iter().collect(),
        skipped_fields,
        authoritative_changes,
        playbook_affected_contacts,
    })
}

fn plan_impact(plan: &GuideFrozenPlan) -> (String, String, Vec<String>) {
    if plan.domain_runtime_parameters.is_some() {
        return (
            "workspace_user_operations".to_string(),
            "Changes the current workspace user-operations runtime configuration.".to_string(),
            vec!["Workspace-wide runtime behavior will change.".to_string()],
        );
    }
    if !plan.playbook_set.is_empty() {
        return (
            "shared_playbook".to_string(),
            format!(
                "Changes a shared playbook; {} contacts were bound at preview time and future bindings also inherit it.",
                plan.playbook_affected_contacts
            ),
            vec!["A shared playbook will change for more than this contact.".to_string()],
        );
    }
    (
        "current_contact".to_string(),
        "Only the selected contact and its operating memory are changed.".to_string(),
        Vec::new(),
    )
}

fn requires_strong_confirmation(scope: &str) -> bool {
    scope != "current_contact"
}

fn materialize_set(source: &Document, timestamp_fields: &[String], now: DateTime) -> Document {
    let mut result = source.clone();
    for field in timestamp_fields {
        result.insert(field, now);
    }
    result.insert("updated_at", now);
    result
}

fn guide_owned_apply_filter(
    preview_id: mongodb::bson::oid::ObjectId,
    workspace_id: &str,
    account_id: &str,
    contact_id: mongodb::bson::oid::ObjectId,
    apply_token: &str,
) -> Document {
    doc! {
        "_id": preview_id,
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_id": contact_id,
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
        .await?; // 冷/过期缓存先 load；损坏 current 指针 fail closed
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
        &contact.workspace_id,
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
    let health_scores = json_document_any(&generated, &["healthScores", "health_scores"])
        .unwrap_or_else(|| health_scores_document(&contact, &memory, latest_review.as_ref()));
    let suggested_changes =
        json_document_any(&generated, &["suggestedChanges", "suggested_changes"])
            .unwrap_or_else(Document::new);
    let frozen_plan = freeze_guide_plan(
        &state,
        &contact,
        &memory,
        playbook.as_ref(),
        domain_config.as_ref(),
        &suggested_changes,
    )
    .await?;
    let (impact_scope, scope_reason, risk_warnings) = plan_impact(&frozen_plan);
    let readable_changes = frozen_plan
        .authoritative_changes
        .iter()
        .map(|change| format!("{} / {}", change.target, change.label))
        .collect();
    let contact_id = contact
        .id
        .ok_or_else(|| AppError::External("contact id missing".to_string()))?;
    let frozen_hash = candidate_hash(
        &admin.current_workspace,
        &payload.account_id,
        contact_id,
        &frozen_plan,
    )?;
    let preview = UserOperationGuidePreview {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: payload.account_id,
        contact_id,
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
        frozen_plan: Some(frozen_plan),
        candidate_hash: Some(frozen_hash),
        apply_receipt: None,
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
    let expected_account_id = payload.expected_account_id.trim();
    if expected_account_id.is_empty() {
        return Err(AppError::BadRequest(
            "expectedAccountId is required".to_string(),
        ));
    }
    let expected_contact_id = parse_object_id(&payload.expected_contact_id)?;
    let requested_hash = payload.candidate_hash.trim();
    if requested_hash.is_empty() {
        return Err(AppError::BadRequest(
            "candidateHash is required".to_string(),
        ));
    }

    let previews = state.db.user_operation_guide_previews();
    let identity_filter = doc! {
        "_id": preview_id,
        "workspace_id": &admin.current_workspace,
        "account_id": expected_account_id,
        "contact_id": expected_contact_id,
    };
    let existing = previews.find_one(identity_filter.clone(), None).await?;
    let Some(existing) = existing else {
        let exists_in_workspace = previews
            .count_documents(
                doc! { "_id": preview_id, "workspace_id": &admin.current_workspace },
                None,
            )
            .await?
            == 1;
        return if exists_in_workspace {
            Err(AppError::Conflict(
                "guide_preview_identity_conflict".to_string(),
            ))
        } else {
            Err(AppError::NotFound("guide preview not found".to_string()))
        };
    };

    if existing.status == "applied" {
        let receipt = validated_apply_receipt(&existing, requested_hash)?;
        return Ok(Json(json!({ "item": receipt_json(receipt) })));
    }

    let plan = existing.frozen_plan.as_ref().ok_or_else(|| {
        AppError::Conflict("guide_preview_legacy_requires_regeneration".to_string())
    })?;
    let stored_hash = existing
        .candidate_hash
        .as_deref()
        .ok_or_else(|| AppError::Conflict("guide_preview_missing_candidate_hash".to_string()))?;
    if stored_hash != requested_hash
        || candidate_hash(
            &existing.workspace_id,
            &existing.account_id,
            existing.contact_id,
            plan,
        )? != stored_hash
    {
        return Err(AppError::Conflict(
            "guide_preview_candidate_hash_mismatch".to_string(),
        ));
    }
    let (authoritative_scope, _, _) = plan_impact(plan);
    if existing.impact_scope != authoritative_scope {
        return Err(AppError::Conflict(
            "guide_preview_scope_mismatch".to_string(),
        ));
    }
    if requires_strong_confirmation(&authoritative_scope) && !payload.confirm_global_impact {
        return Err(AppError::BadRequest(
            "guide_global_confirmation_required".to_string(),
        ));
    }

    let apply_token = uuid::Uuid::new_v4().to_string();
    let now = DateTime::now();
    let stale_before = DateTime::from_millis(now.timestamp_millis() - GUIDE_APPLY_LEASE_MS);
    let preview = previews
        .find_one_and_update(
            guide_claim_filter(
                preview_id,
                &admin.current_workspace,
                expected_account_id,
                expected_contact_id,
                requested_hash,
                stale_before,
            ),
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
        let current = previews.find_one(identity_filter, None).await?;
        if let Some(item) = current {
            if item.status == "applied" {
                let receipt = validated_apply_receipt(&item, requested_hash)?;
                return Ok(Json(json!({ "item": receipt_json(receipt) })));
            }
            return Err(AppError::Conflict(format!(
                "guide_preview_not_pending:{}",
                item.status
            )));
        }
        return Err(AppError::NotFound("guide preview not found".to_string()));
    };

    let result =
        apply_claimed_user_operation_guide_v3(&state, &admin, &preview, preview_id, &apply_token)
            .await;
    if let Err(error) = &result {
        let error_summary: String = error.to_string().chars().take(500).collect();
        let terminal_status = match error {
            AppError::Conflict(code) if code.contains("changed") || code.contains("stale") => {
                "stale"
            }
            _ => "failed",
        };
        if let Err(mark_error) = previews
            .update_one(
                guide_owned_apply_filter(
                    preview_id,
                    &admin.current_workspace,
                    expected_account_id,
                    expected_contact_id,
                    &apply_token,
                ),
                doc! {
                    "$set": {
                        "status": terminal_status,
                        "apply_error": error_summary,
                        "updated_at": DateTime::now(),
                    },
                    "$unset": { "apply_token": "" },
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
    let receipt = result?;
    Ok(Json(json!({ "item": receipt_json(&receipt) })))
}

async fn apply_claimed_user_operation_guide_v3(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    preview: &UserOperationGuidePreview,
    preview_id: mongodb::bson::oid::ObjectId,
    apply_token: &str,
) -> AppResult<GuideApplyReceipt> {
    debug_assert_eq!(preview.status, "applying");
    let plan = preview.frozen_plan.as_ref().ok_or_else(|| {
        AppError::Conflict("guide_preview_legacy_requires_regeneration".to_string())
    })?;
    let stored_hash = preview
        .candidate_hash
        .as_deref()
        .ok_or_else(|| AppError::Conflict("guide_preview_missing_candidate_hash".to_string()))?;
    if candidate_hash(
        &preview.workspace_id,
        &preview.account_id,
        preview.contact_id,
        plan,
    )? != stored_hash
    {
        return Err(AppError::Conflict(
            "guide_preview_candidate_hash_mismatch".to_string(),
        ));
    }
    let (authoritative_scope, _, _) = plan_impact(plan);
    if preview.impact_scope != authoritative_scope {
        return Err(AppError::Conflict(
            "guide_preview_scope_mismatch".to_string(),
        ));
    }

    let committed_at = DateTime::now();
    let receipt = GuideApplyReceipt {
        preview_id,
        candidate_hash: stored_hash.to_string(),
        committed_at,
        applied_fields: plan.applied_fields.clone(),
        skipped_fields: plan.skipped_fields.clone(),
        impact_scope: authoritative_scope.clone(),
    };

    let mut session = state.db.client().start_session(None).await?;
    session
        .start_transaction(TransactionOptions::builder().build())
        .await?;
    let transaction_result: AppResult<()> = async {
        let contact_filter = doc! {
            "_id": preview.contact_id,
            "workspace_id": &preview.workspace_id,
            "account_id": &preview.account_id,
            "updated_at": plan.contact_updated_at,
        };
        let mut contact_set = if plan.contact_set.is_empty() {
            Document::new()
        } else {
            materialize_set(
                &plan.contact_set,
                &plan.contact_timestamp_fields,
                committed_at,
            )
        };
        if plan.contact_set.contains_key("manual_tags") {
            contact_set.insert("manual_tags_updated_at", committed_at);
            contact_set.insert("manual_tags_by", admin.username.clone());
        }
        contact_set.insert(GUIDE_APPLY_GUARD_FIELD, preview_id.to_hex());
        let contact_result = state
            .db
            .contacts()
            .update_one_with_session(
                contact_filter,
                doc! { "$set": contact_set },
                None,
                &mut session,
            )
            .await?;
        if contact_result.matched_count != 1 {
            return Err(AppError::Conflict("guide_contact_changed".to_string()));
        }

        let memory_filter = doc! {
            "workspace_id": &preview.workspace_id,
            "account_id": &preview.account_id,
            "contact_wxid": &preview.contact_wxid,
            "updated_at": plan.memory_updated_at,
        };
        let mut memory_set = if plan.memory_set.is_empty() {
            Document::new()
        } else {
            materialize_set(
                &plan.memory_set,
                &plan.memory_timestamp_fields,
                committed_at,
            )
        };
        memory_set.insert(GUIDE_APPLY_GUARD_FIELD, preview_id.to_hex());
        let memory_result = state
            .db
            .operating_memories()
            .update_one_with_session(
                memory_filter,
                doc! { "$set": memory_set },
                None,
                &mut session,
            )
            .await?;
        if memory_result.matched_count != 1 {
            return Err(AppError::Conflict("guide_memory_changed".to_string()));
        }

        match (plan.playbook_id, plan.playbook_version) {
            (Some(playbook_id), Some(playbook_version)) => {
                let playbook_filter = doc! {
                    "_id": playbook_id,
                    "workspace_id": &preview.workspace_id,
                    "account_id": &preview.account_id,
                    "version": playbook_version,
                    "release_status": "published",
                };
                let playbook_update = if plan.playbook_set.is_empty() {
                    doc! { "$set": { GUIDE_APPLY_GUARD_FIELD: preview_id.to_hex() } }
                } else {
                    let mut set = materialize_set(
                        &plan.playbook_set,
                        &plan.playbook_timestamp_fields,
                        committed_at,
                    );
                    set.insert("created_by", "guide_optimized");
                    set.insert(GUIDE_APPLY_GUARD_FIELD, preview_id.to_hex());
                    doc! { "$set": set, "$inc": { "version": 1_i32 } }
                };
                let result = state
                    .db
                    .operation_playbooks()
                    .update_one_with_session(playbook_filter, playbook_update, None, &mut session)
                    .await?;
                if result.matched_count != 1 {
                    return Err(AppError::Conflict("guide_playbook_changed".to_string()));
                }
            }
            (None, None) => {}
            _ => {
                return Err(AppError::Conflict(
                    "guide_playbook_baseline_invalid".to_string(),
                ));
            }
        }

        match (
            plan.domain_config_id,
            plan.domain_version,
            plan.domain_updated_at,
        ) {
            (Some(config_id), Some(version), Some(updated_at)) => {
                let domain_filter = doc! {
                    "_id": config_id,
                    "workspace_id": &preview.workspace_id,
                    "domain": crate::agent::domain::USER_OPS_DOMAIN_ID,
                    "current_version": true,
                    "version": version,
                    "updated_at": updated_at,
                };
                let mut domain_set = doc! {
                    GUIDE_APPLY_GUARD_FIELD: preview_id.to_hex(),
                };
                if let Some(runtime) = &plan.domain_runtime_parameters {
                    domain_set.insert("runtime_parameters", runtime);
                    domain_set.insert("updated_at", committed_at);
                }
                let domain_update = doc! { "$set": domain_set };
                let result = state
                    .db
                    .operation_domain_configs()
                    .update_one_with_session(domain_filter, domain_update, None, &mut session)
                    .await?;
                if result.matched_count != 1 {
                    return Err(AppError::Conflict("guide_domain_changed".to_string()));
                }
            }
            (None, None, None) => {}
            _ => {
                return Err(AppError::Conflict(
                    "guide_domain_baseline_invalid".to_string(),
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
                        "previewId": preview_id.to_hex(),
                        "candidateHash": stored_hash,
                        "instruction": &preview.instruction,
                        "impactScope": &authoritative_scope,
                        "scopeReason": &preview.scope_reason,
                        "authoritativeChanges": bson::to_bson(&plan.authoritative_changes)?,
                        "appliedFields": bson::to_bson(&plan.applied_fields)?,
                        "skippedFields": bson::to_bson(&plan.skipped_fields)?,
                        "appliedBy": &admin.username,
                    }),
                    created_at: committed_at,
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
                guide_owned_apply_filter(
                    preview_id,
                    &preview.workspace_id,
                    &preview.account_id,
                    preview.contact_id,
                    apply_token,
                ),
                doc! {
                    "$set": {
                        "status": "applied",
                        "apply_protocol_version": GUIDE_APPLY_PROTOCOL_VERSION,
                        "apply_receipt": bson::to_bson(&receipt)?,
                        "applied_at": committed_at,
                        "applied_by": &admin.username,
                        "updated_at": committed_at,
                    },
                    "$unset": {
                        "apply_error": "",
                        "apply_token": "",
                    },
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
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_hash, guide_claim_filter, guide_owned_apply_filter, plan_impact, receipt_json,
        requires_strong_confirmation, GUIDE_APPLY_PROTOCOL_VERSION,
    };
    use crate::models::{GuideApplyReceipt, GuideFrozenPlan, GuideSkippedField};
    use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

    fn minimal_plan() -> GuideFrozenPlan {
        GuideFrozenPlan {
            contact_updated_at: DateTime::from_millis(1_700_000_000_000),
            memory_updated_at: DateTime::from_millis(1_700_000_000_001),
            playbook_id: None,
            playbook_version: None,
            domain_config_id: None,
            domain_version: None,
            domain_updated_at: None,
            contact_set: doc! { "human_profile_note": "next" },
            contact_timestamp_fields: Vec::new(),
            memory_set: Document::new(),
            memory_timestamp_fields: Vec::new(),
            playbook_set: Document::new(),
            playbook_timestamp_fields: Vec::new(),
            domain_runtime_parameters: None,
            applied_fields: vec!["humanProfileNote".to_string()],
            skipped_fields: Vec::new(),
            authoritative_changes: Vec::new(),
            playbook_affected_contacts: 0,
        }
    }

    #[test]
    fn receipt_json_matches_contract_fixture() {
        let receipt = GuideApplyReceipt {
            preview_id: ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap(),
            candidate_hash: "candidate-hash".to_string(),
            committed_at: DateTime::from_millis(1_700_000_000_000),
            applied_fields: vec!["humanProfileNote".to_string()],
            skipped_fields: vec![GuideSkippedField {
                field: "operationState".to_string(),
                reason: "not_allowed".to_string(),
            }],
            impact_scope: "current_contact".to_string(),
        };
        crate::routes::contract_snapshot::assert_contract_fixture(
            "guide_apply_receipt",
            receipt_json(&receipt),
        );
    }

    #[test]
    fn candidate_hash_binds_workspace_account_contact_and_plan() {
        let contact_a = ObjectId::new();
        let contact_b = ObjectId::new();
        let plan = minimal_plan();
        let base = candidate_hash("ws-a", "account-a", contact_a, &plan).unwrap();
        assert_ne!(
            base,
            candidate_hash("ws-b", "account-a", contact_a, &plan).unwrap()
        );
        assert_ne!(
            base,
            candidate_hash("ws-a", "account-b", contact_a, &plan).unwrap()
        );
        assert_ne!(
            base,
            candidate_hash("ws-a", "account-a", contact_b, &plan).unwrap()
        );
        let mut changed = plan.clone();
        changed.contact_set.insert("human_profile_note", "other");
        assert_ne!(
            base,
            candidate_hash("ws-a", "account-a", contact_a, &changed).unwrap()
        );
    }

    #[test]
    fn impact_scope_is_derived_from_frozen_targets() {
        let mut plan = minimal_plan();
        assert_eq!(plan_impact(&plan).0, "current_contact");
        assert!(!requires_strong_confirmation(&plan_impact(&plan).0));

        plan.playbook_id = Some(ObjectId::new());
        plan.playbook_version = Some(1);
        plan.playbook_set.insert("reply_style", "warm");
        assert_eq!(plan_impact(&plan).0, "shared_playbook");
        assert!(requires_strong_confirmation(&plan_impact(&plan).0));

        plan.domain_runtime_parameters = Some(doc! { "maxDailyTouches": 2 });
        assert_eq!(plan_impact(&plan).0, "workspace_user_operations");
        assert!(requires_strong_confirmation(&plan_impact(&plan).0));
    }

    #[test]
    fn guide_claim_filter_allows_retry_and_stale_recovery() {
        let id = ObjectId::new();
        let contact_id = ObjectId::new();
        let stale_before = DateTime::from_millis(1_700_000_000_000);
        assert_eq!(
            guide_claim_filter(
                id,
                "ws-a",
                "account-a",
                contact_id,
                "candidate-hash",
                stale_before,
            ),
            doc! {
                "_id": id,
                "workspace_id": "ws-a",
                "account_id": "account-a",
                "contact_id": contact_id,
                "candidate_hash": "candidate-hash",
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
        let contact_id = ObjectId::new();
        assert_eq!(
            guide_owned_apply_filter(id, "ws-a", "account-a", contact_id, "lease-1"),
            doc! {
                "_id": id,
                "workspace_id": "ws-a",
                "account_id": "account-a",
                "contact_id": contact_id,
                "status": "applying",
                "apply_token": "lease-1",
            }
        );
    }
}
