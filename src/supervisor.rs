//! P1-7：长寿后台 worker 的 panic 兜底 supervisor。
//!
//! 背景：`main.rs` 用 `tokio::spawn` 拉起一组长驻 worker（受监督名单以本文件
//! [`SUPERVISED_WORKERS`] 常量为唯一权威——注释不复制清单/数量，防止再漂移）。
//! 这些 worker 内部都是 `loop { ... sleep ...}`，
//! 但 future 一旦 panic（非 `Result` 路径，如越界 / unwrap None /
//! `expect` 失败），`JoinHandle` 直接被 drop，worker 静默死亡到下次进程重启
//! 才能恢复。生产里这往往以"为什么 follow-up 任务从昨天起再也不跑"出现。
//!
//! 本模块提供 [`spawn_supervised`]：内部包一层 `loop { catch_unwind(...) }`，
//! panic 时记录 `tracing::error!` + agent_events 一行 + 退避重启；连续两次内
//! 太快 panic 时自动指数退避（1s → 2s → 4s → ... 30s 上限），避免热循环。
//!
//! 适用对象：**长驻 worker**。一次性 best-effort spawn（如 decision_taxonomy
//! 候选 upsert、knowledge_task::schedule_cleanup、replay 并行收割）不接入
//! supervisor，让它们 panic 即死，由调用方下次再 spawn 即可。

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures::FutureExt;
use mongodb::bson::{doc, DateTime};
use tokio::time::sleep;

use crate::routes::AppState;

const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64 = 30;
const FAST_PANIC_WINDOW_SECS: u64 = 60;
const CIRCUIT_OPEN_AFTER_FAST_PANICS: u32 = 5;
const CIRCUIT_POLL_SECONDS: u64 = 30;

pub const SUPERVISED_WORKERS: &[&str] = &[
    "task_worker",
    "inbound_reply_worker",
    "import_worker",
    "outbox_dispatcher",
    "post_decision_worker",
    "media_storage_reconciler",
    "strategic_planner",
    "cold_contact_worker",
    "silence_signal_worker",
    "evolutionary_worker",
    "knowledge_digest_worker",
    "knowledge_task_worker",
    "catalog_rebuild_worker",
    "knowledge_feedback_worker",
    "ingest_worker",
    "management_command_sweeper",
];

fn worker_control_id(worker_name: &str) -> String {
    format!("worker::{worker_name}")
}

