//! R3.2 C2 operation_state 派生跨域 —— 非销售状态机的合法/非法迁移校验。
//!
//! 关联：`.kiro/specs/universal-test-coverage/requirements.md` R3.2。
//!
//! ## 为什么是纯函数确定性测（不是真模型）
//! C2 的核心命门是 `check_state_transition`(guards.rs:144，**pub 纯函数**)——它读
//! `OperationDomainConfig.state_machine` 的 `initial`/`allowFromAny`/`allowedFrom` 标志判
//! 迁移合法性，**完全确定性、与行业无关**。gateway 的派生点(`apply_agent_updates`
//! gateway.rs:2726)把 customer_stage 派生成 operation_state 后接它校验，非法→拒写+发
//! `agent.operation_state_transition_rejected` 审计(fail-soft，不阻断已发送 reply)。
//!
//! 现有 `tests/state_transition_pbt.rs` 只在**销售** `default_user_operation_state_machine`
//! 上验。spec R3.2 要的「非销售 FSM 下正确」缺口在此补：构造一个**医疗就诊**状态机
//! （初诊→复诊→方案确认→已治疗 + 失约 allowFromAny），验证同一套引擎在异行业状态机下
//! 合法迁移放行、非法迁移拦截、initial 态/allowFromAny 语义正确——证明引擎真的行业无关。
//!
//! 真模型只在「agent 真产出一个导致非法迁移的 customer_stage→C2 派生→审计落库」端到端链
//! 上有边际价值，但 `apply_agent_updates` 私有、需全 gateway+mongo+真模型，成本高收益低；
//! 命门(非法迁移判定的正确性)用纯函数测最可靠、不 flaky（spec「离线纯函数单测是确定性
//! 地基」）。本文件不依赖 Docker/真模型，默认参与 `cargo test`。

use mongodb::bson::{doc, DateTime, Document};
use wechatagent::agent::check_state_transition;
use wechatagent::models::OperationDomainConfig;

/// 一个**非销售**（医疗就诊）状态机：状态 key 与销售域完全不同，验证引擎不写死销售态。
/// - `initial_consult` 是唯一 `initial:true`（空 from 只能迁入它）；
/// - 线性推进 initial_consult → follow_up → plan_confirmed → treated；
/// - `missed_appointment` 标 `allowFromAny:true`（任何态都可失约，类比销售 cooldown）。
fn medical_state_machine() -> Document {
    doc! {
        "states": [
            { "key": "initial_consult", "initial": true, "allowedFrom": [] },
            { "key": "follow_up", "allowedFrom": ["initial_consult", "plan_confirmed"] },
            { "key": "plan_confirmed", "allowedFrom": ["follow_up"] },
            { "key": "treated", "allowedFrom": ["plan_confirmed", "follow_up"] },
            { "key": "missed_appointment", "allowFromAny": true, "allowedFrom": [] },
        ]
    }
}

fn medical_domain_config() -> OperationDomainConfig {
    OperationDomainConfig {
        id: None,
        workspace_id: "ws-medical".to_string(),
        domain: "medical_consultation".to_string(),
        name: "医疗就诊".to_string(),
        goal: String::new(),
        methodology: String::new(),
        workflow: String::new(),
        tool_policy: String::new(),
        automation_policy: String::new(),
        review_policy: String::new(),
        runtime_parameters: doc! {},
        state_machine: medical_state_machine(),
        status: "active".to_string(),
        updated_at: DateTime::now(),
        version: 1,
        current_version: true,
        previous_version: None,
        seeded_by: None,
        principal_decider: None,
        high_risk_escalation_mode: None,
    }
}

/// 合法迁移在非销售 FSM 下放行（返回 None）。
#[test]
fn cross_domain_legal_transitions_pass() {
    let cfg = medical_domain_config();
    // 线性推进全合法。
    assert!(
        check_state_transition(Some(&cfg), Some("initial_consult"), "follow_up").is_none(),
        "初诊→复诊应合法"
    );
    assert!(
        check_state_transition(Some(&cfg), Some("follow_up"), "plan_confirmed").is_none(),
        "复诊→方案确认应合法"
    );
    assert!(
        check_state_transition(Some(&cfg), Some("plan_confirmed"), "treated").is_none(),
        "方案确认→已治疗应合法"
    );
    // follow_up→treated 也在 treated.allowedFrom 内。
    assert!(
        check_state_transition(Some(&cfg), Some("follow_up"), "treated").is_none(),
        "复诊→已治疗应合法（treated.allowedFrom 含 follow_up）"
    );
    // plan_confirmed→follow_up（回退复查）在 follow_up.allowedFrom 内。
    assert!(
        check_state_transition(Some(&cfg), Some("plan_confirmed"), "follow_up").is_none(),
        "方案确认→复诊（回退）应合法"
    );
}

