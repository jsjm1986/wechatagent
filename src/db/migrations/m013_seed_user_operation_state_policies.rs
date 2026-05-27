//! 2026_05_W4_001：为每个 user_operations 状态机里的 state_key 写一行默认 policy。
//!
//! 默认策略遵循"宽允许 / 窄禁止"原则：所有 state 默认允许 `["reply","silent","follow_up"]`
//! 三动作；只有 `cooldown` state 强制 `forbidden=["reply"]`（冷却期不主动回复）。
//! 已存在的 (workspace_id, domain, state_key) 行被跳过，保留运营人员的手工调整。

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;
use crate::models::OperationStatePolicy;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let mut cursor = db
        .operation_domain_configs()
        .find(doc! { "domain": "user_operations" }, None)
        .await?;
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    while let Some(config) = cursor.try_next().await? {
        let states = config
            .state_machine
            .get_array("states")
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_document().cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for state in &states {
            let Some(state_key) = state.get_str("key").ok().map(ToString::to_string) else {
                continue;
            };
            let existing = db
                .operation_state_policies()
                .find_one(
                    doc! {
                        "workspace_id": &config.workspace_id,
                        "domain": "user_operations",
                        "state_key": &state_key,
                    },
                    None,
                )
                .await?;
            if existing.is_some() {
                skipped += 1;
                continue;
            }
            let (allowed, forbidden): (Vec<String>, Vec<String>) = if state_key == "cooldown" {
                (
                    vec!["silent".to_string(), "follow_up".to_string()],
                    vec!["reply".to_string()],
                )
            } else {
                (
                    vec![
                        "reply".to_string(),
                        "silent".to_string(),
                        "follow_up".to_string(),
                    ],
                    Vec::new(),
                )
            };
            let policy = OperationStatePolicy {
                id: None,
                workspace_id: config.workspace_id.clone(),
                domain: "user_operations".to_string(),
                state_key: state_key.clone(),
                allowed,
                forbidden,
                recommended_pace: None,
                status: "active".to_string(),
                updated_at: DateTime::now(),
                version: 1,
                current_version: true,
                previous_version: None,
                seeded_by: Some("legacy_migration".to_string()),
            };
            db.operation_state_policies()
                .insert_one(&policy, None)
                .await?;
            inserted += 1;
        }
    }
    tracing::info!(
        migration_id = "2026_05_W4_001_seed_user_operation_state_policies",
        inserted,
        skipped,
        "seeded operation_state_policies for user_operations"
    );
    Ok(())
}
