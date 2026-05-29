//! Phase C / C1：reviewer 度量层聚合。
//!
//! 与 [`super::lessons_learned`] 互补：
//! - **reviewer_stats**（本模块）：度量层 —— reviewer 通过率 / 误判率，回答
//!   "reviewer 这个评判者本身准不准"。
//! - lessons_learned：模式层 —— "在 X 条件下用 Y 措辞 → 用户 Z 反应"。
//!
//! 输入：近 N 天 `agent_decision_reviews`，已被 `record_user_reaction_inner`
//! 回填了 `outcome_status`（用户实际反应）与 `reviewer_misjudge_signal`
//! （reviewer 判断与用户反应不一致的信号，见
//! [`crate::agent::reaction::compute_reviewer_misjudge_signal`]）。
//!
//! 输出：每 workspace 一行滚动统计 `reviewer_stats` 文档，字段含
//! considered / approved / approved_but_user_negative / pass_rate /
//! misjudge_rate。该度量是 C2 `negative_example` 候选挑选的上游信号源——
//! misjudge_rate 越高，说明 reviewer 放过了越多实际不该发的内容，对应越多
//! negative_example 候选（候选入队本身已在 reaction.rs 即时完成，本模块只做
//! 度量汇总，不重复入队）。

use mongodb::bson::{doc, DateTime, Document};

use crate::routes::AppState;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ReviewerStatsReport {
    /// 窗口内已拿到用户反应的 review 数（`outcome_status` 已回填）。
    pub considered: i64,
    /// 其中 reviewer 当时判 approved 的数量。
    pub approved: i64,
    /// 其中"approved 但用户负反应"的误判数。
    pub approved_but_user_negative: i64,
}

impl ReviewerStatsReport {
    /// 通过率：approved / considered。considered 为 0 时返回 0.0。
    pub fn pass_rate(&self) -> f64 {
        ratio(self.approved, self.considered)
    }

    /// 误判率：approved_but_user_negative / approved。approved 为 0 时返回 0.0。
    /// 语义：在所有"reviewer 放行"的决策里，有多大比例最终挨了用户负反应。
    pub fn misjudge_rate(&self) -> f64 {
        ratio(self.approved_but_user_negative, self.approved)
    }
}

/// 单 workspace 单轮聚合。N 天窗口由 caller 透传，默认 14d。
/// 纯 mongo count + 一次 upsert，不发 LLM 调用，廉价；每轮都跑。
pub async fn aggregate_reviewer_stats_for_workspace(
    state: &AppState,
    workspace_id: &str,
    window_days: i64,
) -> anyhow::Result<ReviewerStatsReport> {
    let now_ms = DateTime::now().timestamp_millis();
    let since = DateTime::from_millis(now_ms - window_days.max(1) * 24 * 60 * 60 * 1000);
    let coll = state.db.decision_reviews();

    // considered：窗口内已回填用户反应（outcome_status 非空）的 review。
    let considered = coll
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "created_at": { "$gte": since },
                "outcome_status": { "$exists": true, "$ne": null },
            },
            None,
        )
        .await? as i64;

    let approved = coll
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "created_at": { "$gte": since },
                "outcome_status": { "$exists": true, "$ne": null },
                "approved": true,
            },
            None,
        )
        .await? as i64;

    let approved_but_user_negative = coll
        .count_documents(
            doc! {
                "workspace_id": workspace_id,
                "created_at": { "$gte": since },
                "reviewer_misjudge_signal": "approved_but_user_negative",
            },
            None,
        )
        .await? as i64;

    let report = ReviewerStatsReport {
        considered,
        approved,
        approved_but_user_negative,
    };

    // considered=0 时也写一行（pass_rate=0），保证 admin 面板有稳定锚点；幂等 upsert。
    let now = DateTime::now();
    let stat_id = format!("{workspace_id}::reviewer");
    let update = doc! {
        "$set": {
            "workspace_id": workspace_id,
            "window_days": window_days.max(1),
            "considered": report.considered,
            "approved": report.approved,
            "approved_but_user_negative": report.approved_but_user_negative,
            "pass_rate": report.pass_rate(),
            "misjudge_rate": report.misjudge_rate(),
            "updated_at": now,
        },
        "$setOnInsert": {
            "stat_id": &stat_id,
            "created_at": now,
        },
    };
    state
        .db
        .raw()
        .collection::<Document>("reviewer_stats")
        .update_one(
            doc! { "stat_id": &stat_id },
            update,
            mongodb::options::UpdateOptions::builder().upsert(true).build(),
        )
        .await?;

    Ok(report)
}

/// 比值，分母为 0 时返回 0.0（避免 NaN 落库）。
fn ratio(num: i64, den: i64) -> f64 {
    if den <= 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_rate_zero_when_no_reviews() {
        let r = ReviewerStatsReport::default();
        assert_eq!(r.pass_rate(), 0.0);
        assert_eq!(r.misjudge_rate(), 0.0);
    }

    #[test]
    fn pass_rate_and_misjudge_rate_compute() {
        let r = ReviewerStatsReport {
            considered: 100,
            approved: 80,
            approved_but_user_negative: 8,
        };
        assert!((r.pass_rate() - 0.8).abs() < 1e-9);
        // 误判率以 approved 为分母：8 / 80 = 0.1
        assert!((r.misjudge_rate() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn misjudge_rate_zero_when_none_approved() {
        let r = ReviewerStatsReport {
            considered: 10,
            approved: 0,
            approved_but_user_negative: 0,
        };
        assert_eq!(r.misjudge_rate(), 0.0);
    }

    #[test]
    fn stat_id_is_workspace_scoped() {
        let id = format!("{}::reviewer", "ws_a");
        assert!(id.starts_with("ws_a::"));
        assert!(id.ends_with("::reviewer"));
    }
}
