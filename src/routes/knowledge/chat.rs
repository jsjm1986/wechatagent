//! 运营知识库对话补库：chat turn/apply/history + 意图分诊 + 草拟/更新/应用 + 后台任务流。

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use mongodb::bson::{doc, oid::ObjectId, Bson, DateTime, Document};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::AuthenticatedAdmin;
use crate::error::{AppError, AppResult};
use crate::agent;

use super::super::shared::*;
use super::super::AppState;
use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub chunk_id: Option<String>,
    pub item_id: Option<String>,
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
}

pub async fn chat_turn(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Json(body): Json<ChatTurnRequest>,
) -> AppResult<Json<Value>> {
    let trimmed = body.content.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "content cannot be empty".to_string(),
        ));
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
    let operator_id = body
        .operator_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "default".to_string());

    // 加载历史 turns（按 turn_index 升序）
    let history = load_chat_history(&state, &admin.current_workspace, &account_id, &session_id).await?;
    // P1-7：原子预分配两个 turn_index——user turn + assistant turn，避免并发
    // 写者读到同一 last 制造重复索引。返回的是分配后的最大 seq；user 拿
    // `assistant_index - 1`、assistant 拿 `assistant_index`。
    let assistant_index =
        allocate_next_turn_indices(&state, &admin.current_workspace, &session_id, 2).await?;
    let next_index = assistant_index - 1;
    let assistant_turns_so_far = history
        .iter()
        .filter(|t| t.role == "assistant")
        .count() as i32;
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
                item_attached,
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
                    x.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| {
                            x.get("field").and_then(|f| f.as_str()).map(|s| s.to_string())
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
    // knowledge-digest-workstation Phase 4 / P4.4：digest_action intent 命中时
    // LLM 出 plannedSteps + estimatedLlmCalls，转发给前端弹「派工确认」小卡。
    let planned_steps = result.get("plannedSteps").cloned();
    let estimated_llm_calls = result
        .get("estimatedLlmCalls")
        .and_then(|v| v.as_i64());
    let can_apply = patch.is_some()
        && missing_fields.is_empty()
        && draft_kind.is_some();
    let tokens_used = budget.snapshot().tokens_used;

    // 写 assistant turn
    let attachments_for_assistant: Vec<ChatAttachment> = match (&target_chunk_id, &target_pack_id) {
        (Some(c), _) => vec![ChatAttachment {
            chunk_id: Some(c.clone()),
            item_id: None,
        }],
        (None, Some(p)) => vec![ChatAttachment {
            chunk_id: None,
            item_id: Some(p.clone()),
        }],
        _ => body.attachments,
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
        format!(
            "AI 对话补完 sessionId={session_id} 第 {assistant_index} 轮 intent={intent}"
        ),
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
        "missingFields": missing_fields,
        "followupQuestions": followups,
        "canApply": can_apply,
        "targetChunkId": target_chunk_id,
        "targetPackId": target_pack_id,
        "promptKey": prompt_key,
        "tokensUsed": tokens_used,
        "budget": budget_document(&budget),
    })))
}

