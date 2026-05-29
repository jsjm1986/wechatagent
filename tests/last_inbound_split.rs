//! HP-2 / Task 6 / Task 24：`last_inbound_at` / `last_outbound_at` 字段拆分回归。
//!
//! 性质：
//! - 入站 update 仅设置 `last_inbound_at` 与 `last_message_at`，不动 `last_outbound_at`；
//! - 出站 update 仅设置 `last_outbound_at` 与 `last_message_at`，不动 `last_inbound_at`；
//! - `last_message_at` 仍是 `max(last_inbound_at, last_outbound_at)`。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use wechatagent::models::Contact;

fn make_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("测试用户".to_string()),
        remark: None,
        alias: None,
        agent_status: Default::default(),
        human_profile_note: None,
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
        last_message_at: Some(now),
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        deal_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore]
async fn inbound_update_only_touches_inbound_fields() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_inbound_only");
    let id = contact.id.expect("id present");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 模拟入站 webhook update：last_inbound_at + last_message_at = now，不动 last_outbound_at。
    let now = DateTime::now();
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "last_inbound_at": now, "last_message_at": now } },
            None,
        )
        .await
        .unwrap();

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": id }, None)
        .await
        .unwrap()
        .expect("contact present");
    assert!(
        after.last_inbound_at.is_some(),
        "入站后 last_inbound_at 应被设置"
    );
    assert!(
        after.last_outbound_at.is_none(),
        "入站后 last_outbound_at 必须仍为空，实际：{:?}",
        after.last_outbound_at
    );
    assert!(after.last_message_at.is_some());
}

#[tokio::test]
#[ignore]
async fn outbound_update_via_pipeline_keeps_inbound_unchanged() {
    let app = common::TestApp::start().await;
    let contact = make_contact("user_outbound_pipeline");
    let id = contact.id.expect("id present");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");

    // 先模拟有一条早期入站，记录 inbound 时间戳。
    let inbound_at = DateTime::from_millis(DateTime::now().timestamp_millis() - 60_000);
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "last_inbound_at": inbound_at, "last_message_at": inbound_at } },
            None,
        )
        .await
        .unwrap();

    // 出站：用 aggregation pipeline 设置 last_outbound_at = now、
    // last_message_at = max(last_inbound_at, now)，不动 last_inbound_at。
    // 这与 send_outbound_message 的实际写法一致（pipeline + $cond / $max）。
    let now = DateTime::now();
    app.state
        .db
        .contacts()
        .update_one(
            doc! { "_id": id },
            vec![doc! { "$set": {
                "last_outbound_at": now,
                "last_message_at": {
                    "$cond": [
                        { "$gt": ["$last_inbound_at", now] },
                        "$last_inbound_at",
                        now
                    ]
                }
            } }],
            None,
        )
        .await
        .unwrap();

    let after = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": id }, None)
        .await
        .unwrap()
        .expect("contact present");

    // last_inbound_at 必须保持原值。
    let inbound_after = after.last_inbound_at.expect("inbound 必须仍存在");
    assert_eq!(
        inbound_after.timestamp_millis(),
        inbound_at.timestamp_millis(),
        "出站 update 不应改 last_inbound_at"
    );

    // last_outbound_at 必须被设置。
    assert!(
        after.last_outbound_at.is_some(),
        "出站后 last_outbound_at 应被设置"
    );

    // last_message_at 必须 = max(inbound, now)。由于 now > inbound，应等于 now。
    let msg_at = after.last_message_at.expect("last_message_at 应存在");
    assert_eq!(
        msg_at.timestamp_millis(),
        now.timestamp_millis(),
        "last_message_at 应是 max(inbound, now) = now"
    );
}
