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
use crate::models::{Contact, ConversationMessage};
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
    state
        .db
        .decision_reviews()
        .update_one(
            doc! { "_id": review_id },
            doc! {
                "$set": {
                    "outcome_status": outcome,
                    "send_gateway_result.userReactionMessageId": inbound.message_id.clone().unwrap_or_default(),
                    "send_gateway_result.userReactionAt": DateTime::now(),
                    "send_gateway_result.userReactionAnalysis": reaction_analysis.clone(),
                    "reaction_analysis": reaction_analysis
                }
            },
            None,
        )
        .await?;

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
}
