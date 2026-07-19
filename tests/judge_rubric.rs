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

// ── R1.2 失败分级（用 mock provider 确定性验证 gate 行为，无需真模型）──────────
use async_trait::async_trait;
use common::judge::{run_judge_graded, JudgeGate};
use std::sync::atomic::{AtomicUsize, Ordering};
use wechatagent::error::{AppError, AppResult};
use wechatagent::llm::{ChatUsage, LlmJsonResult, LlmProvider};

/// 永远返回指定错误的 mock judge（测 gate 失败处置）。
struct FailingJudge {
    kind: String,
    detail: String,
    calls: AtomicUsize,
}
#[async_trait]
impl LlmProvider for FailingJudge {
    async fn generate_json(&self, _s: &str, _u: &str) -> AppResult<serde_json::Value> {
        unreachable!()
    }
    async fn generate_json_with_usage(&self, _s: &str, _u: &str) -> AppResult<LlmJsonResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(AppError::LlmUnavailable {
            kind: self.kind.clone(),
            retry_count: 0,
            detail: self.detail.clone(),
            hint: String::new(),
        })
    }
}

/// 永远返回合法打分的 mock judge（测成功路径）。
struct ScoringJudge;
#[async_trait]
impl LlmProvider for ScoringJudge {
    async fn generate_json(&self, _s: &str, _u: &str) -> AppResult<serde_json::Value> {
        unreachable!()
    }
    async fn generate_json_with_usage(&self, _s: &str, _u: &str) -> AppResult<LlmJsonResult> {
        Ok(LlmJsonResult {
            value: serde_json::json!({
                "humanLike": {"score": 8, "reason": "x"},
                "emotionalValue": {"score": 7, "reason": "x"},
                "overall": {"score": 8, "reason": "x"},
                "verdict": "ok"
            }),
            usage: ChatUsage::default(),
            latency_ms: 0,
            model: "mock".into(),
            retry_count: 0,
        })
    }
}

fn set_judge_env() {
    std::env::set_var("REAL_LLM_JUDGE", "1");
}

#[tokio::test]
async fn observe_only_swallows_transient_failure() {
    set_judge_env();
    let r = build_judge_rubric(&default_domain_profile("ws"));
    let judge = FailingJudge {
        kind: "http_5xx".into(),
        detail: "LLM HTTP 503".into(),
        calls: AtomicUsize::new(0),
    };
    // ObserveOnly：端点抖动(5xx)全失败 → 返 None，绝不 panic。
    let out = run_judge_graded(&judge, &r, "t", "in", "reply", 2, JudgeGate::ObserveOnly).await;
    assert!(out.is_none(), "ObserveOnly 下抖动失败应返 None 不 panic");
}

#[tokio::test]
#[should_panic(expected = "唯一质量门")]
async fn quality_gate_fails_when_all_samples_fail() {
    set_judge_env();
    let r = build_judge_rubric(&default_domain_profile("ws"));
    let judge = FailingJudge {
        kind: "http_5xx".into(),
        detail: "LLM HTTP 503".into(),
        calls: AtomicUsize::new(0),
    };
    // QualityGate：judge 全失败 → panic（不静默绿）。
    let _ = run_judge_graded(&judge, &r, "t", "in", "reply", 2, JudgeGate::QualityGate).await;
}

#[tokio::test]
#[should_panic(expected = "端点配错")]
async fn endpoint_misconfig_panics_even_observe_only() {
    set_judge_env();
    let r = build_judge_rubric(&default_domain_profile("ws"));
    let judge = FailingJudge {
        kind: "endpoint_not_found".into(),
        detail: "LLM HTTP 404".into(),
        calls: AtomicUsize::new(0),
    };
    // R0.3：端点配错(404/405)即便 ObserveOnly 也 panic，堵漏 /v1 假绿。
    let _ = run_judge_graded(&judge, &r, "t", "in", "reply", 2, JudgeGate::ObserveOnly).await;
}

#[tokio::test]
async fn account_level_402_not_misconfig_observe_swallows() {
    set_judge_env();
    let r = build_judge_rubric(&default_domain_profile("ws"));
    let judge = FailingJudge {
        kind: "http_4xx".into(),
        detail: "LLM HTTP 402: insufficient balance".into(),
        calls: AtomicUsize::new(0),
    };
    // 账户级 402 不算端点配错 → ObserveOnly 照常吞，返 None 不 panic。
    let out = run_judge_graded(&judge, &r, "t", "in", "reply", 1, JudgeGate::ObserveOnly).await;
    assert!(
        out.is_none(),
        "402 账户级应按 gate 处置，ObserveOnly 返 None"
    );
}

#[tokio::test]
async fn scoring_judge_returns_medians() {
    set_judge_env();
    let r = build_judge_rubric(&default_domain_profile("ws"));
    let out = run_judge_graded(
        &ScoringJudge,
        &r,
        "t",
        "in",
        "reply",
        3,
        JudgeGate::QualityGate,
    )
    .await
    .expect("成功打分应返 Some");
    assert_eq!(out.medians.get("humanLike"), Some(&8));
    assert!(out.attempted >= 1);
}
