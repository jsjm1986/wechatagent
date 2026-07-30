//! 运营知识库对话补库：chat turn/apply/history + 意图分诊 + 草拟/更新/应用 + 后台任务流。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, oid::ObjectId, Bson, DateTime, Document},
    ClientSession,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent;
use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};

use super::super::shared::*;
use super::super::AppState;
use super::*;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatAttachmentOperation {
    Update,
}

fn forced_chat_intent(operation: Option<ChatAttachmentOperation>) -> Option<&'static str> {
    match operation {
        Some(ChatAttachmentOperation::Update) => Some("update_chunk"),
        None => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatAttachment {
    pub chunk_id: Option<String>,
    pub item_id: Option<String>,
    #[serde(default)]
    pub expected_updated_at: Option<String>,
    #[serde(default)]
    pub operation: Option<ChatAttachmentOperation>,
}

fn parse_expected_chunk_updated_at(value: &str) -> AppResult<DateTime> {
    DateTime::parse_rfc3339_str(value.trim()).map_err(|_| {
        AppError::BadRequest("attachments.expectedUpdatedAt must be RFC3339".to_string())
    })
}

async fn freeze_chat_chunk_attachments(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    attachments: &mut [ChatAttachment],
) -> AppResult<()> {
    for attachment in attachments {
        if attachment.operation.is_some()
            && attachment
                .chunk_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
        {
            return Err(AppError::BadRequest(
                "attachments.operation requires chunkId".to_string(),
            ));
        }
        let Some(chunk_id) = attachment
            .chunk_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let object_id = parse_object_id(chunk_id)?;
        let chunk = state
            .db
            .operation_knowledge_chunks()
            .find_one(
                doc! {
                    "_id": object_id,
                    "workspace_id": workspace_id,
                    "domain": "user_operations",
                    "$or": [
                        { "account_id": null },
                        { "account_id": account_id },
                    ],
                },
                None,
            )
            .await?
            .ok_or_else(|| AppError::NotFound(format!("chunk {chunk_id} not found")))?;
        if let Some(expected) = attachment.expected_updated_at.as_deref() {
            let expected = parse_expected_chunk_updated_at(expected)?;
            if expected.timestamp_millis() != chunk.updated_at.timestamp_millis() {
                return Err(AppError::Conflict("chat_chunk_snapshot_stale".to_string()));
            }
        }
        attachment.expected_updated_at =
            Some(chunk.updated_at.try_to_rfc3339_string().map_err(|error| {
                AppError::External(format!("serialize chunk updated_at failed: {error}"))
            })?);
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DigestSelectedCardBinding {
    pub card_id: String,
    pub card_hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DigestSelectionBinding {
    pub account_id: String,
    pub report_id: String,
    pub report_date: String,
    pub report_generation: i64,
    pub report_hash: String,
    pub selected_cards: Vec<DigestSelectedCardBinding>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurnRequest {
    /// 缺省则后端 new uuid 当 sessionId。
    pub session_id: Option<String>,
    pub account_id: Option<String>,
    /// knowledge-digest-workstation Phase 5：运营 ID（用于隔离 operator memory）。
    /// 缺省回退到 `default`，与 chat_task_create 字段对齐。
    pub operator_id: Option<String>,
    pub content: String,
    /// 引用的切片 / 知识包；本轮只取第 1 条（≤ 1 attachments）。
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    /// SR-125: explicit operator-selected digest snapshot. Without this
    /// binding a digest_action turn may explain how to select cards, but it
    /// cannot produce an executable dispatch candidate.
    #[serde(default)]
    pub digest_selection: Option<DigestSelectionBinding>,
}

pub async fn chat_turn(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(mut body): Json<ChatTurnRequest>,
) -> AppResult<Json<Value>> {
    let trimmed = body.content.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest("content cannot be empty".to_string()));
    }
    let session_id = body
        .session_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    freeze_chat_chunk_attachments(
        &state,
        &admin.current_workspace,
        &account_id,
        &mut body.attachments,
    )
    .await?;
    ensure_chat_session_identity(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        &admin.user_id,
    )
    .await?;
    let operator_id = body
        .operator_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());

    // 加载历史 turns（按 turn_index 升序）
    let history =
        load_chat_history(&state, &admin.current_workspace, &account_id, &session_id).await?;
    // P1-7：原子预分配两个 turn_index——user turn + assistant turn，避免并发
    // 写者读到同一 last 制造重复索引。返回的是分配后的最大 seq；user 拿
    // `assistant_index - 1`、assistant 拿 `assistant_index`。
    let assistant_index = allocate_next_turn_indices(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        &admin.user_id,
        2,
    )
    .await?;
    let next_index = assistant_index - 1;
    let assistant_turns_so_far = history.iter().filter(|t| t.role == "assistant").count() as i32;
    if assistant_turns_so_far >= CHAT_MAX_TURNS_PER_SESSION {
        return Err(AppError::BadRequest(format!(
            "session {session_id} 已达 {CHAT_MAX_TURNS_PER_SESSION} 轮上限，请「应用为草稿」或开启新会话"
        )));
    }

    // 写 user turn
    write_chat_turn(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        next_index,
        "user",
        None,
        trimmed,
        &body.attachments,
        &[],
        None,
        &[],
        &[],
        "pending",
        0,
        None,
    )
    .await?;

    let attachment = body.attachments.first();
    let chunk_attached = attachment
        .and_then(|a| a.chunk_id.as_deref())
        .filter(|s| !s.trim().is_empty());
    let chunk_expected_updated_at = attachment
        .and_then(|a| a.expected_updated_at.as_deref())
        .filter(|s| !s.trim().is_empty());
    let chunk_operation = attachment.and_then(|a| a.operation);
    let item_attached = attachment
        .and_then(|a| a.item_id.as_deref())
        .filter(|s| !s.trim().is_empty());

    let run_id = format!("chat-{session_id}-turn-{next_index}");
    let budget = Arc::new(agent::RunBudget::new(
        run_id.clone(),
        CHAT_TOKEN_BUDGET_PER_TURN,
        CHAT_MAX_LLM_CALLS_PER_TURN,
        i32::MAX,
    ));

    let result = agent::RUN_BUDGET
        .scope(budget.clone(), async {
            run_chat_turn_pipeline(
                &state,
                &admin.current_workspace,
                &account_id,
                &operator_id,
                &session_id,
                trimmed,
                chunk_attached,
                chunk_expected_updated_at,
                chunk_operation,
                item_attached,
                body.digest_selection.as_ref(),
                &history,
            )
            .await
        })
        .await?;

    let intent = result
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("freeform")
        .to_string();
    let natural_reply = result
        .get("naturalReply")
        .and_then(|v| v.as_str())
        .unwrap_or("（AI 未给出回复）")
        .to_string();
    let patch = result.get("patch").cloned();
    let missing_fields: Vec<String> = result
        .get("missingFields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.as_str().map(|s| s.to_string()).or_else(|| {
                        x.get("field")
                            .and_then(|f| f.as_str())
                            .map(|s| s.to_string())
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let followups: Vec<Value> = result
        .get("followupQuestions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(CHAT_MAX_FOLLOWUPS)
        .collect();
    let draft_kind = result
        .get("draftKind")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let target_chunk_id = result
        .get("targetChunkId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let target_pack_id = result
        .get("targetPackId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let prompt_key = result
        .get("promptKey")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // SR-125：digest_action 返回的是后端按日报权威卡片重建的候选；
    // action / summary / target 均不再来自 LLM 或客户端。
    let planned_steps = result.get("plannedSteps").cloned();
    let estimated_llm_calls = result.get("estimatedLlmCalls").and_then(|v| v.as_i64());
    let digest_selection = result.get("digestSelection").cloned();
    let candidate_hash = result
        .get("candidateHash")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let can_apply = patch.is_some() && missing_fields.is_empty() && draft_kind.is_some();
    let tokens_used = budget.snapshot().tokens_used;

    // 写 assistant turn
    let attachments_for_assistant: Vec<ChatAttachment> = match (&target_chunk_id, &target_pack_id) {
        (Some(c), _) => vec![ChatAttachment {
            chunk_id: Some(c.clone()),
            item_id: None,
            expected_updated_at: result
                .get("expectedUpdatedAt")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    body.attachments
                        .iter()
                        .find(|attachment| attachment.chunk_id.as_deref() == Some(c.as_str()))
                        .and_then(|attachment| attachment.expected_updated_at.clone())
                }),
            operation: body
                .attachments
                .iter()
                .find(|attachment| attachment.chunk_id.as_deref() == Some(c.as_str()))
                .and_then(|attachment| attachment.operation),
        }],
        (None, Some(p)) => vec![ChatAttachment {
            chunk_id: None,
            item_id: Some(p.clone()),
            expected_updated_at: None,
            operation: None,
        }],
        _ => body.attachments,
    };
    let candidate_attachment = match (
        digest_selection.as_ref(),
        candidate_hash.as_deref(),
        planned_steps.as_ref(),
    ) {
        (Some(selection), Some(hash), Some(Value::Array(steps))) if !steps.is_empty() => {
            let selection_doc = bson_from_json(selection).map_err(|error| {
                AppError::Conflict(format!("digest_dispatch_candidate_invalid: {error}"))
            })?;
            let step_docs = steps
                .iter()
                .map(bson_from_json)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    AppError::Conflict(format!("digest_dispatch_candidate_invalid: {error}"))
                })?;
            vec![doc! {
                "kind": "digest_dispatch_candidate",
                "candidateHash": hash,
                "digestSelection": selection_doc,
                "plannedSteps": step_docs,
            }]
        }
        _ => Vec::new(),
    };

    write_chat_turn(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        assistant_index,
        "assistant",
        Some(&intent),
        &natural_reply,
        &attachments_for_assistant,
        &candidate_attachment,
        patch.as_ref(),
        &missing_fields,
        &followups,
        "pending",
        tokens_used,
        prompt_key.as_deref(),
    )
    .await?;

    // P2-15：chat 路径的 KnowledgeUsageLog 必须带 promptVersions，复用 R11 既有 prompt 版本
    // 审计语义（与日报 / management 路径对齐）。一次 turn 可能命中 intent/draft/update/clarify
    // 中的多个，统一拉取 4 把 chat 钥匙的 active 版本号；prompt_versions 拉取失败不阻塞主链路。
    let chat_prompt_versions = prompts::prompt_versions(
        &state.db,
        &admin.current_workspace,
        &[
            "knowledge.chat.intent",
            "knowledge.chat.draft_chunk",
            "knowledge.chat.update_chunk",
            "knowledge.chat.clarify",
        ],
        None,
        None,
    )
    .await
    .unwrap_or_else(|_| doc! {});

    let usage_doc = doc! {
        "kind": "chunk_chat_session",
        "intent": &intent,
        "sessionId": &session_id,
        "turnIndex": assistant_index as i32,
        "missingFieldCount": missing_fields.len() as i32,
        "followupCount": followups.len() as i32,
        "draftKind": draft_kind.clone().unwrap_or_default(),
        "promptKey": prompt_key.clone().unwrap_or_default(),
        "promptVersions": chat_prompt_versions.clone(),
    };
    let _ = state
        .db
        .knowledge_usage_logs()
        .insert_one(
            KnowledgeUsageLog {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: account_id.clone(),
                contact_wxid: None,
                run_id: run_id.clone(),
                knowledge_ids: vec![],
                route_result: usage_doc,
                reply_text: Some(natural_reply.clone()),
                review_approved: false,
                blocked_reason: Some("chunk_chat_session_pending_operator_apply".to_string()),
                tool_trace: vec![doc! { "phase": format!("chunk_chat_turn_{assistant_index}") }],
                created_at: DateTime::now(),
            },
            None,
        )
        .await;
    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_chat_turn",
        format!("AI 对话补完 sessionId={session_id} 第 {assistant_index} 轮 intent={intent}"),
        doc! {
            "kind": "chunk_chat_session",
            "sessionId": &session_id,
            "turnIndex": assistant_index as i32,
            "intent": &intent,
            "missingFieldCount": missing_fields.len() as i32,
            "followupCount": followups.len() as i32,
            "tokensUsed": tokens_used,
            "draftKind": draft_kind.clone().unwrap_or_default(),
            "budget": budget_document(&budget),
        },
    )
    .await;

    Ok(Json(json!({
        "sessionId": session_id,
        "turnIndex": assistant_index,
        "intent": intent,
        "naturalReply": natural_reply,
        "draftKind": draft_kind,
        "draftPreview": patch,
        "plannedSteps": planned_steps,
        "estimatedLlmCalls": estimated_llm_calls,
        "digestSelection": digest_selection,
        "candidateHash": candidate_hash,
        "missingFields": missing_fields,
        "followupQuestions": followups,
        "canApply": can_apply,
        "targetChunkId": target_chunk_id,
        "targetPackId": target_pack_id,
        "expectedUpdatedAt": result.get("expectedUpdatedAt").cloned(),
        "promptKey": prompt_key,
        "tokensUsed": tokens_used,
        "budget": budget_document(&budget),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChatSessionScopeQuery {
    pub account_id: Option<String>,
}

fn expected_chat_account(account_id: Option<&str>, default_account_id: &str) -> String {
    account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_account_id)
        .to_string()
}

pub(in crate::routes) async fn chat_history(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
    Query(query): Query<ChatSessionScopeQuery>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let account_id = expected_chat_account(
        query.account_id.as_deref(),
        &state.config.default_account_id,
    );
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    require_chat_session_identity(
        &state,
        &admin.current_workspace,
        &account_id,
        trimmed,
        &admin.user_id,
    )
    .await?;
    let mut cursor = state
        .db
        .knowledge_chat_turns()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "session_id": trimmed,
            },
            FindOptions::builder()
                .sort(doc! { "turn_index": 1 })
                .build(),
        )
        .await?;
    let mut items: Vec<Value> = vec![];
    while let Some(turn) = cursor.try_next().await? {
        items.push(chat_turn_to_view(&turn));
    }
    Ok(Json(json!({
        "sessionId": trimmed,
        "items": items,
        "total": items.len() as i32,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatApplyRequest {
    pub account_id: Option<String>,
}

pub async fn chat_apply(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
    Json(body): Json<ChatApplyRequest>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let account_id =
        expected_chat_account(body.account_id.as_deref(), &state.config.default_account_id);
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    require_chat_session_identity(
        &state,
        &admin.current_workspace,
        &account_id,
        &trimmed,
        &admin.user_id,
    )
    .await?;
    const MAX_TRANSACTION_ATTEMPTS: usize = 3;
    for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
        match chat_apply_once(&state, &admin, &account_id, &trimmed).await {
            Ok(receipt) => return Ok(Json(receipt)),
            Err(error)
                if attempt + 1 < MAX_TRANSACTION_ATTEMPTS
                    && is_transient_chat_apply_error(&error) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Err(AppError::Conflict(
        "chat_apply_transaction_conflict".to_string(),
    ))
}

fn is_transient_chat_apply_error(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Db(db_error) if db_error.contains_label("TransientTransactionError")
    )
}

async fn chat_apply_once(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    account_id: &str,
    session_id: &str,
) -> AppResult<Value> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result =
        chat_apply_in_transaction(state, admin, account_id, session_id, &mut session).await;
    let receipt = match result {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction(&mut session).await?;
    Ok(receipt)
}

async fn chat_apply_in_transaction(
    state: &AppState,
    admin: &AuthenticatedAdmin,
    account_id: &str,
    session_id: &str,
    session: &mut ClientSession,
) -> AppResult<Value> {
    state
        .db
        .knowledge_chat_session_seqs()
        .find_one_with_session(
            doc! {
                "_id": chat_session_row_id(&admin.current_workspace, session_id),
                "workspace_id": &admin.current_workspace,
                "session_id": session_id,
                "account_id": account_id,
                "owner_admin_id": &admin.user_id,
            },
            None,
            session,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("chat session not found".to_string()))?;

    let history = load_chat_history_with_session(
        state,
        &admin.current_workspace,
        account_id,
        session_id,
        session,
    )
    .await?;
    let last_assistant = history
        .iter()
        .rev()
        .find(|turn| turn.role == "assistant" && turn.patch.is_some())
        .ok_or_else(|| {
            AppError::BadRequest(
                "session 没有可应用的 AI 草稿（需要先发起 chat 让 AI 起草）".to_string(),
            )
        })?;
    if last_assistant.status == "applied" {
        let receipt = last_assistant
            .apply_result
            .clone()
            .ok_or_else(|| AppError::Conflict("chat_apply_receipt_missing".to_string()))?;
        return Ok(Bson::Document(receipt).into());
    }
    if last_assistant.status != "pending" {
        return Err(AppError::Conflict(format!(
            "chat_draft_not_applicable:{}",
            last_assistant.status
        )));
    }

    let turn_id = last_assistant
        .id
        .ok_or_else(|| AppError::External("chat turn missing _id".to_string()))?;
    let intent = last_assistant.intent.as_deref().unwrap_or("freeform");
    let patch = last_assistant
        .patch
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("最近一轮 AI 没有 patch".to_string()))?;
    let claimed = state
        .db
        .knowledge_chat_turns()
        .update_one_with_session(
            doc! {
                "_id": turn_id,
                "workspace_id": &admin.current_workspace,
                "account_id": account_id,
                "session_id": session_id,
                "role": "assistant",
                "status": "pending",
            },
            doc! { "$set": { "status": "applying", "updated_at": DateTime::now() } },
            None,
            session,
        )
        .await?;
    if claimed.matched_count != 1 {
        return Err(AppError::Conflict("chat_apply_claim_conflict".to_string()));
    }

    let operator_statement = history
        .iter()
        .filter(|turn| turn.role == "user")
        .map(|turn| turn.content.trim())
        .filter(|content| !content.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let target_chunk_id = last_assistant
        .attachments
        .iter()
        .filter_map(|attachment| attachment.get_str("chunk_id").ok())
        .find(|value| !value.is_empty());
    let target_chunk_expected_updated_at = last_assistant
        .attachments
        .iter()
        .find(|attachment| {
            attachment
                .get_str("chunk_id")
                .ok()
                .is_some_and(|value| Some(value) == target_chunk_id)
        })
        .and_then(|attachment| attachment.get_str("expected_updated_at").ok());
    let target_pack_id = last_assistant
        .attachments
        .iter()
        .filter_map(|attachment| attachment.get_str("item_id").ok())
        .find(|value| !value.is_empty());

    let result_value = match intent {
        "create_chunk" => {
            let create_account_id =
                (account_id != state.config.default_account_id).then_some(account_id);
            apply_create_chunk_with_session(
                state,
                &admin.current_workspace,
                create_account_id,
                session_id,
                patch,
                target_pack_id,
                &operator_statement,
                session,
            )
            .await?
        }
        "update_chunk" => {
            let chunk_id = target_chunk_id.ok_or_else(|| {
                AppError::BadRequest("update_chunk 需要 attachments.chunkId".to_string())
            })?;
            apply_update_chunk_with_session(
                state,
                &admin.current_workspace,
                account_id,
                chunk_id,
                patch,
                &operator_statement,
                target_chunk_expected_updated_at,
                session,
            )
            .await?
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "intent={other} 不可应用为草稿（仅 create_chunk / update_chunk 可应用）"
            )));
        }
    };

    let result_bson = mongodb::bson::to_bson(&result_value)?;
    let receipt = doc! {
        "ok": true,
        "sessionId": session_id,
        "intent": intent,
        "result": result_bson.clone(),
    };
    let now = DateTime::now();
    state
        .db
        .events()
        .insert_one_with_session(
            crate::models::AgentEvent {
                id: None,
                workspace_id: admin.current_workspace.clone(),
                account_id: account_id.to_string(),
                contact_wxid: None,
                kind: "knowledge_chat_applied".to_string(),
                status: "success".to_string(),
                summary: format!("AI 对话产物落库为草稿 sessionId={session_id} intent={intent}"),
                details: Some(doc! {
                    "kind": "chunk_chat_session",
                    "sessionId": session_id,
                    "turnId": turn_id,
                    "intent": intent,
                    "result": result_bson,
                }),
                created_at: now,
                dedupe_key: Some(format!("knowledge_chat_apply:{}", turn_id.to_hex())),
            },
            None,
            session,
        )
        .await?;
    let finalized = state
        .db
        .knowledge_chat_turns()
        .update_one_with_session(
            doc! {
                "_id": turn_id,
                "workspace_id": &admin.current_workspace,
                "account_id": account_id,
                "session_id": session_id,
                "status": "applying",
            },
            doc! {
                "$set": {
                    "status": "applied",
                    "apply_result": receipt.clone(),
                    "applied_at": now,
                    "updated_at": now,
                }
            },
            None,
            session,
        )
        .await?;
    if finalized.matched_count != 1 {
        return Err(AppError::Conflict(
            "chat_apply_finalize_conflict".to_string(),
        ));
    }
    Ok(Bson::Document(receipt).into())
}

async fn load_chat_history_with_session(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    session: &mut ClientSession,
) -> AppResult<Vec<KnowledgeChatTurn>> {
    let mut cursor = state
        .db
        .knowledge_chat_turns()
        .find_with_session(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "session_id": session_id,
            },
            FindOptions::builder()
                .sort(doc! { "turn_index": 1 })
                .build(),
            session,
        )
        .await?;
    let mut items = Vec::new();
    while let Some(turn) = cursor.next(session).await.transpose()? {
        items.push(turn);
    }
    Ok(items)
}

pub(in crate::routes) async fn chat_discard(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
    Query(query): Query<ChatSessionScopeQuery>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let account_id = expected_chat_account(
        query.account_id.as_deref(),
        &state.config.default_account_id,
    );
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    require_chat_session_identity(
        &state,
        &admin.current_workspace,
        &account_id,
        trimmed,
        &admin.user_id,
    )
    .await?;
    let res = state
        .db
        .knowledge_chat_turns()
        .update_many(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "session_id": trimmed,
                "status": "pending",
            },
            doc! { "$set": { "status": "discarded", "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    Ok(Json(json!({
        "ok": true,
        "sessionId": trimmed,
        "discardedCount": res.modified_count,
    })))
}

// ----- chat 内部辅助 -------------------------------------------------------

fn chat_session_row_id(workspace_id: &str, session_id: &str) -> String {
    format!("{workspace_id}|{session_id}")
}

pub(crate) fn chat_session_bus_key(
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
) -> String {
    format!("{workspace_id}|{account_id}|{session_id}")
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};

    match error.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => {
            is_duplicate_key_code(write_error.code)
        }
        ErrorKind::BulkWrite(failure) => failure.write_errors.as_ref().is_some_and(|errors| {
            errors
                .iter()
                .any(|write_error| is_duplicate_key_code(write_error.code))
        }),
        // findOneAndUpdate with upsert can report an _id collision as a command
        // error rather than a write error. It still represents the same OCC miss.
        ErrorKind::Command(command_error) => is_duplicate_key_code(command_error.code),
        _ => false,
    }
}

