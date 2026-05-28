//! Phase A / A3 收口（路线图欠账）：把 `taxonomy::check_value` 接进 user-ops
//! 决策路径。
//!
//! 调用时机：[`super::decision::decide_reply_with_promote`] 拿到 `AgentDecision`
//! 之后、return 给 gateway 之前。这样：
//!
//! - **alias 改写发生在 reviewer 看到决策之前**，reviewer 拿到的是 canonical
//!   id（`新客户` → `first_contact` 等），评分稳定可比对；
//! - **CandidateNew / Deprecated 通过 risks 通道下游消费**，与 promote_risks /
//!   `validate_and_promote` 保持同一管线，不需要新增写盘字段；
//! - **不阻塞决策**：候选 upsert 是 `tokio::spawn` 异步的（fire-and-forget），
//!   `taxonomy.rs:9` "候选 SHALL NOT 阻塞 Reply Agent" 硬约束。
//!
//! 校验维度：固定先做 `customer_stage` / `intent_level` 两项（LLM 已稳定输出，
//! 字典已注入）。新增 `objection_type` / 自定义 `kind` 时只动本文件的
//! [`TAGGED_FIELDS`] 表。
//!
//! 设计拆分：
//! - [`classify_decision_tags`] 是纯函数：拿 cache + decision，**只**做 4 路分支
//!   + alias 写回 + risks/candidate 收集，不触 db、不 spawn。便于离线 PBT/单测。
//! - [`validate_and_normalize_decision`] 是生产入口：拿 db 句柄，调上面纯函数后
//!   把 candidate 列表 fire-and-forget upsert。

use crate::agent::taxonomy::{
    check_value, global_taxonomy_cache, upsert_candidate, TaxonomyCache, TaxonomyMatch,
};
use crate::agent::types::AgentDecision;
use crate::db::Database;

/// Reply Agent 决策内需要校验的标签字段。新增维度仅扩展本表。
///
/// 元组：`(kind 名称, getter, setter)`。`getter` 取当前值；`setter` 在 alias 命中
/// 时写回 canonical id。`kind` 必须与 `system_taxonomies.kind` 中的 snake_case
/// 字段名一致（`customer_stage` / `intent_level` / 后续 `objection_type`）。
type Getter = fn(&AgentDecision) -> Option<&str>;
type Setter = fn(&mut AgentDecision, String);

const TAGGED_FIELDS: &[(&str, Getter, Setter)] = &[
    (
        "customer_stage",
        |d| d.customer_stage.as_deref(),
        |d, v| d.customer_stage = Some(v),
    ),
    (
        "intent_level",
        |d| d.intent_level.as_deref(),
        |d, v| d.intent_level = Some(v),
    ),
];

/// 纯函数：对 [`AgentDecision`] 中标签字段做 4 路分支。
///
/// - `Active` → 不动；
/// - `AliasActive(canonical)` → 写回 canonical id（reviewer 看到 canonical）；
/// - `Deprecated` → push `taxonomy_deprecated_value:<kind>:<value>` risk；
/// - `CandidateNew` → push `taxonomy_candidate:<kind>:<value>` risk + 收集到
///   待 upsert 列表（由生产入口 [`validate_and_normalize_decision`] 异步落库）。
///
/// 返回 `(risks, candidates)`。**不**触发 review fail —— 候选偏离是软门。
pub(crate) fn classify_decision_tags(
    decision: &mut AgentDecision,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut risks: Vec<String> = Vec::new();
    let mut candidates: Vec<(String, String)> = Vec::new();
    for (kind, get, set) in TAGGED_FIELDS {
        let raw = match get(decision) {
            Some(v) if !v.trim().is_empty() => v.to_string(),
            _ => continue,
        };
        match check_value(kind, &raw, scope_account_id, cache) {
            TaxonomyMatch::Active => {}
            TaxonomyMatch::AliasActive(canonical) => {
                if canonical != raw {
                    set(decision, canonical);
                }
            }
            TaxonomyMatch::Deprecated => {
                risks.push(format!("taxonomy_deprecated_value:{kind}:{raw}"));
            }
            TaxonomyMatch::CandidateNew => {
                risks.push(format!("taxonomy_candidate:{kind}:{raw}"));
                candidates.push(((*kind).to_string(), raw));
            }
        }
    }
    (risks, candidates)
}

