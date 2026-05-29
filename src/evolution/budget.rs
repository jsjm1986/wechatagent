//! [`EvolutionBudget`]：演化器单 tick 的 token / 调用次数预算。
//!
//! 与 `crate::agent::budget::RunBudget` 故意**不**共享类型——运行期完全隔离，
//! 演化器预算耗尽不能通过任何路径影响生产 run 的预算判定。

use crate::config::AppConfig;

use super::error::EvolutionError;

/// 单次 tick 的预算追踪器。配额一旦耗尽，候选生成阶段（W2 prompt critic）
/// SHALL 立刻 stop 并整批 drop 候选，threshold 阶段（纯统计）不受影响。
#[derive(Debug, Clone)]
pub struct EvolutionBudget {
    pub token_limit: i64,
    pub token_used: i64,
    pub call_limit: i32,
    pub call_used: i32,
}

impl EvolutionBudget {
    /// 从 `AppConfig` 构造单 tick 预算。
    pub fn from_config(cfg: &AppConfig) -> Self {
        Self {
            token_limit: cfg.evolution_run_token_budget,
            token_used: 0,
            call_limit: cfg.evolution_run_max_llm_calls,
            call_used: 0,
        }
    }

    /// 检查是否还能再消耗一次调用；耗尽时返回 `BudgetExceeded`。
    pub fn check_or_fail(&self) -> Result<(), EvolutionError> {
        if self.exhausted() {
            return Err(EvolutionError::BudgetExceeded {
                tokens_used: self.token_used,
                calls_used: self.call_used,
            });
        }
        Ok(())
    }

    /// 累加一次 LLM 调用消耗。`tokens` 来自 LLM 返回的 usage 字段；
    /// `calls` 一般传 1（每次 LLM JSON 调用记一次）。
    pub fn record_call(&mut self, tokens: i64, calls: i32) {
        self.token_used = self.token_used.saturating_add(tokens.max(0));
        self.call_used = self.call_used.saturating_add(calls.max(0));
    }

    /// 任一维度（token 或 calls）超过上限即视为耗尽。
    pub fn exhausted(&self) -> bool {
        self.token_used >= self.token_limit || self.call_used >= self.call_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(tokens: i64, calls: i32) -> AppConfig {
        // 用一份最小 AppConfig 测试预算行为；其它字段填占位即可。
        AppConfig {
            app_host: "x".to_string(),
            app_port: 0,
            app_base_url: "x".to_string(),
            mongodb_uri: "x".to_string(),
            mongodb_database: "x".to_string(),
            mcp_base_url: "x".to_string(),
            mcp_api_key: "x".to_string(),
            openai_base_url: "x".to_string(),
            openai_api_key: "x".to_string(),
            openai_model: "x".to_string(),
            default_workspace_id: "x".to_string(),
            default_account_id: "x".to_string(),
            agent_recent_message_limit: 12,
            agent_min_reply_interval_seconds: 20,
            task_worker_interval_seconds: 30,
            llm_timeout_seconds: 5,
            llm_max_retries: 1,
            llm_retry_base_ms: 100,
            task_claim_timeout_seconds: 5,
            reaction_analysis_claim_timeout_seconds: 5,
            webhook_rate_limit_window_seconds: 60,
            webhook_rate_limit_capacity: 1000,
            strategic_planner_enabled: false,
            strategic_planner_interval_seconds: 600,
            strategic_planner_silent_threshold_hours: 72,
            strategic_planner_daily_emit_cap: 20,
            strategic_planner_commitment_imminent_window_hours: 8,
            strategic_planner_commitment_emit_dedup_hours: 24,
            strategic_planner_stage_stagnation_threshold_days: 14,
            strategic_planner_stage_stagnation_recent_inbound_hours: 24,
            strategic_planner_block_rate_window_hours: 24,
            strategic_planner_block_rate_min_runs: 3,
            strategic_planner_block_rate_threshold: 0.6,
            strategic_planner_priority_enabled: true,
            cold_contact_worker_enabled: false,
            cold_contact_threshold_hours: 168,
            cold_contact_daily_emit_cap: 5,
            silence_signal_worker_enabled: false,
            silence_threshold_seconds: 86400,
            silence_signal_interval_seconds: 0,
            silence_signal_daily_cap: 500,
            dynamic_confidence_min_samples: 5,
            dynamic_confidence_real_outcome_enabled: true,
            behavior_signal_metrics_enabled: false,
            knowledge_exploration_enabled: false,
            knowledge_exploration_temperature: 1.0,
            evolution_enabled: false,
            evolution_tick_seconds: 600,
            evolution_run_token_budget: tokens,
            evolution_run_max_llm_calls: calls,
            evolution_eval_window_hours: 72,
            evolution_min_replays: 30,
            evolution_min_send_success_delta: 0.05,
            evolution_min_self_critique_delta: 0.10,
            evolution_max_5gate_hit_increase: 0.10,
            evolution_max_safety_regression_rate: 0.0,
            evolution_replay_concurrency: 4,
            evolution_replay_max_fail_rate: 0.30,
            evolution_threshold_release_cooldown_hours: 24,
            evolution_cohort_per_contact_cap: 3,
            evolution_cohort_sample_per_failure_bucket: 10,
            evolution_auto_release_enabled: false,
            evolution_auto_release_window_hours: 336,
            evolution_auto_release_per_tick_cap: 1,
            knowledge_digest_enabled: false,
            knowledge_digest_run_hour: 9,
            knowledge_digest_run_token_budget: 24000,
            knowledge_digest_run_max_llm_calls: 8,
            knowledge_task_worker_interval_seconds: 30,
            catalog_rebuild_worker_interval_seconds: 0,
            knowledge_feedback_interval_seconds: 0,
            reviewer_dual_enabled: false,
            reviewer_second_provider_base_url: None,
            reviewer_second_provider_api_key: None,
            reviewer_second_provider_model: None,
            reviewer_second_provider_format: "openai".to_string(),
            session_ttl_hours: 8,
            session_cookie_secure: false,
            bootstrap_admin_username: None,
            bootstrap_admin_password: None,
            webhook_verify_signature: false,
            ingest_worker_enabled: false,
            ingest_worker_interval_seconds: 3600,
            jwt_enabled: false,
            jwt_ttl_minutes: 60,
            jwt_private_key_pem: None,
            jwt_public_key_pem: None,
        }
    }

    #[test]
    fn budget_starts_unused() {
        let b = EvolutionBudget::from_config(&cfg(1000, 5));
        assert_eq!(b.token_used, 0);
        assert_eq!(b.call_used, 0);
        assert!(!b.exhausted());
        assert!(b.check_or_fail().is_ok());
    }

    #[test]
    fn budget_exhausts_when_tokens_reach_limit() {
        let mut b = EvolutionBudget::from_config(&cfg(100, 5));
        b.record_call(50, 1);
        assert!(!b.exhausted());
        b.record_call(50, 1);
        assert!(b.exhausted());
        assert!(matches!(
            b.check_or_fail(),
            Err(EvolutionError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn budget_exhausts_when_calls_reach_limit() {
        let mut b = EvolutionBudget::from_config(&cfg(100_000, 2));
        b.record_call(10, 1);
        b.record_call(10, 1);
        assert!(b.exhausted());
    }

    #[test]
    fn record_call_is_saturating_for_negative_inputs() {
        // LLM usage 偶发返回负值（如 0 - cache_hit）时不应回退预算。
        let mut b = EvolutionBudget::from_config(&cfg(100, 5));
        b.record_call(-50, -1);
        assert_eq!(b.token_used, 0);
        assert_eq!(b.call_used, 0);
    }
}
