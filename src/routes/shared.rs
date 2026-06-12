//! 跨模块共享辅助：ObjectId 解析、联系人加载、JSON 序列化等。

use mongodb::{
    bson::{doc, oid::ObjectId, to_bson, to_document, Bson, DateTime, Document},
    options::{FindOneOptions, UpdateOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    error::{AppError, AppResult},
    models::{
        AgentDecisionReview, AgentRunLog, Contact, LlmCallLog, MemoryCandidate, MemoryCardTyped,
        OperatingMemory, OperationPlaybook, UserOperationGuidePreview,
    },
};

use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AccountScopedQuery {
    pub(super) account_id: Option<String>,
}

pub(super) fn parse_object_id(id: &str) -> AppResult<ObjectId> {
    ObjectId::parse_str(id).map_err(|_| AppError::BadRequest("invalid object id".to_string()))
}

/// #154：把用户输入转义为 Mongo `$regex` 字面量，防 ReDoS / regex 注入。
///
/// `list_contacts` 的搜索框 `q` 原样塞进 `Regex { pattern }`，恶意/手滑的
/// `(a+)+$`、`.*.*.*` 等 pattern 会让 Mongo 正则引擎灾难性回溯（DoS），
/// 元字符（`.`、`*`、`|` 等）也会改变查询语义。对所有正则特殊字符前置
/// `\` 后，输入被当作纯字面子串匹配（仍保留 `options:"i"` 大小写不敏感）。
pub(super) fn escape_regex_literal(input: &str) -> String {
    const SPECIAL: &[char] = &[
        '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '-',
    ];
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if SPECIAL.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// 从 `Contact.domain_attributes` 中读取销售域字段（已下线的 customer_stage / intent_level）。
/// 旧字段被 wiki 化，但部分 health/score/event 工具仍以 string-key 形式记录到事件文档。
pub(super) fn contact_domain_str(contact: &Contact, key: &str) -> Option<String> {
    contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_str(key).ok().map(|s| s.to_string()))
}

/// 把 customer_stage / intent_level 以 `domain_attributes.*` dotted-key 形式写入 `$set`。
///
/// `customer_stage`/`intent_level` 早已从 `Contact` 顶层删除、只存在于
/// `domain_attributes` 容器（见 models.rs 注释）。所有读端（planner / memory /
/// decision / health score）都从 `domain_attributes` 取值，因此写端必须写进同一
/// 容器，否则 serde 反序列化时顶层字段被丢弃、读端永远读不到。
///
/// 用 dotted-key（而非 clone 整个 `domain_attributes` 子文档再整体替换）有两个好处：
/// 字段级原子，不会覆盖容器内其它 key；与 escalation 路径写法一致。MongoDB 对不存在
/// 的 `domain_attributes` 会按 dotted-path 自动建嵌套对象。
///
/// `stage_changed` 为真时一并刷新 `domain_attributes.customer_stage_updated_at`
/// （planner 的 stage_stagnation 计时器依赖它）。容器级 `domain_attributes_updated_at`
/// 总是刷新。注意：调用方的同一 `$set` 内不能再出现顶层 `domain_attributes` 键，
/// 否则 MongoDB 会因 path conflict 报错。
///
/// universal-domain-adaptation 1D：dotted-key 写入 + stage 计时器逻辑已收敛到
/// `agent::domain_signals::insert_domain_signal_values` 单一内核（AI 决策路径与本
/// admin 路径共用）。本 wrapper 仅负责把两个 typed 维度参数装进 signals 容器、并
/// 保留 admin 路径「容器时间戳总是刷新」的既有契约（即便无维度写入也刷新）。
pub(super) fn insert_domain_stage_fields(
    set_doc: &mut Document,
    customer_stage: Option<&str>,
    intent_level: Option<&str>,
    stage_changed: bool,
) {
    let mut signals = Document::new();
    if let Some(stage) = customer_stage {
        signals.insert("customer_stage", stage);
    }
    if let Some(intent) = intent_level {
        signals.insert("intent_level", intent);
    }
    crate::agent::domain_signals::insert_domain_signal_values(set_doc, &signals, stage_changed);
    set_doc.insert("domain_attributes_updated_at", DateTime::now());
}

/// 该联系人是否曾被 Agent 运营过（用于"重新启用/重新建档不覆盖历史画像"判定）。
///
/// `Contact` 没有显式的 `first_managed_at` 字段，用 `last_agent_run_at`（跑过 Agent
/// 决策即非空）或 `last_outbound_at`（发过出站消息即非空）作为"曾运营过"的代理信号。
/// 任一非空即视为有运营历史，重新启用时应保留已积累的 stage / 画像 / operation_state，
/// 只切 `agent_status=managed`；全新客户才走完整初始化。
pub(super) fn is_previously_operated(contact: &Contact) -> bool {
    contact.last_agent_run_at.is_some() || contact.last_outbound_at.is_some()
}

pub(super) async fn validate_account(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<()> {
    let found = state
        .db
        .accounts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id
            },
            None,
        )
        .await?;
    if found.is_none() {
        return Err(AppError::NotFound("account not found".to_string()));
    }
    Ok(())
}

