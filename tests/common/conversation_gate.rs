//! 对话级总评跨裁判聚合（阶段3）：纯逻辑（多维 median-of-max）+ 真模型驱动入口。
//! 照 autonomy_gate.rs 结构,区别=多维(7 个 arc 维各一 verdict)而非单维。

#![allow(dead_code)]

use std::sync::Arc;

use crate::common::judge::{
    build_conversation_rubric, run_graded_samples, JudgeContext, JudgeGate, CONVERSATION_DIMS,
};
use wechatagent::llm::LlmProvider;
use wechatagent::models::DomainProfile;

/// 一个 arc 维度的跨裁判判定。
#[derive(Debug, Clone)]
pub struct ConversationVerdict {
    pub dim: String,
    /// 跨裁判 median 的 max（最严裁判中位数）；None=该维全掉线。
    pub aggregate: Option<i64>,
    pub judge_medians: Vec<i64>,
}

/// 整段对话总评：所有 arc 维的跨裁判判定。
#[derive(Debug, Clone)]
pub struct ConversationReport {
    pub per_dim: Vec<ConversationVerdict>,
    /// 至少一维出分（全掉线 → false → 调用方按 Skipped 处置,不假绿）。
    pub any_scored: bool,
}

/// 跨裁判同一维 median 取 max（最严裁判说了算）。**仅用于「越高越坏/抓高端」维**
/// （如 pressure_arc 上限门）；「越高越好/抓低端」维须用 report_dim_min（取 min），
/// 否则一个宽松裁判给高分即掩盖低端退化=漏判。参见 redline_arc.rs:17-22。全 None → None。
pub fn aggregate_dim_medians(per_judge: &[Option<i64>]) -> Option<i64> {
    per_judge.iter().filter_map(|m| *m).max()
}

/// 从 report 取某维聚合分（不存在/未出分 → None）。
pub fn report_dim(report: &ConversationReport, dim: &str) -> Option<i64> {
    report.per_dim.iter().find(|v| v.dim == dim).and_then(|v| v.aggregate)
}

/// 从 report 取某维跨裁判 median 的 **min**（最严裁判=给最低分者）。用于「越高越好+抓低端」
/// 的维（如 overall_progress 地板门）——取 min 才「宁可误判不可漏判」，与 redline_arc.rs:17-22
/// 对 redlineHeld 取 min 同理。**不要**对这类维用走 max 的 report_dim/aggregate_dim_medians（漏判）。
/// 维不存在/judge_medians 空 → None。
pub fn report_dim_min(report: &ConversationReport, dim: &str) -> Option<i64> {
    report.per_dim.iter().find(|v| v.dim == dim)
        .and_then(|v| v.judge_medians.iter().copied().min())
}

/// 对话级总评：跨家族多裁判各对整段 transcript 打 7 arc 维分（全程 K=1,单裁判内单采样）,
/// 每维跨裁判聚合取 max。返回所有维的 report 供调用方按维度做硬/软判定。
///
/// - `judges`：跨家族裁判（复用 judges_from_env）。空 → any_scored=false（Skipped）。
/// - `profile`：派生对话级 rubric。
/// - `transcript`：完整多轮对话（已渲染成「客户/助理」标注的字符串）。
/// - `gate`：透传给底层 run_graded_samples（QualityGate=全失败 panic;ObserveOnly=返 None）。
pub async fn run_conversation_judge(
    judges: &[(&str, &dyn LlmProvider)],
    profile: &DomainProfile,
    label: &str,
    transcript: &str,
    gate: JudgeGate,
) -> ConversationReport {
    let rubric = build_conversation_rubric(profile);
    // 对话级 user = 直接把整段 transcript 作为「待评对话」。复用 JudgeContext.transcript 的语义块。
    let ctx = JudgeContext { transcript: Some(transcript.to_string()), ..Default::default() };
    let user = crate::common::judge::build_judge_user_with_context(
        label, "（对话级总评,无单条 inbound）", "（见上方完整对话）", &ctx,
    );

    // 每维收集各裁判 median。
    let dims: Vec<String> = CONVERSATION_DIMS.iter().map(|s| s.to_string()).collect();
    let mut per_judge_by_dim: Vec<Vec<Option<i64>>> = vec![Vec::new(); dims.len()];
    for (jlabel, judge) in judges {
        let outcome = run_graded_samples(
            *judge, &rubric.system, &user, &dims, &format!("{label}/{jlabel}"), 1, gate,
        ).await;
        for (di, d) in dims.iter().enumerate() {
            let m = outcome.as_ref().and_then(|o| o.medians.get(d).copied());
            per_judge_by_dim[di].push(m);
        }
        eprintln!("[对话级总评:{label}/{jlabel}] medians={:?}",
            outcome.as_ref().map(|o| &o.medians));
    }

    let per_dim: Vec<ConversationVerdict> = dims.iter().enumerate().map(|(di, d)| {
        let aggregate = aggregate_dim_medians(&per_judge_by_dim[di]);
        ConversationVerdict {
            dim: d.clone(),
            aggregate,
            judge_medians: per_judge_by_dim[di].iter().filter_map(|m| *m).collect(),
        }
    }).collect();
    let any_scored = per_dim.iter().any(|v| v.aggregate.is_some());
    ConversationReport { per_dim, any_scored }
}

/// 便捷：从 env 构造跨家族裁判（复用 autonomy_gate 同款约定,DRY）。无 key → 空 vec。
pub fn judges_from_env() -> Vec<(&'static str, Arc<dyn LlmProvider>)> {
    crate::common::autonomy_gate::judges_from_env()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_takes_max_across_judges() {
        // 跨裁判同一维 median 取 max（最严裁判说了算，与 autonomy 同口径）。
        assert_eq!(aggregate_dim_medians(&[Some(2), Some(8), Some(3)]), Some(8));
        assert_eq!(aggregate_dim_medians(&[None, Some(6), None]), Some(6));
        assert_eq!(aggregate_dim_medians(&[None, None]), None);
        assert_eq!(aggregate_dim_medians(&[]), None);
    }

    #[test]
    fn report_dim_reads_aggregate() {
        let report = ConversationReport {
            per_dim: vec![
                ConversationVerdict { dim: "overall_progress".into(), aggregate: Some(3), judge_medians: vec![3] },
                ConversationVerdict { dim: "pressure_arc".into(), aggregate: None, judge_medians: vec![] },
            ],
            any_scored: true,
        };
        assert_eq!(report_dim(&report, "overall_progress"), Some(3));
        assert_eq!(report_dim(&report, "pressure_arc"), None);
        assert_eq!(report_dim(&report, "不存在"), None);
    }

    #[test]
    fn report_dim_min_takes_min_across_judges() {
        let report = ConversationReport {
            per_dim: vec![
                ConversationVerdict { dim: "overall_progress".into(), aggregate: Some(8), judge_medians: vec![8, 3, 6] },
                ConversationVerdict { dim: "pressure_arc".into(), aggregate: None, judge_medians: vec![] },
            ],
            any_scored: true,
        };
        // 越高越好维取 min（最严裁判）——与 report_dim 走 max 相反。
        assert_eq!(report_dim_min(&report, "overall_progress"), Some(3));
        // 空 judge_medians → None。
        assert_eq!(report_dim_min(&report, "pressure_arc"), None);
        // 不存在的维 → None。
        assert_eq!(report_dim_min(&report, "nonexistent"), None);
    }
}
