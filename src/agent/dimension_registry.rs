//! universal-domain-adaptation 主题1：画像维度元数据单一真相源。
//!
//! 收敛此前散落 ≥7 处的维度判断（domain_signals.rs `KNOWN_TYPED_DIMS`、
//! domain_profile.rs `SALES_TYPED_DIMENSION_KINDS`、m020/m021/m023/m024 migration
//! 散文注释、entitlements kind 常量）到一张 const 表。
//!
//! 分工：registry 描述维度的**结构属性**（通道/typed/是否参与决策/取值约束类型）——
//! 这是编译期代码契约；维度的**具体合法取值**仍在 system_taxonomies DB 字典
//! （因行业而异、运营可增删）。registry 只声明"取值要不要查字典校验"。

/// 维度写入通道。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DimensionChannel {
    /// LLM domainSignals 容器（customer_stage/intent_level/purchase_lifecycle/churn_reason）。
    LlmSignals,
    /// admin 直写 domain_attributes（relationship_type）。
    AdminDirect,
    /// gateway 规则派生直写（value_tier）。
    GatewayDerived,
    /// reaction 分析派生（objection_type，第四条隐性通道，强制显式化）。
    ReactionDerived,
}

/// 取值约束来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueSource {
    /// 查 system_taxonomies 字典做 enum 校验。
    Taxonomy,
    /// 值由代码产出、信任，不校验（value_tier ← classify_value_tier）。
    CodeEnum,
    /// 直通无约束。
    FreeText,
}

/// 单个画像维度的结构契约。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DimensionSpec {
    pub kind: &'static str,
    pub channel: DimensionChannel,
    pub typed: bool,
    pub participates_in_decision: bool,
    pub value_source: ValueSource,
}

use DimensionChannel::*;
use ValueSource::*;

/// 维度元数据单一真相源。新增维度在此加一行。
pub(crate) const DIMENSION_REGISTRY: &[DimensionSpec] = &[
    DimensionSpec { kind: "customer_stage", channel: LlmSignals, typed: true, participates_in_decision: true, value_source: Taxonomy },
    DimensionSpec { kind: "intent_level", channel: LlmSignals, typed: true, participates_in_decision: true, value_source: Taxonomy },
    DimensionSpec { kind: "purchase_lifecycle", channel: LlmSignals, typed: false, participates_in_decision: true, value_source: Taxonomy },
    DimensionSpec { kind: "churn_reason", channel: LlmSignals, typed: false, participates_in_decision: true, value_source: Taxonomy },
    DimensionSpec { kind: "value_tier", channel: GatewayDerived, typed: false, participates_in_decision: false, value_source: CodeEnum },
    DimensionSpec { kind: "relationship_type", channel: AdminDirect, typed: false, participates_in_decision: false, value_source: Taxonomy },
    DimensionSpec { kind: "objection_type", channel: ReactionDerived, typed: false, participates_in_decision: false, value_source: Taxonomy },
];

/// 查某维度的契约；未知维度返回 None。
pub(crate) fn spec_for(kind: &str) -> Option<&'static DimensionSpec> {
    DIMENSION_REGISTRY.iter().find(|s| s.kind == kind)
}

/// 派生：所有 typed 维度的 kind（替代硬编码 KNOWN_TYPED_DIMS / SALES_TYPED_DIMENSION_KINDS）。
pub(crate) fn typed_dimension_kinds() -> Vec<&'static str> {
    DIMENSION_REGISTRY.iter().filter(|s| s.typed).map(|s| s.kind).collect()
}

/// 维度取值校验结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DimValidation {
    /// 通过；携带归一后的写入值（alias 归一后为 canonical id，否则为 trim 后原值）。
    Accept(String),
    /// 拒绝（admin 直写通道越界）；携带给人看的错误说明。
    Reject(String),
    /// 静默丢弃越界值（机器产出通道），不阻断已发送回复，调用方写审计。
    DropSilently,
}

/// 字典查询结果（把 taxonomy::check_value 四变体窄化为校验所需三态）。
#[derive(Debug, Clone)]
pub(crate) enum DictLookup {
    /// Active(canonical) 或 Deprecated：字典登记过的合法值。
    Known,
    /// AliasActive → 归一目标 canonical_id。
    Alias(String),
    /// CandidateNew：字典真无（越界）。
    Miss,
}

/// 纯决策：给定维度契约 + 字典查询结果 + 原始值 → 校验结论。无 IO，完全可单测。
pub(crate) fn classify_validation(spec: &DimensionSpec, dict: DictLookup, raw: &str) -> DimValidation {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DimValidation::DropSilently;
    }
    match spec.value_source {
        ValueSource::CodeEnum | ValueSource::FreeText => DimValidation::Accept(trimmed.to_string()),
        ValueSource::Taxonomy => match dict {
            DictLookup::Known => DimValidation::Accept(trimmed.to_string()),
            DictLookup::Alias(canonical) => DimValidation::Accept(canonical),
            DictLookup::Miss => match spec.channel {
                // admin 是人，越界当场报错纠正。
                DimensionChannel::AdminDirect => {
                    DimValidation::Reject(format!("{} 取值 {:?} 不在字典内", spec.kind, trimmed))
                }
                // LLM/reaction 偶发臆造不阻断已发送回复，丢弃 + 留审计（调用方写审计）。
                _ => DimValidation::DropSilently,
            },
        },
    }
}

