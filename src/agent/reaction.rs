//! 用户反应分析 (HP-3)。
//!
//! 该模块负责对用户最新入站消息做异步反应分析（"用户是不是在表达
//! 购买信号 / 反对 / 停止 / 不分类"），并通过 atomic claim 防止并发
//! webhook 重复触发分析。`reclaim_stuck` 兜底把卡死在 `analyzing`
//! 状态超过阈值的 review 重置为 `pending`，避免分析进程崩溃后永远卡死。
//!
//! 波 A1：reaction 路径整体进入 `RUN_BUDGET.scope`，让 LLM 调用计入
//! `agent_run_logs.tokens_used` 并能在预算超额时降级到 `user_replied_unclassified`。

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, to_document, DateTime, Document};

use crate::error::{AppError, AppResult};
use crate::models::{Contact, ConversationMessage, OperationKnowledgeChunk};
use crate::prompts;
use crate::routes::AppState;

use super::budget::{current_run_budget, RunBudget, RUN_BUDGET};
use super::decision::load_user_operation_domain_config;
use super::generate_agent_json;
use super::memory::{effective_memory_card, load_or_create_operating_memory};
use super::outbox;
use super::runtime::UserRuntimeParameters;
use super::types::{doc_bool, doc_string};

pub async fn record_user_reaction(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
) -> AppResult<()> {
    // 波 A1：在最外层为 reaction 路径起一个 RunBudget。即便 stuck 重置阶段
    // 不调用 LLM，只要后续 analyze_user_reaction 命中就能记账并支持降级。
    let domain_config = load_user_operation_domain_config(state, &contact.workspace_id).await?;
    let runtime = UserRuntimeParameters::from_config(domain_config.as_ref(), state);
    let run_id = uuid::Uuid::new_v4().to_string();
    let budget = Arc::new(RunBudget::new(
        run_id.clone(),
        runtime.reaction_token_budget,
        runtime.reaction_max_llm_calls,
        runtime.knowledge_max_tool_calls,
    ));
    RUN_BUDGET
        .scope(
            budget,
            record_user_reaction_inner(state, contact, inbound, run_id),
        )
        .await
}

