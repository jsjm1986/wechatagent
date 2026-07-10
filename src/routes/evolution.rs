//! agent-self-evolution M4 W4 Task 5.5：演化器后台 admin 路由（4 个）。
//!
//! - `GET  /api/evolution/experiments?limit=20`：最近 N 个 experiment 信封 + proposals 摘要
//! - `GET  /api/evolution/proposals/:id`：单条详情（含 cohort_run_ids、shadow_replays 聚合、
//!   Critic reasoning、当前生效值/版本用于 diff 对照）
//! - `POST /api/evolution/proposals/:id/release`  body `{ confirmation: "RELEASE"  }`
//! - `POST /api/evolution/proposals/:id/rollback` body `{ confirmation: "ROLLBACK" }`
//!
//! 复用现有 admin 路由约定（与 `admin_outbox.rs` 同款），不引入演化器专属 token；
//! 不发邮件 / IM / push（Requirements 9.5）。
//!
//! **隔离红线**：本文件严禁触达生产链路写入。任意 handler 顶部都贴上 anchor
//! `// FORBIDDEN: enqueue agent_send_outbox / mcp call`。CI lint
//! `scripts/check-evolution-isolation.sh` 也会扫 `src/evolution/` 阻拦相同字面量；
//! routes 这层通过显式注释提醒未来 reviewer。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, Bson, DateTime},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    evolution::{
        error::EvolutionError,
        release::{release_prompt, release_threshold, rollback_prompt, rollback_threshold},
    },
    models::{Experiment, EvolutionRuntimeFlag, Proposal, ShadowReplay, ThresholdOverride},
};

use super::shared::parse_object_id;
use super::AppState;

const DEFAULT_EXPERIMENT_LIMIT: i64 = 20;
const MAX_EXPERIMENT_LIMIT: i64 = 100;

const RELEASE_CONFIRMATION_LITERAL: &str = "RELEASE";
const ROLLBACK_CONFIRMATION_LITERAL: &str = "ROLLBACK";

/// admin 默认操作者 id；UI 没有登录态可注入时（M4 W4 简化路径）落到该常量。
/// 真正的 SSO/admin auth 在外层 middleware 层挂——M4 不引入 evolution 专属 token。
const DEFAULT_RELEASE_ADMIN: &str = "admin";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ListExperimentsQuery {
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConfirmationRequest {
    confirmation: String,
}

/// `GET /api/evolution/experiments?limit=20` —— 最近 N 个 experiment 信封 + proposals 摘要。
pub(super) async fn list_evolution_experiments(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ListExperimentsQuery>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    let limit = query
        .limit
        .unwrap_or(DEFAULT_EXPERIMENT_LIMIT)
        .clamp(1, MAX_EXPERIMENT_LIMIT);

    let workspace_id = admin.current_workspace.clone();
    let account_id = state.config.default_account_id.clone();

    let mut cursor = state
        .db
        .experiments()
        .find(
            doc! { "workspace_id": &workspace_id, "account_id": &account_id },
            FindOptions::builder()
                .sort(doc! { "started_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;

    let mut experiments: Vec<Experiment> = Vec::new();
    while let Some(exp) = cursor.try_next().await? {
        experiments.push(exp);
    }

    let mut items = Vec::with_capacity(experiments.len());
    for exp in &experiments {
        let proposals = load_proposal_summaries(&state, &exp.experiment_id).await?;
        items.push(experiment_summary_json(exp, proposals));
    }

    Ok(Json(json!({ "items": items })))
}

/// `GET /api/evolution/proposals/:id` —— 单条详情。
pub(super) async fn get_evolution_proposal_detail(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    let proposal_id = parse_object_id(&id)?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal not found: {id}")))?;

    let experiment = state
        .db
        .experiments()
        .find_one(doc! { "experiment_id": &proposal.experiment_id }, None)
        .await?;

    let shadow_summary = aggregate_shadow_replays(&state, proposal_id).await?;
    let current_state = load_current_state_for_diff(&state, &proposal).await?;

    Ok(Json(json!({
        "proposal": proposal_detail_json(&proposal),
        "experiment": experiment.as_ref().map(experiment_envelope_json),
        "cohortRunIds": cohort_run_ids_json(experiment.as_ref(), &proposal),
        "shadowReplays": shadow_summary,
        "currentState": current_state,
    })))
}

/// `POST /api/evolution/proposals/:id/release` —— admin 确认串校验 + dispatch。
pub(super) async fn release_evolution_proposal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ConfirmationRequest>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    if payload.confirmation != RELEASE_CONFIRMATION_LITERAL {
        return Err(AppError::BadRequest(format!(
            "confirmation must be exact string \"{RELEASE_CONFIRMATION_LITERAL}\""
        )));
    }
    let proposal_id = parse_object_id(&id)?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal not found: {id}")))?;

    match proposal.proposal_kind.as_str() {
        "threshold" => release_threshold(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        "prompt" => release_prompt(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown proposal_kind: {other}"
            )))
        }
    }

    Ok(Json(json!({
        "ok": true,
        "proposalId": proposal_id.to_hex(),
        "kind": proposal.proposal_kind,
        "action": "released",
    })))
}

/// `POST /api/evolution/proposals/:id/rollback` —— admin 确认串校验 + dispatch。
pub(super) async fn rollback_evolution_proposal(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ConfirmationRequest>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    if payload.confirmation != ROLLBACK_CONFIRMATION_LITERAL {
        return Err(AppError::BadRequest(format!(
            "confirmation must be exact string \"{ROLLBACK_CONFIRMATION_LITERAL}\""
        )));
    }
    let proposal_id = parse_object_id(&id)?;
    let proposal = state
        .db
        .proposals()
        .find_one(
            doc! { "_id": proposal_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal not found: {id}")))?;

    match proposal.proposal_kind.as_str() {
        "threshold" => rollback_threshold(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        "prompt" => rollback_prompt(&state, proposal_id, &admin.username)
            .await
            .map_err(evolution_error_to_app_error)?,
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown proposal_kind: {other}"
            )))
        }
    }

    Ok(Json(json!({
        "ok": true,
        "proposalId": proposal_id.to_hex(),
        "kind": proposal.proposal_kind,
        "action": "rolled_back",
    })))
}

