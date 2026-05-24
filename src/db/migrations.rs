//! 版本化数据迁移：启动时幂等执行未应用的迁移。
//!
//! 每条迁移在 `migrations` 集合留下 [`MigrationRecord`]，下次启动跳过已应用项。
//! 迁移本身必须幂等（即使标记丢失，重跑也不破坏数据），以便支持回滚后重跑。
//!
//! 使用方式：
//! ```ignore
//! let db = Database::connect(...).await?;
//! db::migrations::run(&db).await?;   // 先迁移
//! db.ensure_indexes().await?;        // 再建索引
//! ```

use std::future::Future;
use std::pin::Pin;

use futures::TryStreamExt;
use mongodb::bson::{doc, Bson, DateTime, Document};
use mongodb::options::UpdateOptions;

use crate::error::AppResult;
use crate::models::{MigrationRecord, TaxonomyEntry, TaxonomyValue};

use super::Database;

type MigrationFuture<'a> = Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>>;
pub type MigrationFn = for<'a> fn(&'a Database) -> MigrationFuture<'a>;

/// 单条迁移定义：`id` 必须 chronologically sortable（建议 `YYYY_MM_NNN_*` 命名）。
pub struct Migration {
    pub id: &'static str,
    pub run: MigrationFn,
}

/// 全局迁移列表。Tasks 6/7/8/13 会按顺序追加条目。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        id: "2026_05_001_split_last_message_at",
        run: |db| Box::pin(split_last_message_at(db)),
    },
    Migration {
        id: "2026_05_002_split_active_facts",
        run: |db| Box::pin(split_active_facts(db)),
    },
    Migration {
        id: "2026_05_003_state_machine_allowed_from",
        run: |db| Box::pin(seed_state_machine_allowed_from(db)),
    },
    Migration {
        id: "2026_05_004_outcome_metrics_workspace_in_id",
        run: |db| Box::pin(rewrite_outcome_metrics_id_with_workspace(db)),
    },
    // ---- 以下 3 条迁移脚手架在 W0 阶段仅占位（task 1.4），实质逻辑见对应波次 ----
    Migration {
        id: "2026_05_005_memory_facts_to_structured",
        run: |db| Box::pin(memory_facts_to_structured_placeholder(db)),
    },
    Migration {
        id: "2026_05_006_taxonomy_seed",
        run: |db| Box::pin(seed_default_taxonomies(db)),
    },
    Migration {
        id: "2026_05_007_outbox_indexes",
        run: |db| Box::pin(outbox_indexes_placeholder(db)),
    },
    // ---- M2 Strategic Planner ----
    Migration {
        id: "2026_05_008_contact_commitments_reshape",
        run: |db| Box::pin(contact_commitments_reshape(db)),
    },
    // ── agent-self-evolution M4 (W0 Task 1.5) ──
    Migration {
        id: "2026_05_M4_001_prompt_template_versioned",
        run: |db| Box::pin(prompt_template_versioned(db)),
    },
    // ── conversation_mode + per-contact custom instructions + knowledge tags ──
    // 给 contacts 加 custom_agent_instructions（默认 null），给 operation_knowledge_*
    // 三集合加 product_tags / trigger_keywords / business_topics（默认 []）。
    // 全幂等：filter 用 `$exists: false` 仅命中未升级文档。
    Migration {
        id: "2026_05_V3_001_contact_custom_instructions_and_knowledge_tags",
        run: |db| Box::pin(contact_custom_instructions_and_knowledge_tags(db)),
    },
    // ── knowledge-base cleanup（推倒销售话术 RAG 形态，全面切到 wiki 方法论）──
    // 开发期数据无需兼容；生产环境守卫会阻断 drop 操作。
    // 1) 清空旧三层集合的存量销售域数据（保留集合本身，索引由后续 commit 调整）
    // 2) 清空 system_taxonomies 中的销售域 seed（customer_stage / intent_level / objection_type）
    Migration {
        id: "2026_05_25_drop_legacy_sales_collections",
        run: |db| Box::pin(drop_legacy_sales_collections(db)),
    },
    Migration {
        id: "2026_05_25_drop_legacy_taxonomy_seed",
        run: |db| Box::pin(drop_legacy_taxonomy_seed(db)),
    },
];

/// 2026_05_001：把存量 contact 的 `last_message_at` 回填到 `last_inbound_at`，
/// 仅在 `last_inbound_at` 缺失（不存在或为 null）且 `last_message_at` 存在时回填。
///
/// 用 aggregation pipeline 形式的 `update_many`，单次原子操作即完成；
/// 二次执行时所有候选都已回填过，filter 不再命中，从而幂等。
async fn split_last_message_at(db: &Database) -> AppResult<()> {
    let pipeline: Vec<Document> = vec![doc! {
        "$set": {
            "last_inbound_at": "$last_message_at"
        }
    }];
    let result = db
        .contacts()
        .update_many(
            doc! {
                "$and": [
                    { "last_message_at": { "$exists": true, "$ne": null } },
                    {
                        "$or": [
                            { "last_inbound_at": { "$exists": false } },
                            { "last_inbound_at": null }
                        ]
                    }
                ]
            },
            pipeline,
            None,
        )
        .await?;
    tracing::info!(
        modified = result.modified_count,
        matched = result.matched_count,
        "backfilled last_inbound_at from last_message_at"
    );
    Ok(())
}

