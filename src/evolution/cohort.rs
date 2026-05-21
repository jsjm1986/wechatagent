//! cohort 选择（M4 W1 Task 2.4）。
//!
//! 在 `evolution_eval_window_hours` 窗口内拉 `agent_run_logs`，按用途切两组：
//! - `threshold` cohort：success + failure 混合，用于 W3 shadow eval 跑 5 闸阈值
//!   命中变化（任何 lifecycle=completed 的 run 都收，但排除 envelope-level 失败）。
//! - `prompt` cohort：`final_review_status` 落在失败子集的 run，用于 W2 Critic
//!   LLM 输入和 W3 shadow eval。
//!
//! 同 contact 去重（每 contact 最多 `evolution_cohort_per_contact_cap` 条），按
//! `created_at` 倒序保留最近的；保证 cohort 不被高频客户淹没。
//!
//! cohort 数 < `evolution_min_replays` 时该路径返回空 vec（Requirements 2.5），
//! 调用方据此跳过 W2 候选生成。

use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, DateTime},
    options::FindOptions,
};

use crate::routes::AppState;

use super::error::EvolutionError;

#[derive(Debug, Default, Clone)]
pub struct Cohorts {
    pub threshold: Vec<ObjectId>,
    pub prompt: Vec<ObjectId>,
}

/// 失败 finalReviewStatus 子集（用于 prompt cohort）。与 outcomes_autonomy 的
/// "blocked-like" 集合保持一致；不要让"成功 + 局部决策"的 run 进入 prompt
/// cohort，因为这些 run 没有"reply 失败"信号让 Critic 学习。
pub const FAILURE_FINAL_REVIEW_STATUSES: &[&str] = &[
    "blocked_unverified_product_claim",
    "held_by_ai_policy",
    "blocked_by_safety_guard",
    "ai_waiting_for_more_context",
    "budget_exceeded",
    "revision_failed",
];

pub async fn select_cohorts(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> Result<Cohorts, EvolutionError> {
    let window_hours = state.config.evolution_eval_window_hours.max(1) as i64;
    let cap_per_contact = state.config.evolution_cohort_per_contact_cap.max(1);
    let min_replays = state.config.evolution_min_replays;

    let now_ms = DateTime::now().timestamp_millis();
    let since = DateTime::from_millis(now_ms.saturating_sub(window_hours * 3600 * 1000));

    let filter = doc! {
        "workspace_id": workspace_id,
        "account_id": account_id,
        "lifecycle": "completed",
        "created_at": { "$gte": since },
    };
    let mut cursor = state
        .db
        .agent_run_logs()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .build(),
        )
        .await
        .map_err(EvolutionError::from)?;

    let mut threshold_pool: Vec<(ObjectId, String, String)> = Vec::new();
    while let Some(run) = cursor.try_next().await.map_err(EvolutionError::from)? {
        let Some(id) = run.id else { continue };
        let contact = run.contact_wxid.clone().unwrap_or_default();
        threshold_pool.push((id, contact, run.final_review_status.clone()));
    }

    // 同 contact 去重，每个 contact 最多保留 cap 条（保留最近的——上层 cursor
    // 已按 created_at 倒序）。空 contact_wxid 视为"无 contact"分组下的一个
    // 自然 contact 组（实际不会大量出现）。
    let threshold = dedup_per_contact(&threshold_pool, cap_per_contact);
    let prompt_pool: Vec<(ObjectId, String, String)> = threshold_pool
        .iter()
        .filter(|(_, _, status)| FAILURE_FINAL_REVIEW_STATUSES.contains(&status.as_str()))
        .cloned()
        .collect();
    let prompt = dedup_per_contact(&prompt_pool, cap_per_contact);

    Ok(Cohorts {
        threshold: if threshold.len() >= min_replays {
            threshold
        } else {
            Vec::new()
        },
        prompt: if prompt.len() >= min_replays {
            prompt
        } else {
            Vec::new()
        },
    })
}

fn dedup_per_contact(
    pool: &[(ObjectId, String, String)],
    cap_per_contact: usize,
) -> Vec<ObjectId> {
    let mut counter: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::with_capacity(pool.len());
    for (id, contact, _) in pool {
        let key = contact.as_str();
        let entry = counter.entry(key).or_insert(0);
        if *entry < cap_per_contact {
            out.push(*id);
            *entry += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(idx: u32, contact: &str, status: &str) -> (ObjectId, String, String) {
        // ObjectId::new() 单调递增足以构造稳定测试 id；用 idx 仅作可读注释。
        let _ = idx;
        (ObjectId::new(), contact.to_string(), status.to_string())
    }

    #[test]
    fn dedup_caps_runs_per_contact() {
        let pool = vec![
            mk(1, "user_a", "approved"),
            mk(2, "user_a", "approved"),
            mk(3, "user_a", "approved"),
            mk(4, "user_a", "approved"),
            mk(5, "user_b", "approved"),
        ];
        let out = dedup_per_contact(&pool, 3);
        // user_a 命中 cap=3，最后一条被丢；user_b 1 条都进。
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn dedup_treats_empty_contact_as_one_group() {
        let pool = vec![
            mk(1, "", "approved"),
            mk(2, "", "approved"),
            mk(3, "", "approved"),
            mk(4, "", "approved"),
        ];
        let out = dedup_per_contact(&pool, 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn failure_status_set_includes_5gate_and_budget() {
        // 维护性断言：FAILURE_FINAL_REVIEW_STATUSES 一旦被改，这里要同步更新。
        // 列出当前覆盖的 6 个分类；未来加新失败类型也来这里登记。
        for s in [
            "blocked_unverified_product_claim",
            "held_by_ai_policy",
            "blocked_by_safety_guard",
            "ai_waiting_for_more_context",
            "budget_exceeded",
            "revision_failed",
        ] {
            assert!(FAILURE_FINAL_REVIEW_STATUSES.contains(&s), "missing {s}");
        }
    }
}