/// 按 `_id` 取联系人，**强制** workspace 隔离。
///
/// 安全契约：`workspace_id` 是必填参数，查询条件恒含 `workspace_id` 过滤。
/// 跨 workspace 的 contact_id 返回 `NotFound`（404，不泄漏存在性），而非
/// 返回他人数据。任何调用方都必须传入当前登录态的 `admin.current_workspace`
/// （webhook / worker 等内部路径传各自上下文的 workspace_id）。签名要求
/// workspace_id 即编译期 fail-closed——漏传无法通过编译。
pub(super) async fn find_contact_by_id(
    state: &AppState,
    workspace_id: &str,
    id: &str,
) -> AppResult<Contact> {
    let object_id = parse_object_id(id)?;
    state
        .db
        .contacts()
        .find_one(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("contact not found".to_string()))
}

pub async fn upsert_contact_from_value(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    contact_value: &Value,
) -> AppResult<Option<Contact>> {
    let wxid = contact_value
        .get("userName")
        .or_else(|| contact_value.get("username"))
        .or_else(|| contact_value.get("wxid"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let Some(wxid) = wxid else {
        return Ok(None);
    };
    let nickname = contact_value
        .get("nickName")
        .or_else(|| contact_value.get("nickname"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let remark = contact_value
        .get("remark")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let alias = contact_value
        .get("alias")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": &wxid
            },
            doc! {
                "$set": {
                    "nickname": &nickname,
                    "remark": &remark,
                    "alias": &alias,
                    "updated_at": DateTime::now()
                },
                "$setOnInsert": {
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "wxid": &wxid,
                    "agent_status": "normal",
                    "created_at": DateTime::now()
                }
            },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": &wxid
            },
            None,
        )
        .await?;
    Ok(contact)
}

pub(super) async fn ensure_operating_memory(
    state: &AppState,
    contact: &Contact,
) -> AppResult<OperatingMemory> {
    // H13：种子记忆卡无 operation_state 时回落状态机初始态（替代写死 "new_contact"）。
    let initial_state = agent::initial_operation_state_for_contact(state, contact).await?;
    if let Some(mut memory) = state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await?
    {
        if !agent::memory_card_has_signal(&effective_route_memory_card_typed(&memory)) {
            let seeded = agent::effective_memory_card_for_contact(&memory, contact, &initial_state);
            if agent::memory_card_has_signal(&seeded) {
                let updated_at = DateTime::now();
                memory.memory_card_version = memory.memory_card_version.saturating_add(1);
                let mut seeded_with_version = seeded;
                seeded_with_version
                    .extra
                    .insert("version", memory.memory_card_version);
                let seeded_doc = mongodb::bson::to_document(&seeded_with_version)
                    .unwrap_or_default();
                memory.memory_card = seeded_with_version;
                memory.memory_card_updated_at = Some(updated_at);
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
                                "memory_card": seeded_doc,
                                "memory_card_version": memory.memory_card_version,
                                "memory_card_updated_at": updated_at,
                                "updated_at": updated_at
                            }
                        },
                        None,
                    )
                    .await?;
            }
        }
        return Ok(memory);
    }
    let mut memory = OperatingMemory {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        user_understanding: doc! {
            "identity": "",
            "businessContext": "",
            "jobsToBeDone": Vec::<String>::new(),
            "painPoints": Vec::<String>::new(),
            "motivations": Vec::<String>::new(),
            "decisionStyle": "",
            "communicationPreference": "",
            "sensitivePoints": Vec::<String>::new()
        },
        relationship_state: doc! {
            "trustLevel": "unknown",
            "temperature": "unknown",
            "lastEmotion": "",
            "relationshipGoal": "",
            "doNotDo": Vec::<String>::new()
        },
        product_fit: doc! {
            "interestedProducts": Vec::<String>::new(),
            "fitReason": "",
            "objections": Vec::<String>::new(),
            "riskPoints": Vec::<String>::new(),
            "unknowns": Vec::<String>::new()
        },
        next_action: doc! {
            "goal": "",
            "recommendedMove": "",
            "avoid": "",
            "timing": "",
            "reason": ""
        },
        context_pack: doc! {
            "confirmedFacts": Vec::<String>::new(),
            "preferences": Vec::<String>::new(),
            "painPoints": Vec::<String>::new(),
            "objections": Vec::<String>::new(),
            "commitments": Vec::<String>::new(),
            "doNotDo": Vec::<String>::new(),
            "relationshipTimeline": Vec::<Document>::new(),
            "recentSignals": Vec::<String>::new(),
            "openQuestions": Vec::<String>::new(),
            "importantQuotes": Vec::<String>::new(),
            "stalenessWarnings": Vec::<String>::new(),
            "deprecatedFacts": Vec::<Document>::new(),
            "conflicts": Vec::<Document>::new()
        },
        context_pack_version: 0,
        context_pack_updated_at: None,
        // task 6.1：`memory_card` 现在是 `MemoryCardTyped`；构造时先用空容器，
        // 紧随其后的 `effective_memory_card_for_contact` 会把 Document 形态
        // 的种子卡通过 `MemoryCardTyped::from_document` 灌入。
        memory_card: MemoryCardTyped::default(),
        memory_card_version: 0,
        memory_card_updated_at: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let mut seeded_typed = agent::effective_memory_card_for_contact(&memory, contact, &initial_state);
    memory.memory_card_version = if agent::memory_card_has_signal(&seeded_typed) {
        1
    } else {
        0
    };
    seeded_typed
        .extra
        .insert("version", memory.memory_card_version);
    memory.memory_card = seeded_typed;
    memory.memory_card_updated_at = if memory.memory_card_version > 0 {
        Some(DateTime::now())
    } else {
        None
    };
    state
        .db
        .operating_memories()
        .insert_one(memory, None)
        .await?;
    state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::External("operating memory missing after insert".to_string()))
}

