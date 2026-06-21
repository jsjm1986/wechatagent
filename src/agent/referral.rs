//! 专属顾问名片引荐：辅助模式判定、候选过滤/渲染（纯函数）、
//! send_outbound_namecard、置「已引荐」态。
use crate::error::{AppError, AppResult};
use crate::models::{ConversationMessage, MessageDirection, ReferralCard};
use crate::routes::AppState;
use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime};
use serde_json::{json, Value};

/// 辅助模式是否对本客户生效。客户级 override > 账号级 enabled > 默认关。
pub(crate) fn assist_mode_active(account_enabled: Option<bool>, override_attr: Option<&str>) -> bool {
    match override_attr {
        Some("force_on") => true,
        Some("force_off") => false,
        _ => account_enabled.unwrap_or(false),
    }
}

/// 发送前准入：仅 enabled 且 approved 的名片可被 AI 选/发。
pub(crate) fn validate_card_sendable(card: &ReferralCard) -> bool {
    card.enabled && card.review_status == "approved"
}

pub(crate) fn filter_referral_candidates<'a>(
    cards: &'a [ReferralCard],
    customer_stage: Option<&str>,
) -> Vec<&'a ReferralCard> {
    cards
        .iter()
        .filter(|c| validate_card_sendable(c))
        .filter(|c| {
            c.target_stages.is_empty()
                || customer_stage
                    .map(|cs| c.target_stages.iter().any(|s| s == cs))
                    .unwrap_or(false)
        })
        .collect()
}

/// 本客户已引荐过的顾问（防重推上下文）。
pub(crate) struct AlreadyReferred {
    pub display_name: String,
    pub card_id: String,
}

pub(crate) fn render_referral_lines(
    candidates: &[&ReferralCard],
    already: Option<&AlreadyReferred>,
) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("可引荐的专属顾问（仅在客户契合触发提示时引荐，没有契合的就不引荐）：\n");
    for c in candidates {
        let id = c.id.map(|i| i.to_hex()).unwrap_or_default();
        let stages = c.target_stages.join(",");
        out.push_str(&format!(
            "- [card:{id}] {} | 阶段:{stages} | 触发提示:{}\n",
            c.display_name, c.send_trigger_hint
        ));
    }
    match already {
        Some(a) => out.push_str(&format!(
            "（本客户引荐历史：已引荐给 {}[card:{}]——除非出现与上次不同的新需求场景，否则不要重复引荐）\n",
            a.display_name, a.card_id
        )),
        None => out.push_str("（本客户引荐历史：尚未引荐）\n"),
    }
    out
}

/// 置「已引荐」态的 $set 子文档（dotted-key，不覆盖其它 domain_attributes）。
pub(crate) fn build_referred_set_doc(card_id: &str, now: DateTime) -> mongodb::bson::Document {
    doc! {
        format!("domain_attributes.{}", crate::models::REFERRED_SPECIALIST_AT_ATTR): now,
        format!("domain_attributes.{}", crate::models::REFERRED_CARD_ID_ATTR): card_id,
        "domain_attributes_updated_at": now,
        "updated_at": now,
    }
}

