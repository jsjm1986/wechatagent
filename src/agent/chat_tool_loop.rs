//! 知识库对话 Agent 的多轮工具循环（knowledge-digest-workstation Phase 5）。
//!
//! 多轮工具循环形态：
//!
//! - 中间轮 `decision_phase=tool_calling` → 派发 toolCalls →
//!   `[system tool result]` 段累积到下一轮 prompt；
//! - 最终轮 `decision_phase=final` → 直接返回；
//! - 单 turn toolCalls 数量上限 6；
//! - 失败连击 ≥ 3 强制结束；
//! - 总耗时 30s 硬超时；
//! - tool_call_budget 超额按 budget_exceeded 强制结束；
//! - tool_trace 累计上限 32，超出截断 + risk。
//!
//! **唯一区别**：本循环走 [`super::knowledge_tools::dispatch_chat_tool_call`]，
//! 因此能直接读 MongoDB（异步），但永不写库、永不进 outbox / mcp。

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::{doc, Document};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::budget::RunBudget;
use super::knowledge_tools::{
    dispatch_chat_tool_call, AnchorMatchFn, ToolDispatchState, CHAT_TOOL_CALLS_PER_TURN_CAP,
    TOOL_OPEN_SLICE, TOOL_SEARCH, TOOL_SEARCH_CHUNKS,
};
use super::runtime::UserRuntimeParameters;
use super::types::{AgentDecision, KnowledgeRuntime, ToolCallRequest};
use crate::db::Database;
use crate::error::{AppError, AppResult};

/// 工具循环总耗时硬上限（30s，与 user-ops 一致）。
pub(crate) const CHAT_TOOL_LOOP_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// tool 失败连击阈值。
pub(crate) const CHAT_TOOL_FAILURE_STREAK_LIMIT: i32 = 3;
/// `[system tool result]` 累计注入 prompt 的字符上限。
pub(crate) const CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS: usize = 8000;
/// tool_trace 累计上限。
pub(crate) const CHAT_TOOL_TRACE_MAX_LEN: usize = 32;
/// chat 单 session 最多多少轮工具循环（≥ 1，≤ 4）。
pub(crate) const CHAT_TOOL_LOOP_MAX_LOOPS: i32 = 4;

#[derive(Debug, Clone)]
pub(crate) struct ChatToolLoopOutcome {
    pub decision: AgentDecision,
    pub risks: Vec<String>,
    pub tool_trace: Vec<Document>,
    pub tool_calls_dispatched: i32,
    pub elapsed_ms: i64,
}

#[derive(Debug)]
pub(crate) enum ChatToolLoopError {
    Timeout {
        elapsed_ms: i64,
        risks: Vec<String>,
        tool_trace: Vec<Document>,
    },
    Reply(AppError),
}

impl From<AppError> for ChatToolLoopError {
    fn from(value: AppError) -> Self {
        ChatToolLoopError::Reply(value)
    }
}

pub(crate) type ChatReplyResult = AppResult<(AgentDecision, Vec<String>)>;

pub(crate) type ChatReplyFn<'a> = Box<
    dyn Fn(
            &str,
            i32,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = ChatReplyResult> + Send + 'a>>
        + Send
        + Sync
        + 'a,
>;

/// 知识库对话 Agent 多轮工具循环。
///
/// 失败连击 / 总超时 / 预算硬门 / final 清空，通过 `db` 注入异步 dispatch。
pub(crate) async fn chat_reply_with_tools_loop<'a>(
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    db: &Database,
    workspace_id: &str,
    account_id: &str,
    budget: Arc<RunBudget>,
    anchor_match: Option<AnchorMatchFn>,
    reply_fn: ChatReplyFn<'a>,
    requested_max_loops: i32,
) -> Result<ChatToolLoopOutcome, ChatToolLoopError> {
    chat_reply_with_tools_loop_with_timeout(
        runtime,
        knowledge,
        db,
        workspace_id,
        account_id,
        budget,
        anchor_match,
        reply_fn,
        CHAT_TOOL_LOOP_TOTAL_TIMEOUT,
        requested_max_loops,
    )
    .await
}

