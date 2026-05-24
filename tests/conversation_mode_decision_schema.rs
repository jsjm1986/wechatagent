//! v3 prompt-pack / Task 270：`RawAgentDecision::validate_and_promote` 对
//! `conversationMode` 字段的严格枚举校验性质测试。
//!
//! 性质（与 `src/agent/types.rs::CONVERSATION_MODE_VALUES` 一一对应）：
//!
//! - **合法枚举一律保留**：`conversation_mode ∈ {casual_relationship,
//!   value_exchange, consultative, boundary_protection}` 时，promote 后的
//!   `decision.conversation_mode` 必须等于原值，且 `risks` SHALL NOT 包含
//!   `invalid_enum_value:conversation_mode:*`。
//! - **未知值一律 reject**：随机非法值（含空白、含中文、纯英文邻近值）
//!   SHALL 触发 `invalid_enum_value:conversation_mode:<v>`，且
//!   `decision.conversation_mode` 落到默认 `casual_relationship`（最保守）。
//! - **缺失必填**：`conversation_mode = None / Some("") / Some("   ")` SHALL
//!   触发 `missing_required_field:conversation_mode`。
//!
//! 这个测试文件是纯函数 PBT，无 Docker / 无 Mongo，跑得快，与
//! `autonomy_protocol_pbt.rs` 走同一模式但仅聚焦 conversation_mode。

use proptest::prelude::*;
use wechatagent::agent::{RawAgentDecision, UserRuntimeParameters};

/// 与 src/agent/types.rs:378 的 `CONVERSATION_MODE_VALUES` 严格对齐。
const VALID_MODES: &[&str] = &[
    "casual_relationship",
    "value_exchange",
    "consultative",
    "boundary_protection",
];

/// 构造一个能通过 `final` 轮全部 R1.3 / R3.1 / R3.2 / R3.3 校验的 baseline
/// raw（low_routine 形态）。所有非 conversation_mode 字段都写满合法值，
/// 这样 risks 输出中除了 conversation_mode 相关那条以外别无其它，
/// 让单一字段的注入式断言变得稳定可读。
fn build_low_routine_baseline_raw() -> RawAgentDecision {
    let mut raw = RawAgentDecision::default();
    raw.decision_phase = Some("final".to_string());
    raw.risk_level = Some("low".to_string());
    raw.knowledge_need = Some("not_required".to_string());
    raw.run_mode = Some("fast_chat".to_string());
    raw.autonomy_mode = Some("auto".to_string());
    raw.needs_review = Some(false);
    raw.consolidation_needed = Some(false);
    raw.operation_state = Some("idle".to_string());
    raw.user_understanding = Some("unchanged".to_string());
    raw.relationship_read = Some("unchanged".to_string());
    raw.operation_goal = Some("unchanged".to_string());
    raw.knowledge_need_reason = Some("无须查询知识库即可回应".to_string());
    raw.memory_update_reason = Some("unchanged".to_string());
    raw.self_critique = Some("回复内容平和无误导".to_string());
    raw.risk_self_check = Some("unchanged".to_string());
    raw.why_should_reply = Some("用户主动打招呼及时寒暄维持关系".to_string());
    raw.why_skip_reply = None;
    raw.should_reply = Some(true);
    raw.reply_text = Some("好的，谢谢你的问候。".to_string());
    raw
}

#[test]
fn all_four_valid_modes_pass_validation() {
    let runtime = UserRuntimeParameters::default();
    for mode in VALID_MODES {
        let mut raw = build_low_routine_baseline_raw();
        raw.conversation_mode = Some((*mode).to_string());
        let (decision, risks) = raw.validate_and_promote(&runtime);

        assert_eq!(
            decision.conversation_mode, *mode,
            "合法 conversation_mode={} 必须原样保留，实际={}",
            mode, decision.conversation_mode
        );
        for r in &risks {
            assert!(
                !r.starts_with("invalid_enum_value:conversation_mode:"),
                "合法 mode 不应触发 invalid_enum_value，risks={:?}",
                risks
            );
            assert!(
                r != "missing_required_field:conversation_mode",
                "合法 mode 不应触发 missing_required_field，risks={:?}",
                risks
            );
        }
    }
}

