//! 2026_07_033: retire the pre-tenant outcome task unique index.
//!
//! The old key omitted `workspace_id`, so two workspaces using the same account id and daily
//! aggregation payload incorrectly collided. `Database::ensure_indexes` creates the replacement
//! workspace-scoped index after migrations complete. Dropping a missing index is intentionally
//! best-effort so fresh databases and partially upgraded deployments remain restart-safe.

use futures::TryStreamExt;

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    const LEGACY_INDEX: &str = "uniq_outcome_aggregation_kind_account_content";
    let collections = db.raw().list_collection_names(None).await?;
    if !collections.iter().any(|name| name == "agent_tasks") {
        return Ok(());
    }
    let mut indexes = db.tasks().list_indexes(None).await?;
    let mut legacy_exists = false;
    while let Some(index) = indexes.try_next().await? {
        legacy_exists |= index
            .options
            .as_ref()
            .and_then(|options| options.name.as_deref())
            == Some(LEGACY_INDEX);
    }
    if legacy_exists {
        db.tasks().drop_index(LEGACY_INDEX, None).await?;
    }
    tracing::info!(
        migration_id = "2026_07_033_task_commit_indexes",
        "retired legacy outcome task unique index"
    );
    Ok(())
}