fn is_duplicate_key_code(code: i32) -> bool {
    matches!(code, 11000 | 11001)
}

async fn ensure_chat_session_identity(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    owner_admin_id: &str,
) -> AppResult<()> {
    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

    let key = chat_session_row_id(workspace_id, session_id);
    let now = DateTime::now();
    let result = state
        .db
        .knowledge_chat_session_seqs()
        .find_one_and_update(
            doc! {
                "_id": &key,
                "$or": [
                    {
                        "workspace_id": workspace_id,
                        "session_id": session_id,
                        "account_id": account_id,
                        "owner_admin_id": owner_admin_id,
                    },
                    {
                        "workspace_id": { "$exists": false },
                        "session_id": { "$exists": false },
                        "account_id": { "$exists": false },
                        "owner_admin_id": { "$exists": false },
                    },
                ],
            },
            doc! {
                "$setOnInsert": {
                    "seq": 0_i64,
                    "created_at": now,
                },
                "$set": {
                    "workspace_id": workspace_id,
                    "session_id": session_id,
                    "account_id": account_id,
                    "owner_admin_id": owner_admin_id,
                    "updated_at": now,
                },
            },
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await;
    match result {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(AppError::Conflict(
            "chat_session_scope_conflict".to_string(),
        )),
        Err(error) if is_duplicate_key_error(&error) => Err(AppError::Conflict(
            "chat_session_scope_conflict".to_string(),
        )),
        Err(error) => Err(error.into()),
    }
}

async fn require_chat_session_identity(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    owner_admin_id: &str,
) -> AppResult<()> {
    let row = state
        .db
        .knowledge_chat_session_seqs()
        .find_one(
            doc! {
                "_id": chat_session_row_id(workspace_id, session_id),
                "workspace_id": workspace_id,
                "session_id": session_id,
                "account_id": account_id,
                "owner_admin_id": owner_admin_id,
            },
            None,
        )
        .await?;
    if row.is_some() {
        Ok(())
    } else {
        Err(AppError::NotFound("chat session not found".to_string()))
    }
}

/// P1-7：原子分配下一个 `turn_index`。
///
/// 历史路径是「`find_one(sort=desc).turn_index + 1`」，并发两个写者会读到同一
/// `last`，写出重复 turn_index。本路径用 `knowledge_chat_session_seqs` 行
/// `{ _id: "{workspace_id}|{session_id}", seq: i64 }`，配 `findOneAndUpdate`
/// `$inc: { seq: count }` `upsert(true)` `returnDocument=After` 单次原子调
/// 用，返回的 `seq` 即为「分配给本次写入的最后一个 turn_index」；调用方需要
/// 一次写多条 turn 时传 `count > 1`，按 `seq - count + 1 .. seq` 顺序使用。
///
/// 注意：本助手 SHALL ONLY 用来分配新 turn_index，不能用来读历史 turn 数；
/// 历史拉取仍走 `load_chat_history`。
pub(in crate::routes) async fn allocate_next_turn_indices(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    owner_admin_id: &str,
    count: u32,
) -> AppResult<i32> {
    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
    let n = count.max(1) as i64;
    let key = chat_session_row_id(workspace_id, session_id);
    let updated = state
        .db
        .knowledge_chat_session_seqs()
        .find_one_and_update(
            doc! {
                "_id": &key,
                "workspace_id": workspace_id,
                "session_id": session_id,
                "account_id": account_id,
                "owner_admin_id": owner_admin_id,
            },
            doc! { "$inc": { "seq": n } },
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let seq = updated
        .as_ref()
        .and_then(|d| d.get_i64("seq").ok())
        .ok_or_else(|| AppError::Conflict("chat_session_scope_conflict".to_string()))?;
    // turn_index 字段在模型里是 i32；上限远超 i32::MAX 时直接 saturating，
    // 单 session ≥ 21 亿 turn 不在产品语义范围内。
    Ok(seq.try_into().unwrap_or(i32::MAX))
}

async fn load_chat_history(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
) -> AppResult<Vec<KnowledgeChatTurn>> {
    let mut filter = doc! {
        "workspace_id": workspace_id,
        "session_id": session_id,
    };
    if account_id != "*" {
        filter.insert("account_id", account_id);
    }
    let mut cursor = state
        .db
        .knowledge_chat_turns()
        .find(
            filter,
            FindOptions::builder()
                .sort(doc! { "turn_index": 1 })
                .build(),
        )
        .await?;
    let mut items = vec![];
    while let Some(t) = cursor.try_next().await? {
        items.push(t);
    }
    Ok(items)
}

#[allow(clippy::too_many_arguments)]
async fn write_chat_turn(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    turn_index: i32,
    role: &str,
    intent: Option<&str>,
    content: &str,
    attachments: &[ChatAttachment],
    extra_attachments: &[Document],
    patch: Option<&Value>,
    missing_fields: &[String],
    followups: &[Value],
    status: &str,
    tokens_used: i64,
    prompt_key: Option<&str>,
) -> AppResult<()> {
    let mut attachments_doc: Vec<Document> = attachments
        .iter()
        .filter_map(|a| {
            let mut d = Document::new();
            if let Some(c) = a.chunk_id.as_deref().filter(|s| !s.is_empty()) {
                d.insert("chunk_id", c.to_string());
            }
            if let Some(i) = a.item_id.as_deref().filter(|s| !s.is_empty()) {
                d.insert("item_id", i.to_string());
            }
            if let Some(expected) = a
                .expected_updated_at
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                d.insert("expected_updated_at", expected.to_string());
            }
            if a.operation == Some(ChatAttachmentOperation::Update) {
                d.insert("operation", "update");
            }
            if d.is_empty() {
                None
            } else {
                Some(d)
            }
        })
        .collect();
    attachments_doc.extend(extra_attachments.iter().cloned());
    let patch_doc = patch
        .and_then(|p| mongodb::bson::to_bson(p).ok())
        .and_then(|b| match b {
            Bson::Document(d) => Some(d),
            _ => None,
        });
    let followup_docs: Vec<Document> = followups
        .iter()
        .filter_map(|v| mongodb::bson::to_bson(v).ok())
        .filter_map(|b| match b {
            Bson::Document(d) => Some(d),
            _ => None,
        })
        .collect();

    state
        .db
        .knowledge_chat_turns()
        .insert_one(
            KnowledgeChatTurn {
                id: None,
                workspace_id: workspace_id.to_string(),
                account_id: account_id.to_string(),
                session_id: session_id.to_string(),
                turn_index,
                role: role.to_string(),
                intent: intent.map(|s| s.to_string()),
                content: content.to_string(),
                attachments: attachments_doc,
                patch: patch_doc,
                missing_fields: missing_fields.to_vec(),
                followup_questions: followup_docs,
                status: status.to_string(),
                apply_result: None,
                applied_at: None,
                tokens_used,
                prompt_key: prompt_key.map(|s| s.to_string()),
                created_at: DateTime::now(),
                kind: None,
                tool_calls: vec![],
            },
            None,
        )
        .await?;
    Ok(())
}

fn chat_turn_to_view(turn: &KnowledgeChatTurn) -> Value {
    json!({
        "id": turn.id.map(|o| o.to_hex()),
        "sessionId": turn.session_id,
        "turnIndex": turn.turn_index,
        "role": turn.role,
        "intent": turn.intent,
        "content": turn.content,
        "attachments": turn.attachments,
        "patch": turn.patch,
        "missingFields": turn.missing_fields,
        "followupQuestions": turn.followup_questions,
        "status": turn.status,
        "tokensUsed": turn.tokens_used,
        "promptKey": turn.prompt_key,
        // knowledge-digest-workstation Phase 4：worker 写的进度 turn 用
        // `kind = task_progress / task_summary / tool_call_log` 区分；
        // freeform / chat 默认不写。
        "kind": turn.kind,
        "toolCalls": turn.tool_calls,
        "createdAt": turn.created_at.try_to_rfc3339_string().unwrap_or_default(),
    })
}

/// 当 LLM 产出了 patch/起草结果却漏写 naturalReply（或留空）时，从结构化
/// 字段确定性地合成一句对话回执。通用于所有 draft/update 分支、与具体业务
/// 领域无关：只读结构化字段名，不内嵌任何样例文案。
fn synthesize_natural_reply_from_patch(out: &Value) -> Option<String> {
    let patch = out.get("patch")?.as_object()?;
    fn field_label(k: &str) -> &str {
        match k {
            "title" => "标题",
            "summary" => "摘要",
            "body" => "正文",
            "tags" => "标签",
            "knowledgeType" | "knowledge_type" => "知识类型",
            "priority" => "优先级",
            other => other,
        }
    }
    let filled: Vec<&str> = patch
        .iter()
        .filter(|(_, v)| match v {
            Value::String(s) => !s.trim().is_empty(),
            Value::Null => false,
            Value::Array(a) => !a.is_empty(),
            _ => true,
        })
        .map(|(k, _)| field_label(k.as_str()))
        .collect();
    if filled.is_empty() {
        return None;
    }
    let missing: Vec<String> = out
        .get("missingFields")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.as_str().map(|s| s.to_string()).or_else(|| {
                        x.get("field")
                            .and_then(|f| f.as_str())
                            .map(|s| s.to_string())
                    })
                })
                .map(|s| field_label(&s).to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut reply = if let Some(t) = patch
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        format!(
            "我已经按您的要求起草好{}，拟定的标题是「{t}」。",
            filled.join("、")
        )
    } else {
        format!("我已经为您起草好了{}。", filled.join("、"))
    };
    if missing.is_empty() {
        reply.push_str("您看一下内容是否准确，确认无误后即可应用为草稿。");
    } else {
        reply.push_str(&format!(
            "还差{} 需要补充，方便的话请再给我一些信息，我好把它补全。",
            missing.join("、")
        ));
    }
    Some(reply)
}