#[test]
fn missing_conversation_mode_pushes_missing_required_field() {
    let runtime = UserRuntimeParameters::default();

    // None
    let mut raw = build_low_routine_baseline_raw();
    raw.conversation_mode = None;
    let (decision, risks) = raw.validate_and_promote(&runtime);
    assert!(
        risks.contains(&"missing_required_field:conversation_mode".to_string()),
        "None SHALL 触发 missing_required_field:conversation_mode, risks={:?}",
        risks
    );
    // 兜底默认值：casual_relationship（最保守）
    assert_eq!(decision.conversation_mode, "casual_relationship");

    // 空字符串
    let mut raw = build_low_routine_baseline_raw();
    raw.conversation_mode = Some(String::new());
    let (_decision, risks) = raw.validate_and_promote(&runtime);
    assert!(
        risks.contains(&"missing_required_field:conversation_mode".to_string()),
        "空字符串 SHALL 触发 missing_required_field, risks={:?}",
        risks
    );

    // 仅空白
    let mut raw = build_low_routine_baseline_raw();
    raw.conversation_mode = Some("   \t  ".to_string());
    let (_decision, risks) = raw.validate_and_promote(&runtime);
    assert!(
        risks.contains(&"missing_required_field:conversation_mode".to_string()),
        "纯空白 SHALL 触发 missing_required_field, risks={:?}",
        risks
    );
}

/// 直观的非法值清单：常见 LLM 漂移（拼写错误 / 邻近概念 / 大小写 /
/// 中文同义词），逐一断言全部被 reject。
#[test]
fn known_drift_values_rejected_with_invalid_enum_value() {
    let runtime = UserRuntimeParameters::default();
    let drift_values = &[
        "Casual_Relationship",  // 大小写错误
        "casualrelationship",   // 缺少下划线
        "consult",              // 截断
        "consultive",           // 拼写错误
        "sales",                // 邻近概念但非协议词
        "support",              // 邻近概念
        "boundary",             // 截断
        "value",                // 截断
        "顾问销售",             // 中文同义
        "寒暄",                 // 中文同义
        "auto",                 // 跨字段污染（autonomy_mode 的值）
        "low",                  // 跨字段污染（risk_level 的值）
        "unknown",              // 占位词
        "null",                 // JSON null 字面量
    ];
    for val in drift_values {
        let mut raw = build_low_routine_baseline_raw();
        raw.conversation_mode = Some((*val).to_string());
        let (decision, risks) = raw.validate_and_promote(&runtime);

        let expected = format!("invalid_enum_value:conversation_mode:{}", val);
        assert!(
            risks.contains(&expected),
            "非法值 {:?} SHALL 触发 invalid_enum_value:conversation_mode:{}, 实际 risks={:?}",
            val,
            val,
            risks
        );
        // 非法值兜底走 default_conversation_mode = casual_relationship
        assert_eq!(
            decision.conversation_mode, "casual_relationship",
            "非法 mode={} 落地 SHALL 走默认 casual_relationship, 实际={}",
            val, decision.conversation_mode
        );
    }
}

// ─────────────────────────────────────────────────────────────────
// 性质测试：随机非法值一律 reject + 落到默认值
// ─────────────────────────────────────────────────────────────────

