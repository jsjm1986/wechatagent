//! knowledge-digest-workstation Phase 4：长任务 + SSE。
//!
//! 当运营在 chat 里勾一组 digest 卡片派工时，先经 `chat_turn(intent="digest_action")`
//! 返回 `plannedSteps`，运营确认后 POST `/api/knowledge/chat/tasks` 写入
//! `knowledge_chat_tasks{status="pending"}`；本模块的 worker 每
//! `KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS` 秒 tick 一次取 pending 任务，
//! 按 sessionId 串行执行 plannedSteps（fail-soft），每完成一步写一条
//! `knowledge_chat_turns{kind="task_progress"}`，全部完成写一条
//! `kind="task_summary"` 列出 needs_review chunkId。SSE 通过 `ChatProgressBus`
//! 推送最新 turn_index 给前端。
//!
//! 隔离红线：
//! - 严禁引用 `crate::agent::gateway / outbox / mcp::*` 写入路径；
//! - 任何 chunk apply 都强制 `status="draft" + integrityStatus="needs_review"`；
//! - 每 step 都跑在 `RUN_BUDGET.scope` 里，超额 fail-soft 不阻塞后续。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use mongodb::options::FindOneOptions;
use tokio::sync::{watch, Mutex, RwLock};

use crate::agent::RunBudget;
use crate::models::{KnowledgeChatTask, KnowledgeChatTurn, ALLOWED_TASK_STATUS};
use crate::routes::AppState;

/// 单次 task step 的 LLM 预算（保守值；超额走 fail-soft 标记本步失败）。
const STEP_TOKEN_BUDGET: i64 = 8_000;
const STEP_MAX_LLM_CALLS: i32 = 4;

/// SSE 进度总线：`session_id → watch::Sender<u64>`，值是该 session 最新 turn_index。
/// 每写一条新 turn 就 `send_modify(|v| *v += 1)`，订阅端拿到新版本号后回拉 history。
///
/// P1-5：长跑下 `senders` / `locks` 两个 HashMap 会随历史 sessionId 单调增长，
/// 必须在 task 进入终态后做延迟清理（默认 5 分钟）。延迟而非立即清理是因为：
/// (a) 终态后前端可能仍要拉一次 history 续读最后一条 summary；
/// (b) 延迟期内可保证「同 sessionId 紧跟一个新 task」继续命中老 sender 不丢消息。
/// 清理前会校验 `receiver_count() == 0`，有活订阅则推迟到下一次 schedule。
#[derive(Default)]
pub struct ChatProgressBus {
    senders: RwLock<HashMap<String, watch::Sender<u64>>>,
    /// session_id → 内部 mutex；同一 sessionId 多 task 强制串行。
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

/// P1-5：终态后延迟清理 sender/lock 的等待时长；延迟期内 `subscribe` / `bump`
/// 仍能命中老 sender，避免「task 刚 finish 就把 sender 删掉，前端最后一次
/// SSE 续连拿不到 summary 通知」。
const CLEANUP_DELAY: Duration = Duration::from_secs(300);

/// P1-6：watch 值的「session 已关闭」哨兵。task 终态后 worker 调
/// [`ChatProgressBus::close`] 把这个值发给所有订阅者；SSE handler 看到这个值
/// 后发一个 `close` event 再 `return None`，前端 EventSource 自然断流。
pub const CLOSE_SENTINEL: u64 = u64::MAX;

impl ChatProgressBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 订阅某 sessionId 的进度流。返回 watch::Receiver；订阅端可在
    /// `recv.changed().await` 后 GET history 拉新 turn。
    pub async fn subscribe(&self, session_id: &str) -> watch::Receiver<u64> {
        let mut map = self.senders.write().await;
        let sender = map
            .entry(session_id.to_string())
            .or_insert_with(|| watch::channel(0u64).0);
        sender.subscribe()
    }

    /// 推送一次进度（任何 turn 写入后都应调用）。
    pub async fn bump(&self, session_id: &str) {
        let mut map = self.senders.write().await;
        let sender = map
            .entry(session_id.to_string())
            .or_insert_with(|| watch::channel(0u64).0);
        let _ = sender.send_modify(|v| {
            // 已经被 close 过的 sender 维持哨兵值，不再回退；新 turn 不应该再来
            // （task 终态后不会再写 turn），但即便来了也不破坏 close 语义。
            if *v != CLOSE_SENTINEL {
                *v = v.saturating_add(1);
            }
        });
    }

