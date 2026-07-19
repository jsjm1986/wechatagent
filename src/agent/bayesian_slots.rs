//! 贝叶斯评估旁路的占槽/淘汰纯函数（子计划4 Task1）。
//!
//! 这是一条纯观察侧路：AI 在压缩周期自由发现至多 6 个客户维度（如「价格敏感度」），
//! 每个维度跟踪一条置信走势线。**永不驱动行为**——这里的信号永不进入任何规划器筛选、
//! 状态机、发送/引荐选择或触达门，仅供管理员展示与未来评估。本文件只含纯槽位管理函数，
//! 不从规划/网关/决策路径导入任何东西。
//!
//! 占槽必须严谨：单次提及（hit=1）永远不能占用一个槽位——需要跨多轮命中累积
//! （min_hits）且强证据累积（min_strong_evidence）双达标。

use crate::models::{BayesianPoint, BayesianSignal};

/// 槽位上限：最多同时正式占用 6 个观察维度。
pub const MAX_BAYESIAN_SLOTS: usize = 6;
/// 单维度走势线历史封顶。
pub const HISTORY_CAP: usize = 100;
/// 走势线上「强证据点」的标记值（存入 `BayesianPoint.reason`）。
/// 强证据由代码侧据消息方向客观判定（锚定客户 Inbound 消息），不信 LLM 自报置信。
const STRONG_POINT_MARKER: &str = "strong";

/// 占槽阈值（可配）。默认要求跨 3 轮命中 + 2 次强证据，杜绝一两句话占槽。
#[derive(Debug, Clone)]
pub struct SlotPromotionThreshold {
    pub min_hits: i32,
    pub min_strong_evidence: i32,
}

impl Default for SlotPromotionThreshold {
    fn default() -> Self {
        Self {
            min_hits: 3,
            min_strong_evidence: 2,
        }
    }
}

/// 本轮观察到的一个维度（由 LLM 输出 + 代码侧证据强度统计得来）。
#[derive(Debug, Clone)]
pub struct ObservedDimension {
    pub dimension: String,
    pub value: String,
    pub confidence: f64,
    pub strong_evidence_count: i32,
}

/// 占槽门：跨多轮命中 + 强证据累积双达标才占。一两句话（hit=1）永远不够。
pub fn should_promote(
    hit_count: i32,
    strong_evidence_count: i32,
    th: &SlotPromotionThreshold,
) -> bool {
    hit_count >= th.min_hits && strong_evidence_count >= th.min_strong_evidence
}

