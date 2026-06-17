//! 2026_06_X5_001：续费闭环阶段2（流失原因字典 + 示例 profile 追加 churn_reason 维度）。
//!
//! 两件事，都幂等：
//!
//! 1. **seed `churn_reason` taxonomy 值集**（scope=`global`，独立 kind）：
//!    `price_too_high` / `effect_unmet` / `need_changed` / `switched_competitor` /
//!    `timing` / `other`。这是「流失原因」维度——区别于 `objection_type`（对话中的
//!    异议）：churn_reason 是「为什么不续/流失」的归因，供 `scan_reactivation` 按原因
//!    精准再激活。用独立 kind 让 AI 经 domainSignals 输出的值被正确校验为合法（非
//!    CandidateNew），扫描器按 `domain_attributes.churn_reason` 干净过滤。
//!
//! 2. **给「带购买生命周期」示例 profile 追加 `churn_reason` 参与决策维度**：让该交易域
//!    示例 profile 的 LLM 能经 `domainSignals.churn_reason` 输出流失原因并通过
//!    `retain_declared_dimensions` 白名单落库。用 `$addToSet` 幂等追加（m020 可能已
//!    seed 过该 profile）；**仅当其 `is_active=false` 草稿态时改**，不动运营已激活配置。
//!
//! 零扰动：DEFAULT 销售 profile 不声明 churn_reason 维度 → AI 不输出、不落库，
//! 行为与改造前逐字等价。红线（§2.2 命名）：取值/标签一律中性词，无禁词。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::db::Database;
use crate::error::AppResult;
use crate::models::{TaxonomyEntry, TaxonomyValue};

/// 流失原因 taxonomy 维度 kind（独立于 objection_type）。
const CHURN_REASON_KIND: &str = "churn_reason";
/// m020 seed 的示例交易域 profile id（本迁移给它追加 churn_reason 维度）。
const EXAMPLE_PROFILE_ID: &str = "sales-with-lifecycle-example";

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    seed_churn_reason_taxonomy(db, now).await?;
    add_churn_reason_dimension_to_example_profile(db).await?;
    Ok(())
}

/// 流失原因六值。`(id, display, desc, aliases)`。
pub(super) fn churn_reason_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let values: &[(&str, &str, &str, &[&str])] = &[
        (
            "price_too_high",
            "价格太贵",
            "因价格/续费费用超出预算或感觉性价比不足而不再续费。",
            &["太贵", "价格高", "续费贵", "预算不够"],
        ),
        (
            "effect_unmet",
            "效果不满意",
            "对已购产品/服务的实际效果、收益不满意，认为不值得续费。",
            &["效果差", "没效果", "不满意", "没达到预期"],
        ),
        (
            "need_changed",
            "需求变化",
            "自身需求/场景发生变化，当前不再需要该产品或服务。",
            &["不需要了", "需求没了", "用不上", "情况变了"],
        ),
        (
            "switched_competitor",
            "转竞品",
            "选择了竞争对手的同类产品/服务作为替代。",
            &["用别家了", "转其他", "竞品替代", "换供应商"],
        ),
        (
            "timing",
            "时机不合适",
            "暂时搁置，时机不对（如近期不打算续、过段时间再说）。",
            &["暂时不续", "再等等", "过段时间", "时机不对"],
        ),
        (
            "other",
            "其他原因",
            "未归类的真实流失原因，待运营审核后补入字典或合并到既有维度。",
            &["其他", "未分类"],
        ),
    ];
    values
        .iter()
        .map(|(id, display, desc, aliases)| TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: CHURN_REASON_KIND.to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                // 流失原因不参与 planner 漏斗排序，无权重/终态语义。
                priority_weight: None,
                is_terminal: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: Some("g5_stage2_migration".to_string()),
        })
        .collect()
}

async fn seed_churn_reason_taxonomy(db: &Database, now: DateTime) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    for entry in churn_reason_seed_entries(now) {
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
        migration_id = "2026_06_X5_001_seed_churn_reason",
        inserted,
        skipped,
        "seeded churn_reason taxonomy (6 values)"
    );
    Ok(())
}

/// churn_reason 参与决策维度的 BSON 子文档。抽成纯函数便于测试锁死「doc 字段名 ==
/// ProfileDimension serde 字段名」——手写 doc!{} 无编译器校验，键名拼错会静默丢字段
/// （反序列化回落 participates_in_decision=false → decision_dimension_kinds 不返回它
/// → retain_declared_dimensions 剔除 → 流失原因落库静默失败）。
pub(super) fn churn_reason_dimension_doc() -> mongodb::bson::Document {
    doc! {
        "kind": CHURN_REASON_KIND,
        "display_name": "流失原因",
        "participates_in_decision": true,
        "description": "客户不续费/流失的归因（价格太贵/效果不满意/需求变化/转竞品/时机不合适）。\
                        由对话推断，供再激活扫描器按原因精准重连。",
    }
}

/// 给 m020 的示例交易域 profile 追加 churn_reason 参与决策维度。仅改草稿态
/// （`is_active=false`），不动运营已激活配置。`$addToSet` 幂等（按完整子文档去重，
/// 重复运行不重复追加）。
async fn add_churn_reason_dimension_to_example_profile(db: &Database) -> AppResult<()> {
    let collection = db.domain_profiles();
    let dim = churn_reason_dimension_doc();
    let result = collection
        .update_one(
            doc! {
                "workspace_id": "default",
                "profile_id": EXAMPLE_PROFILE_ID,
                "is_active": false,
            },
            doc! { "$addToSet": { "profile_dimensions": dim } },
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_06_X5_001_seed_churn_reason",
        matched = result.matched_count,
        modified = result.modified_count,
        "added churn_reason dimension to example lifecycle profile (draft only)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProfileDimension;

    #[test]
    fn churn_reason_seed_covers_six_canonical_values() {
        let now = DateTime::now();
        let entries = churn_reason_seed_entries(now);
        let ids: Vec<&str> = entries.iter().map(|e| e.value.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "price_too_high",
                "effect_unmet",
                "need_changed",
                "switched_competitor",
                "timing",
                "other",
            ]
        );
        for e in &entries {
            assert_eq!(e.scope, "global");
            assert_eq!(e.kind, CHURN_REASON_KIND);
            assert_eq!(e.value.status, "active");
            assert!(e.value.priority_weight.is_none());
            assert!(!e.value.is_terminal);
        }
    }

    /// 命门守护：churn_reason 维度的手写 `doc!{}` 字段名必须与 `ProfileDimension` 的
    /// serde 字段名逐字匹配——否则反序列化静默丢字段（participates_in_decision 回落
    /// false → decision_dimension_kinds 不返回它 → retain_declared_dimensions 剔除
    /// → AI 输出的流失原因永远落不进 domain_attributes.churn_reason）。
    #[test]
    fn churn_reason_dimension_doc_deserializes_with_correct_field_names() {
        let dim: ProfileDimension =
            mongodb::bson::from_document(churn_reason_dimension_doc()).expect("doc 字段名须与 ProfileDimension serde 对齐");
        assert_eq!(dim.kind, CHURN_REASON_KIND);
        assert!(
            dim.participates_in_decision,
            "churn_reason 必须参与决策，否则 retain 白名单会剔除该维度、落库静默失败"
        );
        assert!(!dim.display_name.is_empty());
    }
}