/// chat_turn 的核心 LLM 编排：先识别 intent，再分流到对应子 prompt。
/// 返回的 Value 至少包含 intent / naturalReply；可选 patch / missingFields /
/// followupQuestions / draftKind / targetChunkId / targetPackId / promptKey。
async fn run_chat_turn_pipeline(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    session_id: &str,
    user_content: &str,
    chunk_attached: Option<&str>,
    chunk_expected_updated_at: Option<&str>,
    chunk_operation: Option<ChatAttachmentOperation>,
    item_attached: Option<&str>,
    digest_selection: Option<&DigestSelectionBinding>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    // knowledge-digest-workstation Phase 5：先取运营长期偏好记忆，作为
    // intent 分类与下游分支的 prompt header。与 contacts.memory_card 物理
    // 隔离（仅触达 knowledge_operator_memory collection）。
    let operator_memory =
        agent::load_operator_memory(&state.db, workspace_id, account_id, operator_id, 5)
            .await
            .unwrap_or_default();
    let operator_memory_header = render_operator_memory_for_prompt(&operator_memory);

    // 1. intent 分类
    let intent_result = if digest_selection.is_some() {
        json!({ "intent": "digest_action" })
    } else if let Some(intent) = forced_chat_intent(chunk_operation) {
        json!({ "intent": intent, "targetChunkId": chunk_attached })
    } else {
        classify_intent(
            state,
            workspace_id,
            account_id,
            session_id,
            user_content,
            chunk_attached,
            item_attached,
            history,
            &operator_memory_header,
        )
        .await?
    };
    let intent = intent_result
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("freeform")
        .to_string();
    // An explicit operator attachment is authoritative. The model may classify the
    // requested operation, but it cannot redirect an edit to a different chunk.
    let target_chunk_id = chunk_attached.map(str::to_owned).or_else(|| {
        intent_result
            .get("targetChunkId")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let target_pack_id = intent_result
        .get("targetPackId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| item_attached.map(|s| s.to_string()));

    // 2. 按 intent 分流
    let mut out = match intent.as_str() {
        "create_chunk" => draft_chunk_for_chat(
            state,
            workspace_id,
            account_id,
            session_id,
            user_content,
            target_pack_id.as_deref(),
            history,
        )
        .await
        .map(|mut v| {
            v["draftKind"] = json!("chunk");
            v["promptKey"] = json!("knowledge.chat.draft_chunk");
            v
        })?,
        "update_chunk" => {
            let chunk_id = target_chunk_id.clone().ok_or_else(|| {
                AppError::BadRequest(
                    "update_chunk 需要 attachments.chunkId 或在对话中明确引用切片".to_string(),
                )
            })?;
            let mut v = update_chunk_for_chat(
                state,
                workspace_id,
                account_id,
                session_id,
                user_content,
                &chunk_id,
                chunk_expected_updated_at,
                history,
            )
            .await?;
            v["draftKind"] = json!("chunk_update");
            v["promptKey"] = json!("knowledge.chat.update_chunk");
            v
        }
        "digest_action" => {
            let mut v = dispatch_digest_action_for_chat(
                state,
                workspace_id,
                account_id,
                session_id,
                user_content,
                digest_selection,
                history,
            )
            .await?;
            v["draftKind"] = json!("digest_dispatch");
            v["promptKey"] = json!("knowledge.digest.dispatch");
            v
        }
        "update_operator_memory" => {
            let mut v = update_operator_memory_for_chat(
                state,
                workspace_id,
                account_id,
                operator_id,
                user_content,
                &intent_result,
            )
            .await?;
            v["draftKind"] = json!("operator_memory");
            v["promptKey"] = json!("knowledge.chat.intent");
            v
        }
        "revoke_operator_memory" => {
            let mut v = revoke_operator_memory_for_chat(
                state,
                workspace_id,
                account_id,
                operator_id,
                &intent_result,
            )
            .await?;
            v["draftKind"] = json!("operator_memory");
            v["promptKey"] = json!("knowledge.chat.intent");
            v
        }
        _ => clarify_for_chat(
            state,
            workspace_id,
            account_id,
            session_id,
            user_content,
            history,
        )
        .await
        .map(|mut v| {
            v["promptKey"] = json!("knowledge.chat.clarify");
            v
        })?,
    };

    out["intent"] = json!(intent);
    if let Some(c) = target_chunk_id {
        out["targetChunkId"] = json!(c);
    }
    if let Some(p) = target_pack_id {
        out["targetPackId"] = json!(p);
    }
    let reply_blank = out
        .get("naturalReply")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().is_empty() || s.trim() == "（AI 未给出回复）")
        .unwrap_or(true);
    if reply_blank {
        if let Some(synth) = synthesize_natural_reply_from_patch(&out) {
            out["naturalReply"] = json!(synth);
        }
    }
    Ok(out)
}

fn render_chat_history_for_prompt(history: &[KnowledgeChatTurn]) -> String {
    if history.is_empty() {
        return "（暂无历史）".to_string();
    }
    let mut s = String::new();
    for t in history
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .iter()
        .rev()
    {
        s.push_str(&format!(
            "- [{}] {}: {}\n",
            t.turn_index,
            t.role,
            truncate_for_prompt(&t.content, 200)
        ));
    }
    s
}

/// knowledge-digest-workstation Phase 5：把 KnowledgeOperatorMemory 渲染成
/// system prompt header（≤ 5 条），帮 intent 分类与下游分支保持运营长期偏好。
/// 与 contacts.memory_card 物理隔离，prompt header 也分开命名为「运营长期偏好」。
fn render_operator_memory_for_prompt(
    memories: &[crate::models::KnowledgeOperatorMemory],
) -> String {
    let active = memories.iter().filter(|memory| memory.revoked_at.is_none());
    if active.clone().next().is_none() {
        return String::new();
    }
    let mut s = String::from("【运营长期偏好（仅作上下文，不要写回 chunk patch）】\n");
    for m in active.take(5) {
        let kind_label = match m.kind.as_str() {
            "preference" => "偏好",
            "rejection" => "红线",
            "context" => "背景",
            other => other,
        };
        s.push_str(&format!(
            "- {kind_label}：{}\n",
            truncate_for_prompt(&m.content, 120)
        ));
    }
    s
}

// ===========================================================================
// 知识库 chat agent 的多轮工具循环（knowledge-digest-workstation Phase 5 / P5.2）
// ---------------------------------------------------------------------------
//
// 设计目标：让 chat 三大下游 prompt（draft_chunk / update_chunk / clarify）走真
// 正的 agent tool loop —— Reply Agent 可以多轮自主调用 knowledge.* 工具去观察
// 整个知识库（catalog / search / open_slice / audit_completeness / search_chunks /
// propose_repair / analyze_logs / open_document / verify_anchor）
// 再决定最终输出。
//
// 强约束（与 user-ops tool_loop 保持同构）：
// - 单 turn ≤ CHAT_TOOL_LOOP_MAX_LOOPS=4 轮；
// - 单轮 toolCalls ≤ 6；
// - 单 dispatch 5s timeout；
// - 失败连击 ≥3 强制结束；
// - 总耗时 30s 硬超时；
// - tool_call_budget 超额按 budget_exceeded 强制结束；
// - 永不写库、永不进 outbox、永不进 mcp（与 user-ops gateway 物理隔离）；
// - AI 永不自动 verify：chat 落库由 chat_apply 强制 status=draft + needs_review。
// ===========================================================================

/// 把基础 system prompt 增广上 tool-calling 协议头：
/// - 解释 decisionPhase 取值（tool_calling / final）；
/// - 列出可用 tool 白名单；
/// - 限制 toolCalls 数量与 final 字段约束。
///
/// 注意：本函数只追加协议提示，不删除/改写原 prompt 内容。
fn augment_chat_system_with_tools(base: &str) -> String {
    let tool_list = agent::ALLOWED_CHAT_TOOL_NAMES.join(" / ");
    format!(
        r#"{base}

【tool-calling 协议（chat agent 必须遵守）】
- 输出 JSON 必须包含 `decisionPhase`，取值仅限 `tool_calling` / `final`。
- 当你需要观察知识库当前状态时，输出 `decisionPhase=tool_calling` + `toolCalls` 数组（≤ 6 个），可用工具：
  {tool_list}
  工具的入参字段名遵循 camelCase（如 chunkId / documentId / itemId / sourceQuote / topK / onlyVerified / hours）。
- `tool_calling` 中间轮 **不要** 输出 `naturalReply / patch / missingFields / followupQuestions`；这些字段只在 `final` 轮给。
- 当不再需要更多工具结果、可以给运营回复时，输出 `decisionPhase=final` + 业务字段（naturalReply / patch? / missingFields? / followupQuestions?）；不要再带 toolCalls。
- 单 turn 最多 4 轮工具循环、6 次 LLM call；超过会被 budget 截断。
- 每轮工具结果会以 `[system tool result]` 段附加到 user prompt 末尾，下一轮直接读。
- 不要伪造工具结果；只能使用实际返回的内容。
"#
    )
}

/// 单次 chat tool-calling 循环的入口。
///
/// 行为：
/// 1. 拉取本 workspace 的 [`agent::types::KnowledgeRuntime`] 快照（document/item/chunk）；
/// 2. 用当前 [`agent::RUN_BUDGET`] 当作循环 budget；
/// 3. 构造 reply_fn 闭包：调 `agent::generate_agent_json`（注入累计的
///    `[system tool result]`）→ 用 `RawAgentDecision::validate_and_promote` 反序列化；
/// 4. 调 [`agent::chat_reply_with_tools_loop`]；
/// 5. 在 final 轮把最近一次 LLM 原始 JSON（含 patch / missingFields / followupQuestions /
///    naturalReply 等业务字段）返回给 caller。
///
/// 返回的 Value 形态与原先直接 `generate_agent_json` 输出一致，下游
/// `run_chat_turn_pipeline` / `chat_turn` handler 不需要任何改造。
async fn run_chat_with_tools(
    state: &AppState,
    workspace_id_in: &str,
    account_id: &str,
    session_id: &str,
    run_key: &str,
    prompt_key: &str,
    system: String,
    user: String,
) -> AppResult<Value> {
    use std::pin::Pin;
    use std::sync::Mutex as StdMutex;

    use agent::types::{KnowledgeRuntime, RawAgentDecision};
    use agent::{
        chat_reply_with_tools_loop, ChatReplyFn, ChatToolLoopError, RunBudget,
        UserRuntimeParameters,
    };

    // Load only shared or current-account knowledge into the in-memory tool runtime.
    let workspace_id = workspace_id_in.to_string();
    let documents: Vec<OperationKnowledgeDocument> = state
        .db
        .operation_knowledge_documents()
        .find(
            doc! {
                "workspace_id": &workspace_id,
                "domain": "user_operations",
                "status": "active",
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id },
                ],
            },
            FindOptions::builder()
                .sort(doc! { "updated_at": -1_i32 })
                .limit(80)
                .build(),
        )
        .await?
        .try_collect()
        .await?;
    let chunks: Vec<OperationKnowledgeChunk> = state
        .db
        .operation_knowledge_chunks()
        .find(
            doc! {
                "workspace_id": &workspace_id,
                "domain": "user_operations",
                "status": "active",
                "integrity_status": "verified",
                "$or": [
                    { "account_id": null },
                    { "account_id": account_id },
                ],
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1_i32, "updated_at": -1_i32 })
                .limit(200)
                .build(),
        )
        .await?
        .try_collect()
        .await?;
    let knowledge = KnowledgeRuntime { documents, chunks };
    let runtime = UserRuntimeParameters::default();

    // 取当前 RUN_BUDGET（chat_turn handler 已经 scope 进来了）；
    // 若拿不到——属于不应发生的情况——回退到一个本地 budget（让 loop 仍能跑）。
    let budget = agent::current_run_budget().unwrap_or_else(|| {
        Arc::new(RunBudget::new(
            format!("chat-fallback-{session_id}-{run_key}"),
            CHAT_TOKEN_BUDGET_PER_TURN,
            CHAT_MAX_LLM_CALLS_PER_TURN,
            i32::MAX,
        ))
    });

    // 用 Arc<StdMutex<Option<Value>>> 把每轮 LLM 原始 JSON 透传出来。chat
    // 路径在 `final` 轮需要 patch / missingFields / followupQuestions /
    // naturalReply 等字段，AgentDecision 不直接覆盖这些；最简单是把原始
    // Value 暂存，在循环结束后取出。
    let last_raw: Arc<StdMutex<Option<Value>>> = Arc::new(StdMutex::new(None));

    // reply_fn 闭包：每轮被 chat_reply_with_tools_loop 调用。
    let state_arc = Arc::new(state.clone());
    let workspace_id_owned = workspace_id.clone();
    let account_id_owned = account_id.to_string();
    let session_id_owned = session_id.to_string();
    let run_key_owned = run_key.to_string();
    let prompt_key_owned = prompt_key.to_string();
    let system_owned = system;
    let user_owned = user;
    let last_raw_for_fn = Arc::clone(&last_raw);
    let runtime_for_fn = runtime.clone();

    let reply_fn: ChatReplyFn<'_> = Box::new(move |tool_results: &str, loop_count: i32| {
        let state_arc = Arc::clone(&state_arc);
        let workspace_id_owned = workspace_id_owned.clone();
        let account_id_owned = account_id_owned.clone();
        let session_id_owned = session_id_owned.clone();
        let run_key_owned = run_key_owned.clone();
        let prompt_key_owned = prompt_key_owned.clone();
        let system_owned = system_owned.clone();
        let user_owned = user_owned.clone();
        let tool_results_owned = tool_results.to_string();
        let last_raw = Arc::clone(&last_raw_for_fn);
        let runtime_for_fn = runtime_for_fn.clone();
        let fut: Pin<Box<dyn std::future::Future<Output = _> + Send>> = Box::pin(async move {
            // 把累计的 [system tool result] 注入 user prompt 末尾。
            let user_with_tools = if tool_results_owned.is_empty() {
                user_owned.clone()
            } else {
                format!("{user_owned}\n\n[system tool result]{tool_results_owned}")
            };
            let run_id = format!("chat-{session_id_owned}-{run_key_owned}-loop-{loop_count}");
            let value = agent::generate_agent_json(
                &state_arc,
                &workspace_id_owned,
                Some(&account_id_owned),
                None,
                Some(&run_id),
                &prompt_key_owned,
                &system_owned,
                &user_with_tools,
            )
            .await?;
            // 把原始 JSON 暂存：循环结束后从 last_raw 取出来当 final payload。
            if let Ok(mut guard) = last_raw.lock() {
                *guard = Some(value.clone());
            }
            // 反序列化为 RawAgentDecision，再 promote 到 AgentDecision。
            let raw: RawAgentDecision = serde_json::from_value(value).map_err(AppError::from)?;
            let (decision, promote_risks) = raw.validate_and_promote(&runtime_for_fn);
            Ok((decision, promote_risks))
        });
        fut
    });

    // 跑循环。任意 dispatch 错误以 Value 形态注入下一轮，循环只在 budget /
    // failure_streak / total_timeout 三种情况下提前结束。
    let outcome = chat_reply_with_tools_loop(
        &runtime,
        &knowledge,
        &state.db,
        &workspace_id,
        account_id,
        budget,
        Some(source_anchor_for_quote_ffi as agent::AnchorMatchFn),
        reply_fn,
    )
    .await;
    let final_value = match outcome {
        Ok(outcome) => finalize_chat_tool_loop_payload(
            last_raw.lock().ok().and_then(|g| g.clone()),
            &outcome.decision.decision_phase,
            &outcome.risks,
        ),
        Err(ChatToolLoopError::Timeout { elapsed_ms, .. }) => {
            // 超时——返回温和 final，让上层 handler 仍能写 turn 与 event。
            json!({
                "decisionPhase": "final",
                "naturalReply": format!("（AI 工具循环超时 elapsed_ms={elapsed_ms}，请稍后再试或换个说法）"),
                "toolLoopTruncated": true,
                "toolLoopStopReason": "timeout",
            })
        }
        Err(ChatToolLoopError::Reply(err)) => return Err(err),
    };
    Ok(final_value)
}

