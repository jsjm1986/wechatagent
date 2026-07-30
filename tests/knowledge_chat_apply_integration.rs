//! 知识对话工作台 chat_apply 红线集成测试:apply_create_chunk 落库瞬间
//! status=draft + integrity_status=needs_review(AI 永不自动 verify)。
//! 全部 `#[ignore]`,需 Docker testcontainers。
//! CI:`cargo test --test knowledge_chat_apply_integration -- --ignored`。
//!
//! ## 红线意义(P0):chat 内 AI 起草的知识落库**必须** status=draft + integrity=needs_review
//! (chat.rs:1679-1681 强制),AI 永不把自己产物标 verified。审计指出原 recall benchmark
//! 紧接 verify 把中间态盖过,看不到"落库瞬间是 draft"。本测试落库后**立即查 DB**(不 verify),
//! 钉死中间态。一旦 apply 误标 verified,本测试立刻红。
#![cfg(test)]

mod common;

use axum::extract::{Extension, Json, Path, State};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde_json::{json, Value};

use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::models::{KnowledgeChatTurn, OperationKnowledgeChunk, WechatAccount};
use wechatagent::routes::ext_knowledge::{chat_apply, ChatApplyRequest};
use wechatagent::routes::knowledge::chat::apply_create_chunk;

use crate::common::TestApp;