pub(in crate::routes) async fn chat_history(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let mut cursor = state
        .db
        .knowledge_chat_turns()
        .find(
            doc! {
                "workspace_id": &admin.current_workspace,
                "session_id": trimmed,
            },
            FindOptions::builder().sort(doc! { "turn_index": 1 }).build(),
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
    let history = load_chat_history(&state, &admin.current_workspace, "*", &trimmed).await?;
    let last_assistant = history
        .iter()
        .rev()
        .find(|t| t.role == "assistant" && t.status == "pending" && t.patch.is_some())
        .ok_or_else(|| {
            AppError::BadRequest(
                "session 没有可应用的 AI 草稿（需要先发起 chat 让 AI 起草）".to_string(),
            )
        })?;

    let intent = last_assistant.intent.as_deref().unwrap_or("freeform");
    let patch = last_assistant
        .patch
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("最近一轮 AI 没有 patch".to_string()))?;

    let account_id = body
        .account_id
        .clone()
        .or_else(|| {
            if last_assistant.account_id.is_empty() {
                None
            } else {
                Some(last_assistant.account_id.clone())
            }
        })
        .unwrap_or_else(|| state.config.default_account_id.clone());

    // 取出 attachments 中的 chunk_id / item_id（assistant 已回填）
    let target_chunk_id = last_assistant
        .attachments
        .iter()
        .filter_map(|a| a.get_str("chunk_id").ok())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string());
    let target_pack_id = last_assistant
        .attachments
        .iter()
        .filter_map(|a| a.get_str("item_id").ok())
        .find(|s| !s.is_empty())
        .map(|s| s.to_string());

    let result_value = match intent {
        "create_chunk" => {
            // chat 新建知识无父文档，溯源 = 运营在本会话里的陈述。拼接所有 user-role
            // turn 的正文作为 operator_statement，交给 apply_create_chunk 锚定 sourceQuote。
            let operator_statement = history
                .iter()
                .filter(|t| t.role == "user")
                .map(|t| t.content.trim())
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            // chat 新建知识默认落 workspace 共享域（account_id=null），与 seed/import/manual
            // 等所有其它写入口一致——只有运营显式绑定了某个**非 default** 账号时才把切片私有化
            // 到该账号。否则共享，使共享域召回（account_id=None）与任意联系人生产召回
            // （account_id=Some(contact) 匹配 null OR contact）都能检索到对话补的知识。
            let create_account_id: Option<String> = body
                .account_id
                .clone()
                .or_else(|| {
                    if last_assistant.account_id.is_empty() {
                        None
                    } else {
                        Some(last_assistant.account_id.clone())
                    }
                })
                .filter(|a| !a.is_empty() && *a != state.config.default_account_id);
            apply_create_chunk(&state, &admin.current_workspace, create_account_id.as_deref(), &trimmed, patch, target_pack_id.as_deref(), &operator_statement)
                .await?
        }
        "update_chunk" => {
            let chunk_id = target_chunk_id.clone().ok_or_else(|| {
                AppError::BadRequest("update_chunk 需要 attachments.chunkId".to_string())
            })?;
            apply_update_chunk(&state, &admin.current_workspace, &account_id, &chunk_id, patch).await?
        }
        "update_pack" => {
            let pack_id = target_pack_id.clone().ok_or_else(|| {
                AppError::BadRequest("update_pack 需要 attachments.itemId".to_string())
            })?;
            apply_update_pack(&state, &account_id, &pack_id, patch).await?
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "intent={other} 不可应用为草稿（仅 create_chunk / update_chunk / update_pack 可应用）"
            )));
        }
    };

    // 标 turn applied
    state
        .db
        .knowledge_chat_turns()
        .update_one(
            doc! {
                "_id": last_assistant.id.expect("turn must have id"),
                "workspace_id": &admin.current_workspace,
            },
            doc! { "$set": { "status": "applied", "updated_at": DateTime::now() } },
            None,
        )
        .await?;

    record_repair_event(
        &state,
        &admin.current_workspace,
        &account_id,
        "knowledge_chat_applied",
        format!("AI 对话产物落库为草稿 sessionId={trimmed} intent={intent}"),
        doc! {
            "kind": "chunk_chat_session",
            "sessionId": &trimmed,
            "intent": intent,
            "result": mongodb::bson::to_bson(&result_value).unwrap_or(Bson::Null),
        },
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "sessionId": trimmed,
        "intent": intent,
        "result": result_value,
    })))
}