/// 生产入口：调 [`classify_decision_tags`] 拿 risks/candidates，candidate 列表
/// 通过 `tokio::spawn` 异步 upsert（best-effort，失败仅 warn）。
///
/// 仅要求 tokio runtime 在场（webhook / worker 入口都满足）。返回 risks 由调用方
/// append 到 `promote_risks`。
pub(crate) fn validate_and_normalize_decision(
    db: &Database,
    decision: &mut AgentDecision,
    scope_account_id: &str,
) -> Vec<String> {
    let cache = global_taxonomy_cache();
    let (risks, candidates) = classify_decision_tags(decision, scope_account_id, &cache);
    spawn_candidate_upserts(db, scope_account_id, candidates);
    risks
}

/// 把 `candidates` 列表 fire-and-forget 写盘。抽到独立函数便于未来加入熔断 /
/// 限流策略（例如同一 contact 一轮内 candidate 暴量时降级）。
fn spawn_candidate_upserts(
    db: &Database,
    scope_account_id: &str,
    candidates: Vec<(String, String)>,
) {
    if candidates.is_empty() {
        return;
    }
    let db = db.clone();
    let scope = scope_account_id.to_string();
    tokio::spawn(async move {
        for (kind, raw) in candidates {
            if let Err(err) =
                upsert_candidate(&db, &scope, &kind, &raw, None, 0).await
            {
                tracing::warn!(
                    kind = %kind,
                    raw_value = %raw,
                    ?err,
                    "taxonomy candidate upsert failed (best-effort)"
                );
            }
        }
    });
}

/// 测试入口：手动注入 cache（绕过全局单例 + db），便于 PBT。
#[cfg(test)]
pub(crate) fn classify_with_cache_for_tests(
    decision: &mut AgentDecision,
    scope_account_id: &str,
    cache: &std::sync::Arc<TaxonomyCache>,
) -> (Vec<String>, Vec<(String, String)>) {
    classify_decision_tags(decision, scope_account_id, cache)
}

#[cfg(test)]
mod tests {
    //! 纯单元：覆盖 4 路分支 + alias 改写 + 幂等。upsert 的写盘行为已在
    //! `taxonomy.rs` 单测（596-700）用 testcontainers 跑过，本文件不重复。

    use super::*;
    use crate::agent::taxonomy::taxonomy_cache_for_tests;
    use crate::agent::types::AgentDecision;
    use crate::models::{TaxonomyEntry, TaxonomyValue};
    use mongodb::bson::{oid::ObjectId, DateTime};
    use std::sync::Arc;

    fn mk_cache(entries: Vec<TaxonomyEntry>) -> Arc<TaxonomyCache> {
        Arc::new(taxonomy_cache_for_tests(entries))
    }

    fn mk_entry(
        scope: &str,
        kind: &str,
        canonical: &str,
        status: &str,
        aliases: &[&str],
    ) -> TaxonomyEntry {
        TaxonomyEntry {
            id: Some(ObjectId::new()),
            scope: scope.to_string(),
            kind: kind.to_string(),
            value: TaxonomyValue {
                id: canonical.to_string(),
                display_name: canonical.to_string(),
                description: String::new(),
                aliases: aliases.iter().map(|s| s.to_string()).collect(),
                status: status.to_string(),
            },
            updated_at: DateTime::now(),
            version: 1,
            current_version: true,
            previous_version: None,
            seeded_by: None,
        }
    }