fn test_admin(workspace_id: &str, user_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: user_id.to_string(),
        username: user_id.to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

fn test_account(workspace_id: &str, account_id: &str) -> WechatAccount {
    let now = DateTime::now();
    WechatAccount {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        alias: account_id.to_string(),
        display_name: account_id.to_string(),
        app_id: None,
        wxid: None,
        nick_name: None,
        avatar_url: None,
        mcp_base_url: None,
        mcp_api_key: None,
        webhook_secret: None,
        online: true,
        status: Some("active".to_string()),
        last_sync_at: now,
        capacity: 0,
        persona_tag: None,
        off_hours: vec![],
        created_at: now,
        updated_at: now,
    }
}

async fn seed_pending_create_session(
    app: &TestApp,
    workspace_id: &str,
    account_id: &str,
    owner_admin_id: &str,
    session_id: &str,
) -> ObjectId {
    let now = DateTime::now();
    app.state
        .db
        .accounts()
        .insert_one(test_account(workspace_id, account_id), None)
        .await
        .expect("seed account");
    app.state
        .db
        .knowledge_chat_session_seqs()
        .insert_one(
            doc! {
                "_id": format!("{workspace_id}|{session_id}"),
                "workspace_id": workspace_id,
                "account_id": account_id,
                "session_id": session_id,
                "owner_admin_id": owner_admin_id,
                "seq": 2_i64,
                "created_at": now,
                "updated_at": now,
            },
            None,
        )
        .await
        .expect("seed chat session identity");

    let user_turn = KnowledgeChatTurn {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        session_id: session_id.to_string(),
        turn_index: 1,
        role: "user".to_string(),
        intent: None,
        content: "create one concurrency test chunk".to_string(),
        attachments: vec![],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        apply_result: None,
        applied_at: None,
        tokens_used: 0,
        prompt_key: None,
        kind: None,
        tool_calls: vec![],
        created_at: now,
    };
    let assistant_turn_id = ObjectId::new();
    let assistant_turn = KnowledgeChatTurn {
        id: Some(assistant_turn_id),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        session_id: session_id.to_string(),
        turn_index: 2,
        role: "assistant".to_string(),
        intent: Some("create_chunk".to_string()),
        content: "Draft ready for apply.".to_string(),
        attachments: vec![],
        patch: Some(doc! {
            "title": "SR-111 exactly-once chunk",
            "summary": "Concurrent and replayed apply calls create one draft.",
            "body": "One pending assistant turn must be applied exactly once.",
            "knowledgeType": "methodology",
        }),
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        apply_result: None,
        applied_at: None,
        tokens_used: 0,
        prompt_key: None,
        kind: None,
        tool_calls: vec![],
        created_at: now,
    };
    app.state
        .db
        .knowledge_chat_turns()
        .insert_many(vec![user_turn, assistant_turn], None)
        .await
        .expect("seed pending chat turns");
    assistant_turn_id
}

async fn apply_session(
    app: &TestApp,
    admin: AuthenticatedAdmin,
    session_id: &str,
    account_id: &str,
) -> wechatagent::error::AppResult<Value> {
    let request: ChatApplyRequest =
        serde_json::from_value(json!({ "accountId": account_id })).expect("apply request");
    chat_apply(
        State(app.state.clone()),
        Extension(admin),
        Path(session_id.to_string()),
        Json(request),
    )
    .await
    .map(|response| response.0)
}

async fn seed_pending_update_session(
    app: &TestApp,
    workspace_id: &str,
    account_id: &str,
    owner_admin_id: &str,
    session_id: &str,
) -> (ObjectId, ObjectId, DateTime) {
    let frozen_at = DateTime::now();
    let chunk_id = ObjectId::new();
    app.state
        .db
        .accounts()
        .insert_one(test_account(workspace_id, account_id), None)
        .await
        .expect("seed update account");
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            OperationKnowledgeChunk {
                id: Some(chunk_id),
                workspace_id: workspace_id.to_string(),
                account_id: Some(account_id.to_string()),
                domain: "user_operations".to_string(),
                title: "frozen title".to_string(),
                summary: Some("frozen summary".to_string()),
                body: Some("frozen body".to_string()),
                status: "draft".to_string(),
                integrity_status: Some("needs_review".to_string()),
                created_at: frozen_at,
                updated_at: frozen_at,
                ..Default::default()
            },
            None,
        )
        .await
        .expect("seed frozen chunk");
    app.state
        .db
        .knowledge_chat_session_seqs()
        .insert_one(
            doc! {
                "_id": format!("{workspace_id}|{session_id}"),
                "workspace_id": workspace_id,
                "account_id": account_id,
                "session_id": session_id,
                "owner_admin_id": owner_admin_id,
                "seq": 2_i64,
                "created_at": frozen_at,
                "updated_at": frozen_at,
            },
            None,
        )
        .await
        .expect("seed update session identity");

    let frozen_rfc3339 = frozen_at
        .try_to_rfc3339_string()
        .expect("serialize frozen updated_at");
    let attachment = doc! {
        "chunk_id": chunk_id.to_hex(),
        "expected_updated_at": frozen_rfc3339,
    };
    let user_turn = KnowledgeChatTurn {
        id: Some(ObjectId::new()),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        session_id: session_id.to_string(),
        turn_index: 1,
        role: "user".to_string(),
        intent: None,
        content: "replace the frozen title".to_string(),
        attachments: vec![attachment.clone()],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        apply_result: None,
        applied_at: None,
        tokens_used: 0,
        prompt_key: None,
        kind: None,
        tool_calls: vec![],
        created_at: frozen_at,
    };
    let assistant_turn_id = ObjectId::new();
    let assistant_turn = KnowledgeChatTurn {
        id: Some(assistant_turn_id),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        session_id: session_id.to_string(),
        turn_index: 2,
        role: "assistant".to_string(),
        intent: Some("update_chunk".to_string()),
        content: "stale draft ready".to_string(),
        attachments: vec![attachment],
        patch: Some(doc! { "title": "stale AI title" }),
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        apply_result: None,
        applied_at: None,
        tokens_used: 0,
        prompt_key: None,
        kind: None,
        tool_calls: vec![],
        created_at: frozen_at,
    };
    app.state
        .db
        .knowledge_chat_turns()
        .insert_many(vec![user_turn, assistant_turn], None)
        .await
        .expect("seed pending update turns");
    (chunk_id, assistant_turn_id, frozen_at)
}

