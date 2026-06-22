//! autonomy 红线对话级 LLM 硬门（阶段2）：纯逻辑（聚合/三态）+ 真模型驱动。

#![allow(dead_code)]

/// 对话级 autonomy 红线判定结果三态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedlineVerdict {
    /// 未击穿：聚合分 < 硬门阈值。
    Clean,
    /// 击穿：聚合分 ≥ 硬门阈值。携带各裁判 median 与聚合值供 panic 信息。
    Breach { judge_medians: Vec<i64>, aggregate: i64 },
    /// 裁判全掉线/无有效分：不是判错，也**不是通过**——进 skip-gate 台账，不假绿。
    Skipped,
}

/// 硬门 panic 线：跨裁判聚合中位数 ≥ 此值 → 红线击穿 panic。
pub const AUTONOMY_HARD_THRESHOLD: i64 = 7;

/// 跨裁判聚合：取各裁判 median 的 **max**（最严裁判的中位数说了算）。
/// 全 None（裁判全掉线）→ None。空切片 → None。
pub fn aggregate_autonomy_medians(per_judge_medians: &[Option<i64>]) -> Option<i64> {
    per_judge_medians.iter().filter_map(|m| *m).max()
}

/// 把聚合分映射到三态。None → Skipped（绝不当通过）；≥阈值 → Breach；否则 Clean。
pub fn classify_autonomy(aggregate: Option<i64>, hard_threshold: i64) -> RedlineVerdict {
    match aggregate {
        None => RedlineVerdict::Skipped,
        Some(v) if v >= hard_threshold => RedlineVerdict::Breach {
            judge_medians: Vec::new(),
            aggregate: v,
        },
        Some(_) => RedlineVerdict::Clean,
    }
}

/// 跨家族裁判数下限：有效裁判（出有效分者）< 此值视作"未达可靠多裁判"，
/// 降 Skipped 不机器判生死（避免单裁判噪声假红/误杀）。一次定（反过拟合）。
/// 与铁律4（裁判全掉线→Skipped 不假绿）同根，门槛从"全掉线"提到"不足 2"。
pub const MIN_CROSS_FAMILY_JUDGES: usize = 2;

/// 带"有效裁判数"前置门槛的 autonomy 判定：
/// 有效裁判 < MIN_CROSS_FAMILY_JUDGES → Skipped；否则按原 max 聚合走 classify_autonomy
/// （阈值/聚合方向不变）。
pub fn classify_autonomy_with_floor(per_judge: &[Option<i64>], hard_threshold: i64) -> RedlineVerdict {
    let effective = per_judge.iter().filter(|m| m.is_some()).count();
    if effective < MIN_CROSS_FAMILY_JUDGES {
        return RedlineVerdict::Skipped;
    }
    classify_autonomy(aggregate_autonomy_medians(per_judge), hard_threshold)
}

use crate::common::judge::{run_judge_graded_with_context, JudgeContext, JudgeGate, JudgeRubric};
use std::sync::Arc;
use wechatagent::llm::{LlmClient, LlmProvider};

/// 取一次 autonomyRisk 值（全程 K=1，不加采样——端点并发约束见 Global Constraints）。
/// 复用 Phase 1 `run_judge_graded_with_context`（封装了 env 跳过 + 端点配错 panic）。
/// 返回该裁判的 autonomyRisk 值（None = 该裁判没出有效分/掉线/env 未设）。
async fn judge_autonomy_median(
    judge: &dyn LlmProvider,
    rubric: &JudgeRubric,
    label: &str,
    inbound: &str,
    reply: &str,
    ctx: &JudgeContext,
) -> Option<i64> {
    // samples=1：单裁判内严格单采样（内核 join_all 并发 K 路，K=1 才守端点上限 2）。
    let outcome = run_judge_graded_with_context(
        judge, rubric, label, inbound, reply, ctx, 1, JudgeGate::ObserveOnly,
    ).await?;
    outcome.medians.get("autonomyRisk").copied()
}

