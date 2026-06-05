//! 决策请示通道（Principal Decision Channel）。
//!
//! 运营 Agent 撞"决策墙"（超职权 / 高风险件 / 多轮卡死）时，向幕后真人决策源
//! 请示，拿到裁决后用 AI 口吻向客户转述。客户永远只跟 Agent 对话——真人是
//! 幕后决策源，绝不直接面对客户。这不是真人下场：AI 向内部决策源请示，转述仍由 AI 完成。

use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, OperationKnowledgeChunk, PrincipalDecision,
    ALLOWED_ESCALATION_CATEGORY, PRINCIPAL_ESCALATION_STATUS_PENDING,
    PRINCIPAL_ESCALATION_STATUS_RESOLVED,
};
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};

/// 短码字符集：base32 去掉易混字符（0/O/1/I/L），便于真人在微信里识读。
const SHORT_CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const SHORT_CODE_BODY_LEN: usize = 4;

/// 由一个 0..=u32::MAX 的种子生成短码，形如 "E1A2"（E 前缀 + 4 位 base32）。
/// 纯函数、确定性，便于单测；运行时种子由台账插入侧用计数/时间派生（见 Task 11 insert_pending_escalation 的碰撞重试）。
pub(crate) fn short_code_from_seed(seed: u32) -> String {
    let alpha_len = SHORT_CODE_ALPHABET.len() as u32;
    let mut n = seed;
    let mut body = [0u8; SHORT_CODE_BODY_LEN];
    for slot in body.iter_mut() {
        *slot = SHORT_CODE_ALPHABET[(n % alpha_len) as usize];
        n /= alpha_len;
    }
    let body_str = String::from_utf8(body.to_vec()).expect("alphabet is ASCII");
    format!("E{body_str}")
}

/// 真人回复 → 台账匹配结果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReplyMatch {
    /// 命中唯一一条 pending（带码精确，或不带码但只有一条未决）。
    Matched(String), // short_code
    /// 该真人有 ≥2 条未决且回复不带可识别短码 → 需反问澄清。
    Ambiguous(Vec<String>), // 候选 short_codes
    /// 没有任何未决 → 不当客户决策回流（落"待 admin 确认的真人主动指令"）。
    NoPending,
}

/// 从真人回复文本里抽取短码（弱匹配：忽略大小写，允许带/不带 # 与 E 前缀）。
/// 命中返回规范化短码（大写、含 E 前缀，不含 #）。
pub(crate) fn extract_short_code(reply: &str, pending_codes: &[String]) -> Option<String> {
    let upper = reply.to_uppercase();
    pending_codes
        .iter()
        .find(|code| {
            let c = code.to_uppercase();
            upper.contains(&c) || upper.contains(&format!("#{c}"))
        })
        .cloned()
}

/// 业务决策 #4：根据该真人当前所有 pending 台账 + 回复文本，决定匹配哪一条。
pub(crate) fn match_principal_reply(reply: &str, pending: &[AgentPrincipalEscalation]) -> ReplyMatch {
    let codes: Vec<String> = pending.iter().map(|e| e.short_code.clone()).collect();
    if codes.is_empty() {
        return ReplyMatch::NoPending;
    }
    if let Some(code) = extract_short_code(reply, &codes) {
        return ReplyMatch::Matched(code);
    }
    if codes.len() == 1 {
        return ReplyMatch::Matched(codes[0].clone());
    }
    ReplyMatch::Ambiguous(codes)
}

/// 渲染推给领导的请示卡（结构化、不脱敏）。短码放在最前便于领导引用。
pub(crate) fn render_principal_card(
    short_code: &str,
    customer_label: &str,
    reason: &str,
    question_for_principal: &str,
) -> String {
    format!(
        "【请示 #{short_code}】客户「{customer_label}」\n卡点：{reason}\n请示：{question_for_principal}"
    )
}

