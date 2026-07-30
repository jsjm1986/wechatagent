//! 索引创建集合。
//!
//! 所有索引创建语句集中在 [`ensure_all`]，由 [`super::Database::ensure_indexes`]
//! 调用。运行时其它路径不应该再调用 `create_index`。

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    options::IndexOptions,
    IndexModel,
};
use std::collections::HashSet;

use super::Database;

fn llm_vision_active_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspaceId": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_llm_vision_active_workspace".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "isVisionActive": true })
                .build(),
        )
        .build()
}

fn domain_profile_version_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "profile_id": 1, "version": 1 })
        .options(
            IndexOptions::builder()
                .name("domain_profiles_ws_id_version_unique".to_string())
                .unique(true)
                .build(),
        )
        .build()
}

fn domain_profile_current_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "profile_id": 1 })
        .options(
            IndexOptions::builder()
                .name("domain_profiles_ws_id_current_unique".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "current_version": true })
                .build(),
        )
        .build()
}

fn domain_profile_active_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1 })
        .options(
            IndexOptions::builder()
                .name("domain_profiles_ws_active_unique".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "is_active": true })
                .build(),
        )
        .build()
}

fn operation_playbook_default_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "account_id": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_operation_playbook_default_per_account".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "is_default": true })
                .build(),
        )
        .build()
}

async fn validate_llm_vision_assignments(db: &Database) -> anyhow::Result<()> {
    let mut cursor = db
        .llm_provider_configs()
        .find(doc! { "isVisionActive": true }, None)
        .await?;
    let mut workspaces = HashSet::new();
    while let Some(provider) = cursor.try_next().await? {
        if !provider.supports_vision {
            return Err(anyhow::anyhow!(
                "LLM provider {} in workspace {} is assigned for vision without supportsVision",
                provider.provider_id,
                provider.workspace_id
            ));
        }
        if !workspaces.insert(provider.workspace_id.clone()) {
            return Err(anyhow::anyhow!(
                "workspace {} has multiple active vision providers",
                provider.workspace_id
            ));
        }
    }
    Ok(())
}

fn relationship_suggestion_pending_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "contact_id": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_relationship_pending_ws_contact".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "status": "pending" })
                .build(),
        )
        .build()
}

fn lesson_identity_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "lesson_id": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_lessons_learned_ws_lesson".to_string())
                .unique(true)
                .build(),
        )
        .build()
}

fn lesson_promotion_chunk_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "provenance.source_doc_id": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_kchunks_lesson_promotion_source".to_string())
                .unique(true)
                .partial_filter_expression(doc! {
                    "provenance.source": "lesson_promotion",
                    "provenance.source_doc_id": { "$type": "string" },
                })
                .build(),
        )
        .build()
}

fn gap_signals_pending_dedup_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "dedup_key": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_gap_signals_pending_ws_dedup".to_string())
                .unique(true)
                .partial_filter_expression(doc! {
                    "status": "pending",
                    "dedup_key": { "$type": "string" },
                })
                .build(),
        )
        .build()
}

fn outbox_delivery_finalize_pending_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "status": 1,
            "delivery_finalize_pending": 1,
            "updated_at": 1,
            "_id": 1,
        })
        .options(
            IndexOptions::builder()
                .name("outbox_delivery_finalize_pending_idx".to_string())
                .partial_filter_expression(doc! {
                    "status": "sent",
                    "delivery_finalize_pending": true,
                })
                .build(),
        )
        .build()
}

fn outbox_idempotency_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "idempotency_key": 1,
        })
        .options(
            IndexOptions::builder()
                .name("uniq_outbox_ws_account_idempotency".to_string())
                .unique(true)
                .build(),
        )
        .build()
}

fn management_tool_intent_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "intent_key": 1,
        })
        .options(
            IndexOptions::builder()
                .name("uniq_management_tool_intent".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "intent_key": { "$type": "string" } })
                .build(),
        )
        .build()
}

fn campaign_dispatch_recovery_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspaceId": 1,
            "accountId": 1,
            "status": 1,
            "updatedAt": 1,
        })
        .options(
            IndexOptions::builder()
                .name("campaign_dispatch_recovery_idx".to_string())
                .partial_filter_expression(doc! { "status": "dispatching" })
                .build(),
        )
        .build()
}

fn agent_run_log_outbox_enqueuing_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "status": 1, "created_at": 1, "_id": 1 })
        .options(
            IndexOptions::builder()
                .name("agent_run_log_outbox_enqueuing_idx".to_string())
                .partial_filter_expression(doc! { "status": "outbox_enqueuing" })
                .build(),
        )
        .build()
}

fn outcome_aggregation_task_dedupe_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "kind": 1,
            "account_id": 1,
            "content": 1,
        })
        .options(
            IndexOptions::builder()
                .unique(true)
                .partial_filter_expression(doc! { "kind": "outcome_aggregation" })
                .name("uniq_outcome_aggregation_ws_kind_account_content".to_string())
                .build(),
        )
        .build()
}

fn proactive_daily_quota_ttl_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "expires_at": 1 })
        .options(
            IndexOptions::builder()
                .name("proactive_daily_quotas_expires_ttl".to_string())
                .expire_after(std::time::Duration::from_secs(0))
                .build(),
        )
        .build()
}

fn taxonomy_active_identity_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "scope": 1,
            "kind": 1,
            "value.identityClaims": 1,
        })
        .options(
            IndexOptions::builder()
                .name("uniq_sys_tax_ws_scope_kind_active_identity".to_string())
                .unique(true)
                .partial_filter_expression(doc! {
                    "current_version": true,
                    "value.status": "active",
                })
                .build(),
        )
        .build()
}

fn memory_active_task_key_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "contact_wxid": 1,
            "active_task_key": 1,
        })
        .options(
            IndexOptions::builder()
                .unique(true)
                .partial_filter_expression(doc! {
                    "active_task_key": { "$type": "string" },
                })
                .name("uniq_memory_active_task_key".to_string())
                .build(),
        )
        .build()
}

fn inbound_handoff_pending_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "handoff_status": 1, "created_at": 1, "_id": 1 })
        .options(
            IndexOptions::builder()
                .name("inbound_handoff_pending_idx".to_string())
                .partial_filter_expression(doc! {
                    "direction": "inbound",
                    "handoff_status": "pending",
                })
                .build(),
        )
        .build()
}

fn principal_relay_pending_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "relay_state": 1, "resolved_at": 1, "_id": 1 })
        .options(
            IndexOptions::builder()
                .name("principal_relay_pending_idx".to_string())
                .partial_filter_expression(doc! {
                    "status": "resolved",
                    "relay_state": "pending",
                })
                .build(),
        )
        .build()
}

fn principal_card_delivery_reconcile_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "status": 1,
            "protocol.delivery_state": 1,
            "_id": 1,
        })
        .options(
            IndexOptions::builder()
                .name("principal_card_delivery_reconcile_idx".to_string())
                .partial_filter_expression(doc! {
                    "status": "pending",
                    "protocol.delivery_state": { "$in": ["pending_enqueue", "queued"] },
                })
                .build(),
        )
        .build()
}

fn principal_card_timeout_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "status": 1,
            "protocol.delivery_state": 1,
            "last_pushed_at_ms": 1,
            "_id": 1,
        })
        .options(
            IndexOptions::builder()
                .name("principal_card_timeout_idx".to_string())
                .partial_filter_expression(doc! {
                    "status": "pending",
                    "protocol.delivery_state": "sent",
                    "protocol.policy.timeoutHours": { "$type": "number" },
                    "last_pushed_at_ms": { "$type": "number" },
                })
                .build(),
        )
        .build()
}

fn principal_escalation_pending_dedupe_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "contact_wxid": 1,
            "category": 1,
        })
        .options(
            IndexOptions::builder()
                .unique(true)
                .partial_filter_expression(doc! { "status": "pending" })
                .name("uniq_principal_escalation_pending_ws_account_contact_category".to_string())
                .build(),
        )
        .build()
}

fn send_ledger_outbox_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "outbox_id": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_send_ledger_outbox_id".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "outbox_id": { "$type": "objectId" } })
                .build(),
        )
        .build()
}

fn agent_soul_version_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "agent_kind": 1, "version": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_agent_soul_ws_kind_version".to_string())
                .unique(true)
                .build(),
        )
        .build()
}

fn agent_soul_published_unique_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "agent_kind": 1 })
        .options(
            IndexOptions::builder()
                .name("uniq_agent_soul_published_ws_kind".to_string())
                .unique(true)
                .partial_filter_expression(doc! { "status": "published" })
                .build(),
        )
        .build()
}

