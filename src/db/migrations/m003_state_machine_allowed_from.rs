//! 2026_05_003：为 `user_operations.state_machine` 中缺失的状态补齐
//! `allowedFrom` / `allowFromAny`，但保留运营人员已经自定义的状态名称、
//! 目标、动作和规则。

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;

use super::helpers::merge_allowed_from_defaults;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let default_state_machine = crate::prompts::default_user_operation_state_machine();
    let default_states = default_state_machine
        .get_array("states")
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_document().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut cursor = db
        .operation_domain_configs()
        .find(
            doc! {
                "domain": "user_operations",
                "$or": [
                    { "state_machine.states.allowedFrom": { "$exists": false } },
                    { "state_machine.states.allowFromAny": { "$exists": false } }
                ]
            },
            None,
        )
        .await?;
    let mut modified = 0_u64;
    while let Some(config) = cursor.try_next().await? {
        let Some(id) = config.id else { continue };
        let mut state_machine = config.state_machine.clone();
        if merge_allowed_from_defaults(&mut state_machine, &default_states) {
            db.operation_domain_configs()
                .update_one(
                    doc! { "_id": id },
                    doc! {
                        "$set": {
                            "state_machine": state_machine,
                            "updated_at": DateTime::now()
                        }
                    },
                    None,
                )
                .await?;
            modified += 1;
        }
    }
    tracing::info!(
        modified,
        "seeded state_machine.allowedFrom defaults for user_operations domain"
    );
    Ok(())
}