/// 非法迁移在非销售 FSM 下被拦截（返回 Some，理由含 state_transition_invalid）。
#[test]
fn cross_domain_illegal_transitions_rejected() {
    let cfg = medical_domain_config();
    // 跳步：初诊直接→已治疗（treated.allowedFrom 不含 initial_consult）。
    let reason = check_state_transition(Some(&cfg), Some("initial_consult"), "treated");
    assert!(reason.is_some(), "初诊直接→已治疗应被拦（跳过方案确认/复诊）");
    assert!(
        reason.unwrap().contains("state_transition_invalid"),
        "拦截理由须含 state_transition_invalid"
    );
    // 倒退：已治疗→初诊（initial_consult.allowedFrom 为空且非空 from）。
    assert!(
        check_state_transition(Some(&cfg), Some("treated"), "initial_consult").is_some(),
        "已治疗→初诊应被拦（initial 态不接受任何 from 迁入）"
    );
    // 跳步：初诊→方案确认（plan_confirmed.allowedFrom 只含 follow_up）。
    assert!(
        check_state_transition(Some(&cfg), Some("initial_consult"), "plan_confirmed").is_some(),
        "初诊→方案确认应被拦（须先复诊）"
    );
}

/// initial 态语义跨域：空 from 只能迁入标 initial:true 的态（非销售域 initial 不叫 new_contact）。
#[test]
fn cross_domain_initial_state_semantics() {
    let cfg = medical_domain_config();
    // 空 from → initial_consult（本域 initial 态）合法。
    assert!(
        check_state_transition(Some(&cfg), None, "initial_consult").is_none(),
        "空 from→本域 initial 态(initial_consult)应合法"
    );
    // 空 from → 非 initial 态被拦（证明引擎读 initial 标志而非写死 new_contact）。
    assert!(
        check_state_transition(Some(&cfg), None, "follow_up").is_some(),
        "空 from→非 initial 态应被拦（引擎不写死 new_contact，读 initial 标志）"
    );
    // 空字符串 from 等同空。
    assert!(
        check_state_transition(Some(&cfg), Some(""), "follow_up").is_some(),
        "空字符串 from→非 initial 态应被拦"
    );
}

/// allowFromAny 语义跨域：标 allowFromAny 的态任何 from 都可迁入（医疗失约类比销售 cooldown）。
#[test]
fn cross_domain_allow_from_any() {
    let cfg = medical_domain_config();
    for from in ["initial_consult", "follow_up", "plan_confirmed", "treated"] {
        assert!(
            check_state_transition(Some(&cfg), Some(from), "missed_appointment").is_none(),
            "{from}→失约(allowFromAny)应合法"
        );
    }
    // 空 from→allowFromAny 态也合法。
    assert!(
        check_state_transition(Some(&cfg), None, "missed_appointment").is_none(),
        "空 from→allowFromAny 态应合法"
    );
}

/// unknown_target 跨域：迁向状态机里不存在的态被 fail-closed 拒绝（防幻影态旁路 policy）。
#[test]
fn cross_domain_unknown_target_rejected() {
    let cfg = medical_domain_config();
    let reason = check_state_transition(Some(&cfg), Some("follow_up"), "nonexistent_stage");
    assert!(reason.is_some(), "迁向不存在的态应被拒（fail-closed）");
    assert!(
        reason.unwrap().contains("unknown_target"),
        "拦截理由须含 unknown_target"
    );
}

/// 跨域隔离：销售态名在医疗 FSM 里是 unknown_target（证明两域状态空间不串）。
#[test]
fn sales_state_keys_are_unknown_in_medical_fsm() {
    let cfg = medical_domain_config();
    // 销售域的 new_contact / solution_fit 在医疗 FSM 里不存在 → unknown_target。
    assert!(
        check_state_transition(Some(&cfg), Some("follow_up"), "solution_fit")
            .unwrap()
            .contains("unknown_target"),
        "销售态 solution_fit 在医疗 FSM 应是 unknown_target"
    );
    assert!(
        check_state_transition(Some(&cfg), None, "new_contact")
            .unwrap()
            .contains("unknown_target"),
        "销售 initial 态 new_contact 在医疗 FSM 应是 unknown_target（本域 initial 是 initial_consult）"
    );
}