fn next_fast_panic_count(previous: u32, elapsed_secs: u64) -> u32 {
    if elapsed_secs >= FAST_PANIC_WINDOW_SECS {
        1
    } else {
        previous.saturating_add(1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CircuitStartPermit {
    Closed,
    Probe { token: String },
}

async fn wait_until_circuit_allows_start(
    state: &AppState,
    worker_name: &str,
) -> CircuitStartPermit {
    let collection = state.db.background_worker_controls();
    loop {
        let now = DateTime::now();
        match collection
            .find_one(doc! { "_id": worker_control_id(worker_name) }, None)
            .await
        {
            Ok(None) => return CircuitStartPermit::Closed,
            Ok(Some(row)) if row.get_str("status").ok() == Some("closed") => {
                return CircuitStartPermit::Closed;
            }
            Ok(Some(row)) if row.get_str("status").ok() == Some("open") => {
                sleep(Duration::from_secs(CIRCUIT_POLL_SECONDS)).await;
            }
            Ok(Some(row))
                if row.get_str("status").ok() == Some("half_open")
                    || (row.get_str("status").ok() == Some("probing")
                        && row
                            .get_datetime("probe_locked_until")
                            .map(|deadline| deadline <= &now)
                            .unwrap_or(true)) =>
            {
                let token = uuid::Uuid::new_v4().to_string();
                let probe_locked_until = DateTime::from_millis(
                    now.timestamp_millis() + FAST_PANIC_WINDOW_SECS as i64 * 2_000,
                );
                let claimed = collection
                    .find_one_and_update(
                        doc! {
                            "_id": worker_control_id(worker_name),
                            "$or": [
                                { "status": "half_open" },
                                {
                                    "status": "probing",
                                    "$or": [
                                        { "probe_locked_until": { "$lte": now } },
                                        { "probe_locked_until": null },
                                        { "probe_locked_until": { "$exists": false } },
                                    ],
                                },
                            ],
                        },
                        doc! { "$set": {
                            "status": "probing",
                            "probe_token": &token,
                            "probe_locked_until": probe_locked_until,
                            "probe_started_at": now,
                            "updated_at": now,
                        } },
                        mongodb::options::FindOneAndUpdateOptions::builder()
                            .return_document(mongodb::options::ReturnDocument::After)
                            .build(),
                    )
                    .await;
                match claimed {
                    Ok(Some(row)) if row.get_str("probe_token").ok() == Some(token.as_str()) => {
                        return CircuitStartPermit::Probe { token };
                    }
                    Ok(_) => sleep(Duration::from_secs(CIRCUIT_POLL_SECONDS)).await,
                    Err(error) => {
                        tracing::error!(worker = worker_name, error = %error, "worker probe claim failed; fail closed");
                        sleep(Duration::from_secs(CIRCUIT_POLL_SECONDS)).await;
                    }
                }
            }
            Ok(Some(_)) => sleep(Duration::from_secs(CIRCUIT_POLL_SECONDS)).await,
            Err(error) => {
                tracing::error!(worker = worker_name, error = %error, "worker circuit state unavailable; fail closed");
                sleep(Duration::from_secs(CIRCUIT_POLL_SECONDS)).await;
            }
        }
    }
}
async fn open_worker_circuit(
    state: &AppState,
    worker_name: &str,
    panic_msg: &str,
    rapid_panics: u32,
    probe_token: Option<&str>,
) -> bool {
    let now = DateTime::now();
    let filter = match probe_token {
        Some(token) => doc! {
            "_id": worker_control_id(worker_name),
            "status": "probing",
            "probe_token": token,
        },
        None => doc! {
            "_id": worker_control_id(worker_name),
            "$or": [
                { "status": "closed" },
                { "status": null },
                { "status": { "$exists": false } },
            ],
        },
    };
    let result = state
        .db
        .background_worker_controls()
        .update_one(
            filter,
            doc! {
                "$set": {
                    "worker_name": worker_name,
                    "status": "open",
                    "rapid_panic_count": rapid_panics as i64,
                    "last_panic": panic_msg,
                    "opened_at": now,
                    "updated_at": now,
                },
                "$setOnInsert": { "created_at": now },
                "$unset": {
                    "probe_token": "",
                    "probe_locked_until": "",
                    "probe_started_at": "",
                },
                "$inc": { "circuit_generation": 1_i64 },
            },
            mongodb::options::UpdateOptions::builder()
                // A normal closed->open transition may create the control row. A probe
                // failure is token-fenced and must never upsert after another owner moved it.
                .upsert(probe_token.is_none())
                .build(),
        )
        .await;
    match result {
        Ok(result) => result.matched_count == 1 || result.upserted_id.is_some(),
        Err(error) => {
            tracing::error!(worker = worker_name, error = %error, "failed to persist worker circuit");
            false
        }
    }
}

async fn mark_worker_recovered(
    state: AppState,
    worker_name: &'static str,
    probe_token: String,
    stabilized: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    sleep(Duration::from_secs(FAST_PANIC_WINDOW_SECS)).await;
    let now = DateTime::now();
    let result = state
        .db
        .background_worker_controls()
        .update_one(
            doc! {
                "_id": worker_control_id(worker_name),
                "status": "probing",
                "probe_token": &probe_token,
            },
            doc! {
                "$set": {
                    "status": "closed",
                    "rapid_panic_count": 0_i64,
                    "recovered_at": now,
                    "updated_at": now,
                },
                "$unset": {
                    "last_panic": "",
                    "probe_token": "",
                    "probe_locked_until": "",
                    "probe_started_at": "",
                },
            },
            None,
        )
        .await;
    if result.is_ok_and(|result| result.modified_count == 1) {
        stabilized.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn should_open_circuit(is_probe: bool, probe_stabilized: bool, rapid_panics: u32) -> bool {
    (is_probe && !probe_stabilized) || rapid_panics >= CIRCUIT_OPEN_AFTER_FAST_PANICS
}

pub async fn resume_worker_circuit(
    state: &AppState,
    worker_name: &str,
    actor: &str,
) -> crate::error::AppResult<bool> {
    if !SUPERVISED_WORKERS.contains(&worker_name) {
        return Err(crate::error::AppError::BadRequest(
            "unknown supervised worker".to_string(),
        ));
    }
    let now = DateTime::now();
    let result = state
        .db
        .background_worker_controls()
        .update_one(
            doc! {
                "_id": worker_control_id(worker_name),
                "status": "open",
            },
            doc! {
                "$set": {
                    "status": "half_open",
                    "resume_requested_by": actor,
                    "resume_requested_at": now,
                    "updated_at": now,
                },
                "$unset": {
                    "probe_token": "",
                    "probe_locked_until": "",
                    "probe_started_at": "",
                }
            },
            None,
        )
        .await?;
    Ok(result.modified_count == 1)
}

/// 拉起一个被 supervisor 包裹的长寿 worker。
///
/// `worker_name` 用于 tracing / agent_events.kind=`background_worker_panic` 写盘，
/// 必须稳定且唯一（建议与 `main.rs` 中调用名一致：`"task_worker"` /
/// `"outbox_dispatcher"` / `"cold_contact_worker"` …）。
///
/// `factory` 闭包每次重启都会被调用一次，返回新的 future。这样 worker 内部
/// 持有 `AppState` clone 也能在 panic 后用一份 fresh state 继续跑（避免内部
/// 缓存被 poisoned 的 `Mutex` 之类）。
///
/// 行为：
/// - future 正常返回（`()`）：视为 worker 主动退出，**不**重启，记录 info 日志；
/// - future panic：写 agent_events `kind="background_worker_panic"` +
///   exponential backoff（首次 1s，每次翻倍，封顶 30s），重新调用 `factory`
///   拿新 future 重启；
/// - 60s 内连续 5 次 panic：持久化 open 熔断并停止重启；管理员请求恢复后只有
///   一个副本可领取 half-open probe，probe 稳定 60s 才闭合，任意 panic 立即重开。
pub fn spawn_supervised<F, Fut>(state: AppState, worker_name: &'static str, factory: F)
where
    F: Fn(AppState) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    debug_assert!(SUPERVISED_WORKERS.contains(&worker_name));
    tokio::spawn(async move {
        let mut backoff_secs = INITIAL_BACKOFF_SECS;
        let mut rapid_panics = 0_u32;
        loop {
            let permit = wait_until_circuit_allows_start(&state, worker_name).await;
            let probe_token = match &permit {
                CircuitStartPermit::Closed => None,
                CircuitStartPermit::Probe { token } => Some(token.clone()),
            };
            let probe_stabilized = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let recovery_marker = match permit {
                CircuitStartPermit::Closed => None,
                CircuitStartPermit::Probe { token } => Some(tokio::spawn(mark_worker_recovered(
                    state.clone(),
                    worker_name,
                    token,
                    probe_stabilized.clone(),
                ))),
            };

            let started_at = std::time::Instant::now();
            let fut = factory(state.clone());
            let result = AssertUnwindSafe(fut).catch_unwind().await;
            if let Some(handle) = recovery_marker {
                handle.abort();
            }
            match result {
                Ok(()) => {
                    tracing::info!(
                        worker = worker_name,
                        "background worker exited normally; not restarting"
                    );
                    return;
                }
                Err(panic_payload) => {
                    let panic_msg = panic_payload_to_string(&panic_payload);
                    let elapsed = started_at.elapsed().as_secs();
                    rapid_panics = next_fast_panic_count(rapid_panics, elapsed);
                    if elapsed >= FAST_PANIC_WINDOW_SECS {
                        backoff_secs = INITIAL_BACKOFF_SECS;
                    }
                    // A half-open probe only has special semantics until its token-fenced
                    // 60-second stability transition succeeds. Later panics are ordinary failures.
                    let probe_failed = probe_token.is_some()
                        && !probe_stabilized.load(std::sync::atomic::Ordering::SeqCst);
                    let opening =
                        should_open_circuit(probe_token.is_some(), !probe_failed, rapid_panics);
                    tracing::error!(
                        worker = worker_name,
                        elapsed_secs = elapsed,
                        backoff_secs,
                        rapid_panics,
                        circuit_opening = opening,
                        panic = %panic_msg,
                        "background worker panicked"
                    );
                    let event_kind = if opening {
                        "background_worker_circuit_open"
                    } else {
                        "background_worker_panic"
                    };
                    let _ = crate::agent::write_event_for_account(
                        &state,
                        &state.config.default_workspace_id,
                        "system",
                        None,
                        event_kind,
                        "warning",
                        &format!(
                            "worker={worker_name} elapsed_secs={elapsed} rapid_panics={rapid_panics} backoff_secs={backoff_secs} panic={panic_msg}"
                        ),
                        None,
                    )
                    .await;
                    if opening {
                        // Probe failures are fenced by their exact token; a stale probe cannot
                        // overwrite a newer resume/probe owner. Normal failures create/update by id.
                        let _ = open_worker_circuit(
                            &state,
                            worker_name,
                            &panic_msg,
                            rapid_panics,
                            probe_failed.then_some(probe_token.as_deref()).flatten(),
                        )
                        .await;
                        rapid_panics = 0;
                        backoff_secs = INITIAL_BACKOFF_SECS;
                        continue;
                    }
                    sleep(Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                }
            }
        }
    });
}

fn panic_payload_to_string(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// 不直接构造 AppState（耗时且需要 Mongo），只验证 panic_payload_to_string
    /// 和 backoff 算式两个纯函数语义。supervised 行为本身在集成测试覆盖。
    #[test]
    fn payload_str_literal_decodes() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_payload_to_string(&payload), "boom");
    }

    #[test]
    fn payload_string_decodes() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("dynamic".to_string());
        assert_eq!(panic_payload_to_string(&payload), "dynamic");
    }

    #[test]
    fn payload_unknown_returns_placeholder() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42_i32);
        assert_eq!(
            panic_payload_to_string(&payload),
            "<non-string panic payload>"
        );
    }

    #[test]
    fn rapid_panic_counter_resets_after_stable_window() {
        assert_eq!(next_fast_panic_count(4, FAST_PANIC_WINDOW_SECS), 1);
        assert_eq!(next_fast_panic_count(4, FAST_PANIC_WINDOW_SECS - 1), 5);
    }

    #[test]
    fn circuit_start_permit_distinguishes_closed_and_fenced_probe() {
        assert_ne!(
            CircuitStartPermit::Closed,
            CircuitStartPermit::Probe {
                token: "probe-a".to_string()
            }
        );
        assert_eq!(worker_control_id("task_worker"), "worker::task_worker");
    }

    #[test]
    fn half_open_probe_only_reopens_before_stabilization() {
        assert!(should_open_circuit(true, false, 1));
        assert!(!should_open_circuit(true, true, 1));
        assert!(should_open_circuit(
            true,
            true,
            CIRCUIT_OPEN_AFTER_FAST_PANICS
        ));
        assert!(!should_open_circuit(
            false,
            false,
            CIRCUIT_OPEN_AFTER_FAST_PANICS - 1
        ));
    }

    #[test]
    fn supervised_worker_names_are_unique() {
        let unique = SUPERVISED_WORKERS
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), SUPERVISED_WORKERS.len());
        assert!(SUPERVISED_WORKERS.contains(&"management_command_sweeper"));
    }

    #[test]
    fn backoff_doubles_until_cap() {
        let mut b = INITIAL_BACKOFF_SECS;
        let mut steps = vec![b];
        for _ in 0..10 {
            b = (b * 2).min(MAX_BACKOFF_SECS);
            steps.push(b);
        }
        assert_eq!(steps[0], 1);
        assert_eq!(steps[1], 2);
        assert_eq!(steps[2], 4);
        assert_eq!(steps[3], 8);
        assert_eq!(steps[4], 16);
        assert_eq!(steps[5], MAX_BACKOFF_SECS);
        assert_eq!(*steps.last().unwrap(), MAX_BACKOFF_SECS);
    }

    /// 简单验证 spawn_supervised 在 panic 后能重启；用 AtomicU32 计数 factory
    /// 调用次数。AppState clone 不便构造，所以这里直接用一个 standalone 的
    /// 缩小版 supervisor 验证语义（与生产路径同构造）。
    #[tokio::test]
    async fn supervised_loop_restarts_after_panic() {
        let counter = Arc::new(AtomicU32::new(0));
        let counter_for_factory = counter.clone();
        let handle = tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF_SECS;
            for _ in 0..3 {
                let counter = counter_for_factory.clone();
                let fut = async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    if n < 2 {
                        panic!("synthetic panic #{n}");
                    }
                };
                let result = AssertUnwindSafe(fut).catch_unwind().await;
                if result.is_ok() {
                    return;
                }
                sleep(Duration::from_millis(1)).await;
                backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
                let _ = backoff;
            }
        });
        handle.await.unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
