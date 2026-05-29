//! P1-7：长寿后台 worker 的 panic 兜底 supervisor。
//!
//! 背景：`main.rs` 用 `tokio::spawn` 拉起 8 个长驻 worker（task / outbox /
//! planner / cold_contact / evolution / knowledge_digest / knowledge_task /
//! catalog_rebuild / feedback）。这些 worker 内部都是 `loop { ... sleep ...}`，
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
use tokio::time::sleep;

use crate::routes::AppState;

const INITIAL_BACKOFF_SECS: u64 = 1;
const MAX_BACKOFF_SECS: u64 = 30;
const FAST_PANIC_WINDOW_SECS: u64 = 60;

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
/// - 60s 内连续 panic：保留指数退避；超过 60s 没 panic 视为已稳定，退避计数器
///   归零。
pub fn spawn_supervised<F, Fut>(state: AppState, worker_name: &'static str, factory: F)
where
    F: Fn(AppState) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let mut backoff_secs = INITIAL_BACKOFF_SECS;
        loop {
            let started_at = std::time::Instant::now();
            let fut = factory(state.clone());
            let result = AssertUnwindSafe(fut).catch_unwind().await;
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
                    if elapsed >= FAST_PANIC_WINDOW_SECS {
                        backoff_secs = INITIAL_BACKOFF_SECS;
                    }
                    tracing::error!(
                        worker = worker_name,
                        elapsed_secs = elapsed,
                        backoff_secs,
                        panic = %panic_msg,
                        "background worker panicked; restarting after backoff"
                    );
                    let _ = crate::agent::write_event_for_account(
                        &state,
                        "system",
                        None,
                        "background_worker_panic",
                        "warning",
                        &format!(
                            "worker={worker_name} elapsed_secs={elapsed} backoff_secs={backoff_secs} panic={panic_msg}"
                        ),
                        None,
                    )
                    .await;
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
