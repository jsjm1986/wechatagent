//! SR-029 durable memory-consolidation commit redlines.
//!
//! These tests use the production manual-task entry and production commit
//! reconciler. They deliberately persist crash-window snapshots instead of
//! copying the reconciler's filters.

mod common;

use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use serde_json::json;
use wechatagent::agent::{reconcile_memory_consolidation_commit, run_manual_memory_consolidation};
use wechatagent::error::AppError;
use wechatagent::models::{AgentStatus, Contact, MemoryCandidate};

fn contact(ws: &str, account: &str, wxid: &str) -> Contact {
    let now = DateTime::now();
    Contact {
        id: Some(ObjectId::new()),
        workspace_id: ws.to_string(),
        account_id: account.to_string(),
        wxid: wxid.to_string(),
        nickname: Some("SR-029 contact".to_string()),
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

fn candidate(contact: &Contact) -> MemoryCandidate {
    let now = DateTime::now();
    MemoryCandidate {
        id: Some(ObjectId::new()),
        workspace_id: contact.workspace_id.clone(),
        account_id: contact.account_id.clone(),
        contact_wxid: contact.wxid.clone(),
        run_id: Some("sr029-manual".to_string()),
        source: "manual-redline".to_string(),
        candidates: vec![doc! {
            "type": "preference",
            "content": "客户希望回复简洁",
            "evidence": "客户明确要求回复简洁",
            "importance": 8,
            "confidence": 9,
        }],
        memory_write_score: 8,
        status: "pending".to_string(),
        reason: Some("manual redline".to_string()),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
#[ignore = "requires Docker or TEST_MONGODB_URI"]
async fn manual_consolidation_uses_single_flight_durable_task() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();
    let contact = contact(&ws, &account, "sr029-manual");
    app.state
        .db
        .contacts()
        .insert_one(&contact, None)
        .await
        .expect("insert contact");
    app.state
        .db
        .memory_candidates()
        .insert_one(candidate(&contact), None)
        .await
        .expect("insert candidate");
    app.llm.push_response(json!({
        "memoryCard": {
            "coreFacts": ["客户希望回复简洁"],
            "recentFacts": [],
            "preferences": ["回复简洁"],
            "doNotDo": [],
            "objections": [],
            "openLoops": [],
            "openQuestions": [],
            "deprecatedFacts": [],
            "conflicts": [],
            "confirmedFacts": [],
            "commitments": []
        },
        "summary": "客户偏好简洁回复",
        "discarded": []
    }));

    let task_id = run_manual_memory_consolidation(&app.state, &contact, "admin-sr029")
        .await
        .expect("manual consolidation completes through task protocol");
    assert_eq!(app.llm.calls(), 1);

    let task = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_tasks")
        .find_one(doc! { "_id": task_id }, None)
        .await
        .expect("read task")
        .expect("task remains as audit row");
    assert_eq!(task.get_str("status").unwrap(), "sent");
    assert_eq!(task.get_str("gateway_status").unwrap(), "consolidated");
    assert!(!task.contains_key("prepared_commit"));
    assert!(!task.contains_key("claim_token"));
    assert!(!task.contains_key("active_task_key"));

    let memory = app
        .state
        .db
        .operating_memories()
        .find_one(
            doc! {
                "workspace_id": &ws,
                "account_id": &account,
                "contact_wxid": &contact.wxid,
            },
            None,
        )
        .await
        .expect("read memory")
        .expect("memory written");
    assert_eq!(memory.memory_card_version, 1);
    assert!(memory
        .memory_card
        .core_facts
        .iter()
        .any(|fact| fact.as_text() == "客户希望回复简洁"));
    assert_eq!(
        app.state
            .db
            .memory_candidates()
            .count_documents(
                doc! {
                    "workspace_id": &ws,
                    "account_id": &account,
                    "contact_wxid": &contact.wxid,
                    "status": "pending",
                },
                None,
            )
            .await
            .expect("count pending candidates"),
        0
    );

    let blocker_id = ObjectId::new();
    app.state
        .db
        .raw()
        .collection::<Document>("agent_tasks")
        .insert_one(
            doc! {
                "_id": blocker_id,
                "workspace_id": &ws,
                "account_id": &account,
                "contact_wxid": &contact.wxid,
                "kind": "memory_consolidation",
                "active_task_key": "memory_consolidation",
                "status": "running",
                "run_at": DateTime::now(),
                "content": "existing owner",
                "review_required": false,
                "attempt_count": 1,
                "max_attempts": 3,
                "claim_recovery_count": 0,
                "created_at": DateTime::now(),
                "updated_at": DateTime::now(),
            },
            None,
        )
        .await
        .expect("insert active blocker");
    let before = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_tasks")
        .find_one(doc! { "_id": blocker_id }, None)
        .await
        .expect("read blocker before")
        .unwrap();
    let error = run_manual_memory_consolidation(&app.state, &contact, "admin-sr029")
        .await
        .expect_err("existing owner must be an explicit conflict");
    assert!(matches!(error, AppError::Conflict(_)));
    let after = app
        .state
        .db
        .raw()
        .collection::<Document>("agent_tasks")
        .find_one(doc! { "_id": blocker_id }, None)
        .await
        .expect("read blocker after")
        .unwrap();
    assert_eq!(
        after, before,
        "manual conflict must not mutate existing owner"
    );
    assert_eq!(app.llm.calls(), 1, "conflict must not call the model");
    app.cleanup().await;
}

fn prepared_task(
    task_id: ObjectId,
    candidate_id: ObjectId,
    ws: &str,
    account: &str,
    wxid: &str,
    generation: i64,
) -> Document {
    doc! {
        "_id": task_id,
        "workspace_id": ws,
        "account_id": account,
        "contact_wxid": wxid,
        "kind": "memory_consolidation",
        "active_task_key": "memory_consolidation",
        "status": "committing",
        "claim_token": format!("claim-{generation}"),
        "claim_generation": generation,
        "run_at": DateTime::now(),
        "content": "prepared SR-029 commit",
        "review_required": false,
        "attempt_count": 1,
        "max_attempts": 3,
        "claim_recovery_count": 0,
        "created_at": DateTime::now(),
        "updated_at": DateTime::now(),
        "prepared_commit_kind": "memory_consolidation",
        "prepared_commit": {
            "workspace_id": ws,
            "account_id": account,
            "contact_wxid": wxid,
            "prev_version": 0,
            "next_version": 1,
            "memory_card": {
                "coreFacts": ["prepared fact"],
                "recentFacts": [],
                "deprecatedFacts": [],
                "source": "memory_consolidator_agent",
                "version": 1,
            },
            "confirmed_tags": [],
            "personality_profile": Bson::Null,
            "candidate_ids": [candidate_id],
            "run_id": format!("run-{generation}"),
            "warnings": [],
            "conflicts": [{
                "a_id": "a",
                "b_id": "b",
                "winner": "a",
                "resolution": "newer evidence",
                "auditSource": "model_conflict",
                "runId": format!("run-{generation}"),
                "previousVersion": 0,
                "memoryCardVersion": 1,
            }],
            "summary": "prepared summary",
            "discarded": [],
            "candidate_count": 1,
        },
    }
}

async fn seed_recovery_case(
    app: &common::TestApp,
    case: &str,
    generation: i64,
    preapply_memory: bool,
    preapply_projections: bool,
) -> (ObjectId, ObjectId, String) {
    let ws = app.state.config.default_workspace_id.as_str();
    let account = app.state.config.default_account_id.as_str();
    let wxid = format!("sr029-recovery-{case}");
    let task_id = ObjectId::new();
    let candidate_id = ObjectId::new();
    let now = DateTime::now();
    let mut memory = doc! {
        "_id": ObjectId::new(),
        "workspace_id": ws,
        "account_id": account,
        "contact_wxid": &wxid,
        "memory_card": {},
        "memory_card_version": 0,
        "created_at": now,
        "updated_at": now,
    };
    if preapply_memory {
        memory.insert(
            "memory_card",
            doc! {
                "coreFacts": ["prepared fact"],
                "recentFacts": [],
                "deprecatedFacts": [],
                "source": "memory_consolidator_agent",
                "version": 1,
            },
        );
        memory.insert("memory_card_version", 1);
        memory.insert("memory_source_task_id", task_id);
        memory.insert("memory_source_task_claim_generation", generation);
        memory.insert(
            "memory_applied_commits",
            vec![doc! { "task_id": task_id, "claim_generation": generation }],
        );
    }
    app.state
        .db
        .raw()
        .collection::<Document>("operating_memories")
        .insert_one(memory, None)
        .await
        .expect("insert operating memory");

    let mut contact_doc = doc! {
        "_id": ObjectId::new(),
        "workspace_id": ws,
        "account_id": account,
        "wxid": &wxid,
        "created_at": now,
        "updated_at": now,
    };
    if preapply_projections {
        contact_doc.insert("confirmed_tags", Vec::<Document>::new());
        contact_doc.insert("memory_projection_version", 1);
        contact_doc.insert("memory_projection_source_task_id", task_id);
        contact_doc.insert("memory_projection_source_claim_generation", generation);
    }
    app.state
        .db
        .raw()
        .collection::<Document>("contacts")
        .insert_one(contact_doc, None)
        .await
        .expect("insert contact");

    let mut candidate_doc = doc! {
        "_id": candidate_id,
        "workspace_id": ws,
        "account_id": account,
        "contact_wxid": &wxid,
        "status": "pending",
        "created_at": now,
        "updated_at": now,
    };
    if preapply_projections {
        candidate_doc.insert("status", "consolidated");
        candidate_doc.insert("consolidated_by_task_id", task_id);
        candidate_doc.insert("consolidated_by_claim_generation", generation);
    }
    app.state
        .db
        .raw()
        .collection::<Document>("memory_candidates")
        .insert_one(candidate_doc, None)
        .await
        .expect("insert candidate");
    app.state
        .db
        .raw()
        .collection::<Document>("agent_tasks")
        .insert_one(
            prepared_task(task_id, candidate_id, ws, account, &wxid, generation),
            None,
        )
        .await
        .expect("insert prepared task");

    if preapply_projections {
        app.state
            .db
            .raw()
            .collection::<Document>("agent_events")
            .insert_one(
                doc! {
                    "workspace_id": ws,
                    "account_id": account,
                    "contact_wxid": &wxid,
                    "kind": "memory_conflict_resolved",
                    "status": "info",
                    "summary": "consolidator 解决了一组事实冲突",
                    "details": {
                        "winner": "a",
                        "auditSource": "model_conflict",
                        "runId": format!("run-{generation}"),
                        "previousVersion": 0,
                        "memoryCardVersion": 1,
                    },
                    "created_at": now,
                    "dedupe_key": format!(
                        "memory_commit:{}:{}:conflict:0",
                        task_id.to_hex(), generation
                    ),
                },
                None,
            )
            .await
            .expect("insert already-written conflict event");
    }
    (task_id, candidate_id, wxid)
}

#[tokio::test]
#[ignore = "requires Docker or TEST_MONGODB_URI"]
async fn prepared_commit_replays_all_partial_windows_exactly_once() {
    let app = common::TestApp::start().await;
    let ws = app.state.config.default_workspace_id.clone();
    let account = app.state.config.default_account_id.clone();
    for (case, generation, memory_applied, projections_applied) in [
        ("none", 11_i64, false, false),
        ("memory", 12_i64, true, false),
        ("projections", 13_i64, true, true),
    ] {
        let (task_id, candidate_id, wxid) =
            seed_recovery_case(&app, case, generation, memory_applied, projections_applied).await;

        reconcile_memory_consolidation_commit(&app.state, task_id)
            .await
            .expect("first recovery pass");
        reconcile_memory_consolidation_commit(&app.state, task_id)
            .await
            .expect("second recovery pass is a no-op");

        let raw = app.state.db.raw();
        let task = raw
            .collection::<Document>("agent_tasks")
            .find_one(doc! { "_id": task_id }, None)
            .await
            .expect("read task")
            .unwrap();
        assert_eq!(task.get_str("status").unwrap(), "sent", "case={case}");
        assert_eq!(task.get_str("gateway_status").unwrap(), "consolidated");
        assert!(!task.contains_key("prepared_commit"));
        assert!(!task.contains_key("active_task_key"));

        let memory = raw
            .collection::<Document>("operating_memories")
            .find_one(
                doc! {
                    "workspace_id": &ws,
                    "account_id": &account,
                    "contact_wxid": &wxid,
                },
                None,
            )
            .await
            .expect("read memory")
            .unwrap();
        assert_eq!(memory.get_i32("memory_card_version").unwrap(), 1);
        assert_eq!(
            memory.get_object_id("memory_source_task_id").unwrap(),
            task_id
        );
        assert!(memory
            .get_array("memory_applied_commits")
            .map(|items| items.is_empty())
            .unwrap_or(true));

        let projected = raw
            .collection::<Document>("contacts")
            .find_one(
                doc! { "workspace_id": &ws, "account_id": &account, "wxid": &wxid },
                None,
            )
            .await
            .expect("read contact")
            .unwrap();
        assert_eq!(projected.get_i32("memory_projection_version").unwrap(), 1);
        assert_eq!(
            projected
                .get_object_id("memory_projection_source_task_id")
                .unwrap(),
            task_id
        );

        let candidate = raw
            .collection::<Document>("memory_candidates")
            .find_one(doc! { "_id": candidate_id }, None)
            .await
            .expect("read candidate")
            .unwrap();
        assert_eq!(candidate.get_str("status").unwrap(), "consolidated");
        assert_eq!(
            candidate.get_object_id("consolidated_by_task_id").unwrap(),
            task_id
        );

        assert_eq!(
            raw.collection::<Document>("agent_events")
                .count_documents(
                    doc! {
                        "workspace_id": &ws,
                        "dedupe_key": format!(
                            "memory_commit:{}:{}:conflict:0",
                            task_id.to_hex(), generation
                        ),
                    },
                    None,
                )
                .await
                .expect("count conflict event"),
            1,
            "case={case}"
        );
        let conflict = raw
            .collection::<Document>("agent_events")
            .find_one(
                doc! {
                    "workspace_id": &ws,
                    "dedupe_key": format!(
                        "memory_commit:{}:{}:conflict:0",
                        task_id.to_hex(), generation
                    ),
                },
                None,
            )
            .await
            .expect("read conflict event")
            .expect("conflict event exists");
        let details = conflict.get_document("details").expect("conflict details");
        assert_eq!(
            details.get_str("runId").unwrap(),
            format!("run-{generation}")
        );
        assert_eq!(details.get_i32("previousVersion").unwrap(), 0);
        assert_eq!(details.get_i32("memoryCardVersion").unwrap(), 1);
        assert_eq!(details.get_str("auditSource").unwrap(), "model_conflict");
        assert_eq!(
            raw.collection::<Document>("agent_events")
                .count_documents(
                    doc! {
                        "workspace_id": &ws,
                        "dedupe_key": format!(
                            "memory_commit:{}:{}:complete",
                            task_id.to_hex(), generation
                        ),
                    },
                    None,
                )
                .await
                .expect("count completion event"),
            1,
            "case={case}"
        );
    }
    app.cleanup().await;
}