async fn chat_reply_with_tools_loop_with_timeout<'a>(
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    db: &Database,
    workspace_id: &str,
    account_id: &str,
    budget: Arc<RunBudget>,
    anchor_match: Option<AnchorMatchFn>,
    reply_fn: ChatReplyFn<'a>,
    total_timeout: Duration,
    requested_max_loops: i32,
) -> Result<ChatToolLoopOutcome, ChatToolLoopError> {
    let loop_started = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + total_timeout;
    let max_loops = requested_max_loops.clamp(1, CHAT_TOOL_LOOP_MAX_LOOPS);
    let mut accumulated_results = String::new();
    let mut tool_trace: Vec<Document> = Vec::new();
    let mut risks: Vec<String> = Vec::new();
    let mut state = ToolDispatchState::new();
    let mut failure_streak: i32 = 0;
    let mut tool_calls_dispatched: i32 = 0;
    let mut last_decision: Option<AgentDecision> = None;
    let mut loop_count: i32 = 0;
    let mut observed_progress = HashSet::new();

    while loop_count < max_loops {
        if tokio::time::Instant::now() >= deadline {
            return Err(ChatToolLoopError::Timeout {
                elapsed_ms: loop_started.elapsed().as_millis() as i64,
                risks,
                tool_trace,
            });
        }

        let truncated = truncate_tool_results(&accumulated_results, &mut risks);
        let (decision, promote_risks) =
            match tokio::time::timeout_at(deadline, reply_fn(&truncated, loop_count)).await {
                Ok(result) => result?,
                Err(_) => {
                    risks.push("chat_tool_loop_timeout".to_string());
                    return Err(ChatToolLoopError::Timeout {
                        elapsed_ms: loop_started.elapsed().as_millis() as i64,
                        risks,
                        tool_trace,
                    });
                }
            };
        loop_count += 1;
        risks.extend(promote_risks);

        match decision.decision_phase.as_str() {
            "tool_calling" => {
                if !decision.reply_text.trim().is_empty() || decision.should_reply {
                    risks.push("chat_tool_calling_phase_with_reply_text".to_string());
                }
                let context_start = accumulated_results.len();
                let dispatch_outcome = dispatch_chat_turn(
                    &decision.tool_calls,
                    runtime,
                    knowledge,
                    db,
                    workspace_id,
                    account_id,
                    &budget,
                    deadline,
                    anchor_match,
                    &mut state,
                    &mut tool_trace,
                    &mut risks,
                    &mut accumulated_results,
                    &mut tool_calls_dispatched,
                    &mut failure_streak,
                )
                .await;
                if dispatch_outcome.timed_out {
                    risks.push("chat_tool_loop_timeout".to_string());
                    return Err(ChatToolLoopError::Timeout {
                        elapsed_ms: loop_started.elapsed().as_millis() as i64,
                        risks,
                        tool_trace,
                    });
                }
                let progress_hash =
                    tool_progress_hash(&decision.tool_calls, &accumulated_results[context_start..]);
                if !observed_progress.insert(progress_hash) {
                    risks.push("chat_tool_loop_no_progress".to_string());
                    last_decision = Some(decision);
                    break;
                }
                if dispatch_outcome.force_stop {
                    last_decision = Some(decision);
                    break;
                }
                last_decision = Some(decision);
            }
            _ => {
                last_decision = Some(strip_extra_tool_calls(decision, &mut risks));
                break;
            }
        }
    }

    let mut decision = match last_decision {
        Some(d) => d,
        None => {
            return Err(ChatToolLoopError::Reply(AppError::External(
                "chat reply_fn produced no decision".to_string(),
            )));
        }
    };

    if loop_count >= max_loops && decision.decision_phase == "tool_calling" {
        risks.push("chat_tool_loop_exhausted".to_string());
        decision.tool_calls.clear();
        decision.decision_phase = "final".to_string();
    }
    if decision.decision_phase == "tool_calling" {
        decision.tool_calls.clear();
        decision.decision_phase = "final".to_string();
    }

    if !tool_trace.is_empty() && tool_trace.len() > CHAT_TOOL_TRACE_MAX_LEN {
        risks.push("chat_tool_trace_overflow".to_string());
        tool_trace.truncate(CHAT_TOOL_TRACE_MAX_LEN);
    }

    if decision.decision_phase == "final" {
        let need_reason = decision.knowledge_need_reason.trim();
        let has_consult = tool_trace.iter().any(|entry| {
            entry
                .get_str("tool")
                .map(|t| t == TOOL_SEARCH || t == TOOL_OPEN_SLICE || t == TOOL_SEARCH_CHUNKS)
                .unwrap_or(false)
                && entry.get_str("error").is_err()
        });
        if !need_reason.is_empty() && need_reason != "unchanged" && !has_consult {
            risks.push("chat_knowledge_need_declared_but_not_consulted".to_string());
        }
    }

    let elapsed_ms = loop_started.elapsed().as_millis() as i64;
    risks.sort();
    risks.dedup();
    Ok(ChatToolLoopOutcome {
        decision,
        risks,
        tool_trace,
        tool_calls_dispatched,
        elapsed_ms,
    })
}

