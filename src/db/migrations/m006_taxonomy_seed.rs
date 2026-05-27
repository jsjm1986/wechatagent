//! 2026_05_006（W3 task 4.9 实质实现）：把 prompt 中硬编码的运营术语写入
//! `system_taxonomies`，scope=`global`，作为 `customer_stage / intent_level /
//! objection_type` 三个维度的默认字典。
//!
//! 幂等机制（双层保险）：
//!
//! 1. **migration 框架层**：同一 `migration_id` 在 `migrations` 集合的
//!    `_id` 唯一约束下二次启动会被 `run_with` 直接 skip，不会进到这里。
//! 2. **upsert 层**：即使 migration 记录丢失被强制重跑，本函数对每条
//!    `(scope, kind, value.id)` 走 `update_one + upsert(true)`，依赖
//!    `ensure_system_taxonomies_indexes` 创建的 `(scope, kind, value.id)`
//!    唯一索引保证不会重复插入；`$setOnInsert` 用于不覆盖运营人员后续
//!    通过后台 API 改过的 `displayName / description / aliases / status`
//!    字段，仅在记录不存在时才写入默认值。
//!
//! 数据来源：与 `src/prompts.rs` 中现有 prompt 文案对齐，确保升级后老
//! contact 的 `customer_stage / intent_level` 取值仍能通过 R8 字典校验
//! （或通过 alias 命中升级到 canonical id）。详见 design.md §3.3 与
//! requirements.md R8.8 / R11.7。

use mongodb::bson::{doc, DateTime};
use mongodb::options::UpdateOptions;

use crate::db::Database;
use crate::error::AppResult;
use crate::models::{TaxonomyEntry, TaxonomyValue};

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
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
        let mut doc_to_set = mongodb::bson::to_document(&entry)?;
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
pub(super) fn default_taxonomy_seed_entries(now: DateTime) -> Vec<TaxonomyEntry> {
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

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(
            stages.contains(&"new_contact"),
            "customer_stage 必须包含 new_contact，stages: {:?}",
            stages
        );
        assert!(stages.contains(&"need_discovery"));
        assert!(stages.contains(&"objection_handling"));
        assert!(stages.contains(&"customer_success"));
        assert_eq!(stages.len(), 9, "9 个状态 key 一一对应：{:?}", stages);

        assert_eq!(intents.len(), 3);
        assert!(intents.contains(&"high"));
        assert!(intents.contains(&"medium"));
        assert!(intents.contains(&"low"));

        assert!(objections.len() >= 6);
        assert!(objections.contains(&"price"));
        assert!(objections.contains(&"trust"));
        assert!(objections.contains(&"other"));

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

    #[test]
    fn customer_stage_aliases_cover_legacy_chinese_terms() {
        let now = DateTime::now();
        let entries = default_taxonomy_seed_entries(now);

        let aliases: Vec<String> = entries
            .iter()
            .filter(|e| e.kind == "customer_stage")
            .flat_map(|e| e.value.aliases.clone())
            .collect();

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