fn finalize_chat_tool_loop_payload(
    raw: Option<Value>,
    normalized_phase: &str,
    risks: &[String],
) -> Value {
    let raw_is_final = raw
        .as_ref()
        .and_then(|value| value.get("decisionPhase"))
        .and_then(Value::as_str)
        == Some("final");
    if raw_is_final && normalized_phase == "final" {
        return raw.expect("raw_is_final requires a raw payload");
    }
    json!({
        "decisionPhase": "final",
        "naturalReply": "（AI 工具探索未能完成，请稍后重试或补充更具体的信息。）",
        "toolLoopTruncated": true,
        "toolLoopStopReason": forced_chat_final_reason(risks),
    })
}

fn forced_chat_final_reason(risks: &[String]) -> &'static str {
    if risks
        .iter()
        .any(|risk| risk == "chat_tool_budget_exhausted")
    {
        "budget_exhausted"
    } else if risks
        .iter()
        .any(|risk| risk == "chat_tool_call_failure_streak")
    {
        "tool_failure_streak"
    } else if risks.iter().any(|risk| risk == "chat_tool_loop_exhausted") {
        "loop_exhausted"
    } else {
        "forced_stop"
    }
}

/// `verify_anchor` 工具的 source_quote→anchor 模糊匹配实现适配器。
/// 把 `source_anchor_for_quote(raw_content, document_id, source_quote)` 中
/// 的 `Option<ObjectId>` 参数转为 `Option<String>`（hex），让其符合
/// [`agent::AnchorMatchFn`] 的纯函数签名（避免 knowledge_tools.rs 直接依赖
/// mongodb::bson::oid::ObjectId 与 routes 模块）。
fn source_anchor_for_quote_ffi(
    raw_content: &str,
    document_id_hex: Option<String>,
    source_quote: &str,
) -> Option<Document> {
    let oid = document_id_hex
        .as_deref()
        .and_then(|h| ObjectId::parse_str(h).ok());
    source_anchor_for_quote(raw_content, oid, source_quote)
}

/// knowledge-digest-workstation Phase 5：intent=update_operator_memory 分支。
///
/// 落库 KnowledgeOperatorMemory 一条；返回的 Value 满足 chat_turn handler 对
/// `naturalReply / missingFields / followupQuestions` 的约定，但不出 patch
/// （AI 偏好/红线不进 chunk）。
async fn update_operator_memory_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    user_content: &str,
    intent_result: &Value,
) -> AppResult<Value> {
    let kind = intent_result
        .get("memoryKind")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("preference");
    let content = intent_result
        .get("memoryContent")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| user_content.trim().to_string());
    if !["preference", "rejection", "context"].contains(&kind) {
        return Ok(json!({
            "naturalReply": "AI 没法判定您要立的是偏好还是红线，能再说得具体一点吗？",
            "missingFields": ["memoryKind"],
            "followupQuestions": [{
                "id": "q1",
                "field": "memoryKind",
                "question": "请明确：是偏好（preference）/ 红线（rejection）/ 背景（context）？",
            }],
        }));
    }
    let mem = agent::record_operator_memory(
        &state.db,
        workspace_id,
        account_id,
        operator_id,
        kind,
        &content,
    )
    .await?;
    let kind_label = match kind {
        "preference" => "偏好",
        "rejection" => "红线",
        "context" => "背景",
        other => other,
    };
    let summary = format!(
        "已记下您的{kind_label}：{}",
        truncate_for_prompt(&content, 80)
    );
    record_repair_event(
        state,
        workspace_id,
        account_id,
        "knowledge_operator_memory_added",
        summary.clone(),
        doc! {
            "kind": "operator_memory",
            "memoryKind": kind,
            "operatorId": operator_id,
            "memoryId": mem.id.map(|o| o.to_hex()).unwrap_or_default(),
        },
    )
    .await;
    Ok(json!({
        "naturalReply": format!("{summary}。AI 会在下次起草时遵守这条偏好；如需撤销请直接告诉我。"),
        "missingFields": Vec::<String>::new(),
        "followupQuestions": Vec::<Value>::new(),
        "operatorMemory": {
            "id": mem.id.map(|o| o.to_hex()),
            "kind": mem.kind,
            "content": mem.content,
        }
    }))
}

async fn revoke_operator_memory_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    operator_id: &str,
    intent_result: &Value,
) -> AppResult<Value> {
    let memory_id = intent_result
        .get("memoryId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(memory_id) = memory_id else {
        return Ok(json!({
            "naturalReply": "请把要撤销的 memoryId 发给我，我只会撤销该账号下属于您的那一条运营记忆。",
            "missingFields": ["memoryId"],
            "followupQuestions": [{
                "id": "q1",
                "field": "memoryId",
                "question": "要撤销哪一条运营记忆？请提供 memoryId。",
            }],
        }));
    };
    let object_id = ObjectId::parse_str(memory_id)
        .map_err(|_| AppError::BadRequest("memoryId is invalid".to_string()))?;
    let reason = intent_result
        .get("revocationReason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("operator requested revocation");
    let outcome = agent::revoke_operator_memory(
        &state.db,
        workspace_id,
        account_id,
        operator_id,
        object_id,
        operator_id,
        reason,
    )
    .await?;
    let memory = outcome.memory;
    let summary = if outcome.already_revoked {
        format!("运营记忆 {} 已经撤销，无需重复处理", memory_id)
    } else {
        format!(
            "已撤销运营记忆 {}：{}",
            memory_id,
            truncate_for_prompt(&memory.content, 80)
        )
    };
    if !outcome.already_revoked {
        record_repair_event(
            state,
            workspace_id,
            account_id,
            "knowledge_operator_memory_revoked",
            summary.clone(),
            doc! {
                "kind": "operator_memory",
                "memoryId": memory_id,
                "operatorId": operator_id,
                "revocationReason": reason,
            },
        )
        .await;
    }
    Ok(json!({
        "naturalReply": format!("{summary}。这条内容不会再注入后续起草。"),
        "missingFields": Vec::<String>::new(),
        "followupQuestions": Vec::<Value>::new(),
        "operatorMemory": {
            "id": memory.id.map(|id| id.to_hex()),
            "kind": memory.kind,
            "content": memory.content,
            "revokedAt": memory.revoked_at.and_then(|value| value.try_to_rfc3339_string().ok()),
            "alreadyRevoked": outcome.already_revoked,
        }
    }))
}

async fn classify_intent(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    chunk_attached: Option<&str>,
    item_attached: Option<&str>,
    history: &[KnowledgeChatTurn],
    operator_memory_header: &str,
) -> AppResult<Value> {
    let system_base = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.intent",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，仅识别意图。只输出 JSON: {intent, confidence, targetChunkId?, targetPackId?, memoryKind?, memoryContent?, memoryId?, revocationReason?, userIntentSummary}.".to_string()
    });
    let system = if operator_memory_header.is_empty() {
        system_base
    } else {
        format!("{system_base}\n\n{operator_memory_header}")
    };
    let user = format!(
        r#"运营本轮输入：
{user_content}

引用的 chunkId（可能为空）：{}
引用的 packId（可能为空）：{}

最近历史（最多 6 条）：
{}

请输出 JSON，intent 必须在 [create_chunk, update_chunk, clarify_chunk, digest_action, update_operator_memory, revoke_operator_memory, freeform] 中。运营要求撤销记忆时必须选择 revoke_operator_memory，并从原话提取 memoryId；不要把撤销请求误判为新增记忆。"#,
        chunk_attached.unwrap_or("(无)"),
        item_attached.unwrap_or("(无)"),
        render_chat_history_for_prompt(history),
    );
    let run_id = format!("chat-{session_id}-intent");
    agent::generate_agent_json(
        state,
        workspace_id,
        Some(account_id),
        None,
        Some(&run_id),
        "knowledge.chat.intent",
        &system,
        &user,
    )
    .await
}

