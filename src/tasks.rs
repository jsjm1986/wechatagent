use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime, Document},
    options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument},
};
use tokio::time::sleep;

use crate::{agent, error::AppResult, models::assert_agent_task_status_valid, routes::AppState};

/// 一次 AgentTask claim 的不可转移所有权凭证。token 每次 claim 都重新生成；
/// generation 只落库审计，正确性只依赖 `_id + status=running + claim_token` CAS。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskClaim {
    pub task_id: ObjectId,
    pub claim_token: String,
    /// 每次 claim 原子递增。token 负责不可伪造所有权，generation 负责把晚到业务
    /// 投影做成单调写，避免旧执行者覆盖新执行者已经提交的结果。
    pub claim_generation: i64,
}

impl TaskClaim {
    pub fn owned_running_filter(&self) -> Document {
        doc! {
            "_id": self.task_id,
            "status": "running",
            "claim_token": &self.claim_token,
            "claim_generation": self.claim_generation,
        }
    }

    pub fn committing_filter(&self) -> Document {
        doc! {
            "_id": self.task_id,
            "status": "committing",
            "claim_token": &self.claim_token,
            "claim_generation": self.claim_generation,
        }
    }
}

/// Linearization point for non-send task side effects. Cancellation and this CAS compete on the
/// same task document. Once `committing` wins, the prepared payload is durable and can be replayed
/// idempotently after a crash; cancellation deliberately no longer accepts this state.
pub async fn prepare_task_commit_if_owned(
    state: &AppState,
    claim: &TaskClaim,
    commit_kind: &str,
    payload: Document,
) -> AppResult<bool> {
    assert_agent_task_status_valid("committing");
    let result = state
        .db
        .tasks()
        .update_one(
            claim.owned_running_filter(),
            doc! {
                "$set": {
                    "status": "committing",
                    "gateway_status": format!("{commit_kind}_committing"),
                    "prepared_commit_kind": commit_kind,
                    "prepared_commit": payload,
                    "updated_at": DateTime::now(),
                },
                "$unset": { "claimed_at": "" },
            },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

pub async fn finalize_task_commit_if_owned(
    state: &AppState,
    claim: &TaskClaim,
    gateway_status: &str,
) -> AppResult<bool> {
    assert_agent_task_status_valid("sent");
    let result = state
        .db
        .tasks()
        .update_one(
            claim.committing_filter(),
            doc! {
                "$set": {
                    "status": "sent",
                    "gateway_status": gateway_status,
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "claim_token": "",
                    "prepared_commit_kind": "",
                    "prepared_commit": "",
                },
            },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

/// A prepared commit may discover that its optimistic business precondition was superseded
/// before any target-side write occurred. Return that exact commit to `retry` so a later claim can
/// recompute from fresh state; the committing token is still required for the transition.
pub async fn requeue_task_commit_if_owned(
    state: &AppState,
    claim: &TaskClaim,
    gateway_status: &str,
) -> AppResult<bool> {
    assert_agent_task_status_valid("retry");
    let result = state
        .db
        .tasks()
        .update_one(
            claim.committing_filter(),
            doc! {
                "$set": {
                    "status": "retry",
                    "gateway_status": gateway_status,
                    "next_retry_at": DateTime::now(),
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "claim_token": "",
                    "prepared_commit_kind": "",
                    "prepared_commit": "",
                },
            },
            None,
        )
        .await?;
    Ok(result.matched_count == 1)
}

fn task_outbox_marker_prepare_filter(decision_id: ObjectId, claim_token: &str) -> Document {
    doc! {
        "decision_id": decision_id,
        "$or": [
            { "task_send_authorization_token": { "$exists": false } },
            { "task_send_authorization_token": null },
            { "task_send_authorization_token": claim_token },
        ],
    }
}

fn task_outbox_commit_filter(claim: &TaskClaim, decision_id: ObjectId) -> Document {
    let mut filter = claim.owned_running_filter();
    filter.insert("outbox_decision_id", decision_id);
    filter
}

fn task_claim_send_terminal_filter(claim: &TaskClaim) -> Document {
    doc! {
        "_id": claim.task_id,
        "status": { "$in": ["outbox_enqueued", "sent"] },
        "claim_token": &claim.claim_token,
        "claim_generation": claim.claim_generation,
    }
}

/// 任务处理器的执行上下文。生产 worker/Admin 路径携带不可复用 claim；
/// 旧的直接调用入口只携带 task_id，以保持测试和人工工具兼容。
#[derive(Debug, Clone)]
pub struct TaskRunContext {
    pub task_id: ObjectId,
    pub claim: Option<TaskClaim>,
}

impl TaskRunContext {
    pub fn new(task_id: ObjectId, claim: Option<&TaskClaim>) -> Self {
        Self {
            task_id,
            claim: claim.cloned(),
        }
    }

    pub fn write_filter(&self) -> Document {
        self.claim
            .as_ref()
            .map(TaskClaim::owned_running_filter)
            .unwrap_or_else(|| doc! { "_id": self.task_id })
    }
}

async fn claim_task_with_filter(
    state: &AppState,
    mut filter: Document,
) -> AppResult<Option<(crate::models::AgentTask, TaskClaim)>> {
    let claim_token = uuid::Uuid::new_v4().to_string();
    if !filter.contains_key("status") {
        filter.insert("status", doc! { "$in": ["pending", "retry", "failed"] });
    }
    let now = DateTime::now();
    let claimed = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one_and_update(
            filter,
            doc! {
                "$set": {
                    "status": "running",
                    "updated_at": now,
                    "claimed_at": now,
                    "claim_token": &claim_token,
                },
                "$inc": {
                    "attempt_count": 1,
                    "claim_generation": 1i64,
                },
                "$unset": {
                    "outbox_decision_id": "",
                    "next_retry_at": "",
                    "rerun_requested": "",
                },
            },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let Some(raw) = claimed else {
        return Ok(None);
    };
    let task_id = raw.get_object_id("_id").map_err(|error| {
        crate::error::AppError::External(format!("claimed task _id invalid: {error}"))
    })?;
    let claim_generation = raw
        .get_i64("claim_generation")
        .or_else(|_| raw.get_i32("claim_generation").map(i64::from))
        .unwrap_or(1);
    let task = mongodb::bson::from_document(raw).map_err(|error| {
        crate::error::AppError::External(format!("claimed task decode failed: {error}"))
    })?;
    Ok(Some((
        task,
        TaskClaim {
            task_id,
            claim_token,
            claim_generation,
        },
    )))
}

/// Admin“立即复核”和集成红线共用的原子 claim。生产 worker 也使用同一底层协议。
pub async fn claim_task_by_id(
    state: &AppState,
    task_id: ObjectId,
    workspace_id: Option<&str>,
) -> AppResult<Option<(crate::models::AgentTask, TaskClaim)>> {
    let mut filter = doc! { "_id": task_id };
    if let Some(workspace_id) = workspace_id {
        filter.insert("workspace_id", workspace_id);
    }
    claim_task_with_filter(state, filter).await
}

/// Admin task actions must bind the rendered task to its immutable account.
/// Background workers intentionally continue to use `claim_task_by_id` because
/// they claim from an already account-scoped scheduler query.
pub async fn claim_task_by_id_for_account(
    state: &AppState,
    task_id: ObjectId,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Option<(crate::models::AgentTask, TaskClaim)>> {
    claim_task_with_filter(
        state,
        doc! {
            "_id": task_id,
            "workspace_id": workspace_id,
            "account_id": account_id,
        },
    )
    .await
}

pub async fn task_claim_is_current(state: &AppState, claim: &TaskClaim) -> AppResult<bool> {
    Ok(state
        .db
        .tasks()
        .count_documents(claim.owned_running_filter(), None)
        .await?
        == 1)
}

/// 在创建任何 Outbox 前，把本次 decision 绑定到仍归当前 token 所有的 task。
pub async fn bind_task_decision_if_owned(
    state: &AppState,
    claim: &TaskClaim,
    decision_id: ObjectId,
) -> AppResult<bool> {
    // `find_one_and_update` is the coverage-snapshot linearization point. If a newer inbound
    // refreshes the reusable task first, this exact claim no longer matches. If it arrives later,
    // it fences this decision before authorization and remains beyond the frozen watermark.
    let task = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one_and_update(
            claim.owned_running_filter(),
            doc! { "$set": {
                "outbox_decision_id": decision_id,
                "updated_at": DateTime::now(),
            } },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let Some(task) = task else {
        return Ok(false);
    };

    let mut set = doc! {
        "source_task_id": claim.task_id,
        "source_task_claim_token": &claim.claim_token,
    };
    if task.get_str("kind").ok() == Some(crate::webhooks::DURABLE_INBOUND_REPLY_KIND) {
        set.insert("reply_coverage_kind", "passive_reply");
        if let Ok(id) = task.get_object_id("latest_inbound_id") {
            set.insert("covers_through_inbound_id", id);
        }
        if let Ok(created_at) = task.get_datetime("latest_inbound_created_at") {
            set.insert("covers_through_inbound_created_at", *created_at);
        }
    }
    state
        .db
        .decision_reviews()
        .clone_with_type::<Document>()
        .update_one(doc! { "_id": decision_id }, doc! { "$set": set }, None)
        .await?;
    Ok(true)
}

/// SR-177：接管同一 durable inbound task 在“Outbox 已写入、Task 尚未授权”崩溃窗
/// 留下的旧 Outbox。
///
/// 只有以下条件同时成立才允许改绑：
/// - 当前 claim 仍拥有 `inbound_reply` task，且 task 已绑定 `new_decision_id`；
/// - 旧 decision 确由同一 task 的另一个（已失效）claim 产生；
/// - Outbox 从未跨过远端发送边界、未被人工取消、未发生 lease reclaim；
/// - Outbox 仍 pending，或仅因 `stale_task_claim` 被 Dispatcher 取消。
///
/// CAS 成功后 Outbox 回到干净 pending，并改绑当前 decision/run。调用方仍须执行
/// [`authorize_task_outbox_if_owned`]；接管本身绝不构成发送授权。
pub async fn adopt_recoverable_durable_outbox_if_owned(
    state: &AppState,
    claim: &TaskClaim,
    new_decision_id: ObjectId,
    new_run_id: &str,
    existing_outbox_id: ObjectId,
    existing_decision_id: ObjectId,
) -> AppResult<bool> {
    if existing_decision_id == new_decision_id || new_run_id.trim().is_empty() {
        return Ok(false);
    }

    let current_owner = state
        .db
        .tasks()
        .count_documents(
            {
                let mut filter = claim.owned_running_filter();
                filter.insert("kind", crate::webhooks::DURABLE_INBOUND_REPLY_KIND);
                filter.insert("outbox_decision_id", new_decision_id);
                filter
            },
            None,
        )
        .await?
        == 1;
    if !current_owner {
        return Ok(false);
    }

    let old_review = state
        .db
        .decision_reviews()
        .find_one(
            doc! {
                "_id": existing_decision_id,
                "source_task_id": claim.task_id,
                "source_task_claim_token": { "$exists": true, "$ne": &claim.claim_token },
            },
            None,
        )
        .await?;
    if old_review.is_none() {
        return Ok(false);
    }

    let now = DateTime::now();
    let result = state
        .db
        .collection_agent_send_outbox()
        .update_one(
            doc! {
                "_id": existing_outbox_id,
                "decision_id": existing_decision_id,
                "cancel_requested": { "$ne": true },
                "reclaimed_in_flight": { "$ne": true },
                "reclaim_count": { "$in": [0, null] },
                "attempt": { "$in": [0, null] },
                "$and": [
                    { "$or": [
                        { "send_started_at": { "$exists": false } },
                        { "send_started_at": null },
                    ] },
                    { "$or": [
                        { "task_send_authorization_token": { "$exists": false } },
                        { "task_send_authorization_token": null },
                    ] },
                    { "$or": [
                        { "status": "pending" },
                        {
                            "status": "canceled",
                            "cancel_reason": { "$regex": "^stale_task_claim:" },
                        },
                    ] },
                ],
            },
            doc! {
                "$set": {
                    "decision_id": new_decision_id,
                    "run_id": new_run_id,
                    "status": "pending",
                    "updated_at": now,
                    "cancel_requested": false,
                },
                "$unset": {
                    "cancel_reason": "",
                    "last_error": "",
                    "next_retry_at": "",
                    "worker_id": "",
                    "locked_until": "",
                    "claim_token": "",
                    "cancel_requested_at": "",
                    "send_started_at": "",
                    "task_send_authorization_token": "",
                },
            },
            None,
        )
        .await?;
    if result.modified_count != 1 {
        return Ok(false);
    }

    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": existing_decision_id },
            doc! { "$set": {
                "status": "superseded_by_task_recovery",
                "superseded_by_decision_id": new_decision_id,
            } },
            None,
        )
        .await?;
    Ok(true)
}

/// Outbox 全部写入后，以同一 token 提交客户发送意图。Dispatcher 只放行该终态。
pub async fn authorize_task_outbox_if_owned(
    state: &AppState,
    claim: &TaskClaim,
    decision_id: ObjectId,
) -> AppResult<bool> {
    assert_agent_task_status_valid("outbox_enqueued");

    // Prepare the single-document send markers before committing the Task. A prepared marker is
    // not authorization by itself: Dispatcher also requires this exact task/token/decision to be
    // in outbox_enqueued|sent. Therefore a crash here can only defer, and a later reclaim makes
    // the old marker stale. Doing the Task CAS first would leave a committed task with no marker
    // if this projection failed, permanently stranding paths that have no ordinary run log.
    let outbox = state.db.collection_agent_send_outbox();
    let total = outbox
        .count_documents(doc! { "decision_id": decision_id }, None)
        .await?;
    if total == 0 {
        tracing::warn!(%decision_id, task_id = %claim.task_id, "refusing task authorization without outbox rows");
        return Ok(false);
    }
    let prepared = outbox
        .update_many(
            task_outbox_marker_prepare_filter(decision_id, &claim.claim_token),
            doc! { "$set": {
                "task_send_authorization_token": &claim.claim_token,
                "updated_at": DateTime::now(),
            } },
            None,
        )
        .await?;
    if prepared.matched_count != total {
        tracing::error!(
            %decision_id,
            task_id = %claim.task_id,
            expected = total,
            matched = prepared.matched_count,
            "outbox authorization marker conflicts with another task claim"
        );
        return Ok(false);
    }

    let result = state
        .db
        .tasks()
        .update_one(
            task_outbox_commit_filter(claim, decision_id),
            doc! {
                "$set": {
                    "status": "outbox_enqueued",
                    "gateway_status": "outbox_enqueued",
                    "updated_at": DateTime::now(),
                },
                "$unset": { "claimed_at": "" },
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Ok(false);
    }
    // This CAS is the authorization linearization point. Wake now, and once more after the
    // dispatcher's one-second Building deferral in case enqueue won the race before this CAS.
    crate::agent::outbox_dispatcher::notify_outbox_work();
    crate::agent::outbox_dispatcher::notify_outbox_work_after(Duration::from_millis(1_050));
    Ok(true)
}

pub async fn run_task_worker(state: AppState) {
    loop {
        if let Err(error) = tick(&state).await {
            tracing::error!(error = %error, "task worker tick failed");
        }
        sleep(Duration::from_secs(
            state.config.task_worker_interval_seconds,
        ))
        .await;
    }
}

/// Dedicated recovery lane for customer inbound replies. Webhooks still perform an immediate
/// wake-up, while this worker guarantees that restart/backlog recovery never queues behind
/// profiling, consolidation, campaigns, or proactive follow-ups.
pub async fn run_inbound_reply_worker(state: AppState) {
    loop {
        if let Err(error) = tick_inbound_replies(&state).await {
            tracing::error!(error = %error, "inbound reply worker tick failed");
        }
        sleep(Duration::from_millis(250)).await;
    }
}

/// 执行一条已经由调用方以 [`TaskClaim`] 原子认领的任务，并在执行期间续约 lease。
/// worker 与 Admin“立即复核”共用此入口，避免后者绕过 token/heartbeat 协议。
pub async fn execute_claimed_task(
    state: &AppState,
    task: crate::models::AgentTask,
    claim: &TaskClaim,
) -> AppResult<()> {
    let heartbeat = spawn_claim_heartbeat(
        state.clone(),
        claim.clone(),
        state.config.task_claim_timeout_seconds,
    );
    let result = if task.kind == "memory_consolidation" {
        agent::handle_memory_consolidation_task_with_claim(state, task, Some(claim)).await
    } else if task.kind == "outcome_aggregation" {
        handle_outcome_aggregation_task(state, task, Some(claim)).await
    } else if task.kind == "initial_profile" {
        crate::routes::contacts::handle_initial_profile_task_with_claim(state, &task, Some(claim))
            .await
    } else {
        agent::handle_follow_up_task_with_claim(state, task, Some(claim)).await
    };
    heartbeat.abort();
    result
}

/// Claim and execute one specific due task through the same ownership, heartbeat,
/// retry, and terminalization protocol used by the background worker. Webhook
/// ingestion uses this as a low-latency wake-up hint after it has durably
/// materialized an inbound-reply task; a process crash simply leaves the same
/// task for the ordinary worker scan.
pub async fn run_due_task_by_id(state: &AppState, task_id: ObjectId) -> AppResult<bool> {
    let now = DateTime::now();
    let Some((task, claim)) = claim_task_with_filter(
        state,
        doc! {
            "_id": task_id,
            "manual_reply_run_id": { "$exists": false },
            "$or": [
                { "status": "pending", "run_at": { "$lte": now } },
                { "status": "retry", "next_retry_at": { "$lte": now } },
            ],
        },
    )
    .await?
    else {
        return Ok(false);
    };
    process_claimed_task(state, task, claim).await?;
    Ok(true)
}

/// Execute and settle an already claimed task. Keeping this in one function is
/// important: the low-latency webhook wake-up and the periodic worker must not
/// diverge on retry/failure fencing semantics.
async fn process_claimed_task(
    state: &AppState,
    task: crate::models::AgentTask,
    claim: TaskClaim,
) -> AppResult<()> {
    let task_account_id = task.account_id.clone();
    let task_workspace_id = task.workspace_id.clone();
    let task_contact_wxid = task.contact_wxid.clone();
    let task_kind = task.kind.clone();
    let attempt_count = task.attempt_count;
    let max_attempts = if task.max_attempts <= 0 {
        3
    } else {
        task.max_attempts
    };
    let result = execute_claimed_task(state, task, &claim).await;

    match result {
        Ok(()) => {
            // Some handlers deliberately return Ok after losing ownership or after choosing a
            // non-send terminal state. Emit the send-success audit only when this exact claim
            // reached the durable outbox authorization terminal.
            let send_authorized = state
                .db
                .tasks()
                .count_documents(task_claim_send_terminal_filter(&claim), None)
                .await?
                == 1;
            if send_authorized {
                agent::write_event_for_account(
                    state,
                    &task_workspace_id,
                    &task_account_id,
                    Some(&task_contact_wxid),
                    "follow_up_processed",
                    "success",
                    "跟进任务已通过发送网关处理",
                    None,
                )
                .await?;
            }
        }
        Err(error) => {
            if crate::llm::is_llm_account_unavailable(&error) {
                assert_agent_task_status_valid("retry");
                let now = DateTime::now();
                let updated = state
                    .db
                    .tasks()
                    .update_one(
                        claim.owned_running_filter(),
                        provider_unavailable_settlement_update(now),
                        None,
                    )
                    .await?;
                if updated.matched_count == 1 {
                    agent::write_event_for_account(
                        state,
                        &task_workspace_id,
                        &task_account_id,
                        Some(&task_contact_wxid),
                        "follow_up_blocked_provider_unavailable",
                        "retry",
                        "大模型账户不可用，任务已保留并等待服务恢复",
                        None,
                    )
                    .await?;
                }
                return Ok(());
            }
            if attempt_count < max_attempts {
                let delay_seconds = retry_delay_seconds(attempt_count);
                assert_agent_task_status_valid("retry");
                let updated = state
                    .db
                    .tasks()
                    .update_one(
                        claim.owned_running_filter(),
                        doc! {
                            "$set": {
                                "status": "retry",
                                "gateway_status": "retry_scheduled",
                                "error": error.to_string(),
                                "next_retry_at": DateTime::from_millis(
                                    DateTime::now().timestamp_millis() + delay_seconds * 1000
                                ),
                                "updated_at": DateTime::now()
                            },
                            "$unset": {
                                "claimed_at": "",
                                "claim_token": "",
                                "outbox_decision_id": "",
                            }
                        },
                        None,
                    )
                    .await?;
                if updated.matched_count == 1 {
                    agent::write_event_for_account(
                        state,
                        &task_workspace_id,
                        &task_account_id,
                        Some(&task_contact_wxid),
                        "follow_up_retry_scheduled",
                        "retry",
                        &format!(
                            "跟进任务失败，已安排第 {attempt_count}/{max_attempts} 次重试：{error}"
                        ),
                        None,
                    )
                    .await?;
                }
                return Ok(());
            }
            assert_agent_task_status_valid("failed");
            let mut terminal_unset = doc! {
                "claimed_at": "",
                "claim_token": "",
                "outbox_decision_id": "",
            };
            // Durable inbound tasks deliberately retain their stable active key so
            // the next inbound can revive the same row and fence this generation.
            if task_kind != "memory_consolidation"
                && task_kind != crate::webhooks::DURABLE_INBOUND_REPLY_KIND
            {
                terminal_unset.insert("active_task_key", "");
                terminal_unset.insert("rerun_requested", "");
            }
            let updated = state
                .db
                .tasks()
                .update_one(
                    claim.owned_running_filter(),
                    doc! {
                        "$set": {
                            "status": "failed",
                            "gateway_status": "failed",
                            "error": error.to_string(),
                            "updated_at": DateTime::now()
                        },
                        "$unset": terminal_unset
                    },
                    None,
                )
                .await?;
            if updated.matched_count == 1 {
                agent::write_event_for_account(
                    state,
                    &task_workspace_id,
                    &task_account_id,
                    Some(&task_contact_wxid),
                    "follow_up_failed",
                    "failed",
                    &error.to_string(),
                    None,
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// HP-1 / Task 9：在每次 tick 开头扫描 `status="running"` 但 `claimed_at`
/// 已超过 [`AppConfig::task_claim_timeout_seconds`] 的任务，重置回 `retry`
/// 让后续 tick 重新 claim。
///
/// `claimed_at` 缺失视作老任务：用进程启动时间 `APP_STARTED_AT` 作为下界，
/// 只回收启动前留下来的；本进程启动后的运行任务即使没写 claimed_at 也跳过
/// 一次，避免误回收正在跑的任务。
///
/// 累计 `claim_recovery_count` ≥ 3 时直接标 `failed`，避免无限循环（按累计次数、
/// 无时间窗——比 24h 滑窗更保守，健康任务本就不累积重试）。
async fn reclaim_stale_running_tasks(state: &AppState) -> anyhow::Result<usize> {
    let timeout_secs = state.config.task_claim_timeout_seconds.max(1) as i64;
    let now_ms = DateTime::now().timestamp_millis();
    let stale_before = DateTime::from_millis(now_ms - timeout_secs * 1000);
    // 进程启动时间。OnceCell 没填时（极端情况）退化为 stale_before，等价于
    // "缺失 claimed_at 的老任务一律可回收"。
    let process_started_at = crate::APP_STARTED_AT.get().copied().unwrap_or(stale_before);

    let filter = doc! {
        "status": "running",
        "$or": [
            { "claimed_at": { "$lt": stale_before } },
            {
                "$and": [
                    { "claimed_at": { "$exists": false } },
                    { "updated_at": { "$lt": process_started_at } }
                ]
            }
        ]
    };

    // 读原始 BSON，保留不进入 AgentTask 业务 DTO 的 claim_token。若先反序列化成
    // AgentTask 再二次读取 token，旧 lease 在两次读取之间可能已被新 owner 接管，
    // 回收器会拿新 token 去误回收新 claim。
    let mut cursor = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find(filter, None)
        .await?;
    let mut recovered = 0usize;
    while let Some(raw_task) = cursor.try_next().await? {
        let claim_token = raw_task.get_str("claim_token").ok().map(str::to_string);
        let claim_generation = raw_task
            .get_i64("claim_generation")
            .or_else(|_| raw_task.get_i32("claim_generation").map(i64::from))
            .ok();
        let task: crate::models::AgentTask = mongodb::bson::from_document(raw_task)?;
        let Some(task_id) = task.id else { continue };
        let mut owned_filter = doc! { "_id": task_id, "status": "running" };
        match claim_token.as_deref() {
            Some(token) => {
                owned_filter.insert("claim_token", token);
            }
            None => {
                // 兼容部署前遗留的 running 文档；只匹配仍无 token 的同一旧 owner。
                owned_filter.insert("claim_token", doc! { "$exists": false });
            }
        }
        let claimed_at = task.claimed_at;
        match claimed_at {
            Some(snapshot) => {
                // 必须匹配扫描时看到的 lease 时间。若 owner 在 find→update 窗口内成功
                // heartbeat，claimed_at 已变化，旧 scanner 的 CAS 必须失败。
                owned_filter.insert("claimed_at", snapshot);
            }
            None => {
                owned_filter.insert("claimed_at", doc! { "$exists": false });
            }
        }
        if let Some(generation) = claim_generation {
            owned_filter.insert("claim_generation", generation);
        } else if claim_token.is_some() {
            // 新协议的 token 必须与 generation 成对存在；畸形行宁可不回收，也不能用
            // 猜测 generation 的 filter 误伤后来 owner。部署前无 token 的旧行仍走兼容分支。
            tracing::error!(%task_id, "running task has claim_token but no claim_generation; refusing stale reclaim");
            continue;
        }
        let claimed_at_ms = claimed_at.map(|d| d.timestamp_millis()).unwrap_or(0);
        let stuck_seconds = if claimed_at_ms > 0 {
            ((now_ms - claimed_at_ms) / 1000).max(0)
        } else {
            0
        };
        let recovery_count = task.claim_recovery_count.saturating_add(1);
        // 累计回收次数 ≥ 3 → 直接 failed，防止死循环。
        if recovery_count >= 3 {
            assert_agent_task_status_valid("failed");
            let mut terminal_unset = doc! {
                "claimed_at": "",
                "claim_token": "",
                "outbox_decision_id": "",
            };
            if task.kind != "memory_consolidation"
                && task.kind != crate::webhooks::DURABLE_INBOUND_REPLY_KIND
            {
                terminal_unset.insert("active_task_key", "");
                terminal_unset.insert("rerun_requested", "");
            }
            let res = state
                .db
                .tasks()
                .update_one(
                    owned_filter.clone(),
                    doc! {
                        "$set": {
                            "status": "failed",
                            "gateway_status": "claim_recovery_exhausted",
                            "error": "task stuck in running state and exceeded recovery attempts",
                            "updated_at": DateTime::now()
                        },
                        "$inc": { "claim_recovery_count": 1 },
                        "$unset": terminal_unset
                    },
                    None,
                )
                .await?;
            if res.modified_count == 1 {
                let _ = agent::write_event_for_account(
                    state,
                    &task.workspace_id,
                    &task.account_id,
                    Some(&task.contact_wxid),
                    "claim_recovery_exhausted",
                    "failed",
                    "任务多次卡死无法回收，已强制 failed",
                    Some(doc! {
                        "task_id": task_id.to_hex(),
                        "kind": &task.kind,
                        "previous_attempt_count": task.attempt_count,
                        "stuck_seconds": stuck_seconds,
                        "recovery_count": recovery_count
                    }),
                )
                .await;
                let _ = agent::write_event_for_account(
                    state,
                    &task.workspace_id,
                    &task.account_id,
                    Some(&task.contact_wxid),
                    "follow_up_failed",
                    "failed",
                    "任务多次卡死无法回收，已强制 failed",
                    None,
                )
                .await;
            }
            continue;
        }
        // 普通回收路径：CAS update 确保只有"还在 running"的任务被重置。
        assert_agent_task_status_valid("retry");
        let res = state
            .db
            .tasks()
            .update_one(
                owned_filter,
                doc! {
                    "$set": {
                        "status": "retry",
                        "gateway_status": "claim_timeout_recovered",
                        "next_retry_at": DateTime::now(),
                        "updated_at": DateTime::now()
                    },
                    "$inc": { "claim_recovery_count": 1 },
                    "$unset": {
                        "claimed_at": "",
                        "claim_token": "",
                        "outbox_decision_id": "",
                    }
                },
                None,
            )
            .await?;
        if res.modified_count == 1 {
            recovered += 1;
            let _ = agent::write_event_for_account(
                state,
                &task.workspace_id,
                &task.account_id,
                Some(&task.contact_wxid),
                "task_claim_recovered",
                "recovered",
                "Worker stale 任务已被回收为 retry",
                Some(doc! {
                    "task_id": task_id.to_hex(),
                    "kind": &task.kind,
                    "previous_attempt_count": task.attempt_count,
                    "stuck_seconds": stuck_seconds,
                    "recovery_count": recovery_count
                }),
            )
            .await;
        }
    }
    if recovered > 0 {
        tracing::info!(recovered, "reclaimed stale running tasks");
    }
    Ok(recovered)
}

async fn reconcile_committing_tasks(state: &AppState) -> anyhow::Result<()> {
    let mut cursor = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find(
            doc! { "status": "committing" },
            FindOptions::builder().limit(20).build(),
        )
        .await?;
    while let Some(task) = cursor.try_next().await? {
        let Some(task_id) = task.get_object_id("_id").ok() else {
            continue;
        };
        let kind = task.get_str("prepared_commit_kind").unwrap_or_default();
        let result = match kind {
            "initial_profile_enrollment" => {
                crate::routes::contacts::reconcile_initial_profile_enrollment(state, task_id)
                    .await
                    .map(|_| ())
            }
            "initial_profile" => {
                crate::routes::contacts::reconcile_initial_profile_commit(state, task_id).await
            }
            "outcome_aggregation" => reconcile_outcome_aggregation_commit(state, task_id).await,
            "memory_consolidation" => {
                crate::agent::reconcile_memory_consolidation_commit(state, task_id).await
            }
            // Campaign reconciliation runs immediately after this generic pass
            // and owns the CampaignSend -> task release ordering.
            "campaign_fanout" => continue,
            _ => {
                tracing::error!(%task_id, kind, "unknown prepared task commit kind");
                continue;
            }
        };
        if let Err(error) = result {
            tracing::warn!(%task_id, kind, %error, "task commit reconciliation failed");
        }
    }
    Ok(())
}

async fn revive_failed_memory_tasks_with_rerun(state: &AppState) -> anyhow::Result<u64> {
    assert_agent_task_status_valid("retry");
    let result = state
        .db
        .tasks()
        .update_many(
            doc! {
                "kind": "memory_consolidation",
                "status": "failed",
                "active_task_key": "memory_consolidation",
                "rerun_requested": true,
            },
            doc! {
                "$set": {
                    "status": "retry",
                    "gateway_status": "memory_candidates_arrived",
                    "next_retry_at": DateTime::now(),
                    "attempt_count": 0,
                    "claim_recovery_count": 0,
                    "updated_at": DateTime::now(),
                },
                "$unset": {
                    "error": "",
                    "rerun_requested": "",
                },
            },
            None,
        )
        .await?;
    Ok(result.modified_count)
}

async fn tick_inbound_replies(state: &AppState) -> anyhow::Result<()> {
    // Materialize any webhook->task handoff interrupted by a process crash before scanning.
    let _ = crate::webhooks::reconcile_pending_inbound_handoffs(state).await?;
    let now = DateTime::now();
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "kind": crate::webhooks::DURABLE_INBOUND_REPLY_KIND,
                "status": { "$in": ["pending", "retry"] },
                "$or": [
                    { "run_at": { "$lte": now } },
                    { "next_retry_at": { "$lte": now } },
                ],
            },
            FindOptions::builder()
                .limit(20)
                .sort(doc! { "next_retry_at": 1, "run_at": 1, "_id": 1 })
                .build(),
        )
        .await?;
    while let Some(task) = cursor.try_next().await? {
        if let Some(task_id) = task.id {
            let _ = run_due_task_by_id(state, task_id).await?;
        }
    }
    Ok(())
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    reconcile_committing_tasks(state).await?;
    // SR-177: an inbound message and its pending handoff marker are persisted in
    // one insert. Recover the narrow crash window before task materialization.
    let _ = crate::webhooks::reconcile_pending_inbound_handoffs(state).await?;
    // SR-054: a resolution persists a durable relay intent before task
    // materialization. Recover any crash/interruption before claiming work.
    let _ = crate::agent::escalation::reconcile_pending_relay_intents(state).await?;
    let _ = crate::agent::escalation::reconcile_principal_card_deliveries(state).await?;
    let _ = crate::agent::system_incident::reconcile_notifications(state).await?;
    // HC-021: the first dispatch freezes one audience snapshot before materializing
    // deterministic tasks. Resume any process crash between those durable steps.
    let _ = crate::routes::campaigns::reconcile_campaign_dispatches(state).await?;
    // HP-1：先回收 stale running，再 claim 新任务。
    let _ = reclaim_stale_running_tasks(state).await?;
    // A candidate may arrive while its single-flight memory task is crossing running -> failed.
    // The scheduler leaves rerun_requested on that row; revive it before scanning runnable work.
    let _ = revive_failed_memory_tasks_with_rerun(state).await?;
    // S-19 / Task 17：保证当日 outcome 聚合任务存在。
    let _ = ensure_today_outcome_aggregation_tasks(state).await;
    // Ask-Human Phase 1 / Task 10：超时未答的请示改派链上下一位真人并重推卡。
    let _ = crate::agent::escalation::scan_escalation_timeouts(state).await;
    // 主动发送台账：回扫已过响应窗口的条目，回填转化（响应率/阶段推进）。
    let _ = crate::agent::send_ledger::scan_send_ledger_outcomes(state).await;
    let mut cursor = state
        .db
        .tasks()
        .find(
            doc! {
                "status": { "$in": ["pending", "retry"] },
                "kind": { "$ne": crate::webhooks::DURABLE_INBOUND_REPLY_KIND },
                "$or": [
                    { "run_at": { "$lte": DateTime::now() } },
                    { "next_retry_at": { "$lte": DateTime::now() } }
                ]
            },
            FindOptions::builder()
                .limit(20)
                .sort(doc! { "next_retry_at": 1, "run_at": 1 })
                .build(),
        )
        .await?;

    while let Some(task) = cursor.try_next().await? {
        let Some(task_id) = task.id else {
            continue;
        };
        // Re-check status and due time in the atomic claim. An inbound may have
        // refreshed this reusable task's run_at after the cursor snapshot; using
        // only `_id + old status` here would execute before the new debounce
        // deadline. The webhook wake-up uses this same entry point.
        let _ = run_due_task_by_id(state, task_id).await?;
    }
    Ok(())
}

/// P1-8：任务重试退避带 ±20% jitter，避免 MCP/LLM 故障同时恢复后所有重试
/// 任务在同一墙钟 tick 形成 thundering herd 砸回上游。
///
/// 公式：base = `min(60 * 2^(attempt-1), 900)`（60s / 120s / 240s / 480s / 900s
/// 上限），jitter01 ∈ [0, 1] 时实际延迟落在 base * [0.8, 1.2] 区间内。
/// jitter01=0.5 即 0 jitter（基线），便于单测。
fn retry_delay_seconds(attempt_count: i32) -> i64 {
    retry_delay_seconds_seeded(attempt_count, fastrand::f64())
}

fn provider_unavailable_retry_delay_seconds() -> i64 {
    5 * 60
}

fn provider_unavailable_settlement_update(now: DateTime) -> Document {
    doc! {
        "$set": {
            "status": "retry",
            "gateway_status": "blocked_provider_unavailable",
            "error": "llm account unavailable",
            "next_retry_at": DateTime::from_millis(
                now.timestamp_millis() + provider_unavailable_retry_delay_seconds() * 1000
            ),
            "updated_at": now,
        },
        "$inc": { "attempt_count": -1i32 },
        "$unset": {
            "claimed_at": "",
            "claim_token": "",
            "outbox_decision_id": "",
        },
    }
}

fn retry_delay_seconds_seeded(attempt_count: i32, jitter01: f64) -> i64 {
    let capped = attempt_count.clamp(1, 6);
    let base = 60_i64.saturating_mul(1_i64 << (capped - 1)).min(900);
    let j = jitter01.clamp(0.0, 1.0);
    let factor = (j - 0.5) * 2.0 * 0.2;
    let delta = (base as f64 * factor).round() as i64;
    base + delta
}

/// P1-9：续约心跳间隔。基于 `task_claim_timeout_seconds` 推导：
/// 取 timeout/2 但夹在 [5, 60] 内——下界保证不抖太频繁、上界保证 timeout=120s
/// 时仍有两次心跳机会，避免一次心跳失败就被 reclaim。
pub(crate) fn claim_heartbeat_interval_seconds(task_claim_timeout_seconds: u64) -> u64 {
    let half = task_claim_timeout_seconds / 2;
    half.clamp(5, 60)
}

/// P1-9：spawn 一个长跑后台任务给 `task_id` 续约 claimed_at。
/// 调用方在 work future 结束后 `.abort()` 该 handle，停止心跳。
///
/// 故意不走 supervisor.spawn_supervised：心跳的退出条件是"上游 work future
/// 完成 / abort"，而 supervisor 的语义是"panic 后无限重启"。心跳 panic 反而
/// 应该让其消失，让 reclaim_stale_running_tasks 兜底。
fn spawn_claim_heartbeat(
    state: AppState,
    claim: TaskClaim,
    task_claim_timeout_seconds: u64,
) -> tokio::task::JoinHandle<()> {
    let interval = claim_heartbeat_interval_seconds(task_claim_timeout_seconds);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        // 第一次 tick 立刻触发；下一次 tick 跳过首拍。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            // 只对仍处于 running 的任务续约：终态/被 reclaim 的任务直接 stop。
            let res = state
                .db
                .tasks()
                .update_one(
                    claim.owned_running_filter(),
                    doc! { "$set": { "claimed_at": DateTime::now() } },
                    None,
                )
                .await;
            match res {
                Ok(r) if r.modified_count == 0 => {
                    // 任务已不在 running（被 reclaim 或已落终态）→ 退出心跳。
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(
                        task_id = %claim.task_id.to_hex(),
                        error = %error,
                        "claim_heartbeat update failed; will retry next tick"
                    );
                }
            }
        }
    })
}

/// S-19 / Task 17：保证当日所有 (account, horizon) 都有一条 `outcome_aggregation`
/// 任务。在 [`tick`] 入口被调用，幂等（基于 task content 中的日期 + horizon 去重）。
///
/// P1-1：原 `find_one + insert_one` 是 TOCTOU；多副本/重叠 tick 都会通过
/// 检查并双写。改为直接 `insert_one`，依赖 db/indexes.rs:90 的
/// `uniq_outcome_aggregation_ws_kind_account_content` partial unique index
/// 在 MongoDB 侧原子去重，11000 dup-key 视作"已经有人插过了"忽略即可。
async fn ensure_today_outcome_aggregation_tasks(state: &AppState) -> anyhow::Result<()> {
    use mongodb::bson::DateTime as BsonDt;
    let today = today_date_string();
    let now = BsonDt::now();
    let mut accounts_cursor = state.db.accounts().find(doc! {}, None).await?;
    while let Some(account) = accounts_cursor.try_next().await? {
        for horizon in ["7d", "30d"].iter() {
            let content = format!("{{\"horizon\":\"{horizon}\",\"date\":\"{today}\"}}");
            let result = state
                .db
                .tasks()
                .insert_one(
                    crate::models::AgentTask {
                        id: None,
                        workspace_id: account.workspace_id.clone(),
                        account_id: account.account_id.clone(),
                        contact_wxid: "_outcome_aggregation".to_string(),
                        kind: "outcome_aggregation".to_string(),
                        run_at: now,
                        expires_at: None,
                        content,
                        status: "pending".to_string(),
                        source_decision_id: None,
                        review_required: false,
                        attempt_count: 0,
                        max_attempts: 3,
                        next_retry_at: None,
                        gateway_status: None,
                        cancel_reason: None,
                        error: None,
                        claimed_at: None,
                        claim_recovery_count: 0,
                        created_at: now,
                        updated_at: now,
                    },
                    None,
                )
                .await;
            if let Err(error) = result {
                if !is_duplicate_key_error(&error) {
                    return Err(error.into());
                }
                // dup-key：当日已有该 (account, horizon) 的 outcome_aggregation 任务，
                // 幂等忽略。
            }
        }
    }
    Ok(())
}

/// 判定 mongodb 错误是否为 DuplicateKey（code 11000 / 11001）。
/// 与 `agent::outbox::is_duplicate_key_error` 同语义；不跨 mod 复用以避免
/// tasks 反向依赖 agent 内部 helper。
fn is_duplicate_key_error(err: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000 || write_error.code == 11001
        }
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .as_ref()
            .map(|errs| errs.iter().any(|e| e.code == 11000 || e.code == 11001))
            .unwrap_or(false),
        _ => false,
    }
}

fn today_date_string() -> String {
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    // 截断到日；用 epoch 起点开始的天数转成 YYYY-MM-DD（粗糙但足够幂等用）。
    let days = now_ms / day_ms;
    let secs = days * 24 * 60 * 60;
    let datetime =
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).unwrap_or_else(chrono::Utc::now);
    datetime.format("%Y-%m-%d").to_string()
}

/// S-19 / Task 17：处理一次 outcome 聚合任务，把 24h/7d/30d 的指标写入
/// `agent_outcome_metrics` 集合（按 _id 幂等）。
async fn handle_outcome_aggregation_task(
    state: &AppState,
    task: crate::models::AgentTask,
    task_claim: Option<&TaskClaim>,
) -> AppResult<()> {
    let Some(task_id) = task.id else {
        return Ok(());
    };
    // 解析 content 拿 horizon / date
    let parsed: serde_json::Value =
        serde_json::from_str(&task.content).unwrap_or(serde_json::json!({}));
    let horizon = parsed
        .get("horizon")
        .and_then(|v| v.as_str())
        .unwrap_or("7d")
        .to_string();
    let date = parsed
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let horizon_days: i64 = if horizon == "30d" { 30 } else { 7 };
    let now_ms = mongodb::bson::DateTime::now().timestamp_millis();
    let window_start =
        mongodb::bson::DateTime::from_millis(now_ms - horizon_days * 24 * 60 * 60 * 1000);

    // reply_rate：发出消息（outbound）后 horizon_days 内有 inbound 的比例。
    let _outbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "direction": "outbound",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    // 严格按"每条 outbound 后 horizon 窗口内是否有用户 inbound"计算。
    let mut outbound_total = 0_i64;
    let mut replied_outbound_total = 0_i64;
    let mut cur = state
        .db
        .messages()
        .find(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "direction": "outbound",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await?;
    while let Some(msg) = cur.try_next().await? {
        outbound_total += 1;
        let reply_window_end = DateTime::from_millis(
            msg.created_at.timestamp_millis() + horizon_days * 24 * 60 * 60 * 1000,
        );
        let has_inbound_after_outbound = state
            .db
            .messages()
            .count_documents(
                doc! {
                    "workspace_id": &task.workspace_id,
                    "account_id": &task.account_id,
                    "contact_wxid": &msg.contact_wxid,
                    "direction": "inbound",
                    "created_at": {
                        "$gt": msg.created_at,
                        "$lte": reply_window_end
                    }
                },
                None,
            )
            .await
            .unwrap_or(0);
        if has_inbound_after_outbound > 0 {
            replied_outbound_total += 1;
        }
    }
    let reply_rate = if outbound_total > 0 {
        Some(replied_outbound_total as f64 / outbound_total as f64)
    } else {
        // 波 A2：outbound 总数为 0 时返回 None（无数据），不写 0 误导前端。
        None
    };

    // conversation_depth：每个 managed contact 平均 inbound 数。
    let inbound_count = state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "direction": "inbound",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let managed_count = state
        .db
        .contacts()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "agent_status": "managed"
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let conversation_depth = if managed_count > 0 {
        Some(inbound_count as f64 / managed_count as f64)
    } else {
        // 波 A2：无 managed contact 时无意义，返回 None。
        None
    };

    // agent_block_rate：blocked review / total review。
    let blocked = state
        .db
        .decision_reviews()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "status": "blocked",
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let review_total = state
        .db
        .decision_reviews()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "created_at": { "$gte": window_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let agent_block_rate = if review_total > 0 {
        Some(blocked as f64 / review_total as f64)
    } else {
        // 波 A2：review 总数为 0 时返回 None。
        None
    };

    // 波 A2：ai_hold_cleared_rate 暂无事件源（AI 自暂缓后由 AI 自身澄清恢复
    // 继续的比例），写 None 表示"指标不可用"，不再以 0 静默冒充零成功率。
    let ai_hold_cleared_rate: Option<f64> = None;

    // daily_run_count / daily_run_token_total：当日 agent_run_logs 聚合（不取 horizon，固定取 24h）。
    let day_start = mongodb::bson::DateTime::from_millis(now_ms - 24 * 60 * 60 * 1000);
    let daily_run_count = state
        .db
        .agent_run_logs()
        .count_documents(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "created_at": { "$gte": day_start }
            },
            None,
        )
        .await
        .unwrap_or(0) as i64;
    let mut daily_run_token_total = 0i64;
    let mut runs_cur = state
        .db
        .agent_run_logs()
        .find(
            doc! {
                "workspace_id": &task.workspace_id,
                "account_id": &task.account_id,
                "created_at": { "$gte": day_start }
            },
            None,
        )
        .await?;
    while let Some(run) = runs_cur.try_next().await? {
        daily_run_token_total += run.tokens_used;
    }

    let metric = crate::models::AgentOutcomeMetric {
        id: format!(
            "{}:{}:{}:{}",
            task.workspace_id, task.account_id, horizon, date
        ),
        workspace_id: task.workspace_id.clone(),
        account_id: task.account_id.clone(),
        horizon: horizon.clone(),
        date: date.clone(),
        reply_rate,
        conversation_depth,
        ai_hold_cleared_rate,
        agent_block_rate,
        daily_run_count,
        daily_run_token_total,
        source_task_id: task_claim.map(|claim| claim.task_id),
        source_task_claim_generation: task_claim.map(|claim| claim.claim_generation).unwrap_or(0),
        created_at: mongodb::bson::DateTime::now(),
    };
    let metric_doc = mongodb::bson::to_document(&metric)?;
    if let Some(claim) = task_claim {
        if !prepare_task_commit_if_owned(
            state,
            claim,
            "outcome_aggregation",
            doc! { "metric": metric_doc },
        )
        .await?
        {
            return Ok(());
        }
        return reconcile_outcome_aggregation_commit(state, claim.task_id).await;
    }

    let metric_filter = outcome_metric_write_filter(&metric.id, task_claim);
    if let Err(error) = state
        .db
        .outcome_metrics()
        .update_one(
            metric_filter,
            doc! { "$set": metric_doc },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await
    {
        // 若更高 generation 已写同一 _id，带条件的 upsert 会撞 duplicate key；这是
        // 旧 owner 被 fencing 的正常结果，不得让它回写 task retry/failed。
        if task_claim.is_some() && is_duplicate_key_error(&error) {
            return Ok(());
        }
        return Err(error.into());
    }
    assert_agent_task_status_valid("sent");
    let task_filter = task_claim
        .map(TaskClaim::owned_running_filter)
        .unwrap_or_else(|| doc! { "_id": task_id });
    state
        .db
        .tasks()
        .update_one(
            task_filter,
            doc! {
                "$set": {
                    "status": "sent",
                    "gateway_status": "aggregated",
                    "updated_at": mongodb::bson::DateTime::now()
                },
                "$unset": { "claimed_at": "", "claim_token": "" }
            },
            None,
        )
        .await?;
    Ok(())
}

async fn reconcile_outcome_aggregation_commit(
    state: &AppState,
    task_id: ObjectId,
) -> AppResult<()> {
    let Some(raw) = state
        .db
        .tasks()
        .clone_with_type::<Document>()
        .find_one(
            doc! {
                "_id": task_id,
                "status": "committing",
                "prepared_commit_kind": "outcome_aggregation",
            },
            None,
        )
        .await?
    else {
        return Ok(());
    };
    let claim_token = raw
        .get_str("claim_token")
        .map(str::to_string)
        .map_err(|error| {
            crate::error::AppError::External(format!("outcome commit token missing: {error}"))
        })?;
    let claim_generation = raw
        .get_i64("claim_generation")
        .or_else(|_| raw.get_i32("claim_generation").map(i64::from))
        .map_err(|error| {
            crate::error::AppError::External(format!("outcome commit generation missing: {error}"))
        })?;
    let claim = TaskClaim {
        task_id,
        claim_token,
        claim_generation,
    };
    let metric = raw
        .get_document("prepared_commit")
        .and_then(|prepared| prepared.get_document("metric"))
        .map_err(|error| {
            crate::error::AppError::External(format!("outcome prepared metric missing: {error}"))
        })?
        .clone();
    let metric_id = metric.get_str("_id").map_err(|error| {
        crate::error::AppError::External(format!("outcome metric id missing: {error}"))
    })?;
    let result = state
        .db
        .outcome_metrics()
        .update_one(
            outcome_metric_write_filter(metric_id, Some(&claim)),
            doc! { "$set": metric },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await;
    match result {
        Ok(_) => {}
        Err(error) if is_duplicate_key_error(&error) => {
            // A higher generation from this task already owns the projection. The prepared
            // commit is stale but complete from the task protocol's point of view.
        }
        Err(error) => return Err(error.into()),
    }
    let _ = finalize_task_commit_if_owned(state, &claim, "aggregated").await?;
    Ok(())
}

fn outcome_metric_write_filter(metric_id: &str, claim: Option<&TaskClaim>) -> Document {
    let mut filter = doc! { "_id": metric_id };
    if let Some(claim) = claim {
        filter.insert(
            "$or",
            vec![
                doc! { "source_task_id": { "$exists": false } },
                doc! {
                    "source_task_id": claim.task_id,
                    "source_task_claim_generation": { "$lte": claim.claim_generation }
                },
            ],
        );
    }
    filter
}

#[cfg(test)]
mod tests {
    //! tasks.rs 单元测试：claim 时序结构相关。
    //!
    //! 真实的"claim 后写 claimed_at"行为依赖 MongoDB，覆盖在
    //! `tests/worker_reclaim.rs`（Task 9）。这里只做结构性回归，确保
    //! `AgentTask` schema 包含 `claimed_at` 与 `claim_recovery_count` 字段，
    //! 防止后续重构误删字段而不被发现。

    use crate::models::AgentTask;
    use mongodb::bson::{doc, oid::ObjectId, DateTime};

    use super::{
        outcome_metric_write_filter, task_claim_send_terminal_filter, task_outbox_commit_filter,
        task_outbox_marker_prepare_filter, TaskClaim,
    };

    #[test]
    fn task_outbox_commit_filter_fences_owner_and_decision() {
        let task_id = ObjectId::parse_str("64b64c000000000000000034").unwrap();
        let decision_id = ObjectId::parse_str("64b64c000000000000000035").unwrap();
        let claim = TaskClaim {
            task_id,
            claim_token: "claim-token".to_string(),
            claim_generation: 7,
        };
        assert_eq!(
            task_outbox_commit_filter(&claim, decision_id),
            doc! {
                "_id": task_id,
                "status": "running",
                "claim_token": "claim-token",
                "claim_generation": 7i64,
                "outbox_decision_id": decision_id,
            }
        );
    }

    #[test]
    fn task_outbox_marker_prepare_filter_never_overwrites_another_owner() {
        let decision_id = ObjectId::parse_str("64b64c000000000000000035").unwrap();
        assert_eq!(
            task_outbox_marker_prepare_filter(decision_id, "claim-token"),
            doc! {
                "decision_id": decision_id,
                "$or": [
                    { "task_send_authorization_token": { "$exists": false } },
                    { "task_send_authorization_token": null },
                    { "task_send_authorization_token": "claim-token" },
                ],
            }
        );
    }

    #[test]
    fn send_terminal_filter_requires_the_exact_claim() {
        let task_id = ObjectId::parse_str("64b64c000000000000000034").unwrap();
        let claim = TaskClaim {
            task_id,
            claim_token: "claim-token".to_string(),
            claim_generation: 7,
        };
        assert_eq!(
            task_claim_send_terminal_filter(&claim),
            doc! {
                "_id": task_id,
                "status": { "$in": ["outbox_enqueued", "sent"] },
                "claim_token": "claim-token",
                "claim_generation": 7i64,
            }
        );
    }

    #[test]
    fn outcome_projection_never_allows_a_different_task_owner() {
        let task_id = ObjectId::parse_str("64b64c000000000000000034").unwrap();
        let claim = TaskClaim {
            task_id,
            claim_token: "claim-token".to_string(),
            claim_generation: 7,
        };
        assert_eq!(
            outcome_metric_write_filter("metric-1", Some(&claim)),
            doc! {
                "_id": "metric-1",
                "$or": [
                    { "source_task_id": { "$exists": false } },
                    {
                        "source_task_id": task_id,
                        "source_task_claim_generation": { "$lte": 7i64 }
                    },
                ],
            }
        );
    }

    /// HP-1 / Task 7 schema 回归：
    /// `AgentTask` 必须支持 `claimed_at: Option<DateTime>` 与
    /// `claim_recovery_count: i32` 两个新字段，且默认值为 None / 0。
    #[test]
    fn agent_task_supports_claim_tracking_fields() {
        let now = DateTime::now();
        let task = AgentTask {
            id: None,
            workspace_id: "default".to_string(),
            account_id: "default".to_string(),
            contact_wxid: "user_test".to_string(),
            kind: "follow_up".to_string(),
            run_at: now,
            expires_at: None,
            content: "demo".to_string(),
            status: "pending".to_string(),
            source_decision_id: None,
            review_required: false,
            attempt_count: 0,
            max_attempts: 3,
            next_retry_at: None,
            gateway_status: None,
            cancel_reason: None,
            error: None,
            claimed_at: None,
            claim_recovery_count: 0,
            created_at: now,
            updated_at: now,
        };
        assert!(task.claimed_at.is_none());
        assert_eq!(task.claim_recovery_count, 0);

        // 模拟 claim 后赋值：claimed_at 应为 Some(now)。
        let claimed = AgentTask {
            claimed_at: Some(now),
            claim_recovery_count: 1,
            ..task
        };
        assert!(claimed.claimed_at.is_some());
        assert_eq!(claimed.claim_recovery_count, 1);
    }

    /// P1-8：jitter01=0.5 应回到无 jitter 基线，验证 attempt 1..6 的指数表
    /// 与 900s 上限。
    #[test]
    fn retry_delay_baseline_without_jitter() {
        use super::retry_delay_seconds_seeded;
        // attempt → base = 60 * 2^(attempt-1)，封顶 900。
        assert_eq!(retry_delay_seconds_seeded(1, 0.5), 60);
        assert_eq!(retry_delay_seconds_seeded(2, 0.5), 120);
        assert_eq!(retry_delay_seconds_seeded(3, 0.5), 240);
        assert_eq!(retry_delay_seconds_seeded(4, 0.5), 480);
        assert_eq!(retry_delay_seconds_seeded(5, 0.5), 900);
        assert_eq!(retry_delay_seconds_seeded(6, 0.5), 900);
        // 越界 attempt 也封顶。
        assert_eq!(retry_delay_seconds_seeded(99, 0.5), 900);
    }

    #[test]
    fn provider_unavailable_settlement_preserves_retry_budget() {
        let now = DateTime::from_millis(1_000);
        let update = super::provider_unavailable_settlement_update(now);
        let set = update.get_document("$set").unwrap();
        assert_eq!(set.get_str("status").unwrap(), "retry");
        assert_eq!(
            set.get_str("gateway_status").unwrap(),
            "blocked_provider_unavailable"
        );
        assert_eq!(set.get_str("error").unwrap(), "llm account unavailable");
        assert_eq!(
            set.get_datetime("next_retry_at")
                .unwrap()
                .timestamp_millis(),
            301_000
        );
        assert_eq!(
            update
                .get_document("$inc")
                .unwrap()
                .get_i32("attempt_count")
                .unwrap(),
            -1
        );
    }

    /// P1-8：jitter ∈ ±20% → attempt=2 base=120s 实际落在 [96, 144]。
    #[test]
    fn retry_delay_jitter_within_bounds() {
        use super::retry_delay_seconds_seeded;
        let lo = retry_delay_seconds_seeded(2, 0.0);
        let hi = retry_delay_seconds_seeded(2, 1.0);
        assert!(lo >= 96 && lo <= 120, "low jitter out of range: {lo}");
        assert!(hi >= 120 && hi <= 144, "high jitter out of range: {hi}");
        // 上限场景也守住 ±20%：900 * 0.8 = 720。
        let lo_cap = retry_delay_seconds_seeded(6, 0.0);
        assert!(
            lo_cap >= 720 && lo_cap <= 900,
            "cap low jitter out of range: {lo_cap}"
        );
    }

    /// P1-8：随机 jitter 不会越过 ±20% 区间，避免回归引入 bias。
    #[test]
    fn retry_delay_random_jitter_stays_in_band() {
        use super::retry_delay_seconds_seeded;
        for _ in 0..200 {
            let j = fastrand::f64();
            let v = retry_delay_seconds_seeded(3, j);
            // attempt=3 base=240，区间 [192, 288]。
            assert!(v >= 192 && v <= 288, "v={v} jitter01={j} out of [192, 288]");
        }
    }

    /// P1-9：心跳间隔 = timeout/2，但夹在 [5, 60]。
    #[test]
    fn claim_heartbeat_interval_clamps() {
        use super::claim_heartbeat_interval_seconds;
        // 下界：timeout=5 → 5/2=2 → 夹到 5。
        assert_eq!(claim_heartbeat_interval_seconds(5), 5);
        // 默认 30s timeout → 15s。
        assert_eq!(claim_heartbeat_interval_seconds(30), 15);
        // 60s timeout → 30s。
        assert_eq!(claim_heartbeat_interval_seconds(60), 30);
        // 上界：timeout=600s 直接夹到 60。
        assert_eq!(claim_heartbeat_interval_seconds(600), 60);
        // 0：clamp 下界保护。
        assert_eq!(claim_heartbeat_interval_seconds(0), 5);
    }

    /// P1-9：心跳间隔严格 < task_claim_timeout，避免一次心跳失败就被 reclaim。
    /// 唯一例外是夹到上界 60 的极长 timeout——那种 timeout 下无论如何都不会
    /// 被 reclaim 误判。
    #[test]
    fn claim_heartbeat_strictly_below_timeout_in_normal_range() {
        use super::claim_heartbeat_interval_seconds;
        for timeout in [10_u64, 20, 30, 60, 90, 119] {
            let interval = claim_heartbeat_interval_seconds(timeout);
            assert!(
                (interval as u64) < timeout,
                "interval={interval} must be < timeout={timeout}"
            );
        }
    }
}
