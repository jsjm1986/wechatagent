//! 决策请示通道——纯函数层（短码 / 匹配 / 授权 / 信号 / 卡死判定 / 出站守卫 / verdict 校验）。
//! 无 I/O、无 async、无 db/mcp/state 依赖，全部可单测（见文件末 mod tests）。

use crate::agent::types::AgentTrigger;
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, Contact, OperationDomainConfig, PrincipalDecision,
    ALLOWED_PRINCIPAL_VERDICT, AWAITING_PRINCIPAL_DECISION_ATTR, PRINCIPAL_VERDICT_DEFERRED,
};

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
/// `pub`（而非 `pub(crate)`）：供 tests/principal_decision_channel.rs 的 §14.9b
/// 红线纯函数测试在 crate 外断言该兜底文案不含任何转接类措辞。
pub fn fallback_holding_reply() -> &'static str {
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

/// 高风险件升级模式。decision Agent 据此判断被风险闸门拦下的件是否要请示领导，
/// hold→升级路径(`escalate_held_decision`)据此决定 ai_policy 类是否升级。
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

/// 二次防护：目标 wxid 必须严格等于该 workspace 配置的 principal_decider。
/// 用于推请示卡前，杜绝把内部请示卡误发给客户。
///
/// 当前所有请示卡发送路径（`trigger_principal_escalation` / `escalate_held_decision`）
/// 的目标 wxid 都直接取自 `principal_decider_wxid()` 这一权威配置查询，故无需再调本守卫
/// （调了也是同源恒真）。保留本函数作为「目标 vs 配置」不同源场景的防御 API + 其不变量单测；
/// `#[allow(dead_code)]` 标注当前无生产调用点。
#[allow(dead_code)]
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

/// 该 trigger 是否是 relay 转述（领导裁决回送客户）。
/// relay 走合成 Inbound，content 以哨兵 `PRINCIPAL_RELAY_SENTINEL` 开头——
/// 合成消息仅由 `ConversationMessage::synthetic_principal_relay` 构造、逐字以哨兵开头；
/// 真实客户消息经 prompt_isolation 隔离，不会以 `__PRINCIPAL_RELAY__` 开头。
/// 网关据此对 relay 豁免频控类 precheck（领导回复是客户期待内的被动应答，不该被
/// rate_limited/cooldown/daily_limit 拦掉——否则领导裁决永远送不到客户）。
pub(crate) fn is_principal_relay_trigger(trigger: &AgentTrigger<'_>) -> bool {
    matches!(
        trigger,
        AgentTrigger::Inbound(m) if m.content.starts_with(crate::models::PRINCIPAL_RELAY_SENTINEL)
    )
}

/// relay 出站红线守卫：检测一条**拟发给客户**的转述文本是否泄漏了内部 relay 载荷。
///
/// relay 转述要求 decision Agent 把领导裁决用 AI 口吻重组、**绝不**把合成消息里的
/// 哨兵 `__PRINCIPAL_RELAY__` 或 `verdict=`/`substance=`/`constraints=` 等内部字段标记
/// 透传给客户（见 prompts.rs relay 转述模式契约 + synthetic_principal_relay 载荷格式）。
/// 该约束此前**只**靠 prompt 约束，无代码级兜底；本函数是出站方向的代码守卫——
/// 网关在 relay run 入 outbox 前调用，命中即 fail-closed（不发泄漏文本），与解读侧
/// `sanitize_verdict` 的代码级兜底对称。纯函数，便于单测。
pub(crate) fn relay_output_leaks_internal_payload(reply_text: &str) -> bool {
    reply_text.contains(crate::models::PRINCIPAL_RELAY_SENTINEL)
        || reply_text.contains("verdict=")
        || reply_text.contains("substance=")
        || reply_text.contains("constraints=")
}

/// 从意图轨迹尾部数"连续未推进"轮数：未推进 = 相邻条目 `intent` 相同（含都为空串）。
/// 例：轨迹 [A,B,B,B] → 末三条 intent 相同 → 返回 3。空轨迹返回 0。
fn consecutive_unprogressed_turns(trajectory: &[crate::models::IntentTrajectoryEntry]) -> u32 {
    let Some(last) = trajectory.last() else {
        return 0;
    };
    let mut count = 0u32;
    for entry in trajectory.iter().rev() {
        if entry.intent == last.intent {
            count += 1;
        } else {
            break;
        }
    }
    count
}

