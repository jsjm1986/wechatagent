//! Phase G P1-4：知识 chunk 协作提示 + 事件总线 + WebSocket 推送。
//!
//! ## 形态
//! - presence：进程内 `DashMap<(workspace_id, chunk_id), ChunkEditLock>`；TTL 5 分钟，
//!   进程重启即清。它只表达“谁正在查看/编辑”，不授予写权，也不阻止提交。
//!   - acquire/release 先验证 chunk 属于当前 workspace，避免跨租户探测或占位；
//!   - 同 owner 调 acquire/heartbeat 视为续期；非 owner 调撞 presence 返回 409；
//!   - acquire/release 都使用 DashMap entry 原子状态转换，旧 owner 不能删除新 owner；
//!   - 真正并发写保护仍由 mutation 的 Mongo transaction + version/CAS 提供。
//! - 事件：`tokio::sync::broadcast::Sender<ChunkEvent>`；订阅端：WebSocket。
//!   - 进程内多副本广播；多进程部署需要 Redis pub/sub —— 在 Out-of-scope 留 P2。
//! - 路由：
//!   - `POST   /api/operation-knowledge/chunks/:id/lock`（acquire / 续期）
//!   - `DELETE /api/operation-knowledge/chunks/:id/lock`（release）
//!   - `GET    /api/ws/chunks`（WebSocket，server-push 事件流）
//!
//! ## 与 patch/archive/restore/... 的耦合
//! 本文件只在 chunk 编辑路由的 handler
//! 末尾通过 `state.chunk_event_bus.send(ChunkEvent::Revised{..})` 推一笔事件，
//! 失败不阻塞写入主流程（broadcast::Sender::send 仅在无 receiver 时返回 Err，
//! 当前进程没人订阅时直接吞掉）。

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use dashmap::{mapref::entry::Entry, DashMap};
use futures::{SinkExt, StreamExt};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::AppState;

/// Presence TTL：单次 acquire / heartbeat 后保留协作提示的时长。前端心跳间隔建议 60s。
pub const CHUNK_LOCK_TTL_SECONDS: i64 = 300;

/// 事件 broadcast 通道容量。订阅者跟不上时会丢老事件——锁/版本场景容忍丢，
/// 客户端通过 reload 自愈。
pub const CHUNK_EVENT_CHANNEL_CAPACITY: usize = 256;

