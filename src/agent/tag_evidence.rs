use crate::models::{ConversationMessage, Evidence, MessageDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStrength {
    Strong,
    Weak,
}

/// 把 LLM 给的窗口序位（0-based，按 created_at 排序后的下标）映射成 Evidence。
/// 越界序位 / 无 _id 的消息直接丢弃（fail-closed：锚不上不放水）。
pub fn resolve_evidence(window: &[ConversationMessage], turn_indices: &[i32]) -> Vec<Evidence> {
    let mut out = Vec::new();
    for &idx in turn_indices {
        if idx < 0 {
            continue;
        }
        let Some(msg) = window.get(idx as usize) else {
            continue;
        };
        let Some(oid) = msg.id else {
            continue;
        };
        out.push(Evidence {
            turn: idx,
            msg_id: oid.to_hex(),
        });
    }
    out
}

/// 强证据：至少一条证据指向客户本人(Inbound)消息，且 LLM 标注 explicit_intent=true。
/// 否则弱。强弱由消息 direction + explicit 标志客观决定，不读 LLM 自称置信。
pub fn evidence_strength(
    evidences: &[Evidence],
    window: &[ConversationMessage],
    explicit_intent: bool,
) -> EvidenceStrength {
    if !explicit_intent {
        return EvidenceStrength::Weak;
    }
    let anchored_to_customer = evidences.iter().any(|e| {
        window
            .get(e.turn as usize)
            .map(|m| matches!(m.direction, MessageDirection::Inbound))
            .unwrap_or(false)
    });
    if anchored_to_customer {
        EvidenceStrength::Strong
    } else {
        EvidenceStrength::Weak
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversationMessage, MessageDirection};
    use mongodb::bson::oid::ObjectId;

    fn msg(dir: MessageDirection) -> ConversationMessage {
        ConversationMessage {
            id: Some(ObjectId::new()),
            workspace_id: "w".into(),
            account_id: "a".into(),
            contact_wxid: "c".into(),
            message_id: None,
            dedupe_key: None,
            direction: dir,
            content: "x".into(),
            msg_type: None,
            media_ref: None,
            raw: None,
            is_synthetic_relay: false,
            created_at: mongodb::bson::DateTime::from_millis(0),
        }
    }

    #[test]
    fn resolve_evidence_maps_index_to_oid_and_drops_out_of_range() {
        let w = vec![
            msg(MessageDirection::Inbound),
            msg(MessageDirection::Outbound),
        ];
        let ev = resolve_evidence(&w, &[0, 5]); // 0 有效, 5 越界
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].turn, 0);
        assert_eq!(ev[0].msg_id, w[0].id.unwrap().to_hex());
    }

    #[test]
    fn strength_strong_when_inbound_and_explicit() {
        let w = vec![msg(MessageDirection::Inbound)];
        let ev = resolve_evidence(&w, &[0]);
        assert!(matches!(
            evidence_strength(&ev, &w, true),
            EvidenceStrength::Strong
        ));
    }

    #[test]
    fn strength_weak_when_outbound_even_if_explicit() {
        let w = vec![msg(MessageDirection::Outbound)];
        let ev = resolve_evidence(&w, &[0]);
        assert!(matches!(
            evidence_strength(&ev, &w, true),
            EvidenceStrength::Weak
        ));
    }

    #[test]
    fn strength_weak_when_not_explicit() {
        let w = vec![msg(MessageDirection::Inbound)];
        let ev = resolve_evidence(&w, &[0]);
        assert!(matches!(
            evidence_strength(&ev, &w, false),
            EvidenceStrength::Weak
        ));
    }
}
