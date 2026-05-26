//! Phase B / B1 / B6：`review_passed` `pressure_risk` 软闸阈值 PBT。
//!
//! 验证 4 个 property（每条默认 256 cases，单文件累计 1024 ≥ 64 plan 门）：
//!
//! 1. **above_threshold_blocks** — `pressure_risk >= runtime.pressure_risk_block_at`
//!    且 ≠ 0 → `review_passed=false`。
//! 2. **zero_pressure_risk_passes_legacy** — `pressure_risk == 0`（reviewer 未填
//!    或老数据反序列化默认）→ 永不参与拦截，`review_passed` 由其它分项决定。
//! 3. **below_threshold_passes** — `pressure_risk ∈ [1, threshold-1]` → 不拦截。
//! 4. **threshold_boundary_is_strict** — 闸语义是 `>=`：
//!    * threshold-1 → pass
//!    * threshold   → block
//!    * threshold+1 → block
//!    且与 humanLike 闸联动（two gates 双触发不会重复打乱结果）。
//!
//! 不接 mongo / mock LLM —— 纯 in-memory `review_passed` 调用。

use proptest::prelude::*;
use wechatagent::agent::{review_passed, DecisionReviewResult, ReviewScores, UserRuntimeParameters};

fn full_pass_review(pressure_risk: i32) -> DecisionReviewResult {
    DecisionReviewResult {
        approved: true,
        scores: ReviewScores {
            human_like: 80,
            emotional_value: 80,
            hallucination_score: 1,
            knowledge_grounding_score: 80,
            pressure_risk,
        },
        ..Default::default()
    }
}

// ── Property 1：above_threshold_blocks ──────────────────────────────────

proptest! {
    /// `pressure_risk ∈ [threshold, 100]` 且非 0 → 拦截。
    /// 0 不在该 property 的覆盖范围（见 zero_pressure_risk_passes_legacy）。
    #[test]
    fn above_threshold_blocks(
        delta in 0i32..=20,
    ) {
        let runtime = UserRuntimeParameters::default();
        let pressure_risk = runtime.pressure_risk_block_at + delta;
        // pressure_risk == 0 走 legacy 豁免路径，本 property 不覆盖。
        prop_assume!(pressure_risk != 0);
        let review = full_pass_review(pressure_risk);
        prop_assert!(
            !review_passed(&review, &runtime),
            "pressure_risk={} threshold={} should block",
            pressure_risk,
            runtime.pressure_risk_block_at
        );
    }
}

// ── Property 2：zero_pressure_risk_passes_legacy ────────────────────────

proptest! {
    /// `pressure_risk == 0`（reviewer 未给分 / 老数据反序列化默认）→ 不参与拦截。
    /// 用其它满分分项的 review，验证 0 不会让 review_passed 退化为 false。
    #[test]
    fn zero_pressure_risk_passes_legacy(
        // 只是为了让 proptest 跑足 case 数。本 property 没有自由变量。
        _seed in 0u64..1024,
    ) {
        let runtime = UserRuntimeParameters::default();
        let review = full_pass_review(0);
        prop_assert!(
            review_passed(&review, &runtime),
            "pressure_risk=0 应当作 reviewer 未填豁免，不参与拦截"
        );
    }
}

// ── Property 3：below_threshold_passes ──────────────────────────────────

proptest! {
    /// `pressure_risk ∈ [1, threshold-1]` 且其它分项满分 → 不拦截。
    #[test]
    fn below_threshold_passes(
        delta in 1i32..=20,
    ) {
        let runtime = UserRuntimeParameters::default();
        let pressure_risk = (runtime.pressure_risk_block_at - delta).max(1);
        // 仅在严格小于阈值（且非 0）的窗口里 prop_assume；threshold==1 这种退化场景跳过。
        prop_assume!(
            pressure_risk >= 1
                && pressure_risk < runtime.pressure_risk_block_at
        );
        let review = full_pass_review(pressure_risk);
        prop_assert!(
            review_passed(&review, &runtime),
            "pressure_risk={} threshold={} should pass",
            pressure_risk,
            runtime.pressure_risk_block_at
        );
    }
}

// ── Property 4：threshold_boundary_is_strict ────────────────────────────

proptest! {
    /// 阈值附近 ±1 的三档：
    ///   * threshold-1 → pass
    ///   * threshold   → block
    ///   * threshold+1 → block
    /// 同时构造一组 humanLike 已经 pass / 已经 block 的 review，验证 pressure_risk
    /// 闸不会"修复"humanLike 闸 —— 也就是双闸是 AND 关系，任一触发就拦截。
    #[test]
    fn threshold_boundary_is_strict(
        emotional_noise in 5i32..=20,
        knowledge_noise in 7i32..=20,
    ) {
        let runtime = UserRuntimeParameters::default();
        let make_review = |pressure_risk: i32, human_like: i32| DecisionReviewResult {
            approved: true,
            scores: ReviewScores {
                human_like,
                emotional_value: runtime.emotional_value_rewrite_below + emotional_noise,
                hallucination_score: 1,
                knowledge_grounding_score: runtime.product_accuracy_block_below + knowledge_noise,
                pressure_risk,
            },
            ..Default::default()
        };

        // ── humanLike 通过的轨：单独检验 pressure_risk 闸 ──
        let human_pass = runtime.human_like_rewrite_below + 5;
        let below = make_review(runtime.pressure_risk_block_at - 1, human_pass);
        let equal = make_review(runtime.pressure_risk_block_at, human_pass);
        let above = make_review(runtime.pressure_risk_block_at + 1, human_pass);

        prop_assert!(
            review_passed(&below, &runtime),
            "below pressure threshold should pass (threshold={})",
            runtime.pressure_risk_block_at
        );
        prop_assert!(
            !review_passed(&equal, &runtime),
            "exactly at pressure threshold should block"
        );
        prop_assert!(
            !review_passed(&above, &runtime),
            "above pressure threshold should block"
        );

        // ── humanLike 不通过的轨：双闸 AND，pressure_risk pass 也救不了 humanLike fail ──
        let human_fail = runtime.human_like_rewrite_below - 1;
        if human_fail >= 0 {
            let pressure_pass_human_fail = make_review(runtime.pressure_risk_block_at - 1, human_fail);
            prop_assert!(
                !review_passed(&pressure_pass_human_fail, &runtime),
                "humanLike fail 不能被 pressure_risk pass 抵消"
            );
        }
    }
}
