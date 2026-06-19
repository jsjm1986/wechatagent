//! universal-domain-adaptation 主题1：画像维度元数据单一真相源。
//!
//! 收敛此前散落 ≥7 处的维度判断（domain_signals.rs `KNOWN_TYPED_DIMS`、
//! domain_profile.rs `SALES_TYPED_DIMENSION_KINDS`、m020/m021/m023/m024 migration
//! 散文注释、entitlements kind 常量）到一张 const 表。
//!
//! 分工：registry 描述维度的**结构属性**（通道/typed/取值约束类型）——
//! 这是编译期代码契约；维度的**具体合法取值**仍在 system_taxonomies DB 字典
//! （因行业而异、运营可增删）。registry 只声明"取值要不要查字典校验"。
//! 「是否参与决策」是运营可经 UI 改的业务属性，真相源在 DB
//! `ProfileDimension.participates_in_decision`（models.rs），registry 不再固化。

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

/// 写入意图：决定 Taxonomy 越界值的处置倾向，与维度的默认通道正交。
/// admin 是权威直接写入方——越界一律 Reject 当场报错；机器产出按维度通道容错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteIntent {
    AdminWrite,   // admin 直接写入：越界恒 Reject
    MachineWrite, // LLM/reaction 等机器产出：按维度通道（LlmSignals/ReactionDerived→Drop）
}

/// 取值约束来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueSource {
    /// 查 system_taxonomies 字典做 enum 校验。
    Taxonomy,
    /// 值由代码产出、信任，不校验（value_tier ← classify_value_tier）。
    CodeEnum,
    /// 直通无约束。预留第三态：classify_validation 已覆盖（与 CodeEnum 同样 Accept 直通），
    /// 但当前 DIMENSION_REGISTRY 7 维度无一用 FreeText（全 Taxonomy/CodeEnum），故标 allow——
    /// 等未来有"自由文本不查字典"的维度声明即构造，无需改 classify。
    #[allow(dead_code)]
    FreeText,
}

/// 单个画像维度的结构契约。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DimensionSpec {
    pub kind: &'static str,
    pub channel: DimensionChannel,
    pub typed: bool,
    pub value_source: ValueSource,
}

use DimensionChannel::*;
use ValueSource::*;

/// 维度元数据单一真相源。新增维度在此加一行。
pub(crate) const DIMENSION_REGISTRY: &[DimensionSpec] = &[
    DimensionSpec { kind: "customer_stage", channel: LlmSignals, typed: true, value_source: Taxonomy },
    DimensionSpec { kind: "intent_level", channel: LlmSignals, typed: true, value_source: Taxonomy },
    DimensionSpec { kind: "purchase_lifecycle", channel: LlmSignals, typed: false, value_source: Taxonomy },
    DimensionSpec { kind: "churn_reason", channel: LlmSignals, typed: false, value_source: Taxonomy },
    DimensionSpec { kind: "value_tier", channel: GatewayDerived, typed: false, value_source: CodeEnum },
    DimensionSpec { kind: "relationship_type", channel: AdminDirect, typed: false, value_source: Taxonomy },
    DimensionSpec { kind: "objection_type", channel: ReactionDerived, typed: false, value_source: Taxonomy },
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

/// 字典查询结果（把 taxonomy::check_value 四变体窄化为校验所需四态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DictLookup {
    /// Active(canonical) 或 Deprecated：字典登记过的合法值。
    Known,
    /// AliasActive → 归一目标 canonical_id。
    Alias(String),
    /// CandidateNew **且该 kind 字典有其它条目**：值真越界（字典里没这个值）。
    Miss,
    /// 该 kind 字典整个为空（未配置）。与 Miss 区分：未配置不是越界，属「未约束」，
    /// 回退信任原值（对齐 taxonomy::dimension_value_weights 空缓存回落 DEFAULT、
    /// decision_taxonomy 对 dict-miss 软处理）。由 lookup_dict 配合 kind_has_entries 判别。
    KindUnconfigured,
}