async fn record_user_reaction_inner(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    fallback_run_id: String,
) -> AppResult<()> {
    // 先做 stuck reaction 兜底：把 analyzing 卡死超过阈值的 review 重置为 pending，
    // 以便本次 webhook 能重新 claim。
    let stuck_threshold_ms =
        (state.config.reaction_analysis_claim_timeout_seconds.max(1)) as i64 * 1000;
    let stuck_before =
        DateTime::from_millis(DateTime::now().timestamp_millis() - stuck_threshold_ms);
    let _ = state
        .db
        .decision_reviews()
        .update_many(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "outcome_status": "analyzing",
                "reaction_claimed_at": { "$lt": stuck_before }
            },
            doc! {
                "$set": { "outcome_status": "pending" },
                "$unset": { "reaction_claimed_at": "" }
            },
            None,
        )
        .await?;

    // HP-3：用 find_one_and_update 把 outcome_status 从 pending/null 原子置为 analyzing。
    // 拿到 Some(review) 才意味着抢到了锁，可以安全调 LLM；其他并发 webhook 直接跳过。
    let claim_filter = doc! {
        "workspace_id": &contact.workspace_id,
        "account_id": &contact.account_id,
        "contact_wxid": &contact.wxid,
        "status": "sent",
        "$or": [
            { "outcome_status": null },
            { "outcome_status": "pending" }
        ]
    };
    let claim_update = doc! {
        "$set": {
            "outcome_status": "analyzing",
            "reaction_claimed_at": DateTime::now()
        }
    };
    let claim_options = mongodb::options::FindOneAndUpdateOptions::builder()
        .sort(doc! { "created_at": -1 })
        .build();
    let claimed = state
        .db
        .decision_reviews()
        .find_one_and_update(claim_filter, claim_update, claim_options)
        .await?;
    let Some(claimed_review) = claimed else {
        // 没抢到锁（或没有 pending review），直接跳过；本次 webhook 不会调 LLM。
        return Ok(());
    };

    let run_id_owned: String = claimed_review
        .run_id
        .clone()
        .unwrap_or_else(|| fallback_run_id.clone());
    let review_id: ObjectId = match claimed_review.id {
        Some(id) => id,
        None => return Ok(()),
    };

    // 波 A1：进入 LLM 之前先做预算检查；超额则降级为 user_replied_unclassified
    // 并在 budget 上 mark_degraded，便于上游审计。
    let budget_exceeded = current_run_budget()
        .map(|b| b.is_exceeded())
        .unwrap_or(false);
    let reaction_analysis = if budget_exceeded {
        if let Some(b) = current_run_budget() {
            b.mark_degraded("reaction_skipped_budget_exceeded".to_string());
        }
        doc! {
            "outcomeStatus": "user_replied_unclassified",
            "confidence": 0,
            "degraded": true,
            "degradedReason": "reaction_skipped_budget_exceeded"
        }
    } else {
        analyze_user_reaction(state, contact, inbound, Some(run_id_owned.as_str()))
            .await
            .unwrap_or_else(|_| {
                doc! { "outcomeStatus": "user_replied_unclassified", "confidence": 0 }
            })
    };
    let outcome = reaction_outcome_status(&reaction_analysis);
    let outcome_for_outbox = outcome.clone();
    let reaction_analysis_for_trajectory = reaction_analysis.clone();
    // Phase C / C1: 用 reviewer 当时的 approved 标志 + 用户实际反应 outcome 计算 misjudge 信号。
    // approved=true 但用户负反应 → approved_but_user_negative（reviewer 放过了实际不该发的内容）。
    // 该信号供 feedback_worker 周期汇总到 reviewer_stats，并作为 C2 negative_example 候选挑选源。
    let reviewer_misjudge_signal = compute_reviewer_misjudge_signal(claimed_review.approved, &outcome);
    let mut update_set = doc! {
        "outcome_status": outcome,
        "send_gateway_result.userReactionMessageId": inbound.message_id.clone().unwrap_or_default(),
        "send_gateway_result.userReactionAt": DateTime::now(),
        "send_gateway_result.userReactionAnalysis": reaction_analysis.clone(),
        "reaction_analysis": reaction_analysis,
    };
    if let Some(signal) = reviewer_misjudge_signal.as_ref() {
        update_set.insert("reviewer_misjudge_signal", signal);
    }
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": review_id },
            doc! { "$set": update_set },
            None,
        )
        .await?;

    // Phase D / D1：把 reaction outcome 追加到 contact.intent_trajectory（滑窗 50）。
    // mongo `$push + $slice: -50` 一步完成 append + 上限裁剪；并发追加（同一 contact
    // 同时收两条入站消息）天然安全 —— 都会落进数组、超出 50 的旧条目被裁掉。
    // best-effort：失败仅 warn，不影响 reaction 主路径。
    if let Err(err) = push_intent_trajectory_entry(
        state,
        contact,
        &outcome_for_outbox,
        &reaction_analysis_for_trajectory,
    )
    .await
    {
        tracing::warn!(
            contact_wxid = %contact.wxid,
            error = %err,
            "push_intent_trajectory_entry failed (best-effort)"
        );
    }

    // Phase C / C2: reviewer 误判 + 用户负反应 → 把发出去的 reply_text 入 chunk
    // review queue（chunk_type=negative_example, integrity_status=needs_review），
    // 由 admin 复核后才会真正进入 negative_example 召回。Best-effort：失败仅 warn。
    if reviewer_misjudge_signal.as_deref() == Some("approved_but_user_negative") {
        if let Some(reply_text) = claimed_review.reply_text.as_deref() {
            if !reply_text.trim().is_empty() {
                if let Err(err) = enqueue_negative_example_chunk(
                    state,
                    contact,
                    reply_text,
                    review_id,
                    &outcome_for_outbox,
                )
                .await
                {
                    tracing::warn!(
                        contact_wxid = %contact.wxid,
                        review_id = %review_id,
                        error = %err,
                        "enqueue_negative_example_chunk failed (best-effort)"
                    );
                }
            }
        }
    }

    // W4 / Task 5.6（R13.6）：若用户反应表示停止 / cooldown，立即把同 contact
    // 名下还在 pending / in_flight 的 outbox entry 一并取消，避免 dispatcher
    // 在用户已经表态"别再发了"之后继续推进过期决策。Best-effort：取消失败
    // 仅记录 warning，不影响 reaction 记录主路径成功落地。
    if outbox::outcome_signals_stop(&outcome_for_outbox) {
        match outbox::cancel_for_contact_on_user_reaction(
            state,
            &contact.account_id,
            &contact.wxid,
        )
        .await
        {
            Ok(count) if count > 0 => {
                tracing::info!(
                    account_id = %contact.account_id,
                    contact_wxid = %contact.wxid,
                    canceled = count,
                    outcome = %outcome_for_outbox,
                    "outbox entries canceled by user_reaction_stop_requested"
                );
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    account_id = %contact.account_id,
                    contact_wxid = %contact.wxid,
                    outcome = %outcome_for_outbox,
                    error = %err,
                    "cancel_for_contact_on_user_reaction failed (best-effort)"
                );
            }
        }
    }
    Ok(())
}