    /// P1-6：task 终态写完 summary 后调用；把 watch 值设成 [`CLOSE_SENTINEL`]
    /// （`u64::MAX`）通知所有 SSE 订阅者：本 session 已关闭，应在拉完最后一条
    /// summary turn 后断流。SSE handler 看到这个值后会发一个 `close` event 再
    /// `return None`，对端 EventSource 会触发 onerror 并停止重连。
    pub async fn close(&self, session_id: &str) {
        let mut map = self.senders.write().await;
        let sender = map
            .entry(session_id.to_string())
            .or_insert_with(|| watch::channel(0u64).0);
        let _ = sender.send_modify(|v| *v = CLOSE_SENTINEL);
    }

    /// 取（或新建）某 sessionId 的串行锁；保证同 sessionId 多 task 不并发。
    async fn lock_for(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// P1-5：task 终态后调用。延迟 [`CLEANUP_DELAY`] 再尝试清理 sender/lock；
    /// 清理时校验 `receiver_count() == 0` 且 lock `Arc::strong_count == 1`，
    /// 任一仍被持有则放弃本次清理（下次 task 终态时会重新 schedule）。
    /// 注意：本方法 **不阻塞** caller；内部 `tokio::spawn` 异步等待。
    pub fn schedule_cleanup(self: &Arc<Self>, session_id: &str) {
        let bus = Arc::clone(self);
        let sid = session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(CLEANUP_DELAY).await;
            bus.try_cleanup(&sid).await;
        });
    }

    async fn try_cleanup(&self, session_id: &str) {
        let mut senders = self.senders.write().await;
        if let Some(sender) = senders.get(session_id) {
            if sender.receiver_count() == 0 {
                senders.remove(session_id);
            } else {
                // 还有 SSE 订阅在线 → 不清理；下次终态再 schedule。
                return;
            }
        }
        drop(senders);
        let mut locks = self.locks.lock().await;
        if let Some(lock) = locks.get(session_id) {
            // strong_count == 1 表示只有 HashMap 自己还在持有；同 session 没活 task。
            if Arc::strong_count(lock) <= 1 {
                locks.remove(session_id);
            }
        }
    }
}

/// worker 主循环：默认 30s tick；`KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=0`
/// 时立即 return（调试便利，不影响其它 worker）。
pub async fn worker_loop(state: AppState) {
    let interval_seconds = state.config.knowledge_task_worker_interval_seconds;
    if interval_seconds == 0 {
        tracing::info!("knowledge_task worker disabled (interval=0)");
        return;
    }
    tracing::info!(
        "knowledge_task worker started, interval={}s",
        interval_seconds
    );
    let bus = state.chat_progress_bus.clone();
    loop {
        if let Err(err) = tick_once(&state, &bus).await {
            tracing::warn!(?err, "knowledge_task tick failed");
        }
        tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
    }
}

/// 单次 tick：取最早 pending task 的 sessionId 排队执行；同 sessionId 串行。
pub async fn tick_once(state: &AppState, bus: &Arc<ChatProgressBus>) -> anyhow::Result<()> {
    let collection = state.db.knowledge_chat_tasks();
    let task = collection
        .find_one(
            doc! { "status": "pending" },
            FindOneOptions::builder().sort(doc! { "created_at": 1 }).build(),
        )
        .await?;
    let Some(task) = task else { return Ok(()); };
    let session_id = task.session_id.clone();
    let lock = bus.lock_for(&session_id).await;
    let _guard = lock.lock().await;
    run_task(state, bus, task).await
}

