//! Transaction regressions for taxonomy approval and guide application.
//! Requires Docker because MongoDB multi-document transactions need a replica set.

#![cfg(test)]

mod common;

use axum::Router;
use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use reqwest::StatusCode;
use tokio::net::TcpListener;
use wechatagent::auth::session::{
    authenticate, bootstrap_admin_if_needed, create_session,
};
use wechatagent::auth::SESSION_COOKIE_NAME;
use wechatagent::models::{
    AgentStatus, Contact, TaxonomyCandidate, TaxonomyEntry, TaxonomyValue,
    UserOperationGuidePreview,
};
use wechatagent::routes::api_router;

use crate::common::TestApp;

async fn start_api(app: &TestApp) -> (String, String, tokio::task::JoinHandle<()>) {
    let workspace_id = app.state.config.default_workspace_id.clone();
    bootstrap_admin_if_needed(
        &app.state.db,
        Some("transaction_test_admin"),
        Some("transaction-test-password"),
        Some(&workspace_id),
    )
    .await
    .expect("bootstrap admin");
    let admin = authenticate(
        &app.state.db,
        "transaction_test_admin",
        "transaction-test-password",
    )
    .await
    .expect("authenticate admin");
    let session = create_session(&app.state.db, &admin, 1, &workspace_id)
        .await
        .expect("create session");

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test API");
    let address = listener.local_addr().expect("test API address");
    let router = Router::new()
        .nest("/api", api_router(app.state.clone()))
        .with_state(app.state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve test API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

fn pending_candidate(workspace_id: &str, kind: &str, raw_value: &str) -> TaxonomyCandidate {
    TaxonomyCandidate {
        id: None,
        workspace_id: workspace_id.to_string(),
        scope: "global".to_string(),
        kind: kind.to_string(),
        raw_value: raw_value.to_string(),
        evidence: Some("transaction regression".to_string()),
        confidence: 8,
        first_seen_at: DateTime::now(),
        last_seen_at: DateTime::now(),
        occurrences: 1,
        status: "pending".to_string(),
        reviewed_at: None,
        reviewed_by: None,
        suggested_display_name: None,
    }
}

fn historical_taxonomy(
    workspace_id: &str,
    kind: &str,
    value_id: &str,
    version: i32,
) -> TaxonomyEntry {
    TaxonomyEntry {
        id: None,
        workspace_id: workspace_id.to_string(),
        scope: "global".to_string(),
        kind: kind.to_string(),
        value: TaxonomyValue {
            id: value_id.to_string(),
            display_name: "historical".to_string(),
            description: String::new(),
            aliases: Vec::new(),
            status: "deprecated".to_string(),
            priority_weight: None,
            is_terminal: false,
            is_reactivation_target: false,
        },
        updated_at: DateTime::now(),
        version,
        current_version: false,
        previous_version: None,
        seeded_by: Some("test".to_string()),
    }
}

fn managed_contact(
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
    playbook_id: ObjectId,
) -> Contact {
    Contact {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        wxid: wxid.to_string(),
        nickname: None,
        remark: None,
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: AgentStatus::Managed,
        human_profile_note: None,
        custom_agent_instructions: None,
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: None,
        playbook_id: Some(playbook_id),
        playbook_version: Some(1),
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
        operation_state: Some("new_contact".to_string()),
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

#[tokio::test]
#[ignore]
async fn taxonomy_approval_rolls_back_claim_when_dictionary_insert_fails() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let suffix = ObjectId::new().to_hex();
    let kind = format!("transaction_kind_{suffix}");
    let raw_value = format!("raw_{suffix}");
    let canonical_id = format!("canonical_{suffix}");

    let candidate_result = app
        .state
        .db
        .collection_taxonomy_candidates()
        .insert_one(pending_candidate(&workspace_id, &kind, &raw_value), None)
        .await
        .expect("insert candidate");
    let candidate_id = candidate_result
        .inserted_id
        .as_object_id()
        .expect("candidate id");
    app.state
        .db
        .collection_system_taxonomies()
        .insert_one(
            historical_taxonomy(&workspace_id, &kind, &canonical_id, i32::MAX),
            None,
        )
        .await
        .expect("insert conflicting historical taxonomy");

    let (base_url, cookie, server) = start_api(&app).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!(
            "{base_url}/admin/taxonomy-candidates/{candidate_id}/approve"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "canonicalValue": { "id": canonical_id, "label": "Canonical" }
        }))
        .send()
        .await
        .expect("approve request");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let after_failure = app
        .state
        .db
        .collection_taxonomy_candidates()
        .find_one(doc! { "_id": candidate_id }, None)
        .await
        .expect("load candidate")
        .expect("candidate exists");
    assert_eq!(after_failure.status, "pending");

    app.state
        .db
        .collection_system_taxonomies()
        .delete_many(
            doc! {
                "workspace_id": &workspace_id,
                "kind": &kind,
                "value.id": &canonical_id,
            },
            None,
        )
        .await
        .expect("remove conflict");
    let retry = client
        .post(format!(
            "{base_url}/admin/taxonomy-candidates/{candidate_id}/approve"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "canonicalValue": { "id": canonical_id, "label": "Canonical" }
        }))
        .send()
        .await
        .expect("retry approve request");
    assert_eq!(retry.status(), StatusCode::OK);

    let approved = app
        .state
        .db
        .collection_taxonomy_candidates()
        .find_one(doc! { "_id": candidate_id }, None)
        .await
        .expect("load approved candidate")
        .expect("candidate exists");
    assert_eq!(approved.status, "approved");
    let current_count = app
        .state
        .db
        .collection_system_taxonomies()
        .count_documents(
            doc! {
                "workspace_id": &workspace_id,
                "kind": &kind,
                "value.id": &canonical_id,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("count current taxonomy");
    assert_eq!(current_count, 1);
    server.abort();
}

#[tokio::test]
#[ignore]
async fn guide_apply_rolls_back_all_writes_and_retries_once() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let mut playbook = wechatagent::prompts::default_playbook(&workspace_id, &account_id);
    playbook.name = format!("transaction-playbook-{}", ObjectId::new().to_hex());
    playbook.is_default = false;
    let playbook_result = app
        .state
        .db
        .operation_playbooks()
        .insert_one(playbook, None)
        .await
        .expect("insert playbook");
    let playbook_id = playbook_result
        .inserted_id
        .as_object_id()
        .expect("playbook id");

    let wxid = format!("guide_transaction_{}", ObjectId::new().to_hex());
    let contact_result = app
        .state
        .db
        .contacts()
        .insert_one(
            managed_contact(&workspace_id, &account_id, &wxid, playbook_id),
            None,
        )
        .await
        .expect("insert contact");
    let contact_id = contact_result
        .inserted_id
        .as_object_id()
        .expect("contact id");
    let preview = UserOperationGuidePreview {
        id: None,
        workspace_id: workspace_id.clone(),
        account_id: account_id.clone(),
        contact_id,
        contact_wxid: wxid.clone(),
        instruction: "update contact and playbook".to_string(),
        mode: "smart".to_string(),
        status: "pending".to_string(),
        summary: "transaction regression".to_string(),
        impact_scope: "current_contact".to_string(),
        scope_reason: "test".to_string(),
        readable_changes: vec!["test".to_string()],
        health_scores: Document::new(),
        suggested_changes: doc! {
            "humanProfileNote": "committed note",
            "playbookPatch": { "replyStyle": "committed style" },
        },
        risk_warnings: Vec::new(),
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let preview_result = app
        .state
        .db
        .user_operation_guide_previews()
        .insert_one(preview, None)
        .await
        .expect("insert preview");
    let preview_id = preview_result
        .inserted_id
        .as_object_id()
        .expect("preview id");
    let dedupe_key = format!("guide_apply:{preview_id}");
    app.state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .insert_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "contact_wxid": &wxid,
                "kind": "test_conflict",
                "status": "succeeded",
                "summary": "force duplicate key",
                "created_at": DateTime::now(),
                "dedupe_key": &dedupe_key,
            },
            None,
        )
        .await
        .expect("insert conflicting event");

    let (base_url, cookie, server) = start_api(&app).await;
    let client = reqwest::Client::new();
    let apply_url = format!("{base_url}/user-operations/guide/apply");
    let first = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "previewId": preview_id.to_hex() }))
        .send()
        .await
        .expect("first guide apply");
    assert_eq!(first.status(), StatusCode::BAD_GATEWAY);

    let failed_contact = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load contact after rollback")
        .expect("contact exists");
    assert_eq!(failed_contact.human_profile_note, None);
    let failed_playbook = app
        .state
        .db
        .operation_playbooks()
        .find_one(doc! { "_id": playbook_id }, None)
        .await
        .expect("load playbook after rollback")
        .expect("playbook exists");
    assert_eq!(failed_playbook.version, 1);
    assert_ne!(failed_playbook.reply_style.as_deref(), Some("committed style"));
    let failed_preview = app
        .state
        .db
        .raw()
        .collection::<Document>("user_operation_guide_previews")
        .find_one(doc! { "_id": preview_id }, None)
        .await
        .expect("load failed preview")
        .expect("preview exists");
    assert_eq!(failed_preview.get_str("status").ok(), Some("failed"));
    assert_eq!(failed_preview.get_i32("apply_protocol_version").ok(), Some(2));

    app.state
        .db
        .raw()
        .collection::<Document>("agent_events")
        .delete_one(doc! { "dedupe_key": &dedupe_key }, None)
        .await
        .expect("remove event conflict");
    let retry = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "previewId": preview_id.to_hex() }))
        .send()
        .await
        .expect("retry guide apply");
    assert_eq!(retry.status(), StatusCode::OK);

    let committed_contact = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load committed contact")
        .expect("contact exists");
    assert_eq!(
        committed_contact.human_profile_note.as_deref(),
        Some("committed note")
    );
    let committed_playbook = app
        .state
        .db
        .operation_playbooks()
        .find_one(doc! { "_id": playbook_id }, None)
        .await
        .expect("load committed playbook")
        .expect("playbook exists");
    assert_eq!(committed_playbook.version, 2);
    assert_eq!(
        committed_playbook.reply_style.as_deref(),
        Some("committed style")
    );

    let replay = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "previewId": preview_id.to_hex() }))
        .send()
        .await
        .expect("replay guide apply");
    assert_eq!(replay.status(), StatusCode::CONFLICT);
    let final_playbook = app
        .state
        .db
        .operation_playbooks()
        .find_one(doc! { "_id": playbook_id }, None)
        .await
        .expect("load final playbook")
        .expect("playbook exists");
    assert_eq!(final_playbook.version, 2);
    server.abort();
}