async fn analyze_user_reaction(
    state: &AppState,
    contact: &Contact,
    inbound: &ConversationMessage,
    run_id: Option<&str>,
) -> AppResult<Document> {
    let memory = load_or_create_operating_memory(state, contact).await?;
    let system = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.reaction.system",
    )
    .await?;
    let task = prompts::load_prompt(
        &state.db,
        &state.config.default_workspace_id,
        "user.reaction.task",
    )
    .await?;
    let user = format!(
        r#"{}

客户 wxid: {}
客户昵称: {}
长期记忆卡片:
{}

运营记忆:
{}

用户最新回复:
{}"#,
        task,
        contact.wxid,
        contact.nickname.clone().unwrap_or_default(),
        // task 6.3：`effective_memory_card` 现在返回 `MemoryCardTyped`；
        // prompt 序列化为 JSON 时在边界 `to_document()` 一次性转换。
        serde_json::to_string(&effective_memory_card(&memory).to_document())
            .unwrap_or_default(),
        serde_json::to_string(&memory).unwrap_or_default(),
        inbound.content
    );
    let value = generate_agent_json(
        state,
        Some(&contact.account_id),
        Some(&contact.wxid),
        run_id,
        "user.reaction.task",
        &system,
        &user,
    )
    .await?;
    to_document(&value).map_err(AppError::from)
}

pub(crate) fn reaction_outcome_status(analysis: &Document) -> String {
    if let Some(status) =
        doc_string(analysis, "outcomeStatus").or_else(|| doc_string(analysis, "outcome_status"))
    {
        return status;
    }
    if doc_bool(analysis, "stopRequested") || doc_bool(analysis, "stop_requested") {
        "user_replied_stop_requested".to_string()
    } else if doc_bool(analysis, "buyingSignal") || doc_bool(analysis, "buying_signal") {
        "user_replied_buying_signal".to_string()
    } else if doc_bool(analysis, "objection") {
        "user_replied_objection".to_string()
    } else {
        "user_replied_unclassified".to_string()
    }
}

/// Phase C / C1: 比对 reviewer 当时的 approved 判断与用户实际反应 outcome，
/// 输出 reviewer 误判信号；无误判返回 None。
///
/// 当前覆盖路径：reviewer `approved=true` 且用户落入负向 outcome
/// （`user_replied_objection` / `user_replied_stop_requested` / `user_replied_unsubscribed`
/// / `user_replied_negative` 等）→ `approved_but_user_negative`。
///
/// `blocked_but_user_positive` 分支需要旁路扫描被 review 拦截但用户仍持续正向互动的
/// 历史，更适合 feedback_worker 周期任务，C1 第一刀不在此处计算。
pub(crate) fn compute_reviewer_misjudge_signal(
    reviewer_approved: bool,
    outcome_status: &str,
) -> Option<String> {
    if !reviewer_approved {
        return None;
    }
    if is_negative_outcome(outcome_status) {
        Some("approved_but_user_negative".to_string())
    } else {
        None
    }
}

