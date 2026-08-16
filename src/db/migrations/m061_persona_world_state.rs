//! Audit the account-wide current persona world-state pointer before creating its unique index.

use std::collections::BTreeMap;

use futures::TryStreamExt;
use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::{AppError, AppResult};

fn audit_current_rows(rows: impl IntoIterator<Item = Document>) -> AppResult<usize> {
    let mut currents: BTreeMap<(String, String), usize> = BTreeMap::new();
    for row in rows {
        let workspace_id = row
            .get_str("workspace_id")
            .map_err(|_| AppError::Conflict("persona world state missing workspace_id".into()))?;
        let account_id = row
            .get_str("account_id")
            .map_err(|_| AppError::Conflict("persona world state missing account_id".into()))?;
        if workspace_id.trim().is_empty() || account_id.trim().is_empty() {
            return Err(AppError::Conflict(
                "persona world state has empty ownership identity".into(),
            ));
        }
        *currents
            .entry((workspace_id.to_string(), account_id.to_string()))
            .or_default() += 1;
    }
    if let Some(((workspace_id, account_id), count)) =
        currents.iter().find(|(_, count)| **count > 1)
    {
        return Err(AppError::Conflict(format!(
            "multiple current persona world states for {workspace_id}/{account_id}: {count}"
        )));
    }
    Ok(currents.len())
}

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let mut cursor = db
        .persona_world_states()
        .clone_with_type::<Document>()
        .find(doc! { "current": true }, None)
        .await?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        rows.push(row);
    }
    let accounts = audit_current_rows(rows)?;
    tracing::info!(
        migration_id = "2026_08_061_persona_world_state",
        accounts,
        "audited current persona world-state invariant"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_current_state_fails_without_election() {
        let rows = [
            doc! { "workspace_id": "ws", "account_id": "a" },
            doc! { "workspace_id": "ws", "account_id": "a" },
        ];
        assert!(audit_current_rows(rows).is_err());
    }
}