async fn load_proposal_summaries(
    state: &AppState,
    experiment_id: &str,
) -> AppResult<Vec<Proposal>> {
    let mut cursor = state
        .db
        .proposals()
        .find(
            doc! { "experiment_id": experiment_id },
            FindOptions::builder().sort(doc! { "created_at": 1 }).build(),
        )
        .await?;
    let mut out = Vec::new();
    while let Some(p) = cursor.try_next().await? {
        out.push(p);
    }
    Ok(out)
}

async fn aggregate_shadow_replays(
    state: &AppState,
    proposal_id: ObjectId,
) -> AppResult<Value> {
    let mut cursor = state
        .db
        .shadow_replays()
        .find(doc! { "proposal_id": proposal_id }, None)
        .await?;
    let mut completed = 0_i64;
    let mut failed = 0_i64;
    let mut samples: Vec<Value> = Vec::new();
    while let Some(r) = cursor.try_next().await? {
        if r.status == "completed" {
            completed += 1;
        } else if r.status == "failed" {
            failed += 1;
        }
        if samples.len() < 5 {
            samples.push(shadow_replay_json(&r));
        }
    }
    Ok(json!({
        "totalCompleted": completed,
        "totalFailed": failed,
        "samples": samples,
    }))
}

async fn load_current_state_for_diff(
    state: &AppState,
    proposal: &Proposal,
) -> AppResult<Value> {
    match proposal.proposal_kind.as_str() {
        "threshold" => {
            let gate_key = match proposal.gate_key.as_deref() {
                Some(k) if !k.is_empty() => k,
                _ => {
                    return Ok(json!({
                        "kind": "threshold",
                        "gateKey": null,
                        "currentValue": null,
                        "source": "missing_gate_key",
                    }))
                }
            };
            // 优先返回最新生效的 threshold_overrides（rolled_back_at=null），
            // 没有则 fallback 到 AppConfig 内置 baseline（与 threshold.rs::current_threshold_value 同源）。
            let latest_override = state
                .db
                .threshold_overrides()
                .find_one(
                    doc! {
                        "workspace_id": &proposal.workspace_id,
                        "account_id": &proposal.account_id,
                        "gate_key": gate_key,
                        "rolled_back_at": null,
                    },
                    mongodb::options::FindOneOptions::builder()
                        .sort(doc! { "released_at": -1 })
                        .build(),
                )
                .await?;
            let (current_value, source) = match latest_override.as_ref() {
                Some(o) => (o.value, "threshold_overrides"),
                None => (baseline_threshold_value(&state.config, gate_key), "appconfig_baseline"),
            };
            Ok(json!({
                "kind": "threshold",
                "gateKey": gate_key,
                "currentValue": current_value,
                "proposedValue": proposal.proposed_value,
                "source": source,
                "latestOverride": latest_override.as_ref().map(threshold_override_json),
            }))
        }
        "prompt" => {
            let prompt_key = match proposal.proposed_template_key.as_deref() {
                Some(k) if !k.is_empty() => k,
                _ => {
                    return Ok(json!({
                        "kind": "prompt",
                        "promptKey": null,
                        "currentVersion": null,
                        "currentContent": null,
                        "source": "missing_prompt_key",
                    }))
                }
            };
            let current = state
                .db
                .prompt_templates()
                .find_one(
                    doc! {
                        "workspace_id": &proposal.workspace_id,
                        "prompt_key": prompt_key,
                        "current_version": true,
                    },
                    None,
                )
                .await?;
            Ok(json!({
                "kind": "prompt",
                "promptKey": prompt_key,
                "section": proposal.proposed_section,
                "currentVersion": current.as_ref().map(|c| c.version),
                "currentContent": current.as_ref().map(|c| c.content.clone()),
                "previousVersion": current.as_ref().and_then(|c| c.previous_version),
                "source": if current.is_some() { "prompt_templates" } else { "missing" },
            }))
        }
        other => Ok(json!({ "kind": other, "note": "unknown proposal_kind" })),
    }
}

fn baseline_threshold_value(config: &crate::config::AppConfig, gate: &str) -> f64 {
    match gate {
        "fact_risk_block" => 6.0,
        "pressure_risk_block" => 7.0,
        "human_like_score_rewrite" => 6.0,
        "emotional_value_rewrite" => 5.0,
        "product_accuracy_score_block" => 7.0,
        "planner_block_rate_threshold" => config.strategic_planner_block_rate_threshold,
        _ => 0.0,
    }
}