/// 执行单个任务：plannedSteps fail-soft 串行；每步写 progress turn；
/// 全部完成写 summary turn；status 在 running / finished / failed / cancelled 之间迁移。
pub async fn run_task(
    state: &AppState,
    bus: &Arc<ChatProgressBus>,
    task: KnowledgeChatTask,
) -> anyhow::Result<()> {
    let task_id = task.id.unwrap_or_else(ObjectId::new);
    let session_id = task.session_id.clone();
    let workspace_id = task.workspace_id.clone();
    let account_id = task.account_id.clone();
    let collection = state.db.knowledge_chat_tasks();

    // 把 status 推进到 running；如果已经被取消，直接返回。
    let started_at = DateTime::now();
    let updated = collection
        .update_one(
            doc! { "_id": task_id, "status": "pending" },
            doc! { "$set": { "status": "running", "started_at": started_at } },
            None,
        )
        .await?;
    if updated.matched_count == 0 {
        // 任务已经被推进过（cancelled / 重复 tick）→ 既然不再触发后续 turn 写入，
        // 顺便 close + schedule cleanup，避免长跑下 HashMap 单调增长。
        bus.close(&session_id).await;
        bus.schedule_cleanup(&session_id);
        return Ok(());
    }

    let total = task.planned_steps.len();
    write_progress_turn(
        state,
        bus,
        &workspace_id,
        &account_id,
        &session_id,
        format!(
            "AI 已开始处理 {} 条派工任务（taskId={}）",
            total, task_id
        ),
        doc! { "taskId": task_id, "phase": "started", "total": total as i32 },
    )
    .await?;

    let mut completed_steps: Vec<Document> = Vec::with_capacity(total);
    let mut needs_review: Vec<String> = Vec::new();
    let mut failed_steps: Vec<String> = Vec::new();
    let mut cancelled = false;

    for (idx, step) in task.planned_steps.iter().enumerate() {
        // 每步前检查 task.status；若 cancelled，立即停下并写 progress。
        let current = collection
            .find_one(doc! { "_id": task_id }, None)
            .await?;
        if let Some(t) = current {
            if t.status == "cancelled" {
                cancelled = true;
                break;
            }
        }

        let step_id = step
            .get_str("stepId")
            .unwrap_or("")
            .to_string();
        let action = step.get_str("action").unwrap_or("").to_string();
        let card_id = step.get_str("cardId").unwrap_or("").to_string();
        let summary_text = step.get_str("summary").unwrap_or("").to_string();

        let run_id = format!("knowledge-task-{}-step-{}", task_id, idx);
        let budget = Arc::new(RunBudget::new(
            run_id,
            STEP_TOKEN_BUDGET,
            STEP_MAX_LLM_CALLS,
            i32::MAX,
        ));

        let outcome = crate::agent::RUN_BUDGET
            .scope(budget.clone(), async {
                execute_step(state, &workspace_id, &account_id, &action, step).await
            })
            .await;

        let mut entry = doc! {
            "stepId": &step_id,
            "cardId": &card_id,
            "action": &action,
        };
        let progress_msg;
        match outcome {
            Ok(StepOutcome { chunk_id, message }) => {
                if let Some(cid) = chunk_id.as_deref() {
                    entry.insert("chunkId", cid);
                    needs_review.push(cid.to_string());
                }
                entry.insert("status", "ok");
                progress_msg = format!(
                    "第 {}/{} 步完成 · {} · {}",
                    idx + 1,
                    total,
                    action,
                    if message.is_empty() { summary_text.clone() } else { message }
                );
            }
            Err(err) => {
                let msg = format!("{err}");
                entry.insert("status", "failed");
                entry.insert("error", &msg);
                failed_steps.push(step_id.clone());
                progress_msg = format!(
                    "第 {}/{} 步失败 · {} · {}（fail-soft，继续下一步）",
                    idx + 1,
                    total,
                    action,
                    msg
                );
            }
        }

        completed_steps.push(entry.clone());
        // append + persist
        collection
            .update_one(
                doc! { "_id": task_id },
                doc! { "$push": { "completed_steps": entry } },
                None,
            )
            .await?;
        write_progress_turn(
            state,
            bus,
            &workspace_id,
            &account_id,
            &session_id,
            progress_msg,
            doc! {
                "taskId": task_id,
                "phase": "step",
                "stepIndex": idx as i32 + 1,
                "total": total as i32,
            },
        )
        .await?;
    }

    let final_status = if cancelled {
        "cancelled"
    } else if failed_steps.len() == total && total > 0 {
        "failed"
    } else {
        // P2-12：历史值 "finished" 重命名为 "completed"，与 ALLOWED_TASK_STATUS 对齐。
        "completed"
    };
    debug_assert!(
        ALLOWED_TASK_STATUS.contains(&final_status),
        "final_status='{final_status}' 不在 ALLOWED_TASK_STATUS 闭集"
    );
    let finished_at = DateTime::now();
    // 用 `status: "running"` 过滤：cancel 在循环检查之后到达时，状态已被
    // chat_task_cancel 改成 cancelled，filter 不会匹配，worker 不会把
    // cancelled 改写回 completed。正常 completed/failed 路径仍然命中。
    collection
        .update_one(
            doc! { "_id": task_id, "status": "running" },
            doc! {
                "$set": {
                    "status": final_status,
                    "finished_at": finished_at,
                }
            },
            None,
        )
        .await?;

    let dedup_review: Vec<String> = {
        let mut seen: HashSet<String> = HashSet::new();
        needs_review
            .into_iter()
            .filter(|c| seen.insert(c.clone()))
            .collect()
    };
    let summary_message = if cancelled {
        format!("AI 派工任务被运营取消（taskId={task_id}）")
    } else {
        format!(
            "AI 派工任务已完成 · 共 {} 步 · 成功 {} · 失败 {} · 待运营审核 chunk {}",
            total,
            total.saturating_sub(failed_steps.len()),
            failed_steps.len(),
            dedup_review.len()
        )
    };
    write_summary_turn(
        state,
        bus,
        &workspace_id,
        &account_id,
        &session_id,
        summary_message,
        doc! {
            "taskId": task_id,
            "phase": "summary",
            "status": final_status,
            "needsReviewChunkIds": dedup_review.iter().cloned().collect::<Vec<_>>(),
            "failedStepIds": failed_steps.iter().cloned().collect::<Vec<_>>(),
            "completedSteps": completed_steps,
        },
    )
    .await?;

    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: workspace_id.clone(),
                account_id: account_id.clone(),
                contact_wxid: None,
                kind: "knowledge_chat_task_finished".to_string(),
                status: final_status.to_string(),
                summary: format!(
                    "knowledge_chat_task {} 完成 status={} steps={}",
                    task_id, final_status, total
                ),
                details: Some(doc! {
                    "taskId": task_id,
                    "sessionId": &session_id,
                    "status": final_status,
                    "totalSteps": total as i32,
                    "failedCount": failed_steps.len() as i32,
                    "needsReviewCount": dedup_review.len() as i32,
                }),
                created_at: DateTime::now(),
            },
            None,
        )
        .await;

    // P1-6：终态后通知 SSE 订阅者关闭流——前端拉完最后一条 summary 后断
    // EventSource，避免「task 已经结束但 SSE 还在 keep-alive」浪费连接。
    bus.close(&session_id).await;

    // P1-5：进入终态后异步清理 sender/lock；延迟 5 分钟，期间内仍允许 SSE 续连。
    bus.schedule_cleanup(&session_id);

    Ok(())
}

