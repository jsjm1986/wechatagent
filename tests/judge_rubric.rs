//! R1.1 judge profile 化的纯函数单测宿主。
//!
//! `tests/common/judge.rs` 的 `build_judge_rubric` 派生逻辑是纯函数（不碰 DB/LLM），
//! 但 `tests/common/` 是被各集成测试 `mod common;` 引入的子模块——它的 `#[cfg(test)]
//! mod tests` 不会被任何 `--ignored` 真模型测试触发。本文件作为**轻量宿主**把这些
//! 派生逻辑单测暴露成可本地 `cargo test --test judge_rubric` 直跑的用例（无 Docker、
//! 无真模型、无 env-gate），守住 R1.1 的契约：销售域键集 ⊇ 现有 JUDGE_SYSTEM、
//! 情感域极性翻转（pressureRisk 不 manipulationRisk）。

mod common;

use common::judge::{build_judge_rubric, build_judge_user, JudgeRubric, HARD_GATE_DIMS};
use wechatagent::agent::{default_domain_profile, example_emotional_companion_profile};

fn dims_of(r: &JudgeRubric) -> Vec<&str> {
    r.dims.iter().map(|s| s.as_str()).collect()
}

#[test]
fn sales_default_superset_of_legacy_six_keys() {
    let r = build_judge_rubric(&default_domain_profile("ws"));
    let dims = dims_of(&r);
    for key in [
        "humanLike",
        "emotionalValue",
        "helpfulness",
        "manipulationRisk",
        "factualRestraint",
        "overall",
    ] {
        assert!(
            dims.contains(&key),
            "销售域必须含现有 JUDGE_SYSTEM 维「{key}」(基准不破)，dims={dims:?}"
        );
    }
    assert!(!dims.contains(&"pressureRisk"), "销售域不应有陪伴极性维");
}

#[test]
fn companion_polarity_flips() {
    let r = build_judge_rubric(&example_emotional_companion_profile("ws"));
    let dims = dims_of(&r);
    assert!(dims.contains(&"pressureRisk"), "陪伴域必须含 pressureRisk");
    assert!(
        !dims.contains(&"manipulationRisk"),
        "陪伴域必须不含销售极性维 manipulationRisk（标尺随域翻转），dims={dims:?}"
    );
    assert!(dims.contains(&"personaConsistency"));
    assert!(dims.contains(&"scenarioAppropriateness"));
}

#[test]
fn hard_gate_dims_present_in_both_domains() {
    let sales = build_judge_rubric(&default_domain_profile("ws"));
    let comp = build_judge_rubric(&example_emotional_companion_profile("ws"));
    for key in HARD_GATE_DIMS {
        assert!(dims_of(&sales).contains(&key), "销售缺硬闸维 {key}");
        assert!(dims_of(&comp).contains(&key), "陪伴缺硬闸维 {key}");
    }
}

#[test]
fn two_domains_distinct_systems() {
    let sales = build_judge_rubric(&default_domain_profile("ws"));
    let comp = build_judge_rubric(&example_emotional_companion_profile("ws"));
    assert_ne!(sales.system, comp.system, "两域 system 应有实质差异");
    assert_ne!(sales.dims, comp.dims);
}

#[test]
fn companion_system_injects_prompt_fragment_context() {
    let r = build_judge_rubric(&example_emotional_companion_profile("ws"));
    assert!(
        r.system.contains("不等于施压") || r.system.contains("不是销售"),
        "情感域 system 应注入 prompt_fragment 陪伴语境"
    );
}

#[test]
fn judge_user_contains_all_three_fields() {
    let u = build_judge_user("场景X", "用户说Y", "回复Z");
    assert!(u.contains("场景X") && u.contains("用户说Y") && u.contains("回复Z"));
}

#[test]
fn output_format_lists_dims_as_json_keys() {
    let r = build_judge_rubric(&default_domain_profile("ws"));
    // system 末尾的「键固定为」段应列出所有 dims + verdict。
    assert!(r.system.contains("verdict"));
    for d in &r.dims {
        assert!(
            r.system.contains(d.as_str()),
            "system 应在键清单列出维度 {d}"
        );
    }
}
