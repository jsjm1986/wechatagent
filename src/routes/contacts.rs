//! 联系人路由：联系人画像、操作记忆、运营状态等用户级别接口。

use axum::{
    extract::{Path, Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, to_bson, DateTime, Document, Regex},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    error::{AppError, AppResult},
    mcp::{self},
    models::{
        ApiContact, ContactQuery, EnableAgentRequest, ImportContactsRequest, ProfileNoteRequest,
        SearchImportRequest,
    },
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationProfileRequest {
    #[serde(default)]
    tags: Vec<String>,
    customer_stage: Option<String>,
    intent_level: Option<String>,
    last_commitment: Option<String>,
    follow_up_policy: Option<String>,
    #[serde(default)]
    profile_attributes: Document,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperatingMemoryRequest {
    #[serde(default)]
    user_understanding: Document,
    #[serde(default)]
    relationship_state: Document,
    #[serde(default)]
    product_fit: Document,
    #[serde(default)]
    next_action: Document,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MemoryCandidateQuery {
    status: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn list_contacts(
    State(state): State<AppState>,
    Query(query): Query<ContactQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! {};
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    filter.insert("workspace_id", &state.config.default_workspace_id);
    filter.insert("account_id", &account_id);
    if let Some(status) = query.status {
        if !status.is_empty() {
            filter.insert("agent_status", status);
        }
    }
    if let Some(q) = query.q {
        if !q.is_empty() {
            filter.insert(
                "$or",
                vec![
                    doc! { "nickname": Regex { pattern: q.clone(), options: "i".to_string() } },
                    doc! { "remark": Regex { pattern: q.clone(), options: "i".to_string() } },
                    doc! { "wxid": Regex { pattern: q.clone(), options: "i".to_string() } },
                    doc! { "alias": Regex { pattern: q, options: "i".to_string() } },
                ],
            );
        }
    }
    let mut cursor = state
        .db
        .contacts()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(query.limit.unwrap_or(100).clamp(1, 500))
                .skip(query.skip)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(contact) = cursor.try_next().await? {
        items.push(ApiContact::from(contact));
    }
    Ok(Json(json!({ "items": items })))
}

/// 波 A3：只搜索不写库的纯查询接口。
///
/// MCP 调 `contacts_search` 返回原始候选列表，前端可在用户确认后再调
/// [`import_contacts`] 写入。这避免了原 `search-import` "搜索即写库"
/// 的副作用与契约误解。
pub(super) async fn search_contacts_endpoint(
    State(state): State<AppState>,
    Json(payload): Json<SearchImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let result = mcp::logged_call_for_account(
        &state,
        &account_id,
        "contacts_search",
        json!({
            "query": payload.query,
            "limit": 20
        }),
    )
    .await?;
    let items = result
        .get("items")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(Json(json!({
        "accountId": account_id,
        "items": items
    })))
}

/// 波 A3：把 search 返回的候选导入本地 contacts 集合。
///
/// 兼容两种入参：
/// - `{ "query": "...", "accountId": "..." }`：等价于先 search 再导入（沿用旧
///   `search-import` 行为，便于过渡）。
/// - `{ "candidates": [...], "accountId": "..." }`：直接导入前端拿到的候选项。
pub(super) async fn import_contacts_endpoint(
    State(state): State<AppState>,
    Json(payload): Json<ImportContactsRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let candidates: Vec<Value> = if !payload.candidates.is_empty() {
        payload.candidates.clone()
    } else if let Some(query) = payload.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        let result = mcp::logged_call_for_account(
            &state,
            &account_id,
            "contacts_search",
            json!({ "query": query, "limit": 20 }),
        )
        .await?;
        result
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
    } else {
        return Err(AppError::BadRequest(
            "either query or candidates is required".to_string(),
        ));
    };
    let mut imported = Vec::new();
    for item in candidates {
        let contact_value = item.get("contact").unwrap_or(&item);
        if let Some(contact) =
            upsert_contact_from_value(&state, &account_id, contact_value).await?
        {
            imported.push(ApiContact::from(contact));
        }
    }
    Ok(Json(json!({ "items": imported })))
}

/// **DEPRECATED 波 A3**：旧合并入口，行为等于 search 再 import。请改用
/// [`search_contacts_endpoint`] / [`import_contacts_endpoint`]。
pub(super) async fn search_import_contacts(
    State(state): State<AppState>,
    Json(payload): Json<SearchImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let result = mcp::logged_call_for_account(
        &state,
        &account_id,
        "contacts_search",
        json!({
            "query": payload.query,
            "limit": 20
        }),
    )
    .await?;
    let items = result
        .get("items")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut imported = Vec::new();
    for item in items {
        if let Some(contact_value) = item.get("contact") {
            if let Some(contact) =
                upsert_contact_from_value(&state, &account_id, contact_value).await?
            {
                imported.push(ApiContact::from(contact));
            }
        }
    }
    Ok(Json(json!({
        "items": imported,
        "deprecated": true,
        "deprecationNote": "Use POST /api/contacts/search and /api/contacts/import instead."
    })))
}

pub(super) async fn get_contact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn enable_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<EnableAgentRequest>,
) -> AppResult<Json<Value>> {
    if payload.human_profile_note.trim().is_empty() {
        return Err(AppError::BadRequest(
            "humanProfileNote is required".to_string(),
        ));
    }
    let object_id = parse_object_id(&id)?;
    let contact = find_contact_by_id(&state, &id).await?;
    let playbook =
        resolve_playbook_for_contact(&state, &contact.account_id, payload.playbook_id.as_deref())
            .await?;
    let generated = agent::build_initial_operation_profile(
        &state,
        &payload.human_profile_note,
        Some(&playbook),
    )
    .await?;
    let commitments_bson = commitments_with_optional_text(
        &contact.commitments,
        generated.last_commitment.as_deref(),
    );
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "agent_status": "managed",
                    "human_profile_note": payload.human_profile_note,
                    "agent_profile": to_bson(&generated.agent_profile)?,
                    "playbook_id": playbook.id,
                    "playbook_version": playbook.version,
                    "tags": generated.tags,
                    "customer_stage": generated.customer_stage,
                    "customer_stage_updated_at": DateTime::now(),
                    "intent_level": generated.intent_level,
                    "commitments": commitments_bson,
                    "follow_up_policy": generated.follow_up_policy,
                    "operation_state": "new_contact",
                    "operation_state_reason": "初次纳入 Agent 运营，等待后续互动确认阶段",
                    "operation_state_confidence": 6,
                    "operation_state_updated_at": DateTime::now(),
                    "profile_attributes": generated.profile_attributes,
                    "profile_updated_at": DateTime::now(),
                    "updated_at": DateTime::now()
                },
                "$unset": {
                    "last_commitment": ""
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn disable_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "agent_status": "normal",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn update_profile_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<ProfileNoteRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let contact = find_contact_by_id(&state, &id).await?;
    let playbook = agent::load_operation_playbook_for_contact(&state, &contact).await?;
    let generated = agent::build_initial_operation_profile(
        &state,
        &payload.human_profile_note,
        playbook.as_ref(),
    )
    .await?;
    let commitments_bson = commitments_with_optional_text(
        &contact.commitments,
        generated.last_commitment.as_deref(),
    );
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": {
                    "human_profile_note": payload.human_profile_note,
                    "agent_profile": to_bson(&generated.agent_profile)?,
                    "tags": generated.tags,
                    "customer_stage": generated.customer_stage,
                    "customer_stage_updated_at": DateTime::now(),
                    "intent_level": generated.intent_level,
                    "commitments": commitments_bson,
                    "follow_up_policy": generated.follow_up_policy,
                    "operation_state": "new_contact",
                    "operation_state_reason": "根据人工备注重新生成初始运营状态",
                    "operation_state_confidence": 6,
                    "operation_state_updated_at": DateTime::now(),
                    "profile_attributes": generated.profile_attributes,
                    "profile_updated_at": DateTime::now(),
                    "updated_at": DateTime::now()
                },
                "$unset": {
                    "last_commitment": ""
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn update_operation_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<OperationProfileRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let current = find_contact_by_id(&state, &id).await?;
    let new_stage = normalize_optional(payload.customer_stage);
    let stage_changed = current.customer_stage.as_deref() != new_stage.as_deref();
    let commitments_bson = commitments_with_optional_text(
        &current.commitments,
        normalize_optional(payload.last_commitment).as_deref(),
    );
    let mut set_doc = doc! {
        "tags": payload.tags,
        "customer_stage": &new_stage,
        "intent_level": normalize_optional(payload.intent_level),
        "commitments": commitments_bson,
        "follow_up_policy": normalize_optional(payload.follow_up_policy),
        "profile_attributes": payload.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    if stage_changed {
        set_doc.insert("customer_stage_updated_at", DateTime::now());
    }
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id },
            doc! {
                "$set": set_doc,
                "$unset": { "last_commitment": "" }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn analyze_contact_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    let playbook = agent::load_operation_playbook_for_contact(&state, &contact).await?;
    let note = contact.human_profile_note.clone().unwrap_or_else(|| {
        format!(
            "微信好友：{}",
            contact
                .remark
                .clone()
                .or(contact.nickname.clone())
                .unwrap_or(contact.wxid.clone())
        )
    });
    let generated =
        agent::build_initial_operation_profile(&state, &note, playbook.as_ref()).await?;
    let commitments_bson = commitments_with_optional_text(
        &contact.commitments,
        generated.last_commitment.as_deref(),
    );
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! {
                "$set": {
                    "agent_profile": to_bson(&generated.agent_profile)?,
                    "tags": generated.tags,
                    "customer_stage": generated.customer_stage,
                    "customer_stage_updated_at": DateTime::now(),
                    "intent_level": generated.intent_level,
                    "commitments": commitments_bson,
                    "follow_up_policy": generated.follow_up_policy,
                    "operation_state": "new_contact",
                    "operation_state_reason": "AI 重新分析后等待后续互动确认阶段",
                    "operation_state_confidence": 6,
                    "operation_state_updated_at": DateTime::now(),
                    "profile_attributes": generated.profile_attributes,
                    "profile_updated_at": DateTime::now(),
                    "updated_at": DateTime::now()
                },
                "$unset": {
                    "last_commitment": ""
                }
            },
            None,
        )
        .await?;
    let updated = find_contact_by_id(&state, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(updated) })))
}