async fn draft_chunk_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    target_pack_id: Option<&str>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.draft_chunk",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，起草新切片草稿。只输出 JSON: {patch, missingFields, followupQuestions, naturalReply}.".to_string()
    });
    // operation_knowledge_items 已删除；catalog/pack_payload 永远为空。
    let catalog: Vec<Value> = vec![];
    let _ = target_pack_id;
    let pack_payload = Value::Null;
    let user = format!(
        r#"运营本轮输入：
{user_content}

知识库已有 pack catalog（≤ 10）：
{}

运营引用的 pack（可能为空）：
{}

最近历史（最多 6 条）：
{}

起草要求：
- patch 必须把运营本轮明确点名要起草的字段全部填上——运营若说「起草标题、摘要和正文」，patch 就必须同时含非空的 title、summary、body 三者，缺任何一个都算答非所问。
- body（正文）是切片的实体内容，承载可验证事实，绝不能因为它最长就省略或留空；其余字段齐全而独缺 body 视为未完成起草。
- 信息确实不足以填某字段时，把该字段名写进 missingFields 并用 followupQuestions 向运营追问，而不是静默丢弃运营已点名的字段。
- naturalReply 必填、不可留空：用对话口吻向运营回报你起草了什么、还差什么，这是给人看的回执，不能只产 patch 就沉默。回执要展示关键产出本身（如把拟定的标题、摘要要点直接说出来），而不是只声明「我起草了标题/摘要」这类字段名——让运营不必去翻 patch 就能判断对不对；仍缺的字段则顺带引导补全。

请按 system 中 schema 输出 JSON 起草一条新切片草稿。"#,
        serde_json::to_string_pretty(&catalog).unwrap_or_default(),
        serde_json::to_string_pretty(&pack_payload).unwrap_or_default(),
        render_chat_history_for_prompt(history),
    );
    let augmented_system = augment_chat_system_with_tools(&system);
    run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "draft",
        "knowledge.chat.draft_chunk",
        augmented_system,
        user,
    )
    .await
}

async fn update_chunk_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    chunk_id: &str,
    expected_updated_at: Option<&str>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let oid = parse_object_id(chunk_id)?;
    let mut chunk_filter = doc! {
        "_id": oid,
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "$or": [
            { "account_id": null },
            { "account_id": account_id },
        ],
    };
    if let Some(expected) = expected_updated_at {
        chunk_filter.insert("updated_at", parse_expected_chunk_updated_at(expected)?);
    }
    let chunk = state
        .db
        .operation_knowledge_chunks()
        .find_one(chunk_filter, None)
        .await?
        .ok_or_else(|| AppError::Conflict("chat_chunk_snapshot_stale".to_string()))?;
    let frozen_updated_at = chunk.updated_at.try_to_rfc3339_string().map_err(|error| {
        AppError::External(format!("serialize chunk updated_at failed: {error}"))
    })?;
    let document_payload = if let Some(document_id) = chunk.document_id {
        state
            .db
            .operation_knowledge_documents()
            .find_one(
                doc! {
                    "_id": document_id,
                    "workspace_id": workspace_id,
                    "domain": "user_operations",
                    "$or": [
                        { "account_id": null },
                        { "account_id": account_id },
                    ],
                },
                None,
            )
            .await?
            .map(|d| {
                json!({
                    "title": d.title,
                    "rawText": truncate_for_prompt(d.raw_content.as_deref().unwrap_or(""), 4000),
                })
            })
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.update_chunk",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，按运营对话给出已选切片的修改 patch。只输出 JSON: {patch, missingFields, followupQuestions, naturalReply}.".to_string()
    });
    let user = format!(
        r#"运营本轮输入：
{user_content}

待修改切片当前内容：
{}

父文档（可能为空，已截断到 4000 字）：
{}

最近历史（最多 6 条）：
{}

请仅对运营提到的字段做改动；其它字段省略。"#,
        serde_json::to_string_pretty(&operation_knowledge_chunk_json(chunk.clone()))
            .unwrap_or_default(),
        serde_json::to_string_pretty(&document_payload).unwrap_or_default(),
        render_chat_history_for_prompt(history),
    );
    let augmented_system = augment_chat_system_with_tools(&system);
    let mut result = run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "update",
        "knowledge.chat.update_chunk",
        augmented_system,
        user,
    )
    .await?;
    result["expectedUpdatedAt"] = json!(frozen_updated_at);
    Ok(result)
}

async fn clarify_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.chat.clarify",
    )
    .await
    .unwrap_or_else(|_| {
        "你是知识库对话 Agent，做澄清回答。只输出 JSON: {naturalReply, askMoreField?, askMoreQuestion?, nextSuggestion?}.".to_string()
    });
    let user = format!(
        r#"运营本轮输入：
{user_content}

最近历史（最多 6 条）：
{}

请按 system 中 schema 输出 JSON。"#,
        render_chat_history_for_prompt(history),
    );
    let augmented_system = augment_chat_system_with_tools(&system);
    run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "clarify",
        "knowledge.chat.clarify",
        augmented_system,
        user,
    )
    .await
}

/// knowledge-digest-workstation Phase 4 / Task #360：
/// 把运营从今日日报勾出的一组卡片转成 `plannedSteps` 序列。
///
/// 调 `knowledge.digest.dispatch` PromptSpec；输入是当日 cards 摘要 + 运营本轮文字；
/// 输出含 `plannedSteps[] / estimatedLlmCalls / naturalReply`，由前端拿到后弹「派工
/// 确认」小卡，确认后再 POST `/api/knowledge/chat/tasks` 落 `KnowledgeChatTask`。
///
/// 与 update_chunk_for_chat 不同：本路径不出 patch、不直接落库，仅是步骤计划。
async fn dispatch_digest_action_for_chat(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    session_id: &str,
    user_content: &str,
    digest_selection: Option<&DigestSelectionBinding>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    if let Some(selection) = digest_selection {
        if selection.account_id != account_id || selection.report_date != report_date {
            return Err(AppError::Conflict(
                "digest_dispatch_snapshot_stale".to_string(),
            ));
        }
    }
    let report = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("digest_dispatch_report_missing".to_string()))?;

    let binding = if let Some(selection) = digest_selection {
        selection.clone()
    } else {
        let system = prompts::load_prompt(
            &state.db,
            workspace_id,
            "knowledge.digest.dispatch",
        )
        .await
        .unwrap_or_else(|_| {
            "你是 AI 调度器，从候选卡片中挑选与运营要求相符的 cardId。只输出 JSON: {plannedSteps:[{cardId}], naturalReply}.".to_string()
        });
        let card_summaries = report
            .cards
            .iter()
            .filter(|card| !report.dismissed_card_ids.contains(&card.card_id))
            .take(20)
            .map(|card| {
                json!({
                    "cardId": card.card_id.to_hex(),
                    "kind": card.kind,
                    "title": card.title,
                    "summary": card.summary,
                    "suggestedAction": card.suggested_action,
                    "severity": card.severity,
                })
            })
            .collect::<Vec<_>>();
        let user = format!(
            r#"运营本轮输入：
{user_content}

今日日报候选卡片（最多 20 条，未被 dismiss）：
{cards}

最近历史（最多 6 条）：
{history}

只挑选确实匹配的 cardId，最多 8 张。action、summary、target 不由你决定。"#,
            cards =
                serde_json::to_string_pretty(&card_summaries).unwrap_or_else(|_| "[]".to_string()),
            history = render_chat_history_for_prompt(history),
        );
        let run_id = format!("chat-{session_id}-dispatch");
        let llm_value = agent::generate_agent_json(
            state,
            workspace_id,
            Some(account_id),
            None,
            Some(&run_id),
            "knowledge.digest.dispatch",
            &system,
            &user,
        )
        .await?;
        let allowed = report
            .cards
            .iter()
            .filter(|card| !report.dismissed_card_ids.contains(&card.card_id))
            .map(|card| (card.card_id.to_hex(), card))
            .collect::<std::collections::HashMap<_, _>>();
        let mut seen = std::collections::HashSet::new();
        let selected_cards = llm_value
            .get("plannedSteps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|step| step.get("cardId").and_then(Value::as_str))
            .filter(|card_id| seen.insert((*card_id).to_string()))
            .filter_map(|card_id| {
                allowed.get(card_id).map(|card| DigestSelectedCardBinding {
                    card_id: card_id.to_string(),
                    card_hash: crate::knowledge_digest::digest_card_snapshot_hash(card),
                })
            })
            .take(8)
            .collect::<Vec<_>>();
        if selected_cards.is_empty() {
            return Ok(json!({
                "naturalReply": llm_value
                    .get("naturalReply")
                    .and_then(Value::as_str)
                    .unwrap_or("没有找到可安全绑定的日报卡片，请先在今日摘要中勾选后再派工。"),
                "plannedSteps": [],
                "estimatedLlmCalls": 0,
            }));
        }
        DigestSelectionBinding {
            account_id: account_id.to_string(),
            report_id: report
                .id
                .ok_or_else(|| AppError::Conflict("digest_report_identity_missing".to_string()))?
                .to_hex(),
            report_date: report.report_date.clone(),
            report_generation: report.current_generation,
            report_hash: crate::knowledge_digest::digest_report_snapshot_hash(&report),
            selected_cards,
        }
    };

    let resolved = resolve_digest_selection(&report, &binding)?;
    let planned_steps = resolved
        .steps
        .iter()
        .map(|step| serde_json::to_value(step).unwrap_or_else(|_| json!({})))
        .collect::<Vec<_>>();
    Ok(json!({
        "naturalReply": format!(
            "已按当前日报锁定 {} 张卡片及其执行目标，请确认后派工。",
            planned_steps.len()
        ),
        "plannedSteps": planned_steps,
        "estimatedLlmCalls": planned_steps.len(),
        "digestSelection": binding,
        "candidateHash": resolved.candidate_hash,
    }))
}

/// create / update 两条对话补库路径共用的「运营陈述 → sourceQuote → source_anchors」
/// 锚定规则（D2 红线核心：quote 与 anchor 必须成对，绝不出现 quote 改了 anchor 没跟上
/// 的失配）。抽成纯函数以消除两处重复实现的 drift 风险，并可在本地 lib 单测里锁死不变量
/// （`apply_*_chunk` 本身是 async+db，只有 CI 集成测试能跑）。
///
/// - `statement`：运营在会话里的口头陈述（溯源原文）。调用方传入前无需 trim。
/// - `patch_quote`：LLM patch 给出的候选 sourceQuote（create 来自 payload，update 来自
///   update_doc）。
///
/// 返回 `quote`：`Some` 表示应把 chunk 的 sourceQuote 改写为该值；`None` 表示**不改动**
/// 现有 quote（仅当 statement 为空——无出处可溯源时）。返回 `anchors`：对最终 quote 的
/// 锚定结果（锚不上则空，让 D2 verify 闸合法拒绝，绝不放水）。
struct QuoteAnchorResolution {
    quote: Option<String>,
    anchors: Vec<Document>,
}

fn resolve_quote_anchors(statement: &str, patch_quote: Option<&str>) -> QuoteAnchorResolution {
    let statement = statement.trim();
    if statement.is_empty() {
        // 没有运营陈述可溯源：维持原 quote 不动、清空 anchors，verify 仍按 D2 合法拒绝。
        return QuoteAnchorResolution {
            quote: None,
            anchors: vec![],
        };
    }
    // 优先采用 patch 给出的 sourceQuote（若能在运营陈述中锚定），否则回退用运营陈述全文
    // 作为 quote。这样 D2 verify 闸（sourceQuote + source_anchors 双非空）才能"凭真实出处"
    // 合法通过——是补齐溯源，而非削弱闸门。
    let quote = patch_quote
        .map(str::trim)
        .filter(|q| !q.is_empty() && source_anchor_for_quote(statement, None, q).is_some())
        .map(|q| q.to_string())
        .unwrap_or_else(|| statement.to_string());
    let anchors = source_anchor_for_quote(statement, None, &quote)
        .map(|d| vec![d])
        .unwrap_or_default();
    QuoteAnchorResolution {
        quote: Some(quote),
        anchors,
    }
}