/// 增量更新贝叶斯信号。已占槽→更新值/置信/history（封顶 HISTORY_CAP）；
/// 未占槽→累积一条观察线，达阈值且未满 6 槽则 lock 占槽。永不驱动行为，纯观测。
///
/// 借用处理：第一遍 `iter_mut` 只做「找到或新建信号 + 推 history + 截断」；
/// 占槽判定单独放第二遍，先快照已占槽数算出 budget，再顺序遍历未占槽信号 lock，
/// 避免 `iter_mut` 与 `signals.iter().filter` 同时借用冲突。
///
/// hits/strong 口径（apply 路径）：
/// - `hits = signal.history.len()`——已观察的轮数（跨轮累积，这才是「多轮命中」的本意）；
/// - `strong = history 中被标记为强证据点（reason == STRONG_POINT_MARKER）的点数`——
///   强证据由代码侧据消息方向客观判定（`ObservedDimension.strong_evidence_count`，
///   锚定客户 Inbound 消息才算），**不信 LLM 自报 confidence**。本轮观察若
///   `strong_evidence_count >= 1` 则把当轮 history 点标记为强证据点，跨轮累积可复算。
pub fn apply_bayesian_update(
    signals: &mut Vec<BayesianSignal>,
    observed: &[ObservedDimension],
    turn: i32,
    th: &SlotPromotionThreshold,
) {
    // 第一遍：更新已有信号或新建观察线，并截断 history。
    for obs in observed {
        // 强证据由代码侧据消息方向客观判定：本轮锚定客户 Inbound 消息的证据 >=1 即标记强证据点。
        let reason = if obs.strong_evidence_count >= 1 {
            Some(STRONG_POINT_MARKER.to_string())
        } else {
            None
        };
        if let Some(sig) = signals.iter_mut().find(|s| s.dimension == obs.dimension) {
            let value_changed = sig.current_value != obs.value;
            let confidence_changed = (sig.current_confidence - obs.confidence).abs() > f64::EPSILON;
            sig.current_value = obs.value.clone();
            sig.current_confidence = obs.confidence;
            sig.history.push(BayesianPoint {
                turn,
                value: obs.value.clone(),
                confidence: obs.confidence,
                value_changed,
                confidence_changed,
                reason,
            });
            while sig.history.len() > HISTORY_CAP {
                sig.history.remove(0);
            }
        } else {
            signals.push(BayesianSignal {
                dimension: obs.dimension.clone(),
                current_value: obs.value.clone(),
                current_confidence: obs.confidence,
                locked: false,
                history: vec![BayesianPoint {
                    turn,
                    value: obs.value.clone(),
                    confidence: obs.confidence,
                    value_changed: false,
                    confidence_changed: false,
                    reason,
                }],
            });
        }
    }

    // 第二遍：占槽判定（与上面的 iter_mut 借用解耦）。先快照已占槽数算 budget。
    let locked_count = signals.iter().filter(|s| s.locked).count();
    let mut budget = MAX_BAYESIAN_SLOTS.saturating_sub(locked_count);
    for sig in signals.iter_mut() {
        if budget == 0 {
            break;
        }
        if !sig.locked {
            let hits = sig.history.len() as i32;
            // strong 取自代码侧强证据标记点数（跨轮累积），不再从 confidence 反推。
            let strong = sig
                .history
                .iter()
                .filter(|p| p.reason.as_deref() == Some(STRONG_POINT_MARKER))
                .count() as i32;
            if should_promote(hits, strong, th) {
                sig.locked = true;
                budget -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_promote_below_threshold() {
        let th = SlotPromotionThreshold {
            min_hits: 3,
            min_strong_evidence: 2,
        };
        assert!(!should_promote(2, 2, &th)); // hits 不够
        assert!(!should_promote(3, 1, &th)); // 强证据不够
        assert!(should_promote(3, 2, &th)); // 双达标
    }

    #[test]
    fn single_mention_never_promotes() {
        // 用户红线：一两句话不能占槽。
        let th = SlotPromotionThreshold {
            min_hits: 3,
            min_strong_evidence: 2,
        };
        assert!(!should_promote(1, 1, &th));
    }

    #[test]
    fn history_capped_at_100() {
        let th = SlotPromotionThreshold {
            min_hits: 1,
            min_strong_evidence: 0,
        };
        let mut signals = vec![];
        for turn in 0..150 {
            apply_bayesian_update(
                &mut signals,
                &[ObservedDimension {
                    dimension: "价格敏感度".into(),
                    value: "高".into(),
                    confidence: 0.6,
                    strong_evidence_count: 1,
                }],
                turn,
                &th,
            );
        }
        let sig = signals
            .iter()
            .find(|s| s.dimension == "价格敏感度")
            .unwrap();
        assert!(sig.locked);
        assert!(sig.history.len() <= 100);
    }

    #[test]
    fn never_exceeds_six_locked_slots() {
        let th = SlotPromotionThreshold {
            min_hits: 1,
            min_strong_evidence: 0,
        };
        let mut signals = vec![];
        for d in 0..10 {
            apply_bayesian_update(
                &mut signals,
                &[ObservedDimension {
                    dimension: format!("dim{d}"),
                    value: "v".into(),
                    confidence: 0.5,
                    strong_evidence_count: 1,
                }],
                0,
                &th,
            );
        }
        assert!(signals.iter().filter(|s| s.locked).count() <= MAX_BAYESIAN_SLOTS);
    }

    #[test]
    fn promotion_uses_code_side_strong_not_confidence() {
        // Option B：占槽强证据口径取自代码侧（Inbound 锚定数），不信 LLM 自报 confidence。
        let th = SlotPromotionThreshold {
            min_hits: 3,
            min_strong_evidence: 2,
        };

        // 高置信(0.9)但代码侧强证据=0：跨 5 轮命中达 min_hits，仍不应占槽（strong=0 < 2）。
        let mut signals = vec![];
        for turn in 0..5 {
            apply_bayesian_update(
                &mut signals,
                &[ObservedDimension {
                    dimension: "价格敏感度".into(),
                    value: "高".into(),
                    confidence: 0.9,
                    strong_evidence_count: 0,
                }],
                turn,
                &th,
            );
        }
        let sig = signals
            .iter()
            .find(|s| s.dimension == "价格敏感度")
            .unwrap();
        assert!(
            !sig.locked,
            "高置信但代码侧强证据=0 不得占槽（confidence 不再驱动占槽）"
        );

        // 低置信(0.3)但代码侧强证据=1：跨 3 轮累积 hits=3 且 strong=3 >= 2 → 占槽。
        let mut signals2 = vec![];
        for turn in 0..3 {
            apply_bayesian_update(
                &mut signals2,
                &[ObservedDimension {
                    dimension: "决策角色".into(),
                    value: "拍板人".into(),
                    confidence: 0.3,
                    strong_evidence_count: 1,
                }],
                turn,
                &th,
            );
        }
        let sig2 = signals2.iter().find(|s| s.dimension == "决策角色").unwrap();
        assert!(sig2.locked, "代码侧强证据累积达标即占槽，与 LLM 低置信无关");
    }
}
