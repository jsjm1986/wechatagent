//! 2026_06_X4_001：客观购买事实增强 spec §6（G1 购买生命周期维度）。
//!
//! 两件事，都幂等（`$setOnInsert` upsert，不覆盖运营后续编辑）：
//!
//! 1. **seed `purchase_lifecycle` taxonomy 四值**（scope=`global`）：
//!    `not_purchased` / `purchased` / `aftercare` / `repurchase`。value.id 与
//!    `agent::entitlements::G1_*` 常量逐字一致——G4→G1 纠偏产出的覆盖值必须落在
//!    该字典内，否则下游 taxonomy 校验把它当 CandidateNew。
//!
//! 2. **seed 一份「带购买生命周期」的示例行业 profile**（`profile_id=
//!    sales-with-lifecycle-example`，draft 态 `is_active=false` +
//!    `current_version=false`）：在销售域 DEFAULT 两维基础上追加 `purchase_lifecycle`
//!    作为第三个「参与决策」维度。**不激活**——运营在引导/审核 UI 确认后才
//!    publish+activate。这保证零扰动：无 active profile 时运行时仍回落
//!    DEFAULT_PROFILE（两维），与改造前逐字等价；本 seed 只是把"开箱即用的示例"
//!    放进库，供前端演示 + 运营改造。
//!
//! 红线（§2.2 命名）：取值/标签一律 AI 中性词，不含 `人工`/`接管`/`takeover` 等禁词。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::agent::domain_profile::default_domain_profile;
use crate::agent::entitlements::{
    G1_AFTERCARE, G1_DIMENSION_KIND, G1_NOT_PURCHASED, G1_PURCHASED, G1_REPURCHASE,
};
use crate::db::Database;
use crate::error::AppResult;
use crate::models::{ProfileDimension, TaxonomyEntry, TaxonomyValue};

/// 示例行业 profile 的 profile_id（draft，不自动激活）。
const EXAMPLE_PROFILE_ID: &str = "sales-with-lifecycle-example";

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    seed_purchase_lifecycle_taxonomy(db, now).await?;
    seed_example_profile(db, now).await?;
    Ok(())
}

/// spec §6：购买生命周期 taxonomy 四值。`(id, display, desc, aliases)`。
pub(super) fn purchase_lifecycle_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let values: &[(&str, &str, &str, &[&str])] = &[
        (
            G1_NOT_PURCHASED,
            "未购买",
            "尚无任何已核实成交；处于咨询/了解阶段。",
            &["未成交", "咨询期", "未购"],
        ),
        (
            G1_PURCHASED,
            "已购买",
            "有已核实成交持有，但无明确售后时效或时效信息未知。",
            &["已成交", "已购", "成交客户"],
        ),
        (
            G1_AFTERCARE,
            "售后期",
            "有已核实成交且仍在售后/有效期内，运营应转向关怀与履约。",
            &["售后期内", "服务期", "有效期内"],
        ),
        (
            G1_REPURCHASE,
            "复购期",
            "已购买后进入可复购/续费窗口，可基于真实价值做低压唤醒。",
            &["复购", "续费期", "回购"],
        ),
    ];
    values
        .iter()
        .map(|(id, display, desc, aliases)| TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: G1_DIMENSION_KIND.to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                // 购买生命周期不参与 planner 漏斗排序，无权重/终态语义。
                priority_weight: None,
                is_terminal: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: Some("g1_migration".to_string()),
        })
        .collect()
}

async fn seed_purchase_lifecycle_taxonomy(db: &Database, now: DateTime) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    for entry in purchase_lifecycle_seed_entries(now) {
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
        migration_id = "2026_06_X4_001_seed_purchase_lifecycle",
        inserted,
        skipped,
        "seeded purchase_lifecycle taxonomy (4 values)"
    );
    Ok(())
}

/// 构造示例 profile：销售域 DEFAULT 两维 + 追加 purchase_lifecycle（参与决策）。
/// draft 态：`is_active=false` + `current_version=false`，不自动生效。
pub(super) fn example_profile_with_lifecycle(workspace_id: &str) -> crate::models::DomainProfile {
    let mut profile = default_domain_profile(workspace_id);
    profile.profile_id = EXAMPLE_PROFILE_ID.to_string();
    profile.display_name = "销售 + 购买生命周期（示例草稿）".to_string();
    profile.description =
        "在销售域两维基础上追加「购买生命周期」参与决策维度的示例草稿；未激活，\
         运营在审核 UI 确认后 publish+activate 生效。"
            .to_string();
    profile.profile_dimensions.push(ProfileDimension {
        kind: G1_DIMENSION_KIND.to_string(),
        display_name: "购买生命周期".to_string(),
        participates_in_decision: true,
        description: "客户在购买旅程中的阶段：未购买/已购买/售后期/复购期。\
                      由对话推断，并以 G4 已核实持有事实为客观锚纠偏。"
            .to_string(),
    });
    // draft：不自动生效。引导/审核层 publish+activate 时再翻这两个标志。
    profile.is_active = false;
    profile.current_version = false;
    profile.seeded_by = Some("g1_migration".to_string());
    profile
}

async fn seed_example_profile(db: &Database, now: DateTime) -> AppResult<()> {
    let workspace_id = "default";
    let collection = db.domain_profiles();
    let filter = doc! {
        "workspace_id": workspace_id,
        "profile_id": EXAMPLE_PROFILE_ID,
    };
    let mut profile = example_profile_with_lifecycle(workspace_id);
    profile.created_at = now;
    profile.updated_at = now;
    let mut doc_to_set = mongodb::bson::to_document(&profile)?;
    doc_to_set.remove("_id");
    let result = collection
        .update_one(
            filter,
            doc! { "$setOnInsert": doc_to_set },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
    tracing::info!(
        migration_id = "2026_06_X4_001_seed_purchase_lifecycle",
        upserted = result.upserted_id.is_some(),
        "seeded example domain profile (draft, inactive)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxonomy_seed_covers_four_canonical_values() {
        let now = DateTime::now();
        let entries = purchase_lifecycle_seed_entries(now);
        let ids: Vec<&str> = entries.iter().map(|e| e.value.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![G1_NOT_PURCHASED, G1_PURCHASED, G1_AFTERCARE, G1_REPURCHASE]
        );
        for e in &entries {
            assert_eq!(e.scope, "global");
            assert_eq!(e.kind, G1_DIMENSION_KIND);
            assert_eq!(e.value.status, "active");
            assert!(e.value.priority_weight.is_none());
            assert!(!e.value.is_terminal);
        }
    }

    #[test]
    fn example_profile_is_draft_with_lifecycle_dimension() {
        let p = example_profile_with_lifecycle("default");
        assert_eq!(p.profile_id, EXAMPLE_PROFILE_ID);
        // draft：不自动生效（运行时仍回落 DEFAULT_PROFILE，零扰动）。
        assert!(!p.is_active);
        assert!(!p.current_version);
        // 三维：销售两维 + purchase_lifecycle。
        let kinds: Vec<&str> = p
            .profile_dimensions
            .iter()
            .map(|d| d.kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["customer_stage", "intent_level", G1_DIMENSION_KIND]);
        // G1 维度参与决策。
        let g1 = p
            .profile_dimensions
            .iter()
            .find(|d| d.kind == G1_DIMENSION_KIND)
            .expect("g1 dim present");
        assert!(g1.participates_in_decision);
    }

    #[test]
    fn taxonomy_entries_unique_by_scope_kind_id() {
        let now = DateTime::now();
        let entries = purchase_lifecycle_seed_entries(now);
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
