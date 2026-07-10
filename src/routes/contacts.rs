//! 联系人路由：联系人画像、操作记忆、运营状态等用户级别接口。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, to_bson, DateTime, Document, Regex},
    options::{FindOneOptions, FindOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    mcp::{self},
    models::{
        ApiContact, Contact, ContactQuery, CustomAgentInstructionsRequest, EnableAgentRequest,
        ImportContactsRequest, ProfileNoteRequest, SearchImportRequest,
    },
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationProfileRequest {
    customer_stage: Option<String>,
    intent_level: Option<String>,
    /// §3.7 数字分身：关系类型（customer/peer/friend，走 system_taxonomies）。运营接入时
    /// 设定，决定 planner 选哪套 OperationMode（驱动力组合）。`None` → 不改动现值。
    relationship_type: Option<String>,
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
pub struct AssistOverrideRequest {
    mode: String,
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
    /// 成交金额，最小币种单位整数（分，19900=¥199.00）。前端 ×100 转分后传入。
    amount: Option<i64>,
    currency: Option<String>,
    note: Option<String>,
    /// 成交真相源可信度（G3 §2）。admin 后台登记缺省 `staff_confirmed`（admin 登记即
    /// 高可信，§4.4）。仅接受闭集 `staff_confirmed` | `payment_verified`——
    /// `conversation_inferred` 是 AI 疑似线索，绝不走 admin 直登通道（§5.5：疑似线索
    /// 经审核才落 staff_confirmed），传入即 400。
    verification: Option<String>,
    /// 关联产品 product_id（可选）。给定时从本 workspace active 产品表解引用，按
    /// 成交当时拷贝名/价/SKU 落 `OutcomeProductRef` 快照（§4.3 订单式冻结，非活引用）。
    product_id: Option<String>,
    /// 件数（可选，默认 1）。
    quantity: Option<u32>,
    /// 事件方向（§4.5）：`deal`（正向成交，缺省）| `reversal`（退款/撤单）。
    /// reversal 不删原成交（审计完整性），append 一条反向事件由 G4 投影按 product_id
    /// 抵消净件数；reversal 必须给 product_id（无产品的退款没有可抵消标的，无意义）。
    event_kind: Option<String>,
}

/// 待启用档预览取原文的最大字符数（按字符非字节，避免中文截半）。
const INBOUND_PREVIEW_MAX_CHARS: usize = 30;

/// 按字符（非字节）截断，避免中文截半。超长加省略号。
/// 这是纯原文截断，绝非 LLM 智能摘要——未托管联系人不调 LLM（产品红线）。
pub(crate) fn truncate_preview(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(max_chars).collect();
    format!("{head}…")
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
                // 最近主动来消息的人排最前（热线索优先）；last_inbound_at 为空的
                // 老记录用 updated_at 兜底。Mongo 多键 sort 按顺序生效。
                .sort(doc! { "last_inbound_at": -1, "updated_at": -1 })
                .limit(query.limit.unwrap_or(100).clamp(1, 500))
                .skip(query.skip)
                .build(),
        )
        .await?;
    // 读时兜底富化（spec 改动3.1）：webhook 只在首次建档时按 roster 富化昵称/头像，
    // 若某真人在其 roster 快照（>24h 异步刷新）尚未收录时来消息，建档时拿不到昵称头像
    // 且再无自愈点。此处读一次快照（整表一条 doc，非 per-contact），对 nickname/avatar
    // 为空的联系人补齐——下次快照刷新后列表即显示正确身份。快照缺失/读失败不阻断（返回空 map）。
    let roster_identity: std::collections::HashMap<String, (Option<String>, Option<String>)> =
        match crate::mcp::read_roster_snapshot(&state, &admin.current_workspace, &account_id).await {
            Ok(Some(snap)) => snap
                .friends
                .into_iter()
                .map(|f| (f.wxid, (f.nickname, f.avatar_url)))
                .collect(),
            _ => std::collections::HashMap::new(),
        };
    let mut items = Vec::new();
    while let Some(contact) = cursor.try_next().await? {
        // 双保险：即使 migration 已清，读时再过滤一次非真人（公众号 gh_/群
        // @chatroom），防历史残留或新 bug 漏写。复用 webhook 建档同源判据。
        if !crate::webhooks::is_operatable_person(&contact.wxid) {
            continue;
        }
        let wxid = contact.wxid.clone();
        let mut api = ApiContact::from(contact);
        // 读时兜底富化：仅补空字段，不覆盖已有 nickname/avatar（与建档 $set 语义一致）。
        if api.nickname.is_none() || api.avatar_url.is_none() {
            if let Some((roster_nick, roster_avatar)) = roster_identity.get(&wxid) {
                if api.nickname.is_none() {
                    api.nickname = roster_nick.clone();
                }
                if api.avatar_url.is_none() {
                    api.avatar_url = roster_avatar.clone();
                }
            }
        }
        // 最近一条入站消息原文截断（待启用档展示，帮运营判断是否开 Agent）。
        // direction 存储值为小写 "inbound"（MessageDirection serde rename_all="lowercase"）。
        // 纯原文截断，非 LLM 摘要——normal 联系人不调 LLM。
        if let Ok(Some(msg)) = state
            .db
            .messages()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id,
                    "contact_wxid": &wxid,
                    "direction": "inbound",
                },
                FindOneOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .build(),
            )
            .await
        {
            api.last_inbound_preview =
                Some(truncate_preview(&msg.content, INBOUND_PREVIEW_MAX_CHARS));
        }
        items.push(api);
    }
    Ok(Json(json!({ "items": items })))
}