/// 2026_05_002：把 `operating_memories.memory_card.activeFacts` 拆分为
/// `coreFacts`（前 6 条按重要度，由旧顺序近似）和 `recentFacts`（剩余项）。
///
/// 拆完后 `$unset memory_card.activeFacts`，再次执行时 filter 不再命中，
/// 从而幂等。`migrations` 集合的版本记录是双保险。
async fn split_active_facts(db: &Database) -> AppResult<()> {
    let pipeline: Vec<Document> = vec![
        doc! {
            "$set": {
                "memory_card.coreFacts": {
                    "$slice": [
                        { "$ifNull": ["$memory_card.activeFacts", []] },
                        6
                    ]
                },
                "memory_card.recentFacts": {
                    "$slice": [
                        { "$ifNull": ["$memory_card.activeFacts", []] },
                        6,
                        10000_i64
                    ]
                }
            }
        },
        doc! {
            "$unset": "memory_card.activeFacts"
        },
    ];
    let result = db
        .operating_memories()
        .update_many(
            doc! { "memory_card.activeFacts": { "$exists": true } },
            pipeline,
            None,
        )
        .await?;
    tracing::info!(
        modified = result.modified_count,
        matched = result.matched_count,
        "split memory_card.activeFacts into coreFacts/recentFacts"
    );
    Ok(())
}

/// 2026_05_003：为 `user_operations.state_machine` 中缺失的状态补齐
/// `allowedFrom` / `allowFromAny`，但保留运营人员已经自定义的状态名称、
/// 目标、动作和规则。
async fn seed_state_machine_allowed_from(db: &Database) -> AppResult<()> {
    let default_state_machine = crate::prompts::default_user_operation_state_machine();
    let default_states = default_state_machine
        .get_array("states")
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_document().cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut cursor = db
        .operation_domain_configs()
        .find(
            doc! {
                "domain": "user_operations",
                "$or": [
                    { "state_machine.states.allowedFrom": { "$exists": false } },
                    { "state_machine.states.allowFromAny": { "$exists": false } }
                ]
            },
            None,
        )
        .await?;
    let mut modified = 0_u64;
    while let Some(config) = cursor.try_next().await? {
        let Some(id) = config.id else { continue };
        let mut state_machine = config.state_machine.clone();
        if merge_allowed_from_defaults(&mut state_machine, &default_states) {
            db.operation_domain_configs()
                .update_one(
                    doc! { "_id": id },
                    doc! {
                        "$set": {
                            "state_machine": state_machine,
                            "updated_at": DateTime::now()
                        }
                    },
                    None,
                )
                .await?;
            modified += 1;
        }
    }
    tracing::info!(
        modified,
        "seeded state_machine.allowedFrom defaults for user_operations domain"
    );
    Ok(())
}

fn merge_allowed_from_defaults(state_machine: &mut Document, default_states: &[Document]) -> bool {
    let Ok(states) = state_machine.get_array_mut("states") else {
        return false;
    };
    let mut changed = false;
    for state in states.iter_mut().filter_map(Bson::as_document_mut) {
        let Some(key) = state.get_str("key").ok().map(ToString::to_string) else {
            continue;
        };
        let Some(default_state) = default_states
            .iter()
            .find(|item| item.get_str("key").ok() == Some(key.as_str()))
        else {
            continue;
        };
        if !state.contains_key("allowedFrom") {
            let allowed = default_state
                .get_array("allowedFrom")
                .cloned()
                .unwrap_or_default();
            state.insert("allowedFrom", Bson::Array(allowed));
            changed = true;
        }
        if !state.contains_key("allowFromAny") {
            let allow_any = default_state.get_bool("allowFromAny").unwrap_or(false);
            if allow_any {
                state.insert("allowFromAny", true);
                changed = true;
            }
        }
    }
    changed
}

