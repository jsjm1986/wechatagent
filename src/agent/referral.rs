//! 专属顾问名片引荐：辅助模式判定、候选过滤/渲染（纯函数）、
//! send_outbound_namecard、置「已引荐」态。
use crate::error::{AppError, AppResult};
use crate::models::{ConversationMessage, MessageDirection, ReferralCard};
use crate::routes::AppState;
use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime};
use serde_json::{json, Value};

/// ④reviewer 让位段（辅助模式下注入 reviewer system prompt）。同时消解两条 hold 路径：
/// ① 引荐专属顾问不属于「除『我』外不得出现人类角色」红线所禁——红线在引荐这一受控动作上让位；
/// ② 引荐不是产品能力声明（不涉产品功能/价格/效果）→ 不计入 hallucination/产品准确度评分。
/// 措辞红线：用「专属顾问/增配/我仍在场辅助」，措辞合规、可过提交期词表门。
/// assist 关账号不注入此段（reviewer system prompt 字节等价）。
pub(crate) const REVIEWER_ASSIST_YIELD_NOTE: &str = "\n【受控例外·辅助模式】本账号已开启辅助模式。在辅助模式下，AI 主动为契合条件的客户增配一位「专属顾问」（决策里的 namecardToSend）是本账号的受控业务动作，定位是「我为你增配一位更专属的顾问、我仍在场辅助答疑」。评审时请注意两点：① 这一引荐动作不属于「对话中除『我』之外不得出现任何人类角色」红线所禁止的范畴，该红线在「引荐专属顾问」上让位，不要因此判 needs_revision；② 引荐专属顾问不是对产品能力/价格/效果的声明，不计入 hallucination/产品准确度评分（不要据此抬高 factRisk）。仅当客户真正契合（明确要签约/到店/深度对接）时才引荐，不为引荐而引荐。\n";

/// 辅助模式是否对本客户生效。客户级 override > 账号级 enabled > 默认关。
pub(crate) fn assist_mode_active(
    account_enabled: Option<bool>,
    override_attr: Option<&str>,
) -> bool {
    match override_attr {
        Some("force_on") => true,
        Some("force_off") => false,
        _ => account_enabled.unwrap_or(false),
    }
}

/// 发送前准入：仅 enabled + approved + account 归属匹配的名片可被 AI 选/发。
///
/// KE-03：account 归属校验与 enabled/approved 同层（三条发送路径——候选加载
/// `filter_referral_candidates`、gateway 二次准入、`send_outbound_namecard`——
/// 全部经过本纯函数做二次校验，故 account 归属加在此处一处生效、口径单一）。
/// `account_id` = 本 contact 的账号。global scope 卡（`card.account_id=None`）
/// 任何账号可用；绑定某账号的卡仅该账号可用——与候选加载 DB filter
/// `build_referral_cards_filter` 的 `$or:[{account_id:null},{account_id:==account_id}]`
/// 口径完全一致，杜绝「同 workspace 内绑定账号 A 的名片经账号 B 会话推出」。
pub(crate) fn validate_card_sendable(card: &ReferralCard, account_id: &str) -> bool {
    let account_ok = match card.account_id.as_deref() {
        None => true,                       // global scope 卡：任何账号可用
        Some(bound) => bound == account_id, // 绑定卡：仅本账号
    };
    card.enabled && card.review_status == "approved" && account_ok
}