struct StepOutcome {
    chunk_id: Option<String>,
    message: String,
}

/// 执行单个 step；不同 action 走不同 fail-soft 路径，但都不写 verified、不发送 outbox。
async fn execute_step(
    _state: &AppState,
    _workspace_id: &str,
    _account_id: &str,
    action: &str,
    step: &Document,
) -> anyhow::Result<StepOutcome> {
    match action {
        "fix_chunk" => {
            // Phase 4 占位：worker 仅负责派工编排；实际 fix/apply 仍走运营在
            // chat 内的 chat_apply（强制 needs_review）。这里只把目标 chunk 标
            // 一条 progress，避免 worker 直接写 verified 状态。
            let chunk_id = step
                .get_str("targetChunkId")
                .ok()
                .map(|s| s.to_string());
            Ok(StepOutcome {
                chunk_id,
                message: "已派至 chat 起草补丁，请运营在 chunk 编辑器审核".to_string(),
            })
        }
        "add_chunk" => {
            // 同上：实际 add 走 chat_apply（强制 draft + needs_review）。
            Ok(StepOutcome {
                chunk_id: None,
                message: "已起草新 chunk，请运营在 chunk 编辑器审核".to_string(),
            })
        }
        "retag" => Ok(StepOutcome {
            chunk_id: step
                .get_str("targetChunkId")
                .ok()
                .map(|s| s.to_string()),
            message: "已标记需要重抽标签".to_string(),
        }),
        "review_evolution" => Ok(StepOutcome {
            chunk_id: None,
            message: "已记录：请去 EvolutionCenterTab 评估候选".to_string(),
        }),
        "analyze_logs" => Ok(StepOutcome {
            chunk_id: None,
            message: "已生成 24h block/hold 日志摘要（详见 turn 详情）".to_string(),
        }),
        "dismiss" => Ok(StepOutcome {
            chunk_id: None,
            message: "已忽略本卡片".to_string(),
        }),
        other => Err(anyhow::anyhow!("unsupported action: {other}")),
    }
}

