//! Corrective migration for review findings whose original migration markers
//! may already exist on upgraded databases.

use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};

use crate::db::Database;
use crate::error::AppResult;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    reconcile_prompt_currents(db).await?;
    super::m029_cleanup_contact_identity::run_step(db).await?;
    Ok(())
}

async fn reconcile_prompt_currents(db: &Database) -> AppResult<()> {
    let coll = db.raw().collection::<Document>("prompt_templates");
    let mut cursor = coll
        .aggregate(
            vec![
                doc! { "$match": { "status": { "$ne": "archived" } } },
                doc! { "$addFields": {
                    "_migration_active_priority": {
                        "$cond": [{ "$eq": ["$status", "active"] }, 1, 0]
                    }
                } },
                doc! { "$sort": {
                    "workspace_id": 1,
                    "prompt_key": 1,
                    "_migration_active_priority": -1,
                    "version": -1,
                    "_id": 1,
                } },
                doc! { "$group": {
                    "_id": { "workspace_id": "$workspace_id", "prompt_key": "$prompt_key" },
                    "target_id": { "$first": "$_id" },
                } },
            ],
            None,
        )
        .await?;

    while let Some(group) = cursor.try_next().await? {
        let scope = group.get_document("_id").ok();
        let workspace_id = scope.and_then(|d| d.get_str("workspace_id").ok());
        let prompt_key = scope.and_then(|d| d.get_str("prompt_key").ok());
        let target_id: Option<ObjectId> = group.get("target_id").and_then(Bson::as_object_id);
        let (Some(workspace_id), Some(prompt_key), Some(target_id)) =
            (workspace_id, prompt_key, target_id)
        else {
            continue;
        };
        // The single-current unique index may already exist on an upgraded
        // database. Demote the old pointer before promoting the winner so the
        // corrective migration can run under that index without E11000.
        coll.update_many(
            doc! {
                "workspace_id": workspace_id,
                "prompt_key": prompt_key,
                "_id": { "$ne": target_id },
            },
            doc! { "$set": { "current_version": false } },
            None,
        )
        .await?;
        coll.update_one(
            doc! { "_id": target_id },
            doc! { "$set": { "current_version": true } },
            None,
        )
        .await?;
    }
    Ok(())
}