fn send_ledger_contact_history_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "contact_wxid": 1,
            "sent_at": -1,
        })
        .build()
}

fn send_ledger_target_stats_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "send_kind": 1,
            "target_id": 1,
        })
        .build()
}

pub(crate) fn behavior_signal_identity_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "account_id": 1, "dedupe_key": 1 })
        .options(
            IndexOptions::builder()
                .unique(true)
                .name("uniq_behavior_signals_ws_account_dedupe_key".to_string())
                .partial_filter_expression(doc! {
                    "dedupe_key": { "$type": "string" }
                })
                .build(),
        )
        .build()
}

pub(crate) fn behavior_signal_timeline_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! {
            "workspace_id": 1,
            "account_id": 1,
            "contact_wxid": 1,
            "observed_at": -1,
        })
        .build()
}

pub(crate) fn chunk_revision_identity_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "chunk_id": 1, "revision_id": -1 })
        .options(
            IndexOptions::builder()
                .name("chunk_revisions_ws_chunk_rev_idx".to_string())
                .build(),
        )
        .build()
}

pub(crate) fn chunk_revision_timeline_index() -> IndexModel {
    IndexModel::builder()
        .keys(doc! { "workspace_id": 1, "created_at": -1 })
        .options(
            IndexOptions::builder()
                .name("chunk_revisions_ws_created_at_idx".to_string())
                .build(),
        )
        .build()
}

fn index_option_semantics(options: Option<&IndexOptions>) -> anyhow::Result<Document> {
    let mut semantics = mongodb::bson::to_document(&options.cloned().unwrap_or_default())?;
    // MongoDB returns the server-selected index version, and historical
    // deployments may have retained a different name for the same index.
    // Neither changes query or constraint semantics. Every other serialized
    // option (unique, sparse, partial filter, TTL, collation, hidden, storage
    // engine, wildcard projection, etc.) remains part of the equality check.
    semantics.remove("name");
    semantics.remove("v");
    Ok(semantics)
}

fn equivalent_index_options(
    existing: Option<&IndexOptions>,
    desired: Option<&IndexOptions>,
) -> anyhow::Result<bool> {
    Ok(index_option_semantics(existing)? == index_option_semantics(desired)?)
}

fn is_namespace_not_found(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Command(command) if command.code == 26
    )
}