/// 安抚占位的确定性兜底文案。统一占位模型下，占位是 decision Agent 本轮 reply_text 经
/// outbox 正常发出；本函数仅作回落参考（LLM 未给合适占位 / 降级场景），不由网关直接发送。
/// 红线：绝不提转接类措辞，只说"帮你确认一下"这类 AI 自然话术。
pub(crate) fn fallback_holding_reply() -> &'static str {
    "这个我帮你确认一下，稍等我给你准信。"
}

/// 该条已 resolved 的授权当前是否仍可用于转述。
/// expires=None 视为不过期（如纯拒绝类裁决无时效）。
pub(crate) fn authorization_is_usable(
    expires_at: Option<mongodb::bson::DateTime>,
    now: mongodb::bson::DateTime,
) -> bool {
    match expires_at {
        None => true,
        Some(exp) => now.timestamp_millis() < exp.timestamp_millis(),
    }
}

/// 转述前选用的事实源：授权有效用真人 substance；过期则回落"不再可用"信号。
pub(crate) fn relay_substance_if_usable<'a>(
    decision: &'a PrincipalDecision,
    expires_at: Option<mongodb::bson::DateTime>,
    now: mongodb::bson::DateTime,
) -> Option<&'a str> {
    if authorization_is_usable(expires_at, now) {
        Some(&decision.substance)
    } else {
        None
    }
}

/// 高风险件升级模式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HighRiskEscalationMode {
    /// 所有被静默 hold 的高风险件都请示真人。
    All,
    /// 只升级实质需决策/授权的件（默认，保守）。
    DecisionOnly,
}

/// 从 workspace 配置字符串解析升级模式；未配/未知值回落 DecisionOnly（保守默认）。
pub(crate) fn parse_high_risk_mode(raw: Option<&str>) -> HighRiskEscalationMode {
    match raw {
        Some("all") => HighRiskEscalationMode::All,
        _ => HighRiskEscalationMode::DecisionOnly,
    }
}

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

/// 二次防护：目标 wxid 必须严格等于该 workspace 配置的 principal_decider。
/// 用于推请示卡前，杜绝把内部请示卡误发给客户。
pub(crate) fn assert_target_is_principal(
    target_wxid: &str,
    configured_principal: &str,
) -> AppResult<()> {
    if target_wxid == configured_principal {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "请示卡目标 wxid 与配置的领导不符，拒发（target={target_wxid}）"
        )))
    }
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