/// 计数端点的 filter 构造（抽纯函数便于单测口径正确性）。
/// base 与 `list_contacts`（本文件上方）的 workspace+account filter 同源；
/// managed 在其上加 `agent_status="managed"`。AgentStatus 仅 Normal/Managed
/// 两态（models.rs），故调用方 `normal = all - managed` 精确无第三态遗漏。
fn contact_count_filters(workspace_id: &str, account_id: &str) -> (Document, Document) {
    let base = doc! { "workspace_id": workspace_id, "account_id": account_id };
    let mut managed = base.clone();
    managed.insert("agent_status", "managed");
    (base, managed)
}

/// `GET /api/contacts/counts?accountId=xxx`
///
/// 返回运营池三个 tab 的**后端真实计数** `{ all, managed, normal }`，
/// 不受 `list_contacts` 的 limit 截断影响。口径与 `list_contacts` 的
/// workspace+account filter 同源。IDOR：workspace 来自 AuthenticatedAdmin，
/// 不接受请求体 workspace。
pub(super) async fn count_contacts(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ContactQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let (base, managed_filter) = contact_count_filters(&admin.current_workspace, &account_id);
    let all = state.db.contacts().count_documents(base, None).await?;
    let managed = state.db.contacts().count_documents(managed_filter, None).await?;
    let normal = all.saturating_sub(managed);
    Ok(Json(json!({ "all": all, "managed": managed, "normal": normal })))
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

/// `GET /api/contacts/:id/outcome-events`
///
/// G3 成交记录读端点（spec §8.5.3「成交记录」Tab）。返回本 contact 的全部
/// `outcome_events`，每条带 `verification` 徽标 + `productRef` 快照 + 金额。
/// OutcomeEvent 已是 camelCase serde，直接序列化即含新字段（旧记录缺字段 → serde
/// 缺省 verification=staff_confirmed / productRef 省略）。
///
/// IDOR（§3.5）：经 `find_contact_by_id` 锁 workspace；不接受请求体 workspace。
/// 按 occurred_at ?? marked_at 倒序（最近成交在前），不 cap（admin 审阅全量）。
pub(super) async fn list_outcome_events(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let mut events = contact.outcome_events;
    // 倒序：实际发生时间优先，缺省回落标记时间（与 G4 投影 occurred_at ?? marked_at 一致）。
    events.sort_by(|a, b| {
        let bk = b.occurred_at.unwrap_or(b.marked_at).timestamp_millis();
        let ak = a.occurred_at.unwrap_or(a.marked_at).timestamp_millis();
        bk.cmp(&ak)
    });
    Ok(Json(json!({ "items": events })))
}

/// `GET /api/contacts/:id/entitlements`
///
/// G4 持有投影读端点（spec §8.5.3「客户持有」Tab + §9 #6）。派生视图不落库，运行时
/// 对本 contact 的 outcome_events 跑 `project_entitlements`。read 端点不受 prompt 软上限
/// 约束（cap_n = usize::MAX，§5.1 注释明确「read 端点不受此限」）。
///
/// IDOR（§3.5）：经 `find_contact_by_id` 锁 contact workspace；active 产品按同一
/// workspace + status=active 加载（`load_active_products` filter 含 workspace_id）。
/// 零扰动：无产品域/无成交 → entitlements 空数组。
pub(super) async fn list_entitlements(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let active_products =
        agent::entitlements::load_active_products(&state.db, &contact.workspace_id).await;
    // read 端点全量，不 cap（§5.1：read 端点不受 ENTITLEMENTS_PROMPT_CAP 限）。
    let (entitlements, total) = agent::entitlements::project_entitlements(
        &contact.outcome_events,
        &active_products,
        DateTime::now(),
        usize::MAX,
    );
    let items: Vec<Value> = entitlements
        .iter()
        .map(|e| {
            json!({
                "productId": e.product_id,
                "name": e.name,
                "ownedSince": e.owned_since.timestamp_millis(),
                "quantity": e.quantity,
                "inAftercare": e.in_aftercare,
                "expiresAt": e.expires_at.map(|d| d.timestamp_millis()),
            })
        })
        .collect();
    Ok(Json(json!({ "items": items, "total": total })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RosterQuery {
    pub account_id: String,
    #[serde(default)]
    pub force: bool,
}

pub(super) async fn roster_endpoint(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<RosterQuery>,
) -> AppResult<Json<Value>> {
    // account 必须属于当前 workspace（fail-closed）。
    validate_account(&state, &admin.current_workspace, &query.account_id).await?;
    let ws = &admin.current_workspace;
    let acc = &query.account_id;

    // 决定本次返回的 friends + syncing（快照优先 / force 强拉 / 失败兜底旧快照）。
    let (friends, syncing): (Vec<mcp::RosterFriend>, bool) = if query.force {
        // 强制刷新(仅用户手动点「刷新」触发,前端自动轮询不带force):
        // 不再同步阻塞拉全量(异步工具首返pending、大body易TimedOut、占连接名额)。
        // 触发后台单飞刷新;有旧快照先返回旧快照(后台写好下次秒回),无则syncing:true。
        mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
        match mcp::read_roster_snapshot(&state, ws, acc).await? {
            Some(snap) => (snap.friends, false),
            None => (Vec::new(), true),
        }
    } else {
        // 非 force：快照优先。有快照秒回；stale 则后台自刷。无快照走同步拉一次。
        match mcp::read_roster_snapshot(&state, ws, acc).await? {
            Some(snap) => {
                if mcp::snapshot_is_stale(snap.fetched_at, mongodb::bson::DateTime::now()) {
                    mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
                }
                (snap.friends, false)
            }
            None => {
                // 首次无快照:不再同步阻塞 fetch_roster_for_account(6s窗口常拿pending、
                // 占连接名额)。立即返回syncing:true,后台单飞拉取,前端进只读轮询,
                // 后台写好快照后下一轮普通读秒出。
                mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
                (Vec::new(), true)
            }
        }
    };

    // 本地已入库联系人：wxid -> agent_status。拿到 friends（快照或实时）后统一拼装。
    let mut cursor = state
        .db
        .contacts()
        .find(doc! { "workspace_id": ws, "account_id": acc }, None)
        .await?;
    let mut status_by_wxid: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    while let Some(c) = cursor.try_next().await? {
        let status = match c.agent_status {
            crate::models::AgentStatus::Managed => "managed",
            _ => "normal",
        };
        status_by_wxid.insert(c.wxid, status.to_string());
    }

    let items: Vec<Value> = friends
        .into_iter()
        .map(|f| {
            let agent_status = status_by_wxid
                .get(&f.wxid)
                .cloned()
                .unwrap_or_else(|| "not_imported".to_string());
            json!({
                "wxid": f.wxid,
                "nickname": f.nickname,
                "remark": f.remark,
                "avatarUrl": f.avatar_url,
                "sex": f.sex,
                "isNonHuman": f.is_non_human,
                "agentStatus": agent_status,
            })
        })
        .collect();
    let total = items.len();
    Ok(Json(json!({ "items": items, "total": total, "syncing": syncing })))
}

/// 把 AI 生成的初始运营画像落库到指定联系人（切 managed + 画像 + 备注 + playbook）。
///
/// 同步 `enable_agent` 与异步 `handle_initial_profile_task` 两条路径共用此函数，
/// 避免画像落库逻辑（stage/intent 校验、老客户字段保留、operation_state 初始态）
/// 在两处复制后漂移。
pub(super) async fn apply_generated_profile_to_contact(
    state: &AppState,
    workspace_id: &str,
    contact: &Contact,
    note: &str,
    playbook_id: Option<ObjectId>,
    playbook_version: i32,
    generated: &agent::GeneratedOperationProfile,
) -> AppResult<()> {
    let object_id = contact
        .id
        .ok_or_else(|| AppError::BadRequest("contact missing _id".to_string()))?;
    let commitments_bson = commitments_with_optional_text(
        &contact.commitments,
        generated.last_commitment.as_deref(),
    );
    // #72：曾运营过的老客户重新启用时，保留已积累的 stage / operation_state /
    // commitments，不回退到 new_contact；只切 managed + 更新本次显式输入
    // （备注 / playbook / 画像）。全新客户才走完整初始化。
    let mut set_doc = doc! {
        "agent_status": "managed",
        "human_profile_note": note,
        "agent_profile": to_bson(&generated.agent_profile)?,
        "playbook_id": playbook_id,
        "playbook_version": playbook_version,
        "profile_attributes": generated.profile_attributes.clone(),
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let mut unset_doc = Document::new();
    if !is_previously_operated(contact) {
        // H13：初始 operation_state 从 active 状态机的 initial 态取（替代写死 "new_contact"）。
        let domain_config =
            agent::load_user_operation_domain_config(state, workspace_id).await?;
        let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
        // M4：AI 生成的初始画像 stage/intent 经 dimension_registry 校验后再落库
        // （对齐 management.rs 建档路径 + AI 主决策 validate_and_normalize_decision）。
        // AI 产出 → WriteIntent::MachineWrite：越界值 drop（不阻断建档），不像 admin 那样 reject。
        let (gen_stage, gen_intent) = validate_generated_stage_intent(
            state,
            &contact.account_id,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
        )
        .await?;
        insert_domain_stage_fields(&mut set_doc, gen_stage.as_deref(), gen_intent.as_deref(), true);
        set_doc.insert("commitments", commitments_bson);
        set_doc.insert("follow_up_policy", generated.follow_up_policy.clone());
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
    Ok(())
}

/// 异步初始画像任务处理器（`AgentTask.kind == "initial_profile"`）。
///
/// 批量托管（`batch_enable_endpoint`）不同步跑 LLM，而是给每个联系人入队本任务，
/// 由 worker 异步调 `build_initial_operation_profile` 生成画像并经共享的
/// [`apply_generated_profile_to_contact`] 落库。`task.content` 存本批共享运营备注。
pub async fn handle_initial_profile_task(
    state: &AppState,
    task: &crate::models::AgentTask,
) -> AppResult<()> {
    // 按 wxid+account 定位联系人（worker 上下文没有 _id）。
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "wxid": &task.contact_wxid,
            },
            None,
        )
        .await?;
    let Some(contact) = contact else {
        // 联系人已被删/清理：视为完成，不报错重试。
        return Ok(());
    };
    // 批量后又被手动取消托管的，跳过画像回填。
    if !matches!(contact.agent_status, crate::models::AgentStatus::Managed) {
        return Ok(());
    }
    let playbook = resolve_playbook_for_contact(
        state,
        &task.workspace_id,
        &task.account_id,
        contact.playbook_id.map(|o| o.to_hex()).as_deref(),
    )
    .await?;
    let generated = agent::build_initial_operation_profile(
        state,
        &task.workspace_id,
        &task.content,
        Some(&playbook),
    )
    .await?;
    apply_generated_profile_to_contact(
        state,
        &task.workspace_id,
        &contact,
        &task.content,
        playbook.id,
        playbook.version,
        &generated,
    )
    .await
}

pub async fn batch_enable_endpoint(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<crate::models::BatchEnableRequest>,
) -> AppResult<Json<Value>> {
    if payload.shared_note.trim().is_empty() {
        return Err(AppError::BadRequest("sharedNote is required".to_string()));
    }
    if payload.candidates.is_empty() {
        return Err(AppError::BadRequest("candidates is empty".to_string()));
    }
    // account 必须在 wechat_accounts 注册(否则 webhook 入站会被 resolve_account_context 拒收)。
    if state
        .db
        .accounts()
        .find_one(
            doc! { "workspace_id": &admin.current_workspace, "account_id": &payload.account_id },
            None,
        )
        .await?
        .is_none()
    {
        return Err(AppError::BadRequest(format!(
            "account_id={} 在 wechat_accounts 中未注册，无法批量启用 Agent 运营",
            payload.account_id
        )));
    }
    // 可选统一 playbook：校验存在(用 enable 的解析器,None 则账号默认)。
    let playbook = resolve_playbook_for_contact(
        &state,
        &admin.current_workspace,
        &payload.account_id,
        payload.playbook_id.as_deref(),
    )
    .await?;

    // 全新联系人的初始 operation_state 在此**同步**写入（不等异步画像回填）。
    // 竞态修复：batch upsert 只切 managed 时，若客户在 initial_profile 任务跑之前来消息，
    // gateway 的 apply_agent_updates 会无条件把 last_agent_run_at 推到 now
    // （agent/gateway.rs），随后 worker 回填时 is_previously_operated(shared.rs) 因该字段
    // 非空返回 true → 跳过 apply_generated_profile_to_contact 的初始化分支，联系人永远拿不到
    // 状态机 initial 态（operation_state 空 → 停滞计时器/状态机初始态缺位）。故把「纳入运营即
    // 拥有干净初始态」这个动作从异步回填提到同步 upsert——与 initial_profile 回填幂等（回填走
    // 全新分支时会再写同一 initial 态，取值一致）。域配置循环外查一次，避免每 candidate 查库。
    let domain_config =
        agent::load_user_operation_domain_config(&state, &admin.current_workspace).await?;
    let initial_state = agent::initial_operation_state_key(domain_config.as_ref());

    let mut enabled = 0i32;
    let mut queued = 0i32;
    for cand in &payload.candidates {
        // 幂等:已 managed 的不重复入队(但仍刷新 note/avatar)。
        let existing = state
            .db
            .contacts()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &payload.account_id,
                    "wxid": &cand.wxid,
                },
                None,
            )
            .await?;
        let already_managed = existing
            .as_ref()
            .map(|c| matches!(c.agent_status, crate::models::AgentStatus::Managed))
            .unwrap_or(false);

        // upsert 联系人:置 managed + sharedNote + playbook。
        // nickname/remark/avatar_url 只在候选真的带值时才写——MCP 好友列表常缺 remark，
        // 若无条件 $set None 会把已入库联系人已有的备注/头像覆盖成 null（重入运营即丢数据）。
        let mut set_doc = doc! {
            "workspace_id": &admin.current_workspace,
            "account_id": &payload.account_id,
            "wxid": &cand.wxid,
            "agent_status": "managed",
            "human_profile_note": &payload.shared_note,
            "playbook_id": playbook.id,
            "playbook_version": playbook.version,
            "updated_at": DateTime::now(),
        };
        if let Some(nickname) = &cand.nickname {
            set_doc.insert("nickname", nickname);
        }
        if let Some(remark) = &cand.remark {
            set_doc.insert("remark", remark);
        }
        if let Some(avatar_url) = &cand.avatar_url {
            set_doc.insert("avatar_url", avatar_url);
        }
        if let Some(sex) = cand.sex {
            set_doc.insert("sex", sex);
        }
        // 全新联系人（未入库，或已入库但从未被 Agent 运营过）同步落状态机 initial 态。
        // 老客户（is_previously_operated）不碰 operation_state——保留其已积累的运营历史，
        // 与 apply_generated_profile_to_contact 的老客户保留语义一致（#72）。
        let is_new_contact = existing.as_ref().map(|c| !is_previously_operated(c)).unwrap_or(true);
        if is_new_contact {
            set_doc.insert("operation_state", &initial_state);
            set_doc.insert(
                "operation_state_reason",
                "初次纳入 Agent 运营，等待后续互动确认阶段",
            );
            set_doc.insert("operation_state_confidence", 6);
            set_doc.insert("operation_state_updated_at", DateTime::now());
        }
        state
            .db
            .contacts()
            .update_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &payload.account_id,
                    "wxid": &cand.wxid,
                },
                doc! {
                    "$set": set_doc,
                    "$setOnInsert": { "created_at": DateTime::now() },
                },
                mongodb::options::UpdateOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        enabled += 1;

        if !already_managed {
            // 入队异步初始画像任务。
            state
                .db
                .tasks()
                .insert_one(
                    crate::models::AgentTask {
                        id: None,
                        workspace_id: admin.current_workspace.clone(),
                        account_id: payload.account_id.clone(),
                        contact_wxid: cand.wxid.clone(),
                        kind: "initial_profile".to_string(),
                        run_at: DateTime::now(),
                        expires_at: None,
                        content: payload.shared_note.clone(),
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
                        created_at: DateTime::now(),
                        updated_at: DateTime::now(),
                    },
                    None,
                )
                .await?;
            queued += 1;
        }
    }
    Ok(Json(json!({ "enabled": enabled, "queued": queued })))
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
        &admin.current_workspace,
        &payload.human_profile_note,
        Some(&playbook),
    )
    .await?;
    apply_generated_profile_to_contact(
        &state,
        &admin.current_workspace,
        &contact,
        &payload.human_profile_note,
        playbook.id,
        playbook.version,
        &generated,
    )
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
        &admin.current_workspace,
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
    // 标签可信度改造：note 重生成只写 AI 层（agent_profile/profile_attributes），
    // 不写 tags（裸字段已废）、不触碰 manual_tags（运营录入层归 manual_tags 端点管理）。
    let mut set_doc = doc! {
        "human_profile_note": payload.human_profile_note,
        "agent_profile": to_bson(&generated.agent_profile)?,
        "profile_attributes": generated.profile_attributes,
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    let mut unset_doc = doc! {};
    if !is_previously_operated(&contact) {
        // H13：初始 operation_state 从 active 状态机的 initial 态取（替代写死 "new_contact"）。
        let domain_config =
            agent::load_user_operation_domain_config(&state, &admin.current_workspace).await?;
        let initial_state = agent::initial_operation_state_key(domain_config.as_ref());
        let (gen_stage, gen_intent) = validate_generated_stage_intent(
            &state,
            &contact.account_id,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
        )
        .await?;
        insert_domain_stage_fields(
            &mut set_doc,
            gen_stage.as_deref(),
            gen_intent.as_deref(),
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

/// 客户级辅助模式 override 闭集校验（缺口 2）。三态：default（回落账号级）/
/// force_on / force_off。守 gateway 状态枚举闭集纪律。
pub(super) fn is_valid_assist_mode(mode: &str) -> bool {
    matches!(mode, "default" | "force_on" | "force_off")
}

/// 构造「撤销引荐」的 update 文档（红线 §6.3）。$unset 两个 domain_attributes
/// dotted-key（referred_specialist_at + referred_card_id），$set 刷新 updated_at。
/// 必须 unset 两键：escalation/logic.rs 判 referred_specialist_at 键存在才注入
/// 退辅助指引，两键都清才彻底退回主动运营态。抽纯函数以便单测 $unset 形态。
fn build_clear_referral_update() -> Document {
    let now = DateTime::now();
    doc! {
        "$unset": {
            format!("domain_attributes.{}", crate::models::REFERRED_SPECIALIST_AT_ATTR): "",
            format!("domain_attributes.{}", crate::models::REFERRED_CARD_ID_ATTR): "",
        },
        "$set": { "updated_at": now },
    }
}

/// POST /api/contacts/:id/clear-referral：撤销引荐，让客户恢复主动运营（红线 §6.3）。
/// $unset referred_specialist_at + referred_card_id 两键 → escalation 不再注入退辅助
/// 指引。workspace 隔离防 IDOR（仿 update_assist_override）。无 body。
pub async fn clear_referral(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    // workspace 隔离：跨 workspace / 不存在均 404（不泄漏存在性）。
    find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let update = build_clear_referral_update();
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            update,
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// PUT /api/contacts/:id/assist-override：写客户级辅助模式 override。
/// default → $unset（回落账号级 assist_mode_enabled）；force_on/force_off → $set。
/// workspace 隔离防 IDOR。
pub async fn update_assist_override(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<AssistOverrideRequest>,
) -> AppResult<Json<Value>> {
    if !is_valid_assist_mode(&payload.mode) {
        return Err(AppError::BadRequest(
            "mode must be default|force_on|force_off".to_string(),
        ));
    }
    let object_id = parse_object_id(&id)?;
    // workspace 隔离：跨 workspace / 不存在均 404（不泄漏存在性）。
    find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let attr = format!("domain_attributes.{}", crate::models::ASSIST_MODE_OVERRIDE_ATTR);
    let now = DateTime::now();
    let update = if payload.mode == "default" {
        doc! { "$unset": { &attr: "" }, "$set": { "updated_at": now } }
    } else {
        doc! { "$set": { &attr: &payload.mode, "updated_at": now } }
    };
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            update,
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true, "mode": payload.mode })))
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

#[derive(serde::Deserialize)]
pub struct ManualTagsRequest {
    pub tags: Vec<String>,
}

/// `PUT /api/contacts/:id/manual-tags`
///
/// 运营录入标签（运营权威层）。自由文本，去空白去重，AI 永不覆盖本字段。
pub async fn update_manual_tags(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ManualTagsRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let _ = find_contact_by_id(&state, &admin.current_workspace, &id).await?; // 存在 + workspace scope 校验
    let cleaned = normalize_manual_tags(&payload.tags);
    validate_manual_tags(&cleaned)?;
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! {
                "$set": {
                    "manual_tags": &cleaned,
                    "manual_tags_updated_at": DateTime::now(),
                    "manual_tags_by": &admin.username,
                }
            },
            None,
        )
        .await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

/// 去首尾空白、去空串、去重保序。自由文本，不查字典（设计选择）。
pub fn normalize_manual_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let s = t.trim();
        if !s.is_empty() && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    }
    out
}