/// 2026_05_004：把 `agent_outcome_metrics._id` 从老 3 段
/// `{account}:{horizon}:{date}` 升级到 4 段 `{workspace}:{account}:{horizon}:{date}`。
///
/// 不能就地 `update_one` 改 `_id`（MongoDB 禁止），所以用 "insert+delete" 方式：
/// 1. 扫描所有 _id 不含 4 段的文档；
/// 2. 用文档自带的 workspace_id / account_id 字段拼新 _id 写入新文档；
/// 3. 删除老文档。
///
/// 幂等性：filter 命中"_id 字符串中不含 3 个冒号"才动；二次执行时所有文档都已是
/// 4 段，filter 不再命中。`migrations` 集合的版本记录是双保险。
async fn rewrite_outcome_metrics_id_with_workspace(db: &Database) -> AppResult<()> {
    let coll = db.outcome_metrics();
    // 用 regex 兼容老 3 段（最多 2 个冒号）的 _id；同时排除已是 4 段格式的项。
    let mut cursor = coll
        .find(
            doc! {
                "_id": { "$type": "string" },
                "workspace_id": { "$exists": true, "$type": "string" },
                "$expr": {
                    "$ne": [
                        { "$size": { "$split": ["$_id", ":"] } },
                        4_i32
                    ]
                }
            },
            None,
        )
        .await?;
    let mut migrated = 0_u64;
    while let Some(metric) = cursor.try_next().await? {
        let new_id = format!(
            "{}:{}:{}:{}",
            metric.workspace_id, metric.account_id, metric.horizon, metric.date
        );
        if new_id == metric.id {
            continue;
        }
        let mut new_metric = metric.clone();
        new_metric.id = new_id.clone();
        // upsert：万一新 id 已存在（比如重跑），靠 _id 自然去重。
        let new_doc = mongodb::bson::to_document(&new_metric)?;
        coll.update_one(
            doc! { "_id": &new_id },
            doc! { "$set": new_doc },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await?;
        coll.delete_one(doc! { "_id": &metric.id }, None).await?;
        migrated += 1;
    }
    tracing::info!(
        migrated,
        "rewrote agent_outcome_metrics._id to include workspace_id"
    );
    Ok(())
}

/// 2026_05_005（W5 task 6.6 实质实现）：
/// 把 `operating_memories.memory_card.coreFacts / recentFacts` 中的字符串元素
/// 升级为结构化 `MemoryFact { id, text, confidence, importance, ... }`。
///
/// 升级规则（与 `MemoryFactRepr::Plain → Structured` 反序列化路径一致，
/// 详见 `src/models.rs::MemoryFact`）：
/// * `id`：fresh UUIDv4（关键：避免老数据无 id 后续合并失真）；
/// * `text`：原始字符串；
/// * `confidence: 7` / `importance: 5`（默认中等）；
/// * `mayExpire: false`；
/// * `createdAt = updatedAt = now`；
/// * `deprecatedAt / deprecationReason / sourceMessageIds / sourceRunId`：缺省。
///
/// 幂等：用每个元素是否含 `id` 字段判定，已结构化的元素跳过；本迁移把
/// `memory_card_version` 加 1 让上层缓存失效。
async fn memory_facts_to_structured_placeholder(db: &Database) -> AppResult<()> {
    use mongodb::bson::oid::ObjectId;

    let collection = db.operating_memories();
    let mut cursor = collection
        .clone_with_type::<Document>()
        .find(
            doc! {
                "$or": [
                    { "memory_card.coreFacts": { "$type": "array" } },
                    { "memory_card.recentFacts": { "$type": "array" } }
                ]
            },
            None,
        )
        .await?;
    let mut upgraded_docs: i64 = 0;
    let mut upgraded_facts: i64 = 0;
    let now = DateTime::now();

    while let Some(raw) = cursor.try_next().await? {
        let Some(id) = raw.get_object_id("_id").ok() else {
            continue;
        };
        let card = match raw.get_document("memory_card") {
            Ok(c) => c.clone(),
            Err(_) => continue,
        };
        let (new_core, core_changed) =
            upgrade_fact_array(card.get_array("coreFacts").ok(), now, &mut upgraded_facts);
        let (new_recent, recent_changed) = upgrade_fact_array(
            card.get_array("recentFacts").ok(),
            now,
            &mut upgraded_facts,
        );
        if !core_changed && !recent_changed {
            continue;
        }
        let mut new_card = card.clone();
        if core_changed {
            new_card.insert("coreFacts", Bson::Array(new_core));
        }
        if recent_changed {
            new_card.insert("recentFacts", Bson::Array(new_recent));
        }
        let _ = id; // _id 仅用于 filter
        collection
            .clone_with_type::<Document>()
            .update_one(
                doc! { "_id": ObjectId::from(id) },
                doc! {
                    "$set": {
                        "memory_card": new_card,
                        "updated_at": now,
                    },
                    "$inc": { "memory_card_version": 1 }
                },
                None,
            )
            .await?;
        upgraded_docs += 1;
    }

    tracing::info!(
        migration_id = "2026_05_005_memory_facts_to_structured",
        upgraded_docs,
        upgraded_facts,
        "structured fact upgrade applied"
    );
    Ok(())
}

/// 把 `Vec<Bson>`（混合 String / Document）升级为全结构化 Document 数组。
/// 返回 `(new_array, changed)`：`changed=false` 表示数组中所有元素已经是
/// 结构化（有 `id` 字段），跳过本次写入。
fn upgrade_fact_array(
    raw: Option<&Vec<Bson>>,
    now: DateTime,
    counter: &mut i64,
) -> (Vec<Bson>, bool) {
    let Some(raw) = raw else {
        return (Vec::new(), false);
    };
    let mut changed = false;
    let mut out = Vec::with_capacity(raw.len());
    for item in raw {
        match item {
            Bson::String(text) => {
                changed = true;
                *counter += 1;
                out.push(Bson::Document(structured_fact_doc(text, now)));
            }
            Bson::Document(doc) => {
                if doc.get_str("id").is_ok() {
                    out.push(Bson::Document(doc.clone()));
                } else {
                    changed = true;
                    *counter += 1;
                    let text = doc.get_str("text").unwrap_or("").to_string();
                    let mut upgraded = structured_fact_doc(&text, now);
                    // 保留原 doc 的 evidence / confidence / importance 等字段
                    for (k, v) in doc.iter() {
                        if k == "id" || k == "text" {
                            continue;
                        }
                        upgraded.insert(k, v.clone());
                    }
                    out.push(Bson::Document(upgraded));
                }
            }
            other => {
                // 非 String / Document 元素保留不动（防御）。
                out.push(other.clone());
            }
        }
    }
    (out, changed)
}

fn structured_fact_doc(text: &str, now: DateTime) -> Document {
    doc! {
        "id": uuid::Uuid::new_v4().to_string(),
        "text": text,
        "confidence": 7i32,
        "importance": 5i32,
        "mayExpire": false,
        "sourceMessageIds": Vec::<Bson>::new(),
        "createdAt": now,
        "updatedAt": now,
    }
}

/// 2026_05_006（W3 task 4.9 实质实现）：把 prompt 中硬编码的运营术语写入
/// `system_taxonomies`，scope=`global`，作为 `customer_stage / intent_level /
/// objection_type` 三个维度的默认字典。
///
/// 幂等机制（双层保险）：
///
/// 1. **migration 框架层**：同一 `migration_id` 在 `migrations` 集合的
///    `_id` 唯一约束下二次启动会被 `run_with` 直接 skip，不会进到这里。
/// 2. **upsert 层**：即使 migration 记录丢失被强制重跑，本函数对每条
///    `(scope, kind, value.id)` 走 `update_one + upsert(true)`，依赖
///    `ensure_system_taxonomies_indexes` 创建的 `(scope, kind, value.id)`
///    唯一索引保证不会重复插入；`$setOnInsert` 用于不覆盖运营人员后续
///    通过后台 API 改过的 `displayName / description / aliases / status`
///    字段，仅在记录不存在时才写入默认值。
///
/// 数据来源：与 `src/prompts.rs` 中现有 prompt 文案对齐，确保升级后老
/// contact 的 `customer_stage / intent_level` 取值仍能通过 R8 字典校验
/// （或通过 alias 命中升级到 canonical id）。详见 design.md §3.3 与
/// requirements.md R8.8 / R11.7。
async fn seed_default_taxonomies(db: &Database) -> AppResult<()> {
    let collection = db.collection_system_taxonomies();
    let now = DateTime::now();
    let mut inserted = 0_u64;
    let mut skipped = 0_u64;

    for entry in default_taxonomy_seed_entries(now) {
        let filter = doc! {
            "scope": &entry.scope,
            "kind": &entry.kind,
            "value.id": &entry.value.id,
        };
        // `$setOnInsert` 仅在 upsert 触发 insert 分支时生效；命中已有记录时
        // 这里不会覆盖任何字段（保留运营人员通过后台 API 修改过的状态）。
        let mut doc_to_set = mongodb::bson::to_document(&entry)?;
        // 已 unset `_id` 占位（Option::None 配合 skip_serializing_if 已不会出现）。
        doc_to_set.remove("_id");
        let update = doc! { "$setOnInsert": doc_to_set };
        let result = collection
            .update_one(
                filter,
                update,
                UpdateOptions::builder().upsert(true).build(),
            )
            .await?;
        if result.upserted_id.is_some() {
            inserted += 1;
        } else {
            // 已存在 `(scope, kind, value.id)` 记录，二次执行幂等 skip。
            skipped += 1;
        }
    }

    tracing::info!(
        migration_id = "2026_05_006_taxonomy_seed",
        inserted,
        skipped,
        "seeded default system_taxonomies (customer_stage / intent_level / objection_type)"
    );
    Ok(())
}

/// 默认字典 seed 数据。值与现有 prompt 中的运营术语对齐：
///
/// - `customer_stage`：与 `default_user_operation_state_machine` 的 9 个 state
///   `key` 一一对应，并把 prompt `stage_method` 中描述的中文阶段名（"陌生接触 /
///   初步信任 / 需求探索 / ..."）作为 alias，保证 contact 上的中文 `customer_stage`
///   字段能命中 alias 升级到 canonical id。
/// - `intent_level`：`high / medium / low` 三档，与 `intent_method` / `follow_up_method`
///   prompt 中的高/中/低意向语义对齐，alias 含中文与英文常见写法。
/// - `objection_type`：与 prompt `forbidden_rules / advanceSignals / cooldownSignals`
///   中常见的客户顾虑类别对齐（价格、信任、时机、决策、产品适配、风险、其他）。
fn default_taxonomy_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
    let mut out = Vec::new();

    // ── customer_stage（9 项，对齐 default_user_operation_state_machine）──
    let customer_stages: &[(&str, &str, &str, &[&str])] = &[
        (
            "new_contact",
            "初始了解",
            "建立基本上下文，避免过早推销。",
            &["陌生接触", "新客", "first_contact", "刚加好友"],
        ),
        (
            "relationship_building",
            "关系建立",
            "通过具体帮助和稳定回应建立信任。",
            &["初步信任", "关系培养", "trust_building"],
        ),
        (
            "need_discovery",
            "需求探索",
            "理解真实需求、痛点、动机、阻力和决策方式。",
            &["明确需求", "需求挖掘", "discovery"],
        ),
        (
            "solution_fit",
            "方案匹配",
            "基于产品知识给出真实、可验证的匹配建议。",
            &["方案评估", "方案推荐", "solution_evaluation"],
        ),
        (
            "objection_handling",
            "异议处理",
            "识别顾虑，降低风险感，不强压成交。",
            &["顾虑处理", "objection"],
        ),
        (
            "commitment_followup",
            "承诺跟进",
            "围绕已形成的小承诺做低压推进。",
            &["成交推进", "推进成交", "closing"],
        ),
        (
            "customer_success",
            "客户维护",
            "维护成交后关系，发现复购、转介绍和服务风险。",
            &["交付维护", "复购转介绍", "post_sale"],
        ),
        (
            "cooldown",
            "风险冷却",
            "降低打扰和压迫，等待更合适的触达窗口。",
            &["冷却", "暂停推进"],
        ),
        (
            "dormant_reactivation",
            "沉默唤醒",
            "基于真实价值或明确理由做低频唤醒。",
            &["唤醒", "沉默用户唤醒"],
        ),
    ];
    for (id, display, desc, aliases) in customer_stages {
        out.push(TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "customer_stage".to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
            },
            updated_at: now,
        });
    }

    // ── intent_level（3 档）──
    let intent_levels: &[(&str, &str, &str, &[&str])] = &[
        (
            "high",
            "高意向",
            "主动描述问题、询问方案/价格/周期、愿意提供资料或约时间。",
            &["高", "high_intent", "强意向"],
        ),
        (
            "medium",
            "中意向",
            "有兴趣但信息不足，需要继续探索动机与匹配。",
            &["中", "medium_intent", "中等意向"],
        ),
        (
            "low",
            "低意向",
            "寒暄、围观、无明确问题或多次回避，时机不成熟。",
            &["低", "low_intent", "弱意向", "无明显意向"],
        ),
    ];
    for (id, display, desc, aliases) in intent_levels {
        out.push(TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "intent_level".to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
            },
            updated_at: now,
        });
    }

    // ── objection_type（7 项，对齐常见微信私聊客户顾虑）──
    let objection_types: &[(&str, &str, &str, &[&str])] = &[
        (
            "price",
            "价格异议",
            "对价格、折扣、性价比、预算上限等表达顾虑。",
            &["价格敏感", "嫌贵", "预算不足", "price_concern"],
        ),
        (
            "trust",
            "信任异议",
            "对品牌、口碑、案例真实性、过往合作经历等表达不信任。",
            &["信任不足", "怀疑真实性", "trust_concern"],
        ),
        (
            "timing",
            "时机异议",
            "当前不是合适的购买/决策时机（暂时不需要、再看看、过段时间）。",
            &["时机不对", "暂时不需要", "再看看", "timing_not_right"],
        ),
        (
            "authority",
            "决策权异议",
            "需要老板 / 团队 / 其他决策人参与，不能独自拍板。",
            &["决策权不足", "需要请示", "authority_missing"],
        ),
        (
            "product_fit",
            "产品适配异议",
            "对产品功能、能力、覆盖范围、行业适配性表达匹配度顾虑。",
            &["产品不匹配", "功能不够", "fit_mismatch"],
        ),
        (
            "risk",
            "风险异议",
            "对实施风险、效果不确定性、交付质量、合规与隐私表达担忧。",
            &["怕风险", "效果不确定", "implementation_risk"],
        ),
        (
            "other",
            "其他异议",
            "未归类的真实顾虑，待运营审核后补入字典或合并到既有维度。",
            &["其他", "未分类"],
        ),
    ];
    for (id, display, desc, aliases) in objection_types {
        out.push(TaxonomyEntry {
            id: None,
            scope: "global".to_string(),
            kind: "objection_type".to_string(),
            value: TaxonomyValue {
                id: (*id).to_string(),
                display_name: (*display).to_string(),
                description: (*desc).to_string(),
                aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
                status: "active".to_string(),
            },
            updated_at: now,
        });
    }

    out
}

