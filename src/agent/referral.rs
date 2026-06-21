//! 专属顾问名片引荐：辅助模式判定、候选过滤/渲染（纯函数）、
//! send_outbound_namecard、置「已引荐」态。
use crate::models::ReferralCard;

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
}