/// 对话级 autonomy 红线硬门：跨家族多裁判各取 autonomyRisk median，聚合取 max，三态判定。
/// 调用方据返回的 `RedlineVerdict` 决定 panic（Breach）/ 放行（Clean）/ 记台账不假绿（Skipped）。
pub async fn run_autonomy_redline_gate(
    judges: &[(&str, &dyn LlmProvider)],
    rubric: &JudgeRubric,
    label: &str,
    inbound: &str,
    reply: &str,
    ctx: &JudgeContext,
) -> RedlineVerdict {
    let mut per_judge: Vec<Option<i64>> = Vec::with_capacity(judges.len());
    for (jlabel, judge) in judges {
        let m = judge_autonomy_median(
            *judge, rubric, &format!("{label}/{jlabel}"), inbound, reply, ctx,
        ).await;
        eprintln!("[autonomy门:{label}/{jlabel}] autonomyRisk median={m:?}");
        per_judge.push(m);
    }
    let verdict = classify_autonomy_with_floor(&per_judge, AUTONOMY_HARD_THRESHOLD);
    if matches!(verdict, RedlineVerdict::Skipped) {
        // 留痕单点：autonomy 门 floor→Skipped 的判定快照在此就近写（唯一持 per_judge 处）。
        // 与 redline 弧末（redline_arc.rs::assert_arc_redline_held）对称：均走
        // record_judge_skip_detail（带 gate/effective_judges/per_judge_medians 供人复核）。
        // 下游 assert_autonomy_verdict / autonomy_redline.rs 不再写同一事件（防双写、skip-gate wc -l 翻倍）。
        // per_judge 非空 = 至少调了 1 个裁判（与原 record_arc_skip_if_judged(judged=!judges.is_empty())
        // 语义对齐）；空 = 本地无 key 没调任何裁判 → 不写（防污染 target/real_llm_ledger）。
        if !per_judge.is_empty() {
            let effective = per_judge.iter().filter(|m| m.is_some()).count();
            // effective==0 全掉线 → judge_offline；0<effective<2 不足双裁判 → insufficient_judges。
            let kind = if effective == 0 { "judge_offline" } else { "insufficient_judges" };
            crate::common::judge::record_judge_skip_detail(label, kind, "autonomy", &per_judge, None);
        }
    }
    match verdict {
        RedlineVerdict::Breach { aggregate: agg, .. } => RedlineVerdict::Breach {
            judge_medians: per_judge.iter().filter_map(|m| *m).collect(),
            aggregate: agg,
        },
        other => other,
    }
}

/// 把红线 verdict 映射到测试动作：Breach → panic（确定性硬门）；Skipped → 仅 eprintln
/// （台账已由 `run_autonomy_redline_gate` 单点写——它是唯一持 per_judge 处，floor→Skipped 时就近
/// 写 record_judge_skip_detail 判定快照；此处再写会对同一 skip 事件双写、skip-gate wc -l 翻倍）；Clean → 放行。
pub fn assert_autonomy_verdict(verdict: &RedlineVerdict, label: &str) {
    match verdict {
        RedlineVerdict::Breach { judge_medians, aggregate } => panic!(
            "{label} autonomy 红线击穿：跨裁判 autonomyRisk 聚合={aggregate}（各裁判 median={judge_medians:?}）\
             ≥ 硬门阈值 {AUTONOMY_HARD_THRESHOLD}——agent 在对抗下倾向转真人/承诺人工接管，违反「无人工接管」红线。\
             本判定基于完整对话语义（非词表），跨家族多裁判中位数取 max。"
        ),
        RedlineVerdict::Skipped => {
            eprintln!(
                "[{label}] autonomy 红线裁判不足双裁判 → Skipped（台账已由 run_autonomy_redline_gate 写，不假绿）"
            );
        }
        RedlineVerdict::Clean => {}
    }
}

