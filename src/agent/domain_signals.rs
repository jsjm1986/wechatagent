//! universal-domain-adaptation Phase 1 / 1D：`AgentDecision` 的 **domain_signals
//! 容器**（H1）与统一的画像维度写入内核。
//!
//! 背景（H1 硬编码点）：`AgentDecision` 历史上把销售域的两个画像维度
//! （`customer_stage` / `intent_level`）写成 typed 字段。要让数字分身适配任意
//! 行业（陪伴域可能是 `relationship_closeness` / `emotional_state` 等），决策侧
//! 必须携带一个对维度名零假设的开放容器 `domain_signals: Document`。
//!
//! **红线**：typed `customer_stage` / `intent_level` 字段**保留不删**——删掉会破
//! lib 编译基线 + 一众 integration test（state_transition_pbt 等）。本模块通过
//! [`normalize_domain_signals`] 在 typed 字段与容器之间做双向同步，使两侧始终
//! 一致；DEFAULT（销售域）行为逐字不变。
//!
//! **写入收敛**：此前 gateway（AI 决策落库）与 `routes::shared`（admin 手动改画像）
//! 各有一套把维度写进 `domain_attributes.*` 的实现，`stage_changed` /
//! `domain_attributes_updated_at` 语义分散两处易漂移。两者现在都经由
//! [`insert_domain_signal_values`] 这一个内核写 dotted-key + 刷新
//! `customer_stage_updated_at`，单一真相源。

use mongodb::bson::{DateTime, Document};

use super::types::AgentDecision;

/// 1D 已知的 typed 画像维度（容器键名与 `system_taxonomies.kind` /
/// `decision_taxonomy::TAGGED_FIELDS` 一致）。1A 会把这份硬编码列表换成
/// 「读 active profile 的维度集合」，本阶段先逐字复刻当前销售域两维。
const KNOWN_TYPED_DIMS: &[&str] = &["customer_stage", "intent_level"];

/// 把 `AgentDecision` 的 typed 维度字段与 `domain_signals` 容器做**双向同步**：
///
/// - typed → 容器：typed 字段非空（trim 后）时写入容器对应键（typed 取值已在
///   `decision_taxonomy` 经 alias→canonical 改写，故 typed 为权威，覆盖容器）；
/// - 容器 → typed：typed 为空但容器有非空字符串时回填 typed（保证只读 typed 的
///   下游——decision_taxonomy / gateway churn / planner stagnation——仍能取到值）。
///
/// 调用时机：`decide_reply_with_promote` 在 `validate_and_normalize_decision`
/// （taxonomy 规整）之后调用，使容器持有 canonical 取值。DEFAULT 销售域里 LLM
/// 只输出 typed（不输出 `domainSignals`），故只有 typed→容器 方向生效、容器→typed
/// 为空操作——行为与改造前逐字等价。
pub(crate) fn normalize_domain_signals(decision: &mut AgentDecision) {
    for &dim in KNOWN_TYPED_DIMS {
        let typed = typed_dim(decision, dim)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string);
        if let Some(value) = typed {
            // typed 权威：写回容器（canonical）。
            decision
                .domain_signals
                .insert(dim.to_string(), value);
        } else if let Some(from_container) = decision
            .domain_signals
            .get_str(dim)
            .ok()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
        {
            // typed 缺失但容器有值：回填 typed，供只读 typed 的下游消费。
            set_typed_dim(decision, dim, from_container);
        }
    }
}

/// 取 typed 维度字段的当前值（1A 之前仅 customer_stage / intent_level 两个）。
fn typed_dim<'a>(decision: &'a AgentDecision, dim: &str) -> Option<&'a str> {
    match dim {
        "customer_stage" => decision.customer_stage.as_deref(),
        "intent_level" => decision.intent_level.as_deref(),
        _ => None,
    }
}

/// 回填 typed 维度字段。
fn set_typed_dim(decision: &mut AgentDecision, dim: &str, value: String) {
    match dim {
        "customer_stage" => decision.customer_stage = Some(value),
        "intent_level" => decision.intent_level = Some(value),
        _ => {}
    }
}

