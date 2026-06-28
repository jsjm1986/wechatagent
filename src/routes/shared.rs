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
        OperatingMemory, OperationPlaybook, OutcomeEvent, OutcomeProductRef,
        UserOperationGuidePreview,
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

/// admin 写画像维度：把 [`crate::agent::dimension_registry::validate_dimension_value`]
/// 的三通道处置映射成写入决策。`Accept(canonical)` → 写入该值；`DropSilently` → 不写
/// 该键（admin 直写通道理论不触发 Drop，兜底）；`Reject(reason)` → `400 BadRequest`
/// （越界值不静默落库脏值）。
///
/// 单一真相源：guide-preview apply（本模块）与 update_operation_profile（contacts.rs）
/// 两条 admin 路径共用此 helper，避免逻辑重复漂移。
pub(super) fn apply_admin_dim_validation(
    v: crate::agent::dimension_registry::DimValidation,
) -> AppResult<Option<String>> {
    use crate::agent::dimension_registry::DimValidation::*;
    match v {
        Accept(s) => Ok(Some(s)),
        DropSilently => Ok(None),
        Reject(r) => Err(AppError::BadRequest(r)),
    }
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
    let items = health_items_from_scores(&scores);
    json!({
        "scores": scores,
        "items": items
    })
}

/// 把一个 health scores document 组装成 canonical 7 项 health items 数组（JSON）。
///
/// FE-1：原先这 7 行 `health_item` 调用同时出现在 `operation_health_json`（正常加载
/// 路径）与 guide preview 响应里，量纲/风险反转口径必须一致——抽出单一来源消除重复。
/// 缺失键 `get_i32` 失败回落 0；tone 方向（风险类高分=坏、非风险类高分=好）由
/// `health_item` 按 `key.ends_with("Risk")` 自动判定，量纲 0-100。
pub(super) fn health_items_from_scores(scores: &Document) -> Value {
    let score = |key: &str| scores.get_i32(key).unwrap_or(0);
    json!([
        health_item("userUnderstanding", "用户理解完整度", score("userUnderstanding"), "身份、痛点、动机、偏好和禁忌是否清楚"),
        health_item("relationshipQuality", "信任关系质量", score("relationshipQuality"), "当前互动是否适合推进，是否需要先建立信任"),
        health_item("productFit", "产品匹配清晰度", score("productFit"), "是否知道用户需求与产品价值之间的真实匹配"),
        health_item("rhythmRisk", "跟进节奏风险", score("rhythmRisk"), "是否存在过度打扰或冷却中的风险"),
        health_item("knowledgeGrounding", "知识匹配度", score("knowledgeGrounding"), "回应是否被 verified 知识支撑"),
        health_item("hallucinationRisk", "幻觉风险", score("hallucinationRisk"), "是否可能出现编造案例、承诺结果或产品事实不准确"),
        health_item("pressureRisk", "销售压迫感风险", score("pressureRisk"), "表达是否可能显得催促、强推或过度营销")
    ])
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
        // M1：admin 手填值经 dimension_registry 校验（alias→canonical 归一 + 越界拒绝），
        // 与 contacts.rs::update_operation_profile 同口径。guide-preview apply 是 admin
        // 权威写入 → WriteIntent::AdminWrite：越界值 `?` 提前返回 400（不静默落脏值）。
        // DropSilently（admin 通道理论不触发，仅空串兜底）→ 跳过该维度写入，不影响其它字段。
        let validated_stage = apply_admin_dim_validation(
            agent::dimension_registry::validate_dimension_value(
                &state.db,
                "customer_stage",
                &value,
                &contact.account_id,
                agent::dimension_registry::WriteIntent::AdminWrite,
            )
            .await,
        )?;
        let intent = match doc_get_string(changes, "intentLevel") {
            Some(v) => apply_admin_dim_validation(
                agent::dimension_registry::validate_dimension_value(
                    &state.db,
                    "intent_level",
                    &v,
                    &contact.account_id,
                    agent::dimension_registry::WriteIntent::AdminWrite,
                )
                .await,
            )?,
            None => None,
        };
        if let Some(value) = validated_stage {
            // M2：customer_stage 实际变化时同步刷新 customer_stage_updated_at（归一后再比较）。
            let prev = contact_domain_str(contact, "customer_stage");
            let stage_changed = prev.as_deref().map(|s| s != value.as_str()).unwrap_or(true);
            insert_domain_stage_fields(&mut set_doc, Some(&value), intent.as_deref(), stage_changed);
        } else if intent.is_some() {
            // stage 被 drop 但 intent 通过：仍写 intent（stage_changed=false，不刷 stage 计时）。
            insert_domain_stage_fields(&mut set_doc, None, intent.as_deref(), false);
        }
    } else if let Some(value) = doc_get_string(changes, "intentLevel") {
        let validated = apply_admin_dim_validation(
            agent::dimension_registry::validate_dimension_value(
                &state.db,
                "intent_level",
                &value,
                &contact.account_id,
                agent::dimension_registry::WriteIntent::AdminWrite,
            )
            .await,
        )?;
        if let Some(value) = validated {
            insert_domain_stage_fields(&mut set_doc, None, Some(&value), false);
        }
    }
    if let Some(value) = doc_get_string(changes, "followUpPolicy") {
        set_doc.insert("follow_up_policy", value);
    }
    if let Some(value) = doc_get_string(changes, "operationState") {
        // 修复（问题 F）：admin 手改 operation_state 也必须过状态机迁移闸，与 AI 决策
        // 路径（gateway C2）同一道 check_state_transition。此前 admin 直写不校验，可置入
        // 与 customer_stage / 状态机矛盾的值（甚至状态机里不存在的态），造成 planner（读
        // customer_stage）与 policy enforcement（读 operation_state）口径漂移，且休眠
        // contact 无 AI 消息触发 C2 自愈时漂移无限期。admin 是交互操作 → 非法迁移**硬拒**
        // （BadRequest），让操作者立即看到而非静默吞。domain_config=None（未配状态机）时
        // check_state_transition fail-open，行为不变。
        let domain_config = agent::load_user_operation_domain_config_for_contact(
            state,
            &contact.workspace_id,
            &contact.wxid,
        )
        .await?;
        if let Some(reason) = agent::check_state_transition(
            domain_config.as_ref(),
            contact.operation_state.as_deref(),
            &value,
        ) {
            return Err(AppError::BadRequest(format!(
                "非法的 operation_state 迁移：{reason}（admin 手改也须符合状态机；如需任意设值请先调整状态机定义）"
            )));
        }
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
        agent::render_tags_for_prompt(&contact.manual_tags, &contact.confirmed_tags),
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
    // FE-1：preview.health_scores 是 scores document；这里复用 health_items_from_scores
    // （与 operation_health_json 正常加载路径同口径）把它组装成构建好的 7 项 items，
    // 让前端直接消费正确量纲/风险反转的 items，不必自己重建。无论 scores 来自 LLM
    // 还是 health_scores_document 兜底，都过同一组装。
    let health_items = health_items_from_scores(&preview.health_scores);
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
        // 构建好的 health（scores + items 同形态于正常加载路径）。
        "health": { "scores": &preview.health_scores, "items": health_items },
        // 旧 `healthScores` 键保留以兼容尚未迁移的读端；前端迁移后可移除。
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

pub(super) fn decision_review_json(
    review: AgentDecisionReview,
    final_review_status: Option<String>,
    hold_category: Option<String>,
) -> Value {
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
        "finalReviewStatus": final_review_status,
        "holdCategory": hold_category,
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

/// admin 直登成交允许的事件方向闭集（§4.5）。
fn validate_event_kind(input: Option<&str>) -> AppResult<String> {
    match input.map(str::trim) {
        None | Some("") | Some("deal") => Ok("deal".to_string()),
        Some("reversal") => Ok("reversal".to_string()),
        Some(other) => Err(AppError::BadRequest(format!(
            "eventKind 仅接受 deal | reversal（收到 {other:?}）"
        ))),
    }
}

/// admin 直登成交允许的可信度闭集（§4.4 + §5.5）。`conversation_inferred` 是 AI 侧
/// 疑似线索，绝不经 admin 直登写入——它只能由 §5.5 疑似线索通道产出，经核实后转
/// `staff_confirmed`。
fn validate_deal_verification(input: Option<&str>) -> AppResult<String> {
    match input.map(str::trim) {
        None | Some("") | Some("staff_confirmed") => Ok("staff_confirmed".to_string()),
        Some("payment_verified") => Ok("payment_verified".to_string()),
        Some(other) => Err(AppError::BadRequest(format!(
            "verification 仅接受 staff_confirmed | payment_verified（收到 {other:?}）；\
             conversation_inferred 疑似线索须经核实后才落成交，不走直登通道"
        ))),
    }
}

/// 一条成效事件的落库入参（与触发来源解耦：admin 手动登记 / 将来支付回调共用）。
///
/// 校验（amount/currency/verification/event_kind 闭集 + reversal 须带 product）一律在
/// [`add_outcome_event_inner`] 内完成，故此处字段是「原始意图」而非已校验值。
pub(crate) struct OutcomeEventInput {
    /// 事件来源；本阶段 admin 直登恒 `"manual"`，将来支付回调用 `"payment"`。
    pub source: String,
    /// 标记人（admin 用户名 / 将来支付单号或网关标识），用于审计。
    pub marked_by: String,
    /// 审计事件 summary（参数化原硬编码"admin 手动登记成效事件"）。
    pub audit_summary: String,
    pub amount: Option<i64>,
    pub currency: Option<String>,
    /// 原始可信度意图；inner 走 [`validate_deal_verification`] 闭集校验。
    pub verification: Option<String>,
    /// 原始事件方向意图；inner 走 [`validate_event_kind`] 闭集校验。
    pub event_kind: Option<String>,
    pub product_id: Option<String>,
    pub quantity: Option<u32>,
    pub note: Option<String>,
    pub occurred_at_ms: Option<i64>,
}

/// 往 `contact.outcome_events` append 一条成效事件 + 写一条 `outcome_event_marked` 审计，
/// 返回构造出的 [`OutcomeEvent`]。
///
/// 与触发来源无关：admin 手动登记（`add_deal_event`）/ 将来支付回调共用**同一条落库
/// 路径**——校验闭集、`product_ref` 订单式快照、`$push`、审计形状单一真相源，绝不让两
/// 个入口漂移出两套成交写入逻辑。**不重读 contact**（调用方按需重读返回视图）。
///
/// IDOR（§3.5）：产品解引用与 update filter 一律用传入 `contact.workspace_id` 收窄，
/// 等价 admin handler 原先的 `admin.current_workspace`。
pub(crate) async fn add_outcome_event_inner(
    state: &AppState,
    contact: &Contact,
    input: OutcomeEventInput,
) -> AppResult<OutcomeEvent> {
    // find_contact_by_id 必带 `_id` 查回，理论不可能为 None；显式兜底不 silent unwrap。
    let object_id = contact
        .id
        .ok_or_else(|| AppError::External("contact 缺少 _id，无法登记成效事件".to_string()))?;
    // 金额整数化：amount 是最小币种单位整数（分），i64 无 NaN/Inf，只查非负。
    if !crate::models::is_valid_minor_amount(input.amount) {
        return Err(AppError::BadRequest(
            "amount 必须是非负整数（最小币种单位，如分）".to_string(),
        ));
    }
    if let Some(cur) = input
        .currency
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !crate::models::is_valid_currency_code(cur) {
            return Err(AppError::BadRequest(
                "currency 必须是 ISO-4217 三位大写字母币种码（如 CNY）".to_string(),
            ));
        }
    }
    let verification = validate_deal_verification(input.verification.as_deref())?;
    let event_kind = validate_event_kind(input.event_kind.as_deref())?;
    let trimmed_pid = input
        .product_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // reversal 必须关联 product_id：无产品标的的退款没有可抵消对象，G4 投影无从下手（§4.5）。
    if event_kind == "reversal" && trimmed_pid.is_none() {
        return Err(AppError::BadRequest(
            "reversal 退款事件必须关联 product_id（指明抵消哪个产品的持有）".to_string(),
        ));
    }
    // 给定 product_id 时，从本 workspace 产品表解引用，冻结成交当时快照（§4.3）。
    // IDOR（§3.5）：filter 必含 workspace_id；正向成交只认 active（archived 不可新成交），
    // 但 reversal 要能抵消"成交后才被下架"的产品，故退款放宽到任意 status。
    let product_ref = match trimmed_pid {
        Some(pid) => {
            let mut filter = doc! {
                "workspace_id": &contact.workspace_id,
                "product_id": pid,
            };
            if event_kind != "reversal" {
                filter.insert("status", "active");
            }
            let product = state
                .db
                .products()
                .find_one(filter, None)
                .await?
                .ok_or_else(|| {
                    if event_kind == "reversal" {
                        AppError::BadRequest(format!(
                            "product_id {pid:?} 不存在于本工作区，无法登记退款"
                        ))
                    } else {
                        AppError::BadRequest(format!(
                            "product_id {pid:?} 不是本工作区的 active 产品，无法关联成交"
                        ))
                    }
                })?;
            Some(OutcomeProductRef {
                product_id: product.product_id.clone(),
                name: product.name.clone(),
                unit_price: product.price,
                sku: product.sku.clone(),
                quantity: input.quantity.unwrap_or(1).max(1),
                // G4 #4：冻结成交当时的售后期天数，使产品日后 archived 也不丢已购客户的 in_aftercare。
                entitlement_days: agent::entitlements::entitlement_days_of(&product),
            })
        }
        None => None,
    };
    let now = DateTime::now();
    let outcome_event = OutcomeEvent {
        marked_at: now,
        occurred_at: input.occurred_at_ms.map(DateTime::from_millis),
        amount: input.amount,
        currency: normalize_optional(input.currency),
        source: input.source.clone(),
        marked_by: input.marked_by.clone(),
        note: normalize_optional(input.note),
        verification: verification.clone(),
        product_ref: product_ref.clone(),
        event_kind: event_kind.clone(),
    };
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &contact.workspace_id },
            doc! {
                "$push": { "outcome_events": to_bson(&outcome_event)? },
                "$set": { "updated_at": now },
            },
            None,
        )
        .await?;
    agent::write_event_for_account(
        state,
        &contact.account_id,
        Some(&contact.wxid),
        "outcome_event_marked",
        "ok",
        &input.audit_summary,
        Some(doc! {
            "source": &input.source,
            "markedBy": &input.marked_by,
            "amount": input.amount,
            "hasOccurredAt": input.occurred_at_ms.is_some(),
            "verification": &verification,
            "eventKind": &event_kind,
            "productId": product_ref.as_ref().map(|p| p.product_id.clone()),
        }),
    )
    .await?;
    Ok(outcome_event)
}

