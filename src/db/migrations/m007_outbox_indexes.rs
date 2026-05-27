//! 2026_05_007（占位 / W0 即可生效）：
//! `agent_send_outbox` 集合的索引在 task 1.2 已被 `Database::ensure_indexes()` 幂等创建。
//! 本迁移是显式 marker，标记"outbox 表的索引契约自此版本起被纳入迁移轨道"，
//! 二次启动时 `migrations` 集合的 `_id` 唯一约束会让本条 skip。
//!
//! 注意：真正的索引创建发生在 `ensure_indexes()`，那里本身就是幂等的
//! （`create_indexes` 对已存在的相同索引是 no-op），故本迁移即使在历史数据库
//! 上首次执行也是安全的。

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(_db: &Database) -> AppResult<()> {
    tracing::info!(
        migration_id = "2026_05_007_outbox_indexes",
        "scaffold migration applied (no-op marker); outbox indexes are created idempotently in ensure_indexes()"
    );
    Ok(())
}
