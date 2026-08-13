//! 结果三态分类器（真实用户反应 → Hit / Block / Censored）的**单一真相源**。
//!
//! 优化第五波·线 H（H1）：本模块从 `knowledge_wiki::gap_signals` 逐字搬迁而来
//! （2.5-pre-1 引入的极性参数化分类器 + DEFAULT 销售极性常量 + 2.5-main-2 的
//! `resolve_effective_polarity`），供跨域消费方共享同一实现：
//! - 回路①：`gap_signals::refresh_usage_stats_and_confidence`（dynamic_confidence 召回排序）；
//! - 演化器：`evolution::significance`（结果加权放行差判定）与
//!   `evolution::post_release`（+24h 负反应率/三态分布观测）；
//! - `agent::domain_profile::default_outcome_polarity`（DEFAULT_PROFILE seed 同源）。
//!
//! `gap_signals` 保留 `pub use` 再导出，旧引用路径
//! （`crate::knowledge_wiki::gap_signals::classify_outcome_label` 等）行为逐字节等价。
//!
//! 纯函数、无 IO、无 LLM——放在 agent 域是因为 outcome 语义（用户对 AI 发送的
//! 回复作何反应）属于运营结果域，而非知识库域；本模块不引用 gateway / outbox / MCP。

/// 把 `AgentDecisionReview.outcome_status` 判成三态标签（DEFAULT 销售极性）。
///
/// 病根（镜厅效应）是 hit 信号取 reviewer 自评（`review_approved`），系统学的是
/// "reviewer 喜欢哪些 chunk"而非"哪些 chunk 让用户正反应"。换血即把信号源从
/// reviewer 自评换成按 `run_id` join 出来的 `AgentDecisionReview.outcome_status`。
///
/// 三态语义（Iron Law ②：沉默 = 删失，绝不当负例）：
/// - `Hit`：用户确有正向反应（购买信号）→ 计入 hit 分子；
/// - `Block`：用户确有负向反应（异议/止/退订/投诉/负面）→ 计入 block；
/// - `Censored`：沉默 / 无反应 / `pending` / 空 / 含义不明 → **删失**，既不进
///   hit 也不进 block —— 分母只含"用户确有明确反应"的样本。
///
/// 负向集合与 `agent::reaction::DEFAULT_NEGATIVE_OUTCOMES` 同源同值（历史上复刻自
/// 已删除的 `reaction::is_negative_outcome` 5 词真值表）。
///
/// universal-domain-adaptation 2.5-pre-1：本 wrapper 内联当前写死的 5+1 销售极性
/// （正极 buying_signal + 负极 5 词），委托 [`classify_outcome_label_with_polarity`]
/// → 零行为变化。2.5-main-2 把数据源换成 active DomainProfile.outcome_polarity。
pub fn classify_outcome_label(outcome_status: Option<&str>) -> OutcomeLabel {
    classify_outcome_label_with_polarity(
        outcome_status,
        DEFAULT_POSITIVE_OUTCOMES,
        DEFAULT_NEGATIVE_OUTCOMES,
    )
}

/// 2.5-pre-1：DEFAULT 销售域正极（逐字复刻原 `classify_outcome_label` 的 Hit 字面量）。
pub(crate) const DEFAULT_POSITIVE_OUTCOMES: &[&str] = &["user_replied_buying_signal"];

/// 2.5-pre-1：DEFAULT 销售域负极（逐字复刻 `reaction.rs::is_negative_outcome` 的 5 词）。
pub(crate) const DEFAULT_NEGATIVE_OUTCOMES: &[&str] = &[
    "user_replied_objection",
    "user_replied_stop_requested",
    "user_replied_unsubscribed",
    "user_replied_negative",
    "user_replied_complaint",
];

/// universal-domain-adaptation 2.5-pre-1：极性可参数化的 outcome 三态判定核心。
///
/// `positive` / `negative` 是本行业声明的正/负极 outcome 集合（来自
/// DomainProfile.outcome_polarity；DEFAULT 销售域 = `DEFAULT_POSITIVE_OUTCOMES` +
/// `DEFAULT_NEGATIVE_OUTCOMES`）。**删失语义不可配**（Iron Law ②）：不在正/负集里的
/// 一切（含沉默/pending/空/未分类/未知）一律 Censored，绝不臆测为负。正极优先于负极
/// （同一 outcome 同时被两集声明时取 Hit，防误配把购买信号当负例）。
pub fn classify_outcome_label_with_polarity(
    outcome_status: Option<&str>,
    positive: &[impl AsRef<str>],
    negative: &[impl AsRef<str>],
) -> OutcomeLabel {
    let Some(s) = outcome_status else {
        return OutcomeLabel::Censored;
    };
    if positive.iter().any(|p| p.as_ref() == s) {
        OutcomeLabel::Hit
    } else if negative.iter().any(|n| n.as_ref() == s) {
        OutcomeLabel::Block
    } else {
        OutcomeLabel::Censored
    }
}