pub(super) async fn latest_decision_review(
    state: &AppState,
    contact: &Contact,
) -> AppResult<Option<AgentDecisionReview>> {
    state
        .db
        .decision_reviews()
        .find_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            FindOneOptions::builder()
                .sort(doc! { "created_at": -1 })
                .build(),
        )
        .await
        .map_err(Into::into)
}

pub(super) async fn resolve_playbook_for_contact(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    playbook_id: Option<&str>,
) -> AppResult<OperationPlaybook> {
    if let Some(playbook_id) = playbook_id {
        let object_id = parse_object_id(playbook_id)?;
        if let Some(playbook) = state
            .db
            .operation_playbooks()
            .find_one(
                doc! {
                    "_id": object_id,
                    "workspace_id": workspace_id,
                    "account_id": account_id
                },
                None,
            )
            .await?
        {
            return Ok(playbook);
        }
        return Err(AppError::NotFound(
            "operation playbook not found".to_string(),
        ));
    }
    super::playbooks::ensure_default_playbook(state, workspace_id, account_id).await
}

pub(super) fn operation_health_json(
    contact: &Contact,
    memory: &OperatingMemory,
    review: Option<&AgentDecisionReview>,
) -> Value {
    let scores = health_scores_document(contact, memory, review);
    let score = |key: &str| scores.get_i32(key).unwrap_or(0);
    json!({
        "scores": scores,
        "items": [
            health_item("userUnderstanding", "用户理解完整度", score("userUnderstanding"), "身份、痛点、动机、偏好和禁忌是否清楚"),
            health_item("relationshipQuality", "信任关系质量", score("relationshipQuality"), "当前互动是否适合推进，是否需要先建立信任"),
            health_item("productFit", "产品匹配清晰度", score("productFit"), "是否知道用户需求与产品价值之间的真实匹配"),
            health_item("rhythmRisk", "跟进节奏风险", score("rhythmRisk"), "是否存在过度打扰或冷却中的风险"),
            health_item("knowledgeGrounding", "知识匹配度", score("knowledgeGrounding"), "回应是否被 verified 知识支撑"),
            health_item("hallucinationRisk", "幻觉风险", score("hallucinationRisk"), "是否可能出现编造案例、承诺结果或产品事实不准确"),
            health_item("pressureRisk", "销售压迫感风险", score("pressureRisk"), "表达是否可能显得催促、强推或过度营销")
        ]
    })
}

pub(super) fn health_item(key: &str, label: &str, score: i32, detail: &str) -> Value {
    let tone = if key.ends_with("Risk") {
        if score >= 70 {
            "danger"
        } else if score >= 40 {
            "warn"
        } else {
            "good"
        }
    } else if score >= 75 {
        "good"
    } else if score >= 45 {
        "warn"
    } else {
        "danger"
    };
    json!({
        "key": key,
        "label": label,
        "score": score,
        "tone": tone,
        "detail": detail
    })
}

