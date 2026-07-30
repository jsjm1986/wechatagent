//! Materialize the legacy empty-admin-ACL fallback before empty ACL changes
//! meaning from "default workspace" to "no workspace access".
//!
//! This preserves access for pre-upgrade single-workspace admins while making
//! a later deliberate `$set: { workspaces: [] }` an immediate full revocation.

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let default_workspace =
        std::env::var("DEFAULT_WORKSPACE_ID").unwrap_or_else(|_| "default".to_string());
    let result = db
        .raw()
        .collection::<Document>("admin_users")
        .update_many(
            doc! {
                "$or": [
                    { "workspaces": { "$exists": false } },
                    { "workspaces": { "$size": 0 } },
                ]
            },
            doc! {
                "$set": {
                    "workspaces": [&default_workspace],
                    "default_workspace": &default_workspace,
                }
            },
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_07_037_materialize_admin_acl",
        modified = result.modified_count,
        "materialized legacy empty admin ACLs"
    );
    Ok(())
}