/// 当前 chunk 的协作 presence。它是提示信息，不是授权或互斥 lease。
#[derive(Debug, Clone, Serialize)]
pub struct ChunkEditLock {
    pub chunk_id: String,
    pub workspace_id: String,
    pub owner_user_id: String,
    pub owner_username: String,
    pub locked_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ChunkEditLock {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

/// 跨副本广播给前端的事件。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChunkEvent {
    Locked {
        chunk_id: String,
        workspace_id: String,
        owner_user_id: String,
        owner_username: String,
        expires_at: DateTime<Utc>,
    },
    Unlocked {
        chunk_id: String,
        workspace_id: String,
        owner_user_id: String,
    },
    Revised {
        chunk_id: String,
        workspace_id: String,
        revision_kind: String,
        actor: String,
    },
}

/// AppState 持有的 chunk presence 表。复合 key 是必要的防御边界：即使脏数据或测试
/// 让两个 workspace 出现相同 ObjectId，也不能互相观察、占用或释放 presence。
pub type ChunkLockMap = Arc<DashMap<(String, String), ChunkEditLock>>;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockAcquireRequest {}

#[derive(Debug)]
enum PresenceAcquire {
    Acquired {
        lock: ChunkEditLock,
        refreshed: bool,
    },
    Occupied(ChunkEditLock),
}

#[derive(Debug)]
enum PresenceRelease {
    NotPresent,
    NotOwner(ChunkEditLock),
    Released(ChunkEditLock),
}

fn presence_key(workspace_id: &str, chunk_id: &str) -> (String, String) {
    (workspace_id.to_string(), chunk_id.to_string())
}

fn acquire_presence(
    locks: &ChunkLockMap,
    workspace_id: &str,
    chunk_id: &str,
    owner_user_id: &str,
    owner_username: &str,
    now: DateTime<Utc>,
) -> PresenceAcquire {
    let key = presence_key(workspace_id, chunk_id);
    let ttl = Duration::seconds(CHUNK_LOCK_TTL_SECONDS);
    match locks.entry(key) {
        Entry::Occupied(mut entry) => {
            let current = entry.get().clone();
            if !current.is_expired(now) && current.owner_user_id != owner_user_id {
                return PresenceAcquire::Occupied(current);
            }
            let refreshed = !current.is_expired(now) && current.owner_user_id == owner_user_id;
            let lock = ChunkEditLock {
                chunk_id: chunk_id.to_string(),
                workspace_id: workspace_id.to_string(),
                owner_user_id: owner_user_id.to_string(),
                owner_username: owner_username.to_string(),
                locked_at: if refreshed { current.locked_at } else { now },
                expires_at: now + ttl,
            };
            entry.insert(lock.clone());
            PresenceAcquire::Acquired { lock, refreshed }
        }
        Entry::Vacant(entry) => {
            let lock = ChunkEditLock {
                chunk_id: chunk_id.to_string(),
                workspace_id: workspace_id.to_string(),
                owner_user_id: owner_user_id.to_string(),
                owner_username: owner_username.to_string(),
                locked_at: now,
                expires_at: now + ttl,
            };
            entry.insert(lock.clone());
            PresenceAcquire::Acquired {
                lock,
                refreshed: false,
            }
        }
    }
}

fn release_presence(
    locks: &ChunkLockMap,
    workspace_id: &str,
    chunk_id: &str,
    owner_user_id: &str,
    now: DateTime<Utc>,
) -> PresenceRelease {
    match locks.entry(presence_key(workspace_id, chunk_id)) {
        Entry::Vacant(_) => PresenceRelease::NotPresent,
        Entry::Occupied(entry) => {
            let current = entry.get().clone();
            if current.is_expired(now) {
                entry.remove();
                return PresenceRelease::NotPresent;
            }
            if current.owner_user_id != owner_user_id {
                return PresenceRelease::NotOwner(current);
            }
            PresenceRelease::Released(entry.remove())
        }
    }
}

async fn ensure_chunk_in_workspace(
    state: &AppState,
    workspace_id: &str,
    chunk_id: &str,
) -> AppResult<()> {
    let object_id = super::shared::parse_object_id(chunk_id)?;
    let exists = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! { "_id": object_id, "workspace_id": workspace_id },
            None,
        )
        .await?
        .is_some();
    if !exists {
        return Err(AppError::NotFound(
            "operation knowledge chunk not found".to_string(),
        ));
    }
    Ok(())
}

/// `POST /operation-knowledge/chunks/:id/lock` — 登记或续期协作 presence。
///
/// 返回：200 + `{lock: ChunkEditLock, refreshed: bool, advisory: true}` 当当前 admin
/// 已登记；409 + `{error, lock, advisory: true}` 当其它 admin 已登记。409 只表达
/// presence 冲突，不是 mutation 授权失败。
pub async fn acquire_chunk_lock(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(chunk_id): Path<String>,
    Json(_payload): Json<LockAcquireRequest>,
) -> impl IntoResponse {
    if let Err(error) = ensure_chunk_in_workspace(&state, &admin.current_workspace, &chunk_id).await
    {
        return error.into_response();
    }
    let now = Utc::now();
    let (new_lock, refreshed) = match acquire_presence(
        &state.chunk_locks,
        &admin.current_workspace,
        &chunk_id,
        &admin.user_id,
        &admin.username,
        now,
    ) {
        PresenceAcquire::Occupied(lock) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "chunk_presence_by_other",
                    "advisory": true,
                    "lock": lock,
                })),
            )
                .into_response();
        }
        PresenceAcquire::Acquired { lock, refreshed } => (lock, refreshed),
    };

    // 广播 Locked 事件（broadcast 没人订阅时的 Err 直接忽略）
    let _ = state.chunk_event_bus.send(ChunkEvent::Locked {
        chunk_id: chunk_id.clone(),
        workspace_id: new_lock.workspace_id.clone(),
        owner_user_id: new_lock.owner_user_id.clone(),
        owner_username: new_lock.owner_username.clone(),
        expires_at: new_lock.expires_at,
    });

    (
        StatusCode::OK,
        Json(json!({
            "lock": new_lock,
            "refreshed": refreshed,
            "advisory": true,
        })),
    )
        .into_response()
}

