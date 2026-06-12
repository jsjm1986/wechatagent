//! 2026_06_X3_001：H13——为 `user_operations.state_machine` 中的 state 补齐
//! `initial` / `forbidsProactive` 标志位，按默认状态机的同 key 默认值回填，
//! 只补缺失、不覆盖运营人员已写过的值。
//!
//! 引擎 `check_state_transition` 与 planner 从写死的 `"new_contact"` / `"cooldown"`
//! 字符串比较切到读这两个标志位（H13-2/H13-4）后，存量 `operation_domain_configs`
//! 的 state_machine 必须先有标志位，否则空 from 迁移会被全判非法、cooldown 禁触达
//! 失效。本 migration 兜写侧，serde `#[serde(default)]` 兜读侧（双保险）。
//!
//! **部署顺序**：本 migration 必须先于读标志的引擎/planner 代码上线（migrations::run
//! 在 ensure_indexes 前、应用 serve 前执行，天然满足）。

use futures::TryStreamExt;
use mongodb::bson::{doc, DateTime};

use crate::db::Database;
use crate::error::AppResult;

use super::helpers::merge_state_flag_defaults;

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
        .find(doc! { "domain": "user_operations" }, None)
        .await?;
    let mut modified = 0_u64;
    while let Some(config) = cursor.try_next().await? {
        let Some(id) = config.id else { continue };
        let mut state_machine = config.state_machine.clone();
        if merge_state_flag_defaults(&mut state_machine, &default_states) {
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
        "backfilled state_machine initial/forbidsProactive flags for user_operations domain"
    );
    Ok(())
}
