//! Re-run the canonical Prompt current-pointer reconciliation for upgraded
//! databases whose m043 marker was written before planning-only prompt specs
//! were required to remain unpublished drafts.
//!
//! Reusing m043 preserves its full-collection validation-before-write rule and
//! avoids a second, key-specific interpretation of the Prompt lifecycle.

use crate::db::Database;
use crate::error::AppResult;

pub async fn run_step(db: &Database) -> AppResult<()> {
    super::m043_prompt_single_current::run_step(db).await
}