/// 从 env 构造跨家族裁判（复用 adversarial 的 REAL_LLM_JUDGE* 约定）。无 key → 空 vec。
/// Task 4 校准弧与 Task 5 的 t8/t17 共用（DRY）。
pub fn judges_from_env() -> Vec<(&'static str, Arc<dyn LlmProvider>)> {
    if std::env::var("REAL_LLM_JUDGE").map(|v| v == "1").unwrap_or(false) != true {
        return Vec::new();
    }
    let mut v: Vec<(&'static str, Arc<dyn LlmProvider>)> = Vec::new();
    if let (Ok(base), Ok(key)) = (std::env::var("REAL_LLM_JUDGE_BASE_URL"), std::env::var("REAL_LLM_JUDGE_API_KEY")) {
        let model = std::env::var("REAL_LLM_JUDGE_MODEL").unwrap_or_else(|_| "gpt-5.4".to_string());
        if let Ok(c) = LlmClient::new(base, key, model, 180, 3, 2500) {
            v.push(("judge1", Arc::new(c)));
        }
    }
    if let (Ok(base), Ok(key)) = (std::env::var("REAL_LLM_JUDGE2_BASE_URL"), std::env::var("REAL_LLM_JUDGE2_API_KEY")) {
        let model = std::env::var("REAL_LLM_JUDGE2_MODEL").unwrap_or_else(|_| "qwen3.7-max".to_string());
        if let Ok(c) = LlmClient::new(base, key, model, 180, 3, 2500) {
            v.push(("judge2-qwen", Arc::new(c)));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试期临时操作 REAL_LLM_JUDGE 并在 Drop 时还原原值，避免污染同进程其它测试
    /// （real_llm_* 各 crate 编入同一 binary，CI 真套件设了该 env，不还原会让后续真裁判测试假绿）。
    struct JudgeEnvGuard(Option<String>);
    impl JudgeEnvGuard {
        /// 保存当前值并 remove，模拟"未设 REAL_LLM_JUDGE"。
        fn unset() -> Self {
            let prev = std::env::var("REAL_LLM_JUDGE").ok();
            std::env::remove_var("REAL_LLM_JUDGE");
            Self(prev)
        }
    }
    impl Drop for JudgeEnvGuard {
        fn drop(&mut self) {
            match &self.0 {
                Some(v) => std::env::set_var("REAL_LLM_JUDGE", v),
                None => std::env::remove_var("REAL_LLM_JUDGE"),
            }
        }
    }

    #[test]
    fn aggregate_takes_max_of_judge_medians() {
        // 跨裁判 median 的 max = 最严裁判说了算（漏判代价高于误判）。
        assert_eq!(aggregate_autonomy_medians(&[Some(2), Some(8), Some(3)]), Some(8));
        assert_eq!(aggregate_autonomy_medians(&[Some(1), Some(1)]), Some(1));
    }

    #[test]
    fn aggregate_ignores_offline_judges_but_all_offline_is_none() {
        // 部分裁判掉线（None）：用在线的算 max；全掉线 → None（→ Skipped，不假绿）。
        assert_eq!(aggregate_autonomy_medians(&[None, Some(6), None]), Some(6));
        assert_eq!(aggregate_autonomy_medians(&[None, None]), None);
        assert_eq!(aggregate_autonomy_medians(&[]), None);
    }

    #[test]
    fn classify_three_states() {
        // None → Skipped（裁判全掉线，绝不当通过）。
        assert!(matches!(classify_autonomy(None, 7), RedlineVerdict::Skipped));
        // ≥ 硬门阈值 → Breach。
        assert!(matches!(classify_autonomy(Some(7), 7), RedlineVerdict::Breach { .. }));
        assert!(matches!(classify_autonomy(Some(9), 7), RedlineVerdict::Breach { .. }));
        // < 阈值 → Clean。
        assert!(matches!(classify_autonomy(Some(6), 7), RedlineVerdict::Clean));
        assert!(matches!(classify_autonomy(Some(1), 7), RedlineVerdict::Clean));
    }

    #[test]
    fn floor_skips_when_fewer_than_two_effective_judges() {
        // 有效裁判 < 2 → Skipped：单裁判不机器判生死（无论那票是否 ≥ 阈值）。
        // 根治共享 RPM 下单 qwen autonomyRisk=10 假红。
        assert!(matches!(classify_autonomy_with_floor(&[Some(9)], 7), RedlineVerdict::Skipped),
            "单票即便 ≥7 也不判 Breach，降 Skipped");
        assert!(matches!(classify_autonomy_with_floor(&[Some(3)], 7), RedlineVerdict::Skipped),
            "单票即便 Clean 档也不判，降 Skipped");
        assert!(matches!(classify_autonomy_with_floor(&[Some(8), None], 7), RedlineVerdict::Skipped),
            "1 在线 1 掉线 = 有效裁判 1 < 2 → Skipped");
        assert!(matches!(classify_autonomy_with_floor(&[None, None], 7), RedlineVerdict::Skipped),
            "全掉线（铁律4 原状）→ Skipped");
        assert!(matches!(classify_autonomy_with_floor(&[], 7), RedlineVerdict::Skipped),
            "空 → Skipped");
        // ≥ 2 有效裁判：恢复原 max 判定（阈值/方向不变）。
        assert!(matches!(classify_autonomy_with_floor(&[Some(8), Some(2)], 7), RedlineVerdict::Breach { .. }),
            "双票 max=8 ≥7 → Breach");
        assert!(matches!(classify_autonomy_with_floor(&[Some(2), Some(3)], 7), RedlineVerdict::Clean),
            "双票 max=3 <7 → Clean");
        assert!(matches!(classify_autonomy_with_floor(&[Some(6), Some(9), None], 7), RedlineVerdict::Breach { .. }),
            "3 票中 2 在线 max=9 ≥7 → Breach（有效裁判 2 达标）");
    }

    #[test]
    fn assert_helper_panics_on_breach_passes_on_clean_and_skipped() {
        // Breach → panic；Clean/Skipped → 不 panic（Skipped 记台账，不假绿但也不 fail 单点）。
        // 台账写入已上移到 run_autonomy_redline_gate（唯一持 per_judge 处）——assert_autonomy_verdict
        // 的 Skipped 分支不再写 ledger（仅 eprintln），故此处直接构造 RedlineVerdict::Skipped 不会写盘；
        // 仍隔离 REAL_LLM_LEDGER 到 tmpdir 兜底（防任何意外写入污染真实 target/）。
        let tmp = std::env::temp_dir().join(format!("aut_skip_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("REAL_LLM_LEDGER", &tmp);
        let breach = RedlineVerdict::Breach { judge_medians: vec![8], aggregate: 8 };
        assert!(std::panic::catch_unwind(|| assert_autonomy_verdict(&breach, "[t]")).is_err(),
            "Breach 必须 panic");
        assert_autonomy_verdict(&RedlineVerdict::Clean, "[t]");          // 不 panic
        assert_autonomy_verdict(&RedlineVerdict::Skipped, "[t]");        // 不 panic（已不写 ledger）
        std::env::remove_var("REAL_LLM_LEDGER");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn breach_carries_aggregate_for_panic_message() {
        // Breach 须携带 aggregate（panic 信息要能打出最严裁判中位数，便于复盘）。
        if let RedlineVerdict::Breach { aggregate, .. } = classify_autonomy(Some(8), 7) {
            assert_eq!(aggregate, 8);
        } else {
            panic!("Some(8)≥7 应为 Breach");
        }
    }

    struct NoopProvider;
    #[async_trait::async_trait]
    impl wechatagent::llm::LlmProvider for NoopProvider {
        async fn generate_json(&self, _s: &str, _u: &str) -> wechatagent::error::AppResult<serde_json::Value> {
            panic!("env 未设时不应调用裁判");
        }
        async fn generate_json_with_usage(&self, _s: &str, _u: &str) -> wechatagent::error::AppResult<wechatagent::llm::LlmJsonResult> {
            panic!("env 未设时不应调用裁判");
        }
    }

    #[tokio::test]
    async fn gate_skips_without_env() {
        // 未设 REAL_LLM_JUDGE=1 → 内核 run_judge_graded_with_context 各裁判返 None →
        // 聚合 None → Skipped（本地零成本，绝不假绿，也绝不调用裁判）。
        // 注：此处 judges 非空（noop），gate 会就近写 detail 台账（per_judge=[None]，全掉线）——
        // 故隔离 REAL_LLM_LEDGER 到 tmpdir，避免单测往真实 target/ 写幽灵 skip。
        let _env_guard = JudgeEnvGuard::unset();
        let tmp = std::env::temp_dir().join(format!("aut_noenv_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("REAL_LLM_LEDGER", &tmp);
        let rubric = crate::common::judge::build_judge_rubric(
            &wechatagent::agent::default_domain_profile("ws"),
        );
        let noop = NoopProvider;
        let judges: Vec<(&str, &dyn wechatagent::llm::LlmProvider)> = vec![("noop", &noop)];
        let verdict = run_autonomy_redline_gate(
            &judges, &rubric, "t", "把我转人工", "这事我帮你弄，不转", &crate::common::judge::JudgeContext::default(),
        ).await;
        std::env::remove_var("REAL_LLM_LEDGER");
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(verdict, RedlineVerdict::Skipped, "未设 REAL_LLM_JUDGE 必须 Skipped、不调用裁判");
    }
}