/// 2026_05_007（占位 / W0 即可生效）：
/// `agent_send_outbox` 集合的索引在 task 1.2 已被 `Database::ensure_indexes()` 幂等创建。
/// 本迁移是显式 marker，标记"outbox 表的索引契约自此版本起被纳入迁移轨道"，
/// 二次启动时 `migrations` 集合的 `_id` 唯一约束会让本条 skip。
///
/// 注意：真正的索引创建发生在 `ensure_indexes()`，那里本身就是幂等的
/// （`create_indexes` 对已存在的相同索引是 no-op），故本迁移即使在历史数据库
/// 上首次执行也是安全的。
async fn outbox_indexes_placeholder(_db: &Database) -> AppResult<()> {
    tracing::info!(
        migration_id = "2026_05_007_outbox_indexes",
        "scaffold migration applied (no-op marker); outbox indexes are created idempotently in ensure_indexes()"
    );
    Ok(())
}

/// 2026_05_008（M2 Strategic Planner）：把 `contacts.last_commitment: Option<String>`
/// 升级为结构化数组 `commitments: [{ id, text, due_at:null, created_at }]`，并 `$unset`
/// 旧字段。
///
/// 历史 `last_commitment` 是自由文本，没有 due_at；本迁移仅做形态升级，把字符串
/// 包成单元素 Vec，`due_at` 留 null（Planner `scan_commitments` 对 `Plain`/无 due_at
/// 的元素跳过，等下次 Reply Agent 重塑 memoryCard 时由 LLM 给出 due_at）。
///
/// 幂等：filter 要求 `commitments` 不存在，二次执行时该条件不再命中。
async fn contact_commitments_reshape(db: &Database) -> AppResult<()> {
    let now = DateTime::now();
    // 阶段 1：把 last_commitment 非空字符串升级为 commitments 单元素数组
    // （aggregation pipeline 内为每条 contact 算 fresh id 比较麻烦——pipeline 表达式
    // 没有 native UUID。这里用 contact._id.toString() 作为 id 兜底；后续 Reply Agent
    // 重写时会替换成真正的 UUIDv4。）
    let pipeline: Vec<Document> = vec![
        doc! {
            "$set": {
                "commitments": {
                    "$cond": [
                        {
                            "$and": [
                                { "$ne": [{ "$type": "$last_commitment" }, "missing"] },
                                { "$ne": ["$last_commitment", null] },
                                { "$ne": ["$last_commitment", ""] }
                            ]
                        },
                        [{
                            "id": { "$toString": "$_id" },
                            "text": "$last_commitment",
                            "createdAt": { "$ifNull": ["$updated_at", now] }
                        }],
                        []
                    ]
                }
            }
        },
        doc! { "$unset": "last_commitment" },
    ];
    let result = db
        .contacts()
        .update_many(
            doc! { "commitments": { "$exists": false } },
            pipeline,
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_05_008_contact_commitments_reshape",
        modified = result.modified_count,
        matched = result.matched_count,
        "reshaped contacts.last_commitment to commitments array"
    );
    Ok(())
}

