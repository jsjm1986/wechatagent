//! Property 1 / Task 13 / Task 24：状态机 `allowedFrom` 校验。
//!
//! 性质：`check_state_transition(from, to)` 返回 `None`（允许迁移）当且仅当
//! 1. 状态机为空（向后兼容），或
//! 2. 目标 state 设置了 `allowFromAny: true`（如 cooldown），或
//! 3. `from` 在目标 state 的 `allowedFrom` 列表中，或
//! 4. `from` 缺失且 `to == "new_contact"`。
//!
//! 否则必须返回 `Some(reason)`（拦截）。
//!
//! 用 `proptest` 在 `default_user_operation_state_machine()` 上随机生成
//! `(from, to)` 对，用闭式判定与 `check_state_transition` 对比。
//!
//! 不依赖 testcontainers / mongodb / mock LLM，默认参与 `cargo test`。

use mongodb::bson::{doc, DateTime};
use proptest::prelude::*;
use wechatagent::agent::check_state_transition;
use wechatagent::models::OperationDomainConfig;
use wechatagent::prompts::default_user_operation_state_machine;

const STATE_KEYS: &[&str] = &[
    "new_contact",
    "relationship_building",
    "need_discovery",
    "solution_fit",
    "objection_handling",
    "commitment_followup",
    "customer_success",
    "cooldown",
    "dormant_reactivation",
];

fn build_domain_config() -> OperationDomainConfig {
    OperationDomainConfig {
        id: None,
        workspace_id: "default".to_string(),
        domain: "user_operations".to_string(),
        name: "用户运营".to_string(),
        goal: String::new(),
        methodology: String::new(),
        workflow: String::new(),
        tool_policy: String::new(),
        automation_policy: String::new(),
        review_policy: String::new(),
        runtime_parameters: doc! {},
        state_machine: default_user_operation_state_machine(),
        status: "active".to_string(),
        updated_at: DateTime::now(),
    }
}

/// 闭式参考实现：直接读 `default_user_operation_state_machine` 的 `allowedFrom` /
/// `allowFromAny`，按规则计算 expected 是否允许。
fn expected_allow(from: Option<&str>, to: &str) -> bool {
    let machine = default_user_operation_state_machine();
    let states = machine.get_array("states").expect("默认状态机有 states");
    let target = states
        .iter()
        .filter_map(|item| item.as_document())
        .find(|state| state.get_str("key").ok() == Some(to));
    let Some(target) = target else {
        // 目标不存在：check_state_transition 返回 None（target 取不到也直接 None）。
        // 真实实现里 `find` 失败时函数 early-return None，等同于"允许"。
        return true;
    };
    if target.get_bool("allowFromAny").unwrap_or(false) {
        return true;
    }
    let from = from.map(str::trim).filter(|s| !s.is_empty());
    match from {
        None => to == "new_contact",
        Some(f) => target
            .get_array("allowedFrom")
            .map(|arr| arr.iter().any(|item| item.as_str() == Some(f)))
            .unwrap_or(false),
    }
}

proptest! {
    /// PBT 主性质：`check_state_transition` 与闭式参考 `expected_allow` 双向一致。
    #[test]
    fn check_state_transition_matches_reference(
        from_idx in proptest::option::of(0..STATE_KEYS.len()),
        to_idx in 0..STATE_KEYS.len(),
    ) {
        let config = build_domain_config();
        let from = from_idx.map(|i| STATE_KEYS[i]);
        let to = STATE_KEYS[to_idx];
        let blocked = check_state_transition(Some(&config), from, to).is_some();
        let expected = expected_allow(from, to);
        prop_assert_eq!(!blocked, expected,
            "from={:?} to={} blocked={} expected_allow={}", from, to, blocked, expected);
    }
}

/// 必须显式覆盖的代表性 case：`new_contact` 入口、`cooldown` 任意来源、
/// 默认状态机显式列出的自身迁移、非法来源被拦截。
#[test]
fn new_contact_allows_empty_from() {
    let config = build_domain_config();
    assert!(check_state_transition(Some(&config), None, "new_contact").is_none());
    assert!(check_state_transition(Some(&config), Some(""), "new_contact").is_none());
}

#[test]
fn cooldown_allows_any_source() {
    let config = build_domain_config();
    for &state in STATE_KEYS {
        assert!(
            check_state_transition(Some(&config), Some(state), "cooldown").is_none(),
            "cooldown 应允许从 {} 进入",
            state
        );
    }
}

#[test]
fn self_loop_is_allowed_when_listed_in_allowed_from() {
    let config = build_domain_config();
    for &state in STATE_KEYS {
        assert!(
            check_state_transition(Some(&config), Some(state), state).is_none(),
            "{} -> {} 已在默认 allowedFrom 中列出，必须允许",
            state,
            state
        );
    }
}

#[test]
fn invalid_transition_is_blocked() {
    let config = build_domain_config();
    // customer_success 的 allowedFrom 只有 commitment_followup / customer_success；
    // 从 new_contact 直接跳到 customer_success 应该被拦下。
    let blocked = check_state_transition(Some(&config), Some("new_contact"), "customer_success");
    assert!(
        blocked.is_some(),
        "new_contact -> customer_success 必须被拦截"
    );
    let reason = blocked.unwrap();
    assert!(
        reason.contains("state_transition_invalid"),
        "拦截理由应含 state_transition_invalid，实际：{reason}"
    );
}

#[test]
fn empty_state_machine_skips_validation() {
    // 没有 OperationDomainConfig（domain_config = None）时不强校验。
    assert!(check_state_transition(None, Some("anything"), "anything_else").is_none());
}