/// 字典查询薄壳：抄 normalize_dimension_value(:564-566) 取进程级 cache，
/// 把 TaxonomyMatch 四变体映射为 DictLookup 三态。Deprecated 归 Known（合法历史值，不越界）。
async fn lookup_dict(
    db: &crate::db::Database,
    kind: &str,
    trimmed: &str,
    scope_account_id: &str,
) -> DictLookup {
    use crate::agent::taxonomy::{check_value, global_taxonomy_cache, TaxonomyMatch};
    let cache = global_taxonomy_cache();
    cache.find_or_load(db).await;
    match check_value(kind, trimmed, scope_account_id, &cache) {
        TaxonomyMatch::AliasActive(canonical) => DictLookup::Alias(canonical),
        TaxonomyMatch::Active | TaxonomyMatch::Deprecated => DictLookup::Known,
        TaxonomyMatch::CandidateNew => DictLookup::Miss,
    }
}

/// DB 薄壳：查 registry + 字典，委托 classify_validation。未知 kind → 直通信任。
pub(crate) async fn validate_dimension_value(
    db: &crate::db::Database,
    kind: &str,
    raw: &str,
    scope_account_id: &str,
) -> DimValidation {
    let Some(spec) = spec_for(kind) else {
        return DimValidation::Accept(raw.trim().to_string());
    };
    // 非 Taxonomy 源不查字典（Miss 占位，CodeEnum/FreeText 分支不看 dict）。
    let dict = if matches!(spec.value_source, ValueSource::Taxonomy) {
        lookup_dict(db, kind, raw.trim(), scope_account_id).await
    } else {
        DictLookup::Miss
    };
    classify_validation(spec, dict, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_seven_known_dimensions() {
        let kinds: Vec<&str> = DIMENSION_REGISTRY.iter().map(|s| s.kind).collect();
        assert_eq!(kinds.len(), 7);
        for k in [
            "customer_stage", "intent_level", "purchase_lifecycle",
            "churn_reason", "value_tier", "relationship_type", "objection_type",
        ] {
            assert!(kinds.contains(&k), "registry 缺维度 {k}");
        }
    }

    #[test]
    fn typed_dimensions_are_exactly_sales_two() {
        // 收敛护栏：registry 派生的 typed 集合必须逐字等于历史硬编码两维。
        let mut typed = typed_dimension_kinds();
        typed.sort_unstable();
        assert_eq!(typed, vec!["customer_stage", "intent_level"]);
    }

    #[test]
    fn spec_for_known_and_unknown() {
        assert_eq!(spec_for("value_tier").unwrap().channel, DimensionChannel::GatewayDerived);
        assert_eq!(spec_for("value_tier").unwrap().value_source, ValueSource::CodeEnum);
        assert_eq!(spec_for("relationship_type").unwrap().channel, DimensionChannel::AdminDirect);
        assert_eq!(spec_for("objection_type").unwrap().channel, DimensionChannel::ReactionDerived);
        assert!(spec_for("nonexistent").is_none());
    }

    #[test]
    fn registry_kinds_unique() {
        let mut kinds: Vec<&str> = DIMENSION_REGISTRY.iter().map(|s| s.kind).collect();
        let n = kinds.len();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), n, "registry kind 重复");
    }

    #[test]
    fn classify_admin_direct_rejects_out_of_dict() {
        // relationship_type=AdminDirect+Taxonomy：字典真无(CandidateNew) → Reject。
        let spec = spec_for("relationship_type").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "瞎编关系");
        assert!(matches!(r, DimValidation::Reject(_)));
    }

    #[test]
    fn classify_llm_signals_drops_out_of_dict() {
        // customer_stage=LlmSignals+Taxonomy：字典真无(CandidateNew) → DropSilently（不阻断已发送）。
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "臆造态");
        assert!(matches!(r, DimValidation::DropSilently));
    }

    #[test]
    fn classify_alias_normalizes() {
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(spec, DictLookup::Alias("need_discovery".into()), "需求挖掘");
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "need_discovery"));
    }

    #[test]
    fn classify_known_accepts_canonical_and_deprecated() {
        // Known 覆盖 Active(canonical) 与 Deprecated：两者都是字典登记过的合法值 → Accept 原值。
        // 红线：deprecated 是历史合法值，admin 通道也必须 Accept 不得 reject。
        let stage = spec_for("customer_stage").unwrap();
        let r = classify_validation(stage, DictLookup::Known, "need_discovery");
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "need_discovery"));
        let rel = spec_for("relationship_type").unwrap();
        let r2 = classify_validation(rel, DictLookup::Known, "customer");
        assert!(matches!(r2, DimValidation::Accept(ref c) if c == "customer"));
    }

    #[test]
    fn classify_code_enum_trusts() {
        // value_tier=CodeEnum：不查字典直接信任（Miss 占位不影响）。
        let spec = spec_for("value_tier").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "high");
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "high"));
    }

    #[test]
    fn classify_empty_drops() {
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(spec, DictLookup::Known, "  ");
        assert!(matches!(r, DimValidation::DropSilently));
    }
}