/// 纯决策：给定维度契约 + 字典查询结果 + 原始值 + 写入意图 → 校验结论。无 IO，完全可单测。
pub(crate) fn classify_validation(
    spec: &DimensionSpec,
    dict: DictLookup,
    raw: &str,
    intent: WriteIntent,
) -> DimValidation {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DimValidation::DropSilently;
    }
    match spec.value_source {
        ValueSource::CodeEnum | ValueSource::FreeText => DimValidation::Accept(trimmed.to_string()),
        ValueSource::Taxonomy => match dict {
            DictLookup::Known => DimValidation::Accept(trimmed.to_string()),
            DictLookup::Alias(canonical) => DimValidation::Accept(canonical),
            // 该 kind 字典整个未配置（如 m012 删 seed 后、运营未在 admin 重配）：属「未约束」，
            // 所有写入意图（含 AdminWrite/AdminDirect）一致回退信任原值——字典未配置不是写入方
            // 的错，此处 Reject/Drop 会让合法 stage 也写不进，复现 customer_stage 永不落库的回归。
            // 与 taxonomy::dimension_value_weights 空缓存回落 DEFAULT、decision_taxonomy 对
            // dict-miss 软处理一致（未配置=不约束）。
            DictLookup::KindUnconfigured => DimValidation::Accept(trimmed.to_string()),
            DictLookup::Miss => {
                // admin 是权威直接写入方，任何维度越界都当场报错纠正；
                // AdminDirect 通道（如 relationship_type）即便机器路径越界也报错。
                if intent == WriteIntent::AdminWrite
                    || matches!(spec.channel, DimensionChannel::AdminDirect)
                {
                    DimValidation::Reject(format!("{} 取值 {:?} 不在字典内", spec.kind, trimmed))
                } else {
                    // LLM/reaction 偶发臆造不阻断已发送回复，丢弃 + 留审计（调用方写审计）。
                    DimValidation::DropSilently
                }
            }
        },
    }
}

/// 纯函数：把 taxonomy::check_value 四变体映射为 DictLookup 三态。
/// 红线：Active|Deprecated → Known（Deprecated 是字典登记过的合法历史值，不越界）；
/// 仅 CandidateNew → Miss（字典真无）。无 IO，完全可单测——守护这条映射不被误改。
fn match_to_dict(m: crate::agent::taxonomy::TaxonomyMatch) -> DictLookup {
    use crate::agent::taxonomy::TaxonomyMatch;
    match m {
        TaxonomyMatch::AliasActive(canonical) => DictLookup::Alias(canonical),
        TaxonomyMatch::Active | TaxonomyMatch::Deprecated => DictLookup::Known,
        TaxonomyMatch::CandidateNew => DictLookup::Miss,
    }
}

/// 字典查询薄壳：取进程级 taxonomy cache（懒加载兜底），把 TaxonomyMatch 四变体
/// 映射为 DictLookup（映射逻辑见 match_to_dict）。Miss 再经 kind_has_entries 细分：
/// 该 kind 字典整个为空 → KindUnconfigured（未配置，回退信任）；有条目仅此值越界 → Miss。
async fn lookup_dict(
    db: &crate::db::Database,
    kind: &str,
    trimmed: &str,
    scope_account_id: &str,
) -> DictLookup {
    use crate::agent::taxonomy::{check_value, global_taxonomy_cache, kind_has_entries};
    let cache = global_taxonomy_cache();
    cache.find_or_load(db).await;
    match match_to_dict(check_value(kind, trimmed, scope_account_id, &cache)) {
        // check_value 对「字典空」与「值越界」都回 CandidateNew→Miss，这里用 kind_has_entries
        // 细分：该 kind 无任何条目 → KindUnconfigured（未配置）。同一 cache，无中途 reload。
        DictLookup::Miss if !kind_has_entries(kind, scope_account_id, &cache) => {
            DictLookup::KindUnconfigured
        }
        other => other,
    }
}