fn evolution_error_to_app_error(err: EvolutionError) -> AppError {
    match err {
        EvolutionError::InvalidStatus(msg) => AppError::BadRequest(msg),
        EvolutionError::RedlineGateRejected(msg) => AppError::BadRequest(msg),
        EvolutionError::Mongo(e) => AppError::Db(e),
        EvolutionError::Bson(e) => AppError::External(format!("bson decode: {e}")),
        EvolutionError::BudgetExceeded {
            tokens_used,
            calls_used,
        } => AppError::External(format!(
            "evolution budget exceeded (tokens_used={tokens_used}, calls_used={calls_used})"
        )),
        EvolutionError::Internal(msg) => AppError::External(msg),
    }
}

fn experiment_summary_json(exp: &Experiment, proposals: Vec<Proposal>) -> Value {
    let mut counts_by_status = std::collections::BTreeMap::<&'static str, i64>::new();
    for p in &proposals {
        let key = match p.status.as_str() {
            "pending_eval" => "pendingEval",
            "evaluating" => "evaluating",
            "eligible_for_release" => "eligibleForRelease",
            "rejected_below_threshold" => "rejectedBelowThreshold",
            "released" => "released",
            "rolled_back" => "rolledBack",
            _ => "other",
        };
        *counts_by_status.entry(key).or_insert(0) += 1;
    }
    json!({
        "experiment": experiment_envelope_json(exp),
        "proposalsCounts": counts_by_status,
        "proposals": proposals.iter().map(proposal_summary_json).collect::<Vec<_>>(),
    })
}

fn experiment_envelope_json(exp: &Experiment) -> Value {
    json!({
        "experimentId": exp.experiment_id,
        "workspaceId": exp.workspace_id,
        "accountId": exp.account_id,
        "status": exp.status,
        "windowHours": exp.window_hours,
        "startedAt": datetime_to_rfc3339(exp.started_at),
        "updatedAt": datetime_to_rfc3339(exp.updated_at),
        "finishedAt": exp.finished_at.map(datetime_to_rfc3339),
        "cohortThresholdSize": exp.cohort_threshold_run_ids.len() as i64,
        "cohortPromptSize": exp.cohort_prompt_run_ids.len() as i64,
        "budgetUsedTokens": exp.budget_used_tokens,
        "budgetUsedCalls": exp.budget_used_calls,
        "proposalsCount": exp.proposals_count,
        "proposalsEligibleCount": exp.proposals_eligible_count,
    })
}

fn proposal_summary_json(p: &Proposal) -> Value {
    json!({
        "id": p.id.map(|o| o.to_hex()),
        "kind": p.proposal_kind,
        "status": p.status,
        "gateKey": p.gate_key,
        "proposedTemplateKey": p.proposed_template_key,
        "proposedSection": p.proposed_section,
        "currentValue": p.current_value,
        "proposedValue": p.proposed_value,
        "significancePassed": p.significance_passed,
        "evalReplaysCompleted": p.eval_replays_completed,
        "evalReplaysFailed": p.eval_replays_failed,
        "failureReason": p.failure_reason,
        "createdAt": datetime_to_rfc3339(p.created_at),
        "updatedAt": datetime_to_rfc3339(p.updated_at),
    })
}

fn proposal_detail_json(p: &Proposal) -> Value {
    json!({
        "id": p.id.map(|o| o.to_hex()),
        "experimentId": p.experiment_id,
        "workspaceId": p.workspace_id,
        "accountId": p.account_id,
        "kind": p.proposal_kind,
        "status": p.status,
        "gateKey": p.gate_key,
        "currentValue": p.current_value,
        "proposedValue": p.proposed_value,
        "cohortNotes": bson_doc_to_json(&p.cohort_notes),
        "proposedTemplateKey": p.proposed_template_key,
        "proposedSection": p.proposed_section,
        "diffSummary": p.diff_summary,
        "diffSnippet": p.diff_snippet,
        "criticReasoning": p.critic_reasoning,
        "expectedImprovementOn": p.expected_improvement_on,
        "riskNote": p.risk_note,
        "previousPromptVersion": p.previous_prompt_version,
        "evalMetrics": bson_doc_to_json(&p.eval_metrics),
        "evalReplaysCompleted": p.eval_replays_completed,
        "evalReplaysFailed": p.eval_replays_failed,
        "significancePassed": p.significance_passed,
        "failureReason": p.failure_reason,
        "releasedAt": p.released_at.map(datetime_to_rfc3339),
        "releasedBy": p.released_by,
        "rolledBackAt": p.rolled_back_at.map(datetime_to_rfc3339),
        "rolledBackBy": p.rolled_back_by,
        "createdAt": datetime_to_rfc3339(p.created_at),
        "updatedAt": datetime_to_rfc3339(p.updated_at),
    })
}

fn cohort_run_ids_json(experiment: Option<&Experiment>, proposal: &Proposal) -> Value {
    match experiment {
        Some(exp) => match proposal.proposal_kind.as_str() {
            "threshold" => json!(exp
                .cohort_threshold_run_ids
                .iter()
                .map(|o| o.to_hex())
                .collect::<Vec<_>>()),
            "prompt" => json!(exp
                .cohort_prompt_run_ids
                .iter()
                .map(|o| o.to_hex())
                .collect::<Vec<_>>()),
            _ => json!([]),
        },
        None => json!([]),
    }
}

