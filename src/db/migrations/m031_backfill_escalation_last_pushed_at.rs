//! 2026_07_031：回填 agent_principal_escalations 缺失的 last_pushed_at_ms（KD-05 治本）。
//!
//! 背景：KD-05 给台账加 last_pushed_at_ms（骚扰门真实推送时刻，改派刷新）。旧 pending 行
//! 无此字段（serde default→None），count_pushes_today/latest_push_ms 用 $gte/sort 会漏计。
//! 本迁移把现有行的 last_pushed_at_ms 补成 created_at（历史行"最近推送时刻"就近似取创建时刻，
//! 与旧 created_at 口径字节等价）。
//!
//! **不加 APP_ENV=production 守卫**：语义保持型回填（写的就是旧口径值），非破坏、幂等——
//! 与 m018/m022/m025/m030 同类（均无守卫、生产照跑）。误加会致 117 生产静默 SKIP。
//!
//! 幂等：仅 last_pushed_at_ms 缺失的行命中；二次跑 matched=0。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

/// 命中过滤器：缺 last_pushed_at_ms 的台账行（纯函数，便于单测）。
pub(super) fn backfill_filter() -> Document {
    doc! { "last_pushed_at_ms": { "$exists": false } }
}

/// 回填 pipeline：last_pushed_at_ms = created_at 的 epoch ms（纯函数，便于单测）。
/// $toLong($created_at) 把 BSON Date 转 epoch ms（与 last_pushed_at_ms 的 i64 存储一致）。
pub(super) fn backfill_pipeline() -> Vec<Document> {
    vec![doc! { "$set": { "last_pushed_at_ms": { "$toLong": "$created_at" } } }]
}

/// 迁移主体。`pub` 暴露给 tests/ 集成测（同 m018/m029/m030 先例）。
pub async fn run_step(db: &Database) -> AppResult<()> {
    let result = db
        .agent_principal_escalations()
        .update_many(backfill_filter(), backfill_pipeline(), None)
        .await?;
    tracing::info!(
        migration_id = "2026_07_031_backfill_escalation_last_pushed_at",
        modified = result.modified_count,
        matched = result.matched_count,
        "backfilled escalation last_pushed_at_ms from created_at (KD-05)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_targets_missing_field_only() {
        let f = backfill_filter();
        let cond = f.get_document("last_pushed_at_ms").unwrap();
        assert!(
            !cond.get_bool("$exists").unwrap(),
            "只命中 last_pushed_at_ms 缺失的行"
        );
    }

    #[test]
    fn pipeline_sets_from_created_at_as_long() {
        let p = backfill_pipeline();
        assert_eq!(p.len(), 1);
        let set = p[0].get_document("$set").unwrap();
        let field = set.get_document("last_pushed_at_ms").unwrap();
        // $toLong($created_at)：BSON Date → epoch ms i64，与字段存储类型一致。
        assert_eq!(field.get_str("$toLong").unwrap(), "$created_at");
    }
}
