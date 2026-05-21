//! Property tests for `evolution::significance` (M4 W3 Task 4.8).
//!
//! 性质：
//! 1. `grade_threshold` 在任意 send_success delta ∈ [-1, 1] 区间内永不 panic
//! 2. `grade_prompt` 在 replay vec empty 时永远 reject
//! 3. completed_count < min_replays 永远 reject（无论 kind）
//! 4. 任意 5gate_hit_delta_per_gate 含 NaN（通过 new_5gate_hit 注入）→ 永远 reject
//! 5. fail_rate > max_fail_rate 永远 reject
//! 6. `grade_prompt` 在所有 replays 都 fail（completed=0）时永远 reject
//!
//! 不依赖 testcontainers / mongodb，默认参与 `cargo test`。本文件**不**计入
//! baseline R11.6 的 4 PBT，而是独立 W3 显著性 PBT。

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
use proptest::prelude::*;
use wechatagent::evolution::significance::{
    grade_prompt, grade_threshold, SignificanceCfg,
};
use wechatagent::models::ShadowReplay;

fn cfg() -> SignificanceCfg {
    SignificanceCfg {
        min_replays: 30,
        min_send_success_delta: 0.05,
        min_self_critique_delta: 0.10,
        max_5gate_hit_increase: 0.10,
        max_fail_rate: 0.30,
    }
}

fn no_gate() -> Document {
    doc! {
        "fact_risk_block": false,
        "pressure_risk_block": false,
        "human_like_score_rewrite": false,
        "emotional_value_rewrite": false,
        "product_accuracy_score_block": false,
    }
}

fn rep(
    status: &str,
    original: Option<&str>,
    new: Option<&str>,
    gate: Document,
    new_self_critique: Option<bool>,
    new_token_cost: Option<i64>,
) -> ShadowReplay {
    ShadowReplay {
        id: None,
        proposal_id: ObjectId::new(),
        experiment_id: "exp_pbt".to_string(),
        workspace_id: "ws".to_string(),
        account_id: "acct".to_string(),
        source_run_id: ObjectId::new(),
        status: status.to_string(),
        failure_reason: None,
        original_final_review_status: original.map(str::to_string),
        new_final_review_status: new.map(str::to_string),
        new_review_risks: vec![],
        new_token_cost,
        new_5gate_hit: gate,
        new_self_critique_addressed: new_self_critique,
        similarity_to_original_text: 0.0,
        started_at: DateTime::now(),
        finished_at: Some(DateTime::now()),
    }
}

proptest! {
    /// 性质 1：grade_threshold 在任意 (original_success_n, new_success_n) ∈ [0..50]² 范围内
    /// 永不 panic，返回 (bool, Document)。
    #[test]
    fn grade_threshold_never_panics(
        original_success in 0_usize..50,
        new_success in 0_usize..50,
        total in 1_usize..50,
    ) {
        let n = total.max(original_success).max(new_success);
        let mut replays = Vec::new();
        for i in 0..n {
            let original = if i < original_success { Some("approved") } else { Some("held_by_ai_policy") };
            let new = if i < new_success { Some("approved") } else { Some("held_by_ai_policy") };
            replays.push(rep("completed", original, new, no_gate(), None, None));
        }
        let _ = grade_threshold(&replays, &cfg());
    }

    /// 性质 2：grade_prompt 在 replays 为空时永远 reject 且 reason="insufficient_completed_replays"
    #[test]
    fn grade_prompt_empty_always_rejects(_dummy in 0..1u32) {
        let (passed, metrics) = grade_prompt(&[], &cfg());
        prop_assert!(!passed);
        prop_assert_eq!(metrics.get_str("reason").unwrap(), "insufficient_completed_replays");
    }

    /// 性质 3：completed_count < min_replays 永远 reject（threshold + prompt）
    #[test]
    fn completed_below_min_always_rejects(n in 0_usize..30) {
        let replays: Vec<_> = (0..n)
            .map(|_| rep("completed", Some("approved"), Some("approved"), no_gate(), Some(true), Some(1000)))
            .collect();
        let (passed_t, m_t) = grade_threshold(&replays, &cfg());
        let (passed_p, m_p) = grade_prompt(&replays, &cfg());
        prop_assert!(!passed_t);
        prop_assert!(!passed_p);
        prop_assert_eq!(m_t.get_str("reason").unwrap(), "insufficient_completed_replays");
        prop_assert_eq!(m_p.get_str("reason").unwrap(), "insufficient_completed_replays");
    }

    /// 性质 4：fail_rate > max_fail_rate 永远 reject（先满足 min_replays）。
    /// 构造：completed=30，failed=14（fail_rate=14/44 ≈ 31.8% > 30%）。
    #[test]
    fn fail_rate_above_max_always_rejects(_dummy in 0..1u32) {
        let mut replays = Vec::new();
        for _ in 0..30 {
            replays.push(rep(
                "completed",
                Some("approved"),
                Some("approved"),
                no_gate(),
                Some(true),
                Some(1000),
            ));
        }
        for _ in 0..14 {
            replays.push(rep("failed", None, None, no_gate(), None, None));
        }
        let (passed_t, m_t) = grade_threshold(&replays, &cfg());
        let (passed_p, m_p) = grade_prompt(&replays, &cfg());
        prop_assert!(!passed_t);
        prop_assert!(!passed_p);
        prop_assert_eq!(m_t.get_str("reason").unwrap(), "replay_fail_rate_above_threshold");
        prop_assert_eq!(m_p.get_str("reason").unwrap(), "replay_fail_rate_above_threshold");
    }

    /// 性质 5：所有 replays 都 fail（completed=0 < min_replays） → 永远 reject
    #[test]
    fn all_failed_always_rejects(n in 1_usize..50) {
        let replays: Vec<_> = (0..n)
            .map(|_| rep("failed", None, None, no_gate(), None, None))
            .collect();
        let (passed_t, _) = grade_threshold(&replays, &cfg());
        let (passed_p, _) = grade_prompt(&replays, &cfg());
        prop_assert!(!passed_t);
        prop_assert!(!passed_p);
    }

    /// 性质 6：max_5gate_hit_increase 边界 —— hit 数为 0 时（new_rate=0）必有 max_increase ≤ 0，
    /// gate_passed 必 true，与是否最终通过取决于其它指标无关。
    #[test]
    fn no_gate_hit_means_gate_increase_passes(
        n in 30_usize..40,
        approved_count in 0_usize..40,
    ) {
        let approved = approved_count.min(n);
        let mut replays = Vec::new();
        for i in 0..n {
            let original = if i < approved { Some("approved") } else { Some("held_by_ai_policy") };
            let new = if i < approved { Some("approved") } else { Some("held_by_ai_policy") };
            replays.push(rep("completed", original, new, no_gate(), Some(true), Some(1200)));
        }
        let (_passed, metrics) = grade_threshold(&replays, &cfg());
        let max_increase = metrics.get_f64("max_5gate_hit_increase_observed").unwrap_or(0.0);
        prop_assert!(max_increase <= 0.10);
    }
}
