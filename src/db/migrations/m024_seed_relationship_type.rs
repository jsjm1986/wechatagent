//! 2026_06_X8_001：§3.7 数字分身——seed `relationship_type` taxonomy 字典三值。
//!
//! 幂等（`$setOnInsert` upsert，不覆盖运营后续编辑）：seed 独立 kind `relationship_type`
//! 的三值 `customer` / `peer` / `friend`（scope=`global`）。供运营接入时给 contact 标关系
//! 类型 + planner `resolve_operation_mode` 按关系类型选 profile 专属 OperationMode。
//!
//! 与 value_tier 的关键共性：**relationship_type 不经 LLM domainSignals 通道**（运营接入时
//! 经 admin 手动设入 `contact.domain_attributes.relationship_type`），**不需要 profile 维度
//! 声明**（retain_declared_dimensions 白名单只管 LLM 输出容器键，admin 直写分支不受约束）。
//! 故本 migration 只 seed 字典、不追加 profile 维度——字典用于 admin 写侧取值校验
//! （`dimension_registry::validate_dimension_value`，AdminDirect 通道越界 reject）+ UI 标签。
//!
//! 取值命名（已拍板）：英文 id（与 value_tier=high/mid/low、churn_reason 英文 id 惯例一致）
//! + 中文 label。aliases 补中文别名便于运营随手填中文时软归一到 canonical。运营可经
//! `POST /api/admin/taxonomies` 增删改（因行业而异，如加 `supplier` 供应商）。
//!
//! 红线（§2.2 命名）：取值/标签一律中性词，无禁词。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::db::Database;
use crate::error::AppResult;
use crate::models::{TaxonomyEntry, TaxonomyValue};

/// `relationship_type` 字典 kind。
pub(super) const RELATIONSHIP_TYPE_KIND: &str = "relationship_type";
/// 三 canonical 取值 id（英文，运营可扩展）。
pub(super) const RELATIONSHIP_TYPE_CUSTOMER: &str = "customer";
pub(super) const RELATIONSHIP_TYPE_PEER: &str = "peer";
pub(super) const RELATIONSHIP_TYPE_FRIEND: &str = "friend";

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    seed_relationship_type_taxonomy(db, now).await?;
    Ok(())
}

/// 关系类型三值。`(id, display, desc, aliases)`。
pub(super) fn relationship_type_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let values: &[(&str, &str, &str, &[&str])] = &[
        (
            RELATIONSHIP_TYPE_CUSTOMER,
            "客户",
            "购买方/潜在购买方，运营核心。漏斗推进 + 沉默唤醒 + 承诺跟进 + 日历关怀全开。",
            &["客户", "顾客", "买家"],
        ),
        (
            RELATIONSHIP_TYPE_PEER,
            "同行",
            "同业/合作方等专业社交关系。漏斗关、低频维护，祝福偏行业节点。",
            &["同行", "同业", "合作方", "同业伙伴"],
        ),
        (
            RELATIONSHIP_TYPE_FRIEND,
            "朋友",
            "机主的个人社交关系。漏斗关、口吻最像本人，祝福偏个人情感。",
            &["朋友", "好友", "私交"],
        ),
    ];
    values
        .iter()
        .map(|(id, display, desc, aliases)| TaxonomyEntry {
            id: None,
            workspace_id: crate::models::default_taxonomy_workspace_id(),
            scope: "global".to_string(),
            kind: RELATIONSHIP_TYPE_KIND.to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                // 关系类型不参与 planner 漏斗排序（它走 resolve_operation_mode 选范式，非排序权重）。
                priority_weight: None,
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: Some("relationship_type_migration".to_string()),
        })
        .collect()
}

async fn seed_relationship_type_taxonomy(db: &Database, now: DateTime) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    for entry in relationship_type_seed_entries(now) {
        let filter = doc! {
            "workspace_id": &entry.workspace_id,
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
        migration_id = "2026_06_X8_001_seed_relationship_type",
        inserted,
        skipped,
        "seeded relationship_type taxonomy (3 values)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_type_seed_covers_three_canonical_values() {
        let now = DateTime::now();
        let entries = relationship_type_seed_entries(now);
        let ids: Vec<&str> = entries.iter().map(|e| e.value.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                RELATIONSHIP_TYPE_CUSTOMER,
                RELATIONSHIP_TYPE_PEER,
                RELATIONSHIP_TYPE_FRIEND
            ]
        );
        for e in &entries {
            assert_eq!(e.scope, "global");
            assert_eq!(e.kind, RELATIONSHIP_TYPE_KIND);
            assert_eq!(e.value.status, "active");
            assert!(e.value.priority_weight.is_none());
            assert!(!e.value.is_terminal);
            assert!(!e.value.aliases.is_empty());
        }
    }

    #[test]
    fn relationship_type_entries_unique_by_scope_kind_id() {
        let now = DateTime::now();
        let entries = relationship_type_seed_entries(now);
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
