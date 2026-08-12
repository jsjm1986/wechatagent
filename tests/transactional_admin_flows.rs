//! Transaction regressions for taxonomy approval and guide application.
//! Requires Docker because MongoDB multi-document transactions need a replica set.

#![cfg(test)]

mod common;

use axum::{
    extract::{Extension, Json, State},
    Router,
};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use wechatagent::agent::handle_managed_message;
use wechatagent::auth::session::{authenticate, bootstrap_admin_if_needed, create_session};
use wechatagent::auth::{AuthenticatedAdmin, SESSION_COOKIE_NAME};
use wechatagent::models::{
    AgentStatus, Contact, ConversationMessage, GuideAuthoritativeChange, GuideFrozenPlan,
    MessageDirection, RelationshipTypeSuggestion, TaxonomyCandidate, TaxonomyEntry, TaxonomyValue,
    UserOperationGuidePreview,
};
use wechatagent::routes::api_router;
use wechatagent::routes::guide_profile::GenerateProfileRequest;

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
        axum::serve(listener, router).await.expect("serve test API");
    });
    (
        format!("http://{address}/api"),
        format!("{SESSION_COOKIE_NAME}={}", session.session_id),
        server,
    )
}

#[tokio::test]
#[ignore]
async fn operation_domain_reset_appends_version_and_preserves_history() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let domain = "user_operations";
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_domain_configs");
    let filter = doc! {
        "workspace_id": &workspace_id,
        "domain": domain,
    };

    let mut before_cursor = collection
        .find(
            filter.clone(),
            mongodb::options::FindOptions::builder()
                .sort(doc! { "version": 1_i32 })
                .build(),
        )
        .await
        .expect("list operation domain history before reset");
    let mut before = Vec::new();
    while let Some(row) = before_cursor
        .try_next()
        .await
        .expect("read operation domain history before reset")
    {
        before.push(row);
    }
    assert!(!before.is_empty(), "default operation domain must exist");
    let before_ids = before
        .iter()
        .map(|row| row.get_object_id("_id").expect("history ObjectId"))
        .collect::<Vec<_>>();
    let current_before = before
        .iter()
        .filter(|row| row.get_bool("current_version").unwrap_or(false))
        .collect::<Vec<_>>();
    assert_eq!(current_before.len(), 1, "precondition: exactly one current");
    let previous_version = current_before[0]
        .get_i32("version")
        .expect("current version");
    let max_version = before
        .iter()
        .map(|row| row.get_i32("version").expect("history version"))
        .max()
        .expect("max history version");

    let (base_url, cookie, server) = start_api(&app).await;
    let response = reqwest::Client::new()
        .post(format!("{base_url}/operation-domains/{domain}/reset"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .expect("reset operation domain request");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("reset response json");
    assert_eq!(body["ok"], true);
    assert_eq!(body["previousVersion"], previous_version);
    assert_eq!(body["version"], max_version + 1);
    let inserted_id =
        ObjectId::parse_str(body["id"].as_str().expect("reset id")).expect("reset ObjectId");

    let mut after_cursor = collection
        .find(
            filter,
            mongodb::options::FindOptions::builder()
                .sort(doc! { "version": 1_i32 })
                .build(),
        )
        .await
        .expect("list operation domain history after reset");
    let mut after = Vec::new();
    while let Some(row) = after_cursor
        .try_next()
        .await
        .expect("read operation domain history after reset")
    {
        after.push(row);
    }
    assert_eq!(after.len(), before.len() + 1, "reset must append once");
    for old_id in before_ids {
        assert!(
            after
                .iter()
                .any(|row| row.get_object_id("_id").ok() == Some(old_id)),
            "reset must preserve every historical row"
        );
    }
    let current_after = after
        .iter()
        .filter(|row| row.get_bool("current_version").unwrap_or(false))
        .collect::<Vec<_>>();
    assert_eq!(
        current_after.len(),
        1,
        "reset must leave exactly one current"
    );
    let inserted = current_after[0];
    assert_eq!(inserted.get_object_id("_id").ok(), Some(inserted_id));
    assert_eq!(inserted.get_i32("version").ok(), Some(max_version + 1));
    assert_eq!(
        inserted.get_i32("previous_version").ok(),
        Some(previous_version)
    );
    assert_eq!(
        inserted.get_str("seeded_by").ok(),
        Some("admin_reset:transaction_test_admin")
    );

    server.abort();
    app.cleanup().await;
}

#[test]
#[ignore]
fn domain_profile_dimension_kinds_reject_dynamic_paths_and_reserved_names() {
    for kind in [
        " customer_stage",
        "customer.stage",
        "$customer_stage",
        "CustomerStage",
        "客户阶段",
        "value_tier",
        "awaiting_principal_decision",
        "custom_updated_at",
    ] {
        assert!(
            wechatagent::models::validate_profile_dimension_kinds([kind]).is_err(),
            "unsafe dynamic dimension must be rejected: {kind:?}"
        );
    }
    assert!(wechatagent::models::validate_profile_dimension_kinds([
        "customer_stage",
        "parent_emotion_state",
        "subject2",
    ])
    .is_ok());
    assert!(
        wechatagent::models::validate_profile_dimension_kinds(["trust_level", "trust_level"])
            .is_err()
    );
}

#[tokio::test]
#[ignore]
async fn guide_unicode_keys_do_not_panic_and_candidate_lands_as_draft() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let profile_id = format!("unicode-guide-{}", ObjectId::new().to_hex());
    let mut generated =
        serde_json::to_value(wechatagent::agent::default_domain_profile(&workspace_id))
            .expect("serialize default profile fixture");
    let generated_object = generated.as_object_mut().expect("profile JSON object");
    generated_object.insert(
        "客户Stage".to_string(),
        serde_json::json!({ "nestedValue": true }),
    );
    generated_object.insert("éValue".to_string(), serde_json::json!("unicode"));
    app.llm.push_response(generated);

    let response = wechatagent::routes::guide_profile::generate_domain_profile_candidate(
        State(app.state.clone()),
        Extension(AuthenticatedAdmin {
            user_id: "transaction_test_admin".to_string(),
            username: "transaction_test_admin".to_string(),
            current_workspace: workspace_id.clone(),
        }),
        Json(GenerateProfileRequest {
            business_description: "Unicode key normalization regression".to_string(),
            profile_id: profile_id.clone(),
            display_name: Some("Unicode normalization".to_string()),
        }),
    )
    .await
    .expect("Unicode-prefixed generated keys must not panic or reject the candidate");
    let id = ObjectId::parse_str(response.0["id"].as_str().expect("candidate id"))
        .expect("candidate ObjectId");
    let candidate = app
        .state
        .db
        .domain_profiles()
        .find_one(doc! { "_id": id }, None)
        .await
        .expect("load generated candidate")
        .expect("generated candidate exists");
    assert_eq!(candidate.profile_id, profile_id);
    assert_eq!(candidate.release_status, "draft");
    assert!(!candidate.current_version);
    assert!(!candidate.is_active);
    assert_eq!(candidate.seeded_by.as_deref(), Some("generated_by_ai"));
    assert_eq!(app.llm.calls(), 1);

    app.cleanup().await;
}

