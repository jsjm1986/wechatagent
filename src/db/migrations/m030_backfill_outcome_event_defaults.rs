//! 2026_07_030：回填 outcome_events 数组元素缺失的 verification/event_kind 默认值。
//!
//! 背景(KC-05)：`OutcomeEvent.verification`(models.rs:451 default→staff_confirmed) 与
//! `event_kind`(models.rs:464 default→deal) 的 `#[serde(default)]` 只作用于反序列化、
//! Mongo 查询不补。§4.5(2026-06-15)字段上线前登记的老成交事件 BSON 里没这两键，
//! campaign 圈人粗筛 `$elemMatch` 精确匹配(campaigns.rs)对缺字段落空 → product 定向
//! 活动静默漏老客户。防线 A(查询侧 $exists/$ne 对齐)已即时止血；本迁移治本清历史，
//! 彻底消除 serde 默认与 Mongo 查询的长期口径分裂。
//!
//! **不加 APP_ENV=production 守卫**：本回填写的就是 serde 读时本已假设的默认值
//! (staff_confirmed/deal)，语义保持、非破坏、幂等——与 m018/m022/m025 同类(它们均无
//! 守卫、生产照跑)。带守卫的 m011/m012/m014 是破坏性 drop、m016 是多租户前置回填(有
//! "过早回填致误黑"特定危害)，均与本迁移性质不同。误加守卫会致 117 生产静默 SKIP、
//! 防线 B 名存实亡。
//!
//! **存储键**：Contact **无** `#[serde(rename_all)]`(models.rs:148)→ 顶层字段存 snake_case
//! `outcome_events`(见 db/indexes.rs:38-40 索引键 + 防线A campaigns.rs 亦用 snake_case)。
//! 内层 OutcomeEvent 带 `rename_all="camelCase"` → `event_kind`→`eventKind`、`verification` 不变。
//! **兼容 legacy alias**：Contact.outcome_events serde alias="deal_events"(models.rs:248)，
//! 故极老文档数组键可能是 `deal_events`；两个键各回填一次。
//!
//! 合并策略：`$map` 遍历数组，每元素 `$mergeObjects([默认值底, $$ev])` —— 默认值在底、
//! 元素已有键在上覆盖，**只补缺失键**，已有 conversation_inferred/reversal 原值胜出不改。
//!
//! 幂等：二次执行元素已有两键、mergeObjects 结果不变、modified_count → 0。

use mongodb::bson::{doc, Document};

use crate::db::Database;
use crate::error::AppResult;

/// 对单个数组字段名构造回填 pipeline 段(纯函数,便于单测)。
/// `$map` 遍历 `$field`,每元素合并"默认值底 + 元素本身",元素已有键覆盖默认值。
pub(super) fn backfill_array(field: &str) -> Document {
    doc! { "$set": { field: {
        "$map": {
            "input": { "$ifNull": [format!("${field}"), []] },
            "as": "ev",
            "in": { "$mergeObjects": [
                { "verification": "staff_confirmed", "eventKind": "deal" },
                "$$ev",
            ]},
        }
    }}}
}

/// 命中过滤器:两个数组字段任一存在即需回填(纯函数,便于单测)。
pub(super) fn backfill_filter() -> Document {
    doc! {
        "$or": [
            { "outcome_events": { "$exists": true } },
            { "deal_events": { "$exists": true } },
        ]
    }
}

/// 迁移主体。`pub` 暴露给 `tests/` 集成测试(同 m018/m029 先例)。
/// 对 outcome_events 与 legacy deal_events 各跑一次 update_many pipeline。
pub async fn run_step(db: &Database) -> AppResult<()> {
    for field in ["outcome_events", "deal_events"] {
        let result = db
            .contacts()
            .update_many(backfill_filter(), vec![backfill_array(field)], None)
            .await?;
        tracing::info!(
            migration_id = "2026_07_030_backfill_outcome_event_defaults",
            field = field,
            modified = result.modified_count,
            matched = result.matched_count,
            "backfilled missing verification/eventKind defaults into outcome_events elements"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_array_maps_with_defaults_as_base_and_element_on_top() {
        let stage = backfill_array("outcome_events");
        let set = stage.get_document("$set").unwrap();
        let field = set.get_document("outcome_events").unwrap();
        let map = field.get_document("$map").unwrap();
        // $map 遍历该字段(用 $ifNull 兜空)
        assert!(map.contains_key("input"));
        assert_eq!(map.get_str("as").unwrap(), "ev");
        // in 是 $mergeObjects([默认值底, $$ev])——默认在前(底)、元素在后(覆盖)
        let merge = map.get_document("in").unwrap().get_array("$mergeObjects").unwrap();
        assert_eq!(merge.len(), 2);
        let base = merge[0].as_document().unwrap();
        assert_eq!(base.get_str("verification").unwrap(), "staff_confirmed");
        assert_eq!(base.get_str("eventKind").unwrap(), "deal");
        assert_eq!(merge[1].as_str().unwrap(), "$$ev", "元素本身须在末位覆盖默认值(只补缺失键)");
    }

    #[test]
    fn backfill_filter_matches_either_array_key() {
        let filter = backfill_filter();
        let or = filter.get_array("$or").unwrap();
        assert_eq!(or.len(), 2);
        let keys: Vec<String> = or
            .iter()
            .filter_map(|b| b.as_document())
            .flat_map(|d| d.keys().cloned().collect::<Vec<_>>())
            .collect();
        assert!(keys.contains(&"outcome_events".to_string()), "须命中 snake_case outcome_events(Contact 无 rename_all)");
        assert!(keys.contains(&"deal_events".to_string()), "须命中 legacy alias deal_events");
    }
}