pub(super) fn health_scores_document(
    contact: &Contact,
    memory: &OperatingMemory,
    review: Option<&AgentDecisionReview>,
) -> Document {
    let user_understanding = score_presence(&[
        contact.human_profile_note.clone(),
        contact_domain_str(contact, "customer_stage"),
        contact_domain_str(contact, "intent_level"),
        contact.follow_up_policy.clone(),
        doc_string_ref(&memory.user_understanding, "identity"),
        doc_string_ref(&memory.user_understanding, "businessContext"),
        doc_list_text(&memory.user_understanding, "painPoints"),
        doc_list_text(&memory.user_understanding, "sensitivePoints"),
    ]);
    let relationship_quality = score_presence(&[
        doc_string_ref(&memory.relationship_state, "trustLevel"),
        doc_string_ref(&memory.relationship_state, "temperature"),
        doc_string_ref(&memory.relationship_state, "relationshipGoal"),
        doc_string_ref(&memory.relationship_state, "lastEmotion"),
    ]);
    let product_fit = score_presence(&[
        doc_list_text(&memory.product_fit, "interestedProducts"),
        doc_string_ref(&memory.product_fit, "fitReason"),
        doc_list_text(&memory.product_fit, "objections"),
        doc_list_text(&memory.product_fit, "unknowns"),
    ]);
    let review_score = |key: &str| {
        review
            .and_then(|item| item.scores.get_i32(key).ok())
            .unwrap_or(0)
            .clamp(0, 10)
    };
    let mut rhythm_risk = if contact.cooldown_until.is_some() {
        55
    } else {
        20
    };
    if contact.last_agent_run_at.is_some() && contact.last_message_at.is_none() {
        rhythm_risk += 10;
    }
    doc! {
        "userUnderstanding": user_understanding,
        "relationshipQuality": relationship_quality,
        "productFit": product_fit,
        "rhythmRisk": rhythm_risk.clamp(0, 100),
        // P0-4：与 Phase B 三闸/软闸口径对齐——前端 healthFromScores 读
        // `knowledgeGrounding / hallucinationRisk`，后端必须发对应键。
        // 旧 5 闸键 `factRisk` 已下线，不再写入；`pressureRisk` 作为软闸保留。
        "knowledgeGrounding": review_score("knowledgeGroundingScore") * 10,
        "hallucinationRisk": review_score("hallucinationScore") * 10,
        "pressureRisk": review_score("pressureRisk") * 10
    }
}

pub(super) fn score_presence(values: &[Option<String>]) -> i32 {
    let present = values
        .iter()
        .filter(|item| {
            item.as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty() && *text != "unknown")
                .is_some()
        })
        .count() as i32;
    ((present * 100) / values.len().max(1) as i32).clamp(0, 100)
}

pub(super) async fn apply_contact_changes(
    state: &AppState,
    contact: &Contact,
    changes: &Document,
) -> AppResult<()> {
    let mut set_doc = Document::new();
    if let Some(value) = doc_get_string(changes, "humanProfileNote") {
        set_doc.insert("human_profile_note", value);
    }
    if let Some(value) = doc_get_string_vec(changes, "tags") {
        set_doc.insert("tags", to_bson(&value)?);
    }
    if let Some(value) = doc_get_string(changes, "customerStage") {
        // M2：customer_stage 实际变化时同步刷新 customer_stage_updated_at。
        let prev = contact_domain_str(contact, "customer_stage");
        let stage_changed = prev.as_deref().map(|s| s != value.as_str()).unwrap_or(true);
        let intent = doc_get_string(changes, "intentLevel");
        insert_domain_stage_fields(&mut set_doc, Some(&value), intent.as_deref(), stage_changed);
    } else if let Some(value) = doc_get_string(changes, "intentLevel") {
        insert_domain_stage_fields(&mut set_doc, None, Some(&value), false);
    }
    if let Some(value) = doc_get_string(changes, "followUpPolicy") {
        set_doc.insert("follow_up_policy", value);
    }
    if let Some(value) = doc_get_string(changes, "operationState") {
        set_doc.insert("operation_state", value);
        set_doc.insert("operation_state_updated_at", DateTime::now());
    }
    if let Some(value) = doc_get_string(changes, "operationStateReason") {
        set_doc.insert("operation_state_reason", value);
    }
    if let Some(value) = doc_get_document(changes, "operationPolicy") {
        set_doc.insert("operation_policy", value.clone());
    }
    if set_doc.is_empty() {
        return Ok(());
    }
    set_doc.insert("updated_at", DateTime::now());
    state
        .db
        .contacts()
        .update_one(doc! { "_id": contact.id }, doc! { "$set": set_doc }, None)
        .await?;
    Ok(())
}