fn hc015_reply_decision(raw_stage: &str) -> serde_json::Value {
    serde_json::json!({
        "decisionPhase": "final",
        "userUnderstanding": "客户正在表达当前需求，并给出了足够清晰的下一步对话上下文。",
        "relationshipRead": "对话氛围稳定，客户愿意继续交流，关系处于正常推进阶段。",
        "operationGoal": "承接客户当前关注点并确认下一步，不制造额外压力。",
        "knowledgeNeedReason": "本轮不涉及产品能力、价格或效果承诺，无需知识检索。",
        "memoryUpdateReason": "本轮没有需要进入长期记忆的新事实。",
        "selfCritique": "保持简洁，只确认客户当前关注点。",
        "whyShouldReply": "客户主动延续对话，简短回应有助于维持交流。",
        "whySkipReply": "",
        "riskSelfCheck": "回复不包含事实声明、承诺、隐私或越界内容。",
        "riskLevel": "medium",
        "knowledgeNeed": "not_required",
        "runMode": "fast_chat",
        "autonomyMode": "auto",
        "needsReview": true,
        "consolidationNeeded": false,
        "operationState": "new_contact",
        "customerStage": raw_stage,
        "shouldReply": true,
        "replyText": "收到，我们可以先按你最关心的点继续聊。",
        "usedKnowledgeIds": [],
        "conversationMode": "consultative",
        "conversationModeReason": "客户正在明确需求，使用顾问式承接。",
        "bayesianObservations": [
            {
                "dimension": "预算敏感度",
                "value": "高",
                "confidence": 0.4,
                "evidenceTurns": [0]
            },
            {
                "dimension": "预算敏感度",
                "value": "高",
                "confidence": 0.9,
                "evidenceTurns": [0]
            }
        ]
    })
}

fn hc015_review_pass() -> serde_json::Value {
    serde_json::json!({
        "approved": true,
        "scores": {
            "humanLike": 8,
            "emotionalValue": 8,
            "productAccuracy": 8,
            "relationshipProgress": 7,
            "conversionReadiness": 6,
            "pressureRisk": 2,
            "boundaryPrivacySafety": 9,
            "factRisk": 1
        },
        "claimAnalysis": {
            "hasProductClaim": false,
            "requiresProductKnowledge": false,
            "knowledgeSupported": true,
            "reason": "候选回复不包含可核验的产品或业务声明。"
        },
        "risks": [],
        "rewriteInstruction": "",
        "reviewSummary": "回复自然且没有事实或安全风险，可以放行。",
        "needsRevision": false,
        "revisionDirection": "",
        "shouldHold": false,
        "holdReason": "",
        "holdCategory": "",
        "selfCritiqueAddressed": true
    })
}

