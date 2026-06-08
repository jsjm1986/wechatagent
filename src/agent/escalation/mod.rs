//! 决策请示通道（Principal Decision Channel）。
//!
//! 运营 Agent 撞"决策墙"（超职权 / 高风险件 / 多轮卡死）时，向幕后真人决策源
//! 请示，拿到裁决后用 AI 口吻向客户转述。客户永远只跟 Agent 对话——真人是
//! 幕后决策源，绝不直接面对客户。这不是真人下场：AI 向内部决策源请示，转述仍由 AI 完成。

mod ledger;
mod logic;

pub(crate) use ledger::*;
pub(crate) use logic::*;
// fallback_holding_reply 需 crate 外可见（tests/principal_decision_channel.rs §14.9b
// 红线测试在 crate 外断言兜底文案不含转接类措辞）；pub(crate) use logic::* 会把它降级，
// 故单独 pub re-export 还原其原始 `pub` 可见性。
pub use logic::fallback_holding_reply;

use super::generate_agent_json;
use super::types::{AgentDecision, DecisionReviewResult};
use crate::error::{AppError, AppResult};
use crate::models::{
    AgentPrincipalEscalation, AgentTask, Contact, OperationDomainConfig, PrincipalDecision,
    AWAITING_PRINCIPAL_DECISION_ATTR, ESCALATION_CATEGORY_HIGH_RISK_GATED,
    PRINCIPAL_VERDICT_DEFERRED,
};
use crate::mcp;
use crate::prompts;
use crate::routes::AppState;
use mongodb::bson::{doc, DateTime};

/// hold→升级请示：被风险闸门拦下的高风险件，按 workspace 升级模式请示领导并补发安全占位。
///
/// 与 `trigger_principal_escalation` 的区别：后者用于 approved 路径（占位已由 outbox 发出，
/// 本函数只推卡+落台账）；hold 路径无 outbox、客户尚未收到任何回复，故本函数额外**补发安全占位**
/// 安抚客户（体验与 approved 一致），并直接写 awaiting 标记（hold 路径不走 apply_agent_updates）。
///
/// 红线：占位走 `fallback_holding_reply()`（不含任何转接类措辞），客户始终只跟 AI 对话；
/// 真人仅作幕后决策源。调用方对本函数错误只记 warn、不阻断 run、不改终态。
pub(crate) async fn escalate_held_decision(
    state: &AppState,
    contact: &Contact,
    review: &DecisionReviewResult,
    final_decision: &AgentDecision,
    domain_config: Option<&OperationDomainConfig>,
    blocked_status: &str,
) -> AppResult<()> {
    let mode =
        parse_high_risk_mode(domain_config.and_then(|c| c.high_risk_escalation_mode.as_deref()));
    if !should_escalate_held(blocked_status, mode) {
        return Ok(());
    }
    let Some(principal_wxid) =
        principal_decider_wxid(state, &contact.workspace_id, super::domain::USER_OPS_DOMAIN_ID)
            .await?
    else {
        return Ok(()); // 未配置领导 = 本 workspace 未启用请示通道
    };
    if principal_wxid == contact.wxid {
        return Err(AppError::BadRequest(
            "principal_decider 配置等于客户 wxid，拒绝触发请示".into(),
        ));
    }
    // 去重：同客户同类别已有 pending → 不重复推卡骚扰领导。
    if has_pending_for_contact(
        state,
        &contact.workspace_id,
        &contact.wxid,
        ESCALATION_CATEGORY_HIGH_RISK_GATED,
    )
    .await?
    {
        return Ok(());
    }
    let reason = if !review.hold_reason.trim().is_empty() {
        review.hold_reason.clone()
    } else {
        review.review_summary.clone()
    };
    let question = format!(
        "该客户议题触发高风险闸门（{}），AI 暂不自行答复。拟答风险等级：{}。请领导定夺该如何回复。",
        blocked_status, final_decision.risk_level
    );
    let entry = insert_pending_escalation(
        state,
        &contact.workspace_id,
        &contact.account_id,
        &contact.wxid,
        ESCALATION_CATEGORY_HIGH_RISK_GATED,
        &reason,
        &question,
        &principal_wxid,
        false, // 高风险硬闸件默认不泛化（领导裁决可能是个案）
    )
    .await?;
    let customer_label = contact
        .remark
        .clone()
        .or_else(|| contact.nickname.clone())
        .or_else(|| contact.alias.clone())
        .unwrap_or_else(|| contact.wxid.clone());
    let card = render_principal_card(&entry.short_code, &customer_label, &reason, &question);
    mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_text",
        serde_json::json!({ "recipient": principal_wxid, "content": card }),
    )
    .await?;
    // 补发安全占位安抚客户（hold 路径无 outbox，直发；体验与 approved 占位一致）。
    mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_text",
        serde_json::json!({ "recipient": &contact.wxid, "content": fallback_holding_reply() }),
    )
    .await?;
    // 写 awaiting 标记（hold 路径不走 apply_agent_updates，需单独写），
    // 否则下一轮 build_decision_signals_text 读不到等待信号。用 dotted key $set，不覆盖其它 domain_attributes。
    let set_key = format!("domain_attributes.{}", AWAITING_PRINCIPAL_DECISION_ATTR);
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "wxid": &contact.wxid,
            },
            doc! { "$set": { set_key: true, "domain_attributes_updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(())
}

/// 处理 principal_decision_relay task：领导已裁决，把决策用 AI 口吻转述给客户。
pub(crate) async fn handle_principal_decision_relay(
    state: &AppState,
    task: &AgentTask,
) -> AppResult<()> {
    let short_code = task.content.trim();
    let entry = state
        .db
        .agent_principal_escalations()
        .find_one(doc! { "short_code": short_code }, None)
        .await?;
    let Some(entry) = entry else {
        return Ok(());
    };
    let Some(decision) = entry.decision.clone() else {
        return Ok(());
    };

    let now = mongodb::bson::DateTime::now();
    if relay_substance_if_usable(&decision, entry.authorization_expires_at, now).is_none() {
        // 授权过期：不拿过期授权乱承诺，结束。
        return Ok(());
    }

    let contact = state
        .db
        .contacts()
        .find_one(
            doc! {
                "workspace_id": &entry.workspace_id,
                "account_id": &entry.account_id,
                "wxid": &entry.contact_wxid
            },
            None,
        )
        .await?;
    let Some(contact) = contact else {
        return Ok(());
    };

    crate::agent::gateway::relay_principal_decision_to_customer(state, contact, &entry, &decision, task.id)
        .await
}

/// 用 LLM 把真人自然语言回复解读成结构化裁决。绝不原话转发给客户。
/// 解析失败或 verdict 越界时回落 deferred（保守：宁可当"领导还没定"也不乱转述）。
pub(crate) async fn interpret_principal_reply(
    state: &AppState,
    account_id: &str,
    escalation: &AgentPrincipalEscalation,
    principal_reply_text: &str,
) -> AppResult<PrincipalDecision> {
    let user = format!(
        "客户请示问题：{}\n领导回复原话：{}",
        escalation.question_for_principal, principal_reply_text
    );
    let system = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "escalation.principal.interpret",
    )
    .await?;
    let value = generate_agent_json(
        state,
        Some(account_id),
        Some(&escalation.contact_wxid),
        None,
        "escalation.principal.interpret",
        &system,
        &user,
    )
    .await?;
    let decision: PrincipalDecision = match serde_json::from_value(value) {
        Ok(d) => d,
        Err(_) => {
            return Ok(PrincipalDecision {
                verdict: PRINCIPAL_VERDICT_DEFERRED.to_string(),
                substance: String::new(),
                constraints: vec![],
                authorization_window_hours: None,
            });
        }
    };
    Ok(sanitize_verdict(decision))
}