    #[test]
    fn active_canonical_passthrough_no_risk_no_candidate() {
        let cache = mk_cache(vec![mk_entry(
            "global",
            "customer_stage",
            "first_contact",
            "active",
            &[],
        )]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("first_contact".to_string());
        let (risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert_eq!(d.customer_stage.as_deref(), Some("first_contact"));
        assert!(risks.is_empty() && cands.is_empty());
    }

    #[test]
    fn alias_active_rewrites_to_canonical_no_risk() {
        let cache = mk_cache(vec![mk_entry(
            "global",
            "customer_stage",
            "first_contact",
            "active",
            &["新客户", "首次接触"],
        )]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("新客户".to_string());
        let (risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert_eq!(
            d.customer_stage.as_deref(),
            Some("first_contact"),
            "alias 应在 reviewer 看到决策之前被改写为 canonical id"
        );
        assert!(risks.is_empty() && cands.is_empty());
    }

    #[test]
    fn deprecated_pushes_risk_no_rewrite_no_candidate() {
        let cache = mk_cache(vec![mk_entry(
            "global",
            "customer_stage",
            "old_stage_x",
            "deprecated",
            &[],
        )]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("old_stage_x".to_string());
        let (risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert_eq!(d.customer_stage.as_deref(), Some("old_stage_x"));
        assert_eq!(
            risks,
            vec!["taxonomy_deprecated_value:customer_stage:old_stage_x"]
        );
        assert!(cands.is_empty(), "deprecated 不入 candidate 列表");
    }

    #[test]
    fn candidate_new_unknown_pushes_risk_and_candidate_no_rewrite() {
        let cache = mk_cache(vec![mk_entry(
            "global",
            "customer_stage",
            "first_contact",
            "active",
            &[],
        )]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("价格异议中段".to_string());
        let (risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert_eq!(d.customer_stage.as_deref(), Some("价格异议中段"));
        assert_eq!(
            risks,
            vec!["taxonomy_candidate:customer_stage:价格异议中段"]
        );
        assert_eq!(
            cands,
            vec![("customer_stage".to_string(), "价格异议中段".to_string())]
        );
    }

    #[test]
    fn empty_or_missing_field_is_skipped() {
        let cache = mk_cache(vec![]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some(String::new());
        d.intent_level = None;
        let (risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert!(risks.is_empty() && cands.is_empty());
    }

    #[test]
    fn alias_rewrite_is_idempotent() {
        // 同一决策反复跑：第一次改写 alias→canonical；第二次拿 canonical 直接 Active 通过。
        let cache = mk_cache(vec![mk_entry(
            "global",
            "customer_stage",
            "first_contact",
            "active",
            &["新客户"],
        )]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("新客户".to_string());
        let (r1, c1) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        let snap = d.customer_stage.clone();
        let (r2, c2) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert_eq!(snap.as_deref(), Some("first_contact"));
        assert_eq!(d.customer_stage.as_deref(), Some("first_contact"));
        assert!(r1.is_empty() && r2.is_empty(), "幂等不应累积 risks");
        assert!(c1.is_empty() && c2.is_empty(), "alias 命中不入 candidate");
    }

    #[test]
    fn account_scope_overrides_global_when_active() {
        // account 私有字典优先于 global，且 active 标签直接通过。
        let cache = mk_cache(vec![
            mk_entry("global", "customer_stage", "first_contact", "active", &[]),
            mk_entry(
                "acct-special",
                "customer_stage",
                "vip_lead",
                "active",
                &[],
            ),
        ]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("vip_lead".to_string());
        let (risks, _) = classify_with_cache_for_tests(&mut d, "acct-special", &cache);
        assert!(risks.is_empty(), "account 私有字典命中不应产 risk");
    }

    #[test]
    fn intent_level_unknown_also_emits_candidate_risk() {
        // 多字段同时校验：customer_stage active + intent_level 未知。
        let cache = mk_cache(vec![mk_entry(
            "global",
            "customer_stage",
            "first_contact",
            "active",
            &[],
        )]);
        let mut d = AgentDecision::default();
        d.customer_stage = Some("first_contact".to_string());
        d.intent_level = Some("unicorn_tier".to_string());
        let (risks, cands) = classify_with_cache_for_tests(&mut d, "acct-x", &cache);
        assert_eq!(
            risks,
            vec!["taxonomy_candidate:intent_level:unicorn_tier"]
        );
        assert_eq!(
            cands,
            vec![("intent_level".to_string(), "unicorn_tier".to_string())]
        );
    }
}