/// 组装注入 decision prompt 的"请示通道信号"段（纯函数，无 IO）。三信号：
/// ①等待领导决策中（domain_attributes 布尔标记）②多轮卡死（意图轨迹连续未推进+末轮负面）
/// ③高风险升级模式（workspace 配置）。三信号全缺返回空串（decision.rs 据此决定是否拼接）。
pub(crate) fn build_decision_signals_text(
    contact: &Contact,
    domain_config: Option<&OperationDomainConfig>,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    // ① 等待领导决策中：该客户有一条 pending 请示，正在等领导回话。
    let awaiting = contact
        .domain_attributes
        .as_ref()
        .and_then(|d| d.get_bool(AWAITING_PRINCIPAL_DECISION_ATTR).ok())
        .unwrap_or(false);
    if awaiting {
        lines.push(
            "- 该客户当前有一条议题已向领导请示、正在等待裁决：勿就同一越权点反复请示，也不要替领导拍板；客户这条消息里非越权、你能自主答的部分照常自然回复。".to_string(),
        );
    }

    // ② 多轮卡死：同一议题连续未推进 + 最近一轮负面反应（两条件 AND）。
    let turns = consecutive_unprogressed_turns(&contact.intent_trajectory);
    let latest_negative = contact
        .intent_trajectory
        .last()
        .map(|e| crate::agent::reaction::is_negative_outcome(&e.intent))
        .unwrap_or(false);
    if is_stuck_or_undelivered(turns, DEFAULT_STUCK_THRESHOLD, latest_negative) {
        lines.push(
            "- 该议题已连续多轮未推进且客户有负面反应：避免硬推同一话术，考虑换个角度，或如实告诉客户你需要向领导确认后再答复。".to_string(),
        );
    }

    // ③ 高风险升级模式：仅 All 模式需要提示 decision Agent 主动 emit escalation。
    let mode =
        parse_high_risk_mode(domain_config.and_then(|c| c.high_risk_escalation_mode.as_deref()));
    if mode == HighRiskEscalationMode::All {
        lines.push(
            "- 本工作区为全量升级模式：凡触及未验证产品声明或被风险闸门拦下的高风险件，都应输出 escalationRequest 请示领导，不要自行硬答。".to_string(),
        );
    }

    lines.join("\n")
}

/// 被风险闸门 hold 的件是否要升级请示领导（纯函数，业务判定，便于单测）。
/// `blocked_status` 是网关 hold 分支算出的终态字面量（见 review::GatewayStatusFinal::gateway_status_str）。
/// 取舍（已拍板）：安全门/未验证产品声明无条件升级（这两类是"不敢答"的硬风险，领导必须知道）；
/// ai_policy 仅 All 模式升级（保守默认 DecisionOnly 下，策略性暂缓不打扰领导）；
/// 等待更多上下文 / 必填缺失 / 预算超额 / context_changed 一律不升级（不是决策墙，是 AI 自身可恢复的状态）。
pub(crate) fn should_escalate_held(blocked_status: &str, mode: HighRiskEscalationMode) -> bool {
    match blocked_status {
        s if s == crate::agent::types::HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD => true,
        "blocked_unverified_product_claim" => true,
        s if s == crate::agent::types::HOLD_CATEGORY_HELD_BY_AI_POLICY => {
            mode == HighRiskEscalationMode::All
        }
        _ => false,
    }
}

/// 判断 mongodb 错误是否为唯一键冲突（短码碰撞）。
pub(crate) fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(
        *e.kind,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(ref we))
            if we.code == 11000
    )
}

/// 校验 verdict，越界回落 deferred（纯函数，便于单测）。
pub(crate) fn sanitize_verdict(decision: PrincipalDecision) -> PrincipalDecision {
    if ALLOWED_PRINCIPAL_VERDICT.contains(&decision.verdict.as_str()) {
        decision
    } else {
        PrincipalDecision {
            verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
            substance: decision.substance,
            constraints: decision.constraints,
            authorization_window_hours: decision.authorization_window_hours,
        }
    }
}

/// 两条件同时满足才算卡死。纯函数，输入由 `build_decision_signals_text` 从 contact 取。
pub(crate) fn is_stuck_or_undelivered(
    consecutive_unprogressed_turns: u32,
    threshold: u32,
    latest_reaction_is_negative: bool,
) -> bool {
    consecutive_unprogressed_turns >= threshold && latest_reaction_is_negative
}