/// 处理真人（领导）的微信回复。匹配未决台账→解读→resolve→起 relay task。
/// 业务决策 #4：不带码且多条未决时反问澄清（向领导发一条，不回流客户）。
/// 返回 true 表示已作为领导回复消费（调用方据此不再进客户 agent 链路）。
pub(crate) async fn handle_principal_reply(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    principal_wxid: &str,
    reply_text: &str,
) -> AppResult<bool> {
    let pending = list_pending_for_principal(state, workspace_id, principal_wxid).await?;
    match match_principal_reply(reply_text, &pending) {
        ReplyMatch::NoPending => {
            tracing::info!(
                principal_wxid,
                "领导主动消息但无未决请示，不自动生效（待 admin 确认）"
            );
            Ok(true)
        }
        ReplyMatch::Ambiguous(codes) => {
            let list = codes
                .iter()
                .map(|c| format!("#{c}"))
                .collect::<Vec<_>>()
                .join(" / ");
            let ask = format!(
                "您刚回复的是哪一条？目前挂着这几条：{list}，麻烦带上编号（如 #{}）再回我一次。",
                codes.first().cloned().unwrap_or_default()
            );
            mcp::logged_call_for_account(
                state,
                account_id,
                "message_send_text",
                serde_json::json!({ "recipient": principal_wxid, "content": ask }),
            )
            .await?;
            Ok(true)
        }
        ReplyMatch::Matched(short_code) => {
            let entry = pending
                .iter()
                .find(|e| e.short_code == short_code)
                .cloned()
                .expect("matched code must be in pending");
            let decision = interpret_principal_reply(state, account_id, &entry, reply_text).await?;
            if decision.verdict == crate::models::PRINCIPAL_VERDICT_DEFERRED {
                tracing::info!(short_code = %short_code, "领导暂缓，保持 pending 继续等待");
                return Ok(true);
            }
            // 授权过期时间：领导说了算。LLM 解读出领导明确说的时限→authorization_window_hours；
            // 领导没提→None=不设过期窗。不再硬编码默认窗。
            let expires = decision.authorization_window_hours.and_then(|hours| {
                if hours > 0.0 {
                    Some(DateTime::from_millis(
                        DateTime::now().timestamp_millis() + (hours * 3600.0 * 1000.0) as i64,
                    ))
                } else {
                    None
                }
            });
            let resolved = resolve_escalation(state, &short_code, &decision, expires).await?;
            if resolved.is_none() {
                return Ok(true); // 已被并发 resolve；幂等。
            }
            enqueue_relay_task(state, &entry).await?;
            Ok(true)
        }
    }
}
