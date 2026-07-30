//! Re-run destructive legacy cleanup for databases where old production guards
//! wrote a false-positive migration marker. The runner requires explicit
//! production approval before invoking this step.

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    super::m011_drop_legacy_sales_collections::run_step(db).await?;
    super::m012_drop_legacy_taxonomy_seed::run_step(db).await?;
    super::m014_drop_trigger_keywords::run_step(db).await?;
    Ok(())
}
