//! m028：A5 通用化——seed `conversation_mode` taxonomy 字典四值。
//!
//! 幂等（`$setOnInsert` upsert，不覆盖运营编辑）：seed 独立 kind `conversation_mode`
//! 四销售域默认值（scope=`global`）+ 中文 label。供 active-view 下发取值字典,前端
//! `labelFor` 把 canonical 英文翻译成中文(替代写死 switch)。非销售域(情感陪伴等)
//! 可经 `POST /api/admin/taxonomies` 增 intimate_companion 等本域模式。
//!
//! 与 conversation_modes: Vec<String>(profile 声明的本域启用模式列表)解耦:profile
//! 声明"本域用哪几种模式",本字典声明"每个模式 canonical→中文 label"。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::db::Database;
use crate::error::AppResult;
use crate::models::{TaxonomyEntry, TaxonomyValue};

pub(super) const CONVERSATION_MODE_KIND: &str = "conversation_mode";
pub(super) const CONVERSATION_MODE_CASUAL: &str = "casual_relationship";
pub(super) const CONVERSATION_MODE_VALUE_EXCHANGE: &str = "value_exchange";
pub(super) const CONVERSATION_MODE_CONSULTATIVE: &str = "consultative";
pub(super) const CONVERSATION_MODE_BOUNDARY_PROTECTION: &str = "boundary_protection";

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    seed_conversation_mode_taxonomy(db, now).await?;
    Ok(())
}

pub(super) fn conversation_mode_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let values: &[(&str, &str, &str, &[&str])] = &[
        (
            CONVERSATION_MODE_CASUAL,
            "寒暄关系",
            "轻寒暄、建立熟悉度的关系维护对话。",
            &["寒暄", "闲聊"],
        ),
        (
            CONVERSATION_MODE_VALUE_EXCHANGE,
            "价值互换",
            "围绕需求与价值匹配的信息交换。",
            &["价值交换", "互惠"],
        ),
        (
            CONVERSATION_MODE_CONSULTATIVE,
            "顾问咨询",
            "顾问式答疑/方案沟通,提供专业建议。",
            &["顾问", "咨询", "顾问式"],
        ),
        (
            CONVERSATION_MODE_BOUNDARY_PROTECTION,
            "边界保护",
            "客户表达压力/拒绝时的边界保护与降压。",
            &["边界", "降压"],
        ),
    ];
    values
        .iter()
        .map(|(id, display, desc, aliases)| TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: CONVERSATION_MODE_KIND.to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
                priority_weight: None,
                is_terminal: false,
                is_reactivation_target: false,
            },
            updated_at: now,
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: Some("conversation_mode_migration".to_string()),
        })
        .collect()
}

async fn seed_conversation_mode_taxonomy(db: &Database, now: DateTime) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;
    for entry in conversation_mode_seed_entries(now) {
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
        migration_id = "m028_seed_conversation_mode",
        inserted,
        skipped,
        "seeded conversation_mode taxonomy (4 values)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_mode_seed_covers_four_default_values() {
        let now = DateTime::now();
        let entries = conversation_mode_seed_entries(now);
        let ids: Vec<&str> = entries.iter().map(|e| e.value.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                CONVERSATION_MODE_CASUAL,
                CONVERSATION_MODE_VALUE_EXCHANGE,
                CONVERSATION_MODE_CONSULTATIVE,
                CONVERSATION_MODE_BOUNDARY_PROTECTION,
            ]
        );
        for e in &entries {
            assert_eq!(e.scope, "global");
            assert_eq!(e.kind, CONVERSATION_MODE_KIND);
            assert_eq!(e.value.status, "active");
            assert!(!e.value.display_name.is_empty());
        }
    }
}
