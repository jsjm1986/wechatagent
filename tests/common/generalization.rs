//! 泛化门（train/holdout 召回差距）的可复用纯函数 + 单测。
//!
//! 抽自 real_llm_knowledge_quality.rs 的 Q2 收尾断言：对 train / holdout 两个 split
//! 分别求平均召回，断言两者都 ≥ floor 且差距（gap）≤ max_gap。prompt 若被特调适配
//! train 文档，train 召回虚高 / holdout 塌 → gap 爆 = 过拟合信号。
//! 纯函数无 IO，可在任意 test crate 复用，单测无需 Docker。

#![allow(dead_code)]

/// 泛化评估结果。`ok()` 为 false 表示触发了过拟合 / 召回不足红线。
#[derive(Debug, Clone, PartialEq)]
pub struct GeneralizationReport {
    pub train_mean: f64,
    pub holdout_mean: f64,
    pub gap: f64,
    pub train_n: usize,
    pub holdout_n: usize,
    /// 任一 split 为空。空 split 视为不合格（无法评估泛化）。
    pub empty_split: bool,
    /// train_mean < floor。
    pub train_below_floor: bool,
    /// holdout_mean < floor。
    pub holdout_below_floor: bool,
    /// gap > max_gap。
    pub gap_exceeded: bool,
}

impl GeneralizationReport {
    /// 全部红线均未触发才算过。
    pub fn ok(&self) -> bool {
        !self.empty_split
            && !self.train_below_floor
            && !self.holdout_below_floor
            && !self.gap_exceeded
    }
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

/// 计算泛化报告。`floor`=每 split 平均召回下限，`max_gap`=允许的 |train-holdout| 上限。
pub fn generalization_report(
    train: &[f64],
    holdout: &[f64],
    floor: f64,
    max_gap: f64,
) -> GeneralizationReport {
    let train_mean = mean(train);
    let holdout_mean = mean(holdout);
    let gap = (train_mean - holdout_mean).abs();
    let empty_split = train.is_empty() || holdout.is_empty();
    GeneralizationReport {
        train_mean,
        holdout_mean,
        gap,
        train_n: train.len(),
        holdout_n: holdout.len(),
        empty_split,
        // floor 标志带 !is_empty() 守卫：只在"有数据且偏低"时为真；空 split 由 empty_split 独立兜底，语义不串。
        train_below_floor: !train.is_empty() && train_mean < floor,
        holdout_below_floor: !holdout.is_empty() && holdout_mean < floor,
        gap_exceeded: !empty_split && gap > max_gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_when_both_high_and_gap_small() {
        let r = generalization_report(&[0.9, 0.85], &[0.88, 0.82], 0.7, 0.18);
        assert!(r.ok(), "高召回小差距应过：{r:?}");
    }

    #[test]
    fn fails_on_large_generalization_gap() {
        // train 虚高 holdout 塌 → 过拟合。
        let r = generalization_report(&[0.95, 0.95], &[0.50, 0.55], 0.4, 0.18);
        assert!(r.gap_exceeded, "gap 0.4>0.18 应触发：{r:?}");
        assert!(!r.ok());
    }

    #[test]
    fn fails_when_holdout_below_floor() {
        let r = generalization_report(&[0.8], &[0.5], 0.7, 0.5);
        assert!(r.holdout_below_floor, "holdout 0.5<0.7 应触发");
        assert!(!r.ok());
    }

    #[test]
    fn fails_when_train_below_floor() {
        // holdout 高于 floor，确保只有 train_below_floor 单独触发（隔离该 flag）。
        let r = generalization_report(&[0.6], &[0.75], 0.7, 0.5);
        assert!(r.train_below_floor, "train 0.6<0.7 应触发");
        assert!(!r.holdout_below_floor, "holdout 0.75≥0.7 不应触发");
        assert!(!r.ok());
    }

    #[test]
    fn empty_split_is_not_ok() {
        let r = generalization_report(&[], &[0.9], 0.7, 0.18);
        assert!(r.empty_split);
        assert!(!r.ok(), "空 train split 不能算过");
    }

    #[test]
    fn gap_uses_absolute_value() {
        // holdout 高于 train 也算 gap（虽罕见），用绝对值。
        let r = generalization_report(&[0.5], &[0.9], 0.4, 0.18);
        assert!((r.gap - 0.4).abs() < 1e-9, "gap 应为 |0.5-0.9|=0.4");
    }
}
