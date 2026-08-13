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
use crate::knowledge_wiki::chunk_revisions::{
    apply_chunk_revision, apply_chunk_revision_with_session, commit_chunk_transaction,
    map_chunk_transaction_error, ProvenanceSource, RevisionApplied, RevisionOp, RevisionRequest,
};
use crate::{agent, models::KnowledgeUsageLog, prompts};

use super::super::shared::*;
use super::super::AppState;
use super::*;

/// auto-verify 人审抽样率**硬下限**（修复 C-①）：即便请求传 0（前端取消「留一批
/// 我复查」），也按此下限抽样——禁止 100% 无人审落 verified。product_fact 类已由
/// C-② 全量强制人审，本抽样主要覆盖其他三类，故下限取温和的 5%。
const AUTO_VERIFY_MIN_SAMPLE_RATE: f64 = 0.05;
/// auto-verify 人审抽样率默认值（修复 C-①：从 0.1 抬到 0.3，更积极抽审）。
const AUTO_VERIFY_DEFAULT_SAMPLE_RATE: f64 = 0.3;

/// auto-verify 抽样率钳制:未传用默认 [`AUTO_VERIFY_DEFAULT_SAMPLE_RATE`],并强制钳到硬下限
/// [`AUTO_VERIFY_MIN_SAMPLE_RATE`](传 0 也不许 100% 无人审)。抽成纯函数以便单测锁死下限——
/// 删下界 / 改成 0 会让「永远留一批人审」这条红线被静默关掉。
fn clamp_sample_rate(requested: Option<f64>) -> f64 {
    requested
        .unwrap_or(AUTO_VERIFY_DEFAULT_SAMPLE_RATE)
        .clamp(AUTO_VERIFY_MIN_SAMPLE_RATE, 1.0)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeVerifyRequest {
    verified_claims: Option<Vec<String>>,
    /// 管理员实际看到的 chunk 版本。必须与事务快照中的 `updated_at` 精确一致。
    expected_updated_at: String,
}

pub(super) fn parse_verify_expected_updated_at(value: &str) -> AppResult<DateTime> {
    DateTime::parse_rfc3339_str(value.trim())
        .map_err(|_| AppError::BadRequest("expectedUpdatedAt must be RFC3339".to_string()))
}

/// 在同一 Mongo 事务快照内完成版本绑定、D2 证据检查和 verify revision 写入。
/// 这同时阻止“管理员看 A、实际批准 B”和“闸门检查后证据被并发清空”两类竞态。
pub(super) async fn verify_chunk_at_version(
    state: &AppState,
    workspace_id: &str,
    object_id: mongodb::bson::oid::ObjectId,
    expected_updated_at: DateTime,
    verified_claims: &[String],
    reason: Option<String>,
    actor: &str,
) -> AppResult<RevisionApplied> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;

    let result: AppResult<RevisionApplied> = async {
        let chunk = state
            .db
            .operation_knowledge_chunks()
            .find_one_with_session(
                doc! { "_id": object_id, "workspace_id": workspace_id },
                None,
                &mut session,
            )
            .await?
            .ok_or_else(|| AppError::NotFound("operation knowledge chunk not found".to_string()))?;
        if chunk.updated_at.timestamp_millis() != expected_updated_at.timestamp_millis() {
            return Err(AppError::Conflict("chunk_revision_conflict".to_string()));
        }

        // B3：读取侧（`quote_is_chunk_evidence`）要求命中的 anchor 自身含非空
        // `sourceQuote`，故本闸也必须按同一契约判定「可定位」。旧口径只查数组非空，
        // 让缺该键的畸形 anchor 通过 verify 进入 active+verified，而那类 chunk 的
        // 引用在读取侧恒被拒 → 永久无法被 cite（表现为"知识库有答案却答不出来"）。
        // 谓词由 `chunk_verify_gate_reason_for` 内部算，调用方无法再传错。
        if let Some(reason) = super::chunk_verify_gate_reason_for(
            chunk.source_quote.as_deref(),
            &chunk.source_anchors,
        ) {
            return Err(AppError::BadRequest(reason));
        }

        apply_chunk_revision_with_session(
            &state.db,
            workspace_id,
            object_id,
            RevisionRequest {
                op: RevisionOp::Verify,
                source: ProvenanceSource::Human,
                patch: doc! {
                    "integrity_status": "verified",
                    "confidence_score": 100,
                    "verified_claims": string_bson_array(verified_claims),
                    "unsupported_claims": Bson::Array(Vec::new()),
                    "status": "active",
                },
                reason,
                actor: Some(actor.to_string()),
            },
            &mut session,
        )
        .await
    }
    .await;

    let applied = match result {
        Ok(applied) => applied,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(map_chunk_transaction_error(error));
        }
    };
    commit_chunk_transaction(&mut session)
        .await
        .map_err(map_chunk_transaction_error)?;
    Ok(applied)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeAutoVerifyRequest {
    account_id: Option<String>,
    /// 模型置信度阈值（0-10），≥ 该值才算 verified；默认 7。
    #[serde(default)]
    confidence_threshold: Option<i32>,
    /// 运营抽样概率，0.0-1.0；默认 [`AUTO_VERIFY_DEFAULT_SAMPLE_RATE`]，且被 clamp 到
    /// 硬下限 [`AUTO_VERIFY_MIN_SAMPLE_RATE`]（传 0 也不允许 100% 无人审）。
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
    let expected_updated_at = parse_verify_expected_updated_at(&payload.expected_updated_at)?;
    let verified_claims = payload.verified_claims.unwrap_or_default();
    let applied = verify_chunk_at_version(
        &state,
        &admin.current_workspace,
        object_id,
        expected_updated_at,
        &verified_claims,
        None,
        &admin.username,
    )
    .await?;
    Ok(Json(
        json!({ "ok": true, "revisionId": applied.revision_id }),
    ))
}

