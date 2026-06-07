//! 运营知识库切片核验：单条 verify/reject + 批量 auto-verify + D2 状态裁决。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Bson, DateTime, Document},
    options::FindOptions,
};
use serde::Deserialize;
use serde_json::{json, Value};

use std::sync::Arc;

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::{agent, models::KnowledgeUsageLog, prompts};

use super::super::shared::*;
use super::super::AppState;
use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeVerifyRequest {
    verified_claims: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAutoVerifyRequest {
    account_id: Option<String>,
    /// 模型置信度阈值（0-10），≥ 该值才算 verified；默认 7。
    #[serde(default)]
    confidence_threshold: Option<i32>,
    /// 运营抽样概率，0.0-1.0；默认 0.1。
    #[serde(default)]
    human_audit_sample_rate: Option<f64>,
    /// 单次最多处理多少条 chunks，默认 50。
    #[serde(default)]
    limit: Option<i64>,
}

pub async fn verify_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<KnowledgeVerifyRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let chunk = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;

    // D2 不变量：verify 之前必须有 sourceQuote 且能锚定到父文档（source_anchors 非空）。
    // 否则任何路径（运营 verify / AI 修复后 apply-and-verify / 老 UI verify）都不可越过。
    let has_quote = chunk
        .source_quote
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_anchor = !chunk.source_anchors.is_empty();
    if let Some(reason) = chunk_verify_gate_reason(has_quote, has_anchor) {
        return Err(AppError::BadRequest(reason));
    }

    let verified_claims = payload.verified_claims.unwrap_or_default();
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            doc! {
                "$set": {
                    "integrity_status": "verified",
                    "confidence_score": 100,
                    "verified_claims": string_bson_array(&verified_claims),
                    "unsupported_claims": Bson::Array(Vec::new()),
                    "status": "active",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

pub(in crate::routes) async fn reject_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": object_id,
                "workspace_id": &admin.current_workspace
            },
            doc! {
                "$set": {
                    "integrity_status": "rejected",
                    "confidence_score": 0,
                    "status": "rejected",
                    "updated_at": DateTime::now()
                }
            },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// MP-9 / Task 16：批量调用 LLM 对 `needs_review` 的 chunks 自动校验。
///
/// - 串行处理，避免并发烧 token；
/// - confidence ≥ threshold 自动标 `verified`，否则保持 `needs_review`；
/// - 按 `1/N` 概率把判定结果改成 `needs_human_audit` 走 admin 抽查；
/// - 写一条 `agent_events kind="knowledge_auto_verify_done"`。
pub async fn auto_verify_operation_knowledge_chunks(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(payload): Json<KnowledgeAutoVerifyRequest>,
) -> AppResult<Json<Value>> {
    let account_id = payload
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let threshold = payload.confidence_threshold.unwrap_or(7).clamp(0, 10);
    let sample_rate = payload
        .human_audit_sample_rate
        .unwrap_or(0.1)
        .clamp(0.0, 1.0);
    let limit = payload.limit.unwrap_or(50).clamp(1, 500);

    let (token_budget, max_llm_calls) =
        auto_verify_budget_limits(&state, &admin.current_workspace).await?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        token_budget,
        max_llm_calls,
        // agent-autonomy-loop W3 / Task 4.1：auto_verify 路径不进入 tool-loop，
        // 用 i32::MAX 表示"不限 tool call 次数"，等价于关闭 R4.3 的 tool 维度
        // 硬上限；该字段仍参与 record_tool_call 累加，仅不会先于其它维度饱和。
        i32::MAX,
    ));
    let workspace_id = admin.current_workspace.clone();
    agent::RUN_BUDGET
        .scope(
            budget.clone(),
            auto_verify_operation_knowledge_chunks_inner(
                state,
                workspace_id,
                account_id,
                threshold,
                sample_rate,
                limit,
                run_id,
                budget,
            ),
        )
        .await
}

