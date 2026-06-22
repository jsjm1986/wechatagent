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

/// 构造一个空 `state_machine` 的 domain_config —— S1.2 (Phase 0)
/// fail-closed 输入：active domain 必须有非空 state machine。
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
        principal_decider: None,
        high_risk_escalation_mode: None,
        ask_human_policy: None,
        assist_mode_enabled: None,
    }
}

/// 构造一个最小可校验的 state_machine —— `from=A, to=B`，A→B allowed。
fn minimal_state_machine_config() -> OperationDomainConfig {
    let states = vec![
        doc! { "key": "A", "allowedFrom": [] },
        doc! { "key": "B", "allowedFrom": ["A"] },
        doc! { "key": "C", "allowedFrom": [], "allowFromAny": true },
        // H13：new_contact 标 initial:true —— 引擎从写死的 `to=="new_contact"` 改读
        // initial 标志后，空 from 唯一合法目标由本标志声明（与生产 DEFAULT 状态机一致）。
        doc! { "key": "new_contact", "allowedFrom": [], "initial": true },
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
        principal_decider: None,
        high_risk_escalation_mode: None,
        ask_human_policy: None,
        assist_mode_enabled: None,
    }
}

#[test]
fn no_domain_config_skips_validation() {
    // domain_config = None：直接 fail-open。
    assert!(check_state_transition(None, Some("foo"), "bar").is_none());
}

#[test]
fn empty_state_machine_fails_closed() {
    // S1.2 (Phase 0)：active domain 提供 cfg 但 state_machine 为空 → 必须拦截。
    // 启动期 sanity check 会拒绝这种配置；runtime 这里是 defense-in-depth。
    let cfg = empty_state_machine_config();
    let blocked = check_state_transition(Some(&cfg), Some("foo"), "bar");
    assert!(blocked.is_some(), "empty state_machine + active domain 必须 fail-closed");
    let reason = blocked.unwrap();
    assert!(reason.contains("state_transition_invalid"));
    assert!(reason.contains("state_machine_empty"));
}

#[test]
fn unknown_target_state_fails_closed() {
    let cfg = minimal_state_machine_config();
    // 修复（问题 E）：target 不在 states 列表 = 非法迁移目标 → fail-closed 拦截。
    // 此前 `?` 在 find 失败时 early-return None（fail-open），会让未知 customer_stage
    // 经 C2 写入幻影 operation_state 并旁路 policy enforcement。与 state_machine_empty
    // 已 fail-closed 的设计一致。
    let blocked = check_state_transition(Some(&cfg), Some("A"), "Z_unknown");
    assert!(blocked.is_some(), "未登记 target 必须 fail-closed 拦截");
    let reason = blocked.unwrap();
    assert!(reason.contains("state_transition_invalid"));
    assert!(reason.contains("unknown_target"), "拦截理由应标 unknown_target，实际：{reason}");
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

    /// PBT（修复问题 E）：未登记的 target 始终 fail-closed 拦截（Some），
    /// 不再 fail-open 放行。防止未知 customer_stage 写入幻影 operation_state。
    #[test]
    fn unknown_target_always_fails_closed(
        from in "[a-zA-Z_][a-zA-Z0-9_]{0,12}",
        to in "Z_[a-z]{1,8}",
    ) {
        let cfg = minimal_state_machine_config();
        let result = check_state_transition(Some(&cfg), Some(&from), &to);
        prop_assert!(result.is_some(),
            "未登记 target={:?} 必须 fail-closed 拦截，实际放行",
            to);
        prop_assert!(result.as_deref().unwrap().contains("unknown_target"),
            "拦截理由应标 unknown_target，实际={:?}", result);
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
