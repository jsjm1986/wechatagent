//! 评估场景路由：场景增删改查与公式遵从度评估。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Bson, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    models::{Contact, EvaluationScenario},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EvaluationScenarioRequest {
    scenario_id: String,
    title: String,
    #[serde(default)]
    description: String,
    account_id: Option<String>,
    #[serde(default)]
    contact_seed: Document,
    #[serde(default)]
    inbound_messages: Vec<String>,
    #[serde(default)]
    ground_truth: Document,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EvaluationScenarioQuery {
    tag: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FormulaAdherenceRequest {
    account_id: String,
    contact_id: Option<String>,
    #[serde(default)]
    scenario_ids: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

pub(super) async fn list_evaluation_scenarios(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<EvaluationScenarioQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if let Some(tag) = query.tag {
        filter.insert("tags", tag);
    }
    if let Some(status) = query.status {
        filter.insert("status", status);
    }
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let mut cursor = state
        .db
        .evaluation_scenarios()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(item) = cursor.try_next().await? {
        items.push(evaluation_scenario_json(item));
    }
    Ok(Json(json!({ "items": items })))
}

pub(super) async fn create_evaluation_scenario(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<EvaluationScenarioRequest>,
) -> AppResult<Json<Value>> {
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await?;
    let formula_specs = evaluation_formula_specs(&active_profile);
    let status = validated_scenario_status(
        payload.status.as_deref(),
        &payload.ground_truth,
        &formula_specs,
    )?;
    validate_scenario_request(&state, &admin.current_workspace, &payload).await?;
    let now = DateTime::now();
    let scenario = EvaluationScenario {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        scenario_id: payload.scenario_id,
        title: payload.title,
        description: payload.description,
        account_id: payload.account_id,
        contact_seed: payload.contact_seed,
        inbound_messages: payload.inbound_messages,
        ground_truth: payload.ground_truth,
        tags: payload.tags,
        status,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .evaluation_scenarios()
        .insert_one(&scenario, None)
        .await?;
    Ok(Json(json!({ "item": evaluation_scenario_json(scenario) })))
}

pub(super) async fn update_evaluation_scenario(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<EvaluationScenarioRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await?;
    let formula_specs = evaluation_formula_specs(&active_profile);
    let status = validated_scenario_status(
        payload.status.as_deref(),
        &payload.ground_truth,
        &formula_specs,
    )?;
    validate_scenario_request(&state, &admin.current_workspace, &payload).await?;
    let result = state
        .db
        .evaluation_scenarios()
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! {
                "$set": {
                    "scenario_id": payload.scenario_id,
                    "title": payload.title,
                    "description": payload.description,
                    "account_id": payload.account_id,
                    "contact_seed": payload.contact_seed,
                    "inbound_messages": payload.inbound_messages,
                    "ground_truth": payload.ground_truth,
                    "tags": payload.tags,
                    "status": status,
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "evaluation scenario not found".to_string(),
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn delete_evaluation_scenario(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let result = state
        .db
        .evaluation_scenarios()
        .delete_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::NotFound(
            "evaluation scenario not found".to_string(),
        ));
    }
    Ok(Json(json!({ "ok": true })))
}

/// S-18 / Task 18：跑公式遵守度评测，比较模型 review.scores 与 ground_truth。
///
/// 当 evaluation_scenarios 为空时返回 `200 OK` 加 `summary.degraded=true`，便于
/// CI 流水线和 UI 自检不会因数据不全而中断。
pub(super) async fn run_formula_adherence_evaluation(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<FormulaAdherenceRequest>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &admin.current_workspace, &payload.account_id).await?;
    let mut filter = active_scenario_filter(&admin.current_workspace, &payload.account_id);
    if !payload.scenario_ids.is_empty() {
        filter.insert("scenario_id", doc! { "$in": payload.scenario_ids });
    }
    if !payload.tags.is_empty() {
        filter.insert("tags", doc! { "$in": payload.tags });
    }
    let mut cursor = state
        .db
        .evaluation_scenarios()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .build(),
        )
        .await?;
    let mut scenarios = Vec::new();
    while let Some(scenario) = cursor.try_next().await? {
        scenarios.push(scenario);
    }
    if scenarios.is_empty() {
        return Ok(Json(json!({
            "summary": {
                "degraded": true,
                "reason": "no_scenarios",
                "meanAdherence": 0.0
            },
            "items": Vec::<Value>::new()
        })));
    }

    // 跨场景预算上限。每个 simulation 返回自己的 task-local RunBudgetSnapshot；
    // 这里只累计本 evaluation 启动的子 run，不读取共享生产日志。
    let domain_config = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "domain": "user_operations"
            },
            None,
        )
        .await?;
    let typed_runtime = domain_config
        .as_ref()
        .map(|cfg| cfg.runtime_parameters_typed())
        .unwrap_or_default();
    // H13：eval 种子 contact 的初始 operation_state 从 active 状态机取（替代写死 "new_contact"）。
    let seed_initial_state = agent::initial_operation_state_key(domain_config.as_ref());
    let total_token_budget = typed_runtime
        .simulation_token_budget
        .saturating_mul(scenarios.len() as i64);

    let base_contact = match payload.contact_id.as_deref() {
        Some(id) => Some(
            find_contact_by_id_for_account(
                &state,
                &admin.current_workspace,
                &payload.account_id,
                id,
            )
            .await?,
        ),
        None => None,
    };

    // H15：经营公式 + 缺失回落 score key 从 active profile 读（替代写死四公式数组 +
    // score_key_for 映射）。profile.business_formulas 为空（老库无字段/profile 漏配）时
    // 回落内置销售四公式 + score_key_for——DEFAULT_PROFILE 已 seed 四公式，故等价。
    let active_profile =
        agent::domain_profile::load_active_domain_profile(&state.db, &admin.current_workspace)
            .await?;
    let formula_specs = evaluation_formula_specs(&active_profile);
    let mut items: Vec<Value> = Vec::new();
    let mut total_adherence = 0.0_f64;
    let mut counted = 0usize;
    let mut total_tokens_used: i64 = 0;
    let mut total_llm_calls_used: i32 = 0;
    let mut unknown_usage_calls: i32 = 0;
    let mut unscored_count = 0usize;
    let mut degraded = false;
    let mut degraded_reason: Option<&'static str> = None;
    let mut processed_before_budget = 0_usize;
    for scenario in scenarios {
        // 波 C2：进入下一个场景前先看预算是否已经超额。
        if total_tokens_used >= total_token_budget {
            degraded = true;
            degraded_reason = Some("evaluation_budget_exceeded");
            break;
        }
        let truth = validate_ground_truth(&scenario.ground_truth, &formula_specs);
        if !truth.is_valid() {
            unscored_count += 1;
            items.push(json!({
                "scenarioId": scenario.scenario_id,
                "title": scenario.title,
                "groundTruth": &scenario.ground_truth,
                "unscored": true,
                "reason": "missing_or_invalid_ground_truth",
                "missingGroundTruth": truth.missing,
                "invalidGroundTruth": truth.invalid,
            }));
            continue;
        }
        let messages: Vec<String> = scenario.inbound_messages.clone();
        if messages.is_empty() {
            items.push(json!({
                "scenarioId": scenario.scenario_id,
                "skipped": true,
                "reason": "no_inbound_messages"
            }));
            continue;
        }
        let contact = base_contact.clone().unwrap_or_else(|| {
            scenario_contact_from_seed(
                &admin.current_workspace,
                &payload.account_id,
                &scenario,
                &seed_initial_state,
            )
        });
        let simulation =
            match agent::simulate_user_dialogue_with_budget(&state, contact, messages).await {
                Ok(outcome) => outcome,
                Err(err) => {
                    items.push(json!({
                        "scenarioId": scenario.scenario_id,
                        "error": err.to_string()
                    }));
                    continue;
                }
            };
        total_tokens_used = total_tokens_used.saturating_add(simulation.budget.tokens_used);
        total_llm_calls_used =
            total_llm_calls_used.saturating_add(simulation.budget.llm_calls_used);
        unknown_usage_calls =
            unknown_usage_calls.saturating_add(simulation.budget.unknown_usage_calls);
        let turns = match simulation.turns {
            Ok(turns) => turns,
            Err(err) => {
                items.push(json!({
                    "scenarioId": scenario.scenario_id,
                    "error": err.to_string(),
                    "tokensUsed": simulation.budget.tokens_used,
                    "llmCallsUsed": simulation.budget.llm_calls_used,
                    "unknownUsageCalls": simulation.budget.unknown_usage_calls,
                }));
                if simulation.budget.unknown_usage_calls > 0 {
                    degraded = true;
                    degraded_reason = Some("evaluation_budget_usage_unknown");
                    break;
                }
                continue;
            }
        };

        let last = turns.last();
        let mut deviations = serde_json::Map::new();
        let mut predicted = serde_json::Map::new();
        let mut total_delta = 0.0_f64;
        let mut formula_count = 0u32;
        let mut missing_count = 0u32;
        for (formula, score_key) in &formula_specs {
            let formula = formula.as_str();
            let predicted_value = last
                .and_then(|t| t.review.get_document("formulaBreakdown").ok())
                .and_then(|fb| fb.get(formula).cloned())
                .or_else(|| {
                    last.and_then(|t| t.review.get_document("scores").ok())
                        .and_then(|s| s.get(score_key.as_str()).cloned())
                });
            let Some(predicted_value) = predicted_value else {
                deviations.insert(formula.to_string(), json!("missing"));
                predicted.insert(formula.to_string(), Value::Null);
                missing_count += 1;
                continue;
            };
            let predicted_num = bson_to_f64(&predicted_value);
            let truth_num = truth
                .values
                .get(formula)
                .copied()
                .expect("validated ground truth contains every formula");
            let delta = (predicted_num - truth_num).abs();
            deviations.insert(formula.to_string(), json!(delta));
            predicted.insert(formula.to_string(), json!(predicted_num));
            total_delta += delta;
            formula_count += 1;
        }

        // 波 C2：所有公式都缺失时标 invalid，不静默以 0 分参与平均。
        if formula_count == 0 {
            items.push(json!({
                "scenarioId": scenario.scenario_id,
                "title": scenario.title,
                "predicted": Value::Object(predicted),
                "groundTruth": &scenario.ground_truth,
                "deviations": Value::Object(deviations),
                "invalid": true,
                "invalidReason": "all_formulas_missing",
                "missingFormulas": missing_count
            }));
            if simulation.budget.unknown_usage_calls > 0 {
                degraded = true;
                degraded_reason = Some("evaluation_budget_usage_unknown");
                break;
            }
            continue;
        }

        let mean_delta = total_delta / formula_count as f64;
        let adherence_score = (1.0 - (mean_delta / 10.0)).max(0.0);
        total_adherence += adherence_score;
        counted += 1;
        processed_before_budget += 1;
        items.push(json!({
            "scenarioId": scenario.scenario_id,
            "title": scenario.title,
            "predicted": Value::Object(predicted),
            "groundTruth": &scenario.ground_truth,
            "deviations": Value::Object(deviations),
            "adherenceScore": adherence_score,
            "missingFormulas": missing_count
        }));
        if simulation.budget.unknown_usage_calls > 0 {
            degraded = true;
            degraded_reason = Some("evaluation_budget_usage_unknown");
            break;
        }
    }

    let mean_adherence = if counted > 0 {
        total_adherence / counted as f64
    } else {
        0.0
    };

    // 留痕。
    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: payload.account_id.clone(),
                contact_wxid: None,
                kind: "formula_adherence_evaluated".to_string(),
                status: if degraded { "degraded" } else { "success" }.to_string(),
                summary: format!(
                    "完成 {counted} 个场景的公式遵守度评测，平均 adherence = {:.2}",
                    mean_adherence
                ),
                details: Some(doc! {
                    "scenarioCount": counted as i32,
                    "meanAdherence": mean_adherence,
                    "degraded": degraded,
                    "degradedReason": degraded_reason.map(|s| s.to_string()),
                    "processedBeforeBudgetExceeded": processed_before_budget as i32,
                    "totalTokensUsed": total_tokens_used,
                    "totalLlmCallsUsed": total_llm_calls_used,
                    "unknownUsageCalls": unknown_usage_calls,
                    "unscoredCount": unscored_count as i32,
                    "totalTokenBudget": total_token_budget,
                }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;

    Ok(Json(json!({
        "summary": {
            "degraded": degraded,
            "degradedReason": degraded_reason,
            "processedBeforeBudgetExceeded": processed_before_budget,
            "scenarioCount": counted,
            "meanAdherence": mean_adherence,
            "totalTokensUsed": total_tokens_used,
            "totalLlmCallsUsed": total_llm_calls_used,
            "unknownUsageCalls": unknown_usage_calls,
            "usageComplete": unknown_usage_calls == 0,
            "unscoredCount": unscored_count,
            "totalTokenBudget": total_token_budget
        },
        "items": items
    })))
}

#[derive(Debug, Default, PartialEq)]
struct GroundTruthValidation {
    values: std::collections::HashMap<String, f64>,
    missing: Vec<String>,
    invalid: Vec<String>,
}

impl GroundTruthValidation {
    fn is_valid(&self) -> bool {
        self.missing.is_empty() && self.invalid.is_empty()
    }
}

fn evaluation_formula_specs(profile: &crate::models::DomainProfile) -> Vec<(String, String)> {
    if profile.business_formulas.is_empty() {
        crate::agent::domain_profile::default_business_formulas()
    } else {
        profile.business_formulas.clone()
    }
    .into_iter()
    .map(|formula| {
        let score_key = formula
            .eval_score_key
            .unwrap_or_else(|| score_key_for(&formula.key).to_string());
        (formula.key, score_key)
    })
    .collect()
}

fn active_scenario_filter(workspace_id: &str, account_id: &str) -> Document {
    doc! {
        "workspace_id": workspace_id,
        "status": "active",
        "$or": [
            { "account_id": { "$exists": false } },
            { "account_id": null },
            { "account_id": account_id },
        ]
    }
}

fn validate_ground_truth(
    ground_truth: &Document,
    formula_specs: &[(String, String)],
) -> GroundTruthValidation {
    let mut result = GroundTruthValidation::default();
    for (formula, _) in formula_specs {
        match ground_truth.get(formula) {
            None => result.missing.push(formula.clone()),
            Some(value) => match strict_score(value) {
                Some(score) => {
                    result.values.insert(formula.clone(), score);
                }
                None => result.invalid.push(formula.clone()),
            },
        }
    }
    result
}

fn strict_score(value: &Bson) -> Option<f64> {
    let score = match value {
        Bson::Int32(value) => *value as f64,
        Bson::Int64(value) => *value as f64,
        Bson::Double(value) => *value,
        Bson::Decimal128(value) => value.to_string().parse().ok()?,
        _ => return None,
    };
    score
        .is_finite()
        .then_some(score)
        .filter(|score| (0.0..=10.0).contains(score))
}

fn validated_scenario_status(
    requested: Option<&str>,
    ground_truth: &Document,
    formula_specs: &[(String, String)],
) -> AppResult<String> {
    let validation = validate_ground_truth(ground_truth, formula_specs);
    let status = requested.unwrap_or_else(|| {
        if validation.is_valid() {
            "active"
        } else {
            "draft"
        }
    });
    if !matches!(status, "active" | "draft") {
        return Err(AppError::BadRequest(
            "status must be active|draft".to_string(),
        ));
    }
    if status == "active" && !validation.is_valid() {
        return Err(AppError::BadRequest(format!(
            "active scenario requires complete numeric 0..10 groundTruth; missing={:?}, invalid={:?}",
            validation.missing, validation.invalid
        )));
    }
    Ok(status.to_string())
}

async fn validate_scenario_request(
    state: &AppState,
    workspace_id: &str,
    payload: &EvaluationScenarioRequest,
) -> AppResult<()> {
    if payload.scenario_id.trim().is_empty() {
        return Err(AppError::BadRequest("scenarioId is required".to_string()));
    }
    if payload.title.trim().is_empty() {
        return Err(AppError::BadRequest("title is required".to_string()));
    }
    if payload.inbound_messages.is_empty()
        || payload
            .inbound_messages
            .iter()
            .any(|message| message.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "inboundMessages must contain non-empty messages".to_string(),
        ));
    }
    if let Some(account_id) = payload.account_id.as_deref() {
        if account_id.trim().is_empty() {
            return Err(AppError::BadRequest(
                "accountId must not be empty".to_string(),
            ));
        }
        validate_account(state, workspace_id, account_id).await?;
    }
    Ok(())
}

