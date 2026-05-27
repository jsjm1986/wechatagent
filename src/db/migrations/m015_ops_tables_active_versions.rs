//! 2026_05_W4_003（Phase E5-T1）：为 ops 三表（`operation_domain_configs` /
//! `operation_state_policies` / `system_taxonomies`）补齐 `active_versions` 灰度
//! 所需的四元字段：`version` / `current_version` / `previous_version` /
//! `seeded_by`。
//!
//! 字段语义见 `models.rs`：
//! - `version` 单调递增，老数据回填为 `1`；
//! - `current_version=true` 表示当前在被读路径选用（多版本可同时为 true，由
//!   `ab_bucket_for_contact(contact_id)` 哈希分桶决定单 contact 走哪一份）；
//! - `previous_version` / `seeded_by` 用于审计与回滚链路。
//!
//! 老数据没有这四个字段，serde 端通过 `#[serde(default)]` 已能反序列化为
//! `(0, false, None, None)`；这里补齐为 `(1, true, None, "legacy_migration")`，
//! 让 publish/rollout/rollback 路径有非零起点。
//!
//! 幂等：仅修改 `current_version: { $exists: false }` 的文档；二次启动时 `$exists`
//! 全为 true，`update_many` 自然 noop。
//!
//! 注意：唯一索引由 `db::indexes::ensure_indexes` 在迁移之后创建，本步只关心
//! 文档形态；新版本的 (workspace_id, domain[, state_key/value.id], version) 唯一
//! 索引在 indexes.rs 同 PR 一并落地。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let collections: [&str; 3] = [
        "operation_domain_configs",
        "operation_state_policies",
        "system_taxonomies",
    ];
    for coll_name in collections {
        let coll = db.raw().collection::<Document>(coll_name);
        let result = coll
            .update_many(
                doc! { "current_version": { "$exists": false } },
                doc! {
                    "$set": {
                        "version": 1_i32,
                        "current_version": true,
                        "previous_version": null,
                        "seeded_by": "legacy_migration",
                    }
                },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_05_W4_003_ops_tables_active_versions",
            collection = coll_name,
            modified = result.modified_count,
            "backfilled active_versions fields"
        );
    }
    Ok(())
}
