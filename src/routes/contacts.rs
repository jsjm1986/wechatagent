//! 联系人路由：联系人画像、操作记忆、运营状态等用户级别接口。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
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
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    mcp::{self},
    models::{
        ApiContact, ContactQuery, CustomAgentInstructionsRequest, EnableAgentRequest,
        ImportContactsRequest, OutcomeEvent, ProfileNoteRequest, SearchImportRequest,
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

/// `POST /api/contacts/:id/deal-events` 请求体。
///
/// S5（自学习采集管道）：admin 手动登记一条**结果/成效**（T0 硬事件）正例，落
/// `Contact.outcome_events`（universal-domain-adaptation H10：存储已从销售域
/// `deal_events` 泛化为行业中性的 `outcome_events`；路由路径 / 请求类型名保持
/// `deal-events` 不变以维持 API 兼容，无外部消费方依赖具体语义）。本阶段只
/// append-only 记录，不反推任何置信、不归因——为将来 PU learning 铺正例池。
/// 全部字段可选：最小可用只需点一下"标记成效"，金额/币种/发生时间/备注按需回填。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DealEventRequest {
    /// 结果实际发生时间的毫秒时间戳（可选，缺省用服务端 now 作为 marked_at）。
    occurred_at_ms: Option<i64>,
    amount: Option<f64>,
    currency: Option<String>,
    note: Option<String>,
}

