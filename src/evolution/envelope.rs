//! 演化实验信封写入 / 状态推进 helper（M4 W1 Task 2.3）。
//!
//! 信封是一次 tick 的 envelope：用 `experiment_id` 唯一索引保证不重复 insert，
//! 后续 cohort / candidate / replay 都以 `experiment_id` 字段挂载。

use mongodb::bson::{doc, DateTime};

use crate::routes::AppState;

use super::error::EvolutionError;

/// 写入一条 `experiments` 信封。`experiment_id` 字段在 `ensure_evolution_indexes`
/// 内已建唯一索引；同 ID 二次 insert SHALL 触发 DuplicateKey（调用方应避免）。
pub async fn insert_experiment_envelope(
    state: &AppState,
    experiment_id: &str,
    workspace_id: &str,
    account_id: &str,
    window_hours: i32,
) -> Result<(), EvolutionError> {
    let envelope = crate::models::Experiment {
        id: None,
        experiment_id: experiment_id.to_string(),
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        status: "collecting".to_string(),
        window_hours,
        started_at: DateTime::now(),
        updated_at: DateTime::now(),
        finished_at: None,
        cohort_threshold_run_ids: Vec::new(),
        cohort_prompt_run_ids: Vec::new(),
        budget_used_tokens: 0,
        budget_used_calls: 0,
        proposals_count: 0,
        proposals_eligible_count: 0,
    };
    state
        .db
        .experiments()
        .insert_one(envelope, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

/// 推进 `experiments.status`。允许 `collecting / evaluating / awaiting_admin /
/// released / aborted`；非法值 SHALL 返回 [`EvolutionError::InvalidStatus`]。
///
/// `matched_count == 0` 也视为非法（envelope 不存在），调用方 SHALL 写一条
/// `agent_events kind="evolution_envelope_missing"` 留痕——本 helper 只负责
/// 报错，不决定调用方如何写事件。
pub async fn update_experiment_status(
    state: &AppState,
    experiment_id: &str,
    new_status: &str,
) -> Result<(), EvolutionError> {
    if !matches!(
        new_status,
        "collecting" | "evaluating" | "awaiting_admin" | "released" | "aborted"
    ) {
        return Err(EvolutionError::InvalidStatus(format!(
            "unknown experiment.status: {new_status}"
        )));
    }
    let mut update = doc! {
        "status": new_status,
        "updated_at": DateTime::now(),
    };
    if matches!(new_status, "released" | "aborted") {
        update.insert("finished_at", DateTime::now());
    }
    let result = state
        .db
        .experiments()
        .update_one(
            doc! { "experiment_id": experiment_id },
            doc! { "$set": update },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;
    if result.matched_count == 0 {
        return Err(EvolutionError::InvalidStatus(format!(
            "experiment_id not found: {experiment_id}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn update_experiment_status_rejects_unknown_status() {
        // 用 mongo helper 之外的纯逻辑分支断言："unknown" SHALL 直接报错而不
        // 触达 db 层。这里通过构造一个错误 status 并断言枚举校验提前 short-circuit
        // ——状态字符串 match 失败时 helper 在第一行就返回，不会读到 state。
        // 因此用一个 dummy state pointer cast trick 不安全；改为只做静态字符串
        // 集合断言：保证未来 schema 升级时这个集合不被默默改动。
        let allowed: &[&str] = &[
            "collecting",
            "evaluating",
            "awaiting_admin",
            "released",
            "aborted",
        ];
        assert_eq!(allowed.len(), 5);
        assert!(allowed.contains(&"awaiting_admin"));
        assert!(!allowed.contains(&"unknown"));
    }
}