#[tokio::test]
#[ignore]
async fn hc015_gateway_writes_one_candidate_and_one_bayesian_point_per_run() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let suffix = ObjectId::new().to_hex();
    let wxid = format!("hc015_gateway_{suffix}");
    let raw_stage = format!("unknown_stage_{suffix}");

    let mut contact = managed_contact(&workspace_id, &account_id, &wxid, ObjectId::new());
    contact.id = Some(ObjectId::new());
    contact.playbook_id = None;
    contact.playbook_version = None;
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert HC-015 managed contact");

    let inbound = ConversationMessage {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.clone(),
        account_id: account_id.clone(),
        contact_wxid: wxid.clone(),
        message_id: Some(format!("hc015-message-{suffix}")),
        dedupe_key: None,
        direction: MessageDirection::Inbound,
        content: "我想先聊聊预算和下一步。".to_string(),
        msg_type: None,
        media_ref: None,
        raw: None,
        is_synthetic_relay: false,
        created_at: DateTime::now(),
    };
    app.state
        .db
        .messages()
        .insert_one(&inbound, None)
        .await
        .expect("insert HC-015 inbound message");

    app.llm.push_response(hc015_reply_decision(&raw_stage));
    app.llm.push_response(hc015_review_pass());
    app.llm
        .push_response(common::independent_claim_gate_pass_json());

    handle_managed_message(&app.state, contact.clone(), &inbound)
        .await
        .expect("run HC-015 gateway");
    assert_eq!(app.llm.calls(), 3);
    let projected_run_id = common::complete_latest_post_decision(
        &app,
        &workspace_id,
        &account_id,
        &wxid,
        serde_json::json!({
            "customerStage": &raw_stage,
            "domainSignals": { "customer_stage": &raw_stage },
            "bayesianObservations": [
                {
                    "dimension": "预算敏感度",
                    "value": "高",
                    "confidence": 0.4,
                    "evidenceTurns": [0]
                },
                {
                    "dimension": "预算敏感度",
                    "value": "高",
                    "confidence": 0.9,
                    "evidenceTurns": [0]
                }
            ]
        }),
    )
    .await;

    let candidate = app
        .state
        .db
        .collection_taxonomy_candidates()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "scope": &account_id,
                "kind": "customer_stage",
                "raw_value": &raw_stage,
            },
            None,
        )
        .await
        .expect("load HC-015 taxonomy candidate")
        .expect("unknown stage must create a taxonomy candidate");
    assert_eq!(candidate.status, "pending");
    assert_eq!(
        candidate.occurrences, 1,
        "one run must write one occurrence"
    );

    let reloaded = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact.id }, None)
        .await
        .expect("reload HC-015 contact")
        .expect("HC-015 contact exists");
    let signal = reloaded
        .bayesian_signals
        .iter()
        .find(|signal| signal.dimension == "预算敏感度")
        .expect("Bayesian signal persisted");
    assert_eq!(
        signal.history.len(),
        1,
        "same-run duplicates collapse to one point"
    );
    assert_eq!(signal.current_value, "高");
    assert_eq!(signal.current_confidence, 0.9);

    let run = app
        .state
        .db
        .agent_run_logs()
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "account_id": &account_id,
                "contact_wxid": &wxid,
            },
            None,
        )
        .await
        .expect("load HC-015 run")
        .expect("HC-015 run exists");
    assert_eq!(
        signal.history[0].source_run_id.as_deref(),
        Some(projected_run_id.as_str()),
        "Bayesian point must carry the producing run id"
    );
    assert_eq!(run.run_id, projected_run_id);

    app.cleanup().await;
}

fn hc015_taxonomy_row(
    workspace_id: &str,
    kind: &str,
    canonical_id: &str,
    aliases: &[&str],
    version: i32,
    current: bool,
) -> Document {
    doc! {
        "_id": ObjectId::new(),
        "workspace_id": workspace_id,
        "scope": "global",
        "kind": kind,
        "value": {
            "id": canonical_id,
            "displayName": canonical_id,
            "description": "HC-015 migration fixture",
            "aliases": aliases,
            "status": if current { "active" } else { "deprecated" },
            "priorityWeight": 0_i32,
            "isTerminal": false,
            "isReactivationTarget": false,
        },
        "updated_at": DateTime::now(),
        "version": version,
        "current_version": current,
        "previous_version": Bson::Null,
        "seeded_by": "hc015_test",
    }
}

