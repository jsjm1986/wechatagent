//! Re-run workspace backfill for databases where the old production guard
//! wrote a false-positive marker. Production requires explicit approval because
//! assigning unscoped historical rows to DEFAULT_WORKSPACE_ID is an ownership
//! decision.

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    super::m016_backfill_workspace_id_on_legacy_rows::run_step(db).await?;
    // The safe review reconciliation normally runs before this approval-gated
    // ownership backfill. Re-run it after assigning workspace ids so legacy
    // prompt/contact rows that were previously unscoped are not skipped
    // forever behind the already-applied m034 marker.
    super::m034_reconcile_review_fixes::run_step(db).await
}