/// `DELETE /operation-knowledge/chunks/:id/lock` — 释放协作 presence。
///
/// 仅 presence owner 可释放；其它人调用返回 403。不存在视为成功（幂等）。
pub async fn release_chunk_lock(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(chunk_id): Path<String>,
) -> impl IntoResponse {
    if let Err(error) = ensure_chunk_in_workspace(&state, &admin.current_workspace, &chunk_id).await
    {
        return error.into_response();
    }
    match release_presence(
        &state.chunk_locks,
        &admin.current_workspace,
        &chunk_id,
        &admin.user_id,
        Utc::now(),
    ) {
        PresenceRelease::NotPresent => (
            StatusCode::OK,
            Json(json!({"released": false, "reason": "not_present", "advisory": true})),
        )
            .into_response(),
        PresenceRelease::NotOwner(lock) => (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "presence_owned_by_other",
                "advisory": true,
                "lock": lock,
            })),
        )
            .into_response(),
        PresenceRelease::Released(lock) => {
            let _ = state.chunk_event_bus.send(ChunkEvent::Unlocked {
                chunk_id: chunk_id.clone(),
                workspace_id: lock.workspace_id.clone(),
                owner_user_id: lock.owner_user_id.clone(),
            });
            (
                StatusCode::OK,
                Json(json!({"released": true, "advisory": true})),
            )
                .into_response()
        }
    }
}

/// `GET /ws/chunks` — WebSocket server-push。
///
/// 客户端连上后会收到当前 workspace 的 ChunkEvent；客户端发什么文本都会被
/// 静默忽略（保留 ping/pong）。Close 帧或断连即结束 server task。
pub async fn chunk_event_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> impl IntoResponse {
    let workspace = admin.current_workspace.clone();
    let rx = state.chunk_event_bus.subscribe();
    ws.on_upgrade(move |socket| handle_chunk_socket(socket, rx, workspace))
}