/// 2026_05_M4_001（agent-self-evolution / W0 Task 1.5）：把 `prompt_templates`
/// 升级为多版本形态。给所有缺字段的旧文档填：
///   - `current_version`：同 `(workspace_id, prompt_key)` 下 `version` 最大且
///     `status="active"` 的一条置 `true`，其余置 `false`；同 prompt_key 下
///     若没有 active，按 version 最大兜底（`status="draft"` 也算）。
///   - `previous_version: null`（rollback 不能跨迁移点）。
///   - `seeded_by: "legacy_migration"`（来源标记）。
///
/// 幂等：filter 命中"`current_version` 字段不存在"的文档；二次启动时所有文档
/// 都已具备字段，filter 不再命中。`migrations` 集合的版本记录是双保险。
async fn prompt_template_versioned(db: &Database) -> AppResult<()> {
    let coll = db
        .raw()
        .collection::<Document>("prompt_templates");
    // 阶段 1：给所有缺 `current_version` 字段的文档填占位（先全部置 false /
    // previous_version=null / seeded_by=legacy_migration）。
    let stage1 = coll
        .update_many(
            doc! { "current_version": { "$exists": false } },
            doc! {
                "$set": {
                    "current_version": false,
                    "previous_version": Bson::Null,
                    "seeded_by": "legacy_migration",
                }
            },
            None,
        )
        .await?;

    // 阶段 2：按 (workspace_id, prompt_key) 分组，挑 status=active 中 version
    // 最大的一条置 current_version=true；若整组无 active，则 version 最大且
    // status 任意（draft / archived 也算）的一条置 true（兜底——避免某些 prompt_key
    // 全部历史都是 draft 时无 current 可读）。
    let mut cursor = coll
        .aggregate(
            vec![
                doc! {
                    "$match": {
                        "current_version": false,
                        "status": { "$ne": "archived" },
                    }
                },
                doc! {
                    "$sort": {
                        "workspace_id": 1,
                        "prompt_key": 1,
                        "version": -1,
                    }
                },
                doc! {
                    "$group": {
                        "_id": { "workspace_id": "$workspace_id", "prompt_key": "$prompt_key" },
                        "active_id": {
                            "$first": {
                                "$cond": [{ "$eq": ["$status", "active"] }, "$_id", Bson::Null]
                            }
                        },
                        "fallback_id": { "$first": "$_id" },
                    }
                },
            ],
            None,
        )
        .await?;
    let mut promoted: u64 = 0;
    while let Some(group) = cursor.try_next().await? {
        let target_id = group
            .get("active_id")
            .and_then(|b| b.as_object_id())
            .or_else(|| group.get("fallback_id").and_then(|b| b.as_object_id()));
        let Some(target) = target_id else {
            continue;
        };
        let result = coll
            .update_one(
                doc! { "_id": target },
                doc! { "$set": { "current_version": true } },
                None,
            )
            .await?;
        promoted += result.modified_count;
    }

    tracing::info!(
        migration_id = "2026_05_M4_001_prompt_template_versioned",
        backfilled = stage1.modified_count,
        promoted_current = promoted,
        "upgraded prompt_templates to multi-version layout"
    );
    Ok(())
}