/// 发送名片给客户。调用方（dispatcher）已确保经 outbox 幂等。
/// 流程：parse + 查名片 → 准入二次校验（防 AI 幻觉/已撤下名片漏到发送）
/// → MCP message_send_namecard → 落出站 `ConversationMessage`（msg_type=namecard）
/// → 置「已引荐」态。
pub(crate) async fn send_outbound_namecard(
    state: &AppState,
    contact: &crate::models::Contact,
    card_id: &str,
) -> AppResult<Value> {
    let oid = ObjectId::parse_str(card_id)
        .map_err(|_| AppError::External("bad card_id".into()))?;
    let card = state
        .db
        .referral_cards()
        .find_one(doc! { "_id": oid }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("referral card not found".into()))?;

    // 发送前准入二次校验（防 AI 幻觉/已撤下名片一路漏到发送）。
    if !validate_card_sendable(&card) {
        return Err(AppError::External(
            "referral card not sendable (draft/disabled)".into(),
        ));
    }

    // ⚠️ MCP message_send_namecard 入参字段名待 server tools/list 确认，此处占位
    let resp = crate::mcp::logged_call_for_account(
        state,
        &contact.account_id,
        "message_send_namecard",
        json!({ "recipient": contact.wxid, "targetWxid": card.target_wxid }),
    )
    .await?;

    // MCP 已成功 = 名片已送达客户，既成事实。此后落库/置态失败**绝不**返 Err——
    // 否则 dispatcher 会 retry 重发，客户收到重复名片（与 send_outbound_media 对称）。
    let message_id = resp
        .get("newMsgId")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let mut raw = to_document(&resp).unwrap_or_default();
    raw.insert("referralCardId", card_id);
    let now = DateTime::now();
    if let Err(err) = state
        .db
        .messages()
        .insert_one(
            ConversationMessage {
                id: None,
                workspace_id: contact.workspace_id.clone(),
                account_id: contact.account_id.clone(),
                contact_wxid: contact.wxid.clone(),
                message_id,
                dedupe_key: None,
                direction: MessageDirection::Outbound,
                content: card.display_name.clone(),
                msg_type: Some("namecard".to_string()),
                media_ref: Some(card_id.to_string()),
                raw: Some(raw),
                created_at: now,
            },
            None,
        )
        .await
    {
        tracing::error!(
            account_id = %contact.account_id,
            contact_wxid = %contact.wxid,
            error = %err,
            "MCP namecard send succeeded but persisting outbound conversation_messages failed; card delivered but record missing",
        );
    }

    // 置「已引荐」态。同样：MCP 已成功，update 失败不传播（防重发）。
    if let Err(err) = state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! { "$set": build_referred_set_doc(card_id, now) },
            None,
        )
        .await
    {
        tracing::error!(
            account_id = %contact.account_id,
            contact_wxid = %contact.wxid,
            error = %err,
            "MCP namecard send succeeded but updating referred-state failed; card delivered but state not marked",
        );
    }

    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ReferralCard;
    use mongodb::bson::DateTime;

    fn card(enabled: bool, review: &str, stages: Vec<&str>) -> ReferralCard {
        ReferralCard {
            id: None, workspace_id: "ws".into(), account_id: None,
            target_wxid: "wxid_boss".into(), display_name: "老王".into(),
            send_trigger_hint: "要签约时引荐".into(),
            target_stages: stages.into_iter().map(|s| s.to_string()).collect(),
            enabled, review_status: review.into(), review_note: None,
            created_at: DateTime::now(), updated_at: DateTime::now(),
        }
    }

    #[test]
    fn assist_mode_override_beats_account_flag() {
        // 账号关 + 客户 force_on → 开
        assert!(assist_mode_active(Some(false), Some("force_on")));
        // 账号开 + 客户 force_off → 关
        assert!(!assist_mode_active(Some(true), Some("force_off")));
        // 账号开 + 无 override → 开
        assert!(assist_mode_active(Some(true), None));
        // 账号 None + 无 override → 默认关
        assert!(!assist_mode_active(None, None));
        // 无关脏值 override 视为无覆盖
        assert!(assist_mode_active(Some(true), Some("garbage")));
        assert!(!assist_mode_active(Some(false), Some("garbage")));
    }

    #[test]
    fn validate_excludes_draft_and_disabled() {
        assert!(validate_card_sendable(&card(true, "approved", vec![])));
        assert!(!validate_card_sendable(&card(false, "approved", vec![])));
        assert!(!validate_card_sendable(&card(true, "draft", vec![])));
    }

    #[test]
    fn filter_matches_stage_or_empty() {
        let all = vec![
            card(true, "approved", vec!["意向"]),   // 命中
            card(true, "approved", vec!["已成交"]), // 不命中
            card(true, "approved", vec![]),          // 空 = 总命中
            card(false, "approved", vec!["意向"]),  // 排除：disabled
        ];
        let kept = filter_referral_candidates(&all, Some("意向"));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn render_includes_hint_and_already_referred_note() {
        let c = card(true, "approved", vec!["意向"]);
        let line = render_referral_lines(&[&c], None);
        assert!(line.contains("要签约时引荐"));
        assert!(line.contains("老王"));
        let already = AlreadyReferred { display_name: "老王".into(), card_id: "c1".into() };
        let line2 = render_referral_lines(&[&c], Some(&already));
        assert!(line2.contains("已") && line2.contains("老王"));
    }

    #[test]
    fn render_empty_candidates_is_empty() {
        assert_eq!(render_referral_lines(&[], None), "");
    }

    #[test]
    fn referred_set_doc_has_dotted_keys_and_updated_at() {
        let now = DateTime::now();
        let d = build_referred_set_doc("c1", now);
        assert!(d.contains_key("domain_attributes.referred_specialist_at"));
        assert!(d.contains_key("domain_attributes.referred_card_id"));
        assert!(d.contains_key("domain_attributes_updated_at"));
        assert_eq!(d.get_str("domain_attributes.referred_card_id").ok(), Some("c1"));
    }
}