pub(in crate::routes) async fn chat_discard(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(session_id): Path<String>,
) -> AppResult<Json<Value>> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    let res = state
        .db
        .knowledge_chat_turns()
        .update_many(
            doc! {
                "workspace_id": &admin.current_workspace,
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
    session_id: &str,
    count: u32,
) -> AppResult<i32> {
    use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
    let n = count.max(1) as i64;
    let key = format!("{}|{}", workspace_id, session_id);
    let updated = state
        .db
        .knowledge_chat_session_seqs()
        .find_one_and_update(
            doc! { "_id": &key },
            doc! { "$inc": { "seq": n } },
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    let seq = updated
        .as_ref()
        .and_then(|d| d.get_i64("seq").ok())
        .unwrap_or(n);
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
            FindOptions::builder().sort(doc! { "turn_index": 1 }).build(),
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
    patch: Option<&Value>,
    missing_fields: &[String],
    followups: &[Value],
    status: &str,
    tokens_used: i64,
    prompt_key: Option<&str>,
) -> AppResult<()> {
    let attachments_doc: Vec<Document> = attachments
        .iter()
        .filter_map(|a| {
            let mut d = Document::new();
            if let Some(c) = a.chunk_id.as_deref().filter(|s| !s.is_empty()) {
                d.insert("chunk_id", c.to_string());
            }
            if let Some(i) = a.item_id.as_deref().filter(|s| !s.is_empty()) {
                d.insert("item_id", i.to_string());
            }
            if d.is_empty() {
                None
            } else {
                Some(d)
            }
        })
        .collect();
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
                        x.get("field").and_then(|f| f.as_str()).map(|s| s.to_string())
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
        format!("我已经按您的要求起草好{}，拟定的标题是「{t}」。", filled.join("、"))
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
    item_attached: Option<&str>,
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    // knowledge-digest-workstation Phase 5：先取运营长期偏好记忆，作为
    // intent 分类与下游分支的 prompt header。与 contacts.memory_card 物理
    // 隔离（仅触达 knowledge_operator_memory collection）。
    let operator_memory = agent::load_operator_memory(
        &state.db,
        workspace_id,
        account_id,
        operator_id,
        5,
    )
    .await
    .unwrap_or_default();
    let operator_memory_header = render_operator_memory_for_prompt(&operator_memory);

    // 1. intent 分类
    let intent_result = classify_intent(
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
    .await?;
    let intent = intent_result
        .get("intent")
        .and_then(|v| v.as_str())
        .unwrap_or("freeform")
        .to_string();
    let target_chunk_id = intent_result
        .get("targetChunkId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| chunk_attached.map(|s| s.to_string()));
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
                history,
            )
            .await?;
            v["draftKind"] = json!("chunk_update");
            v["promptKey"] = json!("knowledge.chat.update_chunk");
            v
        }
        "update_pack" => {
            let pack_id = target_pack_id.clone().ok_or_else(|| {
                AppError::BadRequest(
                    "update_pack 需要 attachments.itemId 或在对话中明确引用知识包".to_string(),
                )
            })?;
            let mut v = update_pack_for_chat(
                state,
                workspace_id,
                account_id,
                session_id,
                user_content,
                &pack_id,
                history,
            )
            .await?;
            v["draftKind"] = json!("pack_update");
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
        _ => clarify_for_chat(state, workspace_id, account_id, session_id, user_content, history)
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
    for t in history.iter().rev().take(6).collect::<Vec<_>>().iter().rev() {
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
    if memories.is_empty() {
        return String::new();
    }
    let mut s = String::from("【运营长期偏好（仅作上下文，不要写回 chunk patch）】\n");
    for m in memories.iter().take(5) {
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
// propose_repair / analyze_logs / open_document / inspect_pack / verify_anchor）
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

    // 拉 KnowledgeRuntime 快照：documents / items / verified chunks。
    // 与 user-ops `load_operation_knowledge` 的形态对齐，但简化为按 workspace
    // 全量取（chat 不绑定到具体 contact，没有 account_filter）。limit 与 user-ops
    // 一致，避免 KnowledgeRuntime 跨 chunk 数量发散。
    let workspace_id = workspace_id_in.to_string();
    let documents: Vec<OperationKnowledgeDocument> = state
        .db
        .operation_knowledge_documents()
        .find(
            doc! { "workspace_id": &workspace_id, "domain": "user_operations", "status": "active" },
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
            },
            FindOptions::builder()
                .sort(doc! { "priority": -1_i32, "updated_at": -1_i32 })
                .limit(200)
                .build(),
        )
        .await?
        .try_collect()
        .await?;
    let knowledge = KnowledgeRuntime {
        documents,
        chunks,
    };
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
            let run_id = format!(
                "chat-{session_id_owned}-{run_key_owned}-loop-{loop_count}"
            );
            let value = agent::generate_agent_json(
                &state_arc,
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
            let raw: RawAgentDecision =
                serde_json::from_value(value).map_err(AppError::from)?;
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
        budget,
        Some(source_anchor_for_quote_ffi as agent::AnchorMatchFn),
        reply_fn,
    )
    .await;
    let final_value = match outcome {
        Ok(_outcome) => {
            // 取最后一轮 LLM 原始 JSON 作为 final payload。
            // 若 last_raw 为空（reply_fn 一次都没调用成功），用 empty object 兜底。
            last_raw
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| {
                    json!({
                        "decisionPhase": "final",
                        "naturalReply": "（AI 未给出回复）",
                    })
                })
        }
        Err(ChatToolLoopError::Timeout { elapsed_ms, .. }) => {
            // 超时——返回温和 final，让上层 handler 仍能写 turn 与 event。
            json!({
                "decisionPhase": "final",
                "naturalReply": format!("（AI 工具循环超时 elapsed_ms={elapsed_ms}，请稍后再试或换个说法）"),
            })
        }
        Err(ChatToolLoopError::Reply(err)) => return Err(err),
    };
    Ok(final_value)
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
    let summary = format!("已记下您的{kind_label}：{}", truncate_for_prompt(&content, 80));
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
        "你是知识库对话 Agent，仅识别意图。只输出 JSON: {intent, confidence, targetChunkId?, targetPackId?, memoryKind?, memoryContent?, userIntentSummary}.".to_string()
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

请输出 JSON，intent 必须在 [create_chunk, update_chunk, clarify_chunk, update_pack, digest_action, update_operator_memory, freeform] 中。"#,
        chunk_attached.unwrap_or("(无)"),
        item_attached.unwrap_or("(无)"),
        render_chat_history_for_prompt(history),
    );
    let run_id = format!("chat-{session_id}-intent");
    agent::generate_agent_json(
        state,
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
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let oid = parse_object_id(chunk_id)?;
    let chunk = state
        .db
        .operation_knowledge_chunks()
        .find_one(
            doc! {
                "_id": oid,
                "workspace_id": workspace_id,
            },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("chunk {chunk_id} not found")))?;
    let document_payload = if let Some(document_id) = chunk.document_id {
        state
            .db
            .operation_knowledge_documents()
            .find_one(
                doc! {
                    "_id": document_id,
                    "workspace_id": workspace_id,
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
    run_chat_with_tools(
        state,
        workspace_id,
        account_id,
        session_id,
        "update",
        "knowledge.chat.update_chunk",
        augmented_system,
        user,
    )
    .await
}

async fn update_pack_for_chat(
    _state: &AppState,
    _workspace_id: &str,
    _account_id: &str,
    _session_id: &str,
    _user_content: &str,
    pack_id: &str,
    _history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    // operation_knowledge_items 已删除；pack-level chat 路径暂时下线。
    Err(AppError::BadRequest(format!(
        "operation_knowledge_items has been removed; pack {pack_id} chat update is disabled"
    )))
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
    history: &[KnowledgeChatTurn],
) -> AppResult<Value> {
    let system = prompts::load_prompt(
        &state.db,
        workspace_id,
        "knowledge.digest.dispatch",
    )
    .await
    .unwrap_or_else(|_| {
        "你是 AI 调度器，把运营勾的卡片拆成 plannedSteps。只输出 JSON: {plannedSteps, estimatedLlmCalls, naturalReply}.".to_string()
    });

    // 取今日日报里未 dismiss 的卡片摘要（≤ 20 条）作为参考
    // 卡片实际勾选由前端在 attachments 里传，但本轮 chat 不收 cardIds —— 让 LLM
    // 看到全量候选 + 运营自然语言去匹配（运营常说"把这 3 张 fix 了"）。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
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
        .await?;
    let mut card_summaries: Vec<Value> = vec![];
    if let Some(r) = report {
        for c in r.cards.iter().take(20) {
            if r.dismissed_card_ids.contains(&c.card_id) {
                continue;
            }
            card_summaries.push(json!({
                "cardId": c.card_id.to_hex(),
                "kind": c.kind,
                "title": c.title,
                "summary": c.summary,
                "suggestedAction": c.suggested_action,
                "severity": c.severity,
            }));
        }
    }

    let user = format!(
        r#"运营本轮输入：
{user_content}

今日日报候选卡片（最多 20 条，未被 dismiss）：
{cards}

最近历史（最多 6 条）：
{history}

请按 system 中 schema 输出 plannedSteps（步数 ≤ 8、总 estimatedLlmCalls ≤ 12）。
每个 step 必须含 stepId / cardId / action / summary / estimatedLlmCalls。
action 必须在 [fix_chunk, add_chunk, retag, review_evolution, analyze_logs, dismiss] 中。"#,
        cards = serde_json::to_string_pretty(&card_summaries).unwrap_or_else(|_| "[]".to_string()),
        history = render_chat_history_for_prompt(history),
    );
    let run_id = format!("chat-{session_id}-dispatch");
    agent::generate_agent_json(
        state,
        Some(account_id),
        None,
        Some(&run_id),
        "knowledge.digest.dispatch",
        &system,
        &user,
    )
    .await
}

async fn apply_create_chunk(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    session_id: &str,
    patch: &Document,
    target_pack_id: Option<&str>,
    operator_statement: &str,
) -> AppResult<Value> {
    let patch_value: Value = mongodb::bson::Bson::Document(patch.clone()).into();
    let mut payload = chunk_request_from_chat_patch(&patch_value, account_id, target_pack_id);
    // 强制：AI 永不自动 verify
    payload.status = "draft".to_string();
    payload.integrity_status = Some("needs_review".to_string());

    // chat 新建的知识没有父文档，溯源 = 运营在会话里的口头陈述本身。优先采用 LLM
    // patch 给出的 sourceQuote（若能在运营陈述中锚定），否则回退用运营陈述全文作为
    // quote。这样 D2 verify 闸（sourceQuote + source_anchors 双非空）才能"凭真实出处"
    // 合法通过——是补齐溯源，而非削弱闸门。
    let statement = operator_statement.trim();
    if statement.is_empty() {
        // 没有运营陈述可溯源时维持旧行为：verify 仍会按 D2 合法拒绝，绝不放水。
        payload.source_anchors = vec![];
    } else {
        let quote = payload
            .source_quote
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty() && source_anchor_for_quote(statement, None, q).is_some())
            .map(|q| q.to_string())
            .unwrap_or_else(|| statement.to_string());
        payload.source_anchors = source_anchor_for_quote(statement, None, &quote)
            .map(|d| vec![d])
            .unwrap_or_default();
        payload.source_quote = Some(quote);
    }

    validate_operation_knowledge_chunk(&payload)?;
    let chunk = operation_knowledge_chunk_from_request(state, workspace_id, payload, None)?;
    let inserted = state
        .db
        .operation_knowledge_chunks()
        .insert_one(chunk, None)
        .await?;
    let new_id = inserted
        .inserted_id
        .as_object_id()
        .map(|o| o.to_hex())
        .unwrap_or_default();
    Ok(json!({
        "createdChunkId": new_id,
        "sessionId": session_id,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
}

async fn apply_update_chunk(
    state: &AppState,
    workspace_id: &str,
    _account_id: &str,
    chunk_id: &str,
    patch: &Document,
) -> AppResult<Value> {
    let oid = parse_object_id(chunk_id)?;
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
    update_doc.insert("integrity_status", "needs_review");
    update_doc.insert("status", "draft");
    update_doc.insert("updated_at", DateTime::now());
    state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! {
                "_id": oid,
                "workspace_id": workspace_id,
            },
            doc! { "$set": update_doc.clone() },
            None,
        )
        .await?;
    Ok(json!({
        "updatedChunkId": chunk_id,
        "fieldsTouched": update_doc.len() - 3,
        "status": "draft",
        "integrityStatus": "needs_review",
    }))
}

async fn apply_update_pack(
    _state: &AppState,
    _account_id: &str,
    pack_id: &str,
    _patch: &Document,
) -> AppResult<Value> {
    // operation_knowledge_items 已删除；pack-level apply 路径暂时下线。
    Err(AppError::BadRequest(format!(
        "operation_knowledge_items has been removed; pack {pack_id} update is disabled"
    )))
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
        routing_card: s(patch, "routingCard"),
        applicable_scenes: arr(patch, "applicableScenes"),
        not_applicable_scenes: arr(patch, "notApplicableScenes"),
        safe_claims: arr(patch, "safeClaims"),
        forbidden_claims: arr(patch, "forbiddenClaims"),
        evidence_items: arr(patch, "evidenceItems"),
        product_tags: arr(patch, "productTags"),
        business_topics: arr(patch, "businessTopics"),
        source_quote: s(patch, "sourceQuote"),
        source_anchors: vec![],
        integrity_status: Some("needs_review".to_string()),
        confidence_score: None,
        distortion_risks: vec![],
        unsupported_claims: vec![],
        verified_claims: vec![],
        status: "draft".to_string(),
        priority: 0,
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
    #[serde(default)]
    pub card_ids: Vec<String>,
    #[serde(default)]
    pub planned_steps: Vec<Value>,
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
    if body.planned_steps.is_empty() {
        return Err(AppError::BadRequest(
            "plannedSteps 不能为空，请先经 chat dispatch 拿到步骤计划".to_string(),
        ));
    }
    if body.planned_steps.len() > 8 {
        return Err(AppError::BadRequest(
            "plannedSteps 步数超过 8 条，请由前端分批派工".to_string(),
        ));
    }
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| state.config.default_account_id.clone());

    // 把 plannedSteps 序列化成 BSON Document 数组（每条至少含 stepId/cardId/action）。
    // P1-4：action 闭集校验——只接受 worker `execute_step` 已实装的 6 种 action；
    // 越界（如 LLM 幻觉出 `delete_chunk`）必须在入库前 400 拦掉，不能依赖 worker
    // 的 fail-soft match-arm 兜底（fail-soft 会污染 completed_steps + summary 计数）。
    // 该名单与 `parse_cards_from_llm_array` 的 allowed_actions 保持一致。
    const ALLOWED_TASK_ACTIONS: &[&str] = &[
        "fix_chunk",
        "add_chunk",
        "retag",
        "review_evolution",
        "analyze_logs",
        "dismiss",
    ];
    let mut steps_doc: Vec<Document> = Vec::with_capacity(body.planned_steps.len());
    for (idx, step) in body.planned_steps.iter().enumerate() {
        let mut d = bson_from_json(step)
            .map_err(|e| AppError::BadRequest(format!("plannedSteps[{idx}] 非法 JSON: {e}")))?;
        if d.get_str("stepId").is_err() {
            d.insert("stepId", format!("step_{}", idx + 1));
        }
        let action = d.get_str("action").map_err(|_| {
            AppError::BadRequest(format!("plannedSteps[{idx}].action 缺失"))
        })?;
        if !ALLOWED_TASK_ACTIONS.contains(&action) {
            return Err(AppError::BadRequest(format!(
                "plannedSteps[{idx}].action='{action}' 不在允许集合内：{:?}",
                ALLOWED_TASK_ACTIONS
            )));
        }
        steps_doc.push(d);
    }

    // cards 快照：从今日日报里反查（best-effort，缺失也允许落 task）。
    let report_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let report = state
        .db
        .knowledge_daily_reports()
        .find_one(
            doc! {
                "workspace_id": &admin.current_workspace,
                "account_id": &account_id,
                "report_date": &report_date,
            },
            None,
        )
        .await?;
    let mut card_snapshots: Vec<crate::models::KnowledgeDigestCard> = vec![];
    if let Some(r) = report {
        for cid_hex in &body.card_ids {
            if let Ok(oid) = ObjectId::parse_str(cid_hex) {
                if let Some(c) = r.cards.iter().find(|c| c.card_id == oid) {
                    card_snapshots.push(c.clone());
                }
            }
        }
    }

    let task_id = ObjectId::new();
    let task = crate::models::KnowledgeChatTask {
        id: Some(task_id),
        workspace_id: admin.current_workspace.clone(),
        account_id: account_id.clone(),
        session_id: session_id.to_string(),
        operator_id: body.operator_id.clone(),
        cards: card_snapshots,
        planned_steps: steps_doc,
        completed_steps: vec![],
        status: "pending".to_string(),
        error_kind: None,
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
    let next_index = allocate_next_turn_indices(&state, &admin.current_workspace, session_id, 1).await?;
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
            task_id,
            body.planned_steps.len()
        ),
        attachments: vec![doc! { "taskId": task_id, "phase": "queued" }],
        patch: None,
        missing_fields: vec![],
        followup_questions: vec![],
        status: "pending".to_string(),
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
    state.chat_progress_bus.bump(session_id).await;

    Ok(Json(json!({
        "taskId": task_id.to_hex(),
        "sessionId": session_id,
        "status": "pending",
        "totalSteps": body.planned_steps.len() as i32,
    })))
}

/// `GET /api/knowledge/chat/tasks/:id`：查询 task 状态（前端 fallback 拉取）。
pub(in crate::routes) async fn chat_task_get(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id_hex): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid task id: {id_hex}")))?;
    let task = state
        .db
        .knowledge_chat_tasks()
        .find_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            None,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("knowledge_chat_task {id_hex} 不存在")))?;
    Ok(Json(json!({
        "taskId": task.id.map(|i| i.to_hex()).unwrap_or_default(),
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
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id_hex)
        .map_err(|_| AppError::BadRequest(format!("invalid task id: {id_hex}")))?;
    let res = state
        .db
        .knowledge_chat_tasks()
        .update_one(
            doc! {
                "_id": oid,
                "workspace_id": &admin.current_workspace,
                "status": doc! { "$in": ["pending", "running"] }
            },
            doc! { "$set": { "status": "cancelled", "finished_at": DateTime::now() } },
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
                doc! { "_id": oid, "workspace_id": &admin.current_workspace },
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
    Ok(Json(json!({ "ok": true, "taskId": id_hex, "status": "cancelled" })))
}

/// `GET /api/knowledge/chat/sessions/:sid/stream`：SSE 推送最新 turn_index。
/// 客户端按收到的 version 回拉 `chat_history` 拿增量 turn。
///
/// P1-6：watch 值为 [`crate::knowledge_task::CLOSE_SENTINEL`] 时，发一个
/// `close` event 后立即结束流（`return None`）。前端 EventSource 收到 close
/// 事件应主动关闭 + 不再重连，避免占用连接。
pub(in crate::routes) async fn chat_session_stream(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> axum::response::Sse<
    impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use crate::knowledge_task::CLOSE_SENTINEL;
    let rx = state.chat_progress_bus.subscribe(&session_id).await;
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
    Sse::new(stream).keep_alive(KeepAlive::default())
}
