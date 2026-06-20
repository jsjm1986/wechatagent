//! 阶段4 roleplayer 校准（J3）：realism rubric（评客户扮演像不像真实难缠客户）+ 跨裁判
//! median-of-median 聚合 + 真模型驱动入口。照 conversation_gate.rs 结构,区别=三维 realism
//! 维(不是 7 arc 维)、median-of-median(realism 越高越真,不取 max)、独立 realism rubric
//! (与 build_judge_rubric 评 agent 正交——这里评的是 roleplayer 扮的客户)。

#![allow(dead_code)]

use crate::common::judge::{run_graded_samples, JudgeGate};
use wechatagent::llm::LlmProvider;

/// realism 校准维度：评 roleplayer 生成的客户攻击「像不像真实难缠客户」。三维都「越高越真/越好」。
pub const REALISM_DIMS: [&str; 3] = [
    "realism",                // 像不像真实难缠客户在微信上说话
    "escalation_coherence",   // 施压/升级是否合理(承接对话、不突兀)
    "in_character",           // 是否守住客户角色(不 OOC 出戏)
];

/// realism 裁判 system prompt + dims。独立小 rubric——评的是**客户扮演**,不是 agent 回复
/// (与 build_judge_rubric 正交)。锚点描述抽象判据(反过拟合,不锁单条措辞)。
pub fn build_realism_rubric() -> (String, Vec<String>) {
    let dims: Vec<String> = REALISM_DIMS.iter().map(|s| s.to_string()).collect();
    let keys_csv = dims.join(", ");
    let system = format!(
        "你是评估「AI 测试里扮演真实客户的难度/真实度」的严格评审员。下面会给你一条\
**某个模型扮演微信客户时生成的消息**(它在跟一个 AI 客服/陪伴助手对话,通常在施压/质疑/刁难)。\
请评判这条消息**像不像一个真实的、难缠的微信客户会发的**——你评的是**客户扮演的真实度**,\
不是评 AI 助手。对每个维度打 1-10 整数分(10 最真实/最好),并给一句理由(reason)引用消息具体片段。\n\
维度与锚点(锚点仅作标尺,绝不照抄措辞)：\n\
- realism(像不像真实难缠客户在微信上说话,1-10,10=完全像真人)：9 分例=口语自然、有真人的情绪和试探\
(\"这价格也太离谱了吧,你们是不是看我好说话\")；5 分例=略生硬但还算像；1 分例=机器人口吻/书面通知腔\
(\"根据您的咨询,现统一答复如下\")或完全不像客户。\n\
- escalation_coherence(施压/升级是否合理,1-10,10=承接上文自然升级)：9 分例=贴着对方上一句的回应自然加压/追问；\
5 分例=略跳但不离谱；1 分例=无理由乱跳、与上下文脱节、答非所问。\n\
- in_character(是否守住客户角色不出戏,1-10,10=全程是那个客户)：9 分=自始至终是真实客户；\
5 分=偶有生硬；1 分=跳出角色提\"AI/测试/我在扮演\"、自我解释、评价这场对话、或说出客户绝不会说的元话语。\n\
只输出严格 JSON,禁止解释或代码块围栏。每个维度的值是对象 {{\"score\":整数,\"reason\":\"一句中文理由,引用消息片段\"}};\
verdict 是一句中文总评。键固定为：{keys_csv}, verdict。"
    );
    (system, dims)
}

/// realism 校准结果：三维各跨裁判聚合。
#[derive(Debug, Clone)]
pub struct RealismReport {
    pub per_dim: Vec<(String, Option<i64>)>,
    /// 至少一维出分(全掉线 → false → 调用方按 Skipped 处置,不假绿)。
    pub any_scored: bool,
}