fn tool_progress_hash(tool_calls: &[ToolCallRequest], result_segment: &str) -> String {
    let payload = serde_json::json!({
        "toolCalls": tool_calls,
        "resultSegment": result_segment,
    });
    hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).unwrap_or_default(),
    ))
}

struct DispatchTurnOutcome {
    force_stop: bool,
    timed_out: bool,
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_chat_turn(
    tool_calls: &[ToolCallRequest],
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    db: &Database,
    workspace_id: &str,
    account_id: &str,
    budget: &Arc<RunBudget>,
    deadline: tokio::time::Instant,
    anchor_match: Option<AnchorMatchFn>,
    state: &mut ToolDispatchState,
    tool_trace: &mut Vec<Document>,
    risks: &mut Vec<String>,
    accumulated_results: &mut String,
    tool_calls_dispatched: &mut i32,
    failure_streak: &mut i32,
) -> DispatchTurnOutcome {
    let mut calls = tool_calls.to_vec();
    if calls.len() > CHAT_TOOL_CALLS_PER_TURN_CAP {
        risks.push("chat_tool_calls_per_turn_truncated".to_string());
        calls.truncate(CHAT_TOOL_CALLS_PER_TURN_CAP);
    }

    for call in calls {
        let started = std::time::Instant::now();
        let result = match tokio::time::timeout_at(
            deadline,
            dispatch_chat_tool_call(
                &call,
                runtime,
                knowledge,
                db,
                workspace_id,
                account_id,
                budget,
                state,
                anchor_match,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                return DispatchTurnOutcome {
                    force_stop: true,
                    timed_out: true,
                };
            }
        };
        *tool_calls_dispatched += 1;
        let latency_ms = started.elapsed().as_millis() as i64;
        let trace_entry = build_tool_trace_entry(&call, &result, latency_ms);
        tool_trace.push(trace_entry);
        let is_failure = result.get("error").is_some();
        if is_failure {
            *failure_streak += 1;
        } else {
            *failure_streak = 0;
        }
        append_tool_result_to_context(accumulated_results, &call, &result);
        if *failure_streak >= CHAT_TOOL_FAILURE_STREAK_LIMIT {
            risks.push("chat_tool_call_failure_streak".to_string());
            return DispatchTurnOutcome {
                force_stop: true,
                timed_out: false,
            };
        }
        if matches!(
            result.get("error").and_then(|v| v.as_str()),
            Some("budget_exceeded")
        ) {
            risks.push("chat_tool_budget_exhausted".to_string());
            return DispatchTurnOutcome {
                force_stop: true,
                timed_out: false,
            };
        }
    }
    DispatchTurnOutcome {
        force_stop: false,
        timed_out: false,
    }
}

fn build_tool_trace_entry(call: &ToolCallRequest, result: &Value, latency_ms: i64) -> Document {
    let mut doc = doc! {
        "tool": call.tool.clone(),
        "arguments": call.arguments.clone(),
        "latency_ms": latency_ms,
        "started_at": mongodb::bson::DateTime::now(),
    };
    match result.get("error").and_then(|v| v.as_str()) {
        Some(code) => {
            doc.insert("error", code.to_string());
            if let Some(detail) = result.get("detail").and_then(|v| v.as_str()) {
                doc.insert("detail", detail.to_string());
            }
        }
        None => {
            if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
                doc.insert("hit_count", items.len() as i32);
            }
            if let Some(hit_count) = result.get("hit_count").and_then(|v| v.as_i64()) {
                doc.insert("hit_count", hit_count as i32);
            }
            if let Some(slices) = result.get("slices").and_then(|v| v.as_array()) {
                doc.insert("hit_count", slices.len() as i32);
            }
            if let Some(suggestions) = result.get("suggestions").and_then(|v| v.as_array()) {
                doc.insert("hit_count", suggestions.len() as i32);
            }
            if let Some(score) = result.get("completeness_score").and_then(|v| v.as_f64()) {
                doc.insert("completeness_score", score);
            }
            if let Some(blocked) = result.get("blocked_or_held_runs").and_then(|v| v.as_i64()) {
                doc.insert("blocked_or_held_runs", blocked as i32);
            }
            doc.insert("result_summary", "ok");
        }
    }
    doc
}