pub(super) async fn apply_memory_changes(
    state: &AppState,
    contact: &Contact,
    changes: &Document,
) -> AppResult<()> {
    let Some(memory_patch) = doc_get_document(changes, "memory") else {
        return Ok(());
    };
    let memory = ensure_operating_memory(state, contact).await?;
    let mut set_doc = Document::new();
    for (json_key, db_key, existing) in [
        (
            "userUnderstanding",
            "user_understanding",
            memory.user_understanding,
        ),
        (
            "relationshipState",
            "relationship_state",
            memory.relationship_state,
        ),
        ("productFit", "product_fit", memory.product_fit),
        ("nextAction", "next_action", memory.next_action),
    ] {
        if let Some(patch) = doc_get_document(&memory_patch, json_key) {
            let mut merged = existing;
            merge_document(&mut merged, patch);
            set_doc.insert(db_key, merged);
        }
    }
    if set_doc.is_empty() {
        return Ok(());
    }
    set_doc.insert("updated_at", DateTime::now());
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

pub(super) async fn apply_playbook_changes(
    state: &AppState,
    contact: &Contact,
    changes: &Document,
) -> AppResult<()> {
    let Some(playbook_patch) = doc_get_document(changes, "playbookPatch") else {
        return Ok(());
    };
    let Some(playbook_id) = contact.playbook_id else {
        return Ok(());
    };
    let mut set_doc = Document::new();
    for (json_key, db_key) in [
        ("replyStyle", "reply_style"),
        ("followUpMethod", "follow_up_method"),
        ("forbiddenRules", "forbidden_rules"),
        ("successCriteria", "success_criteria"),
    ] {
        if let Some(value) = doc_get_string(&playbook_patch, json_key) {
            set_doc.insert(db_key, value);
        }
    }
    if set_doc.is_empty() {
        return Ok(());
    }
    set_doc.insert("created_by", "guide_optimized");
    set_doc.insert("updated_at", DateTime::now());
    state
        .db
        .operation_playbooks()
        .update_one(
            doc! { "_id": playbook_id, "account_id": &contact.account_id },
            doc! { "$set": set_doc, "$inc": { "version": 1 } },
            None,
        )
        .await?;
    Ok(())
}

pub(super) async fn apply_domain_changes(
    state: &AppState,
    workspace_id: &str,
    changes: &Document,
) -> AppResult<()> {
    let Some(runtime_patch) = doc_get_document(changes, "domainRuntimeParameters") else {
        return Ok(());
    };
    if runtime_patch.is_empty() {
        return Ok(());
    }
    let Some(config) = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations"
            },
            None,
        )
        .await?
    else {
        return Ok(());
    };
    let mut runtime = config.runtime_parameters;
    merge_document(&mut runtime, runtime_patch);
    state
        .db
        .operation_domain_configs()
        .update_one(
            doc! { "_id": config.id },
            doc! { "$set": { "runtime_parameters": runtime, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(())
}

pub(super) fn build_guide_preview_prompt(
    instruction: &str,
    mode: &str,
    contact: &Contact,
    memory: &OperatingMemory,
    playbook: Option<&OperationPlaybook>,
    review: Option<&AgentDecisionReview>,
    health: &Value,
) -> String {
    format!(
        r#"请为微信用户运营 Agent 生成一份“修改预览”，不要直接输出聊天话术。

输出 JSON：
{{
  "summary": "用业务用户能理解的话说明你建议怎么调",
  "impactScope": "current_contact | all_user_operations | agent_personality",
  "scopeReason": "说明为什么是这个影响范围",
  "readableChanges": [
    "将更新用户画像",
    "将调整跟进节奏",
    "不会影响其他用户"
  ],
  "healthScores": {{
    "userUnderstanding": 0-100,
    "relationshipQuality": 0-100,
    "productFit": 0-100,
    "rhythmRisk": 0-100,
    "pressureRisk": 0-100,
    "factRisk": 0-100
  }},
  "suggestedChanges": {{
    "humanProfileNote": "可选，新的运营备注（运营 admin 录入）",
    "tags": ["可选标签"],
    "customerStage": "可选客户阶段",
    "intentLevel": "可选意向等级",
    "followUpPolicy": "可选跟进策略",
    "operationState": "可选运营状态",
    "operationStateReason": "可选状态原因",
    "operationPolicy": {{
      "requireUserReplyBeforeNextOutbound": false,
      "maxConsecutiveAgentOutbounds": 1,
      "cooldownUntil": "可选 RFC3339 时间",
      "blockedTopics": ["可选禁聊主题"],
      "notes": "用业务语言说明这条硬策略从哪里来"
    }},
    "memory": {{
      "userUnderstanding": {{}},
      "relationshipState": {{}},
      "productFit": {{}},
      "nextAction": {{}}
    }},
    "playbookPatch": {{
      "replyStyle": "仅当用户明确要求调整整体方法时输出",
      "followUpMethod": "仅当用户明确要求调整整体方法时输出",
      "forbiddenRules": "仅当用户明确要求调整整体方法时输出"
    }},
    "domainRuntimeParameters": {{
      "maxDailyTouches": 2
    }}
  }},
  "riskWarnings": ["可能影响全部用户的方法论或运行参数必须说明"]
}}

原则：
- 默认只调整当前好友的画像、记忆、备注和跟进策略。
- impactScope 默认必须是 current_contact。
- 只有用户明确说“全局、全部用户、默认方法、整体人格、所有好友”时，impactScope 才能是 all_user_operations 或 agent_personality。
- 只有用户明确说“全局、全部用户、默认方法、运行参数”时，才输出 playbookPatch 或 domainRuntimeParameters。
- readableChanges 必须用产品语言，不要出现 JSON、Prompt、runtime parameters、playbook、状态机。
- 如果用户说“不要再主动发第二条、等他回复、降低打扰、先冷却”等，必须输出 operationPolicy，把自然语言变成硬规则。
- 不要编造用户事实，不确定的信息写入 unknowns。
- 输出必须是业务人员能读懂的中文。

模式：{}
用户指令：{}

当前好友：
wxid：{}
昵称：{}
备注：{}
运营备注：{}
标签：{}
客户阶段：{}
意向等级：{}
跟进策略：{}
运营状态：{} / {}

运营记忆：{}

当前方法：{}

最近复盘：{}

当前健康度：{}"#,
        mode,
        instruction,
        contact.wxid,
        contact.nickname.as_deref().unwrap_or(""),
        contact.remark.as_deref().unwrap_or(""),
        contact.human_profile_note.as_deref().unwrap_or(""),
        contact.tags.join(", "),
        contact_domain_str(contact, "customer_stage").as_deref().unwrap_or(""),
        contact_domain_str(contact, "intent_level").as_deref().unwrap_or(""),
        contact.follow_up_policy.as_deref().unwrap_or(""),
        contact.operation_state.as_deref().unwrap_or(""),
        contact.operation_state_reason.as_deref().unwrap_or(""),
        serde_json::to_string(&operating_memory_json(memory.clone())).unwrap_or_default(),
        playbook.map(playbook_brief).unwrap_or_default(),
        review
            .and_then(|item| item.review_summary.clone())
            .unwrap_or_else(|| "暂无".to_string()),
        serde_json::to_string(health).unwrap_or_default()
    )
}

pub(super) fn playbook_brief(playbook: &OperationPlaybook) -> String {
    format!(
        "名称：{}\n描述：{}\n表达风格：{}\n跟进方法：{}\n禁止行为：{}",
        playbook.name,
        playbook.description.as_deref().unwrap_or(""),
        playbook.reply_style.as_deref().unwrap_or(""),
        playbook.follow_up_method.as_deref().unwrap_or(""),
        playbook.forbidden_rules.as_deref().unwrap_or("")
    )
}

pub(super) fn guide_preview_json(preview: UserOperationGuidePreview) -> Value {
    json!({
        "id": preview.id.map(|id| id.to_hex()).unwrap_or_default(),
        "accountId": preview.account_id,
        "contactId": preview.contact_id.to_hex(),
        "contactWxid": preview.contact_wxid,
        "instruction": preview.instruction,
        "mode": preview.mode,
        "status": preview.status,
        "summary": preview.summary,
        "impactScope": if preview.impact_scope.trim().is_empty() { "current_contact".to_string() } else { preview.impact_scope },
        "scopeReason": if preview.scope_reason.trim().is_empty() { "默认只影响当前好友。".to_string() } else { preview.scope_reason },
        "readableChanges": preview.readable_changes,
        "healthScores": preview.health_scores,
        "suggestedChanges": preview.suggested_changes,
        "riskWarnings": preview.risk_warnings,
        "createdAt": crate::models::dt_to_string(preview.created_at),
        "updatedAt": crate::models::dt_to_string(preview.updated_at)
    })
}

pub(super) fn operating_memory_json(memory: OperatingMemory) -> Value {
    json!({
        "id": memory.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": memory.workspace_id,
        "accountId": memory.account_id,
        "contactWxid": memory.contact_wxid,
        "userUnderstanding": memory.user_understanding,
        "relationshipState": memory.relationship_state,
        "productFit": memory.product_fit,
        "nextAction": memory.next_action,
        "memoryCard": effective_route_memory_card(&memory),
        "memoryCardVersion": memory.memory_card_version,
        "memoryCardUpdatedAt": memory.memory_card_updated_at.and_then(crate::models::dt_to_string),
        "updatedAt": crate::models::dt_to_string(memory.updated_at)
    })
}

pub(super) fn effective_route_memory_card(memory: &OperatingMemory) -> Document {
    // task 6.3：`memory_card` 现在是 `MemoryCardTyped`，typed 才是 canonical
    // 表示。本 helper 只在路由 JSON 响应这种"对外 wire shape"边界才把 typed
    // 转回 Document，业务路径请直接用 `effective_route_memory_card_typed`。
    effective_route_memory_card_typed(memory).to_document()
}

pub(super) fn effective_route_memory_card_typed(memory: &OperatingMemory) -> MemoryCardTyped {
    if !memory.memory_card.is_empty() {
        memory.memory_card.clone()
    } else if !memory.context_pack.is_empty() {
        MemoryCardTyped::from_document(&memory.context_pack)
    } else {
        let mut extra = Document::new();
        extra.insert("coreProfile", doc! {});
        extra.insert("relationshipState", doc! {});
        extra.insert("preferences", Vec::<String>::new());
        extra.insert("doNotDo", Vec::<String>::new());
        extra.insert("commitments", Vec::<String>::new());
        extra.insert("objections", Vec::<String>::new());
        extra.insert("openLoops", Vec::<String>::new());
        extra.insert("recentEpisodeSummary", "");
        extra.insert("conflicts", Vec::<Document>::new());
        MemoryCardTyped {
            core_facts: Vec::new(),
            recent_facts: Vec::new(),
            deprecated_facts: Vec::new(),
            extra,
        }
    }
}

pub(super) fn memory_candidate_json(item: MemoryCandidate) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactWxid": item.contact_wxid,
        "runId": item.run_id,
        "source": item.source,
        "candidates": item.candidates,
        "memoryWriteScore": item.memory_write_score,
        "status": item.status,
        "reason": item.reason,
        "createdAt": crate::models::dt_to_string(item.created_at),
        "updatedAt": crate::models::dt_to_string(item.updated_at)
    })
}