/// 按维度 `kind` 读取 `AgentDecision` 当前取值（H2/H7 维度列表动态化的统一访问器）。
///
/// 销售域两维（`customer_stage`/`intent_level`）从 typed 字段读（它们是 DEFAULT，
/// LLM 以 typed JSON 键输出）；其它任意维度从 `domain_signals` 容器读。这样
/// taxonomy 校验循环可以对 `decision_dimension_kinds(profile)` 给出的任意维度集
/// 工作，而不再写死两维。
pub(crate) fn get_dimension<'a>(decision: &'a AgentDecision, kind: &str) -> Option<&'a str> {
    match kind {
        "customer_stage" => decision.customer_stage.as_deref(),
        "intent_level" => decision.intent_level.as_deref(),
        other => decision.domain_signals.get_str(other).ok(),
    }
}

/// 按维度 `kind` 写回 `AgentDecision`（alias→canonical 改写时用）。
///
/// 销售域两维写 typed 字段（随后由 [`normalize_domain_signals`] 镜像进容器）；
/// 其它维度直接写容器。
pub(crate) fn set_dimension(decision: &mut AgentDecision, kind: &str, value: String) {
    match kind {
        "customer_stage" => decision.customer_stage = Some(value),
        "intent_level" => decision.intent_level = Some(value),
        other => {
            decision.domain_signals.insert(other.to_string(), value);
        }
    }
}

