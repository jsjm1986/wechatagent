//! `behavior_signal_smoke` —— 自学习采集管道（第一阶段）端到端落库冒烟。
//!
//! 依赖 testcontainers MongoDB（`#[ignore]` 守门，CI 用
//! `cargo test --test behavior_signal_smoke -- --ignored`，需 Docker）。
//!
//! 覆盖三件事：
//!   1. **behavior_signals 幂等去重** —— 同一 `dedupe_key` 的信号 insert 两次，
//!      第二次撞 partial unique 索引被 `persist_signal` 吞成 `Ok(false)`，集合里
//!      只留一条；不同 dedupe_key 各落一条。
//!   2. **deal_events 追加** —— 直接 `$push` 一条 manual 成交事件到 Contact，
//!      读回断言 `source="manual"` 且数组长度 +1（S5 正例池 append-only）。
//!   3. **silence worker 单轮幂等** —— 造一条久未回复的 outbound contact，
//!      `silence_signal_worker::tick` 跑两轮，沉默信号只落一条且 `censored=true`。

mod common;

use std::time::Duration;

use mongodb::bson::{doc, DateTime};
use wechatagent::behavior_signals as bs;
use wechatagent::models::{AgentStatus, Contact, OutcomeEvent};

use crate::common::TestApp;

fn contact_template(wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: None,
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        agent_status: AgentStatus::Managed,
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
        operation_policy: Default::default(),
        profile_attributes: Default::default(),
        profile_updated_at: None,
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore = "需要 Docker / testcontainers MongoDB"]
async fn behavior_signal_dedupe_round_trip() {
    let app = TestApp::start().await;
    let state = &app.state;

    let inbound_at = DateTime::from_millis(10_000);
    let sig = bs::build_reply_latency("default", "wxid_a", "msg1", inbound_at, Some(5_000));

    // 首次写入 → Ok(true)。
    let first = bs::persist_signal(state, sig.clone()).await.expect("persist 1");
    assert!(first, "首次写入应成功");
    // 同 dedupe_key 再写 → 撞 partial unique 索引 → Ok(false)。
    let second = bs::persist_signal(state, sig.clone()).await.expect("persist 2");
    assert!(!second, "同 dedupe_key 第二次写入应被幂等吞掉");

    // 不同 dedupe_key（reply_length 同 msg）→ 各落一条。
    let len_sig = bs::build_reply_length("default", "wxid_a", "msg1", inbound_at, "你好👋");
    assert!(bs::persist_signal(state, len_sig).await.expect("persist len"));

    let count = state
        .db
        .behavior_signals()
        .count_documents(doc! { "contact_wxid": "wxid_a" }, None)
        .await
        .expect("count signals");
    assert_eq!(count, 2, "latency + length 各一条，重复 latency 不计");

    // 落库的 latency 信号字段核对（观察层不解释）。
    let stored = state
        .db
        .behavior_signals()
        .find_one(doc! { "signal_type": "reply_latency" }, None)
        .await
        .expect("find latency")
        .expect("latency exists");
    assert_eq!(stored.source, bs::SOURCE_SYSTEM_OBSERVED);
    assert_eq!(stored.confidence, 1.0);
    assert!(!stored.censored);
    assert_eq!(stored.latency_ms, Some(5_000));
}

#[tokio::test]
#[ignore = "需要 Docker / testcontainers MongoDB"]
async fn deal_event_push_round_trip() {
    let app = TestApp::start().await;
    let state = &app.state;

    let mut contact = contact_template("wxid_deal");
    let insert = state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    let oid = insert.inserted_id.as_object_id().expect("oid");
    contact.id = Some(oid);

    let outcome = OutcomeEvent {
        marked_at: DateTime::now(),
        occurred_at: None,
        amount: Some(199.0),
        currency: Some("CNY".to_string()),
        source: "manual".to_string(),
        marked_by: "admin_smoke".to_string(),
        note: Some("首单".to_string()),
    };
    // H10 向后兼容：故意用**旧** `deal_events` key 写库，验证 serde alias 让旧库
    // 文档仍能反序列化到新 `outcome_events` 字段（改名前写入的存量数据不丢）。
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": oid, "workspace_id": "default" },
            doc! { "$push": { "deal_events": mongodb::bson::to_bson(&outcome).unwrap() } },
            None,
        )
        .await
        .expect("push outcome event under legacy key");

    let reloaded = state
        .db
        .contacts()
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("reload")
        .expect("contact exists");
    assert_eq!(reloaded.outcome_events.len(), 1, "旧 deal_events key 经 alias 读入 outcome_events");
    assert_eq!(reloaded.outcome_events[0].source, "manual");
    assert_eq!(reloaded.outcome_events[0].amount, Some(199.0));
    assert_eq!(reloaded.outcome_events[0].marked_by, "admin_smoke");
}

#[tokio::test]
#[ignore = "需要 Docker / testcontainers MongoDB"]
async fn silence_worker_single_round_idempotent() {
    let app = TestApp::start().await;
    // 打开 worker flag + 极短阈值，让单轮 tick 立即判沉默。
    let mut state = app.state.clone();
    state.config.silence_signal_worker_enabled = true;
    state.config.silence_threshold_seconds = 1;
    state.config.silence_signal_interval_seconds = 600;

    // 造一条 24h 前 outbound、之后无 inbound 的 managed contact。
    let mut contact = contact_template("wxid_silent");
    let outbound_at = DateTime::from_millis(DateTime::now().timestamp_millis() - 24 * 3600 * 1000);
    contact.last_outbound_at = Some(outbound_at);
    state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert silent contact");

    // 跑两轮 tick：第二轮应被 dedupe_key 幂等挡掉。
    wechatagent::silence_signal_worker::tick(&state)
        .await
        .expect("tick 1");
    wechatagent::silence_signal_worker::tick(&state)
        .await
        .expect("tick 2");

    let count = state
        .db
        .behavior_signals()
        .count_documents(
            doc! { "contact_wxid": "wxid_silent", "signal_type": "silence" },
            None,
        )
        .await
        .expect("count silence");
    assert_eq!(count, 1, "同一条 outbound 多轮 tick 只落一条沉默信号");

    let stored = state
        .db
        .behavior_signals()
        .find_one(doc! { "signal_type": "silence" }, None)
        .await
        .expect("find silence")
        .expect("silence exists");
    assert!(stored.censored, "沉默信号必须 censored=true（删失，不是负例）");
    assert_eq!(stored.unanswered, Some(true));

    // 给 worker 一点时间确保没有遗留 spawn（本测试只调 tick，不 spawn loop）。
    tokio::time::sleep(Duration::from_millis(10)).await;
}