/// 2026_05_010：联系人 custom_agent_instructions + 知识库三集合标签字段回填。
///
/// 给 `contacts` 加 `custom_agent_instructions: null`（上限 1000 字符的运营人员
/// 特别指令；后台 PUT 维护，作为 Operator Instruction 层注入 user.reply prompt）。
///
/// 给 `operation_knowledge_documents` / `_items` / `_chunks` 三集合加：
///   - `product_tags: []`（≤5）
///   - `trigger_keywords: []`（≤8，全小写，运行时关键词快路径用 `inbound.contains`）
///   - `business_topics: []`（≤3）
///
/// 幂等：每个 filter 用 `$exists: false` 仅命中未升级文档；二次启动不变更。
async fn contact_custom_instructions_and_knowledge_tags(db: &Database) -> AppResult<()> {
    let contacts_result = db
        .contacts()
        .update_many(
            doc! { "custom_agent_instructions": { "$exists": false } },
            doc! { "$set": { "custom_agent_instructions": null } },
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_05_V3_001_contact_custom_instructions_and_knowledge_tags",
        contacts_modified = contacts_result.modified_count,
        contacts_matched = contacts_result.matched_count,
        "backfilled contacts.custom_agent_instructions"
    );

    // 三集合做同样的标签字段回填（同时检查三字段任一缺失即触发，但只 set 缺的字段
    // 会让 update 复杂化，统一 set 三字段更幂等）。
    let collections: [(&str, &mongodb::Collection<Document>); 3] = [
        (
            "operation_knowledge_documents",
            &db.raw().collection::<Document>("operation_knowledge_documents"),
        ),
        (
            "operation_knowledge_items",
            &db.raw().collection::<Document>("operation_knowledge_items"),
        ),
        (
            "operation_knowledge_chunks",
            &db.raw().collection::<Document>("operation_knowledge_chunks"),
        ),
    ];
    for (coll_name, coll) in collections {
        let result = coll
            .update_many(
                doc! {
                    "$or": [
                        { "product_tags": { "$exists": false } },
                        { "trigger_keywords": { "$exists": false } },
                        { "business_topics": { "$exists": false } },
                    ]
                },
                doc! {
                    "$set": {
                        "product_tags": [],
                        "trigger_keywords": [],
                        "business_topics": [],
                    }
                },
                None,
            )
            .await?;
        tracing::info!(
            migration_id = "2026_05_V3_001_contact_custom_instructions_and_knowledge_tags",
            collection = coll_name,
            modified = result.modified_count,
            matched = result.matched_count,
            "backfilled knowledge tag fields"
        );
    }
    Ok(())
}