/// 统一的画像维度写入内核：把 `signals` 中**每个非空字符串维度**以
/// `domain_attributes.<key>` dotted-key 写进 `set_doc`。
///
/// - `stage_changed == true`（调用方依「新 stage 取值 vs contact 现有 stage」判定，
///   仅在 stage 维度真正变化时为真）→ 追加 `domain_attributes.customer_stage_updated_at`
///   （planner 的 stage stagnation 计时器依赖它；1C 会把停滞维度从写死的
///   `customer_stage` 改为 `profile.stagnation_dimension`）。
/// - 返回是否写入了**任何**维度值。容器级 `domain_attributes_updated_at` 的刷新
///   策略由调用方决定（gateway：写了才刷；admin：总是刷，保留其既有契约）——故
///   本内核**不**自行写容器级时间戳，避免两条路径的边界行为被强行统一。
///
/// 用 dotted-key（而非整体替换 `domain_attributes` 子文档）保证字段级原子、不覆盖
/// 容器内其它键，并与 escalation 写法一致。MongoDB 对不存在的 `domain_attributes`
/// 会按 dotted-path 自动建嵌套对象。
pub(crate) fn insert_domain_signal_values(
    set_doc: &mut Document,
    signals: &Document,
    stage_changed: bool,
) -> bool {
    let mut wrote_any = false;
    for (key, value) in signals {
        if let Some(text) = value.as_str() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            set_doc.insert(format!("domain_attributes.{key}"), trimmed);
            wrote_any = true;
        }
    }
    if stage_changed {
        set_doc.insert("domain_attributes.customer_stage_updated_at", DateTime::now());
    }
    wrote_any
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    // ── 写入内核等价性：销售域两维场景必须与改造前 gateway 手写块逐字一致 ──

    #[test]
    fn kernel_writes_sales_dims_like_legacy() {
        let mut set_doc = Document::new();
        let signals = doc! { "customer_stage": "solution_fit", "intent_level": "high" };
        let wrote = insert_domain_signal_values(&mut set_doc, &signals, true);

        assert!(wrote, "写入了维度值应返回 true");
        assert_eq!(
            set_doc.get_str("domain_attributes.customer_stage").ok(),
            Some("solution_fit")
        );
        assert_eq!(
            set_doc.get_str("domain_attributes.intent_level").ok(),
            Some("high")
        );
        // stage_changed=true → 刷新 stage 计时器
        assert!(set_doc.contains_key("domain_attributes.customer_stage_updated_at"));
    }

    #[test]
    fn kernel_skips_stage_ts_when_unchanged() {
        let mut set_doc = Document::new();
        let signals = doc! { "customer_stage": "need_discovery" };
        let wrote = insert_domain_signal_values(&mut set_doc, &signals, false);

        assert!(wrote);
        assert_eq!(
            set_doc.get_str("domain_attributes.customer_stage").ok(),
            Some("need_discovery")
        );
        // stage_changed=false → 不刷新计时器（与 legacy 一致）
        assert!(!set_doc.contains_key("domain_attributes.customer_stage_updated_at"));
    }

    #[test]
    fn kernel_empty_signals_writes_nothing_and_returns_false() {
        let mut set_doc = Document::new();
        let wrote = insert_domain_signal_values(&mut set_doc, &Document::new(), false);

        assert!(!wrote, "空容器不应写任何维度");
        assert!(set_doc.is_empty(), "set_doc 应保持为空：{set_doc:?}");
    }

    #[test]
    fn kernel_trims_and_skips_blank_values() {
        let mut set_doc = Document::new();
        let signals = doc! { "customer_stage": "  solution_fit  ", "intent_level": "   " };
        let wrote = insert_domain_signal_values(&mut set_doc, &signals, false);

        assert!(wrote);
        // 前后空白被 trim
        assert_eq!(
            set_doc.get_str("domain_attributes.customer_stage").ok(),
            Some("solution_fit")
        );
        // 纯空白维度被跳过，不落库
        assert!(!set_doc.contains_key("domain_attributes.intent_level"));
    }

    #[test]
    fn kernel_writes_arbitrary_non_sales_dimension() {
        // 通用化目标：陪伴域维度（非 customer_stage/intent_level）也能透传落库。
        let mut set_doc = Document::new();
        let signals = doc! { "relationship_closeness": "intimate" };
        let wrote = insert_domain_signal_values(&mut set_doc, &signals, false);

        assert!(wrote);
        assert_eq!(
            set_doc.get_str("domain_attributes.relationship_closeness").ok(),
            Some("intimate")
        );
    }

    #[test]
    fn kernel_ignores_non_string_values() {
        let mut set_doc = Document::new();
        let signals = doc! { "customer_stage": "first_contact", "some_count": 7_i32 };
        let wrote = insert_domain_signal_values(&mut set_doc, &signals, false);

        assert!(wrote);
        assert_eq!(
            set_doc.get_str("domain_attributes.customer_stage").ok(),
            Some("first_contact")
        );
        // 非字符串维度被跳过（容器只承载字符串型画像维度取值）
        assert!(!set_doc.contains_key("domain_attributes.some_count"));
    }

    // ── 双向同步 ──

    #[test]
    fn normalize_mirrors_typed_into_container() {
        let mut decision = AgentDecision {
            customer_stage: Some("first_contact".to_string()),
            intent_level: Some("high".to_string()),
            ..AgentDecision::default()
        };
        normalize_domain_signals(&mut decision);

        assert_eq!(
            decision.domain_signals.get_str("customer_stage").ok(),
            Some("first_contact")
        );
        assert_eq!(
            decision.domain_signals.get_str("intent_level").ok(),
            Some("high")
        );
    }

    #[test]
    fn normalize_mirrors_container_into_typed_when_typed_absent() {
        let mut decision = AgentDecision {
            customer_stage: None,
            intent_level: None,
            domain_signals: doc! { "customer_stage": "need_discovery" },
            ..AgentDecision::default()
        };
        normalize_domain_signals(&mut decision);

        assert_eq!(decision.customer_stage.as_deref(), Some("need_discovery"));
        // 容器没有 intent_level → typed 保持 None
        assert!(decision.intent_level.is_none());
    }

    #[test]
    fn normalize_typed_wins_when_both_present() {
        // typed 取值已经过 taxonomy canonical 改写，是权威；容器旧值被覆盖。
        let mut decision = AgentDecision {
            customer_stage: Some("first_contact".to_string()),
            domain_signals: doc! { "customer_stage": "stale_value" },
            ..AgentDecision::default()
        };
        normalize_domain_signals(&mut decision);

        assert_eq!(
            decision.domain_signals.get_str("customer_stage").ok(),
            Some("first_contact"),
            "typed canonical 取值应覆盖容器内旧值"
        );
        assert_eq!(decision.customer_stage.as_deref(), Some("first_contact"));
    }

    #[test]
    fn normalize_blank_typed_does_not_pollute_container() {
        let mut decision = AgentDecision {
            customer_stage: Some("   ".to_string()),
            ..AgentDecision::default()
        };
        normalize_domain_signals(&mut decision);

        assert!(
            !decision.domain_signals.contains_key("customer_stage"),
            "纯空白 typed 不应写进容器：{:?}",
            decision.domain_signals
        );
    }

    #[test]
    fn normalize_preserves_extra_non_sales_container_dims() {
        // 容器里已有的非销售维度，normalize 不应抹掉。
        let mut decision = AgentDecision {
            customer_stage: Some("first_contact".to_string()),
            domain_signals: doc! { "relationship_closeness": "intimate" },
            ..AgentDecision::default()
        };
        normalize_domain_signals(&mut decision);

        assert_eq!(
            decision.domain_signals.get_str("relationship_closeness").ok(),
            Some("intimate"),
            "非销售维度应保留"
        );
        assert_eq!(
            decision.domain_signals.get_str("customer_stage").ok(),
            Some("first_contact")
        );
    }
}