/// 判断 mongodb 错误是否为唯一键冲突（短码碰撞）。
fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(
        *e.kind,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(ref we))
            if we.code == 11000
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_code_has_e_prefix_and_fixed_len() {
        let code = short_code_from_seed(0);
        assert!(code.starts_with('E'));
        assert_eq!(code.len(), 1 + SHORT_CODE_BODY_LEN);
    }

    #[test]
    fn short_code_uses_unambiguous_alphabet_only() {
        let code = short_code_from_seed(123_456);
        for ch in code.chars().skip(1) {
            assert!(
                SHORT_CODE_ALPHABET.contains(&(ch as u8)),
                "char {ch} must be in unambiguous alphabet"
            );
        }
        for bad in ['0', 'O', '1', 'I', 'L'] {
            assert!(!code[1..].contains(bad), "code body must not contain {bad}");
        }
    }

    #[test]
    fn short_code_is_deterministic() {
        assert_eq!(short_code_from_seed(42), short_code_from_seed(42));
    }

    #[test]
    fn short_code_differs_for_different_seeds() {
        assert_ne!(short_code_from_seed(1), short_code_from_seed(2));
    }

    fn make_pending(short_code: &str) -> AgentPrincipalEscalation {
        use crate::models::PRINCIPAL_ESCALATION_STATUS_PENDING;
        AgentPrincipalEscalation {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            contact_wxid: "cust1".into(),
            short_code: short_code.into(),
            status: PRINCIPAL_ESCALATION_STATUS_PENDING.into(),
            category: "out_of_scope_decision".into(),
            reason: "r".into(),
            question_for_principal: "q".into(),
            principal_wxid: "boss".into(),
            decision: None,
            authorization_expires_at: None,
            is_generalizable: false,
            knowledge_proposal_emitted: false,
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
            resolved_at: None,
        }
    }

    #[test]
    fn match_with_explicit_code_hits_that_entry() {
        let pending = vec![make_pending("E1A2"), make_pending("E3B4")];
        assert_eq!(
            match_principal_reply("就按 #E3B4 来吧，可以", &pending),
            ReplyMatch::Matched("E3B4".into())
        );
    }

    #[test]
    fn match_without_code_single_pending_falls_back_to_it() {
        let pending = vec![make_pending("E1A2")];
        assert_eq!(
            match_principal_reply("行，可以给", &pending),
            ReplyMatch::Matched("E1A2".into())
        );
    }

    #[test]
    fn match_without_code_multiple_pending_is_ambiguous() {
        let pending = vec![make_pending("E1A2"), make_pending("E3B4")];
        match match_principal_reply("可以", &pending) {
            ReplyMatch::Ambiguous(codes) => {
                assert_eq!(codes.len(), 2);
                assert!(codes.contains(&"E1A2".to_string()));
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn match_no_pending_returns_no_pending() {
        assert_eq!(match_principal_reply("以后都按 8 折", &[]), ReplyMatch::NoPending);
    }

    #[test]
    fn extract_short_code_is_case_insensitive() {
        let codes = vec!["E1A2".to_string()];
        assert_eq!(extract_short_code("回复 e1a2 同意", &codes), Some("E1A2".into()));
    }

    #[test]
    fn principal_card_puts_code_first_and_is_not_redacted() {
        let card = render_principal_card("E1A2", "张三(老客户)", "超出标准 9 折权限", "是否同意 8 折？");
        assert!(card.starts_with("【请示 #E1A2】"));
        assert!(card.contains("张三(老客户)")); // 对领导不脱敏
        assert!(card.contains("是否同意 8 折？"));
    }

    #[test]
    fn authorization_none_expiry_is_usable() {
        assert!(authorization_is_usable(None, mongodb::bson::DateTime::now()));
    }

    #[test]
    fn authorization_future_expiry_is_usable() {
        let now = mongodb::bson::DateTime::from_millis(1_000);
        let future = mongodb::bson::DateTime::from_millis(2_000);
        assert!(authorization_is_usable(Some(future), now));
    }

    #[test]
    fn authorization_past_expiry_is_not_usable() {
        let now = mongodb::bson::DateTime::from_millis(2_000);
        let past = mongodb::bson::DateTime::from_millis(1_000);
        assert!(!authorization_is_usable(Some(past), now));
    }

    #[test]
    fn relay_substance_none_when_expired() {
        let decision = PrincipalDecision {
            verdict: "conditional".into(),
            substance: "可以 8 折".into(),
            constraints: vec!["本周付款".into()],
            authorization_window_hours: None,
        };
        let now = mongodb::bson::DateTime::from_millis(2_000);
        let past = mongodb::bson::DateTime::from_millis(1_000);
        assert_eq!(relay_substance_if_usable(&decision, Some(past), now), None);
        let future = mongodb::bson::DateTime::from_millis(3_000);
        assert_eq!(
            relay_substance_if_usable(&decision, Some(future), now),
            Some("可以 8 折")
        );
    }

    #[test]
    fn high_risk_mode_parses_all() {
        assert_eq!(parse_high_risk_mode(Some("all")), HighRiskEscalationMode::All);
    }

    #[test]
    fn high_risk_mode_defaults_to_decision_only() {
        assert_eq!(parse_high_risk_mode(None), HighRiskEscalationMode::DecisionOnly);
        assert_eq!(parse_high_risk_mode(Some("garbage")), HighRiskEscalationMode::DecisionOnly);
        assert_eq!(
            parse_high_risk_mode(Some("decision_only")),
            HighRiskEscalationMode::DecisionOnly
        );
    }

    #[test]
    fn assert_target_is_principal_accepts_match() {
        assert!(assert_target_is_principal("boss_wxid", "boss_wxid").is_ok());
    }

    #[test]
    fn assert_target_is_principal_rejects_customer() {
        assert!(assert_target_is_principal("customer_wxid", "boss_wxid").is_err());
    }
}