fn is_negative_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "user_replied_objection"
            | "user_replied_stop_requested"
            | "user_replied_unsubscribed"
            | "user_replied_negative"
            | "user_replied_complaint"
    )
}

/// Phase C / C2：把 reviewer 误判后被用户负反应的回复文本，作为
/// `negative_example` chunk 候选写入 review queue（`integrity_status="needs_review"`）。
///
/// 设计要点：
/// - **不直接进 verified 池**：`integrity_status="needs_review"` 让 admin 在 chunk
///   review queue UI（`routes/knowledge.rs:751` 的 `$in: ["needs_review", null]`
///   过滤已存在）人工复核后才生效，避免脏数据反向训练 reply-agent。
/// - **chunk_type=negative_example**：与 B3 引入的运营用途枚举对齐，
///   `knowledge_router` 把它作为 don't-do 示例段拼接进 prompt（不污染 product_fact / style_template）。
/// - **status="draft"**：在 admin verified 之前不进 active 召回路径。
/// - **idempotent 边界**：以 `(workspace_id, source review_id)` 做去重 —— 同一个
///   review 不会重复入队。idempotency 由 `domain_attributes.source_review_id` 字段持有。
pub(crate) async fn enqueue_negative_example_chunk(
    state: &AppState,
    contact: &Contact,
    reply_text: &str,
    source_review_id: ObjectId,
    user_reaction_outcome: &str,
) -> AppResult<()> {
    let coll = state.db.operation_knowledge_chunks();
    let source_review_id_str = source_review_id.to_hex();

    // 幂等：同一 source_review_id 已经入过队就跳过。
    let existed = coll
        .count_documents(
            doc! {
                "domain_attributes.source_review_id": &source_review_id_str,
            },
            None,
        )
        .await?;
    if existed > 0 {
        return Ok(());
    }

    let now = DateTime::now();
    let title = format!(
        "[reviewer-misjudge] {} 触发的负例",
        truncate_for_title(reply_text, 30)
    );
    let summary = format!(
        "reviewer 通过但用户反应={}，作为 don't-do 示例待人工复核后入库",
        user_reaction_outcome
    );

    let mut domain_attributes = Document::new();
    domain_attributes.insert("source_review_id", &source_review_id_str);
    domain_attributes.insert("source", "reviewer_misjudge");
    domain_attributes.insert("user_reaction_outcome", user_reaction_outcome);
    domain_attributes.insert("contact_wxid", contact.wxid.clone());

    let chunk = OperationKnowledgeChunk {
        id: None,
        workspace_id: contact.workspace_id.clone(),
        account_id: Some(contact.account_id.clone()),
        document_id: None,
        item_id: None,
        domain: "user_operations".to_string(),
        knowledge_type: Some("negative_example".to_string()),
        business_context: Some("reviewer_misjudge_feedback".to_string()),
        title,
        summary: Some(summary),
        body: Some(reply_text.to_string()),
        applicable_scenes: Vec::new(),
        not_applicable_scenes: Vec::new(),
        product_tags: Vec::new(),
        business_topics: Vec::new(),
        source_quote: None,
        source_anchors: Vec::new(),
        integrity_status: Some("needs_review".to_string()),
        confidence_score: Some(0),
        status: "draft".to_string(),
        priority: 0,
        created_at: now,
        updated_at: now,
        domain_attributes: Some(domain_attributes),
        chunk_type: "negative_example".to_string(),
        ..OperationKnowledgeChunk::default()
    };
    coll.insert_one(chunk, None).await?;
    Ok(())
}

fn truncate_for_title(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(max_chars).collect();
    format!("{}…", truncated)
}

