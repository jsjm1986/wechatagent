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
}