fn shadow_replay_json(r: &ShadowReplay) -> Value {
    json!({
        "id": r.id.map(|o| o.to_hex()),
        "sourceRunId": r.source_run_id.to_hex(),
        "status": r.status,
        "failureReason": r.failure_reason,
        "originalFinalReviewStatus": r.original_final_review_status,
        "original5gateHit": bson_doc_to_json(&r.original_5gate_hit),
        "originalSelfCritiqueAddressed": r.original_self_critique_addressed,
        "newFinalReviewStatus": r.new_final_review_status,
        "newReviewRisks": r.new_review_risks,
        "newTokenCost": r.new_token_cost,
        "new5gateHit": bson_doc_to_json(&r.new_5gate_hit),
        "newSelfCritiqueAddressed": r.new_self_critique_addressed,
        "similarityToOriginalText": r.similarity_to_original_text,
        "startedAt": datetime_to_rfc3339(r.started_at),
        "finishedAt": r.finished_at.map(datetime_to_rfc3339),
    })
}

fn threshold_override_json(o: &ThresholdOverride) -> Value {
    json!({
        "id": o.id.map(|x| x.to_hex()),
        "gateKey": o.gate_key,
        "value": o.value,
        "sourceProposalId": o.source_proposal_id.to_hex(),
        "releasedAt": datetime_to_rfc3339(o.released_at),
        "releasedBy": o.released_by,
        "rolledBackAt": o.rolled_back_at.map(datetime_to_rfc3339),
        "rolledBackBy": o.rolled_back_by,
    })
}

fn datetime_to_rfc3339(dt: DateTime) -> String {
    dt.try_to_rfc3339_string()
        .unwrap_or_else(|_| dt.timestamp_millis().to_string())
}

fn bson_doc_to_json(d: &mongodb::bson::Document) -> Value {
    serde_json::to_value(Bson::Document(d.clone())).unwrap_or_else(|_| json!({}))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateRuntimeFlagRequest {
    /// `true`/`false` 总开关；`enabled=false` 时一票否决整个灰度。
    enabled: bool,
    /// 灰度百分比 0..=100；超出范围由 server 钳制。
    rollout_percent: u32,
    /// 操作者审计字段；可选（未登录态写 "admin" 默认值）。
    #[serde(default)]
    updated_by: Option<String>,
    /// EVO-3:per-workspace 自动 release 子闸。None 时不改(保持 upsert 既有值)。
    #[serde(default)]
    threshold_auto_release_enabled: Option<bool>,
}

/// `GET /api/evolution/runtime-flag` —— Phase C / C3 当前灰度配置。
///
/// 文档不存在时返回 `enabled=false, rollout_percent=0` 的逻辑默认（即"未配置=关停"），
/// 不在读路径触发写入；admin 通过 PUT 才显式落库。
pub(super) async fn get_evolution_runtime_flag(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    let workspace_id = admin.current_workspace.clone();
    let flag = state
        .db
        .evolution_runtime_flags()
        .find_one(doc! { "workspace_id": &workspace_id }, None)
        .await?;
    Ok(Json(json!({
        "workspaceId": workspace_id,
        "envEvolutionEnabled": state.config.evolution_enabled,
        "flag": flag.as_ref().map(runtime_flag_json),
    })))
}

/// `PUT /api/evolution/runtime-flag` —— upsert 灰度配置。
pub(super) async fn put_evolution_runtime_flag(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<UpdateRuntimeFlagRequest>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    let workspace_id = admin.current_workspace.clone();
    let rollout_percent = payload.rollout_percent.min(100);
    let updated_by = payload
        .updated_by
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(DEFAULT_RELEASE_ADMIN);
    let now = DateTime::now();

    let mut set_fields = doc! {
        "workspace_id": &workspace_id,
        "enabled": payload.enabled,
        "rollout_percent": rollout_percent as i64,
        "updated_by": updated_by,
        "updated_at": now,
    };
    // EVO-3:子闸字段 None 时不写($set 缺省即保持 upsert 既有值)。
    if let Some(v) = payload.threshold_auto_release_enabled {
        set_fields.insert("threshold_auto_release_enabled", v);
    }

    state
        .db
        .evolution_runtime_flags()
        .update_one(
            doc! { "workspace_id": &workspace_id },
            doc! { "$set": set_fields },
            mongodb::options::UpdateOptions::builder().upsert(true).build(),
        )
        .await?;

    let saved = state
        .db
        .evolution_runtime_flags()
        .find_one(doc! { "workspace_id": &workspace_id }, None)
        .await?
        .ok_or_else(|| {
            AppError::External("evolution_runtime_flags upsert returned no document".to_string())
        })?;
    Ok(Json(json!({
        "ok": true,
        "flag": runtime_flag_json(&saved),
    })))
}

/// `GET /api/evolution/threshold-overrides/audit?gateKey=&limit=` —— Phase C / C5
/// 阈值变更不可变审计日志（release / rollback / auto-release 全量），按 decided_at
/// 倒序。admin 用它追因"某个 gate 何时、被谁、从哪个值改到哪个值、当时命中率多少"。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThresholdAuditQuery {
    gate_key: Option<String>,
    limit: Option<i64>,
}

