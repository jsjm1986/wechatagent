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

/// universal-domain-adaptation H2：参与决策校验的维度集合不再写死，由调用方传入
/// `decision_dimension_kinds(active_profile)`。DEFAULT 销售域返回
/// `["customer_stage","intent_level"]`，逐字等价改造前的 `TAGGED_FIELDS` const。
///
/// 取值读写经 [`super::domain_signals::get_dimension`] / `set_dimension`：销售域两维
/// 走 typed 字段，其它行业维度走 `domain_signals` 容器。

/// 纯函数：对 [`AgentDecision`] 中 `dimension_kinds` 列出的每个维度做 4 路分支。
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
    dimension_kinds: &[String],
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut risks: Vec<String> = Vec::new();
    let mut candidates: Vec<(String, String)> = Vec::new();
    for kind in dimension_kinds {
        let raw = match super::domain_signals::get_dimension(decision, kind) {
            Some(v) if !v.trim().is_empty() => v.to_string(),
            _ => continue,
        };
        match check_value(kind, &raw, scope_account_id, cache) {
            TaxonomyMatch::Active => {}
            TaxonomyMatch::AliasActive(canonical) => {
                if canonical != raw {
                    super::domain_signals::set_dimension(decision, kind, canonical);
                }
            }
            TaxonomyMatch::Deprecated => {
                risks.push(format!("taxonomy_deprecated_value:{kind}:{raw}"));
            }
            TaxonomyMatch::CandidateNew => {
                risks.push(format!("taxonomy_candidate:{kind}:{raw}"));
                candidates.push((kind.clone(), raw));
            }
        }
    }
    (risks, candidates)
}

/// 生产入口：调 [`classify_decision_tags`] 拿 risks/candidates，candidate 列表
/// 通过 `tokio::spawn` 异步 upsert（best-effort，失败仅 warn）。
///
/// `dimension_kinds` 由调用方从 active DomainProfile 取（`decision_dimension_kinds`）。
/// 仅要求 tokio runtime 在场（webhook / worker 入口都满足）。返回 risks 由调用方
/// append 到 `promote_risks`。
pub(crate) fn validate_and_normalize_decision(
    db: &Database,
    decision: &mut AgentDecision,
    dimension_kinds: &[String],
    scope_account_id: &str,
) -> Vec<String> {
    let cache = global_taxonomy_cache();
    let (risks, candidates) =
        classify_decision_tags(decision, dimension_kinds, scope_account_id, &cache);
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
/// 默认用销售域两维（与 DEFAULT profile 的 `decision_dimension_kinds` 一致）。
#[cfg(test)]
pub(crate) fn classify_with_cache_for_tests(
    decision: &mut AgentDecision,
    scope_account_id: &str,
    cache: &std::sync::Arc<TaxonomyCache>,
) -> (Vec<String>, Vec<(String, String)>) {
    let dims = vec!["customer_stage".to_string(), "intent_level".to_string()];
    classify_decision_tags(decision, &dims, scope_account_id, cache)
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
                priority_weight: None,
                is_terminal: false,
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

    #[test]
    fn non_sales_dimension_from_container_is_classified_and_rewritten() {
        // universal-domain-adaptation H2：非销售域维度（如陪伴域 relationship_closeness）
        // 不是 typed 字段，取值落在 domain_signals 容器。传入含该维度的 dimension_kinds
        // 后，classify 应能从容器读取、命中 alias 时改写回容器。
        let cache = mk_cache(vec![mk_entry(
            "global",
            "relationship_closeness",
            "intimate",
            "active",
            &["亲密期", "热恋"],
        )]);
        let mut d = AgentDecision::default();
        d.domain_signals
            .insert("relationship_closeness".to_string(), "热恋".to_string());
        let dims = vec!["relationship_closeness".to_string()];
        let (risks, cands) = classify_decision_tags(&mut d, &dims, "acct-x", &cache);

        assert_eq!(
            d.domain_signals.get_str("relationship_closeness").ok(),
            Some("intimate"),
            "容器维度的 alias 应被改写为 canonical id"
        );
        assert!(risks.is_empty() && cands.is_empty());
    }

    #[test]
    fn non_sales_dimension_unknown_emits_candidate() {
        // 容器维度的未知值也应进候选队列（与 typed 维度一致的软门语义）。
        let cache = mk_cache(vec![mk_entry(
            "global",
            "emotional_state",
            "calm",
            "active",
            &[],
        )]);
        let mut d = AgentDecision::default();
        d.domain_signals
            .insert("emotional_state".to_string(), "焦虑不安".to_string());
        let dims = vec!["emotional_state".to_string()];
        let (risks, cands) = classify_decision_tags(&mut d, &dims, "acct-x", &cache);

        assert_eq!(risks, vec!["taxonomy_candidate:emotional_state:焦虑不安"]);
        assert_eq!(
            cands,
            vec![("emotional_state".to_string(), "焦虑不安".to_string())]
        );
    }
}
