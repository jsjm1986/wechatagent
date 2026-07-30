//! Phase G P1-4：chunk advisory presence 生命周期回归。
//!
//! 默认测试覆盖事件/TTL；两个 ignored Handler 红线使用副本集 MongoDB 验证：
//!   - 请求必须先验证 chunk 属于当前 workspace，跨 workspace 返回 404 且零 presence；
//!   - 同 workspace 并发 acquire 恰好一个 200、一个 advisory 409；
//!   - presence 只作协作提示，mutation 的写权仍由认证、事务与 CAS 决定。

mod common;

use std::sync::Arc;

use axum::{
    body::to_bytes,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Duration;
use mongodb::bson::{oid::ObjectId, DateTime};
use tokio::sync::Barrier;
use wechatagent::{
    auth::AuthenticatedAdmin,
    models::OperationKnowledgeChunk,
    routes::chunk_locks::{
        acquire_chunk_lock, ChunkEditLock, ChunkEvent, LockAcquireRequest, CHUNK_LOCK_TTL_SECONDS,
    },
};

use crate::common::TestApp;

fn admin(user_id: &str, workspace_id: &str) -> AuthenticatedAdmin {
    AuthenticatedAdmin {
        user_id: user_id.to_string(),
        username: user_id.to_string(),
        current_workspace: workspace_id.to_string(),
    }
}

async fn seed_chunk(app: &TestApp, workspace_id: &str) -> ObjectId {
    let id = ObjectId::new();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(
            OperationKnowledgeChunk {
                id: Some(id),
                workspace_id: workspace_id.to_string(),
                domain: "user_operations".to_string(),
                title: "presence target".to_string(),
                status: "draft".to_string(),
                created_at: DateTime::now(),
                updated_at: DateTime::now(),
                ..Default::default()
            },
            None,
        )
        .await
        .expect("seed presence target");
    id
}

#[test]
fn lock_ttl_constant_is_five_minutes() {
    // 防止后续手滑改 TTL：这是合约值，运营前端 heartbeat 节奏依赖它。
    assert_eq!(CHUNK_LOCK_TTL_SECONDS, 300);
}

#[test]
fn lock_event_serialized_kind_is_snake_case() {
    let lock_ev = ChunkEvent::Locked {
        chunk_id: "abc".into(),
        workspace_id: "ws_a".into(),
        owner_user_id: "u1".into(),
        owner_username: "alice".into(),
        expires_at: chrono::Utc::now(),
    };
    let payload = serde_json::to_value(&lock_ev).expect("serialize");
    assert_eq!(payload["kind"].as_str(), Some("locked"));

    let unlock_ev = ChunkEvent::Unlocked {
        chunk_id: "abc".into(),
        workspace_id: "ws_a".into(),
        owner_user_id: "u1".into(),
    };
    let payload = serde_json::to_value(&unlock_ev).expect("serialize");
    assert_eq!(payload["kind"].as_str(), Some("unlocked"));

    let revised_ev = ChunkEvent::Revised {
        chunk_id: "abc".into(),
        workspace_id: "ws_a".into(),
        revision_kind: "patch".into(),
        actor: "alice".into(),
    };
    let payload = serde_json::to_value(&revised_ev).expect("serialize");
    assert_eq!(payload["kind"].as_str(), Some("revised"));
    assert_eq!(payload["revision_kind"].as_str(), Some("patch"));
}

#[test]
fn lock_expiration_boundary() {
    let now = chrono::Utc::now();
    let lock = ChunkEditLock {
        chunk_id: "x".into(),
        workspace_id: "ws".into(),
        owner_user_id: "u".into(),
        owner_username: "alice".into(),
        locked_at: now,
        expires_at: now + Duration::seconds(60),
    };
    assert!(!lock.is_expired(now));
    // 边界：expires_at == now 视为已过期（包含等号）
    assert!(lock.is_expired(now + Duration::seconds(60)));
    assert!(lock.is_expired(now + Duration::seconds(120)));
}

#[tokio::test]
async fn broadcast_channel_delivers_to_late_subscriber() {
    use tokio::sync::broadcast;

    let (tx, _rx_dropped) = broadcast::channel::<ChunkEvent>(8);

    // 提前发一个事件——晚来的订阅者应该看不到
    let _ = tx.send(ChunkEvent::Unlocked {
        chunk_id: "before".into(),
        workspace_id: "ws".into(),
        owner_user_id: "u".into(),
    });

    let mut rx = tx.subscribe();

    // 订阅之后再发一个——这个应该收到
    tx.send(ChunkEvent::Unlocked {
        chunk_id: "after".into(),
        workspace_id: "ws".into(),
        owner_user_id: "u".into(),
    })
    .expect("at least the new subscriber must receive");

    let ev = rx.recv().await.expect("recv after-subscribe event");
    match ev {
        ChunkEvent::Unlocked { chunk_id, .. } => assert_eq!(chunk_id, "after"),
        _ => panic!("unexpected event variant"),
    }
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn cross_workspace_presence_is_not_found_and_leaves_no_entry() {
    let app = TestApp::start_repl_set().await;
    let owner_workspace = app.state.config.default_workspace_id.clone();
    let chunk_id = seed_chunk(&app, &owner_workspace).await;

    let response = acquire_chunk_lock(
        State(app.state.clone()),
        Extension(admin("foreign-admin", "foreign-workspace")),
        Path(chunk_id.to_hex()),
        Json(LockAcquireRequest::default()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        app.state.chunk_locks.is_empty(),
        "cross-workspace lookup must not create or expose advisory presence"
    );
}

#[tokio::test]
#[ignore = "requires replica-set MongoDB / testcontainers"]
async fn concurrent_presence_acquire_has_one_owner_and_one_advisory_conflict() {
    let app = TestApp::start_repl_set().await;
    let workspace_id = app.state.config.default_workspace_id.clone();
    let chunk_id = seed_chunk(&app, &workspace_id).await;
    let barrier = Arc::new(Barrier::new(3));

    let mut tasks = Vec::new();
    for user_id in ["alice", "bob"] {
        let state = app.state.clone();
        let workspace_id = workspace_id.clone();
        let chunk_id = chunk_id.to_hex();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = acquire_chunk_lock(
                State(state),
                Extension(admin(user_id, &workspace_id)),
                Path(chunk_id),
                Json(LockAcquireRequest::default()),
            )
            .await
            .into_response();
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read presence response");
            let body: serde_json::Value =
                serde_json::from_slice(&body).expect("decode presence response");
            (status, body)
        }));
    }
    barrier.wait().await;

    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.expect("presence task"));
    }
    results.sort_by_key(|(status, _)| status.as_u16());
    assert_eq!(results[0].0, StatusCode::OK);
    assert_eq!(results[1].0, StatusCode::CONFLICT);
    assert_eq!(results[0].1["advisory"], true);
    assert_eq!(results[1].1["advisory"], true);
    assert_eq!(results[1].1["error"], "chunk_presence_by_other");
    assert_eq!(app.state.chunk_locks.len(), 1);
    assert!(app
        .state
        .chunk_locks
        .contains_key(&(workspace_id, chunk_id.to_hex())));
}

#[tokio::test]
#[ignore]
async fn lock_acquire_release_smoke_via_dashmap() {
    use dashmap::DashMap;
    use std::sync::Arc;

    // 不走 handler、不走 axum：直接验证 presence 表的复合 key。
    let locks: Arc<DashMap<(String, String), ChunkEditLock>> = Arc::new(DashMap::new());
    let now = chrono::Utc::now();
    let lock = ChunkEditLock {
        chunk_id: "chunk_1".into(),
        workspace_id: "ws_a".into(),
        owner_user_id: "alice".into(),
        owner_username: "alice".into(),
        locked_at: now,
        expires_at: now + Duration::seconds(300),
    };
    let key = ("ws_a".to_string(), "chunk_1".to_string());
    assert!(locks.insert(key.clone(), lock.clone()).is_none());

    // 同 owner 续期——返回旧值
    let renewed = ChunkEditLock {
        expires_at: now + Duration::seconds(600),
        ..lock.clone()
    };
    assert!(locks.insert(key.clone(), renewed).is_some());

    // 释放
    assert!(locks.remove(&key).is_some());
    assert!(locks.get(&key).is_none());
}