#[cfg(test)]
mod tests {
    use super::escape_regex_literal;
    use super::insert_domain_stage_fields;
    use super::{guide_preview_json, health_items_from_scores};
    use super::{validate_deal_verification, validate_event_kind};
    use crate::models::UserOperationGuidePreview;
    use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

    /// 构造一条带指定 health_scores 的 guide preview（其余字段填最小合法值）。
    fn preview_with_scores(scores: Document) -> UserOperationGuidePreview {
        UserOperationGuidePreview {
            id: Some(ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_id: ObjectId::new(),
            contact_wxid: "wxid_guide_preview_test".to_string(),
            instruction: "更关注客户情绪".to_string(),
            mode: "tune".to_string(),
            status: "pending".to_string(),
            summary: "测试预览".to_string(),
            impact_scope: "current_contact".to_string(),
            scope_reason: String::new(),
            readable_changes: vec![],
            health_scores: scores,
            suggested_changes: Document::new(),
            risk_warnings: vec![],
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    /// FE-1 后端回归：guide preview 响应必须含**构建好的** `health.items`（7 项，
    /// 复用 `health_item` 的量纲/风险反转），而非仅裸 `healthScores`。
    ///
    /// 此前 `guide_preview_json` 只输出 `healthScores`（scores document），前端只好
    /// 用一个坏函数自己重建 items（key 错/量纲错/风险方向反）。本测试钉死后端直接
    /// 发对的 items：风险类高分 → danger，非风险类高分 → good。
    #[test]
    fn guide_preview_json_builds_health_items_with_correct_risk_tone() {
        // hallucinationRisk=80（风险类高分 → danger）；userUnderstanding=80（非风险高分 → good）。
        let scores = doc! {
            "userUnderstanding": 80i32,
            "relationshipQuality": 50i32,
            "productFit": 30i32,
            "rhythmRisk": 20i32,
            "knowledgeGrounding": 70i32,
            "hallucinationRisk": 80i32,
            "pressureRisk": 10i32,
        };
        let body = guide_preview_json(preview_with_scores(scores));

        // health.items 是构建好的 canonical 7 项数组。
        let items = body["health"]["items"]
            .as_array()
            .expect("health.items 必须存在且为数组");
        assert_eq!(items.len(), 7, "health items 必须是 canonical 7 项");

        let keys: Vec<&str> = items.iter().filter_map(|i| i["key"].as_str()).collect();
        assert!(keys.contains(&"hallucinationRisk"));
        assert!(keys.contains(&"userUnderstanding"));

        // 风险类高分 → danger（验证 tone 方向：风险维度高分=坏）。
        let hallucination = items
            .iter()
            .find(|i| i["key"] == "hallucinationRisk")
            .expect("hallucinationRisk item present");
        assert_eq!(
            hallucination["tone"], "danger",
            "风险类 hallucinationRisk=80 应判 danger（高分=坏，量纲 0-100）"
        );
        assert_eq!(hallucination["score"], 80);

        // 非风险类高分 → good（验证正常方向：高分=好）。
        let understanding = items
            .iter()
            .find(|i| i["key"] == "userUnderstanding")
            .expect("userUnderstanding item present");
        assert_eq!(
            understanding["tone"], "good",
            "非风险类 userUnderstanding=80 应判 good（高分=好）"
        );

        // scores 仍随响应返回；旧 `healthScores` 键保留以兼容现有读端。
        assert!(body["health"]["scores"].is_object(), "health.scores 应保留 scores document");
        assert!(body["healthScores"].is_object(), "healthScores 旧键应保留向后兼容");
    }

    /// DRY 抽出的 `health_items_from_scores` 与组装函数同口径：缺失键回落 0、
    /// 7 项齐全、风险/非风险 tone 方向正确。
    #[test]
    fn health_items_from_scores_is_canonical_seven_items() {
        // 空 scores：所有键 get_i32 失败回落 0。
        let items_value = health_items_from_scores(&Document::new());
        let items = items_value.as_array().expect("items 数组");
        assert_eq!(items.len(), 7, "缺值时仍输出 canonical 7 项");
        // 全 0：非风险类 0 → danger（低分=坏）；风险类 0 → good（低分=好）。
        let understanding = items
            .iter()
            .find(|i| i["key"] == "userUnderstanding")
            .expect("userUnderstanding present");
        assert_eq!(understanding["score"], 0);
        assert_eq!(understanding["tone"], "danger", "非风险类 0 分应 danger");
        let pressure = items
            .iter()
            .find(|i| i["key"] == "pressureRisk")
            .expect("pressureRisk present");
        assert_eq!(pressure["tone"], "good", "风险类 0 分应 good");
    }

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

    // ── 支付闭环前置重构：成效事件落库校验闭集（搬移自 contacts.rs，语义须逐字不变）──

    // event_kind 闭集：缺省/空/deal → deal；reversal → reversal；其余 → BadRequest。
    #[test]
    fn validate_event_kind_accepts_closed_set_and_rejects_others() {
        assert_eq!(validate_event_kind(None).unwrap(), "deal");
        assert_eq!(validate_event_kind(Some("")).unwrap(), "deal");
        assert_eq!(validate_event_kind(Some("  ")).unwrap(), "deal");
        assert_eq!(validate_event_kind(Some("deal")).unwrap(), "deal");
        assert_eq!(validate_event_kind(Some(" reversal ")).unwrap(), "reversal");
        assert!(validate_event_kind(Some("refund")).is_err());
        assert!(validate_event_kind(Some("DEAL")).is_err());
    }

    // verification 闭集：缺省/空/staff_confirmed → staff_confirmed；payment_verified 直通；
    // conversation_inferred 绝不经直登通道（与 §5.5 红线一致）。
    #[test]
    fn validate_deal_verification_rejects_conversation_inferred_via_direct_path() {
        assert_eq!(
            validate_deal_verification(None).unwrap(),
            "staff_confirmed"
        );
        assert_eq!(
            validate_deal_verification(Some("")).unwrap(),
            "staff_confirmed"
        );
        assert_eq!(
            validate_deal_verification(Some("staff_confirmed")).unwrap(),
            "staff_confirmed"
        );
        assert_eq!(
            validate_deal_verification(Some(" payment_verified ")).unwrap(),
            "payment_verified"
        );
        // AI 侧疑似线索不得经直登写入。
        assert!(validate_deal_verification(Some("conversation_inferred")).is_err());
        assert!(validate_deal_verification(Some("guessed")).is_err());
    }

    /// 契约快照：llm_call_log_json。id 走 ObjectId→hex;account_id/contact_wxid 是
    /// Option（给 Some 穿透 string 分支）;retry_count/final_status 赋值但投影不下发。
    #[test]
    fn llm_call_log_json_matches_contract_fixture() {
        use super::llm_call_log_json;
        use crate::models::LlmCallLog;
        use mongodb::bson::{oid::ObjectId, DateTime};

        let item = LlmCallLog {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899c001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: Some("acc-1".to_string()),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: Some("run-1".to_string()),
            prompt_key: "user.reply".to_string(),
            model: "provider-a".to_string(),
            status: "success".to_string(),
            latency_ms: 1200,
            prompt_tokens: 800,
            completion_tokens: 200,
            total_tokens: 1000,
            prompt_cache_hit_tokens: 600,
            prompt_cache_miss_tokens: 200,
            error: Some("none".to_string()),
            retry_count: 1,
            final_status: Some("success".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = llm_call_log_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("llm_call_log", projected);
    }

    /// 契约快照：memory_candidate_json。candidates:Vec<Document> 桥接,放纯标量
    /// （String/i32）避免 BSON 包装泄漏;id 走 ObjectId→hex。
    #[test]
    fn memory_candidate_json_matches_contract_fixture() {
        use super::memory_candidate_json;
        use crate::models::MemoryCandidate;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let item = MemoryCandidate {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899d001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: "wxid_abc".to_string(),
            run_id: Some("run-1".to_string()),
            source: "consolidator".to_string(),
            candidates: vec![doc! { "text": "客户偏好下午沟通", "confidence": 8i32 }],
            memory_write_score: 7,
            status: "pending".to_string(),
            reason: Some("高价值事实".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = memory_candidate_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("memory_candidate", projected);
    }

    /// 契约快照：operating_memory_json。memory_card/context_pack 都空 → memoryCard
    /// 走 default skeleton 分支（确定形状）;4 个下发 Document 放纯标量;id→hex。
    /// context_pack 系列 + created_at 赋值但投影不下发。
    #[test]
    fn operating_memory_json_matches_contract_fixture() {
        use super::operating_memory_json;
        use crate::models::{MemoryCardTyped, OperatingMemory};
        use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};

        let memory = OperatingMemory {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899e001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: "wxid_abc".to_string(),
            user_understanding: doc! { "identity": "企业主", "businessContext": "餐饮连锁" },
            relationship_state: doc! { "trustLevel": "high", "temperature": "warm" },
            product_fit: doc! { "fitReason": "需要私域自动化" },
            next_action: doc! { "action": "follow_up", "due": "2026-07-01" },
            context_pack: Document::new(),
            context_pack_version: 0,
            context_pack_updated_at: None,
            memory_card: MemoryCardTyped::default(),
            memory_card_version: 3,
            memory_card_updated_at: Some(DateTime::from_millis(1_700_000_050_000)),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let projected = operating_memory_json(memory);
        crate::routes::contract_snapshot::assert_contract_fixture("operating_memory", projected);
    }

    /// 契约快照：agent_run_json。AgentRunLog 35 字段全量构造（无 Default）;6 个
    /// 下发 Document 放纯标量;投影只下发 15 键,其余 20 字段不下发。
    #[test]
    fn agent_run_json_matches_contract_fixture() {
        use super::agent_run_json;
        use crate::models::AgentRunLog;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let item = AgentRunLog {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899f001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: "run-1".to_string(),
            trigger_kind: "inbound_message".to_string(),
            status: "completed".to_string(),
            planner: doc! { "step": "plan", "n": 1i32 },
            context: doc! { "loaded": true },
            knowledge_route: doc! { "matched": 2i32 },
            decision: doc! { "action": "reply" },
            review: doc! { "approved": true },
            gateway_result: doc! { "status": "sent" },
            error: Some("none".to_string()),
            token_budget: 8000,
            tokens_used: 1200,
            llm_calls_used: 3,
            degraded_reasons: vec!["none".to_string()],
            lifecycle: "completed".to_string(),
            source_event_id: "evt-1".to_string(),
            source_kind: "inbound_message".to_string(),
            error_summary: Some("ok".to_string()),
            abort_reason: Some("none".to_string()),
            revision_applied: false,
            revision_reason: "none".to_string(),
            pre_revision_summary: Some("before".to_string()),
            post_revision_summary: Some("after".to_string()),
            self_critique: Some("looks good".to_string()),
            autonomy_mode: "auto".to_string(),
            conversation_mode: "consultative".to_string(),
            conversation_mode_reason: Some("customer_stage:proposal_evaluation".to_string()),
            final_review_status: "approved_sent".to_string(),
            outbox_status: Some("sent".to_string()),
            memory_consolidator_warnings: vec!["none".to_string()],
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = agent_run_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("agent_run", projected);
    }

    /// 契约快照：decision_review_json（29 键）。AgentDecisionReview 29 字段全量构造;
    /// 9 个下发 Document 放纯标量;used_knowledge_ids:Vec<ObjectId>→hex 字符串数组（不泄漏）;
    /// final_review_status/hold_category 是函数参数,给 Some;reaction_claimed_at/
    /// reviewer_misjudge_signal 赋值但投影不下发。
    #[test]
    fn decision_review_json_matches_contract_fixture() {
        use super::decision_review_json;
        use crate::models::AgentDecisionReview;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let review = AgentDecisionReview {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889a0001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: Some("run-1".to_string()),
            inbound_message_id: Some("msg-1".to_string()),
            reply_text: Some("您好，已收到".to_string()),
            approved: true,
            scores: doc! { "humanLikeScore": 8i32, "pressureRisk": 2i32 },
            formula_breakdown: doc! { "weighted": "ok" },
            risks: vec!["low".to_string()],
            rewrite_instruction: Some("无需改写".to_string()),
            review_summary: Some("通过".to_string()),
            playbook_id: Some(ObjectId::parse_str("64a1f2c3e4b5a697889a0002").unwrap()),
            playbook_version: Some(2),
            used_knowledge_ids: vec![
                ObjectId::parse_str("64a1f2c3e4b5a697889a0003").unwrap(),
            ],
            prompt_versions: doc! { "user.reply": "v2" },
            operation_state: Some("negotiation".to_string()),
            next_best_action: doc! { "action": "follow_up" },
            context_pack_snapshot: doc! { "ctx": "snap" },
            domain_config_snapshot: doc! { "domain": "user_operations" },
            runtime_parameters_snapshot: doc! { "temp": "0.7" },
            send_gateway_result: doc! { "status": "sent" },
            outcome_status: Some("replied".to_string()),
            reaction_analysis: doc! { "sentiment": "positive" },
            reaction_claimed_at: Some(DateTime::from_millis(1_700_000_050_000)),
            reviewer_misjudge_signal: Some("none".to_string()),
            status: "approved".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = decision_review_json(
            review,
            Some("approved_sent".to_string()),
            Some("none".to_string()),
        );
        crate::routes::contract_snapshot::assert_contract_fixture("decision_review", projected);
    }
}
