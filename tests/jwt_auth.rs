//! P1-7 集成测试：JWT RS256 issue → verify → expired → tampered。
//!
//! 这一组测试不需要 testcontainers MongoDB，因为 [`wechatagent::auth::jwt`]
//! 是纯函数（密钥 PEM in、token out），所以脱离 DB 跑得起来；同时也是
//! `#[ignore]` 的反义示例——CI 默认就跑，挡 jwt 模块回归。
//!
//! `src/auth/jwt.rs` 自身也有 `#[cfg(test)] mod tests`；本文件复测同等
//! 路径，确保从 `tests/` 集成视角公开的 API 形态稳定（`JwtKeys::from_config`
//! / `issue_jwt` / `verify_jwt`）。

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use wechatagent::auth::jwt::{issue_jwt, verify_jwt, JwtClaims, JwtKeys};
use wechatagent::config::AppConfig;
use wechatagent::error::AppError;

const PRIVATE_PEM: &str = include_str!("fixtures/jwt_test_private.pem");
const PUBLIC_PEM: &str = include_str!("fixtures/jwt_test_public.pem");

fn cfg_with_keys(ttl: i64) -> AppConfig {
    let mut cfg = base_cfg();
    cfg.jwt_enabled = true;
    cfg.jwt_ttl_minutes = ttl;
    cfg.jwt_private_key_pem = Some(PRIVATE_PEM.to_string());
    cfg.jwt_public_key_pem = Some(PUBLIC_PEM.to_string());
    cfg
}

fn base_cfg() -> AppConfig {
    AppConfig {
        app_host: "127.0.0.1".to_string(),
        app_port: 0,
        app_base_url: "http://localhost".to_string(),
        mongodb_uri: "mongodb://x".to_string(),
        mongodb_database: "x".to_string(),
        mcp_base_url: "x".to_string(),
        mcp_api_key: "x".to_string(),
        openai_base_url: "x".to_string(),
        openai_api_key: "x".to_string(),
        openai_model: "x".to_string(),
        default_workspace_id: "default".to_string(),
        default_account_id: "default".to_string(),
        agent_recent_message_limit: 1,
        agent_min_reply_interval_seconds: 1,
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
        evolution_enabled: false,
        evolution_tick_seconds: 600,
        evolution_run_token_budget: 60_000,
        evolution_run_max_llm_calls: 30,
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
        knowledge_digest_run_token_budget: 60_000,
        knowledge_digest_run_max_llm_calls: 30,
        knowledge_task_worker_interval_seconds: 0,
        catalog_rebuild_worker_interval_seconds: 0,
        knowledge_feedback_interval_seconds: 0,
        ingest_worker_enabled: false,
        ingest_worker_interval_seconds: 0,
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
        jwt_enabled: false,
        jwt_ttl_minutes: 60,
        jwt_private_key_pem: None,
        jwt_public_key_pem: None,
    }
}

#[test]
fn from_config_requires_both_keys_when_enabled() {
    let mut cfg = base_cfg();
    cfg.jwt_enabled = true;
    cfg.jwt_private_key_pem = Some(PRIVATE_PEM.to_string());
    // 公钥缺
    let err = JwtKeys::from_config(&cfg).unwrap_err().to_string();
    assert!(err.contains("JWT_PUBLIC_KEY_PEM"), "got: {err}");
}

#[test]
fn issue_then_verify_round_trips_claims() {
    let cfg = cfg_with_keys(5);
    let keys = JwtKeys::from_config(&cfg).expect("from_config");
    let token = issue_jwt(&keys, "u1", "alice", "ws_default").expect("issue");
    let claims = verify_jwt(&keys, &token).expect("verify");
    assert_eq!(claims.sub, "u1");
    assert_eq!(claims.username, "alice");
    assert_eq!(claims.current_workspace, "ws_default");
    assert!(claims.exp > claims.iat);
}

#[test]
fn tampered_token_returns_token_invalid() {
    let cfg = cfg_with_keys(5);
    let keys = JwtKeys::from_config(&cfg).expect("from_config");
    let token = issue_jwt(&keys, "u1", "alice", "ws").expect("issue");
    let mut bad = token.clone();
    bad.pop();
    bad.push('X');
    match verify_jwt(&keys, &bad) {
        Err(AppError::Unauthorized(msg)) => assert_eq!(msg, "token_invalid"),
        other => panic!("expected token_invalid, got {other:?}"),
    }
}

#[test]
fn expired_token_returns_token_expired() {
    let cfg = cfg_with_keys(5);
    let keys = JwtKeys::from_config(&cfg).expect("from_config");
    // 直接构造一个 exp 在过去的 token，绕开 issue_jwt 的"从 now 算 ttl"。
    let claims = JwtClaims {
        sub: "u1".to_string(),
        username: "alice".to_string(),
        current_workspace: "ws".to_string(),
        iat: Utc::now().timestamp() - 600,
        exp: Utc::now().timestamp() - 60,
    };
    let encoding = EncodingKey::from_rsa_pem(PRIVATE_PEM.as_bytes()).unwrap();
    let token = encode(&Header::new(Algorithm::RS256), &claims, &encoding).unwrap();
    match verify_jwt(&keys, &token) {
        Err(AppError::Unauthorized(msg)) => assert_eq!(msg, "token_expired"),
        other => panic!("expected token_expired, got {other:?}"),
    }
}
