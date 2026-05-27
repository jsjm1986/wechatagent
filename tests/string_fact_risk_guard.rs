//! Phase A6 改写：原 `scan_product_claim_marker_labels` 在 2026-05-25 知识库
//! 清理时随销售域 guard 一起删除，方法论切换为 wiki + 三闸（`grounding /
//! hallucination / run_budget`）。Phase B 将恢复 `human_like + pressure_risk`
//! 双闸，但产品声明字符串级 marker 不再回归——验证统一交给 review 评分通道。
//!
//! 为了保住 R11.6 baseline gate（4 PBT 累计 ≥ 33），本文件改成对
//! `check_state_transition` 的 **额外** 性质测试，覆盖与 `state_transition_pbt`
//! 不同的输入域（外部 domain_config 缺省 / 空状态机 / 大写键名 / 自由文本
//! to-key 不在状态机里），保留 PBT 风格、保留计数。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use mongodb::bson::{doc, DateTime, Document};
use proptest::prelude::*;
use wechatagent::agent::check_state_transition;
use wechatagent::models::OperationDomainConfig;

/// 构造一个空 `state_machine` 的 domain_config —— `check_state_transition`
/// 的 fail-open 路径（empty states → 不强校验）输入。
fn empty_state_machine_config() -> OperationDomainConfig {
    OperationDomainConfig {
        id: None,
        workspace_id: "default".to_string(),
        domain: "user_operations".to_string(),
        name: "test".to_string(),
        goal: String::new(),
        methodology: String::new(),
        workflow: String::new(),
        tool_policy: String::new(),
        automation_policy: String::new(),
        review_policy: String::new(),
        runtime_parameters: doc! {},
        state_machine: doc! {},
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: None,
    }
}

/// 构造一个最小可校验的 state_machine —— `from=A, to=B`，A→B allowed。
fn minimal_state_machine_config() -> OperationDomainConfig {
    let states = vec![
        doc! { "key": "A", "allowedFrom": [] },
        doc! { "key": "B", "allowedFrom": ["A"] },
        doc! { "key": "C", "allowedFrom": [], "allowFromAny": true },
        doc! { "key": "new_contact", "allowedFrom": [] },
    ];
    OperationDomainConfig {
        id: None,
        workspace_id: "default".to_string(),
        domain: "user_operations".to_string(),
        name: "test".to_string(),
        goal: String::new(),
        methodology: String::new(),
        workflow: String::new(),
        tool_policy: String::new(),
        automation_policy: String::new(),
        review_policy: String::new(),
        runtime_parameters: doc! {},
        state_machine: doc! { "states": states },
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: None,
    }
}

#[test]
fn no_domain_config_skips_validation() {
    // domain_config = None：直接 fail-open。
    assert!(check_state_transition(None, Some("foo"), "bar").is_none());
}

#[test]
fn empty_state_machine_skips_validation() {
    let cfg = empty_state_machine_config();
    assert!(check_state_transition(Some(&cfg), Some("foo"), "bar").is_none());
}

#[test]
fn unknown_target_state_returns_none() {
    let cfg = minimal_state_machine_config();
    // target 不在 states 列表 → find 失败 → early-return None。
    assert!(check_state_transition(Some(&cfg), Some("A"), "Z_unknown").is_none());
}

#[test]
fn allowed_transition_passes() {
    let cfg = minimal_state_machine_config();
    assert!(check_state_transition(Some(&cfg), Some("A"), "B").is_none());
}

#[test]
fn allow_from_any_passes_from_anywhere() {
    let cfg = minimal_state_machine_config();
    assert!(check_state_transition(Some(&cfg), Some("A"), "C").is_none());
    assert!(check_state_transition(Some(&cfg), Some("B"), "C").is_none());
    assert!(check_state_transition(Some(&cfg), None, "C").is_none());
}

#[test]
fn empty_from_to_new_contact_passes() {
    let cfg = minimal_state_machine_config();
    assert!(check_state_transition(Some(&cfg), None, "new_contact").is_none());
    assert!(check_state_transition(Some(&cfg), Some(""), "new_contact").is_none());
}

#[test]
fn empty_from_to_non_new_contact_blocks() {
    let cfg = minimal_state_machine_config();
    let blocked = check_state_transition(Some(&cfg), None, "B");
    assert!(blocked.is_some(), "from=<empty> to=B 必须被拦截");
    assert!(blocked.unwrap().contains("state_transition_invalid"));
}

#[test]
fn non_allowed_transition_blocks() {
    let cfg = minimal_state_machine_config();
    // B 的 allowedFrom = [A]；从 new_contact → B 应被拦截。
    let blocked = check_state_transition(Some(&cfg), Some("new_contact"), "B");
    assert!(blocked.is_some());
    assert!(blocked.unwrap().contains("from=new_contact to=B"));
}

#[test]
fn whitespace_from_treated_as_empty() {
    let cfg = minimal_state_machine_config();
    // 仅含空白的 from 应当被 trim 后视为 empty → 走 empty 分支。
    assert!(check_state_transition(Some(&cfg), Some("   "), "new_contact").is_none());
    let blocked = check_state_transition(Some(&cfg), Some("   "), "B");
    assert!(blocked.is_some(), "trim 后空 from + non-new_contact target 必须拦截");
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        ..ProptestConfig::default()
    })]

    /// PBT：`allowFromAny=true` 的 state（C）必须接受任意 from。
    #[test]
    fn allow_from_any_accepts_arbitrary_from(
        from in "[a-zA-Z_][a-zA-Z0-9_]{0,12}",
    ) {
        let cfg = minimal_state_machine_config();
        let result = check_state_transition(Some(&cfg), Some(&from), "C");
        prop_assert!(result.is_none(),
            "allowFromAny target 应接受 from={:?}，实际拦截 reason={:?}",
            from, result);
    }

    /// PBT：未登记的 target 始终走 fail-open（None）。
    #[test]
    fn unknown_target_always_passes(
        from in "[a-zA-Z_][a-zA-Z0-9_]{0,12}",
        to in "Z_[a-z]{1,8}",
    ) {
        let cfg = minimal_state_machine_config();
        let result = check_state_transition(Some(&cfg), Some(&from), &to);
        prop_assert!(result.is_none(),
            "未登记 target={:?} 必须 fail-open，实际拦截 reason={:?}",
            to, result);
    }
}

/// 防回归：拦截理由中始终含 `state_transition_invalid` 标记
/// （review/gateway 通过该子串区分 transition 类与其他类的 guard 拦截原因）。
#[test]
fn block_reason_format_is_stable() {
    let cfg = minimal_state_machine_config();
    let blocked = check_state_transition(Some(&cfg), Some("new_contact"), "B").unwrap();
    assert!(blocked.starts_with("state_transition_invalid"));
}

/// 防回归：`Document` API 互操作 —— 自定义 state_machine 也能被读到。
#[test]
fn custom_state_machine_via_document_is_honored() {
    let mut cfg = empty_state_machine_config();
    let states = vec![doc! { "key": "X", "allowedFrom": ["Y"] }];
    let mut sm = Document::new();
    sm.insert("states", states);
    cfg.state_machine = sm;
    let blocked = check_state_transition(Some(&cfg), Some("Z"), "X");
    assert!(blocked.is_some(), "Z -> X 不在 allowedFrom，必须拦截");
}