/// DB 薄壳：查 registry + 字典，委托 classify_validation。未知 kind → 直通信任。
pub(crate) async fn validate_dimension_value(
    db: &crate::db::Database,
    kind: &str,
    raw: &str,
    scope_account_id: &str,
    intent: WriteIntent,
) -> DimValidation {
    let Some(spec) = spec_for(kind) else {
        return DimValidation::Accept(raw.trim().to_string());
    };
    // 空串短路：与 classify_validation 空串语义一致（都是 DropSilently），
    // 提前返回省掉 Taxonomy 源的 cache 加载 + 字典查询这趟无谓 IO。
    if raw.trim().is_empty() {
        return DimValidation::DropSilently;
    }
    // 非 Taxonomy 源不查字典（Miss 占位，CodeEnum/FreeText 分支不看 dict）。
    let dict = if matches!(spec.value_source, ValueSource::Taxonomy) {
        lookup_dict(db, kind, raw.trim(), scope_account_id).await
    } else {
        DictLookup::Miss
    };
    classify_validation(spec, dict, raw, intent)
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
        // relationship_type=AdminDirect+Taxonomy：字典有条目但此值越界(Miss) → Reject。
        // 即便是机器路径写（MachineWrite），AdminDirect 通道仍恒 Reject。
        let spec = spec_for("relationship_type").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "瞎编关系", WriteIntent::MachineWrite);
        assert!(matches!(r, DimValidation::Reject(_)));
    }

    #[test]
    fn classify_llm_signals_drops_out_of_dict() {
        // customer_stage=LlmSignals+Taxonomy：机器路径字典有条目但此值越界(Miss) → DropSilently（不阻断已发送）。
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "臆造态", WriteIntent::MachineWrite);
        assert!(matches!(r, DimValidation::DropSilently));
    }

    #[test]
    fn classify_kind_unconfigured_accepts_machine() {
        // 红线①（本次修复核心）：customer_stage 字典整个未配置（m012 删 seed + 运营未重配）时，
        // 机器路径产出的值回退信任原值，**不 drop**——否则 customer_stage 永不落库、停滞催跟/
        // 再激活全死。区别于 Miss（字典有条目但越界 → drop）。
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(
            spec,
            DictLookup::KindUnconfigured,
            "relationship_building",
            WriteIntent::MachineWrite,
        );
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "relationship_building"));
    }

    #[test]
    fn classify_kind_unconfigured_accepts_admin() {
        // 字典未配置时 admin 直写也回退信任（未配置不是 admin 的错，Reject 会让 admin 也写不进）。
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(
            spec,
            DictLookup::KindUnconfigured,
            "need_discovery",
            WriteIntent::AdminWrite,
        );
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "need_discovery"));
    }

    #[test]
    fn classify_kind_unconfigured_accepts_admin_direct() {
        // AdminDirect 通道（relationship_type）在字典未配置时也 Accept（一致回退信任）——
        // 与 Miss 下 AdminDirect 恒 Reject（classify_admin_direct_rejects_out_of_dict）对照：
        // 「未配置」与「有条目但越界」是两种语义。
        let spec = spec_for("relationship_type").unwrap();
        let r = classify_validation(
            spec,
            DictLookup::KindUnconfigured,
            "customer",
            WriteIntent::MachineWrite,
        );
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "customer"));
    }

    #[test]
    fn classify_admin_write_rejects_machine_channel() {
        // 本次新语义守护：customer_stage 主通道是 LlmSignals（机器容错 drop），
        // 但 admin 是权威写入方——admin 写机器通道维度越界也必须 Reject 当场报错，
        // 绝不静默丢弃（admin 以为存上了实际没存）。
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "臆造态", WriteIntent::AdminWrite);
        assert!(matches!(r, DimValidation::Reject(_)));
    }

    #[test]
    fn classify_alias_normalizes() {
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(
            spec,
            DictLookup::Alias("need_discovery".into()),
            "需求挖掘",
            WriteIntent::AdminWrite,
        );
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "need_discovery"));
    }

    #[test]
    fn classify_known_accepts_canonical_and_deprecated() {
        // Known 覆盖 Active(canonical) 与 Deprecated：两者都是字典登记过的合法值 → Accept 原值。
        // 红线：deprecated 是历史合法值，admin 通道也必须 Accept 不得 reject。
        let stage = spec_for("customer_stage").unwrap();
        let r = classify_validation(stage, DictLookup::Known, "need_discovery", WriteIntent::AdminWrite);
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "need_discovery"));
        let rel = spec_for("relationship_type").unwrap();
        let r2 = classify_validation(rel, DictLookup::Known, "customer", WriteIntent::AdminWrite);
        assert!(matches!(r2, DimValidation::Accept(ref c) if c == "customer"));
    }

    #[test]
    fn classify_code_enum_trusts() {
        // value_tier=CodeEnum：不查字典直接信任（Miss 占位不影响）。
        let spec = spec_for("value_tier").unwrap();
        let r = classify_validation(spec, DictLookup::Miss, "high", WriteIntent::MachineWrite);
        assert!(matches!(r, DimValidation::Accept(ref c) if c == "high"));
    }

    #[test]
    fn classify_empty_drops() {
        let spec = spec_for("customer_stage").unwrap();
        let r = classify_validation(spec, DictLookup::Known, "  ", WriteIntent::AdminWrite);
        assert!(matches!(r, DimValidation::DropSilently));
    }

    #[test]
    fn match_to_dict_maps_all_variants() {
        use crate::agent::taxonomy::TaxonomyMatch;
        // 红线核心：Deprecated 是字典登记过的合法历史值 → Known，绝不能降为 Miss。
        assert_eq!(match_to_dict(TaxonomyMatch::Deprecated), DictLookup::Known);
        assert_eq!(match_to_dict(TaxonomyMatch::Active), DictLookup::Known);
        // 仅 CandidateNew（字典真无）才算越界 → Miss。
        assert_eq!(match_to_dict(TaxonomyMatch::CandidateNew), DictLookup::Miss);
        // AliasActive 归一到 canonical id。
        assert_eq!(
            match_to_dict(TaxonomyMatch::AliasActive("x".to_string())),
            DictLookup::Alias("x".to_string())
        );
    }
}