pub async fn apply_create_chunk(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    session_id: &str,
    patch: &Document,
    target_pack_id: Option<&str>,
    operator_statement: &str,
) -> AppResult<Value> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result = apply_create_chunk_with_session(
        state,
        workspace_id,
        account_id,
        session_id,
        patch,
        target_pack_id,
        operator_statement,
        &mut session,
    )
    .await;
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction(&mut session).await?;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_create_chunk_with_session(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    session_id: &str,
    patch: &Document,
    target_pack_id: Option<&str>,
    operator_statement: &str,
    session: &mut ClientSession,
) -> AppResult<Value> {
    let patch_value: Value = mongodb::bson::Bson::Document(patch.clone()).into();
    let mut payload = chunk_request_from_chat_patch(&patch_value, account_id, target_pack_id);
    // 强制：AI 永不自动 verify
    payload.status = "draft".to_string();
    payload.integrity_status = Some("needs_review".to_string());

    // chat 新建的知识没有父文档，溯源 = 运营在会话里的口头陈述本身。锚定规则与
    // apply_update_chunk 共用 resolve_quote_anchors（见其文档）。
    let resolution = resolve_quote_anchors(operator_statement, payload.source_quote.as_deref());
    payload.source_anchors = resolution.anchors;
    if let Some(quote) = resolution.quote {
        payload.source_quote = Some(quote);
    }

    validate_operation_knowledge_chunk(&payload)?;
    let chunk_id = ObjectId::new();
    let chunk =
        operation_knowledge_chunk_from_request(state, workspace_id, payload, Some(chunk_id))?;
    state
        .db
        .operation_knowledge_chunks()
        .insert_one_with_session(chunk, None, session)
        .await?;
    let applied = crate::knowledge_wiki::chunk_revisions::apply_chunk_revision_with_session(
        &state.db,
        workspace_id,
        chunk_id,
        crate::knowledge_wiki::chunk_revisions::RevisionRequest {
            op: crate::knowledge_wiki::chunk_revisions::RevisionOp::Create,
            source: crate::knowledge_wiki::chunk_revisions::ProvenanceSource::Ai,
            patch: Document::new(),
            reason: Some("知识对话创建草稿".to_string()),
            actor: Some("knowledge_chat".to_string()),
        },
        session,
    )
    .await?;
    Ok(json!({
        "createdChunkId": chunk_id.to_hex(),
        "revisionId": applied.revision_id,
        "sessionId": session_id,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
}

pub(crate) async fn apply_update_chunk(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    chunk_id: &str,
    patch: &Document,
    operator_statement: &str,
) -> AppResult<Value> {
    let mut session = state.db.client().start_session(None).await?;
    session.start_transaction(None).await?;
    let result = apply_update_chunk_with_session(
        state,
        workspace_id,
        account_id,
        chunk_id,
        patch,
        operator_statement,
        None,
        &mut session,
    )
    .await;
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = session.abort_transaction().await;
            return Err(error);
        }
    };
    crate::knowledge_wiki::chunk_revisions::commit_chunk_transaction(&mut session).await?;
    Ok(value)
}

pub(crate) async fn apply_update_chunk_with_session(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    chunk_id: &str,
    patch: &Document,
    operator_statement: &str,
    expected_updated_at: Option<&str>,
    session: &mut ClientSession,
) -> AppResult<Value> {
    let oid = parse_object_id(chunk_id)?;
    let mut chunk_filter = doc! {
        "_id": oid,
        "workspace_id": workspace_id,
        "domain": "user_operations",
        "$or": [
            { "account_id": null },
            { "account_id": account_id },
        ],
    };
    if let Some(expected) = expected_updated_at {
        chunk_filter.insert("updated_at", parse_expected_chunk_updated_at(expected)?);
    }
    state
        .db
        .operation_knowledge_chunks()
        .find_one_with_session(chunk_filter, None, session)
        .await?
        .ok_or_else(|| AppError::Conflict("chat_chunk_snapshot_stale".to_string()))?;
    let mut update_doc = Document::new();
    for key in [
        "title",
        "summary",
        "routing_card",
        "applicable_scenes",
        "not_applicable_scenes",
        "safe_claims",
        "forbidden_claims",
        "evidence_items",
        "product_tags",
        "business_topics",
        "source_quote",
    ]
    .iter()
    {
        // patch 用 camelCase；映射到 storage 的 snake_case。
        let camel = match *key {
            "routing_card" => "routingCard",
            "applicable_scenes" => "applicableScenes",
            "not_applicable_scenes" => "notApplicableScenes",
            "safe_claims" => "safeClaims",
            "forbidden_claims" => "forbiddenClaims",
            "evidence_items" => "evidenceItems",
            "product_tags" => "productTags",
            "business_topics" => "businessTopics",
            "source_quote" => "sourceQuote",
            other => other,
        };
        if let Some(val) = patch.get(camel) {
            update_doc.insert(*key, val.clone());
        }
    }
    if update_doc.is_empty() {
        return Ok(json!({
            "updatedChunkId": chunk_id,
            "fieldsTouched": 0,
            "note": "patch 没有可识别字段，未改动",
        }));
    }
    // 运营经 patch 实际改动的字段数（在追加派生 anchors / 元字段之前定格）。
    let fields_touched = update_doc.len();

    // 若本次 patch 改了 sourceQuote，必须同步重算 source_anchors（与 apply_create_chunk
    // 共用 resolve_quote_anchors）：新 quote 配新 anchor，杜绝"新 quote + 旧 anchor"失配后
    // 仍被 D2 verify 闸（仅校验 anchors 非空）放行的隐患。锚不上就写空 anchors，让
    // D2 合法拒绝 re-verify——这是补齐溯源，绝不削弱闸门。
    if update_doc.contains_key("source_quote") {
        let patch_quote = update_doc.get_str("source_quote").ok();
        let resolution = resolve_quote_anchors(operator_statement, patch_quote);
        update_doc.insert("source_anchors", resolution.anchors);
        if let Some(quote) = resolution.quote {
            update_doc.insert("source_quote", quote);
        }
    }

    // KB-09：落库改走统一入口 apply_chunk_revision（op=Patch, source=Ai）——获 chunk_revisions
    // 审计行 + 数组字段 union（既有 tag 不被整体替换丢弃）+ locked_fields 守门（KB-11）；
    // source=Ai 自动强制 status=draft + integrity_status=needs_review（"AI 永不自动 verify"红线不破）。
    // update_doc 已含 patch 前重算的 source_anchors（复数,不撞 DEFAULT 锁的 source_anchor 单数）。
    let applied = crate::knowledge_wiki::chunk_revisions::apply_chunk_revision_with_session(
        &state.db,
        workspace_id,
        oid,
        crate::knowledge_wiki::chunk_revisions::RevisionRequest {
            op: crate::knowledge_wiki::chunk_revisions::RevisionOp::Patch,
            source: crate::knowledge_wiki::chunk_revisions::ProvenanceSource::Ai,
            patch: update_doc,
            reason: Some("知识对话应用草稿".to_string()),
            actor: Some("knowledge_chat".to_string()),
        },
        session,
    )
    .await?;
    Ok(json!({
        "updatedChunkId": chunk_id,
        "revisionId": applied.revision_id,
        "fieldsTouched": fields_touched,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
}

/// 把 chat 产出的 patch（camelCase JSON）转成 OperationKnowledgeChunkRequest。
/// 缺字段补默认值；让后端的 apply_chunk_integrity 在写入路径上重算 anchor。
fn chunk_request_from_chat_patch(
    patch: &Value,
    account_id: Option<&str>,
    pack_id: Option<&str>,
) -> OperationKnowledgeChunkRequest {
    fn s(v: &Value, k: &str) -> Option<String> {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    fn arr(v: &Value, k: &str) -> Vec<String> {
        v.get(k)
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
    OperationKnowledgeChunkRequest {
        account_id: account_id.map(|s| s.to_string()),
        document_id: None,
        item_id: pack_id.map(|s| s.to_string()),
        domain: "user_operations".to_string(),
        knowledge_type: s(patch, "knowledgeType"),
        business_context: s(patch, "businessContext"),
        title: s(patch, "title").unwrap_or_else(|| "AI 对话产物（草稿）".to_string()),
        summary: s(patch, "summary"),
        body: s(patch, "body"),
        applicable_scenes: arr(patch, "applicableScenes"),
        not_applicable_scenes: arr(patch, "notApplicableScenes"),
        product_tags: arr(patch, "productTags"),
        business_topics: arr(patch, "businessTopics"),
        source_quote: s(patch, "sourceQuote"),
        source_anchors: vec![],
        integrity_status: Some("needs_review".to_string()),
        confidence_score: None,
        distortion_risks: vec![],
        status: "draft".to_string(),
        priority: 0,
        wiki_type: s(patch, "wikiType"),
        chunk_type: s(patch, "chunkType"),
    }
}

// ── knowledge-digest-workstation Phase 4：chat 长任务 + SSE ──────────────────

/// `POST /api/knowledge/chat/tasks`：把 chat dispatch 出的 plannedSteps 落库为
/// `knowledge_chat_tasks{status="pending"}`，由 `KnowledgeTaskWorker` 串行执行。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChatTaskCreateRequest {
    pub session_id: String,
    pub account_id: Option<String>,
    pub operator_id: Option<String>,
    pub digest_selection: DigestSelectionBinding,
    #[serde(default)]
    pub source_turn_index: Option<i32>,
    #[serde(default)]
    pub candidate_hash: Option<String>,
    // Legacy wire fields are accepted only for consistency checks. They are
    // never used as the source of action, summary, report date, or target.
    #[serde(default)]
    pub card_ids: Vec<String>,
    #[serde(default)]
    pub planned_steps: Vec<Value>,
}

/// 从 digest 卡片 target_refs 取第一个 kind=="chunk" 的非空 id。
/// 用于派工落库时把 cardId 解析成 step.targetChunkId（fix_chunk/retag 需要）。
pub(in crate::routes) fn extract_chunk_ref(
    target_refs: &[mongodb::bson::Document],
) -> Option<String> {
    for r in target_refs {
        if r.get_str("kind").ok() == Some("chunk") {
            if let Ok(id) = r.get_str("id") {
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

#[derive(Debug)]
struct ResolvedDigestSelection {
    cards: Vec<crate::models::KnowledgeDigestCard>,
    steps: Vec<Document>,
    candidate_hash: String,
}

fn digest_candidate_hash(binding: &DigestSelectionBinding, steps: &[Document]) -> String {
    use sha2::{Digest, Sha256};

    let canonical = json!({
        "accountId": binding.account_id,
        "reportId": binding.report_id,
        "reportDate": binding.report_date,
        "reportGeneration": binding.report_generation,
        "reportHash": binding.report_hash,
        "selectedCards": binding.selected_cards.iter().map(|selected| json!({
            "cardId": selected.card_id,
            "cardHash": selected.card_hash,
        })).collect::<Vec<_>>(),
        "plannedSteps": steps,
    });
    let bytes = serde_json::to_vec(&canonical).expect("dispatch candidate canonical JSON");
    hex::encode(Sha256::digest(bytes))
}

fn resolve_digest_selection(
    report: &crate::models::KnowledgeDailyReport,
    binding: &DigestSelectionBinding,
) -> AppResult<ResolvedDigestSelection> {
    let report_id = report
        .id
        .ok_or_else(|| AppError::Conflict("digest_report_identity_missing".to_string()))?;
    if binding.account_id != report.account_id
        || binding.report_id != report_id.to_hex()
        || binding.report_date != report.report_date
        || binding.report_generation != report.current_generation
        || binding.report_hash != crate::knowledge_digest::digest_report_snapshot_hash(report)
    {
        return Err(AppError::Conflict(
            "digest_dispatch_snapshot_stale".to_string(),
        ));
    }
    if binding.selected_cards.is_empty() || binding.selected_cards.len() > 8 {
        return Err(AppError::BadRequest(
            "selectedCards 必须包含 1..=8 张卡片".to_string(),
        ));
    }

    let mut seen = std::collections::HashSet::new();
    let mut cards = Vec::with_capacity(binding.selected_cards.len());
    let mut steps = Vec::with_capacity(binding.selected_cards.len());
    for (idx, selected) in binding.selected_cards.iter().enumerate() {
        if !seen.insert(selected.card_id.clone()) {
            return Err(AppError::BadRequest(
                "selectedCards.cardId 不得重复".to_string(),
            ));
        }
        let card_id = ObjectId::parse_str(&selected.card_id).map_err(|_| {
            AppError::BadRequest(format!("invalid selected cardId: {}", selected.card_id))
        })?;
        let card = report
            .cards
            .iter()
            .find(|card| card.card_id == card_id)
            .ok_or_else(|| AppError::Conflict("digest_dispatch_card_missing".to_string()))?;
        if report.dismissed_card_ids.contains(&card_id) {
            return Err(AppError::Conflict(
                "digest_dispatch_card_dismissed".to_string(),
            ));
        }
        if selected.card_hash != crate::knowledge_digest::digest_card_snapshot_hash(card) {
            return Err(AppError::Conflict(
                "digest_dispatch_card_changed".to_string(),
            ));
        }
        if card.suggested_action == "freeform" {
            return Err(AppError::BadRequest(format!(
                "card {} is not dispatchable",
                selected.card_id
            )));
        }
        if ![
            "fix_chunk",
            "add_chunk",
            "retag",
            "review_evolution",
            "dismiss",
        ]
        .contains(&card.suggested_action.as_str())
        {
            return Err(AppError::BadRequest(format!(
                "card {} has unsupported suggestedAction={}",
                selected.card_id, card.suggested_action
            )));
        }

        let mut step = doc! {
            "stepId": format!("step_{}", idx + 1),
            "cardId": &selected.card_id,
            "action": &card.suggested_action,
            "summary": &card.summary,
            "reportDate": &report.report_date,
        };
        if let Some(chunk_id) = extract_chunk_ref(&card.target_refs) {
            step.insert("targetChunkId", chunk_id);
        }
        cards.push(card.clone());
        steps.push(step);
    }

    Ok(ResolvedDigestSelection {
        cards,
        candidate_hash: digest_candidate_hash(binding, &steps),
        steps,
    })
}

fn legacy_dispatch_payload_matches(
    card_ids: &[String],
    planned_steps: &[Value],
    resolved: &ResolvedDigestSelection,
) -> AppResult<()> {
    let authoritative_ids = resolved
        .steps
        .iter()
        .filter_map(|step| step.get_str("cardId").ok())
        .collect::<Vec<_>>();
    if !card_ids.is_empty()
        && card_ids.iter().map(String::as_str).collect::<Vec<_>>() != authoritative_ids
    {
        return Err(AppError::Conflict(
            "digest_dispatch_card_selection_mismatch".to_string(),
        ));
    }
    if planned_steps.is_empty() {
        return Ok(());
    }
    if planned_steps.len() != resolved.steps.len() {
        return Err(AppError::Conflict(
            "digest_dispatch_step_count_mismatch".to_string(),
        ));
    }
    for (idx, (provided, authoritative)) in
        planned_steps.iter().zip(resolved.steps.iter()).enumerate()
    {
        for key in ["cardId", "action", "targetChunkId"] {
            let provided_value = provided.get(key).and_then(Value::as_str);
            let authoritative_value = authoritative.get_str(key).ok();
            if provided_value.is_some() && provided_value != authoritative_value {
                return Err(AppError::Conflict(format!(
                    "plannedSteps[{idx}].{key} does not match selected digest card"
                )));
            }
        }
    }
    Ok(())
}

pub(in crate::routes) async fn chat_task_create(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<ChatTaskCreateRequest>,
) -> AppResult<Json<Value>> {
    let session_id = body.session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::BadRequest("sessionId 不能为空".to_string()));
    }
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    ensure_chat_session_identity(
        &state,
        &admin.current_workspace,
        &account_id,
        session_id,
        &admin.user_id,
    )
    .await?;

    if body.digest_selection.account_id != account_id {
        return Err(AppError::Conflict(
            "digest_dispatch_account_mismatch".to_string(),
        ));
    }

    // SR-125：任务创建重新读取当前权威日报，并由选中的 card 快照重建步骤。
    // 客户端传来的 plannedSteps/cardIds 仅做一致性检查，绝不作为写入来源。
    let report = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &body.digest_selection.report_date,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::Conflict("digest_dispatch_report_missing".to_string()))?;
    let resolved = resolve_digest_selection(&report, &body.digest_selection)?;
    legacy_dispatch_payload_matches(&body.card_ids, &body.planned_steps, &resolved)?;

    if let Some(provided_hash) = body.candidate_hash.as_deref() {
        if provided_hash != resolved.candidate_hash {
            return Err(AppError::Conflict(
                "digest_dispatch_candidate_changed".to_string(),
            ));
        }
    }

    // Chat 确认路径必须回读原 assistant turn 中的服务端候选封印。画布直派没有
    // sourceTurnIndex，但仍受 report/currentGeneration/reportHash/cardHash 全套约束。
    if let Some(source_turn_index) = body.source_turn_index {
        let provided_hash = body.candidate_hash.as_deref().ok_or_else(|| {
            AppError::BadRequest(
                "candidateHash is required when sourceTurnIndex is provided".to_string(),
            )
        })?;
        let source_turn = state
            .db
            .knowledge_chat_turns()
            .find_one(
                doc! {
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id,
                    "session_id": session_id,
                    "turn_index": source_turn_index,
                    "role": "assistant",
                    "intent": "digest_action",
                },
                None,
            )
            .await?
            .ok_or_else(|| AppError::Conflict("digest_dispatch_source_turn_missing".to_string()))?;
        let sealed = source_turn.attachments.iter().any(|attachment| {
            attachment.get_str("kind").ok() == Some("digest_dispatch_candidate")
                && attachment.get_str("candidateHash").ok() == Some(provided_hash)
        });
        if !sealed {
            return Err(AppError::Conflict(
                "digest_dispatch_candidate_unsealed".to_string(),
            ));
        }
    }

    let binding_doc = bson_from_json(&serde_json::to_value(&body.digest_selection)?)
        .map_err(|error| AppError::Conflict(format!("digest_dispatch_binding_invalid: {error}")))?;
    let mut dispatch_binding = doc! {
        "protocol": "digest_dispatch_v1",
        "candidateHash": &resolved.candidate_hash,
        "digestSelection": binding_doc,
    };
    if let Some(source_turn_index) = body.source_turn_index {
        dispatch_binding.insert("sourceTurnIndex", source_turn_index);
    }
    let total_steps = resolved.steps.len();

    let task_id = ObjectId::new();
    let task = crate::models::KnowledgeChatTask {
        id: Some(task_id),
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.clone(),
        session_id: session_id.to_string(),
        owner_admin_id: Some(admin.user_id.clone()),
        operator_id: body.operator_id.clone(),
        cards: resolved.cards,
        dispatch_binding: Some(dispatch_binding),
        planned_steps: resolved.steps,
        completed_steps: vec![],
        step_intents: vec![],
        status: "pending".to_string(),
        error_kind: None,
        attempts: 0,
        claim_generation: 0,
        worker_id: None,
        claim_token: None,
        locked_until: None,
        heartbeat_at: None,
        created_at: DateTime::now(),
        started_at: None,
        finished_at: None,
    };
    state
        .db
        .knowledge_chat_tasks()
        .insert_one(task, None)
        .await?;

    // 立刻写一条 task_progress turn 记录派工已落库。
    // P1-7：原子分配新 turn_index，避免与并发 chat_turn / worker 写入冲突。
    let next_index = allocate_next_turn_indices(
        &state,
        &admin.current_workspace,
        &account_id,
        session_id,
        &admin.user_id,
        1,
    )
    .await?;
    let turn = KnowledgeChatTurn {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.clone(),
        session_id: session_id.to_string(),
        turn_index: next_index,
        role: "system".to_string(),
        intent: Some("digest_action".to_string()),
        content: format!(
            "AI 已收到派工，taskId={}，共 {} 步，等待 worker 串行执行",
            task_id, total_steps
        ),
        attachments: vec![doc! { "taskId": task_id, "phase": "queued" }],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
        apply_result: None,
        applied_at: None,
        tokens_used: 0,
        prompt_key: None,
        kind: Some("task_progress".to_string()),
        tool_calls: vec![],
        created_at: DateTime::now(),
    };
    state
        .db
        .knowledge_chat_turns()
        .insert_one(turn, None)
        .await?;
    state
        .chat_progress_bus
        .bump(&chat_session_bus_key(
            &admin.current_workspace,
            &account_id,
            session_id,
        ))
        .await;

    Ok(Json(json!({
        "taskId": task_id.to_hex(),
        "sessionId": session_id,
        "status": "pending",
        "totalSteps": total_steps as i32,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChatTaskListQuery {
    pub account_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}

/// 任务列表 limit clamp：缺省 50，区间 [1, 200]。
fn clamp_task_list_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 200)
}

/// `GET /api/knowledge/chat/tasks`：列出本 workspace 的长任务（F21 任务总览）。
/// 可选 status 过滤（非法值忽略，与现有 chunk 列表 query 宽松风格一致）；
/// limit clamp [1,200] 默认 50；按 created_at 倒序。列表项不带 plannedSteps/cards
/// 全文（控 payload 体积），详情仍走 GET /tasks/:id。
pub(in crate::routes) async fn chat_task_list(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ChatTaskListQuery>,
) -> AppResult<Json<Value>> {
    let account_id = expected_chat_account(
        query.account_id.as_deref(),
        &state.config.default_account_id,
    );
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    let mut filter = doc! {
        "workspace_id": &admin.current_workspace,
        "account_id": &account_id,
        "owner_admin_id": &admin.user_id,
    };
    if let Some(status) = query.status.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("status", status.trim());
    }
    let limit = clamp_task_list_limit(query.limit);
    let mut cursor = state
        .db
        .knowledge_chat_tasks()
        .find(
            filter,
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        items.push(json!({
            "taskId": task.id.map(|i| i.to_hex()).unwrap_or_default(),
            "accountId": task.account_id,
            "sessionId": task.session_id,
            "status": task.status,
            "errorKind": task.error_kind,
            "totalSteps": task.planned_steps.len() as i32,
            "completedStepCount": task.completed_steps.len() as i32,
            "createdAt": task.created_at.to_string(),
            "startedAt": task.started_at.map(|d| d.to_string()),
            "finishedAt": task.finished_at.map(|d| d.to_string()),
        }));
    }
    Ok(Json(json!({ "items": items })))
}

/// `GET /api/knowledge/chat/tasks/:id`：查询 task 状态（前端 fallback 拉取）。
pub(in crate::routes) async fn chat_task_get(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id_hex): Path<String>,
    Query(query): Query<ChatSessionScopeQuery>,
) -> AppResult<Json<Value>> {
    let account_id = expected_chat_account(
        query.account_id.as_deref(),
        &state.config.default_account_id,
    );
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    let oid = ObjectId::parse_str(&id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid task id: {id_hex}")))?;
    let task = state
        .db
        .knowledge_chat_tasks()
        .find_one(
            doc! {
                "_id": oid,
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "owner_admin_id": &admin.user_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("knowledge_chat_task {id_hex} 不存在")))?;
    Ok(Json(json!({
        "taskId": task.id.map(|i| i.to_hex()).unwrap_or_default(),
        "accountId": task.account_id,
        "sessionId": task.session_id,
        "status": task.status,
        "errorKind": task.error_kind,
        "totalSteps": task.planned_steps.len() as i32,
        "completedSteps": serde_json::to_value(&task.completed_steps).unwrap_or(json!([])),
        "plannedSteps": serde_json::to_value(&task.planned_steps).unwrap_or(json!([])),
        "cards": serde_json::to_value(&task.cards).unwrap_or(json!([])),
        "createdAt": task.created_at.to_string(),
        "startedAt": task.started_at.map(|d| d.to_string()),
        "finishedAt": task.finished_at.map(|d| d.to_string()),
    })))
}

/// `POST /api/knowledge/chat/tasks/:id/cancel`：标 status="cancelled"；
/// worker 在每步开始前 re-read 状态，非 "running" 即停下。
///
/// P2-10：终态幂等——如果 task 已经是 completed / failed / cancelled，本接口
/// 返回 200 `{ ok: true, alreadyTerminated: true }` 而不是 404。理由：前端
/// 有可能在 task 刚 complete 的瞬间 race 一次 cancel，对运营来说"终态"是同一
/// 类语义；只有真正不存在的 task 才返回 404。
pub(in crate::routes) async fn chat_task_cancel(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id_hex): Path<String>,
    Query(query): Query<ChatSessionScopeQuery>,
) -> AppResult<Json<Value>> {
    let account_id = expected_chat_account(
        query.account_id.as_deref(),
        &state.config.default_account_id,
    );
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    let oid = ObjectId::parse_str(&id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid task id: {id_hex}")))?;
    let res = state
        .db
        .knowledge_chat_tasks()
        .update_one(
            doc! {
                "_id": oid,
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "owner_admin_id": &admin.user_id,
                "status": doc! { "$in": ["pending", "running"] }
            },
            doc! {
                "$set": {
                    "status": "cancelled",
                    "finished_at": DateTime::now(),
                },
                "$unset": {
                    "worker_id": "",
                    "claim_token": "",
                    "locked_until": "",
                    "heartbeat_at": "",
                },
            },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        // 未命中可能有两种：(a) task 真不存在；(b) task 已是终态。区分两种是
        // 因为运营前端在 cancel 后会 GET /tasks/:id 拿最终态——对终态返 404
        // 会让运营误以为派工记录丢失。
        let existing = state
            .db
            .knowledge_chat_tasks()
            .find_one(
                doc! {
                    "_id": oid,
                    "workspace_id": &admin.current_workspace,
                    "account_id": &account_id,
                    "owner_admin_id": &admin.user_id,
                },
                None,
            )
            .await?;
        match existing {
            None => {
                return Err(AppError::NotFound(format!(
                    "knowledge_chat_task {id_hex} 不存在"
                )));
            }
            Some(t) => {
                return Ok(Json(json!({
                    "ok": true,
                    "taskId": id_hex,
                    "status": t.status,
                    "alreadyTerminated": true,
                })));
            }
        }
    }
    Ok(Json(
        json!({ "ok": true, "taskId": id_hex, "status": "cancelled" }),
    ))
}

/// `GET /api/knowledge/chat/sessions/:sid/stream`：SSE 推送最新 turn_index。
/// 客户端按收到的 version 回拉 `chat_history` 拿增量 turn。
///
/// P1-6：watch 值为 [`crate::knowledge_task::CLOSE_SENTINEL`] 时，发一个
/// `close` event 后立即结束流（`return None`）。前端 EventSource 收到 close
/// 事件应主动关闭 + 不再重连，避免占用连接。
pub(in crate::routes) async fn chat_session_stream(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
    Query(query): Query<ChatSessionScopeQuery>,
) -> AppResult<
    axum::response::Sse<
        impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
> {
    use crate::knowledge_task::CLOSE_SENTINEL;
    use axum::response::sse::{Event, KeepAlive, Sse};
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let account_id = expected_chat_account(
        query.account_id.as_deref(),
        &state.config.default_account_id,
    );
    validate_account(&state, &admin.current_workspace, &account_id).await?;
    require_chat_session_identity(
        &state,
        &admin.current_workspace,
        &account_id,
        &session_id,
        &admin.user_id,
    )
    .await?;
    let bus_key = chat_session_bus_key(&admin.current_workspace, &account_id, &session_id);
    let rx = state.chat_progress_bus.subscribe(&bus_key).await;
    // 用 futures::stream::unfold 把 watch::Receiver 转成 SSE Stream，
    // 避免引入 tokio-stream 新依赖。state 是 (Receiver, closed) 元组——一旦
    // 推过 close event 就把 closed=true，下一次 poll 时直接 return None。
    let stream = futures::stream::unfold((rx, false), |(mut rx, closed)| async move {
        if closed {
            return None;
        }
        if rx.changed().await.is_err() {
            return None;
        }
        let v = *rx.borrow_and_update();
        if v == CLOSE_SENTINEL {
            // 终态：发一条 close 事件后下次循环立即 None。
            let event = Event::default().event("close").data("done");
            return Some((Ok::<_, std::convert::Infallible>(event), (rx, true)));
        }
        let event = Event::default().event("turn").data(v.to_string());
        Some((Ok::<_, std::convert::Infallible>(event), (rx, false)))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_attachment_accepts_frozen_camel_case_contract_only() {
        let attachment: ChatAttachment = serde_json::from_value(json!({
            "chunkId": "64a1f2c3e4b5a6978899aabb",
            "expectedUpdatedAt": "2026-07-27T03:00:00Z",
            "operation": "update"
        }))
        .expect("camelCase attachment");
        assert_eq!(
            attachment.chunk_id.as_deref(),
            Some("64a1f2c3e4b5a6978899aabb")
        );
        assert_eq!(
            attachment.expected_updated_at.as_deref(),
            Some("2026-07-27T03:00:00Z")
        );
        assert_eq!(attachment.operation, Some(ChatAttachmentOperation::Update));
        assert!(serde_json::from_value::<ChatAttachment>(json!({
            "chunk_id": "64a1f2c3e4b5a6978899aabb"
        }))
        .is_err());
        assert!(serde_json::from_value::<ChatAttachment>(json!({
            "chunkId": "64a1f2c3e4b5a6978899aabb",
            "operation": "create"
        }))
        .is_err());
        assert_eq!(
            forced_chat_intent(attachment.operation),
            Some("update_chunk")
        );
        assert_eq!(forced_chat_intent(None), None);
    }

    #[test]
    fn duplicate_key_codes_are_classified_without_widening_other_db_errors() {
        assert!(is_duplicate_key_code(11000));
        assert!(is_duplicate_key_code(11001));
        assert!(!is_duplicate_key_code(10999));
        assert!(!is_duplicate_key_code(112));
    }

    // resolve_quote_anchors 是 create / update 两条对话补库路径共用的 D2 锚定规则。
    // 这些单测把"quote 与 anchor 必须成对"的不变量钉死在本地 lib 层——`apply_*_chunk`
    // 本身是 async+db、只有 CI 集成测试能跑，纯函数抽出后这条红线终于有了本地安全网。

    #[test]
    fn resolve_blank_statement_keeps_quote_untouched_and_clears_anchors() {
        // 无运营陈述可溯源：不改现有 quote（quote=None），anchors 清空，让 D2 verify
        // 闸按"缺 anchor"合法拒绝——绝不放水。
        let r = resolve_quote_anchors("   ", Some("某个候选引文"));
        assert!(r.quote.is_none(), "statement 空时不应改写 quote");
        assert!(r.anchors.is_empty(), "statement 空时 anchors 必须为空");
    }

    #[test]
    fn resolve_uses_patch_quote_when_it_anchors_in_statement() {
        // patch 给的 quote 是运营陈述的子串 → 采用它，且 anchor 与该 quote 成对。
        let statement = "客户问能不能退款\n我们承诺七天无理由退款\n超过七天不退";
        let r = resolve_quote_anchors(statement, Some("七天无理由退款"));
        assert_eq!(r.quote.as_deref(), Some("七天无理由退款"));
        assert_eq!(r.anchors.len(), 1, "锚定成功必有 1 个 anchor");
        assert_eq!(
            r.anchors[0].get_str("sourceQuote").unwrap(),
            "七天无理由退款",
            "anchor 的 sourceQuote 必须等于返回的 quote（成对，不失配）"
        );
    }

    #[test]
    fn resolve_falls_back_to_full_statement_when_patch_quote_not_in_statement() {
        // patch 给的 quote 不在运营陈述里（锚不上）→ 回退用陈述全文作 quote，并对全文锚定。
        let statement = "我们承诺七天无理由退款";
        let r = resolve_quote_anchors(statement, Some("三十天无理由退款"));
        assert_eq!(
            r.quote.as_deref(),
            Some(statement),
            "patch_quote 锚不上时回退 statement 全文"
        );
        assert_eq!(r.anchors.len(), 1);
        assert_eq!(r.anchors[0].get_str("sourceQuote").unwrap(), statement);
    }

    #[test]
    fn resolve_falls_back_to_full_statement_when_no_patch_quote() {
        // patch 没给 quote → 直接用陈述全文。
        let statement = "我们承诺七天无理由退款";
        let r = resolve_quote_anchors(statement, None);
        assert_eq!(r.quote.as_deref(), Some(statement));
        assert_eq!(r.anchors.len(), 1);
        assert_eq!(r.anchors[0].get_str("sourceQuote").unwrap(), statement);
    }

    #[test]
    fn resolve_blank_patch_quote_falls_back_to_full_statement() {
        // patch 给了空白 quote → 视同没给，回退陈述全文。
        let statement = "我们承诺七天无理由退款";
        let r = resolve_quote_anchors(statement, Some("   "));
        assert_eq!(r.quote.as_deref(), Some(statement));
        assert_eq!(r.anchors.len(), 1);
    }

    #[test]
    fn resolve_d2_invariant_quote_and_anchor_always_paired() {
        // D2 核心不变量：只要返回了 quote（Some），就必有非空 anchors，且 anchor 的
        // sourceQuote 与返回 quote 严格一致。这正是 apply_update_chunk 当初漏写
        // source_anchors 重算所违反的不变量——本测试是它的回归网。
        let cases = [
            ("我们承诺七天无理由退款", Some("七天无理由退款")),
            ("我们承诺七天无理由退款", Some("不存在的引文")),
            ("我们承诺七天无理由退款", None),
        ];
        for (statement, patch_quote) in cases {
            let r = resolve_quote_anchors(statement, patch_quote);
            let quote = r.quote.expect("statement 非空必返回 quote");
            assert_eq!(
                r.anchors.len(),
                1,
                "返回 quote 时 anchors 必非空（statement={statement:?} patch={patch_quote:?}）"
            );
            assert_eq!(
                r.anchors[0].get_str("sourceQuote").unwrap(),
                quote,
                "anchor.sourceQuote 必须与返回 quote 成对一致"
            );
        }
    }

    #[test]
    fn clamp_task_list_limit_defaults_and_bounds() {
        assert_eq!(clamp_task_list_limit(None), 50, "缺省 50");
        assert_eq!(clamp_task_list_limit(Some(0)), 1, "下界 clamp 到 1");
        assert_eq!(clamp_task_list_limit(Some(-5)), 1, "负数 clamp 到 1");
        assert_eq!(clamp_task_list_limit(Some(10)), 10, "区间内原值");
        assert_eq!(clamp_task_list_limit(Some(9999)), 200, "上界 clamp 到 200");
    }

    #[test]
    fn tool_calling_raw_payload_is_replaced_by_explicit_final_fallback() {
        let raw = json!({
            "decisionPhase": "tool_calling",
            "toolCalls": [{ "tool": "knowledge.search", "arguments": { "query": "x" } }]
        });
        let payload = finalize_chat_tool_loop_payload(
            Some(raw),
            "final",
            &["chat_tool_loop_exhausted".to_string()],
        );
        assert_eq!(payload["decisionPhase"], "final");
        assert_eq!(payload["toolLoopTruncated"], true);
        assert_eq!(payload["toolLoopStopReason"], "loop_exhausted");
        assert!(payload.get("toolCalls").is_none());
    }

    #[test]
    fn genuine_final_raw_payload_is_preserved() {
        let raw =
            json!({ "decisionPhase": "final", "naturalReply": "done", "patch": { "title": "t" } });
        let payload = finalize_chat_tool_loop_payload(Some(raw.clone()), "final", &[]);
        assert_eq!(payload, raw);
    }

    #[test]
    fn revoked_operator_memory_is_never_rendered_for_chat_prompt() {
        let now = DateTime::from_millis(1);
        let revoked = crate::models::KnowledgeOperatorMemory {
            id: Some(ObjectId::new()),
            workspace_id: "ws-a".to_string(),
            account_id: "account-a".to_string(),
            operator_id: "operator-a".to_string(),
            kind: "rejection".to_string(),
            content: "不要使用旧模板".to_string(),
            created_at: now,
            last_used_at: now,
            expires_at: None,
            revoked_at: Some(now),
            revoked_by: Some("admin-a".to_string()),
            revocation_reason: Some("规则已过期".to_string()),
        };
        let active = crate::models::KnowledgeOperatorMemory {
            id: Some(ObjectId::new()),
            content: "回复保持简洁".to_string(),
            revoked_at: None,
            revoked_by: None,
            revocation_reason: None,
            ..revoked.clone()
        };

        let rendered = render_operator_memory_for_prompt(&[revoked, active]);
        assert!(!rendered.contains("不要使用旧模板"));
        assert!(rendered.contains("回复保持简洁"));
    }
}

#[cfg(test)]
mod dispatch_resolution_tests {
    use super::{
        extract_chunk_ref, legacy_dispatch_payload_matches, resolve_digest_selection,
        DigestSelectedCardBinding, DigestSelectionBinding,
    };
    use crate::models::{KnowledgeDailyReport, KnowledgeDigestCard};
    use mongodb::bson::{doc, oid::ObjectId, DateTime, Document};
    use serde_json::json;

    fn bound_report() -> (KnowledgeDailyReport, DigestSelectionBinding) {
        let report_id = ObjectId::new();
        let card_id = ObjectId::new();
        let card = KnowledgeDigestCard {
            card_id,
            kind: "chunk_missing_field".to_string(),
            title: "缺少出处".to_string(),
            summary: "为切片补充出处".to_string(),
            target_refs: vec![doc! { "kind": "chunk", "id": "chunk-authoritative" }],
            suggested_action: "fix_chunk".to_string(),
            severity: "warn".to_string(),
            metric: None,
        };
        let report = KnowledgeDailyReport {
            id: Some(report_id),
            workspace_id: "ws-a".to_string(),
            account_id: "account-a".to_string(),
            report_date: "2026-07-27".to_string(),
            generated_at: DateTime::now(),
            generated_by: "worker".to_string(),
            status: "ok".to_string(),
            error_kind: None,
            budget_snapshot: Document::new(),
            cards: vec![card.clone()],
            dismissed_card_ids: vec![],
            prompt_versions: Document::new(),
            attempt_generation: 3,
            current_generation: 3,
            latest_attempt_status: Some("ok".to_string()),
            latest_attempt_error_kind: None,
            latest_attempt_at: Some(DateTime::now()),
            latest_attempt_budget_snapshot: Document::new(),
            last_success_at: Some(DateTime::now()),
        };
        let binding = DigestSelectionBinding {
            account_id: report.account_id.clone(),
            report_id: report_id.to_hex(),
            report_date: report.report_date.clone(),
            report_generation: report.current_generation,
            report_hash: crate::knowledge_digest::digest_report_snapshot_hash(&report),
            selected_cards: vec![DigestSelectedCardBinding {
                card_id: card_id.to_hex(),
                card_hash: crate::knowledge_digest::digest_card_snapshot_hash(&card),
            }],
        };
        (report, binding)
    }

    #[test]
    fn extract_chunk_ref_returns_first_chunk_id() {
        let refs = vec![
            doc! { "kind": "pack", "id": "p1" },
            doc! { "kind": "chunk", "id": "c1" },
            doc! { "kind": "chunk", "id": "c2" },
        ];
        assert_eq!(extract_chunk_ref(&refs), Some("c1".to_string()));
    }

    #[test]
    fn extract_chunk_ref_none_when_no_chunk_ref() {
        let refs = vec![doc! { "kind": "pack", "id": "p1" }];
        assert_eq!(extract_chunk_ref(&refs), None);
    }

    #[test]
    fn extract_chunk_ref_skips_empty_id() {
        let refs = vec![
            doc! { "kind": "chunk", "id": "" },
            doc! { "kind": "chunk", "id": "c9" },
        ];
        assert_eq!(extract_chunk_ref(&refs), Some("c9".to_string()));
    }

    #[test]
    fn dispatch_steps_are_rebuilt_from_authoritative_digest_card() {
        let (report, binding) = bound_report();
        let resolved = resolve_digest_selection(&report, &binding).expect("valid binding");
        assert_eq!(resolved.steps.len(), 1);
        assert_eq!(resolved.steps[0].get_str("action").unwrap(), "fix_chunk");
        assert_eq!(
            resolved.steps[0].get_str("targetChunkId").unwrap(),
            "chunk-authoritative"
        );
        assert_eq!(
            resolved.steps[0].get_str("summary").unwrap(),
            "为切片补充出处"
        );
    }

    #[test]
    fn tampered_client_action_or_target_is_rejected() {
        let (report, binding) = bound_report();
        let resolved = resolve_digest_selection(&report, &binding).expect("valid binding");
        let tampered = vec![json!({
            "cardId": binding.selected_cards[0].card_id,
            "action": "dismiss",
            "targetChunkId": "chunk-attacker-controlled"
        })];
        let error = legacy_dispatch_payload_matches(&[], &tampered, &resolved)
            .expect_err("tampered client step must be rejected");
        assert!(error
            .to_string()
            .contains("does not match selected digest card"));
    }

    #[test]
    fn stale_report_generation_is_rejected() {
        let (report, mut binding) = bound_report();
        binding.report_generation -= 1;
        let error = resolve_digest_selection(&report, &binding)
            .expect_err("stale generation must not dispatch");
        assert_eq!(error.to_string(), "digest_dispatch_snapshot_stale");
    }
}