async fn ensure_index_or_equivalent_name(
    collection: mongodb::Collection<Document>,
    desired: IndexModel,
) -> anyhow::Result<()> {
    let desired_name = desired
        .options
        .as_ref()
        .and_then(|options| options.name.as_deref())
        .unwrap_or("<generated>");
    let desired_semantics = index_option_semantics(desired.options.as_ref())?;
    let mut indexes = match collection.list_indexes(None).await {
        Ok(indexes) => indexes,
        Err(error) if is_namespace_not_found(&error) => {
            // create_index historically created an absent collection as part
            // of fresh-database startup. Preserve that behavior while only
            // treating NamespaceNotFound as proof that no conflicting index
            // can exist; every other list failure remains fatal.
            collection.create_index(desired, None).await?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let mut equivalent_names = Vec::new();

    while let Some(existing) = indexes.try_next().await? {
        if existing.keys != desired.keys {
            continue;
        }
        let existing_name = existing
            .options
            .as_ref()
            .and_then(|options| options.name.clone());
        let options_equivalent =
            equivalent_index_options(existing.options.as_ref(), desired.options.as_ref())?;
        let existing_semantics = index_option_semantics(existing.options.as_ref())?;
        if !options_equivalent {
            anyhow::bail!(
                "collection {} has index {:?} with keys {:?} but incompatible options {:?}; expected index {} options {:?}",
                collection.name(),
                existing_name,
                desired.keys,
                existing_semantics,
                desired_name,
                desired_semantics,
            );
        }
        equivalent_names.push(existing_name);
    }

    if !equivalent_names.is_empty() {
        tracing::info!(
            collection = collection.name(),
            desired_index = desired_name,
            existing_indexes = ?equivalent_names,
            "reusing semantically equivalent index with historical name"
        );
        return Ok(());
    }

    collection.create_index(desired, None).await?;
    Ok(())
}

async fn retire_indexes_with_keys(
    collection: mongodb::Collection<Document>,
    legacy_keys: &[Document],
) -> anyhow::Result<()> {
    let mut cursor = collection.list_indexes(None).await?;
    let mut legacy_names = Vec::new();
    while let Some(index) = cursor.try_next().await? {
        if legacy_keys.iter().any(|keys| index.keys == *keys) {
            if let Some(name) = index.options.and_then(|options| options.name) {
                legacy_names.push(name);
            }
        }
    }
    for name in legacy_names {
        collection.drop_index(name, None).await?;
    }
    Ok(())
}

async fn retire_legacy_behavior_signal_indexes(db: &Database) -> anyhow::Result<()> {
    retire_indexes_with_keys(
        db.raw().collection::<Document>("behavior_signals"),
        &[
            doc! { "workspace_id": 1, "dedupe_key": 1 },
            doc! { "workspace_id": 1, "contact_wxid": 1, "observed_at": -1 },
        ],
    )
    .await
}

async fn retire_legacy_chunk_revision_indexes(db: &Database) -> anyhow::Result<()> {
    retire_indexes_with_keys(
        db.raw().collection::<Document>("chunk_revisions"),
        &[
            doc! { "chunk_id": 1, "revision_id": -1 },
            doc! { "created_at": -1 },
        ],
    )
    .await
}

pub(super) async fn ensure_all(db: &Database) -> anyhow::Result<()> {
    db.accounts()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // Replace the historical non-unique app_id index. If stored duplicates
    // exist, creating the unique index fails startup explicitly so operators
    // can reconcile ownership instead of routing webhooks nondeterministically.
    let _ = db.accounts().drop_index("app_id_1", None).await;
    db.accounts()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "app_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("uniq_wechat_accounts_app_id".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "app_id": { "$type": "string" } })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.contacts()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "wxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // 活动定向推送：按购买产品反查客户。真实 BSON 路径是混合大小写——
    // outcome_events(snake_case，Contact 无 rename_all) + productRef.productId
    // (camelCase，OutcomeEvent/OutcomeProductRef 带 rename_all=camelCase)。
    // outcome_events 是数组 → multikey 索引；$elemMatch 按产品反查命中此索引前缀。
    db.contacts()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "outcome_events.productRef.productId": 1
                })
                .build(),
            None,
        )
        .await?;
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    // 联系人列表批量取每位联系人最新入站：match workspace/account/direction/contact，
    // 再按 contact+created_at 分组取首条。direction 放在 contact 前以匹配等值谓词。
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "direction": 1,
                    "contact_wxid": 1,
                    "created_at": -1,
                    "_id": -1,
                })
                .build(),
            None,
        )
        .await?;
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "message_id": 1 })
                .options(IndexOptions::builder().sparse(true).unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "dedupe_key": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "dedupe_key": { "$type": "string" } })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // SR-177 crash recovery scans only inbound facts whose durable task
    // handoff has not yet been materialized.
    db.messages()
        .create_index(inbound_handoff_pending_index(), None)
        .await?;
    db.tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "run_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "contact_wxid": 1,
                    "kind": 1,
                    "status": 1
                })
                .build(),
            None,
        )
        .await?;
    // P1-1：outcome_aggregation 任务幂等去重靠
    // (workspace_id, kind, account_id, content) 唯一约束。
    // tasks.rs::ensure_today_outcome_aggregation_tasks 之前用 find_one 后 insert_one
    // 存在 TOCTOU；改原子 insert + 11000 dup-key 视作"已存在"前必须有此索引。
    // partial filter 限定 kind 否则会误伤其他 kind 同 content 的合法重复（如
    // follow_up 同一 contact 不同回合的内容）。
    db.tasks()
        .create_index(outcome_aggregation_task_dedupe_index(), None)
        .await?;
    // Memory consolidation uses a durable lease key rather than find-then-insert. Terminal
    // transitions remove active_task_key atomically, so historical rows remain outside the
    // partial unique index while every newly scheduled task is single-flight per contact.
    db.tasks()
        .create_index(memory_active_task_key_index(), None)
        .await?;
    // Proactive daily buckets are short-lived concurrency controls, not the
    // audit source of truth. Task/event rows retain the durable intent while
    // this TTL prevents one small document per scope/day from growing forever.
    db.raw()
        .collection::<Document>("proactive_daily_quotas")
        .create_index(proactive_daily_quota_ttl_index(), None)
        .await?;
    // 异步导入 job：前端按 workspace 跨会话发现进行中 job。
    db.import_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    // import_worker 认领 pending + 孤儿 running（claimed_at 过期）重认领。
    db.import_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "claimed_at": 1 })
                .build(),
            None,
        )
        .await?;
    // 终态 job 24h 清扫（设计要求，防 result 无界堆积）。expireAfterSeconds=0：
    // Mongo 在 `expires_at < now()` 时删。worker 落 completed/failed 时置
    // `expires_at = now + 24h`；pending/running 不设该字段 → TTL 忽略缺失字段，
    // 进行中 job 绝不被误删（与 knowledge_operator_memory 的 expires_at TTL 同构）。
    db.import_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("import_jobs_expires_ttl".to_string())
                        .expire_after(std::time::Duration::from_secs(0))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.events()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "contact_wxid": 1,
                    "created_at": -1
                })
                .build(),
            None,
        )
        .await?;
    // P1-2：可选事件去重锚点。携带 `dedupe_key` 的事件按 (workspace_id, dedupe_key)
    // 严格唯一；不携带的事件落 partial filter 之外，正常重复写。
    db.events()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "dedupe_key": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("uniq_events_workspace_dedupe_key".to_string())
                        .partial_filter_expression(doc! {
                            "dedupe_key": { "$type": "string" }
                        })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // 自学习采集管道 S1–S5：behavior_signals append-only 事件日志。
    //   - `(workspace_id, account_id, dedupe_key)` partial unique：账号内幂等键，同一观察重复采集
    //     只落一次。partialFilterExpression 用 `$type: "string"`（等价 $exists 但
    //     更严）——绝不能用 `$in`，会触发 Error 67 让 ensure_indexes panic。
    //   - `(workspace_id, contact_wxid, observed_at desc)`：按联系人取近期信号。
    db.behavior_signals()
        .create_index(behavior_signal_identity_index(), None)
        .await?;
    db.behavior_signals()
        .create_index(behavior_signal_timeline_index(), None)
        .await?;
    // Both final indexes now exist. Only after that point may a previously
    // applied m039 deployment retire its historical workspace-only indexes.
    // Matching exact keys avoids deleting unrelated operator-created indexes.
    retire_legacy_behavior_signal_indexes(db).await?;
    // P3 采集健康度：behavior_signal_metrics 每日每 workspace 三态计数聚合。
    //   `_id="{workspace_id}:{date}"` 已天然唯一（$inc upsert 幂等），无需额外 unique；
    //   仅加 `(workspace_id, date desc)` 供 REST 端点按时间倒序拉近期健康度。
    db.behavior_signal_metrics()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "date": -1 })
                .build(),
            None,
        )
        .await?;
    db.content_assets()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "kind": 1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    // 销售素材选材查询：按 workspace 过滤可发送(sendable)且已审核(review_status)的素材。
    db.content_assets()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "sendable": 1, "review_status": 1 })
                .build(),
            None,
        )
        .await?;
    // 文件去重：按 file_sha256 命中已上传素材，避免重复上传/重传 MCP media_id。
    db.content_assets()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "file_sha256": 1 })
                .build(),
            None,
        )
        .await?;
    // 专属顾问名片引荐选材：按 workspace/account 过滤已启用(enabled)且已审核(review_status)的名片。
    db.referral_cards()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "enabled": 1,
                    "review_status": 1,
                })
                .build(),
            None,
        )
        .await?;
    // 主动发送台账：单客户发送历史（按时间倒序）。
    db.agent_send_ledger()
        .create_index(send_ledger_contact_history_index(), None)
        .await?;
    // 主动发送台账：素材/名片维度聚合。
    db.agent_send_ledger()
        .create_index(send_ledger_target_stats_index(), None)
        .await?;
    // 每个已确认送达的 Outbox 事实最多产生一条台账。历史无 outbox_id 行不入约束。
    db.agent_send_ledger()
        .create_index(send_ledger_outbox_unique_index(), None)
        .await?;
    // 主动发送台账：回扫服务索引。匹配 scan 查询形状
    // （filter { outcome_evaluated_at: { $exists: false } } + sort { sent_at: 1 }，
    // 全局扫不带 workspace_id），前缀 outcome_evaluated_at 命中过滤、sent_at 命中排序。
    db.agent_send_ledger()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "outcome_evaluated_at": 1, "sent_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.agent_souls()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "agent_kind": 1, "status": 1, "version": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_souls()
        .create_index(agent_soul_version_unique_index(), None)
        .await?;
    db.agent_souls()
        .create_index(agent_soul_published_unique_index(), None)
        .await?;
    db.operation_playbooks()
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! { "workspace_id": 1, "account_id": 1, "is_default": 1, "updated_at": -1 },
                )
                .build(),
            None,
        )
        .await?;
    db.operation_playbooks()
        .create_index(operation_playbook_default_unique_index(), None)
        .await?;
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1, "status": 1, "version": -1 })
                .build(),
            None,
        )
        .await?;
    // Phase E5-T1：operation_domain_configs / operation_state_policies /
    //   system_taxonomies 三表的唯一性索引统一由 `ensure_ops_versioned_indexes`
    //   负责——(workspace_id, domain[, state_key/value.id], version) 4-tuple unique
    //   + (..., current_version=true) 部分索引。这里不再单独建旧的 2-key/3-key
    //   unique:那两处 create_index 会被 ensure_ops_versioned_indexes 立即 drop
    //   掉,且在多版本数据(admin publish 攒下同 (ws,domain[,state_key]) 多 version
    //   行)下建旧 unique 会 E11000 → ensure_indexes 返 Err → 启动崩溃(H8 boot-brick)。
    ensure_ops_versioned_indexes(db).await?;
    db.operating_memories()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_documents()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "domain": 1, "status": 1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "domain": 1, "status": 1, "priority": -1, "updated_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(lesson_promotion_chunk_unique_index(), None)
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "document_id": 1, "item_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    db.knowledge_usage_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    // 回路①（gap_signals::refresh_usage_stats_and_confidence）每 600s 把该 workspace
    // 30d（gap_signals.rs:813 的 30*24h 窗口）全部 usage log try_collect 进内存。该集合
    // 是每次知识命中/拦截都 append 的高写入诊断日志，无 TTL 会从根上无界增长 → 最终
    // 拖垮 feedback_worker 内存。TTL=35d 略大于回路①的 30d 滑窗，只清窗口外历史、
    // 不影响窗口内统计；与 llm_call_logs/agent_run_logs 等诊断日志的 TTL 策略同构。
    db.knowledge_usage_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(35 * 24 * 60 * 60))
                        .name("ttl_knowledge_usage_logs_created_at".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_turns()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "session_id": 1, "turn_index": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchat_turns_session_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_turns()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("kchat_turns_recent_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.decision_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.decision_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "contact_wxid": 1,
                    "status": 1,
                    "outcome_status": 1
                })
                // 注意：不能用 partialFilterExpression { outcome_status: { $in: [...] } }——
                // MongoDB partial index 只接受 $eq/$gt/$gte/$lt/$lte/$exists/$type/$and 及
                // 单值相等，$in/$or 会被拒（Error 67 CannotCreateIndex），且会让整个
                // ensure_indexes panic。reaction claim 查询走前缀
                // (workspace_id, account_id, contact_wxid, status) + outcome_status 等值，
                // 全键复合索引已能覆盖；放弃"只索引活跃 review"的体积优化以换取合法性。
                .build(),
            None,
        )
        .await?;
    // H11-linkage：回路① 成交追认 / outcome join 按 run_id 批量拉 decision_reviews
    // （gap_signals::refresh_usage_stats_and_confidence 的 outcome_by_run）。无此索引
    // 会全表扫高写入量的 decision_reviews。非 unique：不假设一 run 一 review。
    db.decision_reviews()
        .create_index(
            IndexModel::builder().keys(doc! { "run_id": 1 }).build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "run_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(agent_run_log_outbox_enqueuing_index(), None)
        .await?;
    // ── agent-autonomy-loop W0 (Task 1.2) / W6 (Task 7.1) ──
    //
    // R0.8 / R9.5 监控查询索引。BSON key 使用 snake_case，与 `AgentRunLog`
    // 字段未加 `#[serde(rename = ...)]` 的 snake_case 约定一致。
    //
    // W6 修订（Task 7.1）：W0 设计稿规划了 `started_at`，但 W1 落地后
    // `AgentRunLog` 顶层只写 `created_at`（`planner.started_at` 是嵌套 Document
    // 字段，不能作为顶层索引 key 使用）。`outcomes_autonomy::build_horizon_filter`
    // 实际过滤的就是 `created_at`，因此索引在此对齐到 `created_at`，避免
    // 监控聚合走 collection scan。
    //
    // 已部署集群可能仍残留 W0 创建的同形 `started_at` 索引（不会命中任何
    // 文档，是空索引）；它不阻塞写入，可在维护窗口手工 dropIndex。
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "lifecycle": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "final_review_status": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.agent_run_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "autonomy_mode": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.llm_call_logs()
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! { "workspace_id": 1, "account_id": 1, "prompt_key": 1, "created_at": -1 },
                )
                .build(),
            None,
        )
        .await?;
    db.llm_call_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "run_id": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    // #154 / SR-025：crash-recovery post-hoc 核对在热路径上按
    // (workspace_id, account_id, tool_name, created_at>=lb) 查询。复合索引先按租户、
    // 账号、工具和时间窗收敛，request.recipient/content/error
    // 作残余过滤。
    db.mcp_logs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "tool_name": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.memory_candidates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "contact_wxid": 1, "status": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.user_operation_guide_previews()
        .create_index(
            IndexModel::builder()
                .keys(
                    doc! { "workspace_id": 1, "account_id": 1, "contact_id": 1, "created_at": -1 },
                )
                .build(),
            None,
        )
        .await?;
    db.management_messages()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "created_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.command_runs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.tool_calls()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "command_run_id": 1, "created_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.tool_calls()
        .create_index(management_tool_intent_unique_index(), None)
        .await?;
    // S-19 / Task 17：outcome metrics TTL 索引（默认 90 天）。
    let ttl_days: u64 = std::env::var("OUTCOME_METRICS_TTL_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    db.outcome_metrics()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": 1 })
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::from_secs(ttl_days * 24 * 60 * 60))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.outcome_metrics()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "horizon": 1, "date": -1 })
                .build(),
            None,
        )
        .await?;
    // #154：高写入量日志集合 TTL（默认 30 天）。llm_call_logs / agent_run_logs /
    // mcp_call_logs 是每条入站/每次决策都追加的诊断日志，无上限会无限增长并拖慢
    // 上面那些 (workspace_id, ..., created_at) 复合查询。TTL 只清诊断日志，不动
    // 业务事实表（contacts / conversation_messages / agent_send_outbox 等）。
    // 0 表示禁用 TTL（保留全部历史）。
    let log_ttl_days: u64 = std::env::var("DIAGNOSTIC_LOG_TTL_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    if log_ttl_days > 0 {
        let log_ttl = std::time::Duration::from_secs(log_ttl_days * 24 * 60 * 60);
        db.llm_call_logs()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(IndexOptions::builder().expire_after(log_ttl).build())
                    .build(),
                None,
            )
            .await?;
        db.agent_run_logs()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(IndexOptions::builder().expire_after(log_ttl).build())
                    .build(),
                None,
            )
            .await?;
        db.mcp_logs()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(IndexOptions::builder().expire_after(log_ttl).build())
                    .build(),
                None,
            )
            .await?;
    }
    // S-18 / Task 18：evaluation_scenarios 唯一索引（scenario_id 在 workspace 内唯一）。
    db.evaluation_scenarios()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "scenario_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    // ── agent-autonomy-loop W0 (Task 1.2) ──
    //
    // 三个新增 collection 的索引集中在专属 helper：保持 ensure_all 主流程精简，
    // 同时方便 W3 / W4 在落地业务字段时按需新增索引（如 outbox 的 ttl / 字典的
    // alias 命中索引）。
    ensure_agent_send_outbox_indexes(db).await?;
    ensure_system_taxonomies_indexes(db).await?;
    ensure_taxonomy_candidates_indexes(db).await?;
    ensure_relationship_type_suggestions_indexes(db).await?;
    ensure_suspected_deal_signals_indexes(db).await?;
    // ── agent-self-evolution W0 (Task 1.2) ──
    ensure_evolution_indexes(db).await?;
    // LLM 服务商配置：(workspace_id, provider_id) 唯一；is_active 部分索引便于
    // 启动时快速取出当前 active 记录。
    ensure_llm_provider_indexes(db).await?;
    // objective-purchase-facts G2：商品库索引。
    ensure_products_indexes(db).await?;
    ensure_campaigns_indexes(db).await?;
    // 通讯录快照：每 workspace+account 一条，覆盖写，故 (workspace_id, account_id) 唯一。
    db.roster_snapshots()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_dispatch_recovery_index_is_partial() {
        let index = campaign_dispatch_recovery_index();
        assert_eq!(
            index.keys,
            doc! { "workspaceId": 1, "accountId": 1, "status": 1, "updatedAt": 1 }
        );
        let options = index.options.expect("campaign recovery index options");
        assert_eq!(
            options.name.as_deref(),
            Some("campaign_dispatch_recovery_idx")
        );
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "status": "dispatching" })
        );
    }

    #[test]
    fn management_tool_intent_index_is_scoped_partial_unique() {
        let index = management_tool_intent_unique_index();
        assert_eq!(
            index.keys,
            doc! { "workspace_id": 1, "account_id": 1, "intent_key": 1 }
        );
        let options = index.options.expect("management intent index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(options.name.as_deref(), Some("uniq_management_tool_intent"));
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "intent_key": { "$type": "string" } })
        );
    }

    #[test]
    fn proactive_daily_quota_index_expires_at_absolute_time() {
        let index = proactive_daily_quota_ttl_index();
        assert_eq!(index.keys, doc! { "expires_at": 1 });
        let options = index.options.expect("proactive quota ttl options");
        assert_eq!(
            options.name.as_deref(),
            Some("proactive_daily_quotas_expires_ttl")
        );
        assert_eq!(
            options.expire_after,
            Some(std::time::Duration::from_secs(0))
        );
    }

    #[test]
    fn chunk_revision_scoped_indexes_do_not_reuse_legacy_names() {
        let identity = chunk_revision_identity_index();
        assert_eq!(
            identity.keys,
            doc! { "workspace_id": 1, "chunk_id": 1, "revision_id": -1 }
        );
        assert_eq!(
            identity
                .options
                .expect("revision identity options")
                .name
                .as_deref(),
            Some("chunk_revisions_ws_chunk_rev_idx")
        );

        let timeline = chunk_revision_timeline_index();
        assert_eq!(timeline.keys, doc! { "workspace_id": 1, "created_at": -1 });
        assert_eq!(
            timeline
                .options
                .expect("revision timeline options")
                .name
                .as_deref(),
            Some("chunk_revisions_ws_created_at_idx")
        );
    }

    #[test]
    fn equivalent_index_options_ignore_only_name_and_server_version() {
        let desired = IndexOptions::builder().name("new_name".to_string()).build();
        let historical: IndexOptions = mongodb::bson::from_document(doc! {
            "name": "old_name",
            "v": 2,
        })
        .expect("historical index options");
        assert!(equivalent_index_options(Some(&historical), Some(&desired)).unwrap());

        for incompatible in [
            IndexOptions::builder()
                .name("old_name".to_string())
                .unique(true)
                .build(),
            IndexOptions::builder()
                .name("old_name".to_string())
                .partial_filter_expression(doc! { "workspace_id": "default" })
                .build(),
            IndexOptions::builder()
                .name("old_name".to_string())
                .hidden(true)
                .build(),
        ] {
            assert!(!equivalent_index_options(Some(&incompatible), Some(&desired)).unwrap());
        }
    }

    #[test]
    fn relationship_suggestion_index_is_pending_partial_unique() {
        let index = relationship_suggestion_pending_unique_index();
        assert_eq!(index.keys, doc! { "workspace_id": 1, "contact_id": 1 });
        let options = index
            .options
            .expect("relationship suggestion index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.name.as_deref(),
            Some("uniq_relationship_pending_ws_contact")
        );
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "status": "pending" })
        );
    }

    #[test]
    fn lesson_promotion_indexes_lock_lesson_and_chunk_identity() {
        let lesson = lesson_identity_unique_index();
        assert_eq!(lesson.keys, doc! { "workspace_id": 1, "lesson_id": 1 });
        let lesson_options = lesson.options.expect("lesson identity options");
        assert_eq!(lesson_options.unique, Some(true));
        assert_eq!(
            lesson_options.name.as_deref(),
            Some("uniq_lessons_learned_ws_lesson")
        );

        let chunk = lesson_promotion_chunk_unique_index();
        assert_eq!(
            chunk.keys,
            doc! { "workspace_id": 1, "provenance.source_doc_id": 1 }
        );
        let chunk_options = chunk.options.expect("lesson chunk identity options");
        assert_eq!(chunk_options.unique, Some(true));
        assert_eq!(
            chunk_options.partial_filter_expression,
            Some(doc! {
                "provenance.source": "lesson_promotion",
                "provenance.source_doc_id": { "$type": "string" },
            })
        );
    }

    #[test]
    fn llm_vision_active_index_is_workspace_partial_unique() {
        let index = llm_vision_active_unique_index();
        assert_eq!(index.keys, doc! { "workspaceId": 1 });
        let options = index.options.expect("vision active index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.name.as_deref(),
            Some("uniq_llm_vision_active_workspace")
        );
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "isVisionActive": true })
        );
    }

    #[test]
    fn domain_profile_release_indexes_lock_version_current_and_active() {
        let version = domain_profile_version_unique_index();
        assert_eq!(
            version.keys,
            doc! { "workspace_id": 1, "profile_id": 1, "version": 1 }
        );
        assert_eq!(version.options.as_ref().and_then(|o| o.unique), Some(true));

        let current = domain_profile_current_unique_index();
        assert_eq!(current.keys, doc! { "workspace_id": 1, "profile_id": 1 });
        let current_options = current.options.expect("current index options");
        assert_eq!(current_options.unique, Some(true));
        assert_eq!(
            current_options.partial_filter_expression,
            Some(doc! { "current_version": true })
        );

        let active = domain_profile_active_unique_index();
        assert_eq!(active.keys, doc! { "workspace_id": 1 });
        let active_options = active.options.expect("active index options");
        assert_eq!(active_options.unique, Some(true));
        assert_eq!(
            active_options.partial_filter_expression,
            Some(doc! { "is_active": true })
        );
    }

    #[test]
    fn playbook_default_index_is_account_scoped_partial_unique() {
        let index = operation_playbook_default_unique_index();
        assert_eq!(index.keys, doc! { "workspace_id": 1, "account_id": 1 });
        let options = index.options.expect("playbook default index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.name.as_deref(),
            Some("uniq_operation_playbook_default_per_account")
        );
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "is_default": true })
        );
    }

    #[test]
    fn gap_signal_dedup_index_is_pending_partial_unique() {
        let index = gap_signals_pending_dedup_index();
        assert_eq!(index.keys, doc! { "workspace_id": 1, "dedup_key": 1 });

        let options = index.options.expect("dedup index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! {
                "status": "pending",
                "dedup_key": { "$type": "string" },
            })
        );
    }

    #[test]
    fn outbox_delivery_finalize_index_matches_reconcile_scan() {
        let index = outbox_delivery_finalize_pending_index();
        assert_eq!(
            index.keys,
            doc! {
                "status": 1,
                "delivery_finalize_pending": 1,
                "updated_at": 1,
                "_id": 1,
            }
        );
        let options = index.options.expect("delivery finalize index options");
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! {
                "status": "sent",
                "delivery_finalize_pending": true,
            })
        );
    }

    #[test]
    fn agent_run_log_outbox_enqueuing_index_matches_reconcile_scan() {
        let index = agent_run_log_outbox_enqueuing_index();
        assert_eq!(index.keys, doc! { "status": 1, "created_at": 1, "_id": 1 });
        let options = index.options.expect("stale enqueue index options");
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "status": "outbox_enqueuing" })
        );
    }

    #[test]
    fn outcome_task_dedupe_index_is_workspace_scoped() {
        let index = outcome_aggregation_task_dedupe_index();
        assert_eq!(
            index.keys,
            doc! {
                "workspace_id": 1,
                "kind": 1,
                "account_id": 1,
                "content": 1,
            }
        );
        let options = index.options.expect("outcome task index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "kind": "outcome_aggregation" })
        );
    }

    #[test]
    fn taxonomy_identity_index_is_active_current_partial_unique() {
        let index = taxonomy_active_identity_unique_index();
        assert_eq!(
            index.keys,
            doc! {
                "workspace_id": 1,
                "scope": 1,
                "kind": 1,
                "value.identityClaims": 1,
            }
        );
        let options = index.options.expect("taxonomy identity index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.name.as_deref(),
            Some("uniq_sys_tax_ws_scope_kind_active_identity")
        );
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! {
                "current_version": true,
                "value.status": "active",
            })
        );
    }

    #[test]
    fn memory_active_task_index_is_tenant_contact_scoped() {
        let index = memory_active_task_key_index();
        assert_eq!(
            index.keys,
            doc! {
                "workspace_id": 1,
                "account_id": 1,
                "contact_wxid": 1,
                "active_task_key": 1,
            }
        );
        let options = index.options.expect("memory task index options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "active_task_key": { "$type": "string" } })
        );
    }

    #[test]
    fn inbound_handoff_pending_index_matches_recovery_scan() {
        let index = inbound_handoff_pending_index();
        assert_eq!(
            index.keys,
            doc! { "handoff_status": 1, "created_at": 1, "_id": 1 }
        );
        let options = index.options.expect("inbound handoff index options");
        assert_eq!(options.unique, None);
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! {
                "direction": "inbound",
                "handoff_status": "pending",
            })
        );
    }

    #[test]
    fn principal_relay_pending_index_matches_reconcile_scan() {
        let index = principal_relay_pending_index();
        assert_eq!(
            index.keys,
            doc! { "relay_state": 1, "resolved_at": 1, "_id": 1 }
        );
        let options = index
            .options
            .expect("principal relay pending index options");
        assert_eq!(options.unique, None);
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! {
                "status": "resolved",
                "relay_state": "pending",
            })
        );
    }

    #[test]
    fn principal_pending_dedupe_index_uses_full_contact_identity() {
        let index = principal_escalation_pending_dedupe_index();
        assert_eq!(
            index.keys,
            doc! {
                "workspace_id": 1,
                "account_id": 1,
                "contact_wxid": 1,
                "category": 1,
            }
        );
        let options = index.options.expect("principal pending dedupe options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "status": "pending" })
        );
    }

    #[test]
    fn send_ledger_indexes_are_account_scoped_and_outbox_unique() {
        assert_eq!(
            send_ledger_contact_history_index().keys,
            doc! {
                "workspace_id": 1,
                "account_id": 1,
                "contact_wxid": 1,
                "sent_at": -1,
            }
        );
        assert_eq!(
            send_ledger_target_stats_index().keys,
            doc! {
                "workspace_id": 1,
                "account_id": 1,
                "send_kind": 1,
                "target_id": 1,
            }
        );
        let unique = send_ledger_outbox_unique_index();
        assert_eq!(unique.keys, doc! { "outbox_id": 1 });
        let options = unique.options.expect("send ledger unique options");
        assert_eq!(options.unique, Some(true));
        assert_eq!(
            options.partial_filter_expression,
            Some(doc! { "outbox_id": { "$type": "objectId" } })
        );
    }
}

