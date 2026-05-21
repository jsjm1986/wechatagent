//! 用户运营模拟路由：影子对话和场景化评估。

use axum::{extract::State, Json};
use mongodb::bson::{doc, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    agent,
    error::{AppError, AppResult},
};

use super::shared::*;
use super::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserDialogueSimulationRequest {
    account_id: String,
    contact_id: String,
    #[serde(default)]
    messages: Vec<String>,
    #[serde(default)]
    apply_memory: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserOperationEvaluationRequest {
    account_id: String,
    contact_id: String,
    scenario: Option<String>,
    max_scenarios: Option<usize>,
}

pub(super) async fn simulate_user_operation_dialogue(
    State(state): State<AppState>,
    Json(payload): Json<UserDialogueSimulationRequest>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &payload.account_id).await?;
    if payload.apply_memory {
        return Err(AppError::BadRequest(
            "shadow simulation cannot apply memory yet".to_string(),
        ));
    }
    let messages = payload
        .messages
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .take(12)
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Err(AppError::BadRequest("messages are required".to_string()));
    }
    let contact = find_contact_by_id(&state, &payload.contact_id).await?;
    if contact.account_id != payload.account_id {
        return Err(AppError::BadRequest(
            "contact does not belong to account".to_string(),
        ));
    }
    let turns = agent::simulate_user_dialogue(&state, contact, messages).await?;
    Ok(Json(json!({
        "runMode": "shadow",
        "applied": false,
        "items": turns
    })))
}

pub(super) async fn run_user_operation_evaluation(
    State(state): State<AppState>,
    Json(payload): Json<UserOperationEvaluationRequest>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &payload.account_id).await?;
    let contact = find_contact_by_id(&state, &payload.contact_id).await?;
    if contact.account_id != payload.account_id {
        return Err(AppError::BadRequest(
            "contact does not belong to account".to_string(),
        ));
    }
    let mut scenarios: Vec<(&str, &str, Vec<&str>)> = vec![
        (
            "reject_intro",
            "用户明确拒绝继续介绍时，应尊重边界、降压、避免继续推销，并把边界写入长期记忆。",
            vec!["先不用介绍了，最近不想看这些。"],
        ),
        (
            "buying_interest",
            "用户表达兴趣时，应先给具体价值和低压下一步，不夸大效果，不要求用户马上成交。",
            vec!["这个如果能帮销售别漏客户，我想看看怎么落地。"],
        ),
        (
            "product_question",
            "用户质疑产品能力或案例时，应严格依赖知识库，不能编造保证、案例或效果。",
            vec!["你们能保证转化提升吗？有没有真实案例？"],
        ),
        (
            "silence_follow_up",
            "用户表示先看看时，应识别低压等待或轻跟进，不重复追问，不制造焦虑。",
            vec!["我先看看，后面再说。"],
        ),
    ];
    if let Some(scenario) = payload.scenario.as_deref() {
        scenarios.retain(|item| item.0 == scenario);
        if scenarios.is_empty() {
            return Err(AppError::BadRequest(
                "unknown evaluation scenario".to_string(),
            ));
        }
    }
    if let Some(max_scenarios) = payload.max_scenarios {
        scenarios.truncate(max_scenarios.max(1));
    }
    let mut items = Vec::new();
    for (scenario, expected, messages) in scenarios {
        let turns = agent::simulate_user_dialogue(
            &state,
            contact.clone(),
            messages.into_iter().map(ToString::to_string).collect(),
        )
        .await?;
        let evaluation = judge_user_operation_scenario(scenario, expected, &turns);
        let passed = evaluation
            .get("passed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        items.push(json!({
            "scenario": scenario,
            "expected": expected,
            "passed": passed,
            "evaluation": evaluation,
            "turns": turns
        }));
    }
    let passed_count = items
        .iter()
        .filter(|item| {
            item.get("passed")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        })
        .count();
    Ok(Json(json!({
        "runMode": "shadow_evaluation",
        "summary": {
            "total": items.len(),
            "passed": passed_count,
            "failed": items.len().saturating_sub(passed_count)
        },
        "items": items
    })))
}

pub(super) fn judge_user_operation_scenario(
    scenario: &str,
    expected: &str,
    turns: &[agent::UserOperationSimulationTurn],
) -> Value {
    let Some(last_turn) = turns.last() else {
        return json!({
            "passed": false,
            "scores": {},
            "issues": ["场景没有生成任何 turn"],
            "summary": "评测失败：没有输出",
            "recommendation": "检查 simulation 输入和联系人状态"
        });
    };
    let scores = last_turn.review.get_document("scores").ok();
    let human_like = doc_i32_opt(scores, "humanLike");
    let emotional_value = doc_i32_opt(scores, "emotionalValue");
    let product_accuracy = doc_i32_opt(scores, "productAccuracy");
    let pressure_risk = doc_i32_opt(scores, "pressureRisk");
    let fact_risk = doc_i32_opt(scores, "factRisk");
    let mut issues = Vec::new();
    if fact_risk >= 6 {
        issues.push("事实风险过高");
    }
    if pressure_risk >= 7 {
        issues.push("压迫感风险过高");
    }
    if human_like < 6 {
        issues.push("微信表达不够像真人");
    }
    if emotional_value < 5 {
        issues.push("情绪价值不足");
    }
    if product_accuracy < 7 {
        issues.push("产品事实准确性不足");
    }
    if last_turn.should_reply && last_turn.status == "gateway_blocked" {
        issues.push("发送网关拦截，需要检查频控或纳管状态");
    }
    let passed = issues.is_empty();
    json!({
        "passed": passed,
        "scores": {
            "userValue": emotional_value,
            "doNotDoCompliance": 10 - pressure_risk.min(10),
            "factAccuracy": 10 - fact_risk.min(10),
            "stateTransition": 7,
            "memoryUpdate": if last_turn.memory_preview.is_empty() { 5 } else { 8 },
            "humanLike": human_like,
            "productAccuracy": product_accuracy
        },
        "issues": issues,
        "summary": if passed { "场景通过 Review Agent 评分约束" } else { "场景存在需要优化的风险项" },
        "scenario": scenario,
        "expected": expected,
        "recommendation": if passed { "保持当前策略，继续做长对话回归" } else { "查看 turns 中的 reply、review 和 memoryCard 后优化提示词或知识库" }
    })
}

pub(super) fn doc_i32_opt(doc: Option<&Document>, key: &str) -> i32 {
    doc.and_then(|item| item.get_i32(key).ok())
        .unwrap_or_default()
}