/// 单标签字符上限。运营自由文本，但需兜底防超长标签膨胀 reply prompt。
pub const MANUAL_TAG_MAX_CHARS: usize = 64;
/// 标签条数上限。
pub const MANUAL_TAGS_MAX_COUNT: usize = 32;

/// 尺寸兜底校验（与 custom_agent_instructions 的 1000 字符上限同纪律）：
/// manual_tags 经 render_tags_for_prompt join 进 reply prompt，无上限会膨胀 token。
/// 入参须为已 normalize 的标签。
pub fn validate_manual_tags(tags: &[String]) -> Result<(), AppError> {
    if tags.len() > MANUAL_TAGS_MAX_COUNT {
        return Err(AppError::BadRequest(format!(
            "manual_tags 条数上限 {MANUAL_TAGS_MAX_COUNT} 个"
        )));
    }
    if let Some(t) = tags.iter().find(|t| t.chars().count() > MANUAL_TAG_MAX_CHARS) {
        return Err(AppError::BadRequest(format!(
            "单个 manual_tag 长度上限 {MANUAL_TAG_MAX_CHARS} 字符（超限：{t}）"
        )));
    }
    Ok(())
}

/// stage 是否算"发生变更"（决定 insert_domain_stage_fields 是否刷 customer_stage_updated_at）。
/// 红线：stage 未实际写入（`new_stage=None`：空串短路 / DropSilently）时绝不算变更——
/// 否则会无条件刷 customer_stage_updated_at，错误重置下游 stagnation 计时器（值没改却记
/// 了一次"刚变更"）。仅当真写了新 stage 且与旧值不同才算变更。
fn stage_changed(prev_stage: Option<&str>, new_stage: Option<&str>) -> bool {
    new_stage.is_some() && prev_stage != new_stage
}