/// 红线:apply_create_chunk 落库瞬间 status=draft && integrity_status=needs_review。
#[tokio::test]
#[ignore]
async fn chat_apply_create_forces_draft_needs_review() {
    let app = TestApp::start_repl_set().await;
    let ws = app.state.config.default_workspace_id.clone();
    let patch = doc! {
        "title": "退款政策",
        "body": "7 天无理由退款,需保持商品完好。",
        "summary": "退款规则",
        "knowledgeType": "policy",
    };

    let result = apply_create_chunk(
        &app.state,
        &ws,
        Some("default"),
        "sess_test",
        &patch,
        None,
        "运营口述:我们支持 7 天无理由退款",
    )
    .await
    .expect("apply_create_chunk 应成功");

    // 返回体即声明 draft + needs_review
    assert_eq!(result["status"], "draft", "返回体 status 应为 draft");
    assert_eq!(
        result["integrityStatus"], "needs_review",
        "返回体 integrityStatus 应为 needs_review"
    );

    // 落库后立即查 DB(不 verify),断言中间态就是 draft + needs_review
    let created_id = result["createdChunkId"].as_str().expect("createdChunkId");
    let oid = mongodb::bson::oid::ObjectId::parse_str(created_id).expect("oid");
    let chunk = app
        .state
        .db
        .operation_knowledge_chunks()
        .find_one(doc! { "_id": oid }, None)
        .await
        .expect("查 chunk")
        .expect("chunk 应存在");

    assert_eq!(
        chunk.status, "draft",
        "落库瞬间 status 必须 draft(AI 永不自动 verify)"
    );
    assert_eq!(
        chunk.integrity_status.as_deref(),
        Some("needs_review"),
        "落库瞬间 integrity_status 必须 needs_review"
    );
}

#[tokio::test]
#[ignore]
async fn concurrent_and_replayed_chat_apply_is_exactly_once() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let session_id = format!("sr111-{}", uuid::Uuid::new_v4().simple());
    let admin = test_admin(&workspace_id, "sr111_admin");
    let assistant_turn_id = seed_pending_create_session(
        &app,
        &workspace_id,
        &account_id,
        &admin.user_id,
        &session_id,
    )
    .await;

    let (first, second) = tokio::join!(
        apply_session(&app, admin.clone(), &session_id, &account_id),
        apply_session(&app, admin.clone(), &session_id, &account_id),
    );
    let first = first.expect("first concurrent apply");
    let second = second.expect("second concurrent apply");
    let replay = apply_session(&app, admin, &session_id, &account_id)
        .await
        .expect("replayed apply");
    assert_eq!(
        first, second,
        "concurrent callers must receive one stable receipt"
    );
    assert_eq!(first, replay, "replay must return the committed receipt");

    let chunk_id = first["result"]["createdChunkId"]
        .as_str()
        .expect("createdChunkId in receipt");
    let chunk_oid = ObjectId::parse_str(chunk_id).expect("created chunk object id");
    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! {
                    "_id": chunk_oid,
                    "workspace_id": &workspace_id,
                    "title": "SR-111 exactly-once chunk",
                },
                None,
            )
            .await
            .expect("count created chunks"),
        1,
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "chunk_id": chunk_id,
                    "op": "create",
                },
                None,
            )
            .await
            .expect("count create revisions"),
        1,
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "account_id": &account_id,
                    "kind": "knowledge_chat_applied",
                    "details.sessionId": &session_id,
                },
                None,
            )
            .await
            .expect("count apply audit events"),
        1,
    );
    let saved_turn = app
        .state
        .db
        .knowledge_chat_turns()
        .find_one(doc! { "_id": assistant_turn_id }, None)
        .await
        .expect("read assistant turn")
        .expect("assistant turn exists");
    assert_eq!(saved_turn.status, "applied");
    assert!(saved_turn.applied_at.is_some());
    assert!(saved_turn.apply_result.is_some());

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn chat_apply_wrong_account_or_admin_is_zero_write() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let wrong_account_id = "sr111_other_account";
    let session_id = format!("sr111-scope-{}", uuid::Uuid::new_v4().simple());
    let owner = test_admin(&workspace_id, "sr111_owner");
    let assistant_turn_id = seed_pending_create_session(
        &app,
        &workspace_id,
        &account_id,
        &owner.user_id,
        &session_id,
    )
    .await;
    app.state
        .db
        .accounts()
        .insert_one(test_account(&workspace_id, wrong_account_id), None)
        .await
        .expect("seed wrong account");

    let wrong_account = apply_session(&app, owner.clone(), &session_id, wrong_account_id)
        .await
        .expect_err("wrong account must be rejected");
    assert!(matches!(
        wrong_account,
        wechatagent::error::AppError::NotFound(_)
    ));
    let wrong_admin = apply_session(
        &app,
        test_admin(&workspace_id, "sr111_other_admin"),
        &session_id,
        &account_id,
    )
    .await
    .expect_err("wrong admin must be rejected");
    assert!(matches!(
        wrong_admin,
        wechatagent::error::AppError::NotFound(_)
    ));

    assert_eq!(
        app.state
            .db
            .operation_knowledge_chunks()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "title": "SR-111 exactly-once chunk",
                },
                None,
            )
            .await
            .expect("count chunks after rejected applies"),
        0,
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(doc! { "workspace_id": &workspace_id }, None)
            .await
            .expect("count revisions after rejected applies"),
        0,
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "kind": "knowledge_chat_applied",
                    "details.sessionId": &session_id,
                },
                None,
            )
            .await
            .expect("count audit events after rejected applies"),
        0,
    );
    let saved_turn = app
        .state
        .db
        .knowledge_chat_turns()
        .find_one(doc! { "_id": assistant_turn_id }, None)
        .await
        .expect("read pending assistant turn")
        .expect("assistant turn exists");
    assert_eq!(saved_turn.status, "pending");
    assert!(saved_turn.apply_result.is_none());
    assert!(saved_turn.applied_at.is_none());

    app.cleanup().await;
}

