//! 评估场景路由：场景增删改查与公式遵从度评估。

use axum::{
    extract::{Path, Query, State},
    Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
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
    Query(query): Query<EvaluationScenarioQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &state.config.default_workspace_id };
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
    Json(payload): Json<EvaluationScenarioRequest>,
) -> AppResult<Json<Value>> {
    if payload.scenario_id.trim().is_empty() {
        return Err(AppError::BadRequest("scenarioId is required".to_string()));
    }
    let now = DateTime::now();
    let scenario = EvaluationScenario {
        id: None,
        workspace_id: state.config.default_workspace_id.clone(),
        scenario_id: payload.scenario_id,
        title: payload.title,
        description: payload.description,
        account_id: payload.account_id,
        contact_seed: payload.contact_seed,
        inbound_messages: payload.inbound_messages,
        ground_truth: payload.ground_truth,
        tags: payload.tags,
        status: payload.status.unwrap_or_else(|| "active".to_string()),
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
    Path(id): Path<String>,
    Json(payload): Json<EvaluationScenarioRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .evaluation_scenarios()
        .update_one(
            doc! { "_id": object_id },
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
                    "status": payload.status.unwrap_or_else(|| "active".to_string()),
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn delete_evaluation_scenario(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .evaluation_scenarios()
        .delete_one(doc! { "_id": object_id }, None)
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// S-18 / Task 18：跑公式遵守度评测，比较模型 review.scores 与 ground_truth。
///
/// 当 evaluation_scenarios 为空时返回 `200 OK` 加 `summary.degraded=true`，便于
/// CI 流水线和 UI 自检不会因数据不全而中断。
pub(super) async fn run_formula_adherence_evaluation(
    State(state): State<AppState>,
    Json(payload): Json<FormulaAdherenceRequest>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &payload.account_id).await?;
    let mut filter = doc! {
        "workspace_id": &state.config.default_workspace_id,
        "status": "active"
    };
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

    // 波 C2：跨场景预算上限。simulate_user_dialogue 内部各自有一份子预算，
    // 这里维护一个 evaluation 总预算上限：每个场景跑完后把子 run 实际 token
    // 消耗（从 agent_run_logs 累加）汇总进来；超额就 break 并把 degraded
    // 字段设为 true，items 中只保留已完成场景。
    let typed_runtime = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "domain": "user_operations"
            },
            None,
        )
        .await?
        .map(|cfg| cfg.runtime_parameters_typed())
        .unwrap_or_default();
    let total_token_budget = typed_runtime
        .simulation_token_budget
        .saturating_mul(scenarios.len() as i64);

    let base_contact = match payload.contact_id.as_deref() {
        Some(id) => Some(find_contact_by_id(&state, id).await?),
        None => None,
    };

    let formulas = [
        "trust",
        "conversionReadiness",
        "emotionalValue",
        "nextBestActionScore",
    ];
    let mut items: Vec<Value> = Vec::new();
    let mut total_adherence = 0.0_f64;
    let mut counted = 0usize;
    let mut total_tokens_used: i64 = 0;
    let mut degraded = false;
    let mut degraded_reason: Option<&'static str> = None;
    let mut processed_before_budget = 0_usize;
    let evaluation_started_at = DateTime::now();

    for scenario in scenarios {
        // 波 C2：进入下一个场景前先看预算是否已经超额。
        if total_tokens_used >= total_token_budget {
            degraded = true;
            degraded_reason = Some("evaluation_budget_exceeded");
            break;
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
        let contact = base_contact
            .clone()
            .unwrap_or_else(|| scenario_contact_from_seed(&state, &payload.account_id, &scenario));
        let turns = match agent::simulate_user_dialogue(&state, contact, messages).await {
            Ok(t) => t,
            Err(err) => {
                items.push(json!({
                    "scenarioId": scenario.scenario_id,
                    "error": err.to_string()
                }));
                continue;
            }
        };
        // 波 C2：把这次 simulate 实际消耗的 token 累计到 evaluation 总预算。
        // 用 evaluation 启动后的 agent_run_logs 时间戳过滤当前场景的子 run。
        // 简化处理：累加自评测开始至今的所有 run（多场景共享 account），
        // 不会重复计数因为我们每次循环之后才做一次累加。
        let scenario_tokens = sum_scenario_tokens(&state, &payload.account_id, evaluation_started_at)
            .await
            .saturating_sub(total_tokens_used);
        total_tokens_used = total_tokens_used.saturating_add(scenario_tokens);

        let last = turns.last();
        let mut deviations = serde_json::Map::new();
        let mut predicted = serde_json::Map::new();
        let mut total_delta = 0.0_f64;
        let mut formula_count = 0u32;
        let mut missing_count = 0u32;
        for formula in formulas {
            let predicted_value = last
                .and_then(|t| t.review.get_document("formulaBreakdown").ok())
                .and_then(|fb| fb.get(formula).cloned())
                .or_else(|| {
                    last.and_then(|t| t.review.get_document("scores").ok())
                        .and_then(|s| s.get(score_key_for(formula)).cloned())
                });
            let Some(predicted_value) = predicted_value else {
                deviations.insert(formula.to_string(), json!("missing"));
                predicted.insert(formula.to_string(), Value::Null);
                missing_count += 1;
                continue;
            };
            let predicted_num = bson_to_f64(&predicted_value);
            let truth_num = scenario
                .ground_truth
                .get(formula)
                .map(bson_to_f64)
                .unwrap_or(0.0);
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
                workspace_id: state.config.default_workspace_id.clone(),
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
                    "totalTokenBudget": total_token_budget,
                }),
                created_at: DateTime::now(),
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
            "totalTokenBudget": total_token_budget
        },
        "items": items
    })))
}

