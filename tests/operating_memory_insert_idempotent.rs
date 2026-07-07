//! CONC-3：`load_or_create_operating_memory` 的 create 分支并发 insert 时，
//! 输给唯一索引 (workspace_id, account_id, contact_wxid) 的一方应回落 find_one
//! 返回赢家文档，而非 E11000 透传失败（透传会让回复客户之前整轮 run 失败）。

mod common;

use futures::future::join_all;

use mongodb::bson::{oid::ObjectId, DateTime, Document};
use wechatagent::agent::load_or_create_operating_memory;
use wechatagent::models::Contact;

fn make_contact(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: Some("并发首触达客户".to_string()),
        remark: None,
        alias: None,
        avatar_url: None,
        agent_status: Default::default(),
        human_profile_note: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        manual_tags_updated_at: None,
        manual_tags_by: None,
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
        last_message_at: Some(now),
        last_inbound_at: Some(now),
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        locale: None,
        outcome_events: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB"]
async fn concurrent_first_touch_inserts_all_succeed() {
    let app = common::TestApp::start().await;
    let contact = make_contact("wxid_conc3");

    // 同 contact 并发触发 create 分支（首次触达，库里无 operating_memory）。
    // 唯一索引 (workspace_id, account_id, contact_wxid) 让其中至多一方 insert 成功，
    // 其余方收到 E11000；修复前输者透传 Err，修复后回落 find_one 返回赢家文档。
    let futs = (0..4).map(|_| {
        let state = app.state.clone();
        let contact = contact.clone();
        async move { load_or_create_operating_memory(&state, &contact).await }
    });
    let results = join_all(futs).await;

    // 全部返回 Ok（同一 contact_wxid 的文档）；无一返回 Err。
    for r in &results {
        let mem = r
            .as_ref()
            .expect("load_or_create 不应因并发 dup-key 失败");
        assert_eq!(mem.contact_wxid, "wxid_conc3");
    }

    // 唯一索引保证库里恰好一条 operating_memory 行（无重复写穿）。
    let count = app
        .state
        .db
        .operating_memories()
        .count_documents(
            mongodb::bson::doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid
            },
            None,
        )
        .await
        .expect("count operating_memories");
    assert_eq!(count, 1, "并发首触达应只落一条 operating_memory");
}