/// [`classify_outcome_label`] 的三态结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeLabel {
    /// 用户确有正向反应 → hit 分子。
    Hit,
    /// 用户确有负向反应 → block。
    Block,
    /// 删失：沉默 / 无反应 / pending / 含义不明 → 不进任何分母。
    Censored,
}

/// 2.5-main-2：把 active DomainProfile 的 [`OutcomePolarity`](crate::models::OutcomePolarity)
/// 解析成「有效极性」字符串向量对（正极, 负极）。
///
/// **逐极独立回落**（与 main-1 seed 契约一致）：某一极为空 → 该极回落内置销售常量
/// （`DEFAULT_POSITIVE_OUTCOMES` / `DEFAULT_NEGATIVE_OUTCOMES`），非空 → 用 profile 声明的。
/// DEFAULT_PROFILE 的 seed 显式填回这两组常量，故 DEFAULT 下解析结果与回落字节相等
/// → 回路① 召回排序逐字等价。换行业（声明非空极性）时按本行业极性判定。
pub(crate) fn resolve_effective_polarity(
    polarity: &crate::models::OutcomePolarity,
) -> (Vec<String>, Vec<String>) {
    let positive = if polarity.positive.is_empty() {
        DEFAULT_POSITIVE_OUTCOMES
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        polarity.positive.clone()
    };
    let negative = if polarity.negative.is_empty() {
        DEFAULT_NEGATIVE_OUTCOMES
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        polarity.negative.clone()
    };
    (positive, negative)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// H1 三态矩阵：DEFAULT 销售极性下 Hit / Block / Censored 全真值表。
    /// gap_signals 侧的原有测试经 re-export 路径继续锁"旧路径行为逐字节等价"；
    /// 本测试锁新真相源模块自身。
    #[test]
    fn tristate_matrix_under_default_sales_polarity() {
        assert_eq!(
            classify_outcome_label(Some("user_replied_buying_signal")),
            OutcomeLabel::Hit
        );
        for s in DEFAULT_NEGATIVE_OUTCOMES {
            assert_eq!(classify_outcome_label(Some(s)), OutcomeLabel::Block, "{s}");
        }
        for s in [
            None,
            Some(""),
            Some("pending"),
            Some("analyzing"),
            Some("user_replied_unclassified"),
            Some("some_future_status"),
        ] {
            assert_eq!(classify_outcome_label(s), OutcomeLabel::Censored, "{s:?}");
        }
    }

    /// 删失语义不可配 + 正极优先于负极（与 gap_signals 原测试同契约）。
    #[test]
    fn censored_is_not_configurable_and_positive_wins_overlap() {
        let positive = ["pos"];
        let negative = ["neg"];
        for s in [None, Some(""), Some("pending"), Some("unknown_future")] {
            assert_eq!(
                classify_outcome_label_with_polarity(s, &positive, &negative),
                OutcomeLabel::Censored,
                "{s:?} 必删失"
            );
        }
        let both = ["x"];
        assert_eq!(
            classify_outcome_label_with_polarity(Some("x"), &both, &both),
            OutcomeLabel::Hit
        );
    }

    /// resolve_effective_polarity 逐极独立回落（与 gap_signals 原测试同契约）。
    #[test]
    fn resolve_polarity_each_pole_falls_back_independently() {
        let (pos, neg) =
            resolve_effective_polarity(&crate::models::OutcomePolarity::default());
        assert_eq!(pos, vec!["user_replied_buying_signal"]);
        assert_eq!(
            neg,
            DEFAULT_NEGATIVE_OUTCOMES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        let only_neg = crate::models::OutcomePolarity {
            positive: vec![],
            negative: vec!["user_went_cold".to_string()],
        };
        let (pos2, neg2) = resolve_effective_polarity(&only_neg);
        assert_eq!(pos2, vec!["user_replied_buying_signal"]);
        assert_eq!(neg2, vec!["user_went_cold"]);
    }
}