async fn ensure_llm_provider_indexes(db: &Database) -> anyhow::Result<()> {
    // 历史遗留：早期版本错误地用 snake_case 字段建过 unique 索引，
    // 但模型 BSON 层是 camelCase → 旧索引把所有真实文档当成
    // (workspace_id=null, provider_id=null) 重复键。开机时 best-effort drop。
    let _ = db
        .llm_provider_configs()
        .drop_index("workspace_id_1_provider_id_1", None)
        .await;
    let _ = db
        .llm_provider_configs()
        .drop_index("workspace_id_1_is_active_1", None)
        .await;
    db.llm_provider_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspaceId": 1, "providerId": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.llm_provider_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspaceId": 1, "isActive": 1 })
                .build(),
            None,
        )
        .await?;
    validate_llm_vision_assignments(db).await?;
    db.llm_provider_configs()
        .create_index(llm_vision_active_unique_index(), None)
        .await?;
    Ok(())
}

/// objective-purchase-facts G2：`products` 商品库索引。
///
/// - `(workspace_id, product_id)` 唯一：商品业务主键在租户内唯一，CRUD upsert
///   的幂等门，DuplicateKey 视为「product_id 已存在」。
/// - `(workspace_id, status)`：前端商品列表按 status（active/archived）筛选。
///
/// 字段为 snake_case：`Product` 结构未加 `#[serde(rename_all)]`，BSON 层即 snake_case。
async fn ensure_products_indexes(db: &Database) -> anyhow::Result<()> {
    db.products()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "product_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    db.products()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// 活动定向推送索引。
/// - campaigns `(workspaceId, accountId, status)`：按状态列活动。
/// - campaign_sends `(campaignId, contactWxid)` unique：活动级去重闸
///   （同一活动对同一人只推一次，仿 outbox idempotency_key）。
///
/// key 用 camelCase：`Campaign` / `CampaignSend` 均带 `#[serde(rename_all =
/// "camelCase")]`，BSON 层字段即 camelCase。若用 snake_case 会重蹈
/// `ensure_llm_provider_indexes` 的覆辙——索引建在 null 字段上，unique 门把所有
/// 文档当 (null, null) 重复键，第二条 CampaignSend 起全部 DuplicateKey，去重闸失效。
async fn ensure_campaigns_indexes(db: &Database) -> anyhow::Result<()> {
    db.campaigns()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspaceId": 1, "accountId": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    db.campaigns()
        .create_index(campaign_dispatch_recovery_index(), None)
        .await?;
    db.campaign_sends()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "campaignId": 1, "contactWxid": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// agent-autonomy-loop W0 / R13.1：`agent_send_outbox` 索引。
///
/// - `(account_id, status, next_retry_at)`：dispatcher worker 扫描待发送条目。
/// - `(workspace_id, account_id, idempotency_key)` 唯一：租户内强幂等门。
/// - `(status, locked_until)`：崩溃恢复扫描过期 lease。
/// - `(source_event_id, contact_wxid)`：按入站事件追溯发送链路。
async fn ensure_agent_send_outbox_indexes(db: &Database) -> anyhow::Result<()> {
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "account_id": 1, "status": 1, "next_retry_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.collection_agent_send_outbox()
        .create_index(outbox_idempotency_unique_index(), None)
        .await?;
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "locked_until": 1 })
                .build(),
            None,
        )
        .await?;
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "source_event_id": 1, "contact_wxid": 1 })
                .build(),
            None,
        )
        .await?;
    // workspace 内账号级发送间隔闸：查某账号 status=sent 的最大 sent_at。
    // 现有 (account_id,status,next_retry_at) 排序键不是 sent_at，无法支撑 sent_at 倒序，
    // 会触发内存 SORT 随历史线性恶化，故单建此索引。
    db.collection_agent_send_outbox()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "status": 1, "sent_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.collection_agent_send_outbox()
        .create_index(outbox_delivery_finalize_pending_index(), None)
        .await?;
    Ok(())
}