/// Phase A1：把最近 N 轮的 `decision_reviews.reaction_analysis` 渲染为下一轮 prompt 段。
///
/// 输入是按时间倒序（最新在前）的 reaction Document 列表；返回值是装配进
/// system prompt 的纯文本片段。空输入返回空串，调用方据此决定是否拼接。
pub(crate) fn format_reaction_hint(recent: &[Document]) -> String {
    if recent.is_empty() {
        return String::new();
    }
    let mut buf = String::from("[最近用户反应回顾]\n");
    for (i, analysis) in recent.iter().enumerate().take(3) {
        let status = reaction_outcome_status(analysis);
        let buying = doc_bool(analysis, "buyingSignal") || doc_bool(analysis, "buying_signal");
        let objection = doc_bool(analysis, "objection");
        let stop = doc_bool(analysis, "stopRequested") || doc_bool(analysis, "stop_requested");
        let summary = doc_string(analysis, "summary")
            .or_else(|| doc_string(analysis, "note"))
            .unwrap_or_default();
        buf.push_str(&format!(
            "- 第{}轮 status={} buying={} objection={} stop={}",
            i + 1,
            status,
            buying,
            objection,
            stop
        ));
        if !summary.is_empty() {
            buf.push_str(&format!(" 摘要={}", summary));
        }
        buf.push('\n');
    }
    buf
}

/// Phase D / D1：把一条 intent 轨迹追加到 `contacts.intent_trajectory`，并在
/// mongo 端用 `$push + $slice: -50` 维持上限滑窗。
///
/// `turn_index` 取该 contact 的 `conversation_messages` 入站行数估算（best-effort）；
/// `objection_type` 从 reaction 分析的 `objectionType` / `objection_type` 字段读取。
/// 任何字段缺失时落空字符串 / None；调用方将本函数视为副作用 best-effort。
pub(crate) async fn push_intent_trajectory_entry(
    state: &AppState,
    contact: &Contact,
    outcome: &str,
    reaction_analysis: &Document,
) -> AppResult<()> {
    use mongodb::options::CountOptions;

    let turn_index = state
        .db
        .messages()
        .count_documents(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "contact_wxid": &contact.wxid,
                "direction": "inbound",
            },
            CountOptions::builder().build(),
        )
        .await
        .unwrap_or(0) as i32;

    let objection_type = doc_string(reaction_analysis, "objectionType")
        .or_else(|| doc_string(reaction_analysis, "objection_type"))
        .filter(|s| !s.trim().is_empty());

    let mut entry = doc! {
        "turnIndex": turn_index,
        "intent": outcome,
        "recordedAt": DateTime::now(),
    };
    if let Some(t) = objection_type.as_deref() {
        entry.insert("objectionType", t);
    }

    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": &contact.workspace_id,
                "account_id": &contact.account_id,
                "wxid": &contact.wxid,
            },
            doc! {
                "$push": {
                    "intent_trajectory": {
                        "$each": [entry],
                        "$slice": -(crate::models::IntentTrajectoryEntry::MAX_ITEMS as i32),
                    }
                }
            },
            None,
        )
        .await?;
    Ok(())
}

/// Phase D / D1：把最近 N=5 项 intent_trajectory 渲染为 prompt 段。
///
/// 输入是 contact.intent_trajectory（按写入顺序，最早在前）；返回值是
/// 注入下一轮 reply prompt 的纯文本片段。空 trajectory 返回空串。
pub(crate) fn format_intent_trajectory_hint(
    trajectory: &[crate::models::IntentTrajectoryEntry],
) -> String {
    if trajectory.is_empty() {
        return String::new();
    }
    let mut buf = String::from("[最近 intent 轨迹]\n");
    let recent: Vec<&crate::models::IntentTrajectoryEntry> =
        trajectory.iter().rev().take(5).collect();
    for entry in recent.iter().rev() {
        buf.push_str(&format!(
            "- 第{}轮 intent={}",
            entry.turn_index, entry.intent
        ));
        if let Some(t) = entry.objection_type.as_deref() {
            buf.push_str(&format!(" objection_type={}", t));
        }
        buf.push('\n');
    }
    buf
}

#[cfg(test)]
mod a6_tests {
    use super::*;
    use mongodb::bson::doc;

    /// Phase A6: `reaction_hint_present_in_prompt`
    /// 验证 `format_reaction_hint` 能把最近 reaction_analysis 渲染成可注入下一轮 prompt 的文本段。
    #[test]
    fn reaction_hint_present_in_prompt() {
        let recent = vec![
            doc! { "outcomeStatus": "user_replied_objection", "objection": true, "summary": "嫌贵" },
            doc! { "outcomeStatus": "user_replied_buying_signal", "buyingSignal": true },
        ];
        let hint = format_reaction_hint(&recent);
        assert!(hint.contains("[最近用户反应回顾]"), "hint should have header");
        assert!(hint.contains("user_replied_objection"), "first turn status missing");
        assert!(hint.contains("user_replied_buying_signal"), "second turn status missing");
        assert!(hint.contains("摘要=嫌贵"), "summary should be rendered");
        assert!(hint.contains("buying=true"));
        assert!(hint.contains("objection=true"));
    }

