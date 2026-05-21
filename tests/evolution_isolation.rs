//! agent-self-evolution M4 W4 Task 5.9：演化器路径隔离红线断言（集成）。
//!
//! Requirements 6.5：跑 100 次 shadow replay 之后，`agent_send_outbox` 的
//! collection size 必须保持不变 —— 演化器对生产链路 zero-impact。
//!
//! 默认 `#[ignore]`，依赖 Docker（testcontainers MongoDB）。
//!
//! 路径：直接驱动 `evolution::replay::run_shadow_replay` 100 次（threshold
//! kind 短路重判，不走 LLM、不走 outbox / mcp / gateway），最后比对 outbox
//! 集合 count。这条断言守的不只是"replay 不写 outbox"，还是 evolution 整
//! 个 module tree 的隔离红线（gateway / outbox / mcp 任意分支被错误引入都
//! 会让本测试失败 —— 因为 outbox 一旦写入 collection size 就涨了）。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::evolution::replay::run_shadow_replay;
use wechatagent::models::Proposal;

fn empty_proposal_template() -> Proposal {
    Proposal {
        id: Some(ObjectId::new()),
        experiment_id: "exp_isolation_1".to_string(),
        workspace_id: "default".to_string(),
        account_id: "default".to_string(),
        proposal_kind: "threshold".to_string(),
        status: "pending_eval".to_string(),
        gate_key: Some("fact_risk_block".to_string()),
        current_value: Some(6.0),
        proposed_value: Some(7.0),
        cohort_notes: Document::new(),
        proposed_template_key: None,
        proposed_section: None,
        diff_summary: None,
        diff_snippet: None,
        critic_reasoning: None,
        expected_improvement_on: Vec::new(),
        risk_note: None,
        previous_prompt_version: None,
        eval_metrics: Document::new(),
        eval_replays_completed: 0,
        eval_replays_failed: 0,
        significance_passed: None,
        failure_reason: None,
        released_at: None,
        released_by: None,
        rolled_back_at: None,
        rolled_back_by: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    }
}

async fn insert_inbound_message(app: &common::TestApp, message_id: &str) {
    let raw = app
        .state
        .db
        .raw()
        .collection::<Document>("conversation_messages");
    raw.insert_one(
        doc! {
            "_id": ObjectId::new(),
            "workspace_id": "default",
            "account_id": "default",
            "messageId": message_id,
            "direction": "inbound",
            "fromWxid": "user_isolation",
            "toWxid": "default",
            "content": "ping",
            "createdAt": DateTime::now(),
        },
        None,
    )
    .await
    .expect("seed inbound message");
}

async fn insert_run_log_with_scores(app: &common::TestApp, message_id: &str) -> ObjectId {
    let id = ObjectId::new();
    let scores = doc! {
        "factRisk": 8_i32,
        "pressureRisk": 4_i32,
        "humanLike": 7_i32,
        "emotionalValue": 6_i32,
        "productAccuracy": 8_i32,
    };
    let doc = doc! {
        "_id": id,
        "workspace_id": "default",
        "account_id": "default",
        "contact_wxid": "user_isolation",
        "run_id": format!("run_{}", id.to_hex()),
        "trigger_kind": "inbound_message",
        "status": "completed",
        "lifecycle": "completed",
        "source_event_id": message_id,
        "source_kind": "inbound_message",
        "context": doc! { "inboundMessageId": message_id },
        "review": doc! { "scores": scores },
        "decision": doc! {},
        "knowledge_route": doc! {},
        "planner": doc! {},
        "final_review_status": "held_by_ai_policy",
        "revision_applied": false,
        "created_at": DateTime::now(),
    };
    app.state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .insert_one(doc, None)
        .await
        .expect("seed run log");
    id
}

#[tokio::test]
#[ignore]
async fn shadow_replay_does_not_touch_agent_send_outbox() {
    let app = common::TestApp::start().await;

    // 100 条短路 replay 跑完后这两个集合的 count 都应当 = 0。
    let outbox_before = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! {}, None)
        .await
        .expect("count outbox before");
    assert_eq!(outbox_before, 0, "fresh fixture must have empty outbox");

    let messages_before = app
        .state
        .db
        .messages()
        .count_documents(doc! { "direction": "outbound" }, None)
        .await
        .expect("count outbound messages before");

    // 1. 准备一条 proposal 模板 + 一条 inbound message + 一条 source run。
    let proposal = empty_proposal_template();
    let message_id = "msg_isolation_1";
    insert_inbound_message(&app, message_id).await;
    let source_run_id = insert_run_log_with_scores(&app, message_id).await;

    // 2. 对同一 (proposal, source_run) 跑 100 次 shadow replay。每次都会写一行
    //    `shadow_replays`，但**绝对**不能碰 outbox 或 outbound conversation_messages。
    for _ in 0..100 {
        run_shadow_replay(&app.state, &proposal, source_run_id)
            .await
            .expect("shadow replay should not error on threshold path");
    }

    let shadow_count = app
        .state
        .db
        .raw()
        .collection::<Document>("shadow_replays")
        .count_documents(doc! {}, None)
        .await
        .expect("count shadow_replays");
    assert_eq!(
        shadow_count, 100,
        "expect exactly 100 shadow_replay rows written"
    );

    let outbox_after = app
        .state
        .db
        .collection_agent_send_outbox()
        .count_documents(doc! {}, None)
        .await
        .expect("count outbox after");
    assert_eq!(
        outbox_after, 0,
        "evolution shadow replay must NOT enqueue to agent_send_outbox"
    );

    let messages_after = app
        .state
        .db
        .messages()
        .count_documents(doc! { "direction": "outbound" }, None)
        .await
        .expect("count outbound messages after");
    assert_eq!(
        messages_after, messages_before,
        "evolution shadow replay must NOT write outbound conversation_messages"
    );
}
