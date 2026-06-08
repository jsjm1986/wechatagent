//! 决策请示通道——台账 CRUD 层（pending 台账增删查改 / 知识缺口提案 / relay task 入队）。
//! 全部 async + db 访问。

use super::logic::{is_duplicate_key_error, short_code_from_seed};
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, OperationKnowledgeChunk, PrincipalDecision,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED,
};
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};

/// 读取该 workspace+domain 的领导 wxid。未配置返回 None（= 请示通道未启用）。
pub(crate) async fn principal_decider_wxid(
    state: &AppState,
    workspace_id: &str,
    domain: &str,
) -> AppResult<Option<String>> {
    let cfg = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! { "workspace_id": workspace_id, "domain": domain, "current_version": true },
            None,
        )
        .await?;
    Ok(cfg.and_then(|c| c.principal_decider))
}

/// 插入一条 pending 台账。短码碰撞（唯一索引报错）时重试至多 5 次。
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
) -> AppResult<AgentPrincipalEscalation> {
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
            created_at: now,
            updated_at: now,
            resolved_at: None,
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
                return Ok(saved);
            }
            Err(e) => {
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
) -> AppResult<Option<AgentPrincipalEscalation>> {
    let now = DateTime::now();
    let decision_bson = mongodb::bson::to_bson(decision)?;
    let mut set = doc! {
        "status": PRINCIPAL_ESCALATION_STATUS_RESOLVED,
        "decision": decision_bson,
        "updated_at": now,
        "resolved_at": now,
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


/// 反查：在**入站消息自身所属 workspace** 内，from_wxid 是否是某 domain 的 principal_decider。
/// 返回 Some(domain) 表示该 wxid 是本 workspace 的领导。
/// 🔒 关键：必须用入站消息自己的 workspace_id 约束查询——否则 A workspace 的领导 wxid
/// 若恰好也是 B workspace 某业务号的好友，B 收到他消息时会被误路由进 A 的请示流（跨域串扰）。
pub(crate) async fn lookup_principal_config(
    state: &AppState,
    workspace_id: &str,
    from_wxid: &str,
) -> AppResult<Option<String>> {
    let cfg = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "principal_decider": from_wxid,
                "current_version": true,
            },
            None,
        )
        .await?;
    Ok(cfg.map(|c| c.domain))
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