pub(super) async fn list_threshold_override_audit(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ThresholdAuditQuery>,
) -> AppResult<Json<Value>> {
    // FORBIDDEN: enqueue agent_send_outbox / mcp call
    let workspace_id = admin.current_workspace.clone();
    let account_id = state.config.default_account_id.clone();
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let mut filter = doc! {
        "workspace_id": &workspace_id,
        "account_id": &account_id,
    };
    if let Some(gate_key) = query.gate_key.as_deref().filter(|s| !s.trim().is_empty()) {
        filter.insert("gate_key", gate_key);
    }

    let mut cursor = state
        .db
        .threshold_overrides_audit()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "decided_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(a) = cursor.try_next().await? {
        items.push(threshold_override_audit_json(&a));
    }
    Ok(Json(json!({ "items": items })))
}

fn threshold_override_audit_json(a: &crate::models::ThresholdOverrideAudit) -> Value {
    json!({
        "id": a.id.map(|x| x.to_hex()),
        "gateKey": a.gate_key,
        "action": a.action,
        "previousValue": a.previous_value,
        "newValue": a.new_value,
        "sourceProposalId": a.source_proposal_id.to_hex(),
        "decidedBy": a.decided_by,
        "decidedAt": datetime_to_rfc3339(a.decided_at),
        "hitRateObserved": a.hit_rate_observed,
        "significanceMetrics": a.significance_metrics.as_ref().map(bson_doc_to_json),
    })
}

