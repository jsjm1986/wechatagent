//! SR-121 digest snapshot recovery redlines.
//!
//! These tests call the production attempt claim/finalize primitives. MongoDB
//! is required; no LLM behavior is involved.

#![cfg(test)]

mod common;

use mongodb::bson::{doc, oid::ObjectId};
use wechatagent::knowledge_digest::{
    claim_digest_attempt_for_redline, finalize_digest_attempt_for_redline,
};
use wechatagent::models::KnowledgeDigestCard;

use crate::common::TestApp;

fn card(title: &str) -> KnowledgeDigestCard {
    KnowledgeDigestCard {
        card_id: ObjectId::new(),
        kind: "chunk_missing_field".to_string(),
        title: title.to_string(),
        summary: format!("summary for {title}"),
        target_refs: vec![doc! { "kind": "chunk", "id": title }],
        suggested_action: "fix_chunk".to_string(),
        severity: "warn".to_string(),
        metric: None,
    }
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn failed_regeneration_preserves_last_successful_snapshot() {
    let app = TestApp::start().await;
    let workspace_id = format!("sr121-{}", ObjectId::new().to_hex());
    let account_id = "account-a";
    let report_date = "2026-07-26";

    let first =
        claim_digest_attempt_for_redline(&app.state, &workspace_id, account_id, report_date)
            .await
            .expect("claim successful attempt");
    finalize_digest_attempt_for_redline(
        &app.state,
        &workspace_id,
        account_id,
        report_date,
        first,
        "ok",
        None,
        vec![card("committed-card")],
    )
    .await
    .expect("commit successful snapshot");

    let failed =
        claim_digest_attempt_for_redline(&app.state, &workspace_id, account_id, report_date)
            .await
            .expect("claim failed regeneration");
    let visible = finalize_digest_attempt_for_redline(
        &app.state,
        &workspace_id,
        account_id,
        report_date,
        failed,
        "failed",
        Some("upstream_timeout".to_string()),
        vec![],
    )
    .await
    .expect("record failed regeneration");

    assert_eq!(visible.status, "ok");
    assert_eq!(visible.cards.len(), 1);
    assert_eq!(visible.cards[0].title, "committed-card");
    assert_eq!(visible.current_generation, first);
    assert_eq!(visible.attempt_generation, failed);
    assert_eq!(visible.latest_attempt_status.as_deref(), Some("failed"));
    assert_eq!(
        visible.latest_attempt_error_kind.as_deref(),
        Some("upstream_timeout")
    );
    assert!(visible.last_success_at.is_some());
    app.cleanup().await;
}

#[tokio::test]
#[ignore = "requires MongoDB / testcontainers"]
async fn late_generation_cannot_overwrite_newer_success() {
    let app = TestApp::start().await;
    let workspace_id = format!("sr121-{}", ObjectId::new().to_hex());
    let account_id = "account-a";
    let report_date = "2026-07-27";

    let old = claim_digest_attempt_for_redline(&app.state, &workspace_id, account_id, report_date)
        .await
        .expect("claim old attempt");
    let current =
        claim_digest_attempt_for_redline(&app.state, &workspace_id, account_id, report_date)
            .await
            .expect("claim current attempt");
    assert!(current > old);

    finalize_digest_attempt_for_redline(
        &app.state,
        &workspace_id,
        account_id,
        report_date,
        current,
        "ok",
        None,
        vec![card("new-generation")],
    )
    .await
    .expect("commit current attempt");
    let authoritative = finalize_digest_attempt_for_redline(
        &app.state,
        &workspace_id,
        account_id,
        report_date,
        old,
        "ok",
        None,
        vec![card("stale-generation")],
    )
    .await
    .expect("late attempt reads authoritative snapshot");

    assert_eq!(authoritative.current_generation, current);
    assert_eq!(authoritative.cards.len(), 1);
    assert_eq!(authoritative.cards[0].title, "new-generation");
    let persisted = app
        .state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": account_id,
                "report_date": report_date,
            },
            None,
        )
        .await
        .expect("load report")
        .expect("report exists");
    assert_eq!(persisted.current_generation, current);
    assert_eq!(persisted.cards[0].title, "new-generation");
    app.cleanup().await;
}