#[tokio::test]
#[ignore]
async fn hc015_m050_backfills_history_fails_before_write_and_multikey_rejects_conflict() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let collection = app
        .state
        .db
        .raw()
        .collection::<Document>("system_taxonomies");
    let suffix = ObjectId::new().to_hex();

    let backfill_kind = format!("hc015_backfill_{suffix}");
    let current = hc015_taxonomy_row(
        &workspace_id,
        &backfill_kind,
        "canonical",
        &["current-alias"],
        2,
        true,
    );
    let current_id = current.get_object_id("_id").expect("current id");
    let historical = hc015_taxonomy_row(
        &workspace_id,
        &backfill_kind,
        "canonical",
        &["historical-alias"],
        1,
        false,
    );
    let historical_id = historical.get_object_id("_id").expect("historical id");
    collection
        .insert_many([current, historical], None)
        .await
        .expect("insert m050 backfill fixtures");

    wechatagent::db::migrations::m050_taxonomy_identity_claims::run_step(&app.state.db)
        .await
        .expect("run m050 on valid fixtures");
    for (id, expected) in [
        (current_id, vec!["canonical", "current-alias"]),
        (historical_id, vec!["canonical", "historical-alias"]),
    ] {
        let row = collection
            .find_one(doc! { "_id": id }, None)
            .await
            .expect("load backfilled taxonomy")
            .expect("backfilled taxonomy exists");
        let claims = row
            .get_document("value")
            .expect("value")
            .get_array("identityClaims")
            .expect("identityClaims")
            .iter()
            .map(|value| value.as_str().expect("claim string"))
            .collect::<Vec<_>>();
        assert_eq!(claims, expected);
    }

    let conflict_kind = format!("hc015_audit_{suffix}");
    collection
        .drop_index("uniq_sys_tax_ws_scope_kind_active_identity", None)
        .await
        .expect("temporarily drop active identity index for legacy conflict fixture");
    let sentinel = hc015_taxonomy_row(
        &workspace_id,
        &format!("hc015_sentinel_{suffix}"),
        "sentinel",
        &["untouched"],
        1,
        false,
    );
    let sentinel_id = sentinel.get_object_id("_id").expect("sentinel id");
    let first = hc015_taxonomy_row(&workspace_id, &conflict_kind, "first", &["shared"], 1, true);
    let second = hc015_taxonomy_row(
        &workspace_id,
        &conflict_kind,
        "second",
        &["shared"],
        1,
        true,
    );
    collection
        .insert_many([sentinel, first, second], None)
        .await
        .expect("insert m050 conflict fixtures without derived claims");
    let error = wechatagent::db::migrations::m050_taxonomy_identity_claims::run_step(&app.state.db)
        .await
        .expect_err("m050 must reject ambiguous active claims");
    assert!(error.to_string().contains("ambiguous active claim shared"));
    let sentinel_after = collection
        .find_one(doc! { "_id": sentinel_id }, None)
        .await
        .expect("load sentinel after failed m050")
        .expect("sentinel exists");
    assert!(
        !sentinel_after
            .get_document("value")
            .expect("sentinel value")
            .contains_key("identityClaims"),
        "full audit must fail before the first write"
    );

    collection
        .delete_many(doc! { "kind": &conflict_kind }, None)
        .await
        .expect("remove ambiguous fixtures");
    wechatagent::db::migrations::m050_taxonomy_identity_claims::run_step(&app.state.db)
        .await
        .expect("backfill sentinel after removing ambiguous legacy rows");
    app.state
        .db
        .ensure_indexes()
        .await
        .expect("restore active identity unique index");
    let index_kind = format!("hc015_index_{suffix}");
    let mut owner = hc015_taxonomy_row(
        &workspace_id,
        &index_kind,
        "owner-a",
        &["shared-index-claim"],
        1,
        true,
    );
    owner
        .get_document_mut("value")
        .expect("owner value")
        .insert("identityClaims", vec!["owner-a", "shared-index-claim"]);
    collection
        .insert_one(owner, None)
        .await
        .expect("insert first active identity owner");
    let mut contender = hc015_taxonomy_row(
        &workspace_id,
        &index_kind,
        "owner-b",
        &["shared-index-claim"],
        1,
        true,
    );
    contender
        .get_document_mut("value")
        .expect("contender value")
        .insert("identityClaims", vec!["owner-b", "shared-index-claim"]);
    let duplicate = collection
        .insert_one(contender, None)
        .await
        .expect_err("unique multikey index must reject a second active owner");
    let duplicate_text = duplicate.to_string().to_ascii_lowercase();
    assert!(
        duplicate_text.contains("e11000") || duplicate_text.contains("duplicate key"),
        "expected DuplicateKey, got {duplicate}"
    );

    app.cleanup().await;
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