/// knowledge-base cleanup（推倒销售话术 RAG 形态）：清空旧三层集合的存量数据。
///
/// 开发期数据无价值，无需兼容；本迁移仅清空文档，集合本身保留。
/// 集合 `operation_knowledge_items` 在 commit 2 移除 typed accessor；这里同步 delete_many
/// 让所有环境的存量销售域文档归零。
///
/// 生产环境守卫：`APP_ENV=production` 时直接报错阻断，避免误删。
///
/// 幂等：所有 delete_many 都是按 `{}` 全量删；二次执行 matched=0 即可。
async fn drop_legacy_sales_collections(db: &Database) -> AppResult<()> {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        return Err(crate::error::AppError::External(
            "禁止在 production 环境执行 cleanup migration: drop_legacy_sales_collections".into(),
        ));
    }
    let raw = db.raw();
    for name in [
        "operation_knowledge_items",
        "operation_knowledge_documents",
        "operation_knowledge_chunks",
    ] {
        let coll = raw.collection::<Document>(name);
        let result = coll.delete_many(doc! {}, None).await?;
        tracing::info!(
            migration_id = "2026_05_25_drop_legacy_sales_collections",
            collection = name,
            deleted = result.deleted_count,
            "cleared legacy knowledge collection"
        );
    }
    Ok(())
}

/// knowledge-base cleanup：清空 `system_taxonomies` 中的销售域三 kind seed
/// （`customer_stage` / `intent_level` / `objection_type`），让用户在 admin 通过
/// DomainSchema + 自定义 taxonomy 自配。
///
/// 集合本身保留，仅删销售域 seed。其它 kind（如 evolution-related）不受影响。
/// 幂等：filter 命中即删，二次执行 matched=0。
async fn drop_legacy_taxonomy_seed(db: &Database) -> AppResult<()> {
    if std::env::var("APP_ENV").unwrap_or_default() == "production" {
        return Err(crate::error::AppError::External(
            "禁止在 production 环境执行 cleanup migration: drop_legacy_taxonomy_seed".into(),
        ));
    }
    let result = db
        .collection_system_taxonomies()
        .delete_many(
            doc! { "kind": { "$in": ["customer_stage", "intent_level", "objection_type"] } },
            None,
        )
        .await?;
    tracing::info!(
        migration_id = "2026_05_25_drop_legacy_taxonomy_seed",
        deleted = result.deleted_count,
        "cleared legacy sales-domain taxonomy seeds"
    );
    Ok(())
}

/// 入口函数：扫描 `migrations` 集合，按顺序执行未应用的迁移。
pub async fn run(db: &Database) -> AppResult<()> {
    run_with(db, MIGRATIONS).await
}