    #[test]
    fn reaction_hint_empty_when_no_history() {
        let hint = format_reaction_hint(&[]);
        assert!(hint.is_empty(), "empty history yields empty hint");
    }

    /// Phase C / C1: reviewer 误判信号判定。
    /// approved=true + 用户负反应 → approved_but_user_negative；其它输入返回 None。
    #[test]
    fn misjudge_signal_approved_but_user_negative() {
        assert_eq!(
            compute_reviewer_misjudge_signal(true, "user_replied_objection").as_deref(),
            Some("approved_but_user_negative")
        );
        assert_eq!(
            compute_reviewer_misjudge_signal(true, "user_replied_stop_requested").as_deref(),
            Some("approved_but_user_negative")
        );
        assert_eq!(
            compute_reviewer_misjudge_signal(true, "user_replied_complaint").as_deref(),
            Some("approved_but_user_negative")
        );
    }

    #[test]
    fn misjudge_signal_none_when_reviewer_blocked() {
        assert!(compute_reviewer_misjudge_signal(false, "user_replied_objection").is_none());
        assert!(compute_reviewer_misjudge_signal(false, "user_replied_buying_signal").is_none());
    }

    #[test]
    fn misjudge_signal_none_when_outcome_not_negative() {
        assert!(compute_reviewer_misjudge_signal(true, "user_replied_buying_signal").is_none());
        assert!(compute_reviewer_misjudge_signal(true, "user_replied_unclassified").is_none());
    }

    /// Phase C / C2: title 截断按字符数，不按字节，避免破坏 UTF-8 边界。
    #[test]
    fn truncate_for_title_unicode_safe() {
        let text = "这是一段很长的中文回复文本应当被截断";
        let title = truncate_for_title(text, 5);
        assert_eq!(title.chars().count(), 6, "5 chars + ellipsis = 6");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn truncate_for_title_no_truncation_when_short() {
        let text = "短文本";
        let title = truncate_for_title(text, 30);
        assert_eq!(title, "短文本");
    }

    /// Phase D / D1：空 trajectory 不渲染段头。
    #[test]
    fn intent_trajectory_hint_empty_when_no_history() {
        assert!(format_intent_trajectory_hint(&[]).is_empty());
    }

    /// Phase D / D1：渲染最近 5 项；超过 5 仅取最后 5 条；保留写入时间顺序。
    #[test]
    fn intent_trajectory_hint_renders_last_five_in_order() {
        use crate::models::IntentTrajectoryEntry;
        use mongodb::bson::DateTime;
        let entries: Vec<IntentTrajectoryEntry> = (1..=8)
            .map(|i| IntentTrajectoryEntry {
                turn_index: i,
                intent: format!("intent_{i}"),
                objection_type: if i % 2 == 0 {
                    Some(format!("obj_{i}"))
                } else {
                    None
                },
                recorded_at: DateTime::from_millis(i as i64 * 1000),
            })
            .collect();
        let hint = format_intent_trajectory_hint(&entries);
        assert!(hint.starts_with("[最近 intent 轨迹]"));
        // 只渲染最后 5 项 (turn 4..=8)
        assert!(!hint.contains("intent_3"), "should drop turn 3");
        assert!(hint.contains("第4轮 intent=intent_4"));
        assert!(hint.contains("第8轮 intent=intent_8"));
        // objection_type 只在 even 索引时存在
        assert!(hint.contains("objection_type=obj_4"));
        assert!(!hint.contains("objection_type=obj_5"));
        // 顺序：最早的（4）在最前
        let pos_4 = hint.find("第4轮").unwrap();
        let pos_8 = hint.find("第8轮").unwrap();
        assert!(pos_4 < pos_8, "older turn should appear first");
    }
}