pub(super) fn llm_call_log_json(item: LlmCallLog) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactWxid": item.contact_wxid,
        "runId": item.run_id,
        "promptKey": item.prompt_key,
        "model": item.model,
        "status": item.status,
        "latencyMs": item.latency_ms,
        "promptTokens": item.prompt_tokens,
        "completionTokens": item.completion_tokens,
        "totalTokens": item.total_tokens,
        "promptCacheHitTokens": item.prompt_cache_hit_tokens,
        "promptCacheMissTokens": item.prompt_cache_miss_tokens,
        "error": item.error,
        "createdAt": crate::models::dt_to_string(item.created_at)
    })
}

pub(super) fn decision_review_json(review: AgentDecisionReview) -> Value {
    json!({
        "id": review.id.map(|id| id.to_hex()).unwrap_or_default(),
        "runId": review.run_id,
        "workspaceId": review.workspace_id,
        "accountId": review.account_id,
        "contactWxid": review.contact_wxid,
        "inboundMessageId": review.inbound_message_id,
        "replyText": review.reply_text,
        "approved": review.approved,
        "scores": review.scores,
        "formulaBreakdown": review.formula_breakdown,
        "risks": review.risks,
        "rewriteInstruction": review.rewrite_instruction,
        "reviewSummary": review.review_summary,
        "playbookId": review.playbook_id.map(|id| id.to_hex()),
        "playbookVersion": review.playbook_version,
        "usedKnowledgeIds": review.used_knowledge_ids.into_iter().map(|id| id.to_hex()).collect::<Vec<_>>(),
        "promptVersions": review.prompt_versions,
        "operationState": review.operation_state,
        "nextBestAction": review.next_best_action,
        "contextPackSnapshot": review.context_pack_snapshot,
        "domainConfigSnapshot": review.domain_config_snapshot,
        "runtimeParametersSnapshot": review.runtime_parameters_snapshot,
        "sendGatewayResult": review.send_gateway_result,
        "outcomeStatus": review.outcome_status,
        "reactionAnalysis": review.reaction_analysis,
        "status": review.status,
        "createdAt": crate::models::dt_to_string(review.created_at)
    })
}