pub(super) async fn list_contacts(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ContactQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! {};
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    filter.insert("workspace_id", &admin.current_workspace);
    filter.insert("account_id", &account_id);
    if let Some(status) = query.status {
        if !status.is_empty() {
            filter.insert("agent_status", status);
        }
    }
    if let Some(q) = query.q {
        if !q.is_empty() {
            let q = escape_regex_literal(&q);
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<SearchImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<ImportContactsRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
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
            upsert_contact_from_value(&state, &admin.current_workspace, &account_id, contact_value)
                .await?
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<SearchImportRequest>,
) -> AppResult<Json<Value>> {
    if payload.query.trim().is_empty() {
        return Err(AppError::BadRequest("query is required".to_string()));
    }
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
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
                upsert_contact_from_value(&state, &admin.current_workspace, &account_id, contact_value)
                    .await?
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn enable_agent(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<EnableAgentRequest>,
) -> AppResult<Json<Value>> {
    if payload.human_profile_note.trim().is_empty() {
        return Err(AppError::BadRequest(
            "humanProfileNote is required".to_string(),
        ));
    }
    let object_id = parse_object_id(&id)?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    // P1：先校验 contact.account_id 在 wechat_accounts 注册过。否则即使写 managed
    // 进去，webhook 入站时 resolve_account_context 也会因为 appId 匹配不到这个
    // account 直接 400 拒收，AI 永远不会回复。
    if state
        .db
        .accounts()
        .find_one(doc! { "account_id": &contact.account_id }, None)
        .await?
        .is_none()
    {
        return Err(AppError::BadRequest(format!(
            "contact.account_id={} 在 wechat_accounts 中未注册，无法启用 Agent 运营",
            contact.account_id
        )));
    }
    let playbook = resolve_playbook_for_contact(
        &state,
        &admin.current_workspace,
        &contact.account_id,
        payload.playbook_id.as_deref(),
    )
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
    // #72：曾运营过的老客户重新启用时，保留已积累的 stage / operation_state /
    // commitments，不回退到 new_contact；只切 managed + 更新本次 admin 显式输入
    // （备注 / playbook / 画像）。全新客户才走完整初始化。
    let mut set_doc = doc! {
        "agent_status": "managed",
        "human_profile_note": payload.human_profile_note,
        "agent_profile": to_bson(&generated.agent_profile)?,
        "playbook_id": playbook.id,
        "playbook_version": playbook.version,
        "tags": generated.tags,
        "profile_attributes": generated.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let mut unset_doc = Document::new();
    if !is_previously_operated(&contact) {
        // H13：初始 operation_state 从 active 状态机的 initial 态取（替代写死 "new_contact"）。
        let domain_config =
            agent::load_user_operation_domain_config(&state, &admin.current_workspace).await?;
        let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
        insert_domain_stage_fields(
            &mut set_doc,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
            true,
        );
        set_doc.insert("commitments", commitments_bson);
        set_doc.insert("follow_up_policy", generated.follow_up_policy);
        set_doc.insert("operation_state", initial_state);
        set_doc.insert(
            "operation_state_reason",
            "初次纳入 Agent 运营，等待后续互动确认阶段",
        );
        set_doc.insert("operation_state_confidence", 6);
        set_doc.insert("operation_state_updated_at", DateTime::now());
        unset_doc.insert("last_commitment", "");
    }
    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }
    state
        .db
        .contacts()
        .update_one(doc! { "_id": object_id }, update_doc, None)
        .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn disable_agent(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! {
                "$set": {
                    "agent_status": "normal",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn update_profile_note(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ProfileNoteRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
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
    // #72：曾运营过的老客户重新生成画像时保留 stage / operation_state / commitments，
    // 不回退 new_contact；全新客户才完整初始化。
    let mut set_doc = doc! {
        "human_profile_note": payload.human_profile_note,
        "agent_profile": to_bson(&generated.agent_profile)?,
        "tags": generated.tags,
        "profile_attributes": generated.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let mut unset_doc = Document::new();
    if !is_previously_operated(&contact) {
        // H13：初始 operation_state 从 active 状态机的 initial 态取（替代写死 "new_contact"）。
        let domain_config =
            agent::load_user_operation_domain_config(&state, &admin.current_workspace).await?;
        let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
        insert_domain_stage_fields(
            &mut set_doc,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
            true,
        );
        set_doc.insert("commitments", commitments_bson);
        set_doc.insert("follow_up_policy", generated.follow_up_policy);
        set_doc.insert("operation_state", initial_state);
        set_doc.insert(
            "operation_state_reason",
            "根据 admin 备注重新生成初始运营状态",
        );
        set_doc.insert("operation_state_confidence", 6);
        set_doc.insert("operation_state_updated_at", DateTime::now());
        unset_doc.insert("last_commitment", "");
    }
    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            update_doc,
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

/// `PUT /api/contacts/:id/custom-agent-instructions`
///
/// 维护 per-contact 运营人员特别指令（最高优先级 Operator Instruction 层）。
/// 上限 1000 字符，trim 后空字符串等价于"清空"（落库为 null）。
///
/// 该指令会在下一次 user.reply 调用时由 `agent::decision` 注入到 system prompt
/// 末位，覆盖 Soul + Policy 的默认人格判定（详见
/// docs/conversation-mode-design.md）。
pub(super) async fn update_custom_agent_instructions(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<CustomAgentInstructionsRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let trimmed = payload.instructions.trim();
    if trimmed.chars().count() > 1000 {
        return Err(AppError::BadRequest(
            "custom_agent_instructions 长度上限 1000 字符".to_string(),
        ));
    }
    // trim 后空 → 清空（落 null）；非空 → 直接保存原始（不 trim 内部空白，
    // 运营可能用换行 / 前后空白来分块）。
    let value: mongodb::bson::Bson = if trimmed.is_empty() {
        mongodb::bson::Bson::Null
    } else {
        mongodb::bson::Bson::String(payload.instructions.clone())
    };
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! {
                "$set": {
                    "custom_agent_instructions": value,
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn update_operation_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationProfileRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let current = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let new_stage = normalize_optional(payload.customer_stage);
    let prev_stage = current
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()));
    let stage_changed = prev_stage.as_deref() != new_stage.as_deref();
    let commitments_bson = commitments_with_optional_text(
        &current.commitments,
        normalize_optional(payload.last_commitment).as_deref(),
    );
    let mut set_doc = doc! {
        "tags": payload.tags,
        "commitments": commitments_bson,
        "follow_up_policy": normalize_optional(payload.follow_up_policy),
        "profile_attributes": payload.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let intent_level = normalize_optional(payload.intent_level);
    insert_domain_stage_fields(
        &mut set_doc,
        new_stage.as_deref(),
        intent_level.as_deref(),
        stage_changed,
    );
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! {
                "$set": set_doc,
                "$unset": { "last_commitment": "" }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

/// `POST /api/contacts/:id/deal-events`
///
/// S5（自学习采集管道·第一阶段）：admin 手动登记一条成交事件（T0 硬事件正例）。
/// 平台入站只有文字、无支付/订单回填，成交只能靠 admin 手动标记 —— 稀疏、延迟、
/// 只有正例（PU learning 形状）。本阶段**只 append-only 落正例池**：
/// - 不反推任何 chunk 置信；
/// - 不做多触点归因；
/// - `source` 恒 `"manual"`，`marked_by` 取登录 admin，用于审计。
///
/// 写库走 `$push contact.outcome_events` + 一条 `outcome_event_marked` 审计事件。
pub(super) async fn add_deal_event(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<DealEventRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    if let Some(amount) = payload.amount {
        if !amount.is_finite() || amount < 0.0 {
            return Err(AppError::BadRequest(
                "amount 必须是非负有限数".to_string(),
            ));
        }
    }
    let now = DateTime::now();
    let outcome_event = OutcomeEvent {
        marked_at: now,
        occurred_at: payload.occurred_at_ms.map(DateTime::from_millis),
        amount: payload.amount,
        currency: normalize_optional(payload.currency),
        source: "manual".to_string(),
        marked_by: admin.username.clone(),
        note: normalize_optional(payload.note),
    };
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! {
                "$push": { "outcome_events": to_bson(&outcome_event)? },
                "$set": { "updated_at": now },
            },
            None,
        )
        .await?;
    agent::write_event_for_account(
        &state,
        &contact.account_id,
        Some(&contact.wxid),
        "outcome_event_marked",
        "ok",
        "admin 手动登记成效事件",
        Some(doc! {
            "source": "manual",
            "markedBy": &admin.username,
            "amount": payload.amount,
            "hasOccurredAt": payload.occurred_at_ms.is_some(),
        }),
    )
    .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

pub(super) async fn analyze_contact_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
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
    // #72：曾运营过的老客户 AI 重新分析时保留 stage / operation_state / commitments，
    // 不回退 new_contact；全新客户才完整初始化。
    let mut set_doc = doc! {
        "agent_profile": to_bson(&generated.agent_profile)?,
        "tags": generated.tags,
        "profile_attributes": generated.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let mut unset_doc = Document::new();
    if !is_previously_operated(&contact) {
        // H13：初始 operation_state 从 active 状态机的 initial 态取（替代写死 "new_contact"）。
        let domain_config =
            agent::load_user_operation_domain_config(&state, &admin.current_workspace).await?;
        let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
        insert_domain_stage_fields(
            &mut set_doc,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
            true,
        );
        set_doc.insert("commitments", commitments_bson);
        set_doc.insert("follow_up_policy", generated.follow_up_policy);
        set_doc.insert("operation_state", initial_state);
        set_doc.insert(
            "operation_state_reason",
            "AI 重新分析后等待后续互动确认阶段",
        );
        set_doc.insert("operation_state_confidence", 6);
        set_doc.insert("operation_state_updated_at", DateTime::now());
        unset_doc.insert("last_commitment", "");
    }
    let mut update_doc = doc! { "$set": set_doc };
    if !unset_doc.is_empty() {
        update_doc.insert("$unset", unset_doc);
    }
    state
        .db
        .contacts()
        .update_one(doc! { "_id": contact.id }, update_doc, None)
        .await?;
    let updated = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(updated) })))
}

pub(super) async fn get_operating_memory(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    Ok(Json(json!({ "item": operating_memory_json(memory) })))
}

pub(super) async fn update_operating_memory(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperatingMemoryRequest>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    // H13：无 operation_state 时回落状态机初始态（替代写死 "new_contact"）。
    let initial_state = agent::initial_operation_state_for_contact(&state, &contact).await?;
    Ok(Json(json!({
        "item": {
            "contactWxid": contact.wxid,
            // task 6.3：`effective_memory_card_for_contact` 已改为返回
            // `MemoryCardTyped`；路由层 JSON 响应在最末端通过 `to_document()`
            // 转成 Document（保持 wire shape 不变）。
            "memoryCard": agent::effective_memory_card_for_contact(&memory, &contact, &initial_state).to_document(),
            "memoryCardVersion": memory.memory_card_version,
            "memoryCardUpdatedAt": memory.memory_card_updated_at.and_then(crate::models::dt_to_string)
        }
    })))
}

pub(super) async fn list_contact_memory_candidates(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Query(query): Query<MemoryCandidateQuery>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
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
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    agent::consolidate_contact_memory(&state, &contact, None).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    Ok(Json(
        json!({ "ok": true, "item": operating_memory_json(memory) }),
    ))
}

pub(super) async fn get_operation_health(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let memory = ensure_operating_memory(&state, &contact).await?;
    let latest_review = latest_decision_review(&state, &contact).await?;
    Ok(Json(operation_health_json(
        &contact,
        &memory,
        latest_review.as_ref(),
    )))
}
