//! Crash recovery for management command executions.
//!
//! A process can die after taking a command lease and before finalizing the tool result. Because
//! replaying an external side effect is unsafe, stale executions converge to `execution_unknown`.

use std::time::Duration;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime},
    options::FindOptions,
};

use crate::{models::AgentCommandRun, routes::AppState};

const MANAGEMENT_EXECUTION_LEASE_MILLIS: i64 = 5 * 60 * 1000;
const MANAGEMENT_SWEEP_INTERVAL_SECONDS: u64 = 60;
const MANAGEMENT_SWEEP_BATCH: i64 = 100;

pub async fn management_command_sweeper_loop(state: AppState) {
    loop {
        if let Err(error) = sweep_stale_management_commands(&state).await {
            tracing::error!(error = %error, "management command sweeper failed");
        }
        tokio::time::sleep(Duration::from_secs(MANAGEMENT_SWEEP_INTERVAL_SECONDS)).await;
    }
}

pub async fn sweep_stale_management_commands(state: &AppState) -> anyhow::Result<u64> {
    let stale_before = stale_management_execution_before(DateTime::now());
    let cursor = state
        .db
        .command_runs()
        .find(
            stale_management_filter(stale_before),
            FindOptions::builder()
                .sort(doc! { "execution_started_at": 1 })
                .limit(MANAGEMENT_SWEEP_BATCH)
                .build(),
        )
        .await?;
    let candidates: Vec<AgentCommandRun> = cursor.try_collect().await?;
    let mut recovered = 0_u64;
    for run in candidates {
        if recover_one_stale_command(state, &run).await? {
            recovered += 1;
        }
    }
    if recovered > 0 {
        tracing::warn!(
            recovered,
            "stale management commands converged to execution_unknown"
        );
    }
    Ok(recovered)
}

fn stale_management_execution_before(now: DateTime) -> DateTime {
    DateTime::from_millis(now.timestamp_millis() - MANAGEMENT_EXECUTION_LEASE_MILLIS)
}

fn stale_management_filter(stale_before: DateTime) -> mongodb::bson::Document {
    doc! {
        "status": "running",
        "execution_started_at": { "$lte": stale_before },
        "execution_token": { "$type": "string" },
    }
}

async fn recover_one_stale_command(
    state: &AppState,
    run: &AgentCommandRun,
) -> anyhow::Result<bool> {
    let (Some(run_id), Some(token), Some(started_at)) = (
        run.id,
        run.execution_token.as_deref(),
        run.execution_started_at,
    ) else {
        return Ok(false);
    };
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result: anyhow::Result<bool> = async {
        let finalized = state
            .db
            .command_runs()
            .update_one_with_session(
                doc! {
                    "_id": run_id,
                    "workspace_id": &run.workspace_id,
                    "account_id": &run.account_id,
                    "status": "running",
                    "execution_token": token,
                    "execution_started_at": started_at,
                },
                doc! {
                    "$set": {
                        "status": "execution_unknown",
                        "summary": "执行租约过期，结果未知；为避免重复副作用，系统不会自动重放。",
                        "error": "management_execution_lease_expired",
                        "updated_at": DateTime::now(),
                    },
                    "$unset": { "execution_token": "", "execution_started_at": "" },
                },
                None,
                &mut session,
            )
            .await?;
        if finalized.matched_count != 1 {
            return Ok(false);
        }
        state
            .db
            .tool_calls()
            .update_many_with_session(
                doc! {
                    "command_run_id": run_id,
                    "workspace_id": &run.workspace_id,
                    "account_id": &run.account_id,
                    "status": "executing",
                },
                doc! { "$set": {
                    "status": "execution_unknown",
                    "error": "management execution lease expired before outcome was persisted",
                    "finalized_at": DateTime::now(),
                    "updated_at": DateTime::now(),
                } },
                None,
                &mut session,
            )
            .await?;
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => {
            session.commit_transaction().await?;
            Ok(true)
        }
        Ok(false) => {
            let _ = session.abort_transaction().await;
            Ok(false)
        }
        Err(error) => {
            let _ = session.abort_transaction().await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_filter_only_targets_owned_running_commands() {
        let before = DateTime::from_millis(1234);
        let filter = stale_management_filter(before);
        assert_eq!(filter.get_str("status").unwrap(), "running");
        assert_eq!(
            filter
                .get_document("execution_started_at")
                .unwrap()
                .get_datetime("$lte")
                .unwrap(),
            &before
        );
        assert_eq!(
            filter
                .get_document("execution_token")
                .unwrap()
                .get_str("$type")
                .unwrap(),
            "string"
        );
    }

    #[test]
    fn stale_boundary_uses_the_execution_lease() {
        let now = DateTime::from_millis(1_000_000);
        assert_eq!(
            stale_management_execution_before(now).timestamp_millis(),
            1_000_000 - MANAGEMENT_EXECUTION_LEASE_MILLIS
        );
    }
}