fn guide_candidate_hash(
    workspace_id: &str,
    account_id: &str,
    contact_id: ObjectId,
    plan: &GuideFrozenPlan,
) -> String {
    let bytes = mongodb::bson::to_vec(&doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "contact_id": contact_id,
        "frozen_plan": mongodb::bson::to_bson(plan).expect("serialize frozen plan"),
    })
    .expect("serialize candidate envelope");
    hex::encode(Sha256::digest(bytes))
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
            "canonicalValue": { "id": canonical_id, "label": "Canonical" },
            "reviewedBy": "spoofed@attacker.invalid"
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
            "canonicalValue": { "id": canonical_id, "label": "Canonical" },
            "reviewedBy": "spoofed@attacker.invalid"
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
    assert_eq!(
        approved.reviewed_by.as_deref(),
        Some("transaction_test_admin"),
        "SR-058: taxonomy reviewedBy 必须来自认证会话"
    );
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
async fn taxonomy_candidate_merge_appends_alias_version_and_preserves_runtime_fields() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let suffix = ObjectId::new().to_hex();
    let kind = format!("taxonomy_merge_kind_{suffix}");
    let raw_value = format!("raw_alias_{suffix}");
    let canonical_id = format!("canonical_{suffix}");
    let taxonomies = app.state.db.collection_system_taxonomies();

    let mut current = historical_taxonomy(&workspace_id, &kind, &canonical_id, 3);
    current.value.display_name = "Stable canonical label".to_string();
    current.value.description = "Stable operator description".to_string();
    current.value.aliases = vec!["existing-alias".to_string()];
    current.value.status = "active".to_string();
    current.value.priority_weight = Some(42);
    current.value.is_terminal = true;
    current.value.is_reactivation_target = true;
    current.current_version = true;
    current.previous_version = Some(2);
    current.seeded_by = Some("operator".to_string());
    taxonomies
        .insert_one(current, None)
        .await
        .expect("insert current canonical taxonomy");

    let mut historical = historical_taxonomy(&workspace_id, &kind, &canonical_id, 9);
    historical.previous_version = Some(8);
    taxonomies
        .insert_one(historical, None)
        .await
        .expect("insert higher historical taxonomy version");

    let candidate_result = app
        .state
        .db
        .collection_taxonomy_candidates()
        .insert_one(pending_candidate(&workspace_id, &kind, &raw_value), None)
        .await
        .expect("insert merge candidate");
    let candidate_id = candidate_result
        .inserted_id
        .as_object_id()
        .expect("merge candidate id");

    let (base_url, cookie, server) = start_api(&app).await;
    let response = reqwest::Client::new()
        .post(format!(
            "{base_url}/admin/taxonomy-candidates/{candidate_id}/approve"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "canonicalValue": {
                "id": canonical_id,
                "label": "Attempted overwrite",
                "description": "Attempted description overwrite",
                "aliases": ["manual-alias", "existing-alias"]
            }
        }))
        .send()
        .await
        .expect("merge candidate response");
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.expect("merge response json");
    assert_eq!(body["mergedIntoExisting"], true);

    let approved = app
        .state
        .db
        .collection_taxonomy_candidates()
        .find_one(doc! { "_id": candidate_id }, None)
        .await
        .expect("load merged candidate")
        .expect("merged candidate exists");
    assert_eq!(approved.status, "approved");
    assert_eq!(
        approved.reviewed_by.as_deref(),
        Some("transaction_test_admin")
    );

    let merged = taxonomies
        .find_one(
            doc! {
                "workspace_id": &workspace_id,
                "scope": "global",
                "kind": &kind,
                "value.id": &canonical_id,
                "current_version": true,
            },
            None,
        )
        .await
        .expect("load merged current taxonomy")
        .expect("merged current taxonomy exists");
    assert_eq!(merged.version, 10, "new version must follow history max");
    assert_eq!(
        merged.previous_version,
        Some(3),
        "lineage must point to the actual retired current version"
    );
    assert_eq!(merged.value.display_name, "Stable canonical label");
    assert_eq!(merged.value.description, "Stable operator description");
    assert_eq!(merged.value.status, "active");
    assert_eq!(merged.value.priority_weight, Some(42));
    assert!(merged.value.is_terminal);
    assert!(merged.value.is_reactivation_target);
    assert_eq!(
        merged.value.aliases,
        vec![
            "existing-alias".to_string(),
            "manual-alias".to_string(),
            raw_value.clone(),
        ]
    );
    assert_eq!(
        taxonomies
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "scope": "global",
                    "kind": &kind,
                    "value.id": &canonical_id,
                    "current_version": true,
                },
                None,
            )
            .await
            .expect("count current taxonomy versions"),
        1
    );
    server.abort();
}

