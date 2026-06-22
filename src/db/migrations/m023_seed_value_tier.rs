//! 2026_06_X7_001：G6 客户价值分层——seed `value_tier` taxonomy 字典三值。
//!
//! 幂等（`$setOnInsert` upsert，不覆盖运营后续编辑）：seed 独立 kind `value_tier` 的三值
//! `high` / `mid` / `low`（scope=`global`）。供前端展示/筛选 + canonical 取值一致性。
//!
//! 与 G1（purchase_lifecycle）/ churn_reason 的关键区别：**value_tier 是客观计算派生值**
//! （gateway 按累计成交额规则算后直接写 `domain_attributes.value_tier`），**不经 LLM
//! domainSignals 通道、不需要 profile 维度声明**（retain_declared_dimensions 白名单只管
//! LLM 输出的容器键，独立写入分支不受其约束）。故本 migration 只 seed 字典、不追加 profile
//! 维度——字典纯粹用于 UI 标签 + 校验取值集合。value.id 与 `entitlements::VALUE_TIER_*`
//! 常量逐字一致。
//!
//! 红线（§2.2 命名）：取值/标签一律中性词，无禁词。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::agent::entitlements::{VALUE_TIER_HIGH, VALUE_TIER_KIND, VALUE_TIER_LOW, VALUE_TIER_MID};
use crate::db::Database;
use crate::error::AppResult;
use crate::models::{TaxonomyEntry, TaxonomyValue};

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    seed_value_tier_taxonomy(db, now).await?;
    Ok(())
}

/// 价值分层三值。`(id, display, desc, aliases)`。
pub(super) fn value_tier_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let values: &[(&str, &str, &str, &[&str])] = &[
        (
            VALUE_TIER_HIGH,
            "高价值",
            "累计已核实成交额达高价值门槛的客户，运营应优先投入、更用心维护。",
            &["高价值客户", "高净值", "VIP"],
        ),
        (
            VALUE_TIER_MID,
            "中价值",
            "累计已核实成交额达中价值门槛的客户，稳健维护、伺机提升。",
            &["中价值客户", "腰部"],
        ),
        (
            VALUE_TIER_LOW,
            "低价值",
            "累计成交额低于中价值门槛（含未成交）的客户，低成本广覆盖。",
            &["低价值客户", "长尾", "未成交"],
        ),
    ];
    values
        .iter()
        .map(|(id, display, desc, aliases)| TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: VALUE_TIER_KIND.to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                // 价值分层不参与 planner 漏斗排序（它走独立 value_tier_weight 排序维度）。
                priority_weight: None,
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: Some("g6_migration".to_string()),
        })
        .collect()
}

async fn seed_value_tier_taxonomy(db: &Database, now: DateTime) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    for entry in value_tier_seed_entries(now) {
        let filter = doc! {
            "scope": &entry.scope,
            "kind": &entry.kind,
            "value.id": &entry.value.id,
        };
        let mut doc_to_set = mongodb::bson::to_document(&entry)?;
        doc_to_set.remove("_id");
        let result = collection
            .update_one(
                filter,
                doc! { "$setOnInsert": doc_to_set },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        if result.upserted_id.is_some() {
            inserted += 1;
        } else {
            skipped += 1;
        }
    }
    tracing::info!(
        migration_id = "2026_06_X7_001_seed_value_tier",
        inserted,
        skipped,
        "seeded value_tier taxonomy (3 values)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_tier_seed_covers_three_canonical_values() {
        let now = DateTime::now();
        let entries = value_tier_seed_entries(now);
        let ids: Vec<&str> = entries.iter().map(|e| e.value.id.as_str()).collect();
        assert_eq!(ids, vec![VALUE_TIER_HIGH, VALUE_TIER_MID, VALUE_TIER_LOW]);
        for e in &entries {
            assert_eq!(e.scope, "global");
            assert_eq!(e.kind, VALUE_TIER_KIND);
            assert_eq!(e.value.status, "active");
            assert!(e.value.priority_weight.is_none());
            assert!(!e.value.is_terminal);
        }
    }

    #[test]
    fn value_tier_entries_unique_by_scope_kind_id() {
        let now = DateTime::now();
        let entries = value_tier_seed_entries(now);
        let mut keys: Vec<(String, String, String)> = entries
            .iter()
            .map(|e| (e.scope.clone(), e.kind.clone(), e.value.id.clone()))
            .collect();
        let original = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), original);
    }
}
