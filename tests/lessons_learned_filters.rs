//! `lessons_learned` 聚合数据源与过滤条件回归。
//!
//! 真实用户反应写在 `agent_decision_reviews`，安全拦截终态写在
//! `agent_run_logs`。三类模式都必须能从各自权威数据源生成待审核 lesson。

#![cfg(test)]

mod common;

use mongodb::bson::{doc, DateTime, Document};

use crate::common::TestApp;
use wechatagent::knowledge_wiki::lessons_learned::aggregate_lessons_for_workspace;

fn sample_ids(lesson: &Document) -> Vec<&str> {
    lesson
        .get_array("sample_run_ids")
        .expect("sample_run_ids")
        .iter()
        .filter_map(|value| value.as_str())
        .collect()
}

#[tokio::test]
#[ignore = "requires Docker (testcontainers MongoDB)"]
async fn aggregates_all_three_patterns_from_their_authoritative_sources() {
    let app = TestApp::start().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let now = DateTime::now();

    app.state
        .db
        .raw()
        .collection::<Document>("agent_decision_reviews")
        .insert_many(
            vec![
                doc! {
                    "workspace_id": &workspace_id,
                    "approved": true,
                    "outcome_status": "user_replied_buying_signal",
                    "run_id": "run_success",
                    "created_at": now,
                },
                doc! {
                    "workspace_id": &workspace_id,
                    "approved": true,
                    "outcome_status": "user_replied_objection",
                    "reviewer_misjudge_signal": "approved_but_user_negative",
                    "run_id": "run_failure",
                    "created_at": now,
                },
            ],
            None,
        )
        .await
        .expect("seed decision reviews");

    app.state
        .db
        .raw()
        .collection::<Document>("agent_run_logs")
        .insert_one(
            doc! {
                "workspace_id": &workspace_id,
                "lifecycle": "failed_after_decision",
                "final_review_status": "blocked_by_safety_guard",
                "run_id": "run_blocked",
                "created_at": now,
            },
            None,
        )
        .await
        .expect("seed blocked run");

    let report = aggregate_lessons_for_workspace(&app.state, &workspace_id, 14)
        .await
        .expect("aggregate lessons");
    assert_eq!(report.success_lessons, 1);
    assert_eq!(report.failure_lessons, 1);
    assert_eq!(report.blocked_lessons, 1);

    let lessons = app.state.db.raw().collection::<Document>("lessons_learned");
    for (kind, expected_run_id) in [
        ("success", "run_success"),
        ("reviewer_misjudge_negative", "run_failure"),
        ("blocked_by_safety_guard", "run_blocked"),
    ] {
        let lesson = lessons
            .find_one(
                doc! { "lesson_id": format!("{workspace_id}::{kind}") },
                None,
            )
            .await
            .expect("query lesson")
            .expect("lesson should be upserted");
        assert_eq!(lesson.get_i64("count").expect("count"), 1);
        assert_eq!(sample_ids(&lesson), vec![expected_run_id]);
    }
}