async fn handle_chunk_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<ChunkEvent>,
    workspace: String,
) {
    let (mut sink, mut stream) = socket.split();

    // hello frame：让前端确认连接已就绪
    let _ = sink
        .send(Message::Text(
            json!({"kind": "hello", "workspace": workspace}).to_string(),
        ))
        .await;

    loop {
        tokio::select! {
            // 上游事件：广播 → 过滤 workspace → 推送
            ev = rx.recv() => {
                match ev {
                    Ok(event) => {
                        if event_workspace(&event) != workspace {
                            continue;
                        }
                        let payload = match serde_json::to_string(&event) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if sink.send(Message::Text(payload)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // 客户端跟不上时丢老事件，前端 reload 自愈
                        let _ = sink
                            .send(Message::Text(
                                json!({"kind": "lagged"}).to_string(),
                            ))
                            .await;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // 下游消息：基本忽略，遇 Close / 错误结束
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                }
            }
        }
    }
}

fn event_workspace(ev: &ChunkEvent) -> &str {
    match ev {
        ChunkEvent::Locked { workspace_id, .. } => workspace_id,
        ChunkEvent::Unlocked { workspace_id, .. } => workspace_id,
        ChunkEvent::Revised { workspace_id, .. } => workspace_id,
    }
}

/// patch/archive/restore/rollback/split/merge/relate/unrelate 等编辑路径在
/// `apply_chunk_revision` 完成后调一笔，给前端推 reload 信号。
pub fn broadcast_chunk_revised(
    state: &AppState,
    chunk_id: impl Into<String>,
    revision_kind: impl Into<String>,
    actor: impl Into<String>,
) {
    let _ = state.chunk_event_bus.send(ChunkEvent::Revised {
        chunk_id: chunk_id.into(),
        workspace_id: "".into(), // 调用方覆盖
        revision_kind: revision_kind.into(),
        actor: actor.into(),
    });
}

/// 与 broadcast_chunk_revised 等价，但显式带 workspace_id。
pub fn broadcast_chunk_revised_in(
    state: &AppState,
    workspace_id: impl Into<String>,
    chunk_id: impl Into<String>,
    revision_kind: impl Into<String>,
    actor: impl Into<String>,
) {
    let _ = state.chunk_event_bus.send(ChunkEvent::Revised {
        chunk_id: chunk_id.into(),
        workspace_id: workspace_id.into(),
        revision_kind: revision_kind.into(),
        actor: actor.into(),
    });
}

/// 测试 helper：构造一个事件序列化输出，断言枚举 tag/字段稳定。
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn locked_event_shape_is_stable() {
        let ev = ChunkEvent::Locked {
            chunk_id: "abc".into(),
            workspace_id: "ws_a".into(),
            owner_user_id: "u1".into(),
            owner_username: "alice".into(),
            expires_at: Utc::now(),
        };
        let v: Value = serde_json::to_value(&ev).expect("serialize ChunkEvent::Locked");
        assert_eq!(v["kind"], "locked");
        assert_eq!(v["chunk_id"], "abc");
        assert_eq!(v["workspace_id"], "ws_a");
        assert_eq!(v["owner_user_id"], "u1");
    }

    #[test]
    fn unlocked_event_shape_is_stable() {
        let ev = ChunkEvent::Unlocked {
            chunk_id: "abc".into(),
            workspace_id: "ws_a".into(),
            owner_user_id: "u1".into(),
        };
        let v: Value = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(v["kind"], "unlocked");
    }

    #[test]
    fn revised_event_shape_is_stable() {
        let ev = ChunkEvent::Revised {
            chunk_id: "abc".into(),
            workspace_id: "ws_a".into(),
            revision_kind: "patch".into(),
            actor: "alice".into(),
        };
        let v: Value = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(v["kind"], "revised");
        assert_eq!(v["revision_kind"], "patch");
    }

    #[test]
    fn lock_expiration_predicate() {
        let now = Utc::now();
        let lock = ChunkEditLock {
            chunk_id: "x".into(),
            workspace_id: "ws".into(),
            owner_user_id: "u".into(),
            owner_username: "alice".into(),
            locked_at: now,
            expires_at: now + Duration::seconds(60),
        };
        assert!(!lock.is_expired(now));
        assert!(!lock.is_expired(now + Duration::seconds(59)));
        assert!(lock.is_expired(now + Duration::seconds(60)));
        assert!(lock.is_expired(now + Duration::seconds(61)));
    }

    #[test]
    fn presence_is_workspace_scoped_and_release_is_owner_atomic() {
        let locks = Arc::new(DashMap::new());
        let now = Utc::now();
        assert!(matches!(
            acquire_presence(&locks, "ws-a", "same-id", "alice", "Alice", now),
            PresenceAcquire::Acquired {
                refreshed: false,
                ..
            }
        ));
        assert!(matches!(
            acquire_presence(&locks, "ws-b", "same-id", "bob", "Bob", now),
            PresenceAcquire::Acquired {
                refreshed: false,
                ..
            }
        ));
        assert_eq!(locks.len(), 2);
        assert!(matches!(
            release_presence(&locks, "ws-a", "same-id", "bob", now),
            PresenceRelease::NotOwner(_)
        ));
        assert!(matches!(
            release_presence(&locks, "ws-a", "same-id", "alice", now),
            PresenceRelease::Released(_)
        ));
        assert!(locks.contains_key(&presence_key("ws-b", "same-id")));
    }

    #[test]
    fn expired_presence_can_be_reclaimed_without_old_owner_deleting_new_owner() {
        let locks = Arc::new(DashMap::new());
        let start = Utc::now();
        let _ = acquire_presence(&locks, "ws", "chunk", "alice", "Alice", start);
        let after_expiry = start + Duration::seconds(CHUNK_LOCK_TTL_SECONDS + 1);
        assert!(matches!(
            acquire_presence(&locks, "ws", "chunk", "bob", "Bob", after_expiry),
            PresenceAcquire::Acquired {
                refreshed: false,
                ..
            }
        ));
        assert!(matches!(
            release_presence(&locks, "ws", "chunk", "alice", after_expiry),
            PresenceRelease::NotOwner(_)
        ));
        let current = locks.get(&presence_key("ws", "chunk")).unwrap();
        assert_eq!(current.owner_user_id, "bob");
    }
}