pub(super) fn agent_run_json(item: AgentRunLog) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "workspaceId": item.workspace_id,
        "accountId": item.account_id,
        "contactWxid": item.contact_wxid,
        "runId": item.run_id,
        "triggerKind": item.trigger_kind,
        "status": item.status,
        "planner": item.planner,
        "context": item.context,
        "knowledgeRoute": item.knowledge_route,
        "decision": item.decision,
        "review": item.review,
        "gatewayResult": item.gateway_result,
        "error": item.error,
        "createdAt": crate::models::dt_to_string(item.created_at)
    })
}

pub(super) fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

pub(super) fn json_string_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| json_string(value, key))
}

pub(super) fn json_document_any(value: &Value, keys: &[&str]) -> Option<Document> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|item| to_document(item).ok())
            .filter(|doc| !doc.is_empty())
    })
}

pub(super) fn json_string_vec_any(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(|item| {
                if let Some(items) = item.as_array() {
                    Some(
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                            .map(ToString::to_string)
                            .collect::<Vec<_>>(),
                    )
                } else {
                    item.as_str().map(|text| vec![text.trim().to_string()])
                }
            })
        })
        .unwrap_or_default()
}

pub(super) fn json_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

pub(super) fn doc_get_string(doc: &Document, key: &str) -> Option<String> {
    doc.get_str(key)
        .ok()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

pub(super) fn doc_get_document(doc: &Document, key: &str) -> Option<Document> {
    doc.get_document(key).ok().cloned()
}

pub(super) fn doc_get_string_vec(doc: &Document, key: &str) -> Option<Vec<String>> {
    match doc.get(key) {
        Some(Bson::Array(items)) => {
            let values = items
                .iter()
                .filter_map(|item| match item {
                    Bson::String(text) => Some(text.trim().to_string()),
                    _ => None,
                })
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>();
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
        Some(Bson::String(text)) => {
            let values = text
                .split([',', '，', '\n'])
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
        _ => None,
    }
}

pub(super) fn doc_string_ref(doc: &Document, key: &str) -> Option<String> {
    doc.get_str(key)
        .ok()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
}

pub(super) fn doc_list_text(doc: &Document, key: &str) -> Option<String> {
    match doc.get(key) {
        Some(Bson::Array(items)) => {
            let joined = items
                .iter()
                .filter_map(|item| match item {
                    Bson::String(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(", ");
            if joined.trim().is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        Some(Bson::String(text)) if !text.trim().is_empty() => Some(text.clone()),
        _ => None,
    }
}

pub(super) fn merge_document(target: &mut Document, patch: Document) {
    for (key, value) in patch {
        if !matches!(value, Bson::Null) {
            target.insert(key, value);
        }
    }
}

/// agent-autonomy-loop M2：把"单字符串承诺（来自 LLM 输出 / 前端 payload）"
/// 升级为结构化 `Vec<CommitmentRepr>` 的 BSON 表达。
///
/// - `existing`: 联系人当前的 commitments（可能含旧 `Plain(String)` 元素）
/// - `new_text`: 单条新承诺文本，`None` 或空串视为"无新承诺"，直接返回 existing 的 BSON
///
/// 写入策略：去重（按 `text() == new_text`）；超出 8 条时从前淘汰。
pub(super) fn commitments_with_optional_text(
    existing: &[crate::models::CommitmentRepr],
    new_text: Option<&str>,
) -> Bson {
    let mut commitments: Vec<crate::models::CommitmentRepr> = existing.to_vec();
    if let Some(text) = new_text.map(str::trim).filter(|s| !s.is_empty()) {
        let already_present = commitments.iter().any(|c| c.text() == text);
        if !already_present {
            commitments.push(crate::models::CommitmentRepr::Structured(
                crate::models::CommitmentEntry::from_plain_text(text.to_string()),
            ));
            if commitments.len() > 8 {
                let drop = commitments.len() - 8;
                commitments.drain(0..drop);
            }
        }
    }
    to_bson(&commitments).unwrap_or(Bson::Array(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::escape_regex_literal;
    use super::insert_domain_stage_fields;
    use mongodb::bson::Document;

    #[test]
    fn escape_regex_literal_neutralizes_redos_pattern() {
        // 经典灾难性回溯 pattern：转义后每个元字符都被字面化
        assert_eq!(escape_regex_literal("(a+)+$"), "\\(a\\+\\)\\+\\$");
        assert_eq!(escape_regex_literal(".*.*.*"), "\\.\\*\\.\\*\\.\\*");
    }

    #[test]
    fn escape_regex_literal_leaves_plain_text_untouched() {
        assert_eq!(escape_regex_literal("张三"), "张三");
        assert_eq!(escape_regex_literal("alice 99"), "alice 99");
        assert_eq!(escape_regex_literal(""), "");
    }

    #[test]
    fn escape_regex_literal_escapes_every_special_char() {
        for ch in [
            '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '-',
        ] {
            let input: String = ch.to_string();
            let escaped = escape_regex_literal(&input);
            assert_eq!(escaped, format!("\\{ch}"), "char {ch:?} not escaped");
        }
    }

    // #65：customer_stage / intent_level 必须写进 domain_attributes 容器（dotted-key），
    // 绝不写文档顶层——顶层会被 serde 丢弃、读端（planner/memory/decision）读不到。
    #[test]
    fn insert_domain_stage_fields_uses_dotted_keys_never_top_level() {
        let mut set_doc = Document::new();
        insert_domain_stage_fields(&mut set_doc, Some("solution_fit"), Some("high"), true);
        assert_eq!(
            set_doc.get_str("domain_attributes.customer_stage").ok(),
            Some("solution_fit")
        );
        assert_eq!(
            set_doc.get_str("domain_attributes.intent_level").ok(),
            Some("high")
        );
        assert!(set_doc.contains_key("domain_attributes.customer_stage_updated_at"));
        assert!(set_doc.contains_key("domain_attributes_updated_at"));
        // 绝不出现顶层字段（serde 会丢弃）。
        assert!(!set_doc.contains_key("customer_stage"));
        assert!(!set_doc.contains_key("intent_level"));
        assert!(!set_doc.contains_key("customer_stage_updated_at"));
    }

    // stage 未变化时不刷新 customer_stage_updated_at（planner stagnation 计时器不被无谓重置）。
    #[test]
    fn insert_domain_stage_fields_skips_updated_at_when_stage_unchanged() {
        let mut set_doc = Document::new();
        insert_domain_stage_fields(&mut set_doc, Some("need_discovery"), None, false);
        assert_eq!(
            set_doc.get_str("domain_attributes.customer_stage").ok(),
            Some("need_discovery")
        );
        assert!(!set_doc.contains_key("domain_attributes.customer_stage_updated_at"));
        // intent 为 None 时不写 intent 键。
        assert!(!set_doc.contains_key("domain_attributes.intent_level"));
    }

    // None stage + None intent：只刷容器时间戳，不写任何 stage/intent 键（不覆盖已有值）。
    #[test]
    fn insert_domain_stage_fields_no_values_only_touches_container_ts() {
        let mut set_doc = Document::new();
        insert_domain_stage_fields(&mut set_doc, None, None, false);
        assert!(!set_doc.contains_key("domain_attributes.customer_stage"));
        assert!(!set_doc.contains_key("domain_attributes.intent_level"));
        assert!(set_doc.contains_key("domain_attributes_updated_at"));
    }
}