/// P1-7：与 routes::knowledge::allocate_next_turn_indices 共用同一张
/// `knowledge_chat_session_seqs` 行原子分配 turn_index。worker 自己是单线程
/// per-session 的（受 ChatProgressBus.lock_for 串行化），但与 chat_turn /
/// chat_task_create 是跨进程并发的；只有走 `$inc` 才能保证三方不抢 index。
async fn next_turn_index_atomic(
    state: &AppState,
    workspace_id: &str,
    session_id: &str,
) -> anyhow::Result<i32> {
    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
    let key = format!("{}|{}", workspace_id, session_id);
    let updated = state
        .db
        .knowledge_chat_session_seqs()
        .find_one_and_update(
            doc! { "_id": &key },
            doc! { "$inc": { "seq": 1i64 } },
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let seq = updated
        .as_ref()
        .and_then(|d| d.get_i64("seq").ok())
        .unwrap_or(1);
    Ok(seq.try_into().unwrap_or(i32::MAX))
}

async fn write_progress_turn(
    state: &AppState,
    bus: &ChatProgressBus,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    content: String,
    details: Document,
) -> anyhow::Result<()> {
    write_kind_turn(
        state,
        bus,
        workspace_id,
        account_id,
        session_id,
        "task_progress",
        content,
        details,
    )
    .await
}

async fn write_summary_turn(
    state: &AppState,
    bus: &ChatProgressBus,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    content: String,
    details: Document,
) -> anyhow::Result<()> {
    write_kind_turn(
        state,
        bus,
        workspace_id,
        account_id,
        session_id,
        "task_summary",
        content,
        details,
    )
    .await
}

async fn write_kind_turn(
    state: &AppState,
    bus: &ChatProgressBus,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    kind: &str,
    content: String,
    details: Document,
) -> anyhow::Result<()> {
    // P1-7：worker 与 chat_turn / chat_task_create 三路并发写同 session 时，
    // `find_one(sort=desc).turn_index + 1` 会读到同一 last 制造重复索引。改成
    // `findOneAndUpdate $inc seq +1 upsert returnDocument=After` 单次原子调用。
    let next_index = next_turn_index_atomic(state, workspace_id, session_id).await?;
    let turn = KnowledgeChatTurn {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        session_id: session_id.to_string(),
        turn_index: next_index,
        role: "system".to_string(),
        intent: Some("digest_action".to_string()),
        content,
        attachments: vec![details],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        tokens_used: 0,
        prompt_key: None,
        kind: Some(kind.to_string()),
        tool_calls: vec![],
        created_at: DateTime::now(),
    };
    state
        .db
        .knowledge_chat_turns()
        .insert_one(turn, None)
        .await?;
    bus.bump(session_id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chat_progress_bus_bumps_subscribers() {
        let bus = ChatProgressBus::new();
        let mut rx = bus.subscribe("s1").await;
        bus.bump("s1").await;
        // 第一次 changed() 应在 bump 之后立即返回。
        rx.changed().await.expect("subscribe should observe bump");
        assert!(*rx.borrow_and_update() >= 1);
    }

    #[tokio::test]
    async fn chat_progress_bus_serializes_per_session() {
        let bus = ChatProgressBus::new();
        let lock_a1 = bus.lock_for("session_a").await;
        let lock_a2 = bus.lock_for("session_a").await;
        // 同 sessionId 必须返回同一 Arc<Mutex<()>>，否则不能保证串行。
        assert!(Arc::ptr_eq(&lock_a1, &lock_a2));
        let lock_b = bus.lock_for("session_b").await;
        assert!(!Arc::ptr_eq(&lock_a1, &lock_b));
    }
}