fn scenario_contact_from_seed(
    workspace_id: &str,
    account_id: &str,
    scenario: &EvaluationScenario,
    initial_state: &str,
) -> Contact {
    let now = DateTime::now();
    let seed = &scenario.contact_seed;
    let wxid = seed
        .get_str("wxid")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("eval_{}", scenario.scenario_id));
    Contact {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        wxid,
        nickname: seed.get_str("nickname").ok().map(ToString::to_string),
        remark: seed.get_str("remark").ok().map(ToString::to_string),
        alias: None,
        avatar_url: None,
        sex: None,
        agent_status: crate::models::AgentStatus::Managed,
        human_profile_note: seed
            .get_str("humanProfileNote")
            .or_else(|_| seed.get_str("human_profile_note"))
            .ok()
            .map(ToString::to_string),
        custom_agent_instructions: seed
            .get_str("customAgentInstructions")
            .or_else(|_| seed.get_str("custom_agent_instructions"))
            .ok()
            .map(ToString::to_string),
        operation_mode_override: None,
        agent_profile: None,
        memory_summary: seed
            .get_str("memorySummary")
            .or_else(|_| seed.get_str("memory_summary"))
            .ok()
            .map(ToString::to_string),
        playbook_id: None,
        playbook_version: None,
        manual_tags: Vec::new(),
        manual_tags_updated_at: None,
        manual_tags_by: None,
        confirmed_tags: Vec::new(),
        bayesian_signals: Vec::new(),
        personality_profile: None,
        tags_version: 0,
        domain_attributes: {
            let mut doc = Document::new();
            if let Some(value) = seed
                .get_str("customerStage")
                .or_else(|_| seed.get_str("customer_stage"))
                .ok()
            {
                doc.insert("customer_stage", value);
            }
            if let Some(value) = seed
                .get_str("intentLevel")
                .or_else(|_| seed.get_str("intent_level"))
                .ok()
            {
                doc.insert("intent_level", value);
            }
            if doc.is_empty() {
                None
            } else {
                Some(doc)
            }
        },
        domain_attributes_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: seed
            .get_str("operationState")
            .or_else(|_| seed.get_str("operation_state"))
            .ok()
            .map(ToString::to_string)
            // H13：种子未指定时回落状态机初始态（替代写死 "new_contact"）。
            .or_else(|| Some(initial_state.to_string())),
        operation_state_reason: None,
        operation_state_confidence: Some(8),
        operation_state_updated_at: Some(now),
        cooldown_until: None,
        operation_policy: Document::new(),
        profile_attributes: seed
            .get_document("profileAttributes")
            .or_else(|_| seed.get_document("profile_attributes"))
            .cloned()
            .unwrap_or_default(),
        profile_updated_at: Some(now),
        last_message_at: None,
        last_inbound_at: None,
        last_outbound_at: None,
        last_agent_run_at: None,
        last_outbound_style: None,
        intent_trajectory: Vec::new(),
        outcome_events: Vec::new(),
        locale: None,
        created_at: now,
        updated_at: now,
    }
}

