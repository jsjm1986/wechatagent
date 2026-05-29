//! Phase G P1-4：知识 chunk 软锁 + 事件总线 + WebSocket 推送。
//!
//! ## 形态
//! - 锁：进程内 `DashMap<chunk_id, ChunkEditLock>`；TTL 5 分钟，进程重启即清。
//!   - 同 owner 调 acquire/heartbeat 视为续期；非 owner 调撞锁返回 409。
//!   - release 仅 owner 可调；TTL 兜底防忘释放。
//! - 事件：`tokio::sync::broadcast::Sender<ChunkEvent>`；订阅端：WebSocket。
//!   - 进程内多副本广播；多进程部署需要 Redis pub/sub —— 在 Out-of-scope 留 P2。
//! - 路由：
//!   - `POST   /api/operation-knowledge/chunks/:id/lock`（acquire / 续期）
//!   - `DELETE /api/operation-knowledge/chunks/:id/lock`（release）
//!   - `GET    /api/ws/chunks`（WebSocket，server-push 事件流）
//!
//! ## 与 patch/archive/restore/... 的耦合
//! `apply_chunk_revision` 没有改造空间；本文件只在 9 个 chunk 编辑路由的 handler
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
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::auth::AuthenticatedAdmin;

use super::AppState;

/// 软锁 TTL：单次 acquire / heartbeat 后允许的占用时长。前端心跳间隔建议 60s。
pub const CHUNK_LOCK_TTL_SECONDS: i64 = 300;

/// 事件 broadcast 通道容量。订阅者跟不上时会丢老事件——锁/版本场景容忍丢，
/// 客户端通过 reload 自愈。
pub const CHUNK_EVENT_CHANNEL_CAPACITY: usize = 256;

/// 当前持有 chunk 编辑锁的会话信息。
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

/// AppState 持有的 chunk 锁表。key=chunk_id（同一个 chunk 在多 workspace 共享 ObjectId
/// 在数据库里就不应该出现，故只用 chunk_id 做 key 安全）。
pub type ChunkLockMap = Arc<DashMap<String, ChunkEditLock>>;

#[derive(Debug, Deserialize)]
pub struct LockAcquireRequest {
    /// 可选 actor 显示名；默认取 admin.username。
    pub actor_label: Option<String>,
}

/// `POST /operation-knowledge/chunks/:id/lock` — 获取或续期软锁。
///
/// 返回：200 + `{lock: ChunkEditLock, refreshed: bool}` 当当前 admin 持锁；
/// 409 + `{error, lock}` 当其它 admin 持锁；其它情况按 AppError 转译。
pub async fn acquire_chunk_lock(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(chunk_id): Path<String>,
    Json(payload): Json<LockAcquireRequest>,
) -> impl IntoResponse {
    let now = Utc::now();
    let ttl = Duration::seconds(CHUNK_LOCK_TTL_SECONDS);
    let actor_label = payload.actor_label.unwrap_or_else(|| admin.username.clone());

    // 先尝试拿到现有锁的快照判断 owner / 过期
    let existing = state
        .chunk_locks
        .get(&chunk_id)
        .map(|r| r.value().clone());

    if let Some(lock) = existing {
        if !lock.is_expired(now) && lock.owner_user_id != admin.user_id {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "chunk_locked_by_other",
                    "lock": lock,
                })),
            )
                .into_response();
        }
    }

    // 此时锁不存在 / 已过期 / 是自己持有 —— 全都续期或新建
    let new_lock = ChunkEditLock {
        chunk_id: chunk_id.clone(),
        workspace_id: admin.current_workspace.clone(),
        owner_user_id: admin.user_id.clone(),
        owner_username: actor_label,
        locked_at: now,
        expires_at: now + ttl,
    };
    let refreshed = state
        .chunk_locks
        .insert(chunk_id.clone(), new_lock.clone())
        .is_some();

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
        })),
    )
        .into_response()
}

/// `DELETE /operation-knowledge/chunks/:id/lock` — 释放软锁。
///
/// 仅锁的 owner 可释放；其它人调用返回 403。锁已不存在视为成功（幂等）。
pub async fn release_chunk_lock(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(chunk_id): Path<String>,
) -> impl IntoResponse {
    let snapshot = state.chunk_locks.get(&chunk_id).map(|r| r.value().clone());
    match snapshot {
        None => (
            StatusCode::OK,
            Json(json!({"released": false, "reason": "not_locked"})),
        )
            .into_response(),
        Some(lock) if lock.owner_user_id != admin.user_id => (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "lock_owned_by_other",
                "lock": lock,
            })),
        )
            .into_response(),
        Some(lock) => {
            state.chunk_locks.remove(&chunk_id);
            let _ = state.chunk_event_bus.send(ChunkEvent::Unlocked {
                chunk_id: chunk_id.clone(),
                workspace_id: lock.workspace_id.clone(),
                owner_user_id: lock.owner_user_id.clone(),
            });
            (StatusCode::OK, Json(json!({"released": true}))).into_response()
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
}