pub(crate) fn filter_referral_candidates<'a>(
    cards: &'a [ReferralCard],
    customer_stage: Option<&str>,
    account_id: &str,
) -> Vec<&'a ReferralCard> {
    cards
        .iter()
        .filter(|c| validate_card_sendable(c, account_id))
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
        // 维度标签（与素材侧对称）：仅供 AI 参考的候选清单注入，非空才渲染，不作硬过滤门。
        let tags_seg = if c.tags.is_empty() {
            String::new()
        } else {
            format!(" | 标签:{}", c.tags.join(","))
        };
        out.push_str(&format!(
            "- [card:{id}] {} | 阶段:{stages}{tags_seg} | 触发提示:{}\n",
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
    let oid = ObjectId::parse_str(card_id).map_err(|_| AppError::External("bad card_id".into()))?;
    // 查询带 workspace_id scope（防跨租户读名片，与 send_outbound_media 的 IDOR 防御对齐）。
    let card = state
        .db
        .referral_cards()
        .find_one(
            doc! { "_id": oid, "workspace_id": &contact.workspace_id },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("referral card not found".into()))?;

    // 发送前准入二次校验（防 AI 幻觉/已撤下名片一路漏到发送 + KE-03 account 归属）。
    if !validate_card_sendable(&card, &contact.account_id) {
        return Err(AppError::External(
            "referral card not sendable (draft/disabled/account mismatch)".into(),
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

    if !super::gateway::send_receipt_is_ok(&resp) {
        return Err(AppError::External(
            "namecard send returned a negative or unverifiable delivery receipt".into(),
        ));
    }

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
                is_synthetic_relay: false,
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
            id: None,
            workspace_id: "ws".into(),
            account_id: None,
            target_wxid: "wxid_boss".into(),
            display_name: "老王".into(),
            send_trigger_hint: "要签约时引荐".into(),
            target_stages: stages.into_iter().map(|s| s.to_string()).collect(),
            tags: vec![],
            enabled,
            review_status: review.into(),
            review_note: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn reviewer_assist_yield_note_covers_two_paths_and_passes_lint() {
        let note = super::REVIEWER_ASSIST_YIELD_NOTE;
        // 解路径①：引荐不属于「第三方角色失约」红线。
        assert!(note.contains("专属顾问"));
        assert!(note.contains("让位") || note.contains("不属于"));
        // 解路径②：引荐不是产品能力声明,不计入 hallucination/产品准确度。
        assert!(note.contains("不是产品") || note.contains("不计入"));
        // 措辞合规由提交期词表门统一把关——词表本身不在此重复(否则禁词字面量
        // 落到被扫描文件里会自伤),本测试只断言两路径语义。
    }

    #[test]
    fn render_referral_includes_tags() {
        let mut card = ReferralCard {
            id: None,
            workspace_id: "ws".into(),
            account_id: None,
            target_wxid: "wxid_boss".into(),
            display_name: "老王".into(),
            send_trigger_hint: "签约时引荐".into(),
            target_stages: vec!["意向".into()],
            tags: vec!["高客单".into()],
            enabled: true,
            review_status: "approved".into(),
            review_note: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        let out = render_referral_lines(&[&card], None);
        assert!(out.contains("高客单"), "引荐候选应渲染 tags");
        card.tags.clear();
        let out2 = render_referral_lines(&[&card], None);
        assert!(!out2.contains("标签:"), "空 tags 不渲染标签段");
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
        assert!(validate_card_sendable(
            &card(true, "approved", vec![]),
            "acct"
        ));
        assert!(!validate_card_sendable(
            &card(false, "approved", vec![]),
            "acct"
        ));
        assert!(!validate_card_sendable(
            &card(true, "draft", vec![]),
            "acct"
        ));
    }

    #[test]
    fn validate_card_account_scope() {
        // global 卡（account_id=None）→ 任何 account 可用（行为不变，与候选 DB filter $or:[null,...] 一致）。
        assert!(validate_card_sendable(
            &card(true, "approved", vec![]),
            "acct_A"
        ));
        assert!(validate_card_sendable(
            &card(true, "approved", vec![]),
            "acct_B"
        ));

        // 绑定 acct_A 的卡：只有 acct_A 可用，acct_B 拒（KE-03 核心：跨账号不可推）。
        let mut bound = card(true, "approved", vec![]);
        bound.account_id = Some("acct_A".to_string());
        assert!(validate_card_sendable(&bound, "acct_A"), "本账号卡须放行");
        assert!(
            !validate_card_sendable(&bound, "acct_B"),
            "绑定 acct_A 的卡不得经 acct_B 会话推出(KE-03 跨账号防护,回退即红)"
        );

        // account 门与 enabled/approved 门叠加：绑定卡即使 account 匹配,draft/disabled 仍拒。
        let mut bound_draft = card(false, "draft", vec![]);
        bound_draft.account_id = Some("acct_A".to_string());
        assert!(!validate_card_sendable(&bound_draft, "acct_A"));
    }

    #[test]
    fn filter_matches_stage_or_empty() {
        let all = vec![
            card(true, "approved", vec!["意向"]),   // 命中
            card(true, "approved", vec!["已成交"]), // 不命中
            card(true, "approved", vec![]),         // 空 = 总命中
            card(false, "approved", vec!["意向"]),  // 排除：disabled
        ];
        let kept = filter_referral_candidates(&all, Some("意向"), "acct");
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn render_includes_hint_and_already_referred_note() {
        let c = card(true, "approved", vec!["意向"]);
        let line = render_referral_lines(&[&c], None);
        assert!(line.contains("要签约时引荐"));
        assert!(line.contains("老王"));
        let already = AlreadyReferred {
            display_name: "老王".into(),
            card_id: "c1".into(),
        };
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
        assert_eq!(
            d.get_str("domain_attributes.referred_card_id").ok(),
            Some("c1")
        );
    }
}
