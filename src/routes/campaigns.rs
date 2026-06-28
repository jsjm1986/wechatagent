//! 活动定向推送引擎：segment 圈人（两阶段）+ 活动生命周期。
use crate::agent::entitlements;
use crate::models::{Contact, Product, SegmentFilter};
use mongodb::bson::{doc, DateTime, Document};

/// 阶段1：Mongo 粗筛 filter。命中 outcome_events.productRef.productId 索引。
/// product_ids 非空时用 $elemMatch 同元素匹配「买过指定产品 + 高可信 + 正向成交」。
pub(super) fn build_segment_coarse_filter(
    workspace_id: &str,
    account_id: &str,
    filter: &SegmentFilter,
) -> Document {
    let mut d = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "agent_status": "managed",
    };
    // 客户阶段裸字段在粗筛层 filter（domain_attributes.customer_stage 真实路径，
    // 已对 contacts.rs:786 `domain_attributes.get_str("customer_stage")` 核实一致）。
    if let Some(stage) = &filter.customer_stage {
        d.insert("domain_attributes.customer_stage", stage);
    }
    // 产品反查：$elemMatch 同一成交事件内匹配「指定产品 + 高可信 + 正向」。
    if !filter.product_ids.is_empty() {
        d.insert(
            "outcome_events",
            doc! { "$elemMatch": {
                "productRef.productId": { "$in": &filter.product_ids },
                "verification": { "$in": ["staff_confirmed", "payment_verified"] },
                "eventKind": "deal",
            }},
        );
    }
    d
}

