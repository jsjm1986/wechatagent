//! agent-self-evolution M4：演化器模块。
//!
//! **隔离红线**：本模块严禁引用 `crate::agent::gateway / outbox`、`crate::mcp::*`、
//! `agent_send_outbox` 写入路径，或 `run_user_operation_gateway / handle_managed_message
//! / handle_follow_up_task` 等生产链路入口。`scripts/check-evolution-isolation.sh`
//! 在 CI 内静态扫描该目录强制此约束（M4 W0 Task 1.4）。
//!
//! 主循环 [`run_evolutionary_worker`] 由 `main.rs` 无条件 spawn；`EVOLUTION_ENABLED`
//! 是硬上限——为 false（运维硬锁定）时 worker 进函数即 return，不进 tick；为 true
//! 时进常驻 tick 循环，每 tick 内由 mongo runtime flag 决定是否真选 cohort。波次落地节奏：
//! - W1（本波）：worker 主循环 + EvolutionBudget + experiment 信封 + cohort 选择
//! - W2：threshold 候选 + Critic LLM prompt 候选
//! - W3：Shadow eval + 显著性
//! - W4：Release + 前端 + 回滚 + post-release review
//!
//! FORBIDDEN dependencies: gateway / outbox / mcp / tasks / webhooks。

pub mod budget;
pub mod cohort;
pub mod envelope;
pub mod error;
pub mod lint;
pub mod post_release;
pub mod prompt_critic;
pub mod release;
pub mod replay;
pub mod revision;
pub mod significance;
pub mod threshold;

use std::time::Duration;

use mongodb::bson::{doc, oid::ObjectId, DateTime};
use tokio::time::interval;

use crate::routes::AppState;

pub use self::budget::EvolutionBudget;
pub use self::cohort::{select_cohorts, select_cohorts_filtered, Cohorts};
pub use self::envelope::{insert_experiment_envelope, update_experiment_status};
pub use self::error::EvolutionError;

pub mod runtime_flag;
pub use self::runtime_flag::{bucket_for_contact, load_runtime_flag, rollout_bucket_index};

/// 演化器主循环。`EVOLUTION_ENABLED=false`（运维硬锁定）时立即 return；为 true 时
/// 进常驻 tick 循环，实际是否产出由 mongo runtime flag（UI 总开关）每 tick 决定。
///
/// 每 `evolution_tick_seconds` 秒动态枚举所有注册账号并逐 scope 触发
/// [`run_one_tick`]。单个 scope 失败不影响其它租户或下一轮。
pub async fn run_evolutionary_worker(state: AppState) {
    if !state.config.evolution_enabled {
        tracing::info!(
            "evolution worker hard-locked (EVOLUTION_ENABLED=false); not entering tick loop"
        );
        return;
    }
    let tick_seconds = state.config.evolution_tick_seconds.max(60);
    tracing::info!(
        tick_seconds,
        "evolution worker starting (full pipeline: cohort → critic → replay → significance)"
    );
    let mut ticker = interval(Duration::from_secs(tick_seconds));
    loop {
        ticker.tick().await;
        match crate::account_scheduler::list_registered_account_scopes(&state).await {
            Ok(scopes) => {
                for scope in scopes {
                    if let Err(err) =
                        run_one_tick(&state, &scope.workspace_id, &scope.account_id).await
                    {
                        tracing::warn!(
                            ?err,
                            workspace_id = %scope.workspace_id,
                            account_id = %scope.account_id,
                            "evolution scope tick failed; continuing"
                        );
                        let _ = write_tick_failed_event(
                            &state,
                            &scope.workspace_id,
                            &scope.account_id,
                            &err.to_string(),
                        )
                        .await;
                    }
                }
            }
            Err(err) => {
                tracing::warn!(?err, "evolution account scope listing failed; continuing");
            }
        }
    }
}