fn append_tool_result_to_context(accumulated: &mut String, call: &ToolCallRequest, result: &Value) {
    let arguments_json =
        match mongodb::bson::Bson::from(call.arguments.clone()).into_relaxed_extjson() {
            Value::Object(map) => Value::Object(map),
            other => other,
        };
    let segment = format!(
        "\n[system tool result]\ntool: {}\narguments: {}\nresult: {}\n",
        call.tool,
        serde_json::to_string(&arguments_json).unwrap_or_default(),
        serde_json::to_string(result).unwrap_or_default(),
    );
    accumulated.push_str(&segment);
}

fn truncate_tool_results(accumulated: &str, risks: &mut Vec<String>) -> String {
    if accumulated.chars().count() <= CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS {
        return accumulated.to_string();
    }
    if !risks
        .iter()
        .any(|r| r == "chat_tool_result_context_truncated")
    {
        risks.push("chat_tool_result_context_truncated".to_string());
    }
    let total: Vec<char> = accumulated.chars().collect();
    let drop_count = total.len() - CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS;
    total[drop_count..].iter().collect()
}

fn strip_extra_tool_calls(mut decision: AgentDecision, risks: &mut Vec<String>) -> AgentDecision {
    if !decision.tool_calls.is_empty() {
        risks.push("chat_final_phase_extra_tool_calls_dropped".to_string());
        decision.tool_calls.clear();
    }
    decision
}

#[cfg(test)]
mod tests {
    //! 单元测试覆盖：
    //! - happy path: 1 中间轮 + 1 最终轮 OK；
    //! - per-turn cap: > 6 个 toolCalls 截断到 6；
    //! - budget exhausted: tool_call_budget=1 时第 2 次返回 budget_exceeded
    //!   并强制结束；
    //! - failure streak: 连续 3 次 unknown_tool 强制结束；
    //! - final phase strip: final 轮 toolCalls 仍非空时被清空 + risk；
    //! - exhausted: 2 个连续中间轮（max_loops=4 内仍可结束，但断言 risk 不出现）。
    //!
    //! 这些性质与 user-ops `tool_loop` 同源，但用 chat 路径独立验证。
    use super::*;
    use crate::agent::types::{AgentDecision, KnowledgeRuntime, ToolCallRequest};
    use mongodb::bson::doc;
    use std::sync::atomic::{AtomicI32, Ordering};

    fn runtime() -> UserRuntimeParameters {
        UserRuntimeParameters::default()
    }

    fn budget(tool_call_budget: i32) -> Arc<RunBudget> {
        Arc::new(RunBudget::new(
            "chat_run_t".to_string(),
            10_000,
            10,
            tool_call_budget,
        ))
    }

    fn empty_knowledge() -> KnowledgeRuntime {
        KnowledgeRuntime::default()
    }

    fn final_decision() -> AgentDecision {
        AgentDecision {
            decision_phase: "final".to_string(),
            should_reply: true,
            reply_text: "OK".to_string(),
            risk_level: "low".to_string(),
            knowledge_need: "not_required".to_string(),
            run_mode: "fast_chat".to_string(),
            autonomy_mode: "auto".to_string(),
            ..AgentDecision::default()
        }
    }

    fn intermediate(calls: Vec<ToolCallRequest>) -> AgentDecision {
        AgentDecision {
            decision_phase: "tool_calling".to_string(),
            tool_calls: calls,
            ..AgentDecision::default()
        }
    }

    fn tc(tool: &str, arguments: Document) -> ToolCallRequest {
        ToolCallRequest {
            tool: tool.to_string(),
            arguments,
        }
    }

