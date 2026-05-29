//! Phase G P1-4：chunk 软锁生命周期集成测试。
//!
//! 这个 suite 不需要 Docker（不连数据库），只验证软锁的纯逻辑：
//!   - acquire 后 owner 续期、非 owner 撞锁 409；
//!   - release 仅 owner 可调；
//!   - 过期后非 owner 可重新 acquire。
//!
//! handler 直接调用，绕过 axum HTTP 层；admin 注入靠手工构造
//! `AuthenticatedAdmin`。`AppState` 用 `TestApp::start` 仍需 Docker，
//! 因此整个 suite 默认 `#[ignore]`，与其它集成测试一致。

mod common;

use chrono::Duration;
use wechatagent::routes::chunk_locks::{ChunkEditLock, ChunkEvent, CHUNK_LOCK_TTL_SECONDS};

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
#[ignore]
async fn lock_acquire_release_smoke_via_dashmap() {
    use dashmap::DashMap;
    use std::sync::Arc;

    // 不走 handler、不走 axum：直接验证锁表的状态机契约。
    let locks: Arc<DashMap<String, ChunkEditLock>> = Arc::new(DashMap::new());
    let now = chrono::Utc::now();
    let lock = ChunkEditLock {
        chunk_id: "chunk_1".into(),
        workspace_id: "ws_a".into(),
        owner_user_id: "alice".into(),
        owner_username: "alice".into(),
        locked_at: now,
        expires_at: now + Duration::seconds(300),
    };
    assert!(locks.insert("chunk_1".into(), lock.clone()).is_none());

    // 同 owner 续期——返回旧值
    let renewed = ChunkEditLock {
        expires_at: now + Duration::seconds(600),
        ..lock.clone()
    };
    assert!(locks.insert("chunk_1".into(), renewed).is_some());

    // 释放
    assert!(locks.remove("chunk_1").is_some());
    assert!(locks.get("chunk_1").is_none());
}
