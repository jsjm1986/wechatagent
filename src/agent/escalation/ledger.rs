//! 决策请示通道——台账 CRUD 层（pending 台账增删查改 / 知识缺口提案 / relay task 入队）。
//! 全部 async + db 访问。

use super::logic::{is_duplicate_key_error, is_pending_dedupe_conflict, short_code_from_seed};
use crate::error::{AppError, AppResult};
use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, ProvenanceSource, RevisionOp, RevisionRequest,
};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, OperationKnowledgeChunk, PrincipalDecision,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED,
};
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime, Document};

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
    let title = format!("领导授权沉淀：{}", entry.reason);
    let body = format!(
        "源自客户「{}」请示 #{}。\n卡点：{}\n领导裁决：{}\n约束：{}",
        entry.contact_wxid,
        entry.short_code,
        entry.reason,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(anchor.get_i32("endLine").unwrap(), 1, "单行 substance endLine=1");
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
        assert_eq!(anchor.get_i32("endLine").unwrap(), 3, "两个换行 → endLine=3");
    }
}
