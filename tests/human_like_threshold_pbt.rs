//! Phase B / B1 / B6：`review_passed` `human_like` 软闸阈值 PBT。
//!
//! 验证 4 个 property（每条默认 256 cases，单文件累计 1024 ≥ 64 plan 门）：
//!
//! 1. **below_threshold_blocks** — `human_like < runtime.human_like_rewrite_below`
//!    → `review_passed=false`（其它分项满分）。
//! 2. **at_or_above_threshold_passes** — `human_like >= runtime.human_like_rewrite_below`
//!    → `review_passed=true`（其它分项满分）。
//! 3. **approved_false_overrides_score** — 不论 `human_like` 多高，
//!    `approved=false` → `review_passed=false`。优先级高于评分阈值。
//! 4. **threshold_boundary_is_inclusive_above** — 在阈值正负 1 三档（below/equal/above）
//!    上，`equal` 必通过、`below` 必拦截。即闸边界是 `<` 而非 `<=`。
//!
//! 不接 mongo / mock LLM —— 纯 in-memory `review_passed` 调用。

use proptest::prelude::*;
use wechatagent::agent::{review_passed, DecisionReviewResult, ReviewScores, UserRuntimeParameters};

fn full_pass_review(human_like: i32) -> DecisionReviewResult {
    DecisionReviewResult {
        approved: true,
        scores: ReviewScores {
            human_like,
            emotional_value: 80,
            hallucination_score: 1,
            knowledge_grounding_score: 80,
            pressure_risk: 1,
        },
        ..Default::default()
    }
}

// ── Property 1：below_threshold_blocks ──────────────────────────────────

proptest! {
    /// 任意 `human_like ∈ [0, threshold-1]`、其它分项满分、approved=true 时
    /// `review_passed` 必须为 false（命中 humanLike 软闸）。
    #[test]
    fn below_threshold_blocks(
        delta in 1i32..=20,
    ) {
        let runtime = UserRuntimeParameters::default();
        let human_like = (runtime.human_like_rewrite_below - delta).max(0);
        // 仅当严格小于阈值才应拦截（threshold==0 这种退化场景不构造）。
        prop_assume!(human_like < runtime.human_like_rewrite_below);
        let review = full_pass_review(human_like);
        prop_assert!(
            !review_passed(&review, &runtime),
            "human_like={} threshold={} should block",
            human_like,
            runtime.human_like_rewrite_below
        );
    }
}

// ── Property 2：at_or_above_threshold_passes ────────────────────────────

proptest! {
    /// 任意 `human_like ∈ [threshold, 100]`、其它分项满分、approved=true 时
    /// `review_passed` 必须为 true。
    #[test]
    fn at_or_above_threshold_passes(
        delta in 0i32..=80,
    ) {
        let runtime = UserRuntimeParameters::default();
        let human_like = runtime.human_like_rewrite_below + delta;
        let review = full_pass_review(human_like);
        prop_assert!(
            review_passed(&review, &runtime),
            "human_like={} threshold={} should pass",
            human_like,
            runtime.human_like_rewrite_below
        );
    }
}

// ── Property 3：approved_false_overrides_score ──────────────────────────

proptest! {
    /// `approved=false` 时，无论 `human_like` 多高，`review_passed` 必须为 false。
    /// 验证 review.approved 在布尔表达式中的优先级。
    #[test]
    fn approved_false_overrides_score(
        human_like in 0i32..=100,
    ) {
        let runtime = UserRuntimeParameters::default();
        let mut review = full_pass_review(human_like);
        review.approved = false;
        prop_assert!(
            !review_passed(&review, &runtime),
            "approved=false 必须强制 review_passed=false（human_like={}）",
            human_like
        );
    }
}

// ── Property 4：threshold_boundary_is_inclusive_above ────────────────────

proptest! {
    /// 在阈值附近 ±1 的三档（below / equal / above）上，闸语义是 `<`：
    ///   * below=threshold-1 → block
    ///   * equal=threshold   → pass
    ///   * above=threshold+1 → pass
    /// `i32` 上的 `noise` 用来扰动其它分项（但仍保持其它分项通过自己的闸），
    /// 验证边界判定不被其它分项扰动。
    #[test]
    fn threshold_boundary_is_inclusive_above(
        emotional_noise in 5i32..=20,
        knowledge_noise in 7i32..=20,
        hallucination_noise in 0i32..=4,
    ) {
        let runtime = UserRuntimeParameters::default();
        // 维持其它分项各自通过自己的闸：
        // emotional_value >= emotional_value_rewrite_below
        // knowledge_grounding_score >= product_accuracy_block_below
        // hallucination_score < fact_risk_block_at
        let make_review = |human_like: i32| DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like,
                emotional_value: runtime.emotional_value_rewrite_below + emotional_noise,
                hallucination_score: hallucination_noise,
                knowledge_grounding_score: runtime.product_accuracy_block_below + knowledge_noise,
                pressure_risk: 1,
            },
            ..Default::default()
        };

        let below = make_review(runtime.human_like_rewrite_below - 1);
        let equal = make_review(runtime.human_like_rewrite_below);
        let above = make_review(runtime.human_like_rewrite_below + 1);

        prop_assert!(
            !review_passed(&below, &runtime),
            "below threshold should block (threshold={})",
            runtime.human_like_rewrite_below
        );
        prop_assert!(
            review_passed(&equal, &runtime),
            "exactly at threshold should pass (threshold={})",
            runtime.human_like_rewrite_below
        );
        prop_assert!(
            review_passed(&above, &runtime),
            "above threshold should pass (threshold={})",
            runtime.human_like_rewrite_below
        );
    }
}