/// M4：AI 生成的初始画像 stage/intent 落库前经 `system_taxonomies` 字典校验，
/// 与 `management.rs` 建档路径 + AI 主决策 `validate_and_normalize_decision` 对齐。
/// AI 产出 → `WriteIntent::MachineWrite`：越界值 `DropSilently`（返 `None`，不阻断
/// 建档），alias 归一到 canonical，已登记/未配置值原样通过。
async fn validate_generated_stage_intent(
    state: &AppState,
    account_id: &str,
    customer_stage: Option<&str>,
    intent_level: Option<&str>,
) -> AppResult<(Option<String>, Option<String>)> {
    use crate::agent::dimension_registry::{validate_dimension_value, WriteIntent};
    let gen_stage = match customer_stage {
        Some(v) => apply_admin_dim_validation(
            validate_dimension_value(&state.db, "customer_stage", v, account_id, WriteIntent::MachineWrite).await,
        )?,
        None => None,
    };
    let gen_intent = match intent_level {
        Some(v) => apply_admin_dim_validation(
            validate_dimension_value(&state.db, "intent_level", v, account_id, WriteIntent::MachineWrite).await,
        )?,
        None => None,
    };
    Ok((gen_stage, gen_intent))
}

pub async fn update_operation_profile(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<OperationProfileRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let current = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    // M1：admin 手填 stage/intent 经 taxonomy alias→canonical 归一（与 LLM 决策路径
    // 同口径），杜绝同一字段 canonical/alias 漂移污染下游派生。归一在 stage_changed
    // 判定之前，避免"admin 写 alias、库里是 canonical"被误判为变化。
    let new_stage = match normalize_optional(payload.customer_stage) {
        Some(v) => apply_admin_dim_validation(
            crate::agent::dimension_registry::validate_dimension_value(
                &state.db,
                "customer_stage",
                &v,
                &current.account_id,
                crate::agent::dimension_registry::WriteIntent::AdminWrite,
            )
            .await,
        )?,
        None => None,
    };
    let prev_stage = current
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str("customer_stage").ok().map(|s| s.to_string()));
    // stage 实际未写入（new_stage=None：空串短路 / DropSilently）时绝不算 stage 变更——
    // 否则 insert_domain_stage_fields(stage_changed=true) 会无条件刷 customer_stage_updated_at，
    // 错误重置下游 stagnation（停滞）计时器（stage 值没改却记了一次"刚变更"）。
    let stage_changed = stage_changed(prev_stage.as_deref(), new_stage.as_deref());
    let commitments_bson = commitments_with_optional_text(
        &current.commitments,
        normalize_optional(payload.last_commitment).as_deref(),
    );
    let mut set_doc = doc! {
        "commitments": commitments_bson,
        "follow_up_policy": normalize_optional(payload.follow_up_policy),
        "profile_updated_at": DateTime::now(),
        "updated_at": DateTime::now(),
    };
    // 与 gateway.rs 写回一致:profile_attributes 非空才写。前端「运营画像」表单
    // 不管理 profile_attributes(它由 AI 在 gateway 积累),PUT 时不带该字段 →
    // payload 反序列化为空 Document。无条件 $set 会把 AI 积累的画像清空(M13),
    // 故空则跳过、保留现值。
    if !payload.profile_attributes.is_empty() {
        set_doc.insert("profile_attributes", payload.profile_attributes);
    }
    let intent_level = match normalize_optional(payload.intent_level) {
        Some(v) => apply_admin_dim_validation(
            crate::agent::dimension_registry::validate_dimension_value(
                &state.db,
                "intent_level",
                &v,
                &current.account_id,
                crate::agent::dimension_registry::WriteIntent::AdminWrite,
            )
            .await,
        )?,
        None => None,
    };
    insert_domain_stage_fields(
        &mut set_doc,
        new_stage.as_deref(),
        intent_level.as_deref(),
        stage_changed,
    );
    // §3.7：relationship_type 走字典校验后写 domain_attributes（无 stagnation 计时语义，
    // 直接点路径键）。AdminDirect 通道：越界值 → Reject → 400（不静默落脏值）。
    // None → 不写键，不覆盖现值；alias 命中由 validate 内部归一到 canonical。
    if let Some(v) = normalize_optional(payload.relationship_type) {
        let validated = apply_admin_dim_validation(
            crate::agent::dimension_registry::validate_dimension_value(
                &state.db,
                "relationship_type",
                &v,
                &current.account_id,
                crate::agent::dimension_registry::WriteIntent::AdminWrite,
            )
            .await,
        )?;
        if let Some(canonical) = validated {
            set_doc.insert("domain_attributes.relationship_type", canonical);
        }
    }
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
/// 写库走 `$push contact.outcome_events` + 一条 `outcome_event_marked` 审计事件，
/// 落库核心委托给 [`add_outcome_event_inner`]（与将来支付回调共用同一路径）。
pub(super) async fn add_deal_event(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<DealEventRequest>,
) -> AppResult<Json<Value>> {
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    add_outcome_event_inner(
        &state,
        &contact,
        OutcomeEventInput {
            source: "manual".to_string(),
            marked_by: admin.username.clone(),
            audit_summary: "admin 手动登记成效事件".to_string(),
            amount: payload.amount,
            currency: payload.currency,
            verification: payload.verification,
            event_kind: payload.event_kind,
            product_id: payload.product_id,
            quantity: payload.quantity,
            note: payload.note,
            occurred_at_ms: payload.occurred_at_ms,
        },
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
        agent::build_initial_operation_profile(&state, &admin.current_workspace, &note, playbook.as_ref())
            .await?;
    let commitments_bson = commitments_with_optional_text(
        &contact.commitments,
        generated.last_commitment.as_deref(),
    );
    // #72：曾运营过的老客户 AI 重新分析时保留 stage / operation_state / commitments，
    // 不回退 new_contact；全新客户才完整初始化。
    let mut set_doc = doc! {
        "agent_profile": to_bson(&generated.agent_profile)?,
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
        let (gen_stage, gen_intent) = validate_generated_stage_intent(
            &state,
            &contact.account_id,
            generated.customer_stage.as_deref(),
            generated.intent_level.as_deref(),
        )
        .await?;
        insert_domain_stage_fields(
            &mut set_doc,
            gen_stage.as_deref(),
            gen_intent.as_deref(),
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
    let (in_quiet_hours, next_wake_at, quiet_hours_enabled) =
        compute_quiet_hours_view(&state, &contact).await?;
    Ok(Json(operation_health_json(
        &contact,
        &memory,
        latest_review.as_ref(),
        in_quiet_hours,
        next_wake_at,
        quiet_hours_enabled,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_preview_keeps_short_and_cuts_long() {
        assert_eq!(truncate_preview("你好", 30), "你好");
        assert_eq!(truncate_preview("  空白裁剪  ", 30), "空白裁剪");
        let long = "一二三四五六七八九十".repeat(5); // 50 chars
        let out = truncate_preview(&long, 30);
        assert_eq!(out.chars().count(), 31); // 30 + 省略号
        assert!(out.ends_with('…'));
    }

    #[test]
    fn contact_count_filters_isolate_workspace_and_account() {
        let (base, managed) = contact_count_filters("ws1", "acct1");
        // base：仅 workspace + account 隔离（与 list_contacts 同源）。
        assert_eq!(base.get_str("workspace_id").unwrap(), "ws1");
        assert_eq!(base.get_str("account_id").unwrap(), "acct1");
        assert!(base.get("agent_status").is_none(), "base 不得含 agent_status");
        // managed：在 base 基础上加 agent_status=managed。
        assert_eq!(managed.get_str("workspace_id").unwrap(), "ws1");
        assert_eq!(managed.get_str("account_id").unwrap(), "acct1");
        assert_eq!(managed.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn admin_dim_apply_maps_reject_to_bad_request() {
        use crate::agent::dimension_registry::DimValidation;
        let r = apply_admin_dim_validation(DimValidation::Reject("x 越界".into()));
        assert!(matches!(r, Err(AppError::BadRequest(_))));
        let ok = apply_admin_dim_validation(DimValidation::Accept("customer".into()));
        assert!(matches!(ok, Ok(Some(ref v)) if v == "customer"));
        let drop = apply_admin_dim_validation(DimValidation::DropSilently);
        assert!(matches!(drop, Ok(None)));
    }

    #[test]
    fn assist_mode_closed_set() {
        assert!(is_valid_assist_mode("default"));
        assert!(is_valid_assist_mode("force_on"));
        assert!(is_valid_assist_mode("force_off"));
        assert!(!is_valid_assist_mode("on"));
        assert!(!is_valid_assist_mode("true"));
        assert!(!is_valid_assist_mode(""));
        assert!(!is_valid_assist_mode("Force_On"));
    }

    #[test]
    fn stage_changed_false_when_stage_not_written() {
        // 不变式：new_stage=None（stage 未实际写入：空串短路 / DropSilently）时绝不算变更，
        // 即便旧值非空——否则会误刷 customer_stage_updated_at，错误重置 stagnation 计时。
        assert!(!stage_changed(Some("need_discovery"), None));
        assert!(!stage_changed(None, None));
        // 真写了新值且与旧值不同 → 变更；相同 → 不变。
        assert!(stage_changed(Some("need_discovery"), Some("solution_fit")));
        assert!(stage_changed(None, Some("new_contact")));
        assert!(!stage_changed(Some("solution_fit"), Some("solution_fit")));
    }

    #[test]
    fn normalize_manual_tags_trims_dedups_drops_empty() {
        let input = vec![
            "  vip ".to_string(),
            "vip".to_string(),
            "".to_string(),
            "老客户".to_string(),
        ];
        assert_eq!(
            normalize_manual_tags(&input),
            vec!["vip".to_string(), "老客户".to_string()]
        );
    }

    #[test]
    fn normalize_manual_tags_preserves_order() {
        let input = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        assert_eq!(
            normalize_manual_tags(&input),
            vec!["c".to_string(), "a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn normalize_manual_tags_handles_empty() {
        assert_eq!(normalize_manual_tags(&[]), Vec::<String>::new());
        assert_eq!(
            normalize_manual_tags(&["".to_string(), "  ".to_string()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn validate_manual_tags_accepts_within_limits() {
        let tags: Vec<String> = (0..MANUAL_TAGS_MAX_COUNT).map(|i| format!("标签{i}")).collect();
        assert!(validate_manual_tags(&tags).is_ok(), "正好满额应通过");
        assert!(validate_manual_tags(&["a".repeat(MANUAL_TAG_MAX_CHARS)]).is_ok(), "正好满长应通过");
        assert!(validate_manual_tags(&[]).is_ok());
    }

    #[test]
    fn validate_manual_tags_rejects_too_many() {
        let tags: Vec<String> =
            (0..MANUAL_TAGS_MAX_COUNT + 1).map(|i| format!("标签{i}")).collect();
        assert!(validate_manual_tags(&tags).is_err(), "超条数上限应拒绝");
    }

    #[test]
    fn validate_manual_tags_rejects_too_long() {
        let tags = vec!["x".repeat(MANUAL_TAG_MAX_CHARS + 1)];
        assert!(validate_manual_tags(&tags).is_err(), "超单标签长度应拒绝");
    }

    #[test]
    fn clear_referral_unset_doc_drops_both_keys() {
        // 红线 §6.3：撤销引荐必须 $unset 两个键才彻底退态——
        // escalation/logic.rs 判 referred_specialist_at 键存在，
        // 若只 unset 一个键则退辅助指引仍会注入。
        let update = build_clear_referral_update();
        let unset = update.get_document("$unset").expect("缺 $unset 子文档");
        assert!(
            unset.contains_key("domain_attributes.referred_specialist_at"),
            "$unset 须含 referred_specialist_at"
        );
        assert!(
            unset.contains_key("domain_attributes.referred_card_id"),
            "$unset 须含 referred_card_id"
        );
        // $set updated_at（与 update_assist_override 写法对齐）。
        let set = update.get_document("$set").expect("缺 $set 子文档");
        assert!(set.contains_key("updated_at"), "$set 须刷新 updated_at");
    }
}