#[tokio::test]
#[ignore]
async fn relationship_review_ignores_spoofed_actor_and_uses_authenticated_admin() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let wxid = format!("relationship_actor_{}", ObjectId::new().to_hex());
    let contact_result = app
        .state
        .db
        .contacts()
        .insert_one(
            managed_contact(&workspace_id, &account_id, &wxid, ObjectId::new()),
            None,
        )
        .await
        .expect("insert relationship contact");
    let contact_id = contact_result
        .inserted_id
        .as_object_id()
        .expect("relationship contact id");
    let now = DateTime::now();
    let suggestion_result = app
        .state
        .db
        .collection_relationship_type_suggestions()
        .insert_one(
            RelationshipTypeSuggestion {
                id: None,
                workspace_id: workspace_id.clone(),
                account_id: account_id.clone(),
                contact_id: contact_id.to_hex(),
                suggested_value: "peer".to_string(),
                evidence: Some("actor regression".to_string()),
                confidence: 9,
                status: "pending".to_string(),
                occurrences: 1,
                first_seen_at: now,
                last_seen_at: now,
                reviewed_at: None,
                reviewed_by: None,
            },
            None,
        )
        .await
        .expect("insert relationship suggestion");
    let suggestion_id = suggestion_result
        .inserted_id
        .as_object_id()
        .expect("relationship suggestion id");

    let (base_url, cookie, server) = start_api(&app).await;
    let response = reqwest::Client::new()
        .post(format!(
            "{base_url}/admin/relationship-type-suggestions/{suggestion_id}/approve"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "reviewedBy": "spoofed@attacker.invalid"
        }))
        .send()
        .await
        .expect("approve relationship suggestion");
    assert_eq!(response.status(), StatusCode::OK);

    let approved = app
        .state
        .db
        .collection_relationship_type_suggestions()
        .find_one(doc! { "_id": suggestion_id }, None)
        .await
        .expect("load relationship suggestion")
        .expect("relationship suggestion exists");
    assert_eq!(approved.status, "approved");
    assert_eq!(
        approved.reviewed_by.as_deref(),
        Some("transaction_test_admin"),
        "SR-058: relationship reviewedBy 必须来自认证会话"
    );

    let contact_after_first_review = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load relationship contact")
        .expect("relationship contact exists");
    assert_eq!(
        contact_after_first_review
            .domain_attributes
            .as_ref()
            .and_then(|attributes| attributes.get_str("relationship_type").ok()),
        Some("peer"),
        "SR-059: successful review must commit contact and terminal status together"
    );

    // SR-060: the approved history does not occupy the pending slot. A new evidence cycle for
    // the same contact can be inserted, while a second pending row in that cycle is rejected.
    let next_seen = DateTime::now();
    let next_result = app
        .state
        .db
        .collection_relationship_type_suggestions()
        .insert_one(
            RelationshipTypeSuggestion {
                id: None,
                workspace_id: workspace_id.clone(),
                account_id: account_id.clone(),
                contact_id: contact_id.to_hex(),
                suggested_value: "friend".to_string(),
                evidence: Some("new evidence cycle".to_string()),
                confidence: 8,
                status: "pending".to_string(),
                occurrences: 1,
                first_seen_at: next_seen,
                last_seen_at: next_seen,
                reviewed_at: None,
                reviewed_by: None,
            },
            None,
        )
        .await
        .expect("terminal history must not block next pending cycle");
    let next_id = next_result
        .inserted_id
        .as_object_id()
        .expect("next relationship suggestion id");
    let duplicate_pending = app
        .state
        .db
        .collection_relationship_type_suggestions()
        .insert_one(
            RelationshipTypeSuggestion {
                id: None,
                workspace_id: workspace_id.clone(),
                account_id: account_id.clone(),
                contact_id: contact_id.to_hex(),
                suggested_value: "customer".to_string(),
                evidence: Some("competing pending evidence".to_string()),
                confidence: 7,
                status: "pending".to_string(),
                occurrences: 1,
                first_seen_at: DateTime::now(),
                last_seen_at: DateTime::now(),
                reviewed_at: None,
                reviewed_by: None,
            },
            None,
        )
        .await
        .expect_err("partial unique index must allow only one pending cycle per contact");
    assert!(
        duplicate_pending.to_string().contains("E11000"),
        "expected duplicate-key rejection, got {duplicate_pending}"
    );

    // Force the second transaction's contact update to fail after its suggestion CAS. Mongo must
    // roll both operations back, preserving the first cycle's applied relationship value.
    app.state
        .db
        .raw()
        .run_command(
            doc! {
                "collMod": "contacts",
                "validator": {
                    "domain_attributes.relationship_type": { "$ne": "friend" }
                },
                "validationAction": "error",
            },
            None,
        )
        .await
        .expect("install relationship contact rejection validator");
    let failed = reqwest::Client::new()
        .post(format!(
            "{base_url}/admin/relationship-type-suggestions/{next_id}/approve"
        ))
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("failed relationship approval response");
    assert_eq!(failed.status(), StatusCode::BAD_GATEWAY);

    let still_pending = app
        .state
        .db
        .collection_relationship_type_suggestions()
        .find_one(doc! { "_id": next_id }, None)
        .await
        .expect("load rolled-back relationship suggestion")
        .expect("rolled-back relationship suggestion exists");
    assert_eq!(
        still_pending.status, "pending",
        "SR-059: contact write failure must roll suggestion CAS back"
    );
    assert!(still_pending.reviewed_at.is_none());
    assert!(still_pending.reviewed_by.is_none());
    let contact_after_failure = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load contact after rollback")
        .expect("contact after rollback exists");
    assert_eq!(
        contact_after_failure
            .domain_attributes
            .as_ref()
            .and_then(|attributes| attributes.get_str("relationship_type").ok()),
        Some("peer"),
        "SR-059: failed second review must preserve the previously committed profile"
    );
    assert_eq!(
        app.state
            .db
            .collection_relationship_type_suggestions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "contact_id": contact_id.to_hex() },
                None,
            )
            .await
            .expect("count relationship review history"),
        2,
        "one terminal history row and one pending cycle must coexist"
    );
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
    let contact = app
        .state
        .db
        .contacts()
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load guide contact")
        .expect("guide contact exists");
    let memory = wechatagent::agent::load_or_create_operating_memory(&app.state, &contact)
        .await
        .expect("create guide operating memory");
    let frozen_plan = GuideFrozenPlan {
        contact_updated_at: contact.updated_at,
        memory_updated_at: memory.updated_at,
        memory_insert: None,
        playbook_id: Some(playbook_id),
        playbook_version: Some(1),
        domain_config_id: None,
        domain_version: None,
        domain_updated_at: None,
        contact_set: doc! { "human_profile_note": "committed note" },
        contact_timestamp_fields: Vec::new(),
        memory_set: Document::new(),
        memory_timestamp_fields: Vec::new(),
        playbook_set: doc! { "reply_style": "committed style" },
        playbook_timestamp_fields: Vec::new(),
        domain_runtime_parameters: None,
        applied_fields: vec!["humanProfileNote".to_string(), "playbookPatch".to_string()],
        skipped_fields: Vec::new(),
        authoritative_changes: vec![
            GuideAuthoritativeChange {
                target: "contact".to_string(),
                field: "human_profile_note".to_string(),
                label: "human_profile_note".to_string(),
                before: mongodb::bson::Bson::Null,
                after: mongodb::bson::Bson::String("committed note".to_string()),
            },
            GuideAuthoritativeChange {
                target: "playbook".to_string(),
                field: "reply_style".to_string(),
                label: "reply_style".to_string(),
                before: mongodb::bson::Bson::Null,
                after: mongodb::bson::Bson::String("committed style".to_string()),
            },
        ],
        playbook_affected_contacts: 1,
    };
    let candidate_hash = guide_candidate_hash(&workspace_id, &account_id, contact_id, &frozen_plan);
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
        impact_scope: "shared_playbook".to_string(),
        scope_reason: "shared playbook regression".to_string(),
        readable_changes: vec!["test".to_string()],
        health_scores: Document::new(),
        suggested_changes: doc! {
            "humanProfileNote": "committed note",
            "playbookPatch": { "replyStyle": "committed style" },
        },
        risk_warnings: Vec::new(),
        frozen_plan: Some(frozen_plan),
        candidate_hash: Some(candidate_hash.clone()),
        apply_receipt: None,
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

    let raw_previews = app
        .state
        .db
        .raw()
        .collection::<Document>("user_operation_guide_previews");
    let raw_contacts = app.state.db.raw().collection::<Document>("contacts");
    let raw_playbooks = app
        .state
        .db
        .raw()
        .collection::<Document>("operation_playbooks");
    let before_wrong_preview = raw_previews
        .find_one(doc! { "_id": preview_id }, None)
        .await
        .expect("load preview before wrong identity")
        .expect("preview exists");
    let before_wrong_contact = raw_contacts
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load contact before wrong identity")
        .expect("contact exists");
    let before_wrong_playbook = raw_playbooks
        .find_one(doc! { "_id": playbook_id }, None)
        .await
        .expect("load playbook before wrong identity")
        .expect("playbook exists");
    let before_wrong_memories = app
        .state
        .db
        .operating_memories()
        .count_documents(doc! { "workspace_id": &workspace_id }, None)
        .await
        .expect("count memories before wrong identity");
    let before_wrong_tasks = app
        .state
        .db
        .tasks()
        .count_documents(doc! { "workspace_id": &workspace_id }, None)
        .await
        .expect("count tasks before wrong identity");
    let before_wrong_events = app
        .state
        .db
        .events()
        .count_documents(doc! { "workspace_id": &workspace_id }, None)
        .await
        .expect("count events before wrong identity");

    let (base_url, cookie, server) = start_api(&app).await;
    let client = reqwest::Client::new();
    let apply_url = format!("{base_url}/user-operations/guide/apply");

    // SR-150: the caller's current account/contact identity is part of the atomic lease.
    // A stale preview confirmation must not even move the preview out of pending.
    let wrong_identity = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "previewId": preview_id.to_hex(),
            "expectedAccountId": "other-account",
            "expectedContactId": contact_id.to_hex(),
            "candidateHash": &candidate_hash,
            "confirmGlobalImpact": true,
        }))
        .send()
        .await
        .expect("wrong-identity guide apply");
    assert_eq!(wrong_identity.status(), StatusCode::CONFLICT);
    let after_wrong_preview = raw_previews
        .find_one(doc! { "_id": preview_id }, None)
        .await
        .expect("load preview after wrong identity")
        .expect("preview exists");
    let after_wrong_contact = raw_contacts
        .find_one(doc! { "_id": contact_id }, None)
        .await
        .expect("load contact after wrong identity")
        .expect("contact exists");
    let after_wrong_playbook = raw_playbooks
        .find_one(doc! { "_id": playbook_id }, None)
        .await
        .expect("load playbook after wrong identity")
        .expect("playbook exists");
    assert_eq!(after_wrong_preview, before_wrong_preview);
    assert_eq!(after_wrong_contact, before_wrong_contact);
    assert_eq!(after_wrong_playbook, before_wrong_playbook);
    assert_eq!(
        app.state
            .db
            .operating_memories()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .expect("count memories after wrong identity"),
        before_wrong_memories,
    );
    assert_eq!(
        app.state
            .db
            .tasks()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .expect("count tasks after wrong identity"),
        before_wrong_tasks,
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .expect("count events after wrong identity"),
        before_wrong_events,
        "wrong identity must be zero-write before lease acquisition",
    );

    let wrong_hash = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "previewId": preview_id.to_hex(),
            "expectedAccountId": &account_id,
            "expectedContactId": contact_id.to_hex(),
            "candidateHash": "tampered-candidate-hash",
            "confirmGlobalImpact": true,
        }))
        .send()
        .await
        .expect("wrong-hash guide apply");
    assert_eq!(wrong_hash.status(), StatusCode::CONFLICT);

    let missing_confirmation = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "previewId": preview_id.to_hex(),
            "expectedAccountId": &account_id,
            "expectedContactId": contact_id.to_hex(),
            "candidateHash": &candidate_hash,
            "confirmGlobalImpact": false,
        }))
        .send()
        .await
        .expect("missing-confirmation guide apply");
    assert_eq!(missing_confirmation.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        raw_previews
            .find_one(doc! { "_id": preview_id }, None)
            .await
            .expect("load preview after pre-claim rejections")
            .expect("preview exists"),
        before_wrong_preview,
        "identity/hash/confirmation rejection must not claim or mutate the preview",
    );

    let apply_body = serde_json::json!({
        "previewId": preview_id.to_hex(),
        "expectedAccountId": &account_id,
        "expectedContactId": contact_id.to_hex(),
        "candidateHash": &candidate_hash,
        "confirmGlobalImpact": true,
    });
    let first = client
        .post(&apply_url)
        .header(reqwest::header::COOKIE, &cookie)
        .json(&apply_body)
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
    assert_ne!(
        failed_playbook.reply_style.as_deref(),
        Some("committed style")
    );
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
    assert_eq!(
        failed_preview.get_i32("apply_protocol_version").ok(),
        Some(3)
    );

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
        .json(&apply_body)
        .send()
        .await
        .expect("retry guide apply");
    assert_eq!(retry.status(), StatusCode::OK);
    let retry_body: serde_json::Value = retry.json().await.expect("retry receipt json");
    assert_eq!(retry_body["item"]["committed"], true);
    assert_eq!(retry_body["item"]["previewId"], preview_id.to_hex());
    assert_eq!(retry_body["item"]["candidateHash"], candidate_hash);
    assert_eq!(retry_body["item"]["impactScope"], "shared_playbook");

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
        .json(&apply_body)
        .send()
        .await
        .expect("replay guide apply");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body: serde_json::Value = replay.json().await.expect("replay receipt json");
    assert_eq!(
        replay_body, retry_body,
        "replay must return the stable receipt"
    );
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