/// 阶段2：内存精筛。复用 G4 纯函数判净持有/售后/价值分层。
pub(super) fn contact_matches_segment(
    contact: &Contact,
    active_products: &[Product],
    filter: &SegmentFilter,
    now: DateTime,
    mid_threshold: i64,
    high_threshold: i64,
) -> bool {
    // 复用 G4 投影：净持有（退款抵消、净件数>0）。
    let (entitlements, _) = entitlements::project_entitlements(
        &contact.outcome_events,
        active_products,
        now,
        usize::MAX,
    );
    // 产品维度：要求净持有指定产品之一。
    if !filter.product_ids.is_empty() {
        let holds = entitlements
            .iter()
            .any(|e| filter.product_ids.contains(&e.product_id));
        if !holds {
            return false;
        }
    }
    // 售后维度。
    if let Some(aftercare) = filter.aftercare.as_deref() {
        match aftercare {
            "in_aftercare" => {
                if !entitlements.iter().any(|e| e.in_aftercare == Some(true)) {
                    return false;
                }
            }
            "expired" => {
                if !entitlements.iter().any(|e| e.in_aftercare == Some(false)) {
                    return false;
                }
            }
            _ => {} // "any" 或未知：不约束
        }
    }
    // 价值分层维度。
    if let Some(tier) = filter.value_tier.as_deref() {
        let value = entitlements::compute_customer_value_cents(&contact.outcome_events);
        let actual = entitlements::classify_value_tier(value, mid_threshold, high_threshold);
        if actual != tier {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{OutcomeEvent, OutcomeProductRef};

    fn ev(verification: &str, pid: &str, qty: u32, kind: &str, amount: i64) -> OutcomeEvent {
        OutcomeEvent {
            marked_at: DateTime::from_millis(0),
            occurred_at: Some(DateTime::from_millis(0)),
            amount: Some(amount),
            currency: Some("CNY".to_string()),
            source: "manual".to_string(),
            marked_by: "admin".to_string(),
            note: None,
            verification: verification.to_string(),
            product_ref: Some(OutcomeProductRef {
                product_id: pid.to_string(),
                name: "P".to_string(),
                unit_price: Some(amount),
                sku: None,
                quantity: qty,
                entitlement_days: None,
            }),
            event_kind: kind.to_string(),
        }
    }

    fn contact_with(events: Vec<OutcomeEvent>) -> Contact {
        let mut c = base_contact();
        c.outcome_events = events;
        c
    }

    // 照 models.rs 的 Contact 真实字段构造一个最小 base（managed 状态）。
    pub(super) fn base_contact() -> Contact {
        Contact {
            id: None,
            workspace_id: "ws".to_string(),
            account_id: "acc".to_string(),
            wxid: "wx1".to_string(),
            nickname: None,
            remark: None,
            alias: None,
            agent_status: crate::models::AgentStatus::Managed,
            human_profile_note: None,
            custom_agent_instructions: None,
            operation_mode_override: None,
            agent_profile: None,
            memory_summary: None,
            playbook_id: None,
            playbook_version: None,
            manual_tags: Vec::new(),
            manual_tags_updated_at: None,
            manual_tags_by: None,
            confirmed_tags: Vec::new(),
            bayesian_signals: Vec::new(),
            personality_profile: None,
            tags_version: 0,
            domain_attributes: None,
            domain_attributes_updated_at: None,
            commitments: Vec::new(),
            follow_up_policy: None,
            operation_state: None,
            operation_state_reason: None,
            operation_state_confidence: None,
            operation_state_updated_at: None,
            cooldown_until: None,
            operation_policy: Document::new(),
            profile_attributes: Document::new(),
            profile_updated_at: None,
            last_message_at: None,
            last_inbound_at: None,
            last_outbound_at: None,
            last_agent_run_at: None,
            last_outbound_style: None,
            intent_trajectory: Vec::new(),
            outcome_events: Vec::new(),
            locale: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        }
    }

    #[test]
    fn coarse_filter_with_products_uses_elemmatch_real_keys() {
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        let d = build_segment_coarse_filter("ws", "acc", &f);
        // 真实混合大小写路径
        let em = d.get_document("outcome_events").unwrap();
        let elem = em.get_document("$elemMatch").unwrap();
        // productRef.productId（camelCase 内嵌）
        assert!(elem.get_document("productRef").is_ok()
            || elem.contains_key("productRef.productId"));
        // verification 高可信 $in
        assert!(elem.contains_key("verification"));
        // eventKind 正向
        assert_eq!(elem.get_str("eventKind").ok(), Some("deal"));
        // 始终带租户隔离
        assert_eq!(d.get_str("workspace_id").unwrap(), "ws");
        assert_eq!(d.get_str("account_id").unwrap(), "acc");
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn coarse_filter_empty_products_skips_outcome_condition() {
        let f = SegmentFilter::default();
        let d = build_segment_coarse_filter("ws", "acc", &f);
        // 空 product_ids：不加 outcome_events 条件，退化为按其他维度圈纳管客户
        assert!(d.get("outcome_events").is_none());
        assert_eq!(d.get_str("agent_status").unwrap(), "managed");
    }

    #[test]
    fn precise_filter_net_holding_excludes_fully_refunded() {
        // 买1件后全额退款 → 净持有0 → 不命中「买过 vip」
        let events = vec![
            ev("staff_confirmed", "vip", 1, "deal", 19900),
            ev("staff_confirmed", "vip", 1, "reversal", 19900),
        ];
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        assert!(!contact_matches_segment(
            &contact_with(events), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn precise_filter_conversation_inferred_never_matches() {
        // conversation_inferred 不进 G4 投影 → 不算持有
        let events = vec![ev("conversation_inferred", "vip", 1, "deal", 19900)];
        let f = SegmentFilter { product_ids: vec!["vip".into()], ..Default::default() };
        assert!(!contact_matches_segment(
            &contact_with(events), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }

    #[test]
    fn precise_filter_value_tier_high_only() {
        // 累计 35 万分 = high 档（high_threshold=30万）；要求 high → 命中
        let events = vec![ev("staff_confirmed", "vip", 1, "deal", 350000)];
        let f = SegmentFilter {
            product_ids: vec!["vip".into()],
            value_tier: Some("high".into()),
            ..Default::default()
        };
        assert!(contact_matches_segment(
            &contact_with(events.clone()), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
        // 要求 high 但只值 1.99 元(19900分=low) → 不命中
        let cheap = vec![ev("staff_confirmed", "vip", 1, "deal", 19900)];
        assert!(!contact_matches_segment(
            &contact_with(cheap), &[], &f, DateTime::from_millis(1_000_000), 50000, 300000
        ));
    }
}