/// 单次 tick 主流程：
/// 1. 写 `experiments` 信封；
/// 2. 选 cohort（threshold + prompt）；
/// 3. M4 W2：threshold 候选（纯统计，不消 EvolutionBudget）；
/// 4. M4 W2：prompt critic 候选（消 EvolutionBudget；耗尽时 silent skip）；
/// 5. 把 status 推到 `evaluating`（W3 引入 shadow eval 后由 eval 路径
///    切换到 `awaiting_admin`）；
/// 6. 写 `evolution_tick_completed` 事件。
pub async fn run_one_tick(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> Result<(), EvolutionError> {
    // `experiment_id` 有全局唯一索引；ObjectId 避免不同 workspace 复用同名
    // account 时在同一毫秒发生碰撞。后续查询仍显式校验完整 scope。
    let exp_id = format!("exp_{}", ObjectId::new().to_hex());

    // 1. 信封
    insert_experiment_envelope(
        state,
        &exp_id,
        workspace_id,
        account_id,
        state.config.evolution_eval_window_hours as i32,
    )
    .await?;

    // Phase C / C3：mongo runtime flag 决定灰度桶。`enabled=false` 或文档不存在
    // → 全员排除（worker 仍跑空 tick，保留可观察性 + 写 envelope）；`enabled=true`
    // 时按 `hash(contact_id) % 100 < rollout_percent` 分桶。
    //
    // 读失败按 None 处理，避免 mongo 抖动让灰度门误开。
    let runtime_flag = match self::runtime_flag::load_runtime_flag(state, workspace_id).await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                ?err,
                "evolution runtime_flag load failed; treating as disabled this tick"
            );
            None
        }
    };

    // 2. cohort（灰度过滤）
    let cohorts =
        select_cohorts_filtered(state, workspace_id, account_id, runtime_flag.as_ref()).await?;
    let threshold_count = cohorts.threshold.len();
    let prompt_count = cohorts.prompt.len();

    // 推进 cohort 字段
    state
        .db
        .experiments()
        .update_one(
            doc! {
                "experiment_id": &exp_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            doc! {
                "$set": {
                    "cohort_threshold_run_ids": cohorts.threshold.clone(),
                    "cohort_prompt_run_ids": cohorts.prompt.clone(),
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 3. threshold 候选（纯统计，不消 EvolutionBudget）。
    let threshold_proposals =
        threshold::generate(state, &exp_id, workspace_id, account_id, &cohorts.threshold).await?;
    insert_proposals(state, &threshold_proposals).await?;

    // 4. prompt critic 候选（消 EvolutionBudget；BudgetExceeded 不向上传播）。
    let mut budget = EvolutionBudget::from_config(&state.config);
    let prompt_proposals = match prompt_critic::generate(
        state,
        &exp_id,
        workspace_id,
        account_id,
        &cohorts,
        &mut budget,
    )
    .await
    {
        Ok(v) => v,
        Err(EvolutionError::BudgetExceeded {
            tokens_used,
            calls_used,
        }) => {
            write_budget_exceeded_event(
                state,
                workspace_id,
                account_id,
                &exp_id,
                tokens_used,
                calls_used,
            )
            .await?;
            Vec::new()
        }
        Err(e) => return Err(e),
    };
    insert_proposals(state, &prompt_proposals).await?;

    // 5. 写预算用量到 envelope。
    state
        .db
        .experiments()
        .update_one(
            doc! {
                "experiment_id": &exp_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            doc! {
                "$set": {
                    "budget_used_tokens": budget.token_used,
                    "budget_used_calls": budget.call_used as i32,
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 6. M4 W3：shadow replay + 显著性聚合。
    //    pending_eval 候选驱动。threshold 候选纯重判不调 LLM；prompt 候选经
    //    `replay::eval_all` → `prompt_shadow::shadow_replay_prompt_one` 跑真实
    //    Reply+Review 影子演练（调 LLM，但消耗不回写 EvolutionBudget——budget
    //    是 mut 借用无法跨 replay task 计量，eval_all 只做 exhausted() 静态
    //    预检，超额的 replay 直接落 failed 文档、不向上抛）。因此下方
    //    BudgetExceeded 分支是防御性兜底，当前 eval_all 不产生该错误。
    let pending_count = threshold_proposals
        .iter()
        .chain(prompt_proposals.iter())
        .filter(|p| p.status == "pending_eval")
        .count();
    let (eligible_count, rejected_after_eval) = if pending_count > 0 {
        match replay::eval_all(state, &exp_id, workspace_id, account_id, &mut budget).await {
            Ok(()) => {}
            Err(EvolutionError::BudgetExceeded {
                tokens_used,
                calls_used,
            }) => {
                write_budget_exceeded_event(
                    state,
                    workspace_id,
                    account_id,
                    &exp_id,
                    tokens_used,
                    calls_used,
                )
                .await?;
            }
            Err(e) => return Err(e),
        }
        significance::aggregate_and_grade(state, &exp_id, workspace_id, account_id).await?
    } else {
        (0, 0)
    };

    // 7. 推进状态：W3 后无论候选是否存在，都直接走 awaiting_admin
    //    （eligible_for_release 由 admin 二次确认，rejected 也已落字段）。
    update_experiment_status(state, &exp_id, workspace_id, account_id, "awaiting_admin").await?;

    // envelope 上同步写聚合计数，便于前端 EvolutionCenterTab 拉取。
    state
        .db
        .experiments()
        .update_one(
            doc! {
                "experiment_id": &exp_id,
                "workspace_id": workspace_id,
                "account_id": account_id,
            },
            doc! {
                "$set": {
                    "proposals_count": (threshold_proposals.len() + prompt_proposals.len()) as i32,
                    "proposals_eligible_count": eligible_count as i32,
                    "updated_at": DateTime::now(),
                }
            },
            None,
        )
        .await
        .map_err(EvolutionError::from)?;

    // 8. M4 W4 Task 5.6：扫一次到期的 post_release_reviews（+24h 对比窗口）。
    //    单条失败不影响 tick；已 release 的 proposal 仍受 admin 控制是否回滚。
    let post_release_completed = post_release::run_due_reviews(state, workspace_id, account_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(
                ?e,
                "post_release run_due_reviews failed; will retry next tick"
            );
            0
        });

    // （终裁 10-x 清理）：历史 threshold auto-release 接点已删除——HC-017 政策
    // 硬闸恒关使其成为永不可达的自动发布通道；release/rollback 唯一路径是
    // routes/evolution.rs 的管理员显式操作。

    write_tick_completed_event(
        state,
        workspace_id,
        account_id,
        &exp_id,
        threshold_count,
        prompt_count,
        threshold_proposals.len(),
        prompt_proposals.len(),
        budget.token_used,
        eligible_count,
        rejected_after_eval,
        post_release_completed,
    )
    .await?;
    Ok(())
}

async fn insert_proposals(
    state: &AppState,
    proposals: &[crate::models::Proposal],
) -> Result<(), EvolutionError> {
    if proposals.is_empty() {
        return Ok(());
    }
    state
        .db
        .proposals()
        .insert_many(proposals.to_vec(), None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

async fn write_budget_exceeded_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    exp_id: &str,
    tokens_used: i64,
    calls_used: i32,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "evolution_budget_exceeded".to_string(),
        status: "warning".to_string(),
        summary: format!(
            "evolution budget exceeded (tokens_used={tokens_used}, calls_used={calls_used})"
        ),
        details: Some(doc! {
            "experiment_id": exp_id,
            "tokens_used": tokens_used,
            "calls_used": calls_used as i32,
        }),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn write_tick_completed_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    exp_id: &str,
    threshold_count: usize,
    prompt_count: usize,
    threshold_proposals: usize,
    prompt_proposals: usize,
    budget_used_tokens: i64,
    proposals_eligible_count: usize,
    proposals_rejected_count: usize,
    post_release_reviews_completed: usize,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "evolution_tick_completed".to_string(),
        status: "ok".to_string(),
        summary: format!(
            "evolution tick completed (threshold_cohort={threshold_count}, prompt_cohort={prompt_count}, threshold_proposals={threshold_proposals}, prompt_proposals={prompt_proposals}, eligible={proposals_eligible_count}, rejected={proposals_rejected_count}, post_release_reviews_completed={post_release_reviews_completed})"
        ),
        details: Some(doc! {
            "experiment_id": exp_id,
            "threshold_cohort_size": threshold_count as i32,
            "prompt_cohort_size": prompt_count as i32,
            "threshold_proposals_count": threshold_proposals as i32,
            "prompt_proposals_count": prompt_proposals as i32,
            "budget_used_tokens": budget_used_tokens,
            "proposals_eligible_count": proposals_eligible_count as i32,
            "proposals_rejected_count": proposals_rejected_count as i32,
            "post_release_reviews_completed": post_release_reviews_completed as i32,
        }),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

async fn write_tick_failed_event(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    error_summary: &str,
) -> Result<(), EvolutionError> {
    let event = crate::models::AgentEvent {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        contact_wxid: None,
        kind: "evolution_tick_failed".to_string(),
        status: "error".to_string(),
        summary: format!("evolution tick failed: {}", truncate(error_summary, 1024)),
        details: Some(doc! { "error": truncate(error_summary, 1024) }),
        created_at: DateTime::now(),
        dedupe_key: None,
    };
    state
        .db
        .events()
        .insert_one(event, None)
        .await
        .map_err(EvolutionError::from)?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod isolation_contract_tests {
    use std::collections::{BTreeSet, HashSet};
    use std::fs;
    use std::path::PathBuf;

    const EXPECTED_MODULES: &[&str] = &[
        "budget.rs",
        "cohort.rs",
        "envelope.rs",
        "error.rs",
        "lint.rs",
        "mod.rs",
        "post_release.rs",
        "prompt_critic.rs",
        "release.rs",
        "replay.rs",
        "revision.rs",
        "runtime_flag.rs",
        "significance.rs",
        "threshold.rs",
    ];

    fn source_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/evolution")
    }

    fn production_lines(source: &str) -> String {
        let source = source.split("\n#[cfg(test)]").next().unwrap_or(source);
        source
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                !line.starts_with("//") && !line.starts_with("#")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_evolution_module_is_explicitly_reviewed() {
        let actual: BTreeSet<String> = fs::read_dir(source_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|value| value.to_str()) == Some("rs"))
                    .then(|| path.file_name().unwrap().to_string_lossy().into_owned())
            })
            .collect();
        let expected: BTreeSet<String> =
            EXPECTED_MODULES.iter().map(|v| (*v).to_string()).collect();
        assert_eq!(
            actual, expected,
            "new evolution modules require an isolation review"
        );
    }

    #[test]
    fn production_dependencies_exclude_side_effect_entrypoints() {
        let forbidden = [
            "crate::agent::gateway",
            "crate::agent::outbox",
            "crate::mcp",
            "crate::tasks",
            "crate::webhooks",
            "run_user_operation_gateway",
            "handle_managed_message",
            "handle_follow_up_task",
            "agent_send_outbox",
        ];
        for file in EXPECTED_MODULES {
            let source = fs::read_to_string(source_dir().join(file)).unwrap();
            let source = production_lines(&source);
            for symbol in forbidden {
                assert!(
                    !source.contains(symbol),
                    "{file} references forbidden {symbol}"
                );
            }
        }
    }

    #[test]
    fn agent_bridge_dependencies_are_closed_and_reviewed() {
        let allowed: HashSet<&str> = [
            "crate::agent::domain_profile",
            // H2 换血：结果三态分类器单一真相源（纯函数模块，零发送链依赖）——
            // significance / post_release 按 run_id join 真实用户反应后经它判
            // Hit / Block / Censored。
            "crate::agent::outcome_label",
            "crate::agent::prompt_shadow",
            "crate::agent::run_envelope",
            "crate::agent::runtime",
        ]
        .into_iter()
        .collect();
        for file in EXPECTED_MODULES {
            let source = production_lines(&fs::read_to_string(source_dir().join(file)).unwrap());
            for line in source
                .lines()
                .filter(|line| line.contains("crate::agent::"))
            {
                let start = line.find("crate::agent::").unwrap();
                let suffix = &line[start..];
                let dependency = suffix
                    .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':'))
                    .next()
                    .unwrap();
                let module = dependency
                    .rsplit_once("::")
                    .map(|(prefix, _)| prefix)
                    .unwrap_or(dependency);
                assert!(
                    allowed.contains(dependency) || allowed.contains(module),
                    "{file} has unreviewed agent dependency {dependency}"
                );
            }
        }
    }

    #[test]
    fn replay_persists_only_shadow_replay_rows() {
        let source = production_lines(&fs::read_to_string(source_dir().join("replay.rs")).unwrap());
        let mut last_accessor = "";
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('.') && trimmed.ends_with("()") {
                last_accessor = trimmed.trim_start_matches('.').trim_end_matches("()");
            }
            if trimmed.contains(".insert_one(") || trimmed.starts_with(".insert_one(") {
                assert_eq!(
                    last_accessor, "shadow_replays",
                    "replay write escaped shadow_replays"
                );
            }
            assert!(
                !trimmed.contains(".update_one("),
                "replay must not mutate source/business rows"
            );
            assert!(!trimmed.contains(".delete_"), "replay must not delete rows");
        }
    }

    #[test]
    fn prompt_shadow_bridge_has_no_send_or_write_dependency() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/agent/prompt_shadow.rs");
        let source = production_lines(&fs::read_to_string(path).unwrap());
        for forbidden in [
            "super::outbox",
            "crate::agent::outbox",
            "crate::mcp",
            "insert_one(",
            "update_one(",
            "delete_one(",
            "replace_one(",
        ] {
            assert!(
                !source.contains(forbidden),
                "prompt shadow bridge references {forbidden}"
            );
        }
    }
}