async fn auto_verify_budget_limits(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<(i64, i32)> {
    let config = state
        .db
        .operation_domain_configs()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "domain": "user_operations"
            },
            None,
        )
        .await?;
    let params = config.as_ref().map(|item| &item.runtime_parameters);
    // R15 / ISSUE-009：auto-verify 是批处理（一次跑 N 条 chunk），不能复用 user-ops
    // 单 run 内的 `runMaxLlmCalls`（默认 6，含义=单次会话 tool-call 预算）；
    // 否则 limit=50 会被默默缩到 6，degraded 直接触发 budget_exceeded。
    // 专属 key `autoVerifyMaxLlmCalls`，默认 100；token 预算同样独立。
    Ok((
        doc_i64_with_default(params, "autoVerifyTokenBudget", 240000),
        doc_i32_with_default(params, "autoVerifyMaxLlmCalls", 100).max(1),
    ))
}

fn doc_i64_with_default(doc: Option<&Document>, key: &str, default: i64) -> i64 {
    doc.and_then(|item| {
        item.get_i64(key)
            .ok()
            .or_else(|| item.get_i32(key).ok().map(i64::from))
    })
    .unwrap_or(default)
}

fn doc_i32_with_default(doc: Option<&Document>, key: &str, default: i32) -> i32 {
    doc.and_then(|item| {
        item.get_i32(key).ok().or_else(|| {
            item.get_i64(key)
                .ok()
                .and_then(|value| i32::try_from(value).ok())
        })
    })
    .unwrap_or(default)
}