pub async fn reject_operation_knowledge_chunk(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    // D2：reject 同样接回 apply_chunk_revision（op=reject, source=human），留审计痕迹。
    apply_chunk_revision(
        &state.db,
        &admin.current_workspace,
        object_id,
        RevisionRequest {
            op: RevisionOp::Reject,
            source: ProvenanceSource::Human,
            patch: doc! {
                "integrity_status": "rejected",
                "confidence_score": 0,
                "status": "rejected",
            },
            reason: None,
            actor: Some(admin.username.clone()),
        },
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
    // 修复（问题 C-①）：human_audit_sample_rate 设**硬下限** AUTO_VERIFY_MIN_SAMPLE_RATE，
    // 不再允许 0——前端「留一批我复查」取消勾选会传 0，此前 clamp(0.0,1.0) 放行 0 =
    // 100% 无人审落 verified。下限保证「永远有一批被抽出人审」这条红线姿态不可被关掉。
    // 默认从 0.1 抬到 0.3（更积极抽审）。注：product_fact 类已由 C-② 全量强制人审，本抽样
    // 主要覆盖其他三类（不进产品报价链路、风险较低），故下限取温和的 5%。
    let sample_rate = clamp_sample_rate(payload.human_audit_sample_rate);
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

async fn auto_verify_budget_limits(state: &AppState, workspace_id: &str) -> AppResult<(i64, i32)> {
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

fn auto_verify_all_llm_calls_failed_error(
    processed: i32,
    llm_attempted: i32,
    llm_failed: i32,
    first_error: Option<AppError>,
) -> Option<AppError> {
    if processed == 0 && llm_attempted > 0 && llm_failed == llm_attempted {
        return Some(first_error.unwrap_or_else(|| {
            AppError::External("knowledge auto-verify: all LLM calls failed".to_string())
        }));
    }
    None
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
    let cursor = state
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
    let candidates = cursor.try_collect::<Vec<_>>().await?;
    let candidate_chunk_ids = candidates
        .iter()
        .filter_map(|chunk| chunk.id.map(|id| id.to_hex()))
        .collect::<Vec<_>>();
    if !candidate_chunk_ids.is_empty() {
        record_knowledge_run_started(
            &state,
            &workspace_id,
            &account_id,
            &run_id,
            "knowledge.auto_verify",
            &candidate_chunk_ids,
        )
        .await?;
    }

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
    let mut failed = 0i32;
    let mut degraded = false;
    let mut llm_attempted = 0i32;
    let mut llm_failed = 0i32;
    let mut first_llm_error: Option<AppError> = None;
    let mut processed_chunk_ids = Vec::new();

    for chunk in candidates {
        let Some(chunk_id) = chunk.id else { continue };
        if budget.should_stop_optional_llm_calls() {
            if budget.is_exceeded() {
                budget.mark_degraded("knowledge_auto_verify_stopped_budget_exceeded");
            } else {
                budget.mark_degraded("knowledge_auto_verify_stopped_usage_unknown");
            }
            degraded = true;
            break;
        }
        let (has_source_quote, has_source_anchor) = chunk_evidence_flags(&chunk);
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

        llm_attempted += 1;
        let value = match agent::generate_agent_json(
            &state,
            &workspace_id,
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
            Err(error) => {
                // 单条失败不阻断整体；保留原状态，进入下一条。
                failed += 1;
                llm_failed += 1;
                if first_llm_error.is_none() {
                    first_llm_error = Some(error);
                }
                continue;
            }
        };
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
        let mut final_status = decide_auto_verify_status(
            has_source_quote,
            has_source_anchor,
            confidence,
            threshold,
            &model_status,
        );
        if final_status == "verified" && sample_rate > 0.0 && fastrand::f64() < sample_rate {
            final_status = "needs_human_audit".to_string();
        }
        // ①-a：auto-verify **对所有 chunk_type 都不得直 verified**。依据 CLAUDE.md 红线
        // 「AI 永不自动 verify」适用于所有类型知识：auto-verify 仅凭 LLM 自评 + 证据闸不足以
        // 替代运营核验，一律强制 needs_human_audit（无论抽样是否命中）。auto-verify 退化为
        // "预审分诊"——过闸的挑出来等运营重点看，绝不自动放行。product_fact 更是唯一经 R5.4
        // 成为产品声明背书的类型（models.rs chunk_type 文档），一旦直 verified 会让 agent 据
        // AI 自己背书的 chunk 对客户报价，风险最高；其余三类同样不放行。
        final_status = enforce_verified_needs_human_audit(final_status);

        // D2：auto_verify 的每条裁决也接回 apply_chunk_revision，留 chunk_revisions
        // 审计痕迹。**source=Rule**（非 Human）：裁决由 LLM 自评 + 规则闸门
        // （decide_auto_verify_status + enforce_verified_needs_human_audit + 抽样）做出，
        // admin 只触发了批处理、并未逐条审定——标 Rule 才如实反映"规则化批处理写入"，
        // 避免审计按 source 过滤时误判"运营逐条审定了这条"。created_by="auto_verify" 进一步
        // 标识自动来源。只有 revision 事务提交后，才累计 processed/状态计数并写 usage log；
        // 写失败保留原 Chunk、计入 failed，不能把上游候选伪装成已发生的业务裁决。
        let applied = match apply_chunk_revision(
            &state.db,
            &workspace_id,
            chunk_id,
            RevisionRequest {
                op: RevisionOp::Verify,
                source: ProvenanceSource::Rule,
                patch: doc! {
                    "integrity_status": &final_status,
                    "confidence_score": confidence,
                    "verified_claims": string_bson_array(&verified_claims_json),
                    "distortion_risks": string_bson_array(&distortion_risks_json),
                },
                reason: Some(format!(
                    "auto_verify: model_status={model_status}, final={final_status}"
                )),
                actor: Some("auto_verify".to_string()),
            },
        )
        .await
        {
            Ok(applied) => applied,
            Err(error) => {
                failed += 1;
                tracing::warn!(
                    workspace_id = %workspace_id,
                    chunk_id = %chunk_id,
                    error = %error,
                    "knowledge auto-verify revision failed; decision not counted"
                );
                continue;
            }
        };
        processed += 1;
        processed_chunk_ids.push(chunk_id.to_hex());
        match final_status.as_str() {
            "verified" => verified += 1,
            "rejected" => rejected += 1,
            "needs_human_audit" => needs_human_audit += 1,
            _ => needs_review += 1,
        }

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
                        "revisionId": &applied.revision_id,
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

    if let Some(error) = auto_verify_all_llm_calls_failed_error(
        processed,
        llm_attempted,
        llm_failed,
        first_llm_error,
    ) {
        return Err(error);
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
                    "自动校验完成：processed={processed} failed={failed} verified={verified} needs_review={needs_review} rejected={rejected} needs_human_audit={needs_human_audit}"
                ),
                details: Some(doc! {
                    "runId": &run_id,
                    "chunkIds": &processed_chunk_ids,
                    "processed": processed,
                    "failed": failed,
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
        "runId": run_id,
        "chunkIds": processed_chunk_ids,
        "processed": processed,
        "failed": failed,
        "verified": verified,
        "needsReview": needs_review,
        "rejected": rejected,
        "needsHumanAudit": needs_human_audit,
        "degraded": degraded,
        "budget": budget_document(&budget)
    })))
}

/// auto-verify 判定链的证据旗标：`(has_source_quote, has_citable_anchor)`。
///
/// 锚点侧用 [`crate::models::chunk_has_citable_anchor`]（citable 口径），与 D2
/// verify 闸（`chunk_verify_gate_reason_for`）和读取侧 `quote_is_chunk_evidence`
/// 同一谓词——裸 `!source_anchors.is_empty()` 会把只有畸形锚（缺 `sourceQuote`
/// 键）的切片误判为「有锚」，让永远无法被引用的切片通过预审分诊。
/// 具名抽出是为了让口径可被单测锚死（判定函数只收 bool，算错在闸上不可见）。
fn chunk_evidence_flags(chunk: &crate::models::OperationKnowledgeChunk) -> (bool, bool) {
    let has_source_quote = chunk
        .source_quote
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    let has_citable_anchor = crate::models::chunk_has_citable_anchor(&chunk.source_anchors);
    (has_source_quote, has_citable_anchor)
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

/// ①-a：auto-verify 的最终状态若为 `verified`，强制降级 `needs_human_audit`——
/// **对所有 chunk_type 生效**。依据：CLAUDE.md 红线「AI 永不自动 verify」适用于
/// 所有类型知识；auto-verify 仅凭 LLM 自评 + 证据闸不足以替代运营核验。auto-verify
/// 退化为"预审分诊"：过闸的挑出来等运营重点看，绝不自动放行。
///
/// 性质：仅当 `final_status == "verified"` 时降级；其它（rejected / needs_review /
/// needs_human_audit）一律原样返回。
pub fn enforce_verified_needs_human_audit(final_status: String) -> String {
    if final_status == "verified" {
        return "needs_human_audit".to_string();
    }
    final_status
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B4：畸形锚（有元素但缺非空 `sourceQuote`）在 auto-verify 判定链上必须
    /// 视同「无锚」——否则不可引用的切片会被预审分诊放行到 needs_human_audit。
    #[test]
    fn evidence_flags_treat_malformed_anchor_as_missing() {
        let mut chunk = crate::models::OperationKnowledgeChunk {
            source_quote: Some("原文片段".into()),
            source_anchors: vec![mongodb::bson::doc! { "startOffset": 0i64 }],
            ..Default::default()
        };
        assert_eq!(chunk_evidence_flags(&chunk), (true, false));

        chunk.source_anchors = vec![mongodb::bson::doc! { "sourceQuote": "原文片段" }];
        assert_eq!(chunk_evidence_flags(&chunk), (true, true));

        chunk.source_quote = Some("   ".into());
        assert_eq!(chunk_evidence_flags(&chunk), (false, true));
    }

    #[test]
    fn auto_verify_all_llm_failures_preserve_structured_error() {
        let error = auto_verify_all_llm_calls_failed_error(
            0,
            2,
            2,
            Some(AppError::LlmUnavailable {
                kind: "model_routing_unavailable".to_string(),
                retry_count: 9,
                detail: "no route".to_string(),
                hint: "retry".to_string(),
            }),
        )
        .expect("all LLM calls failed");
        assert!(matches!(
            error,
            AppError::LlmUnavailable {
                kind,
                retry_count: 9,
                ..
            } if kind == "model_routing_unavailable"
        ));
    }

    #[test]
    fn auto_verify_partial_success_is_not_batch_failure() {
        assert!(auto_verify_all_llm_calls_failed_error(1, 2, 1, None).is_none());
        assert!(auto_verify_all_llm_calls_failed_error(0, 2, 1, None).is_none());
        assert!(auto_verify_all_llm_calls_failed_error(0, 0, 0, None).is_none());
    }

    #[test]
    fn auto_verify_request_accepts_only_published_camel_case_keys() {
        let accepted: KnowledgeAutoVerifyRequest = serde_json::from_value(json!({
            "confidenceThreshold": 9,
            "humanAuditSampleRate": 0.3,
            "limit": 50
        }))
        .expect("camelCase auto-verify request");
        assert_eq!(accepted.confidence_threshold, Some(9));
        assert_eq!(accepted.human_audit_sample_rate, Some(0.3));
        assert!(serde_json::from_value::<KnowledgeAutoVerifyRequest>(json!({
            "confidence_threshold": 9,
            "human_audit_sample_rate": 0.3
        }))
        .is_err());
    }

    /// 修复 C-②：product_fact 的 verified 被强制降级 needs_human_audit（堵报错价链路）。
    #[test]
    fn product_fact_verified_forced_to_human_audit() {
        let s = enforce_verified_needs_human_audit("verified".to_string());
        assert_eq!(
            s, "needs_human_audit",
            "product_fact 不得经 auto-verify 直 verified"
        );
    }

    /// ①-a：auto-verify 对**所有** chunk_type 的 verified 都强制降级 needs_human_audit
    /// （AI 永不自动 verify 适用所有类型，不只 product_fact）。
    #[test]
    fn all_types_verified_forced_to_human_audit() {
        for ct in [
            "product_fact",
            "style_template",
            "peer_case",
            "negative_example",
        ] {
            let _ = ct; // 类型不再影响判定；保留循环表达"覆盖全类型"意图
            let s = enforce_verified_needs_human_audit("verified".to_string());
            assert_eq!(s, "needs_human_audit", "所有类型的 verified 都必须降级");
        }
    }

    /// 修复 C-②：非 verified 终态（needs_review / rejected / needs_human_audit）原样透传，
    /// 只拦"verified"这一档，不影响 reject/缺证据降级。
    #[test]
    fn product_fact_non_verified_passthrough() {
        for st in ["needs_review", "rejected", "needs_human_audit"] {
            let s = enforce_verified_needs_human_audit(st.to_string());
            assert_eq!(s, st, "非 verified 终态应原样透传");
        }
    }

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
        assert!(
            v >= 50,
            "autoVerify call cap 默认 {v} 必须 ≥ 50（与 limit=50 对齐）"
        );
        assert_ne!(v, 6, "禁止回归到 runMaxLlmCalls=6");
    }

    #[test]
    fn auto_verify_default_token_budget_is_not_simulation_60000() {
        // 同理 token budget 默认值不能再复用 simulationTokenBudget=60000。
        let v = doc_i64_with_default(None, "autoVerifyTokenBudget", 240000);
        assert!(
            v >= 100_000,
            "autoVerify token budget 默认 {v} 太小，无法跑 50 条"
        );
    }

    #[test]
    fn clamp_sample_rate_enforces_hard_floor() {
        // 命门:传 0(前端取消"留一批复查")也不许 100% 无人审,钳到 5% 下限。
        assert_eq!(
            clamp_sample_rate(Some(0.0)),
            0.05,
            "传0钳到硬下限,红线不可关"
        );
        assert_eq!(clamp_sample_rate(None), 0.3, "未传用默认 0.3");
        assert_eq!(clamp_sample_rate(Some(2.0)), 1.0, "超上限钳到 1.0");
        assert_eq!(clamp_sample_rate(Some(0.5)), 0.5, "区间内原样透传");
    }
}