/// 生成"形如合法但实际非法"的随机字符串：
/// - 至少 1 char、至多 32 chars；
/// - 由 ascii 小写字母 / 数字 / 下划线组成；
/// - 与 4 个合法值都不相等。
fn invalid_mode_strategy() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-z0-9_]{1,32}")
        .expect("regex compiles")
        .prop_filter(
            "must NOT be one of the four valid modes",
            |s| !VALID_MODES.iter().any(|v| *v == s.as_str()),
        )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// P_invalid_mode：任意非法字符串 SHALL 触发
    /// `invalid_enum_value:conversation_mode:<v>`，且 promote 后的
    /// `conversation_mode` 落到默认值 `casual_relationship`。
    #[test]
    fn pbt_invalid_mode_always_rejected(mode in invalid_mode_strategy()) {
        let runtime = UserRuntimeParameters::default();
        let mut raw = build_low_routine_baseline_raw();
        raw.conversation_mode = Some(mode.clone());
        let (decision, risks) = raw.validate_and_promote(&runtime);

        let expected = format!("invalid_enum_value:conversation_mode:{}", mode);
        prop_assert!(
            risks.contains(&expected),
            "非法 mode={:?} SHALL 触发 {:?}，实际 risks={:?}",
            mode, expected, risks
        );
        prop_assert_eq!(
            decision.conversation_mode.as_str(),
            "casual_relationship",
            "非法 mode 落地 SHALL 走默认值"
        );
    }

    /// P_valid_mode：四个合法枚举随机抽样下，promote SHALL 不引入
    /// `invalid_enum_value` / `missing_required_field` 与 conversation_mode
    /// 相关的 risk，且字段被原样保留。
    #[test]
    fn pbt_valid_mode_always_passes(idx in 0usize..VALID_MODES.len()) {
        let mode = VALID_MODES[idx];
        let runtime = UserRuntimeParameters::default();
        let mut raw = build_low_routine_baseline_raw();
        raw.conversation_mode = Some(mode.to_string());
        let (decision, risks) = raw.validate_and_promote(&runtime);

        prop_assert_eq!(decision.conversation_mode.as_str(), mode);
        for r in &risks {
            prop_assert!(
                !r.starts_with("invalid_enum_value:conversation_mode:"),
                "合法 mode={} 不应触发 invalid_enum_value，risks={:?}",
                mode, risks
            );
            prop_assert!(
                r != "missing_required_field:conversation_mode",
                "合法 mode={} 不应触发 missing_required_field，risks={:?}",
                mode, risks
            );
        }
    }
}

#[test]
fn sunset_path_skips_conversation_mode_validation() {
    // R11 sunset 路径：autonomy_protocol_enabled=false 时所有校验跳过，
    // 即便 conversation_mode 写得很离谱，risks 也应为空。
    let mut runtime = UserRuntimeParameters::default();
    runtime.autonomy_protocol_enabled = false;

    let mut raw = build_low_routine_baseline_raw();
    raw.conversation_mode = Some("clearly_invalid_value".to_string());
    let (_decision, risks) = raw.validate_and_promote(&runtime);

    assert!(
        risks.is_empty(),
        "sunset 路径 SHALL 跳过校验, risks={:?}",
        risks
    );
}

#[test]
fn tool_calling_phase_skips_conversation_mode_validation() {
    // tool_calling 中间轮：跳过全部 R3 严格枚举校验，conversation_mode 的
    // 缺失 / 非法 SHALL NOT 触发任何 conversation_mode 相关 risk
    // （tool_calls 留空 / None 不会引入额外 invalid_tool_call 噪声）。
    let mut raw = RawAgentDecision::default();
    raw.decision_phase = Some("tool_calling".to_string());
    raw.tool_calls = None;
    raw.conversation_mode = Some("totally_bogus_mode".to_string());

    let runtime = UserRuntimeParameters::default();
    let (_decision, risks) = raw.validate_and_promote(&runtime);

    for r in &risks {
        assert!(
            !r.starts_with("invalid_enum_value:conversation_mode:"),
            "tool_calling 中间轮 SHALL 跳过 conversation_mode 校验，risks={:?}",
            risks
        );
        assert!(
            r != "missing_required_field:conversation_mode",
            "tool_calling 中间轮不应触发 missing_required_field, risks={:?}",
            risks
        );
    }
}