pub(super) async fn get_operating_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    Ok(Json(json!({ "item": operating_memory_json(memory) })))
}

pub(super) async fn update_operating_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<OperatingMemoryRequest>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    ensure_operating_memory(&state, &contact).await?;
    state
        .db
        .operating_memories()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            doc! {
                "$set": {
                    "user_understanding": payload.user_understanding,
                    "relationship_state": payload.relationship_state,
                    "product_fit": payload.product_fit,
                    "next_action": payload.next_action,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    Ok(Json(json!({ "item": operating_memory_json(memory) })))
}

pub(super) async fn get_contact_memory_card(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    Ok(Json(json!({
        "item": {
            "contactWxid": contact.wxid,
            // task 6.3：`effective_memory_card_for_contact` 已改为返回
            // `MemoryCardTyped`；路由层 JSON 响应在最末端通过 `to_document()`
            // 转成 Document（保持 wire shape 不变）。
            "memoryCard": agent::effective_memory_card_for_contact(&memory, &contact).to_document(),
            "memoryCardVersion": memory.memory_card_version,
            "memoryCardUpdatedAt": memory.memory_card_updated_at.and_then(crate::models::dt_to_string)
        }
    })))
}

pub(super) async fn list_contact_memory_candidates(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<MemoryCandidateQuery>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    let mut filter = doc! {
        "workspace_id": &contact.workspace_id,
        "account_id": &contact.account_id,
        "contact_wxid": &contact.wxid
    };
    if let Some(status) = query.status {
        filter.insert("status", status);
    }
    let mut cursor = state
        .db
        .memory_candidates()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(query.limit.unwrap_or(50).clamp(1, 200))
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(memory_candidate_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn run_contact_memory_consolidation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    agent::consolidate_contact_memory(&state, &contact, None).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    Ok(Json(
        json!({ "ok": true, "item": operating_memory_json(memory) }),
    ))
}

pub(super) async fn get_operation_health(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &id).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    let latest_review = latest_decision_review(&state, &contact).await?;
    Ok(Json(operation_health_json(
        &contact,
        &memory,
        latest_review.as_ref(),
    )))
}
