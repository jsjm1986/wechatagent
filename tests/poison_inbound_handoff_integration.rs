//! 缺陷 #2（毒丸消息行）回归：`reconcile_pending_inbound_handoffs` 遇到无法
//! 反序列化的 pending 行时，必须隔离该行（`handoff_status=quarantined` + 事件）
//! 并继续处理其后的正常行，而不是把 Err 传播回 task worker 使两个 tick 永久停摆
//! （坏行按 `created_at` 升序恒排最前，`?` 中止会形成永久毒丸）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, MessageDirection, OperationMode,
};
use wechatagent::webhooks::{reconcile_pending_inbound_handoffs, DURABLE_INBOUND_REPLY_KIND};

fn managed_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    let mut operation_mode = OperationMode::default();
    operation_mode.quiet_hours.enabled_override = Some(false);
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("poison-pill contact".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: Some(operation_mode),
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
        operation_state: Some("need_discovery".to_string()),
        operation_state_reason: None,
        operation_state_confidence: Some(7),
        operation_state_updated_at: None,
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: Document::new(),
        profile_updated_at: None,
        last_message_at: Some(now),
        last_inbound_at: Some(now),
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

fn inbound(wxid: &str, id: ObjectId, created_at: DateTime, suffix: &str) -> ConversationMessage {
    ConversationMessage {
        id: Some(id),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        contact_wxid: wxid.to_string(),
        message_id: Some(format!("poison-{suffix}")),
        dedupe_key: Some(format!("message:poison-{suffix}")),
        direction: MessageDirection::Inbound,
        content: format!("poison-pill inbound {suffix}"),
        msg_type: Some("text".to_string()),
        media_ref: None,
        raw: Some(doc! { "source": "poison-pill-test" }),
        is_synthetic_relay: false,
        created_at,
    }
}

async fn insert_pending_handoff(
    state: &wechatagent::routes::AppState,
    message: &ConversationMessage,
) {
    let mut raw = to_document(message).expect("serialize inbound");
    raw.insert("handoff_status", "pending");
    state
        .db
        .messages()
        .clone_with_type::<Document>()
        .insert_one(raw, None)
        .await
        .expect("insert pending inbound handoff");
}

#[tokio::test]
#[ignore]
async fn undecodable_pending_row_is_quarantined_and_later_rows_still_reconcile() {
    let app = common::TestApp::start().await;
    let contact = managed_contact("poison-quarantine");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert managed contact");

    let base_ms = DateTime::now().timestamp_millis();
    // 坏行：direction=inbound + handoff_status=pending，但缺 typed 必填字段
    // `content` → `bson::from_document::<ConversationMessage>` 必失败。created_at
    // 早于正常行，使其在 `created_at:1` 升序扫描中恒排最前（毒丸位）。
    let poison_id = ObjectId::new();
    app.state
        .db
        .messages()
        .clone_with_type::<Document>()
        .insert_one(
            doc! {
                "_id": poison_id,
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": &contact.wxid,
                "direction": "inbound",
                "handoff_status": "pending",
                "created_at": DateTime::from_millis(base_ms - 60_000),
            },
            None,
        )
        .await
        .expect("insert poison pending row");

    let good = inbound(
        &contact.wxid,
        ObjectId::new(),
        DateTime::from_millis(base_ms),
        "good",
    );
    insert_pending_handoff(&app.state, &good).await;

    // 毒丸在场时 reconcile 必须返回 Ok，且正常行照常物化。
    let recovered = reconcile_pending_inbound_handoffs(&app.state)
        .await
        .expect("reconcile must not fail on a single undecodable row");
    assert_eq!(recovered, 1, "exactly the good row is recovered");

    let messages = app.state.db.messages().clone_with_type::<Document>();
    let poison_row = messages
        .find_one(doc! { "_id": poison_id }, None)
        .await
        .expect("read poison row")
        .expect("poison row still present");
    assert_eq!(
        poison_row.get_str("handoff_status").unwrap(),
        "quarantined",
        "undecodable row must be isolated out of the scan filter"
    );
    assert!(
        poison_row.get_datetime("handoff_updated_at").is_ok(),
        "quarantine stamps handoff_updated_at"
    );

    let good_row = messages
        .find_one(doc! { "_id": good.id.unwrap() }, None)
        .await
        .expect("read good row")
        .expect("good row present");
    assert_eq!(good_row.get_str("handoff_status").unwrap(), "materialized");

    let task_count = app
        .state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .count_documents(
            doc! {
                "workspace_id": "default",
                "account_id": "default",
                "contact_wxid": &contact.wxid,
                "kind": DURABLE_INBOUND_REPLY_KIND,
            },
            None,
        )
        .await
        .expect("count durable tasks");
    assert_eq!(task_count, 1, "good row must still materialize its task");

    let event = app
        .state
        .db
        .events()
        .clone_with_type::<Document>()
        .find_one(doc! { "kind": "inbound_handoff_quarantined" }, None)
        .await
        .expect("query quarantine event")
        .expect("quarantine event must be written");
    let details = event.get_document("details").expect("event details");
    assert_eq!(
        details.get_object_id("message_id").expect("message_id"),
        poison_id
    );
    assert!(
        !details
            .get_str("decode_error")
            .expect("decode_error text")
            .is_empty(),
        "event carries the decode error text"
    );

    // 隔离后重扫：quarantined 行不再命中 filter，不重复隔离、不重复计数。
    let recovered_again = reconcile_pending_inbound_handoffs(&app.state)
        .await
        .expect("second reconcile is clean");
    assert_eq!(recovered_again, 0);
    let event_count = app
        .state
        .db
        .events()
        .clone_with_type::<Document>()
        .count_documents(doc! { "kind": "inbound_handoff_quarantined" }, None)
        .await
        .expect("count quarantine events");
    assert_eq!(event_count, 1, "quarantine event is written exactly once");

    app.cleanup().await;
}