/// 波 C2：累加从 `since` 起到现在 evaluation 这个 account 上的所有 agent_run_logs.tokens_used。
async fn sum_scenario_tokens(state: &AppState, account_id: &str, since: DateTime) -> i64 {
    let mut total = 0_i64;
    let Ok(mut cur) = state
        .db
        .agent_run_logs()
        .find(
            doc! {
                "workspace_id": &state.config.default_workspace_id,
                "account_id": account_id,
                "created_at": { "$gte": since }
            },
            None,
        )
        .await
    else {
        return 0;
    };
    while let Ok(Some(run)) = cur.try_next().await {
        total = total.saturating_add(run.tokens_used);
    }
    total
}

fn scenario_contact_from_seed(
    state: &AppState,
    account_id: &str,
    scenario: &EvaluationScenario,
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
        workspace_id: state.config.default_workspace_id.clone(),
        account_id: account_id.to_string(),
        wxid,
        nickname: seed.get_str("nickname").ok().map(ToString::to_string),
        remark: seed.get_str("remark").ok().map(ToString::to_string),
        alias: None,
        agent_status: crate::models::AgentStatus::Managed,
        human_profile_note: seed
            .get_str("humanProfileNote")
            .or_else(|_| seed.get_str("human_profile_note"))
            .ok()
            .map(ToString::to_string),
        agent_profile: None,
        memory_summary: seed
            .get_str("memorySummary")
            .or_else(|_| seed.get_str("memory_summary"))
            .ok()
            .map(ToString::to_string),
        playbook_id: None,
        playbook_version: None,
        tags: seed
            .get_array("tags")
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        customer_stage: seed
            .get_str("customerStage")
            .or_else(|_| seed.get_str("customer_stage"))
            .ok()
            .map(ToString::to_string),
        intent_level: seed
            .get_str("intentLevel")
            .or_else(|_| seed.get_str("intent_level"))
            .ok()
            .map(ToString::to_string),
        customer_stage_updated_at: None,
        commitments: Vec::new(),
        follow_up_policy: None,
        operation_state: seed
            .get_str("operationState")
            .or_else(|_| seed.get_str("operation_state"))
            .ok()
            .map(ToString::to_string)
            .or_else(|| Some("new_contact".to_string())),
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
}