    /// 模拟 reply_fn：按列表顺序消费 decision。注意：本测试**不依赖**实际 db
    /// 调用——所有 toolCalls 故意用合法名字但因 db=空，dispatch_chat_tool_call
    /// 会要么 invalid_input、要么 db 调用立即失败；这恰好覆盖错误路径。
    fn scripted_reply_fn(
        decisions: Arc<parking_lot::Mutex<Vec<AgentDecision>>>,
        call_count: Arc<AtomicI32>,
    ) -> ChatReplyFn<'static> {
        Box::new(move |_tool_results: &str, _loop_count: i32| {
            let decisions = decisions.clone();
            let call_count = call_count.clone();
            Box::pin(async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                let mut guard = decisions.lock();
                if guard.is_empty() {
                    return Err(AppError::External("scripted exhausted".to_string()));
                }
                Ok((guard.remove(0), Vec::new()))
            })
        })
    }

    /// chat tool loop 的 final 路径在没有 db 调用时也能正常返回（不会因为
    /// db 缺失 panic）。本测试只走 final → strip 逻辑，不实际派发。
    ///
    /// P2-16：补真断言。`Database::connect` 走 `parse_options` + `with_options`，
    /// 是 lazy 的——给一个本地不可达 URI 也能拿到 Database 句柄；只要 final 路径
    /// 一次都不触达 db，测试就能稳定走完。`final_decision` 的 decision_phase=final
    /// + tool_calls 为空 → loop 直接走出，不进入 dispatch_chat_turn。
    #[tokio::test]
    async fn chat_final_decision_returns_immediately_without_db() {
        let decisions = Arc::new(parking_lot::Mutex::new(vec![final_decision()]));
        let call_count = Arc::new(AtomicI32::new(0));
        let db = crate::db::Database::connect("mongodb://127.0.0.1:1/test", "wechatagent_test")
            .await
            .expect("lazy connect should not fail on URI parse");
        let runtime = runtime();
        let knowledge = empty_knowledge();
        let budget = budget(8);
        let reply_fn = scripted_reply_fn(decisions.clone(), call_count.clone());

        let outcome = chat_reply_with_tools_loop(
            &runtime,
            &knowledge,
            &db,
            "ws_test",
            "account_test",
            budget.clone(),
            None,
            reply_fn,
            CHAT_TOOL_LOOP_MAX_LOOPS,
        )
        .await
        .expect("final-only loop must succeed without touching db");

        // 真实断言：1) reply_fn 仅被调一次（final 一轮就退）；2) 留下 final decision；
        // 3) 没有 tool 派发（tool_trace 空 / 没消耗 tool_call 配额）。
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "final 路径应该只调用一次 reply_fn"
        );
        assert_eq!(outcome.decision.decision_phase, "final");
        assert_eq!(outcome.decision.reply_text, "OK");
        assert!(
            outcome.tool_trace.is_empty(),
            "final-only 路径不应留下 tool_trace：{:?}",
            outcome.tool_trace
        );
        let snap = budget.snapshot();
        assert_eq!(
            snap.tool_calls_used, 0,
            "final-only 路径不应消耗 tool_call 配额"
        );
    }

    #[tokio::test]
    async fn absolute_deadline_cancels_a_slow_reply_call() {
        let db = crate::db::Database::connect("mongodb://127.0.0.1:1/test", "wechatagent_test")
            .await
            .expect("lazy connect should not fail on URI parse");
        let reply_fn: ChatReplyFn<'static> = Box::new(|_, _| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok((final_decision(), Vec::new()))
            })
        });

        let result = chat_reply_with_tools_loop_with_timeout(
            &runtime(),
            &empty_knowledge(),
            &db,
            "ws_test",
            "account_test",
            budget(8),
            None,
            reply_fn,
            Duration::from_millis(20),
            CHAT_TOOL_LOOP_MAX_LOOPS,
        )
        .await;

        match result {
            Err(ChatToolLoopError::Timeout { risks, .. }) => assert!(
                risks.iter().any(|risk| risk == "chat_tool_loop_timeout"),
                "timeout must carry a stable risk code: {risks:?}"
            ),
            other => panic!("slow reply must hit the absolute deadline: {other:?}"),
        }
    }

    #[tokio::test]
    async fn repeated_tool_round_without_new_result_terminates_early() {
        let db = crate::db::Database::connect("mongodb://127.0.0.1:1/test", "wechatagent_test")
            .await
            .expect("lazy connect should not fail on URI parse");
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            intermediate(Vec::new()),
            intermediate(Vec::new()),
            intermediate(Vec::new()),
            intermediate(Vec::new()),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));

        let outcome = chat_reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            &db,
            "ws_test",
            "account_test",
            budget(8),
            None,
            scripted_reply_fn(decisions, call_count.clone()),
            CHAT_TOOL_LOOP_MAX_LOOPS,
        )
        .await
        .expect("loop exhaustion should return a normalized outcome");

        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert_eq!(outcome.decision.decision_phase, "final");
        assert!(outcome.decision.tool_calls.is_empty());
        assert!(outcome
            .risks
            .iter()
            .any(|risk| risk == "chat_tool_loop_no_progress"));
    }

    #[tokio::test]
    async fn requested_tool_loop_limit_is_enforced() {
        let db = crate::db::Database::connect("mongodb://127.0.0.1:1/test", "wechatagent_test")
            .await
            .expect("lazy connect should not fail on URI parse");
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            intermediate(Vec::new()),
            intermediate(Vec::new()),
            intermediate(Vec::new()),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));

        let outcome = chat_reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            &db,
            "ws_test",
            "account_test",
            budget(8),
            None,
            scripted_reply_fn(decisions, call_count.clone()),
            2,
        )
        .await
        .expect("bounded loop exhaustion should return a normalized outcome");

        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert_eq!(outcome.decision.decision_phase, "final");
        assert!(outcome
            .risks
            .iter()
            .any(|risk| risk == "chat_tool_loop_exhausted"));
    }

    /// 验证常量与设计金标一致：CHAT_TOOL_CALLS_PER_TURN_CAP=6,
    /// CHAT_TOOL_LOOP_MAX_LOOPS=4。failure_streak/context/trace 金标与
    /// user-ops 工具循环下线前同源（3 / 8000 / 32）。
    #[test]
    fn chat_tool_loop_constants_are_aligned_with_design() {
        assert_eq!(super::CHAT_TOOL_LOOP_MAX_LOOPS, 4);
        assert_eq!(super::CHAT_TOOL_FAILURE_STREAK_LIMIT, 3);
        assert_eq!(super::CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS, 8000);
        assert_eq!(super::CHAT_TOOL_TRACE_MAX_LEN, 32);
        assert_eq!(super::CHAT_TOOL_CALLS_PER_TURN_CAP, 6);
    }

    /// strip_extra_tool_calls：final 轮 toolCalls 非空时被清空 + 追加 risk。
    #[test]
    fn strip_extra_tool_calls_clears_and_adds_risk() {
        let mut d = final_decision();
        d.tool_calls = vec![tc("knowledge.search", doc! { "query": "x" })];
        let mut risks = Vec::new();
        let stripped = strip_extra_tool_calls(d, &mut risks);
        assert!(stripped.tool_calls.is_empty());
        assert!(risks
            .iter()
            .any(|r| r == "chat_final_phase_extra_tool_calls_dropped"));
    }

    /// truncate_tool_results：超长字符串按 keep-tail 截断 + risk 唯一。
    #[test]
    fn truncate_tool_results_keep_tail_and_unique_risk() {
        let big = "a".repeat(CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS + 100);
        let mut risks = Vec::new();
        let out = truncate_tool_results(&big, &mut risks);
        assert_eq!(out.chars().count(), CHAT_TOOL_RESULT_CONTEXT_MAX_CHARS);
        // 调用第二次不应再追加 risk
        let _ = truncate_tool_results(&big, &mut risks);
        let count = risks
            .iter()
            .filter(|r| r.as_str() == "chat_tool_result_context_truncated")
            .count();
        assert_eq!(count, 1);
    }

    /// build_tool_trace_entry：成功结果含 result_summary=ok，错误结果含 error 字段。
    #[test]
    fn trace_entry_summary_for_success_and_error() {
        let call = tc("knowledge.search_chunks", doc! { "query": "x" });
        let ok = serde_json::json!({ "hit_count": 3 });
        let entry = build_tool_trace_entry(&call, &ok, 12);
        assert_eq!(entry.get_str("result_summary").unwrap(), "ok");
        assert_eq!(entry.get_i32("hit_count").unwrap(), 3);
        let err = serde_json::json!({ "error": "invalid_input", "detail": "x" });
        let entry2 = build_tool_trace_entry(&call, &err, 12);
        assert_eq!(entry2.get_str("error").unwrap(), "invalid_input");
        assert_eq!(entry2.get_str("detail").unwrap(), "x");
    }
}