/// agent-autonomy-loop W0 / R8.1：`system_taxonomies` 索引。
///
/// 历史上 `(scope, kind, value.id)` 直接走 unique，保证 seed migration 与 admin
/// approve upsert 幂等。Phase E5-T1 引入 active_versions 灰度后，唯一性维度变成
/// `(scope, kind, value.id, version)`，由 [`ensure_ops_versioned_indexes`] 创建；
/// 这里只保留非唯一辅助索引（按 (scope, kind, status) 列字典），列表查询命中。
async fn ensure_system_taxonomies_indexes(db: &Database) -> anyhow::Result<()> {
    let _ = db
        .collection_system_taxonomies()
        .drop_index("sys_tax_scope_kind_status_idx", None)
        .await;
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "scope": 1, "kind": 1, "value.status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("sys_tax_scope_kind_status_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// Ops 三表保留多版本历史，但每个逻辑 scope 只允许一个 current 指针。
///
/// 旧形态：(workspace_id, domain) / (workspace_id, domain, state_key) /
/// (scope, kind, value.id) 三个 unique 索引一一对应一行；同 key 不能同时存在
/// 多个版本，无法做灰度。
///
/// `version: i32` 扩展业务唯一键以保留历史；m048 在索引创建前收敛存量
/// current 指针，随后 partial unique index 从数据库层阻止再次出现多 current。
///
/// 升级顺序：先以新名称建立只含 logical scope key 的 partial unique index，
/// 成功后才 best-effort 删除旧的 `(scope..., current_version)` 非唯一辅助索引。
/// 因此首次升级和后续重启都不会主动撤掉已经生效的唯一约束。
async fn ensure_ops_versioned_indexes(db: &Database) -> anyhow::Result<()> {
    // ── operation_domain_configs ──
    let _ = db
        .operation_domain_configs()
        .drop_index("workspace_id_1_domain_1", None)
        .await;
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1, "version": 1 })
                .options(
                    IndexOptions::builder()
                        .name("op_domain_ws_domain_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_domain_configs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "domain": 1 })
                .options(
                    IndexOptions::builder()
                        .name("uniq_op_domain_ws_domain_current".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    let _ = db
        .operation_domain_configs()
        .drop_index("op_domain_ws_domain_current_idx", None)
        .await;

    // ── operation_state_policies ──
    let _ = db
        .operation_state_policies()
        .drop_index("workspace_id_1_domain_1_state_key_1", None)
        .await;
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "domain": 1,
                    "state_key": 1,
                    "version": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("op_state_policy_ws_domain_state_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_state_policies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "domain": 1,
                    "state_key": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("uniq_op_state_policy_ws_domain_state_current".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    let _ = db
        .operation_state_policies()
        .drop_index("op_state_policy_ws_domain_state_current_idx", None)
        .await;

    // ── system_taxonomies ──
    //
    // 退役旧的非 workspace / 非版本索引以及早期同名非 unique current 索引；
    // m048 已在本 helper 运行前收敛指针，因此可安全建立版本唯一与 partial unique。
    for legacy_name in [
        "scope_1_kind_1_value.id_1",
        "sys_tax_scope_kind_value_version_unique",
        "sys_tax_scope_kind_value_current_idx",
    ] {
        let _ = db
            .collection_system_taxonomies()
            .drop_index(legacy_name, None)
            .await;
    }
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "scope": 1,
                    "kind": 1,
                    "value.id": 1,
                    "version": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("sys_tax_ws_scope_kind_value_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.collection_system_taxonomies()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "scope": 1,
                    "kind": 1,
                    "value.id": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("uniq_sys_tax_ws_scope_kind_value_current".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // SR-046: canonical ids and aliases share one active identity namespace.
    // `value.identityClaims` is an array, so this compound unique multikey
    // index rejects alias↔alias and alias↔canonical races at commit time.
    db.collection_system_taxonomies()
        .create_index(taxonomy_active_identity_unique_index(), None)
        .await?;
    let _ = db
        .collection_system_taxonomies()
        .drop_index("sys_tax_ws_scope_kind_value_current_idx", None)
        .await;
    Ok(())
}

/// agent-autonomy-loop W0 / R8.3：`taxonomy_candidates` 索引。
///
/// - `(scope, kind, status)`：后台列表 `?status=pending` 查询。
/// - `(scope, kind, raw_value)` 唯一：`upsert_candidate` 幂等键，重复值仅累加
///   `occurrences` / 更新 `last_seen_at`。
async fn ensure_taxonomy_candidates_indexes(db: &Database) -> anyhow::Result<()> {
    for legacy_name in ["scope_1_kind_1_status_1", "scope_1_kind_1_raw_value_1"] {
        let _ = db
            .collection_taxonomy_candidates()
            .drop_index(legacy_name, None)
            .await;
    }
    db.collection_taxonomy_candidates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "scope": 1, "kind": 1, "status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("tax_candidate_ws_scope_kind_status_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.collection_taxonomy_candidates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "scope": 1, "kind": 1, "raw_value": 1 })
                .options(
                    IndexOptions::builder()
                        .name("tax_candidate_ws_scope_kind_raw_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// 数字分身建议链 T5：`relationship_type_suggestions` 索引。
///
/// - `(workspace_id, contact_id)` 仅 `status=pending` 时唯一：重复观察累加
///   `occurrences` / 刷新 `last_seen_at`；终态历史不占槽，新证据可开启下一审核周期。
/// - `(workspace_id, status)`：后台审核列表按 status（pending/approved/rejected）筛选。
///
/// 字段为 snake_case：`RelationshipTypeSuggestion` 未加 `#[serde(rename_all)]`，
/// BSON 层即 snake_case，须与此处索引字段逐字一致。
async fn ensure_relationship_type_suggestions_indexes(db: &Database) -> anyhow::Result<()> {
    db.collection_relationship_type_suggestions()
        .create_index(relationship_suggestion_pending_unique_index(), None)
        .await?;
    db.collection_relationship_type_suggestions()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// F23：`suspected_deal_signals` 索引（疑似成交待核实闭环·方案B）。
///
/// - `(workspace_id, contact_id)` **部分唯一**（partialFilterExpression
///   `{status:"pending"}`）：同一 contact 在同 workspace **至多一条 pending** 待核实
///   信号，重复观察累加 `occurrences` / 刷新 `last_seen_at`，DuplicateKey 视为
///   「pending 信号已存在」（gateway upsert 锚此）。approved/rejected 终态记录**不占**
///   唯一槽 → 同一 contact 经核实闭环后，真实二次成交能再生成一条新 pending 进队列。
///   （历史全量 unique `workspace_id_1_contact_id_1` 会让终态记录永久阻断后续 pending，
///   已改；见 gateway.rs upsert 注释。）
/// - `(workspace_id, status)`：后台核实列表按 status（pending/approved/rejected）筛选。
///
/// 字段为 snake_case：`SuspectedDealSignal` 未加 `#[serde(rename_all)]`，
/// BSON 层即 snake_case，须与此处索引字段逐字一致。
async fn ensure_suspected_deal_signals_indexes(db: &Database) -> anyhow::Result<()> {
    // 旧全量 unique (workspace_id, contact_id) → 部分 unique(仅 status=pending)。
    // 同键不同 options 必须先 drop 旧索引否则 create 报 code 85 IndexOptionsConflict。
    // best-effort：旧索引不存在(全新库 / 已迁移)时 IndexNotFound 被吞,二次启动安全。
    // 旧索引无显式 name → MongoDB 自动命名 "workspace_id_1_contact_id_1"。
    let _ = db
        .collection_suspected_deal_signals()
        .drop_index("workspace_id_1_contact_id_1", None)
        .await;
    db.collection_suspected_deal_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "contact_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "status": "pending" })
                        .name("uniq_suspected_deal_pending_ws_contact".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.collection_suspected_deal_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    Ok(())
}

/// agent-self-evolution W0 (Task 1.2)：5 张新 collection + prompt_templates
/// 多版本辅助索引。
///
/// - `experiments`：`(workspace_id, account_id, started_at desc)` 列表查询；
///   `(experiment_id)` 唯一保证 envelope 不重复 insert；另有
///   `(workspace_id, account_id, experiment_id)` 支撑租户内关联查询（Requirements 1.3）。
/// - `proposals`：`(workspace_id, account_id, status, created_at desc)` 后台
///   按状态分页；`(workspace_id, account_id, experiment_id)` 反查 cohort 下所有
///   proposal（Requirements 5.x）。
/// - `shadow_replays`：`(workspace_id, account_id, proposal_id)` 聚合；`(workspace_id, account_id,
///   started_at desc)` 后台监控（Requirements 5.x）。
/// - `threshold_overrides`：`(workspace_id, account_id, gate_key, released_at
///   desc)` 是 `resolve_thresholds` 取最新有效值的核心查询路径（Requirements 6.2）。
/// - `prompt_templates` 多版本支持：`(workspace_id, prompt_key, current_version)`
///   过滤 current 那条；`(workspace_id, prompt_key, version)` 唯一保证同 key
///   下版本号不冲突（Requirements 6.4 / 6.5）。
async fn ensure_evolution_indexes(db: &Database) -> anyhow::Result<()> {
    // experiments
    db.experiments()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1, "started_at": -1 })
                .build(),
            None,
        )
        .await?;
    db.experiments()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "experiment_id": 1,
                })
                .build(),
            None,
        )
        .await?;
    db.experiments()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "experiment_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;

    // proposals
    db.proposals()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "status": 1,
                    "created_at": -1,
                })
                .build(),
            None,
        )
        .await?;
    db.proposals()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "experiment_id": 1,
                })
                .build(),
            None,
        )
        .await?;

    // shadow_replays
    db.shadow_replays()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "proposal_id": 1,
                })
                .build(),
            None,
        )
        .await?;
    db.shadow_replays()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "started_at": -1,
                })
                .build(),
            None,
        )
        .await?;

    // threshold_overrides
    db.threshold_overrides()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "gate_key": 1,
                    "released_at": -1,
                })
                .build(),
            None,
        )
        .await?;
    db.threshold_overrides()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "gate_key": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("uniq_threshold_current_per_scoped_gate".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.threshold_overrides()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "source_proposal_id": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("uniq_threshold_artifact_per_proposal".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! {
                            "source_proposal_id": { "$type": "objectId" },
                        })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // threshold_overrides_audit（Phase C / C5）：不可变变更日志。admin 审计读路径
    // 按 (workspace_id, account_id, gate_key, decided_at desc) 拉单 gate 历史，
    // 也支持去掉 gate_key 拉全量；append-only，无 unique 约束。
    db.threshold_overrides_audit()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "gate_key": 1,
                    "decided_at": -1,
                })
                .build(),
            None,
        )
        .await?;

    // post_release_reviews（W4 Task 5.6 一并加，避免 W4 再补一波索引）
    db.post_release_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "scheduled_at": 1,
                    "completed": 1,
                })
                .build(),
            None,
        )
        .await?;
    db.post_release_reviews()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "proposal_id": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("uniq_post_release_review_protocol_v1".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "protocol_version": 1 })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // prompt_templates 多版本辅助：(workspace_id, prompt_key, current_version)
    // 用于 ensure_prompt_pack_v2 + release_prompt 在同 key 下定位 current 那条；
    // (workspace_id, prompt_key, version) 唯一保证多版本不冲突。
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1, "current_version": 1 })
                .build(),
            None,
        )
        .await?;
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1 })
                .options(
                    IndexOptions::builder()
                        .name("uniq_prompt_current_pointer".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "current_version": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "source_proposal_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("uniq_prompt_artifact_per_proposal".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! {
                            "source_proposal_id": { "$type": "objectId" },
                        })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.prompt_templates()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "prompt_key": 1, "version": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;

    // ── knowledge-digest-workstation ──
    //
    // knowledge_daily_reports：(workspace_id, account_id, report_date) 三元组
    // 复合 unique，保证一天一份；同时支持按 (account_id, report_date desc) 拉
    // 当日 / 最近 N 天日报。
    db.knowledge_daily_reports()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "report_date": -1,
                })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;

    // knowledge_chat_tasks：worker 取 pending 用 (status, created_at)；
    // chat 面板按 sessionId 拉历史用 (session_id, status)；
    // 任务总览列表 chat_task_list 按 workspace 过滤 + created_at 倒序拉，
    // 用 (workspace_id, created_at) 服务该查询避免全表扫。
    db.knowledge_chat_tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "locked_until": 1, "created_at": 1 })
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "status": 1 })
                .build(),
            None,
        )
        .await?;
    db.knowledge_chat_tasks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "created_at": -1 })
                .build(),
            None,
        )
        .await?;

    // knowledge_operator_memory：chat 注入按
    // (account_id, operator_id, last_used_at desc) 拉 top N。
    db.knowledge_operator_memory()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "workspace_id": 1,
                    "account_id": 1,
                    "operator_id": 1,
                    "last_used_at": -1,
                })
                .build(),
            None,
        )
        .await?;

    // P1-9：knowledge_operator_memory.expires_at 上挂 TTL 索引（expireAfterSeconds=0）。
    // MongoDB 后台进程会在 `expires_at < now()` 时把对应文档自动删除——长期跑下
    // 来运营 memory 不会无界堆积；`expires_at == None` 的文档不会被 TTL 命中
    // （MongoDB TTL 只清理 BSON Date 字段，缺失字段会被忽略）。
    // 名字 `kop_memory_expires_ttl` 显式标记，避免与上面的 last_used_at 索引误并。
    db.knowledge_operator_memory()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kop_memory_expires_ttl".to_string())
                        .expire_after(std::time::Duration::from_secs(0))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── knowledge-wiki Phase A：4 个新 collection 的索引 + chunks 新字段索引 ──
    //
    // 这一组索引服务"四件事"的检索面：
    //   * chunk_revisions：按 chunk_id 时间倒序读 timeline；按 created_at 全局
    //     扫"最近 N 条"；
    //   * knowledge_gap_signals：worker 拉 pending 任务、admin 看 timeline；
    //   * domain_schemas：workspace+schema_id+version 唯一标识，加 is_active
    //     快路径；
    //   * catalog_rebuild_jobs：workspace+status+queued_at 决定 worker 取哪批；
    //   * operation_knowledge_chunks 三条新查询路径：按 wiki_type 分组、按
    //     valid_to 找 stale、按 dynamic_confidence 取 top。
    //
    // 旧 chunks 索引（document_id+item_id+status / status+priority）
    // 仍然保留，召回算法零改动。
    let chunk_revisions = db.raw().collection::<Document>("chunk_revisions");
    ensure_index_or_equivalent_name(chunk_revisions.clone(), chunk_revision_identity_index())
        .await?;
    ensure_index_or_equivalent_name(chunk_revisions, chunk_revision_timeline_index()).await?;
    // As with behavior signals, an already-recorded m039 migration will not
    // rerun. Retire the unscoped indexes only after both scoped replacements
    // are known to exist, preserving continuous query coverage on upgrades.
    retire_legacy_chunk_revision_indexes(db).await?;
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1, "kind": 1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_status_kind_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_created_at_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "signal_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_signal_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.knowledge_gap_signals()
        .create_index(gap_signals_pending_dedup_index(), None)
        .await?;
    // LintView dashboard：按 (kind, status) 分组的时间线视图。
    // 与 gap_signals_status_kind_idx 的差异是字段顺序与排序键 —— 前端
    // /api/knowledge/gap-signals?kind=X 直接走这条避免 in-memory sort。
    db.knowledge_gap_signals()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "kind": 1, "status": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("gap_signals_kind_status_created_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // m044 validates all rows before index creation. Existing deployments may
    // still have these non-unique helper indexes, so replace them explicitly.
    for legacy_name in [
        "domain_schemas_ws_id_version_idx",
        "domain_schemas_ws_active_idx",
    ] {
        let _ = db.domain_schemas().drop_index(legacy_name, None).await;
    }
    db.domain_schemas()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "schema_id": 1, "version": 1 })
                .options(
                    IndexOptions::builder()
                        .name("domain_schemas_ws_id_version_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.domain_schemas()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("domain_schemas_ws_active_unique".to_string())
                        .unique(true)
                        .partial_filter_expression(doc! { "is_active": true })
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    for legacy_name in [
        "domain_profiles_ws_id_version_idx",
        "domain_profiles_ws_active_idx",
    ] {
        let _ = db.domain_profiles().drop_index(legacy_name, None).await;
    }
    db.domain_profiles()
        .create_index(domain_profile_version_unique_index(), None)
        .await?;
    db.domain_profiles()
        .create_index(domain_profile_current_unique_index(), None)
        .await?;
    db.domain_profiles()
        .create_index(domain_profile_active_unique_index(), None)
        .await?;
    let _ = db
        .catalog_rebuild_jobs()
        .drop_index("catalog_jobs_status_queued_idx", None)
        .await;
    db.catalog_rebuild_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! {
                    "status": 1,
                    "next_retry_at": 1,
                    "target_generation": 1,
                    "queued_at": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("catalog_jobs_retry_claim_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.catalog_rebuild_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "locked_until": 1 })
                .options(
                    IndexOptions::builder()
                        .name("catalog_jobs_lease_reclaim_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.catalog_rebuild_jobs()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "job_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("catalog_jobs_job_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "wiki_type": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_wiki_type_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "valid_to": 1, "status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_valid_to_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.operation_knowledge_chunks()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "dynamic_confidence": -1 })
                .options(
                    IndexOptions::builder()
                        .name("kchunks_dynamic_confidence_idx".to_string())
                        .sparse(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── P0 鉴权 / Session ─────────────────────────────────────────────────
    // admin_users.username unique：登录路径按 username 查；同名禁止。
    db.raw()
        .collection::<mongodb::bson::Document>("admin_users")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "username": 1 })
                .options(
                    IndexOptions::builder()
                        .name("admin_users_username_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // admin_sessions.session_id unique：cookie 唯一定位 session。
    db.raw()
        .collection::<mongodb::bson::Document>("admin_sessions")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "session_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("admin_sessions_session_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // admin_sessions.expires_at TTL：mongo 自动清理过期 session。
    // expireAfterSeconds=0 表示「字段时间到达即过期」（不是字段时间 + N 秒）。
    db.raw()
        .collection::<mongodb::bson::Document>("admin_sessions")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("admin_sessions_ttl".to_string())
                        .expire_after(std::time::Duration::from_secs(0))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // Authentication failures are append-only security audit rows containing
    // only process-salted fingerprints. These indexes support recent incident
    // review by client or target without storing raw usernames or addresses.
    let auth_security_events = db
        .raw()
        .collection::<mongodb::bson::Document>("auth_security_events");
    auth_security_events
        .create_index(
            IndexModel::builder()
                .keys(doc! { "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("auth_security_events_created_at_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    auth_security_events
        .create_index(
            IndexModel::builder()
                .keys(doc! { "expires_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("auth_security_events_expires_ttl_v1".to_string())
                        .expire_after(std::time::Duration::from_secs(0))
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    auth_security_events
        .create_index(
            IndexModel::builder()
                .keys(doc! { "client_fingerprint": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("auth_security_events_client_created_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    auth_security_events
        .create_index(
            IndexModel::builder()
                .keys(doc! { "target_fingerprint": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("auth_security_events_target_created_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── P1-6 auto-ingest ──────────────────────────────────────────────────
    // ingest_sources：worker 每轮扫 (workspace_id, kind, status="active") 决定要拉哪些 source。
    db.ingest_sources()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "kind": 1, "status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("ingest_sources_ws_kind_status_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // Lease recovery and active/failing candidate scans. The worker still
    // performs a source-id CAS after scanning; this index bounds the scan.
    db.ingest_sources()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "status": 1, "locked_until": 1, "last_fetched_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name("ingest_sources_lease_reclaim_idx".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    // ingest_sources.source_id unique：CRUD 与 worker 落点都按 source_id 定位单行。
    db.ingest_sources()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "source_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("ingest_sources_source_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── Phase C / C1 reviewer_stats ───────────────────────────────────────
    // reviewer_stats.stat_id unique：feedback_worker 每 workspace 一行滚动统计，
    // upsert 落点按 stat_id (`<workspace_id>::reviewer`) 定位。
    db.raw()
        .collection::<mongodb::bson::Document>("reviewer_stats")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "stat_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("reviewer_stats_stat_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── D（自学习可观测）deal_attribution_stats ───────────────────────────
    // 同 reviewer_stats：feedback_worker 每 workspace 一行滚动统计，upsert 落点按
    // stat_id (`<workspace_id>::deal_attribution`) 定位，存最近一轮 30d 窗口成交追认
    // 强化的命中数（H11-linkage 效果观测）。
    db.raw()
        .collection::<mongodb::bson::Document>("deal_attribution_stats")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "stat_id": 1 })
                .options(
                    IndexOptions::builder()
                        .name("deal_attribution_stats_stat_id_unique".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            None,
        )
        .await?;

    // ── Phase G / P2 lessons_learned ──────────────────────────────────────
    // lessons_learned 经 raw collection 读写（无 typed accessor）；list 查询按
    // {workspace_id} 过滤 + {updated_at:-1} 排序（lessons_learned.rs:60,76），
    // 此复合索引覆盖该访问路径。
    db.raw()
        .collection::<mongodb::bson::Document>("lessons_learned")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "updated_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("lessons_learned_ws_updated".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.raw()
        .collection::<mongodb::bson::Document>("lessons_learned")
        .create_index(lesson_identity_unique_index(), None)
        .await?;

    // agent_principal_escalations：复合查询索引 + 短码唯一索引
    db.agent_principal_escalations()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "status": 1, "contact_wxid": 1 })
                .options(
                    IndexOptions::builder()
                        .name("idx_principal_escalation_ws_status_contact".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.agent_principal_escalations()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "short_code": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .name("uniq_principal_escalation_short_code".to_string())
                        .build(),
                )
                .build(),
            None,
        )
        .await?;
    db.agent_principal_escalations()
        .create_index(principal_relay_pending_index(), None)
        .await?;
    db.agent_principal_escalations()
        .create_index(principal_card_delivery_reconcile_index(), None)
        .await?;
    db.agent_principal_escalations()
        .create_index(principal_card_timeout_index(), None)
        .await?;
    // 同账号客户同类别只允许一条 pending 请示：完整业务身份是
    // (workspace, account, contact)。不同账号可复用同一 wxid，不能互相压制。
    // 此前用 has_pending_for_contact (count) 后再 insert_pending_escalation，存在 TOCTOU——
    // follow-up worker 与 webhook debounce runner 是两个独立 tokio 任务，可并发跑同一 contact，
    // 各 count 到 0 → 各插一条 → 领导被推两张卡。partial filter 限定 status=pending 否则会
    // 误伤 resolved 历史（同客户同类别本就可多次历史请示）。insert 侧捕获本索引的 11000
    // dup-key 当作"已存在 pending"静默跳过推卡（见 ledger::insert_pending_escalation）。
    db.agent_principal_escalations()
        .create_index(principal_escalation_pending_dedupe_index(), None)
        .await?;

    Ok(())
}
