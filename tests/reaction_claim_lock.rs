//! HP-3 / Task 10 回归：record_user_reaction 的 atomic claim 锁。
//!
//! 默认 `#[ignore]`，需要 Docker（testcontainers MongoDB）。
//!
//! 端到端"N 个并发 webhook 触发同 contact 至多 1 次 LLM 调用"由 PBT 在
//! Task 24 中收口；本文件提供 schema 和模型层面的回归。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use wechatagent::models::AgentDecisionReview;

fn pending_review(workspace: &str, account: &str, wxid: &str) -> AgentDecisionReview {
    AgentDecisionReview {
        id: Some(ObjectId::new()),
        workspace_id: workspace.to_string(),
        account_id: account.to_string(),
        contact_wxid: Some(wxid.to_string()),
        run_id: Some("run_test".to_string()),
        inbound_message_id: None,
        reply_text: Some("hi".to_string()),
        approved: true,
        scores: Document::new(),
        formula_breakdown: Document::new(),
        risks: Vec::new(),
        rewrite_instruction: None,
        review_summary: None,
        playbook_id: None,
        playbook_version: None,
        used_knowledge_ids: Vec::new(),
        prompt_versions: Document::new(),
        operation_state: None,
        next_best_action: Document::new(),
        context_pack_snapshot: Document::new(),
        domain_config_snapshot: Document::new(),
        runtime_parameters_snapshot: Document::new(),
        send_gateway_result: Document::new(),
        outcome_status: Some("pending".to_string()),
        reaction_analysis: Document::new(),
        reaction_claimed_at: None,
        status: "sent".to_string(),
        created_at: DateTime::now(),
    }
}

#[tokio::test]
#[ignore]
async fn analyzing_state_can_be_claimed_atomically() {
    let app = common::TestApp::start().await;
    let review = pending_review("default", "default", "user_concurrent");
    let id = review.id.expect("id present");
    app.state
        .db
        .decision_reviews()
        .insert_one(&review, None)
        .await
        .expect("insert pending review");

    // 第一次 claim：pending → analyzing 应成功。
    let first = app
        .state
        .db
        .decision_reviews()
        .find_one_and_update(
            doc! {
                "_id": id,
                "$or": [
                    { "outcome_status": null },
                    { "outcome_status": "pending" }
                ]
            },
            doc! {
                "$set": {
                    "outcome_status": "analyzing",
                    "reaction_claimed_at": DateTime::now()
                }
            },
            None,
        )
        .await
        .unwrap();
    assert!(first.is_some(), "first claim should succeed");

    // 第二次相同条件 claim：因为 outcome_status 现在是 analyzing，filter 不命中。
    let second = app
        .state
        .db
        .decision_reviews()
        .find_one_and_update(
            doc! {
                "_id": id,
                "$or": [
                    { "outcome_status": null },
                    { "outcome_status": "pending" }
                ]
            },
            doc! {
                "$set": {
                    "outcome_status": "analyzing",
                    "reaction_claimed_at": DateTime::now()
                }
            },
            None,
        )
        .await
        .unwrap();
    assert!(second.is_none(), "concurrent second claim should fail");
}

/// Task 24：N=10 并发 claim 测试。所有并发请求中至多 1 个能成功 claim 到 analyzing。
#[tokio::test]
#[ignore]
async fn ten_concurrent_claims_at_most_one_succeeds() {
    let app = common::TestApp::start().await;
    let review = pending_review("default", "default", "user_n10");
    let id = review.id.expect("id present");
    app.state
        .db
        .decision_reviews()
        .insert_one(&review, None)
        .await
        .expect("insert pending review");

    let mut handles = Vec::new();
    for _ in 0..10 {
        let db = app.state.db.clone();
        let task_id = id;
        handles.push(tokio::spawn(async move {
            db.decision_reviews()
                .find_one_and_update(
                    doc! {
                        "_id": task_id,
                        "$or": [
                            { "outcome_status": null },
                            { "outcome_status": "pending" }
                        ]
                    },
                    doc! {
                        "$set": {
                            "outcome_status": "analyzing",
                            "reaction_claimed_at": DateTime::now()
                        }
                    },
                    None,
                )
                .await
                .map(|opt| opt.is_some())
                .unwrap_or(false)
        }));
    }

    let mut successful_claims = 0;
    for handle in handles {
        if handle.await.unwrap_or(false) {
            successful_claims += 1;
        }
    }
    assert_eq!(
        successful_claims, 1,
        "N=10 并发 claim 中应恰好 1 个成功，实际 {}",
        successful_claims
    );
}
