//! 2026_06_Y0_001：seed「销售 + 关系分化」示例 profile（draft, inactive）。
//!
//! 提供一个 per_relationship_operation_mode 已配三套范式的可激活 profile，让运营给
//! contact 标 relationship_type 后主动触达真分化。draft/inactive——运营审阅后手动
//! activate 才生效，不改零配置启动（DEFAULT 仍 active）。幂等 `$setOnInsert`。
//!
//! 红线：取值/标签一律 AI 中性词，不含 `人工`/`接管`/`takeover` 等禁词。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::agent::domain_profile::example_sales_with_relationships_profile;
use crate::db::Database;
use crate::error::AppResult;

const PROFILE_ID: &str = "sales_with_relationships";

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    let workspace_id = "default";
    let collection = db.domain_profiles();
    let filter = doc! { "workspace_id": workspace_id, "profile_id": PROFILE_ID };
    let mut profile = example_sales_with_relationships_profile(workspace_id);
    profile.created_at = now;
    profile.updated_at = now;
    // draft/inactive：运营手动 activate 才生效。
    profile.current_version = false;
    profile.is_active = false;
    profile.seeded_by = Some("system".to_string());
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
        migration_id = "2026_06_Y0_001_seed_sales_with_relationships",
        upserted = result.upserted_id.is_some(),
        "seeded sales+relationships domain profile (draft, inactive)"
    );
    Ok(())
}