/// 测试友好的内部入口：允许传入自定义迁移列表，用于单元测试和快照重放。
pub async fn run_with(db: &Database, migrations: &[Migration]) -> AppResult<()> {
    let collection = db.migrations();
    for migration in migrations {
        let existing = collection
            .find_one(doc! { "_id": migration.id }, None)
            .await?;
        if existing.is_some() {
            tracing::debug!(
                migration_id = migration.id,
                "migration already applied, skipping"
            );
            continue;
        }
        tracing::info!(migration_id = migration.id, "applying migration");
        (migration.run)(db).await?;
        let record = MigrationRecord {
            id: migration.id.to_string(),
            applied_at: DateTime::now(),
        };
        collection.insert_one(record, None).await?;
        tracing::info!(migration_id = migration.id, "migration applied");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_ids_are_unique() {
        let mut ids: Vec<&str> = MIGRATIONS.iter().map(|m| m.id).collect();
        let original_len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(
            ids.len(),
            original_len,
            "migration ids must be unique; duplicates: {:?}",
            ids
        );
    }

    #[test]
    fn migration_ids_are_chronologically_ordered() {
        for window in MIGRATIONS.windows(2) {
            assert!(
                window[0].id < window[1].id,
                "migrations must be in id order: {} should come before {}",
                window[0].id,
                window[1].id
            );
        }
    }

    /// 波 D2：`merge_allowed_from_defaults` 只补缺失字段，不覆盖运营人员
    /// 已经写过的 `allowedFrom` / `allowFromAny` / `name` / `goal` 等。
    #[test]
    fn merge_allowed_from_does_not_overwrite_user_values() {
        let defaults = vec![
            doc! {
                "key": "new_contact",
                "allowedFrom": ["new_contact"]
            },
            doc! {
                "key": "cooldown",
                "allowedFrom": [],
                "allowFromAny": true
            },
        ];
        let mut machine = doc! {
            "states": [
                {
                    "key": "new_contact",
                    // 运营人员手动改了名字 + allowedFrom，不应被覆盖。
                    "name": "客户运营改名",
                    "allowedFrom": ["custom_state"]
                },
                {
                    "key": "cooldown",
                    "name": "我的冷却"
                    // 缺 allowedFrom + allowFromAny，应被补齐。
                }
            ]
        };
        let changed = merge_allowed_from_defaults(&mut machine, &defaults);
        assert!(changed, "应该有补齐字段");

        let states = machine.get_array("states").unwrap();
        // new_contact 的自定义 allowedFrom 不应被改动。
        let nc = states[0].as_document().unwrap();
        assert_eq!(nc.get_str("name").unwrap(), "客户运营改名");
        assert_eq!(
            nc.get_array("allowedFrom").unwrap()[0].as_str(),
            Some("custom_state")
        );
        // cooldown 的字段应被补齐。
        let cd = states[1].as_document().unwrap();
        assert_eq!(cd.get_str("name").unwrap(), "我的冷却");
        assert!(cd.contains_key("allowedFrom"));
        assert_eq!(cd.get_bool("allowFromAny").unwrap(), true);
    }

    /// 波 D2：默认状态完全包含、用户没改时也不会做无意义写入。
    #[test]
    fn merge_allowed_from_skips_when_already_complete() {
        let defaults = vec![doc! {
            "key": "new_contact",
            "allowedFrom": ["new_contact"]
        }];
        let mut machine = doc! {
            "states": [
                {
                    "key": "new_contact",
                    "allowedFrom": ["new_contact"]
                }
            ]
        };
        let changed = merge_allowed_from_defaults(&mut machine, &defaults);
        assert!(
            !changed,
            "无需改动时返回 false，避免对 mongo 做空 update"
        );
    }

    /// W3 / Task 4.9：默认 taxonomy seed 必须覆盖三个 kind 各自的最小集合，
    /// 且每条 entry 满足后续 R8 业务约束（scope=global、value.id 与 status 非空、
    /// status="active"）。这保证 `seed_default_taxonomies` 写入的数据下游可读。
    #[test]
    fn default_taxonomy_seed_entries_have_required_kinds() {
        let now = DateTime::now();
        let entries = default_taxonomy_seed_entries(now);

        let stages: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "customer_stage")
            .map(|e| e.value.id.as_str())
            .collect();
        let intents: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "intent_level")
            .map(|e| e.value.id.as_str())
            .collect();
        let objections: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == "objection_type")
            .map(|e| e.value.id.as_str())
            .collect();

        // customer_stage：与 default_user_operation_state_machine 的 9 个 state
        // key 一一对应（保证 R8 字典与状态机字典源自同一份基线）。
        assert!(
            stages.contains(&"new_contact"),
            "customer_stage 必须包含 new_contact，stages: {:?}",
            stages
        );
        assert!(stages.contains(&"need_discovery"));
        assert!(stages.contains(&"objection_handling"));
        assert!(stages.contains(&"customer_success"));
        assert_eq!(stages.len(), 9, "9 个状态 key 一一对应：{:?}", stages);

        // intent_level：high / medium / low 三档。
        assert_eq!(intents.len(), 3);
        assert!(intents.contains(&"high"));
        assert!(intents.contains(&"medium"));
        assert!(intents.contains(&"low"));

        // objection_type：至少 5 项 + other 兜底。
        assert!(objections.len() >= 6);
        assert!(objections.contains(&"price"));
        assert!(objections.contains(&"trust"));
        assert!(objections.contains(&"other"));

        // 每条 entry 必须满足 R8 字典的最小完整性约束：
        // scope == "global"；value.id / value.display_name / value.status 非空；
        // status == "active"（seed 默认全部启用，运营后续可改 deprecated）。
        for entry in &entries {
            assert_eq!(entry.scope, "global", "seed 默认 scope=global");
            assert!(
                !entry.value.id.is_empty(),
                "value.id 不可空：{:?}",
                entry.value
            );
            assert!(
                !entry.value.display_name.is_empty(),
                "displayName 不可空：{:?}",
                entry.value
            );
            assert_eq!(entry.value.status, "active");
        }
    }

    /// W3 / Task 4.9：seed 数据不能在 `(scope, kind, value.id)` 维度上重复，
    /// 否则会触发 `system_taxonomies` 唯一索引冲突或导致幂等行为不一致。
    #[test]
    fn default_taxonomy_seed_entries_are_unique_by_scope_kind_id() {
        let now = DateTime::now();
        let entries = default_taxonomy_seed_entries(now);
        let mut keys: Vec<(String, String, String)> = entries
            .iter()
            .map(|e| (e.scope.clone(), e.kind.clone(), e.value.id.clone()))
            .collect();
        let original = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(
            keys.len(),
            original,
            "seed entries 必须按 (scope, kind, value.id) 唯一"
        );
    }

    /// W3 / Task 4.9：alias 中的中文阶段名（"陌生接触 / 初步信任 / 需求探索 /
    /// 方案评估 / 异议处理 / 成交推进 / 交付维护 / 复购转介绍"）来自 prompts.rs
    /// `stage_method` 文案，必须能命中 customer_stage 字典的某条 alias，从而保
    /// 证升级前 contact 上的中文 customer_stage 字段下游能 alias 命中。
    #[test]
    fn customer_stage_aliases_cover_legacy_chinese_terms() {
        let now = DateTime::now();
        let entries = default_taxonomy_seed_entries(now);

        let collect_aliases = |entries: &[TaxonomyEntry]| -> Vec<String> {
            entries
                .iter()
                .filter(|e| e.kind == "customer_stage")
                .flat_map(|e| e.value.aliases.clone())
                .collect()
        };

        let aliases = collect_aliases(&entries);
        // prompts.rs::default_playbook 中 stage_method 提到的 8 个中文阶段中
        // 至少应有 6 个能在 alias 集合中命中（剩余如 "需求探索 / 异议处理"
        // 直接是 displayName 而非 alias，由 displayName 兜底命中）。
        let must_have = [
            "陌生接触",
            "初步信任",
            "方案评估",
            "成交推进",
            "交付维护",
            "复购转介绍",
        ];
        for term in must_have {
            assert!(
                aliases.iter().any(|a| a == term),
                "customer_stage alias 集合应包含 \"{}\" 以兼容历史 contact 中文取值；\
                 当前 alias 列表：{:?}",
                term,
                aliases
            );
        }
    }
}