async fn auto_verify_operation_knowledge_chunks_inner(
    state: AppState,
    workspace_id: String,
    account_id: String,
    threshold: i32,
    sample_rate: f64,
    limit: i64,
    run_id: String,
    budget: Arc<agent::RunBudget>,
) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &workspace_id,
                "domain": "user_operations",
                "integrity_status": { "$in": ["needs_review", null] },
                "$or": [
                    { "account_id": null },
                    { "account_id": &account_id }
                ]
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;

    let system = prompts::load_prompt(
        &state.db,
        &workspace_id,
        "knowledge.auto_verify",
    )
    .await
    .unwrap_or_else(|_| {
        "你是 WechatAgent 知识库自动校验 Agent。只输出严格 JSON。只有 sourceQuote 非空且 sourceAnchors 可定位来源时，才允许 verified。".to_string()
    });

    let mut verified = 0i32;
    let mut needs_review = 0i32;
    let mut rejected = 0i32;
    let mut needs_human_audit = 0i32;
    let mut processed = 0i32;
    let mut degraded = false;

    while let Some(chunk) = cursor.try_next().await? {
        let Some(chunk_id) = chunk.id else { continue };
        if budget.is_exceeded() {
            budget.mark_degraded("knowledge_auto_verify_stopped_budget_exceeded");
            degraded = true;
            break;
        }
        let has_source_quote = chunk
            .source_quote
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        let has_source_anchor = !chunk.source_anchors.is_empty();
        let user = format!(
            r#"请对下面这条知识切片做自动校验。
切片 ID: {}
标题: {}
摘要: {}
正文: {}
source_quote: {}
source_anchors: {}

输出 JSON：
{{
  "confidenceScore": 0,
  "integrityStatus": "verified",
  "verifiedClaims": [],
  "distortionRisks": []
}}"#,
            chunk_id.to_hex(),
            chunk.title,
            chunk.summary.clone().unwrap_or_default(),
            chunk.body.clone().unwrap_or_default(),
            chunk.source_quote.clone().unwrap_or_default(),
            serde_json::to_string(&chunk.source_anchors).unwrap_or_default(),
        );

        let value = match agent::generate_agent_json(
            &state,
            Some(&account_id),
            None,
            Some(&run_id),
            "knowledge.auto_verify",
            &system,
            &user,
        )
        .await
        {
            Ok(v) => v,
            Err(_) => {
                // 单条失败不阻断整体；保留原状态，进入下一条。
                continue;
            }
        };
        processed += 1;

        let confidence = value
            .get("confidenceScore")
            .or_else(|| value.get("confidence_score"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let model_status = value
            .get("integrityStatus")
            .or_else(|| value.get("integrity_status"))
            .and_then(|v| v.as_str())
            .unwrap_or("needs_review")
            .to_string();
        let verified_claims_json = value
            .get("verifiedClaims")
            .or_else(|| value.get("verified_claims"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let distortion_risks_json = value
            .get("distortionRisks")
            .or_else(|| value.get("distortion_risks"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // 决定最终 status：必须有原文引用和锚点，threshold + 抽样改 needs_human_audit。
        let mut final_status =
            decide_auto_verify_status(has_source_quote, has_source_anchor, confidence, threshold, &model_status);
        if final_status == "verified" && sample_rate > 0.0 && fastrand::f64() < sample_rate {
            final_status = "needs_human_audit".to_string();
        }

        match final_status.as_str() {
            "verified" => verified += 1,
            "rejected" => rejected += 1,
            "needs_human_audit" => needs_human_audit += 1,
            _ => needs_review += 1,
        }

        let _ = state
            .db
            .operation_knowledge_chunks()
            .update_one(
                doc! { "_id": chunk_id },
                doc! {
                    "$set": {
                        "integrity_status": &final_status,
                        "confidence_score": confidence,
                        "verified_claims": string_bson_array(&verified_claims_json),
                        "distortion_risks": string_bson_array(&distortion_risks_json),
                        "updated_at": DateTime::now()
                    }
                },
                None,
            )
            .await;
        let _ = state
            .db
            .knowledge_usage_logs()
            .insert_one(
                KnowledgeUsageLog {
                    id: None,
                    workspace_id: workspace_id.clone(),
                    account_id: account_id.clone(),
                    contact_wxid: None,
                    run_id: run_id.clone(),
                    knowledge_ids: vec![chunk_id],
                    route_result: doc! {
                        "kind": "knowledge_auto_verify",
                        "promptKey": "knowledge.auto_verify",
                        "chunkId": chunk_id.to_hex(),
                        "confidenceScore": confidence,
                        "modelStatus": model_status,
                        "finalStatus": &final_status,
                        "hasSourceQuote": has_source_quote,
                        "hasSourceAnchor": has_source_anchor,
                    },
                    reply_text: None,
                    review_approved: final_status == "verified",
                    blocked_reason: if final_status == "verified" {
                        None
                    } else {
                        Some("knowledge_auto_verify_not_verified".to_string())
                    },
                    tool_trace: vec![doc! {
                        "sourceAnchorCount": chunk.source_anchors.len() as i32,
                        "sourceQuotePresent": has_source_quote,
                    }],
                    created_at: DateTime::now(),
                },
                None,
            )
            .await;
    }

    let _ = state
        .db
        .events()
        .insert_one(
            crate::models::AgentEvent {
                id: None,
                workspace_id: workspace_id.clone(),
                account_id: account_id.clone(),
                contact_wxid: None,
                kind: "knowledge_auto_verify_done".to_string(),
                status: "success".to_string(),
                summary: format!(
                    "自动校验完成：verified={verified} needs_review={needs_review} rejected={rejected} needs_human_audit={needs_human_audit}"
                ),
                details: Some(doc! {
                    "processed": processed,
                    "verified": verified,
                    "needsReview": needs_review,
                    "rejected": rejected,
                    "needsHumanAudit": needs_human_audit,
                    "confidenceThreshold": threshold,
                    "humanAuditSampleRate": sample_rate,
                    "degraded": degraded,
                    "budget": budget_document(&budget)
                }),
                created_at: DateTime::now(),
                dedupe_key: None,
            },
            None,
        )
        .await;

    Ok(Json(json!({
        "processed": processed,
        "verified": verified,
        "needsReview": needs_review,
        "rejected": rejected,
        "needsHumanAudit": needs_human_audit,
        "degraded": degraded,
        "budget": budget_document(&budget)
    })))
}

/// 波 D2：knowledge auto-verify 的"最终状态"判定（先于 admin 后台抽样）。
///
/// 性质：
/// - `verified` ⇔ source_quote 非空 ∧ source_anchors 可定位 ∧ LLM 输出
///   `integrityStatus="verified"` ∧ confidence ≥ threshold；
/// - `rejected` ⇔ LLM 明确给出 `rejected` 且不满足 verified 全部条件；
/// - 其它一律 `needs_review`，**包括** 4 项之一缺失但 LLM 自称 verified。
///
/// 这是 spec「auto-verify 证据强约束」的关键判定，单测覆盖防止后续误改。
pub fn decide_auto_verify_status(
    has_source_quote: bool,
    has_source_anchor: bool,
    confidence: i32,
    threshold: i32,
    model_status: &str,
) -> String {
    if has_source_quote
        && has_source_anchor
        && confidence >= threshold
        && model_status == "verified"
    {
        return "verified".to_string();
    }
    if model_status == "rejected" {
        return "rejected".to_string();
    }
    "needs_review".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 波 D2：4 项证据齐 → verified。
    #[test]
    fn verified_when_all_evidence_present_and_confident() {
        let s = decide_auto_verify_status(true, true, 8, 7, "verified");
        assert_eq!(s, "verified");
    }

    /// 波 D2：缺 source_quote（即使其它都齐）→ needs_review。
    #[test]
    fn needs_review_when_source_quote_missing() {
        let s = decide_auto_verify_status(false, true, 8, 7, "verified");
        assert_eq!(s, "needs_review", "缺 source_quote 必须降级");
    }

    /// 波 D2：缺 source_anchor → needs_review。
    #[test]
    fn needs_review_when_source_anchor_missing() {
        let s = decide_auto_verify_status(true, false, 9, 7, "verified");
        assert_eq!(s, "needs_review", "缺 source_anchor 必须降级");
    }

    /// 波 D2：confidence 低于 threshold → needs_review，即便 LLM 自称 verified。
    #[test]
    fn needs_review_when_confidence_below_threshold() {
        let s = decide_auto_verify_status(true, true, 5, 7, "verified");
        assert_eq!(s, "needs_review");
    }

    /// 波 D2：LLM 给 rejected 直接采纳。
    #[test]
    fn passes_through_rejected_status() {
        let s = decide_auto_verify_status(true, true, 9, 7, "rejected");
        assert_eq!(s, "rejected");
    }

    /// 波 D2：未知 model_status 默认 needs_review，不会偷渡为 verified。
    #[test]
    fn unknown_model_status_falls_back_to_needs_review() {
        let s = decide_auto_verify_status(true, true, 9, 7, "");
        assert_eq!(s, "needs_review");
        let s = decide_auto_verify_status(true, true, 9, 7, "uncertain");
        assert_eq!(s, "needs_review");
    }

    /// R15 / ISSUE-009：auto-verify 默认 budget 不能复用 user-ops 单 run 的
    /// `runMaxLlmCalls=6`，否则 limit=50 调用一次只能跑 6 条 chunk。
    /// 这里只断默认值，避免回归到 6。
    #[test]
    fn auto_verify_default_call_cap_is_not_run_max_llm_calls_six() {
        // 直接测 doc_i32_with_default 在没有 config 时的默认行为：返回 100，不是 6。
        let v = doc_i32_with_default(None, "autoVerifyMaxLlmCalls", 100);
        assert!(v >= 50, "autoVerify call cap 默认 {v} 必须 ≥ 50（与 limit=50 对齐）");
        assert_ne!(v, 6, "禁止回归到 runMaxLlmCalls=6");
    }

    #[test]
    fn auto_verify_default_token_budget_is_not_simulation_60000() {
        // 同理 token budget 默认值不能再复用 simulationTokenBudget=60000。
        let v = doc_i64_with_default(None, "autoVerifyTokenBudget", 240000);
        assert!(v >= 100_000, "autoVerify token budget 默认 {v} 太小，无法跑 50 条");
    }
}