fn runtime_flag_json(f: &EvolutionRuntimeFlag) -> Value {
    json!({
        "workspaceId": f.workspace_id,
        "enabled": f.enabled,
        "rolloutPercent": f.rollout_percent_clamped(),
        "rolloutPercentRaw": f.rollout_percent,
        "thresholdAutoReleaseEnabled": f.threshold_auto_release_enabled,
        "updatedBy": f.updated_by,
        "updatedAt": datetime_to_rfc3339(f.updated_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_threshold_value_matches_threshold_module() {
        let mut cfg = test_app_config();
        cfg.strategic_planner_block_rate_threshold = 0.42;
        assert_eq!(baseline_threshold_value(&cfg, "fact_risk_block"), 6.0);
        assert_eq!(baseline_threshold_value(&cfg, "pressure_risk_block"), 7.0);
        assert_eq!(baseline_threshold_value(&cfg, "human_like_score_rewrite"), 6.0);
        assert_eq!(baseline_threshold_value(&cfg, "emotional_value_rewrite"), 5.0);
        assert_eq!(baseline_threshold_value(&cfg, "product_accuracy_score_block"), 7.0);
        assert!((baseline_threshold_value(&cfg, "planner_block_rate_threshold") - 0.42).abs() < 1e-9);
        assert_eq!(baseline_threshold_value(&cfg, "unknown_gate"), 0.0);
    }

    #[test]
    fn evolution_error_to_app_error_maps_invalid_status_to_bad_request() {
        let err = EvolutionError::InvalidStatus("proposal not eligible".to_string());
        match evolution_error_to_app_error(err) {
            AppError::BadRequest(msg) => assert!(msg.contains("not eligible")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn evolution_error_redline_rejected_maps_to_bad_request() {
        let err = EvolutionError::RedlineGateRejected("命中禁用词表".to_string());
        match evolution_error_to_app_error(err) {
            AppError::BadRequest(msg) => assert!(msg.contains("禁用词")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn evolution_error_budget_exceeded_maps_to_external() {
        let err = EvolutionError::BudgetExceeded {
            tokens_used: 1234,
            calls_used: 5,
        };
        match evolution_error_to_app_error(err) {
            AppError::External(msg) => {
                assert!(msg.contains("1234"));
                assert!(msg.contains("calls_used=5"));
            }
            other => panic!("expected External, got {other:?}"),
        }
    }

    #[test]
    fn threshold_override_audit_json_carries_value_transition() {
        let audit = crate::models::ThresholdOverrideAudit {
            id: Some(ObjectId::new()),
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            gate_key: "fact_risk_block".to_string(),
            action: "auto_released".to_string(),
            previous_value: Some(6.0),
            new_value: Some(5.5),
            source_proposal_id: ObjectId::new(),
            decided_by: "evolution_auto_release".to_string(),
            decided_at: DateTime::now(),
            hit_rate_observed: Some(0.012),
            significance_metrics: None,
        };
        let v = threshold_override_audit_json(&audit);
        assert_eq!(v["gateKey"], "fact_risk_block");
        assert_eq!(v["action"], "auto_released");
        assert_eq!(v["previousValue"], 6.0);
        assert_eq!(v["newValue"], 5.5);
        assert_eq!(v["decidedBy"], "evolution_auto_release");
        // None significance_metrics 必须序列化成 null，不是缺字段。
        assert!(v["significanceMetrics"].is_null());
    }

    #[test]
    fn shadow_replay_json_exposes_original_side_for_comparison() {
        use mongodb::bson::doc;
        let r = ShadowReplay {
            id: Some(ObjectId::new()),
            proposal_id: ObjectId::new(),
            experiment_id: "EXP1".to_string(),
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            source_run_id: ObjectId::new(),
            status: "completed".to_string(),
            failure_reason: None,
            original_final_review_status: Some("held_by_ai_policy".to_string()),
            original_5gate_hit: doc! {
                "fact_risk_block": true,
                "pressure_risk_block": false,
                "human_like_score_rewrite": false,
                "emotional_value_rewrite": false,
                "product_accuracy_score_block": false,
            },
            original_self_critique_addressed: Some(false),
            new_final_review_status: Some("approved".to_string()),
            new_review_risks: Vec::new(),
            new_token_cost: Some(321),
            new_5gate_hit: doc! {
                "fact_risk_block": false,
                "pressure_risk_block": false,
                "human_like_score_rewrite": false,
                "emotional_value_rewrite": false,
                "product_accuracy_score_block": false,
            },
            new_self_critique_addressed: Some(true),
            similarity_to_original_text: 0.0,
            started_at: DateTime::now(),
            finished_at: Some(DateTime::now()),
        };
        let v = shadow_replay_json(&r);
        // 新增 original 侧对照字段
        assert_eq!(v["original5gateHit"]["fact_risk_block"], true);
        assert_eq!(v["original5gateHit"]["human_like_score_rewrite"], false);
        assert_eq!(v["originalSelfCritiqueAddressed"], false);
        // 既有 new 侧不回归
        assert_eq!(v["new5gateHit"]["fact_risk_block"], false);
        assert_eq!(v["newSelfCritiqueAddressed"], true);
        assert_eq!(v["originalFinalReviewStatus"], "held_by_ai_policy");
    }

    #[test]
    fn shadow_replay_json_empty_original_5gate_is_empty_object() {
        let r = ShadowReplay {
            id: None,
            proposal_id: ObjectId::new(),
            experiment_id: "EXP1".to_string(),
            workspace_id: "default".to_string(),
            account_id: "acct".to_string(),
            source_run_id: ObjectId::new(),
            status: "failed".to_string(),
            failure_reason: Some("source_message_unavailable".to_string()),
            original_final_review_status: None,
            original_5gate_hit: mongodb::bson::Document::new(),
            original_self_critique_addressed: None,
            new_final_review_status: None,
            new_review_risks: Vec::new(),
            new_token_cost: None,
            new_5gate_hit: mongodb::bson::Document::new(),
            new_self_critique_addressed: None,
            similarity_to_original_text: 0.0,
            started_at: DateTime::now(),
            finished_at: None,
        };
        let v = shadow_replay_json(&r);
        // 空 Document → {}；Option None → null
        assert_eq!(v["original5gateHit"], serde_json::json!({}));
        assert!(v["originalSelfCritiqueAddressed"].is_null());
    }

    fn test_app_config() -> crate::config::AppConfig {
        crate::config::AppConfig {
            app_host: "127.0.0.1".to_string(),
            app_port: 0,
            app_base_url: "http://localhost".to_string(),
            mongodb_uri: "mongodb://localhost".to_string(),
            mongodb_database: "x".to_string(),
            mcp_base_url: "http://x".to_string(),
            mcp_api_key: "x".to_string(),
            openai_base_url: "http://x".to_string(),
            openai_api_key: "x".to_string(),
            openai_model: "x".to_string(),
            default_workspace_id: "default".to_string(),
            default_account_id: "default".to_string(),
            agent_recent_message_limit: 12,
            agent_min_reply_interval_seconds: 20,
            account_send_min_interval_ms: 1000,
            account_send_max_interval_ms: 4000,
            agent_reply_max_segment_chars: 120,
            agent_reply_max_segments: 4,
            message_debounce_window_ms: 4000,
            task_worker_interval_seconds: 30,
            completeness_cache_ttl_seconds: 300,
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
            strategic_planner_commitment_fallback_due_hours: 72,
            strategic_planner_commitment_emit_dedup_hours: 24,
            strategic_planner_stage_stagnation_threshold_days: 14,
            strategic_planner_stage_stagnation_recent_inbound_hours: 24,
            strategic_planner_calendar_lookahead_days: 1,
            strategic_planner_calendar_daily_cap: 3,
            strategic_planner_calendar_tz_offset_hours: 8,
            strategic_planner_renewal_lookahead_days: 14,
            strategic_planner_renewal_grace_days: 7,
            strategic_planner_renewal_daily_cap: 3,
            strategic_planner_reactivation_dormant_days: 30,
            strategic_planner_reactivation_cadence_days: 30,
            strategic_planner_reactivation_daily_cap: 3,
            value_tier_mid_threshold_cents: 50000,
            value_tier_high_threshold_cents: 300000,
            strategic_planner_block_rate_window_hours: 24,
            strategic_planner_block_rate_min_runs: 3,
            strategic_planner_block_rate_threshold: 0.6,
            strategic_planner_priority_enabled: true,
            cold_contact_worker_enabled: false,
            cold_contact_threshold_hours: 168,
            cold_contact_daily_emit_cap: 5,
            holding_reply_min_interval_hours: 6.0,
            holding_reply_token_budget: 3000,
            account_daily_send_soft_cap: 500,
            silence_signal_worker_enabled: false,
            silence_threshold_seconds: 86400,
            silence_signal_interval_seconds: 0,
            silence_signal_daily_cap: 500,
            dynamic_confidence_min_samples: 5,
            dynamic_confidence_real_outcome_enabled: true,
            behavior_signal_metrics_enabled: false,
            knowledge_exploration_enabled: false,
            progressive_tier_enabled: true,
            knowledge_exploration_temperature: 1.0,
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
            evolution_max_negative_reaction_increase: 0.05,
            evolution_auto_release_enabled: false,
            evolution_auto_release_window_hours: 336,
            evolution_auto_release_per_tick_cap: 1,
            evolution_auto_release_negative_reaction_gate_enabled: false,
            evolution_auto_release_max_negative_reaction_rate: 0.30,
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
            webhook_timestamp_skew_seconds: 300,
            ingest_worker_enabled: false,
            ingest_worker_interval_seconds: 3600,
            jwt_enabled: false,
            jwt_ttl_minutes: 60,
            jwt_private_key_pem: None,
            jwt_public_key_pem: None,
            media_storage_dir: "./media".to_string(),
            media_max_file_size_mb: 50,
            media_id_cache_ttl_hours: 24,
            wake_jitter_max_seconds: 900,
        }
    }

    /// 契约快照:runtime_flag_json。EvolutionRuntimeFlag 7 字段全量构造
    /// (updated_by 给 Some);rolloutPercent=rollout_percent_clamped() 方法值 +
    /// rolloutPercentRaw=原始字段双发;id 不下发。投影下发 7 顶层键。
    #[test]
    fn runtime_flag_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let f = EvolutionRuntimeFlag {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            enabled: true,
            rollout_percent: 25,
            updated_by: Some("admin-1".to_string()),
            threshold_auto_release_enabled: false,
            updated_at: DateTime::from_millis(1_700_000_000_000),
        };
        let value = runtime_flag_json(&f);
        crate::routes::contract_snapshot::assert_contract_fixture("runtime_flag", value);
    }

    /// 契约快照:threshold_override_json。ThresholdOverride 9 字段全量构造
    /// (rolled_back_at/rolled_back_by 两 Option 给 Some);id→Option.map(to_hex);
    /// source_proposal_id→to_hex()(非 Option)。投影下发 8 顶层键。
    #[test]
    fn threshold_override_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let o = ThresholdOverride {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            gate_key: "fact_risk_block".to_string(),
            value: 5.5,
            source_proposal_id: ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap(),
            released_at: DateTime::from_millis(1_700_000_000_000),
            released_by: "admin-1".to_string(),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_100_000)),
            rolled_back_by: Some("admin-2".to_string()),
        };
        let value = threshold_override_json(&o);
        crate::routes::contract_snapshot::assert_contract_fixture("threshold_override", value);
    }

    /// 契约快照:threshold_override_audit_json。ThresholdOverrideAudit 12 字段全量构造
    /// (previous_value/new_value/hit_rate_observed 给 Some;significance_metrics 给
    /// Some(纯标量 doc!) 暴露 significanceMetrics 非 null);id→Option.map(to_hex);
    /// source_proposal_id→to_hex()。投影下发 10 顶层键。
    #[test]
    fn threshold_override_audit_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let a = crate::models::ThresholdOverrideAudit {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            gate_key: "fact_risk_block".to_string(),
            action: "auto_released".to_string(),
            previous_value: Some(6.0),
            new_value: Some(5.5),
            source_proposal_id: ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap(),
            decided_by: "evolution_auto_release".to_string(),
            decided_at: DateTime::from_millis(1_700_000_000_000),
            hit_rate_observed: Some(0.012),
            significance_metrics: Some(doc! { "pValue": 0.03, "sampleSize": 120 }),
        };
        let value = threshold_override_audit_json(&a);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "threshold_override_audit",
            value,
        );
    }

    /// 契约快照:experiment_envelope_json。Experiment 15 字段全量构造
    /// (finished_at 给 Some;两 cohort Vec 非空——投影只下发 .len() 数字不泄漏 ObjectId);
    /// id 不下发。投影下发 14 顶层键。
    #[test]
    fn experiment_envelope_json_matches_contract_fixture() {
        use mongodb::bson::{oid::ObjectId, DateTime};
        let exp = Experiment {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            status: "evaluating".to_string(),
            window_hours: 24,
            started_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            finished_at: Some(DateTime::from_millis(1_700_000_200_000)),
            cohort_threshold_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap()],
            cohort_prompt_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439013").unwrap()],
            budget_used_tokens: 45000,
            budget_used_calls: 120,
            proposals_count: 5,
            proposals_eligible_count: 2,
        };
        let value = experiment_envelope_json(&exp);
        crate::routes::contract_snapshot::assert_contract_fixture("experiment_envelope", value);
    }

    /// 契约快照:proposal_summary_json。Proposal 29 字段全量构造(各 Option 给 Some、
    /// expected_improvement_on 非空 Vec、cohort_notes/eval_metrics 纯标量 doc!);
    /// id→Option.map(to_hex)。投影下发 14 顶层键(summary 子集)。
    #[test]
    fn proposal_summary_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let p = Proposal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "eligible_for_release".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(5.5),
            cohort_notes: doc! { "hitRate": 0.012 },
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("收紧 fact_risk 阈值".to_string()),
            diff_snippet: Some("- 6.0\n+ 5.5".to_string()),
            critic_reasoning: Some("命中率证据充分".to_string()),
            expected_improvement_on: vec!["fact_accuracy".to_string()],
            risk_note: Some("低风险".to_string()),
            previous_prompt_version: Some("v11".to_string()),
            eval_metrics: doc! { "pValue": 0.03 },
            eval_replays_completed: 20,
            eval_replays_failed: 1,
            significance_passed: Some(true),
            failure_reason: Some("none".to_string()),
            released_at: Some(DateTime::from_millis(1_700_000_300_000)),
            released_by: Some("admin-1".to_string()),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_400_000)),
            rolled_back_by: Some("admin-2".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = proposal_summary_json(&p);
        crate::routes::contract_snapshot::assert_contract_fixture("proposal_summary", value);
    }

    /// 契约快照:shadow_replay_json。ShadowReplay 19 字段全量构造(各 Option 给 Some、
    /// new_review_risks 非空 Vec、两 5gate Document 纯标量 bool doc!);
    /// id→Option.map(to_hex);source_run_id→to_hex()。投影下发 15 顶层键。
    #[test]
    fn shadow_replay_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let r = ShadowReplay {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            proposal_id: ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap(),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            source_run_id: ObjectId::parse_str("507f1f77bcf86cd799439013").unwrap(),
            status: "completed".to_string(),
            failure_reason: Some("none".to_string()),
            original_final_review_status: Some("held_by_ai_policy".to_string()),
            original_5gate_hit: doc! { "fact_risk_block": true, "pressure_risk_block": false },
            original_self_critique_addressed: Some(false),
            new_final_review_status: Some("approved".to_string()),
            new_review_risks: vec!["minor_tone".to_string()],
            new_token_cost: Some(321),
            new_5gate_hit: doc! { "fact_risk_block": false, "pressure_risk_block": false },
            new_self_critique_addressed: Some(true),
            similarity_to_original_text: 0.85,
            started_at: DateTime::from_millis(1_700_000_000_000),
            finished_at: Some(DateTime::from_millis(1_700_000_100_000)),
        };
        let value = shadow_replay_json(&r);
        crate::routes::contract_snapshot::assert_contract_fixture("shadow_replay", value);
    }

    /// 契约快照:proposal_detail_json。Proposal 29 字段全量构造(同 Task5 形态;各 Option
    /// 给 Some、expected_improvement_on 非空、cohort_notes/eval_metrics 纯标量 doc!——
    /// detail 下发 cohortNotes/evalMetrics 故必须纯标量避泄漏);id→Option.map(to_hex)。
    /// 投影下发 29 顶层键。
    #[test]
    fn proposal_detail_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let p = Proposal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "eligible_for_release".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(5.5),
            cohort_notes: doc! { "hitRate": 0.012 },
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("收紧 fact_risk 阈值".to_string()),
            diff_snippet: Some("- 6.0\n+ 5.5".to_string()),
            critic_reasoning: Some("命中率证据充分".to_string()),
            expected_improvement_on: vec!["fact_accuracy".to_string()],
            risk_note: Some("低风险".to_string()),
            previous_prompt_version: Some("v11".to_string()),
            eval_metrics: doc! { "pValue": 0.03 },
            eval_replays_completed: 20,
            eval_replays_failed: 1,
            significance_passed: Some(true),
            failure_reason: Some("none".to_string()),
            released_at: Some(DateTime::from_millis(1_700_000_300_000)),
            released_by: Some("admin-1".to_string()),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_400_000)),
            rolled_back_by: Some("admin-2".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = proposal_detail_json(&p);
        crate::routes::contract_snapshot::assert_contract_fixture("proposal_detail", value);
    }

    /// 契约快照:experiment_summary_json。聚合投影吃 (&Experiment, Vec<Proposal>)。
    /// 顶层 3 键:experiment(嵌套 envelope)/proposalsCounts(BTreeMap status 计数)/
    /// proposals(数组)。构造 1 Experiment + 含 1 Proposal 的非空 Vec。
    #[test]
    fn experiment_summary_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let exp = Experiment {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            status: "evaluating".to_string(),
            window_hours: 24,
            started_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            finished_at: Some(DateTime::from_millis(1_700_000_200_000)),
            cohort_threshold_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439012").unwrap()],
            cohort_prompt_run_ids: vec![ObjectId::parse_str("507f1f77bcf86cd799439013").unwrap()],
            budget_used_tokens: 45000,
            budget_used_calls: 120,
            proposals_count: 1,
            proposals_eligible_count: 1,
        };
        let p = Proposal {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439014").unwrap()),
            experiment_id: "EXP-1".to_string(),
            workspace_id: "ws-1".to_string(),
            account_id: "acc-1".to_string(),
            proposal_kind: "threshold".to_string(),
            status: "eligible_for_release".to_string(),
            gate_key: Some("fact_risk_block".to_string()),
            current_value: Some(6.0),
            proposed_value: Some(5.5),
            cohort_notes: doc! { "hitRate": 0.012 },
            proposed_template_key: Some("reply_agent_main".to_string()),
            proposed_section: Some("policy".to_string()),
            diff_summary: Some("收紧 fact_risk 阈值".to_string()),
            diff_snippet: Some("- 6.0\n+ 5.5".to_string()),
            critic_reasoning: Some("命中率证据充分".to_string()),
            expected_improvement_on: vec!["fact_accuracy".to_string()],
            risk_note: Some("低风险".to_string()),
            previous_prompt_version: Some("v11".to_string()),
            eval_metrics: doc! { "pValue": 0.03 },
            eval_replays_completed: 20,
            eval_replays_failed: 1,
            significance_passed: Some(true),
            failure_reason: Some("none".to_string()),
            released_at: Some(DateTime::from_millis(1_700_000_300_000)),
            released_by: Some("admin-1".to_string()),
            rolled_back_at: Some(DateTime::from_millis(1_700_000_400_000)),
            rolled_back_by: Some("admin-2".to_string()),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = experiment_summary_json(&exp, vec![p]);
        crate::routes::contract_snapshot::assert_contract_fixture("experiment_summary", value);
    }
}