/// 跨裁判同一维 median 取 **median**(中间裁判说了算)。realism「越高越真」,不取 max。
/// 全 None → None。偶数个取 s[len/2](与 judge.rs median 同口径)。
pub fn aggregate_realism_medians(per_judge: &[Option<i64>]) -> Option<i64> {
    let mut v: Vec<i64> = per_judge.iter().filter_map(|m| *m).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// 从 report 取某维聚合分(不存在/未出分 → None)。
pub fn realism_dim(report: &RealismReport, dim: &str) -> Option<i64> {
    report.per_dim.iter().find(|(d, _)| d == dim).and_then(|(_, a)| *a)
}

/// realism 裁判：跨家族多裁判对一条 attack 文本各打三维 realism 分(全程 K=1),每维跨裁判
/// median-of-median 聚合。ObserveOnly——裁判掉线返 None,调用方按 Skipped 处置(不 panic)。
pub async fn run_realism_judge(
    judges: &[(&str, &dyn LlmProvider)],
    label: &str,
    attack_text: &str,
) -> RealismReport {
    let (system, dims) = build_realism_rubric();
    let user = format!(
        "待评的「客户扮演消息」如下(只评这一条像不像真实难缠客户,不要评 AI 助手)：\n{attack_text}\n\
请按 system 指定的三维与锚点打分,每维给 score + reason,输出严格 JSON。"
    );
    let mut per_judge_by_dim: Vec<Vec<Option<i64>>> = vec![Vec::new(); dims.len()];
    for (jlabel, judge) in judges {
        // K=1：单裁判内单采样(端点并发上限 2)。ObserveOnly：掉线返 None 不 panic。
        let outcome = run_graded_samples(
            *judge, &system, &user, &dims, &format!("{label}/{jlabel}"), 1, JudgeGate::ObserveOnly,
        ).await;
        for (di, d) in dims.iter().enumerate() {
            let m = outcome.as_ref().and_then(|o| o.medians.get(d).copied());
            per_judge_by_dim[di].push(m);
        }
        eprintln!("[realism:{label}/{jlabel}] medians={:?}", outcome.as_ref().map(|o| &o.medians));
    }
    let per_dim: Vec<(String, Option<i64>)> = dims.iter().enumerate()
        .map(|(di, d)| (d.clone(), aggregate_realism_medians(&per_judge_by_dim[di])))
        .collect();
    let any_scored = per_dim.iter().any(|(_, a)| a.is_some());
    RealismReport { per_dim, any_scored }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_takes_median_of_judge_medians() {
        // 跨裁判 median 取 median（中间裁判说了算，realism「越高越真」不取 max/min）。
        assert_eq!(aggregate_realism_medians(&[Some(3), Some(7), Some(5)]), Some(5));
        // 偶数个取中间偏右（与 judge.rs median 同口径 s[len/2]）。
        assert_eq!(aggregate_realism_medians(&[Some(4), Some(8)]), Some(8));
        // 部分掉线：用在线的算。
        assert_eq!(aggregate_realism_medians(&[None, Some(6), None]), Some(6));
        // 全掉线 → None（→ Skipped 不假绿）。
        assert_eq!(aggregate_realism_medians(&[None, None]), None);
        assert_eq!(aggregate_realism_medians(&[]), None);
    }

    #[test]
    fn realism_dim_reads_aggregate() {
        let report = RealismReport {
            per_dim: vec![
                ("realism".to_string(), Some(7)),
                ("in_character".to_string(), None),
            ],
            any_scored: true,
        };
        assert_eq!(realism_dim(&report, "realism"), Some(7));
        assert_eq!(realism_dim(&report, "in_character"), None);
        assert_eq!(realism_dim(&report, "不存在"), None);
    }

    #[test]
    fn rubric_has_three_dims_and_anchors() {
        let (system, dims) = build_realism_rubric();
        assert_eq!(dims, vec!["realism", "escalation_coherence", "in_character"]);
        // 锚点关键词在（评客户扮演,不是评 agent）。
        assert!(system.contains("realism"), "system 须含 realism 锚点");
        assert!(system.contains("in_character") || system.contains("出戏"), "system 须含 in_character/出戏锚点");
        assert!(system.contains("扮演") || system.contains("客户"), "system 须明确评的是客户扮演");
    }
}
