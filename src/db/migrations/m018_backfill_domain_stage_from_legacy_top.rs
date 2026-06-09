//! 2026_06_X2_001：把残留在 contact 文档**顶层**的 `customer_stage` /
//! `intent_level` / `customer_stage_updated_at` 回填进 `domain_attributes` 容器。
//!
//! 背景:`Contact` 模型早已删除顶层销售域字段、只保留 `domain_attributes` 容器,
//! 但在 PR-A(读写口径收口)之前,多处 admin 写路径一直把这些字段写到文档顶层
//! → serde 反序列化时丢弃、读端(planner / memory / decision)读不到。PR-A 止血
//! (不再产生新顶层值)后,本迁移清理线上老文档的顶层残留。
//!
//! **只回填、不 `$unset`**:把顶层值搬进 `domain_attributes`,但保留顶层字段不删,
//! 使迁移完全可逆(回滚后顶层数据仍在)。物理删除顶层留待后续 m019,等读端在生产
//! 验证稳定后再做。
//!
//! 合并策略:`$mergeObjects([顶层残留, 现有 domain_attributes])` —— 顶层值在前作
//! "底",现有 `domain_attributes` 在后,同名 key 由后者覆盖。即**仅在
//! `domain_attributes` 缺该 key 时才用顶层值回填**,绝不让陈旧顶层值盖掉较新的
//! domain 值(PR-A 之后 AI 自治路径写的就是 domain,可能比顶层新)。
//!
//! 幂等:首次执行把顶层值并入 domain;二次执行时 domain 已有该 key、mergeObjects
//! 结果不变,`modified_count` 为 0(顶层字段保留不影响结果稳定性)。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

/// 顶层残留字段 → `domain_attributes.*` 的回填聚合管道(纯函数,便于单测)。
///
/// 每个顶层字段用 `$cond` 包裹:存在(`$type` ≠ `"missing"`)才纳入合并底层,
/// 缺失则贡献空对象。`$ifNull($domain_attributes, {})` 放在 `$mergeObjects` 末位
/// 保证现有 domain 值覆盖顶层底值(新覆旧)。
pub(super) fn build_backfill_pipeline() -> Vec<Document> {
    let merge_field = |field: &str| -> Document {
        doc! {
            "$cond": [
                { "$ne": [{ "$type": format!("${field}") }, "missing"] },
                { field: format!("${field}") },
                {}
            ]
        }
    };
    vec![doc! {
        "$set": {
            "domain_attributes": {
                "$mergeObjects": [
                    merge_field("customer_stage"),
                    merge_field("intent_level"),
                    merge_field("customer_stage_updated_at"),
                    { "$ifNull": ["$domain_attributes", {}] }
                ]
            }
        }
    }]
}

/// 命中过滤器:三个顶层残留字段之一存在即需回填(纯函数,便于单测)。
pub(super) fn backfill_filter() -> Document {
    doc! {
        "$or": [
            { "customer_stage": { "$exists": true } },
            { "intent_level": { "$exists": true } },
            { "customer_stage_updated_at": { "$exists": true } }
        ]
    }
}

/// 迁移主体。`pub` 暴露给 `tests/` 集成测试:`TestApp::start()` 在空库上跑过本迁移
/// 后账册已存在,集成测试需对预置的顶层残留数据**单独**调用本函数验证回填语义。
pub async fn run_step(db: &Database) -> AppResult<()> {
    let result = db
        .contacts()
        .update_many(backfill_filter(), build_backfill_pipeline(), None)
        .await?;
    tracing::info!(
        migration_id = "2026_06_X2_001_backfill_domain_stage_from_legacy_top",
        modified = result.modified_count,
        matched = result.matched_count,
        "backfilled legacy top-level customer_stage/intent_level into domain_attributes (kept top-level for reversibility)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_merges_into_domain_attributes_without_unset() {
        let pipeline = build_backfill_pipeline();
        // 单 $set 阶段,绝不含 $unset(本迁移可逆,顶层保留)。
        assert_eq!(pipeline.len(), 1);
        let stage = &pipeline[0];
        assert!(stage.contains_key("$set"));
        assert!(!stage.contains_key("$unset"));
    }

    #[test]
    fn pipeline_targets_domain_attributes_with_existing_last_in_merge() {
        let pipeline = build_backfill_pipeline();
        let merge = pipeline[0]
            .get_document("$set")
            .unwrap()
            .get_document("domain_attributes")
            .unwrap()
            .get_array("$mergeObjects")
            .unwrap();
        // 四个合并源:三个顶层字段 + 末位现有 domain_attributes。
        assert_eq!(merge.len(), 4);
        // 末位必须是 $ifNull(现有 domain),保证现有 domain 值覆盖顶层底值(新覆旧)。
        let last = merge.last().unwrap().as_document().unwrap();
        let if_null = last.get_array("$ifNull").unwrap();
        assert_eq!(if_null[0].as_str(), Some("$domain_attributes"));
    }

    #[test]
    fn filter_matches_any_legacy_top_level_field() {
        let filter = backfill_filter();
        let or = filter.get_array("$or").unwrap();
        assert_eq!(or.len(), 3);
        // 三个顶层残留字段各一个 $exists 子句。
        let keys: Vec<String> = or
            .iter()
            .filter_map(|b| b.as_document())
            .flat_map(|d| d.keys().cloned().collect::<Vec<_>>())
            .collect();
        assert!(keys.contains(&"customer_stage".to_string()));
        assert!(keys.contains(&"intent_level".to_string()));
        assert!(keys.contains(&"customer_stage_updated_at".to_string()));
    }
}