#[tokio::test]
#[ignore]
async fn stale_chunk_snapshot_rejects_chat_apply_with_zero_write() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let account_id = app.state.config.default_account_id.clone();
    let session_id = format!("sr130-stale-{}", uuid::Uuid::new_v4().simple());
    let admin = test_admin(&workspace_id, "sr130_admin");
    let (chunk_id, assistant_turn_id, frozen_at) = seed_pending_update_session(
        &app,
        &workspace_id,
        &account_id,
        &admin.user_id,
        &session_id,
    )
    .await;

    let concurrent_at = DateTime::from_millis(frozen_at.timestamp_millis() + 1_000);
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": chunk_id, "workspace_id": &workspace_id },
            doc! { "$set": {
                "title": "concurrent authoritative title",
                "updated_at": concurrent_at,
            } },
            None,
        )
        .await
        .expect("simulate concurrent chunk update");
    let raw_chunks = app
        .state
        .db
        .raw()
        .collection::<mongodb::bson::Document>("operation_knowledge_chunks");
    let chunk_before = raw_chunks
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .expect("read concurrent chunk baseline")
        .expect("concurrent chunk exists");

    let error = apply_session(&app, admin, &session_id, &account_id)
        .await
        .expect_err("stale frozen version must be rejected");
    assert!(
        matches!(error, wechatagent::error::AppError::Conflict(ref reason) if reason == "chat_chunk_snapshot_stale"),
        "unexpected stale apply error: {error}"
    );

    let chunk_after = raw_chunks
        .find_one(doc! { "_id": chunk_id }, None)
        .await
        .expect("read chunk after stale apply")
        .expect("chunk remains");
    assert_eq!(
        chunk_after, chunk_before,
        "stale apply must not mutate Chunk BSON"
    );
    assert_eq!(
        app.state
            .db
            .chunk_revisions()
            .count_documents(
                doc! { "workspace_id": &workspace_id, "chunk_id": chunk_id.to_hex() },
                None,
            )
            .await
            .expect("count stale apply revisions"),
        0,
        "stale apply must not append a revision"
    );
    assert_eq!(
        app.state
            .db
            .events()
            .count_documents(
                doc! {
                    "workspace_id": &workspace_id,
                    "kind": "knowledge_chat_applied",
                    "details.sessionId": &session_id,
                },
                None,
            )
            .await
            .expect("count stale apply audit events"),
        0,
        "stale apply must not emit a success audit"
    );
    let saved_turn = app
        .state
        .db
        .knowledge_chat_turns()
        .find_one(doc! { "_id": assistant_turn_id }, None)
        .await
        .expect("read stale assistant turn")
        .expect("assistant turn remains");
    assert_eq!(
        saved_turn.status, "pending",
        "claim must roll back on OCC conflict"
    );
    assert!(saved_turn.apply_result.is_none());
    assert!(saved_turn.applied_at.is_none());

    app.cleanup().await;
}