pub(super) fn evaluation_scenario_json(item: EvaluationScenario) -> Value {
    json!({
        "id": item.id.map(|id| id.to_hex()).unwrap_or_default(),
        "scenarioId": item.scenario_id,
        "title": item.title,
        "description": item.description,
        "accountId": item.account_id,
        "contactSeed": item.contact_seed,
        "inboundMessages": item.inbound_messages,
        "groundTruth": item.ground_truth,
        "tags": item.tags,
        "status": item.status,
        "createdAt": crate::models::dt_to_string(item.created_at),
        "updatedAt": crate::models::dt_to_string(item.updated_at)
    })
}

pub(super) fn score_key_for(formula: &str) -> &'static str {
    // Review.scores 的 key 命名与 formula_breakdown 不完全一致；这里映射近似项作为 fallback。
    match formula {
        "trust" => "humanLike",
        "conversionReadiness" => "conversionReadiness",
        "emotionalValue" => "emotionalValue",
        "nextBestActionScore" => "relationshipProgress",
        _ => "humanLike",
    }
}

pub(super) fn bson_to_f64(value: &mongodb::bson::Bson) -> f64 {
    match value {
        mongodb::bson::Bson::Int32(i) => *i as f64,
        mongodb::bson::Bson::Int64(i) => *i as f64,
        mongodb::bson::Bson::Double(f) => *f,
        mongodb::bson::Bson::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::Bson;

    /// 波 C2：bson_to_f64 处理多种数值类型并降级 0.0。
    #[test]
    fn bson_to_f64_handles_numeric_kinds() {
        assert_eq!(bson_to_f64(&Bson::Int32(7)), 7.0);
        assert_eq!(bson_to_f64(&Bson::Int64(9)), 9.0);
        assert_eq!(bson_to_f64(&Bson::Double(3.5)), 3.5);
        assert_eq!(bson_to_f64(&Bson::String("4.2".into())), 4.2);
        assert_eq!(bson_to_f64(&Bson::Boolean(true)), 0.0);
        assert_eq!(bson_to_f64(&Bson::Null), 0.0);
    }

    /// 波 C2：四个公式都映射到 review.scores 中的合理 fallback key。
    #[test]
    fn score_key_for_maps_all_formulas() {
        assert_eq!(score_key_for("trust"), "humanLike");
        assert_eq!(score_key_for("conversionReadiness"), "conversionReadiness");
        assert_eq!(score_key_for("emotionalValue"), "emotionalValue");
        assert_eq!(score_key_for("nextBestActionScore"), "relationshipProgress");
        assert_eq!(score_key_for("unknown"), "humanLike");
    }

    #[test]
    fn active_scenario_filter_allows_global_and_requested_account_only() {
        assert_eq!(
            active_scenario_filter("ws-a", "account-a"),
            doc! {
                "workspace_id": "ws-a",
                "status": "active",
                "$or": [
                    { "account_id": { "$exists": false } },
                    { "account_id": null },
                    { "account_id": "account-a" },
                ]
            }
        );
    }

    #[test]
    fn ground_truth_requires_every_formula_as_numeric_zero_to_ten() {
        let specs = vec![
            ("trust".to_string(), "humanLike".to_string()),
            ("emotionalValue".to_string(), "emotionalValue".to_string()),
        ];
        let complete = validate_ground_truth(&doc! { "trust": 7, "emotionalValue": 8.5 }, &specs);
        assert!(complete.is_valid());
        assert_eq!(complete.values.get("trust"), Some(&7.0));

        let decimal = "9.25".parse::<mongodb::bson::Decimal128>().unwrap();
        let decimal_truth =
            validate_ground_truth(&doc! { "trust": decimal, "emotionalValue": 8 }, &specs);
        assert!(decimal_truth.is_valid());
        assert_eq!(decimal_truth.values.get("trust"), Some(&9.25));

        let invalid = validate_ground_truth(&doc! { "trust": "7", "emotionalValue": 11 }, &specs);
        assert_eq!(invalid.invalid, vec!["trust", "emotionalValue"]);
        assert!(invalid.missing.is_empty());

        let missing = validate_ground_truth(&doc! { "trust": 7 }, &specs);
        assert_eq!(missing.missing, vec!["emotionalValue"]);
        assert!(!missing.is_valid());
    }

    #[test]
    fn incomplete_truth_defaults_to_draft_and_cannot_be_explicitly_active() {
        let specs = vec![("trust".to_string(), "humanLike".to_string())];
        assert_eq!(
            validated_scenario_status(None, &Document::new(), &specs).unwrap(),
            "draft"
        );
        assert_eq!(
            validated_scenario_status(None, &doc! { "trust": 7 }, &specs).unwrap(),
            "active"
        );
        assert!(validated_scenario_status(Some("active"), &Document::new(), &specs).is_err());
        assert!(validated_scenario_status(Some("archived"), &doc! { "trust": 7 }, &specs).is_err());
    }

    /// 第 77 点护栏补盲区：`score_key_for`（evaluations fallback 映射）与
    /// `default_business_formulas` 的 `eval_score_key`（单一真相源）是手工维护的两份
    /// 映射。本测试锁死二者一致——改了 single source 的 eval_score_key 却忘改 fallback
    /// （或反之）时测试即红，防止两份 DEFAULT 销售映射静默漂移。
    #[test]
    fn score_key_for_matches_default_formula_eval_keys() {
        for f in crate::agent::domain_profile::default_business_formulas() {
            let eval_key = f
                .eval_score_key
                .as_deref()
                .expect("DEFAULT 四公式都应显式声明 eval_score_key");
            assert_eq!(
                score_key_for(&f.key),
                eval_key,
                "公式 {} 的 fallback score_key_for 与 single-source eval_score_key 漂移",
                f.key
            );
        }
    }

    /// 契约快照:evaluation_scenario_json。EvaluationScenario 13 字段全量构造
    /// (account_id 给 Some;contact_seed/ground_truth 用纯标量 doc! 照搬生产种子形状——
    /// 整数走 bson Int32 渲染干净不泄漏;铁律:绝不塞 ObjectId/DateTime);
    /// id→Option.map(to_hex).unwrap_or_default();created_at/updated_at→dt_to_string。
    /// 投影下发 12 顶层键(漏发 workspaceId)。
    #[test]
    fn evaluation_scenario_json_matches_contract_fixture() {
        use mongodb::bson::{doc, oid::ObjectId, DateTime};
        let item = EvaluationScenario {
            id: Some(ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap()),
            workspace_id: "ws-1".to_string(),
            scenario_id: "example_high_intent_user".to_string(),
            title: "高意向用户主动询问产品能力".to_string(),
            description: "用户主动表达需求并询问能否落地".to_string(),
            account_id: Some("acc-1".to_string()),
            contact_seed: doc! { "operationState": "need_discovery", "intentLevel": "高意向" },
            inbound_messages: vec!["AI 能不能帮忙跟进?".to_string()],
            ground_truth: doc! {
                "trust": 7,
                "conversionReadiness": 6,
                "emotionalValue": 7,
                "nextBestActionScore": 7
            },
            tags: vec!["example".to_string(), "high_intent".to_string()],
            status: "active".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        };
        let value = evaluation_scenario_json(item);
        crate::routes::contract_snapshot::assert_contract_fixture("evaluation_scenario", value);
    }
}
