//! 决策请示通道——台账 CRUD 层（pending 台账增删查改 / 知识缺口提案 / relay task 入队）。
//! 全部 async + db 访问。

use super::logic::{is_duplicate_key_error, is_pending_dedupe_conflict, short_code_from_seed};
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, OperationKnowledgeChunk, PrincipalDecision,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED,
};
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};

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
            decision: None,
            authorization_expires_at: None,
            is_generalizable,
            knowledge_proposal_emitted: false,
            last_holding_reply_ms: None,
            last_pushed_at_ms: Some(now.timestamp_millis()),
            created_at: now,
            updated_at: now,
            resolved_at: None,
            resolved_via: None,
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

/// 查某 workspace 下某领导 wxid 当前所有 pending 台账（按创建时间升序）。
pub(crate) async fn list_pending_for_principal(
    state: &AppState,
    workspace_id: &str,
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
    contact_wxid: &str,
    category: &str,
) -> AppResult<bool> {
    let count = state
        .db
        .agent_principal_escalations()
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
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
    short_code: &str,
    decision: &PrincipalDecision,
    authorization_expires_at: Option<DateTime>,
    resolved_via: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let now = DateTime::now();
    let decision_bson = mongodb::bson::to_bson(decision)?;
    let mut set = doc! {
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "decision": decision_bson,
        "updated_at": now,
        "resolved_at": now,
        "resolved_via": resolved_via,
    };
    if let Some(exp) = authorization_expires_at {
        set.insert("authorization_expires_at", exp);
    }
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! { "short_code": short_code, "status": PRINCIPAL_ESCALATION_STATUS_PENDING },
            doc! { "$set": set },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
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
    let title = format!("真人决策沉淀（待审核）：{}", escalation.reason);
    let body = format!(
        "源自客户「{}」请示 #{}。\n卡点：{}\n领导裁决：{}\n约束：{}",
        escalation.contact_wxid,
        escalation.short_code,
        escalation.reason,
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
    from_wxid: &str,
) -> AppResult<Option<String>> {
    use futures::TryStreamExt;
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
        if crate::agent::escalation::policy::is_decider_for_config(&cfg, from_wxid) {
            return Ok(Some(cfg.domain));
        }
    }
    Ok(None)
}

/// 创建 principal_decision_relay task（立即可执行）。
pub(crate) async fn enqueue_relay_task(state: &AppState, entry: &AgentPrincipalEscalation) -> AppResult<()> {
    let now = DateTime::now();
    let task = AgentTask {
        id: None,
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
    state.db.tasks().insert_one(&task, None).await?;
    Ok(())
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

/// 改派 pending 请示到另一位决策人（仅 pending 可改派；workspace 约束防 IDOR）。
///
/// 仅在推卡成功后调用，落库同时刷新 updated_at，使 age（scan 用 now-updated_at）自"改派
/// 成功时刻"起算——新决策人由此获得完整 timeout 窗。
pub(crate) async fn reassign_escalation(
    state: &AppState,
    workspace_id: &str,
    short_code: &str,
    to_wxid: &str,
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let updated = state
        .db
        .agent_principal_escalations()
        .find_one_and_update(
            doc! {
                "workspace_id": workspace_id,
                "short_code": short_code,
                "status": PRINCIPAL_ESCALATION_STATUS_PENDING,
            },
            doc! { "$set": {
                "principal_wxid": to_wxid,
                "updated_at": DateTime::now(),
                // KD-05：改派=给 next 的新推送时刻，与 updated_at 同步刷新，骚扰门据此正确计对 next 的打扰。
                "last_pushed_at_ms": DateTime::now().timestamp_millis(),
            } },
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?;
    Ok(updated)
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
            doc! { "workspace_id": workspace_id, "principal_wxid": principal_wxid },
            mongodb::options::FindOneOptions::builder()
                // KD-05：按真实最近推送时刻排序取最近一次推卡时刻（改派刷新后才准）。
                .sort(doc! { "last_pushed_at_ms": -1 })
                .build(),
        )
        .await?;
    // last_pushed_at_ms 已是 epoch ms；旧行缺字段→None（m031 backfill 前），用 created_at 兜底保口径。
    Ok(latest.and_then(|e| e.last_pushed_at_ms.or_else(|| Some(e.created_at.timestamp_millis()))))
}