/// 默认卡死轮阈值（spec：默认 3，可配）。
pub(crate) const DEFAULT_STUCK_THRESHOLD: u32 = 3;

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

    #[test]
    fn sanitize_verdict_keeps_valid() {
        let d = PrincipalDecision {
            verdict: "approved".into(),
            substance: "ok".into(),
            constraints: vec![],
            authorization_window_hours: None,
        };
        assert_eq!(sanitize_verdict(d).verdict, "approved");
    }

    #[test]
    fn sanitize_verdict_falls_back_on_garbage() {
        let d = PrincipalDecision {
            verdict: "maybe_lol".into(),
            substance: "x".into(),
            constraints: vec![],
            authorization_window_hours: Some(24.0),
        };
        let out = sanitize_verdict(d);
        assert_eq!(out.verdict, "deferred");
        assert_eq!(out.substance, "x");
        assert_eq!(out.authorization_window_hours, Some(24.0));
    }

    #[test]
    fn stuck_needs_both_conditions() {
        // 轮数够但无负面反应 → 不触发
        assert!(!is_stuck_or_undelivered(5, 3, false));
        // 有负面反应但轮数不够 → 不触发
        assert!(!is_stuck_or_undelivered(2, 3, true));
        // 两者都满足 → 触发
        assert!(is_stuck_or_undelivered(3, 3, true));
        assert!(is_stuck_or_undelivered(4, 3, true));
    }

    #[test]
    fn default_stuck_threshold_is_three() {
        assert_eq!(DEFAULT_STUCK_THRESHOLD, 3);
    }

    fn make_contact(wxid: &str) -> crate::models::Contact {
        use crate::models::{AgentStatus, Contact};
        let now = mongodb::bson::DateTime::now();
        Contact {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            wxid: wxid.into(),
            nickname: None,
            remark: None,
            alias: None,
            agent_status: AgentStatus::Managed,
            human_profile_note: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            tags: Vec::new(),
            domain_attributes: None,
            domain_attributes_updated_at: None,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: mongodb::bson::Document::new(),
            profile_attributes: mongodb::bson::Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn relay_trigger_detected_for_synthetic_relay() {
        let contact = make_contact("cust1");
        let msg = crate::models::ConversationMessage::synthetic_principal_relay(
            &contact,
            "approved",
            "可以给 8 折",
            &[],
        );
        assert!(is_principal_relay_trigger(&AgentTrigger::Inbound(&msg)));
    }

    #[test]
    fn relay_trigger_not_detected_for_normal_inbound() {
        let contact = make_contact("cust1");
        let mut msg = crate::models::ConversationMessage::synthetic_principal_relay(
            &contact, "approved", "x", &[],
        );
        // 普通客户消息：内容不以哨兵开头。
        msg.content = "老板能不能再便宜点".into();
        assert!(!is_principal_relay_trigger(&AgentTrigger::Inbound(&msg)));
    }

    #[test]
    fn relay_output_guard_passes_clean_ai_voice_transcription() {
        // 正常转述：AI 口吻重组，不含任何内部字段标记 → 不应误判泄漏。
        assert!(!relay_output_leaks_internal_payload(
            "跟领导申请下来啦，可以给你 8 折，不过得麻烦你这周内完成付款，可以吗？"
        ));
        assert!(!relay_output_leaks_internal_payload(""));
        // "条件"等中文词不含 "constraints=" 字面 → 不误判。
        assert!(!relay_output_leaks_internal_payload(
            "这个折扣是有条件的：本周付款。verdict 这种说法不会单独出现"
        ));
    }

    #[test]
    fn relay_output_guard_catches_sentinel_leak() {
        // LLM 失误把哨兵透传 → 命中。
        assert!(relay_output_leaks_internal_payload(
            "__PRINCIPAL_RELAY__\nverdict=approved\nsubstance=可以给8折"
        ));
        assert!(relay_output_leaks_internal_payload(
            "好的，__PRINCIPAL_RELAY__ 领导说可以"
        ));
    }

    #[test]
    fn relay_output_guard_catches_field_marker_leak() {
        // 即使没透传哨兵，但漏了任一内部字段标记也命中。
        assert!(relay_output_leaks_internal_payload("verdict=approved 可以给你"));
        assert!(relay_output_leaks_internal_payload("领导说 substance=8折优惠"));
        assert!(relay_output_leaks_internal_payload("constraints=本周付款"));
    }

    fn traj_entry(intent: &str) -> crate::models::IntentTrajectoryEntry {
        crate::models::IntentTrajectoryEntry {
            turn_index: 0,
            intent: intent.into(),
            objection_type: None,
            recorded_at: mongodb::bson::DateTime::now(),
        }
    }

    #[test]
    fn signals_empty_when_no_signal_present() {
        let contact = make_contact("cust1");
        assert!(build_decision_signals_text(&contact, None).is_empty());
    }

    #[test]
    fn signals_emit_awaiting_when_marker_set() {
        let mut contact = make_contact("cust1");
        let mut attrs = mongodb::bson::Document::new();
        attrs.insert(AWAITING_PRINCIPAL_DECISION_ATTR, true);
        contact.domain_attributes = Some(attrs);
        let text = build_decision_signals_text(&contact, None);
        assert!(text.contains("正在等待裁决"), "应出等待领导信号，实际：{text}");
    }

    #[test]
    fn signals_emit_stuck_on_three_same_intent_plus_negative() {
        let mut contact = make_contact("cust1");
        // 连续 3 轮同一负面 intent → 卡死两条件同时满足。
        contact.intent_trajectory = vec![
            traj_entry("user_replied_objection"),
            traj_entry("user_replied_objection"),
            traj_entry("user_replied_objection"),
        ];
        let text = build_decision_signals_text(&contact, None);
        assert!(text.contains("连续多轮未推进"), "应出卡死信号，实际：{text}");
    }

    #[test]
    fn signals_no_stuck_below_threshold() {
        let mut contact = make_contact("cust1");
        // 仅 2 轮同 intent，未达阈值 3。
        contact.intent_trajectory = vec![
            traj_entry("user_replied_objection"),
            traj_entry("user_replied_objection"),
        ];
        assert!(!build_decision_signals_text(&contact, None).contains("连续多轮未推进"));
    }

    #[test]
    fn signals_no_stuck_when_latest_not_negative() {
        let mut contact = make_contact("cust1");
        // 连续 3 轮但末轮非负面 intent → 不触发。
        contact.intent_trajectory = vec![
            traj_entry("user_replied_positive"),
            traj_entry("user_replied_positive"),
            traj_entry("user_replied_positive"),
        ];
        assert!(!build_decision_signals_text(&contact, None).contains("连续多轮未推进"));
    }

    #[test]
    fn consecutive_unprogressed_counts_tail_run() {
        let traj = vec![
            traj_entry("a"),
            traj_entry("b"),
            traj_entry("b"),
            traj_entry("b"),
        ];
        assert_eq!(consecutive_unprogressed_turns(&traj), 3);
        assert_eq!(consecutive_unprogressed_turns(&[]), 0);
    }

    #[test]
    fn should_escalate_held_safety_guard_unconditional() {
        use crate::agent::types::HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD;
        // 安全门：两种模式都升级。
        assert!(should_escalate_held(
            HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD,
            HighRiskEscalationMode::DecisionOnly
        ));
        assert!(should_escalate_held(
            HOLD_CATEGORY_BLOCKED_BY_SAFETY_GUARD,
            HighRiskEscalationMode::All
        ));
    }

    #[test]
    fn should_escalate_held_unverified_product_unconditional() {
        // 未验证产品声明：两种模式都升级。
        assert!(should_escalate_held(
            "blocked_unverified_product_claim",
            HighRiskEscalationMode::DecisionOnly
        ));
        assert!(should_escalate_held(
            "blocked_unverified_product_claim",
            HighRiskEscalationMode::All
        ));
    }

    #[test]
    fn should_escalate_held_ai_policy_only_in_all_mode() {
        use crate::agent::types::HOLD_CATEGORY_HELD_BY_AI_POLICY;
        // 策略性暂缓：仅 All 模式升级，保守 DecisionOnly 不打扰领导。
        assert!(should_escalate_held(
            HOLD_CATEGORY_HELD_BY_AI_POLICY,
            HighRiskEscalationMode::All
        ));
        assert!(!should_escalate_held(
            HOLD_CATEGORY_HELD_BY_AI_POLICY,
            HighRiskEscalationMode::DecisionOnly
        ));
    }

    #[test]
    fn should_escalate_held_waiting_context_never() {
        use crate::agent::types::HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT;
        // 等待更多上下文：不是决策墙，是 AI 自身可恢复状态，永不升级。
        assert!(!should_escalate_held(
            HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT,
            HighRiskEscalationMode::All
        ));
        assert!(!should_escalate_held(
            HOLD_CATEGORY_AI_WAITING_FOR_MORE_CONTEXT,
            HighRiskEscalationMode::DecisionOnly
        ));
    }

    #[test]
    fn should_escalate_held_other_terminal_states_never() {
        // 必填缺失 / 预算超额 / context_changed：均非决策墙，不升级。
        for s in [
            "blocked_by_required_field",
            "blocked_by_budget",
            "context_changed",
        ] {
            assert!(!should_escalate_held(s, HighRiskEscalationMode::All), "{s} 不应升级");
            assert!(
                !should_escalate_held(s, HighRiskEscalationMode::DecisionOnly),
                "{s} 不应升级"
            );
        }
    }
}
