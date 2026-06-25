use crate::models::ConversationMessage;

/// 从最近消息往回累积，取到 char_budget（按 content 字符数）或 max_messages（条数）
/// 谁先到为准。返回按时间正序（旧→新）的子集，供装 prompt 顺读。
/// 微信碎消息适配：字符预算保信息量下限，条数防垃圾号（全寒暄）空耗回溯。
pub fn take_window_by_budget(
    msgs: &[ConversationMessage],
    char_budget: usize,
    max_messages: usize,
) -> Vec<ConversationMessage> {
    let mut acc_chars = 0usize;
    let mut picked: Vec<ConversationMessage> = Vec::new();
    // 从最新往最旧回溯（假设入参已按 created_at 升序；若不确定，调用方负责排序）。
    for m in msgs.iter().rev() {
        if picked.len() >= max_messages {
            break;
        }
        let len = m.content.chars().count();
        if !picked.is_empty() && acc_chars + len > char_budget {
            break;
        }
        acc_chars += len;
        picked.push(m.clone());
    }
    picked.reverse(); // 回到时间正序
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversationMessage, MessageDirection};
    use mongodb::bson::oid::ObjectId;

    fn msg(content: &str, ms: i64) -> ConversationMessage {
        ConversationMessage {
            id: Some(ObjectId::new()),
            workspace_id: "w".into(),
            account_id: "a".into(),
            contact_wxid: "c".into(),
            message_id: None,
            dedupe_key: None,
            direction: MessageDirection::Inbound,
            content: content.into(),
            msg_type: None,
            media_ref: None,
            raw: None,
            created_at: mongodb::bson::DateTime::from_millis(ms),
        }
    }

    #[test]
    fn stops_at_message_count_cap_for_short_spam() {
        // 100 条短消息（"在"=1字），条数上限 60 先到。
        let all: Vec<_> = (0..100).map(|i| msg("在", i)).collect();
        let w = take_window_by_budget(&all, 6000, 60);
        assert_eq!(w.len(), 60);
        // 返回的是最近 60 条，按时间正序（最旧的在前）。
        assert_eq!(w.first().unwrap().created_at.timestamp_millis(), 40);
        assert_eq!(w.last().unwrap().created_at.timestamp_millis(), 99);
    }

    #[test]
    fn stops_at_char_budget_for_long_messages() {
        // 每条 2000 字，char_budget 6000 → 取约 3 条（条数上限 60 不触达）。
        let long = "x".repeat(2000);
        let all: Vec<_> = (0..50).map(|i| msg(&long, i)).collect();
        let w = take_window_by_budget(&all, 6000, 60);
        assert!(w.len() <= 4 && !w.is_empty());
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(take_window_by_budget(&[], 6000, 60).is_empty());
    }
}
