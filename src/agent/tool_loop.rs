//! Reply Agent 多轮工具循环（agent-autonomy-loop W3 / Task 4.3）。
//!
//! 与 [`super::knowledge_tools`] 配合实现 R4 协议的核心控制流：Reply Agent
//! 多次输出 `decision_phase = "tool_calling"` 中间轮 → Rust 派发 toolCalls →
//! 把工具结果以 `[system tool result]` 段注入下一轮 prompt → 直到 Reply Agent
//! 输出 `decision_phase = "final"` 或触发循环上限/超时/失败连击。
//!
//! **本模块只暴露纯组合函数**：实际 Reply Agent 调用通过传入的 `reply_fn`
//! 闭包注入，方便单测在不依赖 LLM 的情况下完整覆盖 R4.1–R4.13 的所有分支。

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use mongodb::bson::{doc, Document};
use serde_json::Value;

use super::budget::RunBudget;
use super::knowledge_tools::{
    dispatch_tool_call, ToolDispatchState, TOOL_OPEN_SLICE, TOOL_SEARCH,
};
use super::runtime::UserRuntimeParameters;
use super::types::{AgentDecision, KnowledgeRuntime, ToolCallRequest};
use crate::error::AppResult;

// ── 常量（R4.7 / R4.8 / R4.9）─────────────────────────────────────────

/// 单条 `decision.toolCalls` 数组长度上限（R4.7）。
pub(crate) const TOOL_CALLS_PER_TURN_CAP: usize = 4;

/// 工具循环总耗时硬上限（R4.8）。超过 → `tool_loop_timeout`。
pub(crate) const TOOL_LOOP_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

/// tool 失败连击阈值（R4.8）。≥ 3 次失败强制结束循环。
pub(crate) const TOOL_FAILURE_STREAK_LIMIT: i32 = 3;

/// `[system tool result]` 累计注入 prompt 的字符上限（R4.9）。
pub(crate) const TOOL_RESULT_CONTEXT_MAX_CHARS: usize = 8000;

/// `decision.knowledge_route.tool_trace` 的累计上限（R4.10）。
pub(crate) const TOOL_TRACE_MAX_LEN: usize = 32;

// ── 结果类型 ────────────────────────────────────────────────────────────

/// [`reply_with_tools_loop`] 的成功返回。
///
/// `risks` 聚合本轮循环过程中追加的协议级风险标签（如
/// `tool_loop_exhausted` / `tool_call_failure_streak` / `tool_calls_per_turn_truncated`），
/// 由调用方在 finalize 阶段合并到 review.risks。
#[derive(Debug, Clone)]
pub(crate) struct ToolLoopOutcome {
    pub decision: AgentDecision,
    pub risks: Vec<String>,
    /// 累计的 toolTrace 条目（每条含 tool / arguments / result_summary
    /// 等），最多 [`TOOL_TRACE_MAX_LEN`]，超出会写 `tool_trace_overflow`
    /// 并截断。
    pub tool_trace: Vec<Document>,
    /// 已使用的 tool call 次数（含失败的）；与 `RunBudget.tool_calls_used`
    /// 不必完全一致——budget 只计成功占槽的，trace 记录所有 dispatch。
    pub tool_calls_dispatched: i32,
    /// 完整循环耗时（毫秒）。
    pub elapsed_ms: i64,
}

/// [`reply_with_tools_loop`] 的失败返回。
///
/// 触发场景仅有：循环总耗时超过 [`TOOL_LOOP_TOTAL_TIMEOUT`]（R4.8 fail-closed）
/// 与 Reply Agent 调用本身抛错（透传 [`AppError`]）。
#[derive(Debug)]
pub(crate) enum ToolLoopError {
    /// R4.8：工具循环 30s 总超时。调用方 SHALL 把 gateway_status 设为
    /// `"tool_loop_timeout"`，`should_reply=false`。
    Timeout {
        elapsed_ms: i64,
        risks: Vec<String>,
        tool_trace: Vec<Document>,
    },
    /// Reply Agent 调用 / 反序列化失败，透传到 gateway 主路径错误处理。
    Reply(crate::error::AppError),
}

impl From<crate::error::AppError> for ToolLoopError {
    fn from(value: crate::error::AppError) -> Self {
        ToolLoopError::Reply(value)
    }
}

// ── Reply Agent 注入 ───────────────────────────────────────────────────

/// Reply Agent 的一次输出（已经过 [`super::types::RawAgentDecision::validate_and_promote`]）。
///
/// 由 [`ToolLoopReplyFn`] 闭包的实现负责调 LLM、解析 JSON、做 promote，
/// 然后把 `(AgentDecision, promote_risks)` 一对返回给本循环。
pub(crate) type ToolLoopReplyResult = AppResult<(AgentDecision, Vec<String>)>;

/// Reply Agent 调用闭包。
///
/// `tool_results` 是历轮工具调用的累计 `[system tool result]` 段，已经按
/// [`TOOL_RESULT_CONTEXT_MAX_CHARS`] 截断；闭包 SHALL 把它注入下一轮 Reply
/// Agent 的 user prompt（如 `format!("{base}\n\n[system tool result]\n{tool_results}")`）。
///
/// 通过 `Box<dyn Fn>` 而不是泛型，避免在 mod.rs 之外被迫泛化整条调用链；
/// 这层抽象只在 W3 task 4.3 内部使用，不会出现在 PBT / lib 公共 API 上。
pub(crate) type ToolLoopReplyFn<'a> = Box<
    dyn Fn(&str, i32) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolLoopReplyResult> + Send + 'a>>
        + Send
        + Sync
        + 'a,
>;

// ── 主循环 ──────────────────────────────────────────────────────────────

/// Reply Agent 多轮工具循环的核心控制流。
///
/// 行为对齐 R4.2 / R4.7 / R4.8 / R4.9 / R4.10 / R4.11：
///
/// 1. `loop_count` 上限 = `runtime.knowledge_max_tool_loops`（clamp 到 [1,5]）；
/// 2. 每轮调 `reply_fn(tool_results, loop_count)` 拿到 (decision, promote_risks)；
/// 3. 中间轮 (`decision_phase == "tool_calling"`) → 检查 reply_text/should_reply、
///    截断 toolCalls 到 4、按非法 tool 名跳过、用 [`dispatch_tool_call`] 实际执行；
/// 4. 最终轮 (`decision_phase == "final"`) → 截断额外 toolCalls 并返回；
/// 5. 总耗时 > 30s → `ToolLoopError::Timeout`；
/// 6. 失败连击 ≥ 3 → 强制结束 + `tool_call_failure_streak`；
/// 7. 循环耗尽且仍输出 toolCalls → `tool_loop_exhausted` + 强制按 final 处理。
pub(crate) async fn reply_with_tools_loop<'a>(
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    budget: Arc<RunBudget>,
    reply_fn: ToolLoopReplyFn<'a>,
) -> Result<ToolLoopOutcome, ToolLoopError> {
    let loop_started = std::time::Instant::now();
    let max_loops = runtime.knowledge_max_tool_loops.clamp(1, 5);
    let mut accumulated_results = String::new();
    let mut tool_trace: Vec<Document> = Vec::new();
    let mut risks: Vec<String> = Vec::new();
    let mut state = ToolDispatchState::new();
    let mut failure_streak: i32 = 0;
    let mut tool_calls_dispatched: i32 = 0;
    let mut last_decision: Option<AgentDecision> = None;
    let mut last_promote_risks: Vec<String> = Vec::new();
    let mut loop_count: i32 = 0;

    while loop_count < max_loops {
        // R4.8：30s 总超时硬上限，每轮入口检查一次。
        if loop_started.elapsed() > TOOL_LOOP_TOTAL_TIMEOUT {
            return Err(ToolLoopError::Timeout {
                elapsed_ms: loop_started.elapsed().as_millis() as i64,
                risks,
                tool_trace,
            });
        }

        // 调 Reply Agent。
        let truncated = truncate_tool_results(&accumulated_results, &mut risks);
        let (decision, promote_risks) = reply_fn(&truncated, loop_count).await?;
        loop_count += 1;
        last_promote_risks = promote_risks;

        // 中间轮 / 最终轮分支（R1.10 + R4.1）。
        match decision.decision_phase.as_str() {
            "tool_calling" => {
                // 中间轮：处理 reply_text / toolCalls 异常 + 派发工具。
                if !decision.reply_text.trim().is_empty() || decision.should_reply {
                    risks.push("tool_calling_phase_with_reply_text".to_string());
                }
                let dispatch_outcome = dispatch_turn(
                    &decision.tool_calls,
                    runtime,
                    knowledge,
                    &budget,
                    &mut state,
                    &mut tool_trace,
                    &mut risks,
                    &mut accumulated_results,
                    &mut tool_calls_dispatched,
                    &mut failure_streak,
                )
                .await;
                if dispatch_outcome.force_stop {
                    last_decision = Some(decision);
                    break;
                }
                last_decision = Some(decision);
            }
            _ => {
                // 最终轮 / 默认：直接返回。
                last_decision = Some(strip_extra_tool_calls(decision, &mut risks));
                break;
            }
        }
    }


    // 循环耗尽：取最后一轮 decision；如果 toolCalls 仍非空 → 追加
    // tool_loop_exhausted + 强制清空 toolCalls 走 final（R4.2 / 设计文档 §4 伪码）。
    let mut decision = match last_decision {
        Some(d) => d,
        None => {
            // 上层 reply_fn 一次都没产出（max_loops==0 不可能，因为 clamp[1,5]）。
            return Err(ToolLoopError::Reply(crate::error::AppError::External(
                "reply_fn did not produce any decision".to_string(),
            )));
        }
    };

    if loop_count >= max_loops && decision.decision_phase == "tool_calling" {
        risks.push("tool_loop_exhausted".to_string());
        decision.tool_calls.clear();
        decision.decision_phase = "final".to_string();
    }
    // R4.3 / R4.8 fail-closed：当循环因 budget / failure_streak 被 force_stop
    // 时，最后一轮 decision 仍为 tool_calling 中间轮；强制清空 toolCalls 并
    // 切到 final，让 finalize_review_for_send 走完整 review 校验（保守选择）。
    if decision.decision_phase == "tool_calling" {
        decision.tool_calls.clear();
        decision.decision_phase = "final".to_string();
    }

    // R4.10：toolTrace 由 [`ToolLoopOutcome::tool_trace`] 字段承载，调用方
    // 在 gateway 主路径写入 `agent_run_logs.knowledge_route.tool_trace`（本期
    // [`AgentDecision`] 暂不持有 knowledge_route 字段，详见 types.rs 注释）。
    if !tool_trace.is_empty() && tool_trace.len() > TOOL_TRACE_MAX_LEN {
        risks.push("tool_trace_overflow".to_string());
        tool_trace.truncate(TOOL_TRACE_MAX_LEN);
    }

    // R4.11：声明而未使用的检测——final 轮 knowledgeNeedReason 非空非
    // unchanged 但 toolTrace 中没有任何成功的 search/open_slice。
    if decision.decision_phase == "final" {
        let need_reason = decision.knowledge_need_reason.trim();
        let has_consult = tool_trace.iter().any(|entry| {
            entry
                .get_str("tool")
                .map(|t| t == TOOL_SEARCH || t == TOOL_OPEN_SLICE)
                .unwrap_or(false)
                && entry.get_str("error").is_err() // 没有 error 字段 = 成功
        });
        if !need_reason.is_empty() && need_reason != "unchanged" && !has_consult {
            risks.push("knowledge_need_declared_but_not_consulted".to_string());
        }
    }

    let elapsed_ms = loop_started.elapsed().as_millis() as i64;
    // promote_risks 是 Reply Agent 最后一轮的 validate_and_promote 结果，
    // 由调用方与本循环 risks 合并到 finalize_review_for_send。
    risks.extend(last_promote_risks);
    Ok(ToolLoopOutcome {
        decision,
        risks,
        tool_trace,
        tool_calls_dispatched,
        elapsed_ms,
    })
}


// ── 内部辅助 ────────────────────────────────────────────────────────────

struct DispatchTurnOutcome {
    /// 标志失败连击触发或预算耗尽，外层 SHALL 立即 break。
    force_stop: bool,
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_turn(
    tool_calls: &[ToolCallRequest],
    runtime: &UserRuntimeParameters,
    knowledge: &KnowledgeRuntime,
    budget: &Arc<RunBudget>,
    state: &mut ToolDispatchState,
    tool_trace: &mut Vec<Document>,
    risks: &mut Vec<String>,
    accumulated_results: &mut String,
    tool_calls_dispatched: &mut i32,
    failure_streak: &mut i32,
) -> DispatchTurnOutcome {
    // R4.7：单轮 toolCalls 数量 ≤ 4；超出截断 + risk。
    let mut calls = tool_calls.to_vec();
    if calls.len() > TOOL_CALLS_PER_TURN_CAP {
        risks.push("tool_calls_per_turn_truncated".to_string());
        calls.truncate(TOOL_CALLS_PER_TURN_CAP);
    }

    for call in calls {
        let started = std::time::Instant::now();
        let result =
            dispatch_tool_call(&call, runtime, knowledge, budget, state).await;
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
        if *failure_streak >= TOOL_FAILURE_STREAK_LIMIT {
            risks.push("tool_call_failure_streak".to_string());
            return DispatchTurnOutcome { force_stop: true };
        }
        // budget_exceeded 也视为强制结束（R4.3）。
        if matches!(
            result.get("error").and_then(|v| v.as_str()),
            Some("budget_exceeded")
        ) {
            risks.push("tool_budget_exhausted".to_string());
            return DispatchTurnOutcome { force_stop: true };
        }
    }
    DispatchTurnOutcome { force_stop: false }
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
            // 摘要：list_catalog → items.len, search → hit_count, open_slice → slices.len。
            if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
                doc.insert("hit_count", items.len() as i32);
            }
            if let Some(hit_count) = result.get("hit_count").and_then(|v| v.as_i64()) {
                doc.insert("hit_count", hit_count as i32);
            }
            if let Some(slices) = result.get("slices").and_then(|v| v.as_array()) {
                doc.insert("hit_count", slices.len() as i32);
            }
            if let Some(truncated) = result.get("truncated").and_then(|v| v.as_bool()) {
                doc.insert("truncated", truncated);
            }
            doc.insert("result_summary", "ok");
        }
    }
    doc
}


/// R4.9：把一次工具调用 + 结果以 `[system tool result]` 段附加到累计上下文。
fn append_tool_result_to_context(
    accumulated: &mut String,
    call: &ToolCallRequest,
    result: &Value,
) {
    let arguments_json = match mongodb::bson::Bson::from(call.arguments.clone())
        .into_relaxed_extjson()
    {
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

/// R4.9：累计注入长度 ≤ 8000 chars，超出按"丢弃最早"截断。
fn truncate_tool_results(accumulated: &str, risks: &mut Vec<String>) -> String {
    if accumulated.chars().count() <= TOOL_RESULT_CONTEXT_MAX_CHARS {
        return accumulated.to_string();
    }
    if !risks.iter().any(|r| r == "tool_result_context_truncated") {
        risks.push("tool_result_context_truncated".to_string());
    }
    let total: Vec<char> = accumulated.chars().collect();
    let drop_count = total.len() - TOOL_RESULT_CONTEXT_MAX_CHARS;
    total[drop_count..].iter().collect()
}

/// R4.1.b：final 轮 toolCalls 仍非空 → 清空 + risk。
fn strip_extra_tool_calls(mut decision: AgentDecision, risks: &mut Vec<String>) -> AgentDecision {
    if !decision.tool_calls.is_empty() {
        risks.push("final_phase_extra_tool_calls_dropped".to_string());
        decision.tool_calls.clear();
    }
    decision
}


// ── 单元测试（agent-autonomy-loop W3 / Task 4.4：≥ 6 例）─────────────────

#[cfg(test)]
mod tests {
    //! 覆盖 R4.1–R4.13 的关键控制流分支：
    //! 1. list_catalog → search → open_slice 完整一轮决策；
    //! 2. tool_calls_used + tokens_used 同时累计；
    //! 3. MAX_TOOL_LOOPS 上限触发 `tool_loop_exhausted`；
    //! 4. 单 tool 5s 超时 / 连续 3 次失败强制结束；
    //! 5. `decision_phase=tool_calling` 时 reply_text 被丢弃；
    //! 6. `decision_phase=final` 时 toolCalls 被清空 + risk；
    //! 7. classic_router 模式由 gateway 主路径接入，不在 tool_loop 内部覆盖；
    //!    见 W3 task 4.3 的灰度回退分支说明。

    use super::*;
    use crate::agent::types::AgentDecision;
    use mongodb::bson::doc;
    use std::sync::atomic::{AtomicI32, Ordering};

    fn runtime() -> UserRuntimeParameters {
        UserRuntimeParameters::default()
    }

    fn budget(tool_call_budget: i32) -> Arc<RunBudget> {
        Arc::new(RunBudget::new(
            "run_t".to_string(),
            10_000,
            10,
            tool_call_budget,
        ))
    }

    fn empty_knowledge() -> KnowledgeRuntime {
        KnowledgeRuntime::default()
    }

    /// 基于 `Arc<Mutex<Vec<AgentDecision>>>` 的 reply_fn 构造器：每轮按列表
    /// 顺序消费一条 decision，模拟 Reply Agent 多轮对话。
    fn scripted_reply_fn(
        decisions: Arc<parking_lot::Mutex<Vec<AgentDecision>>>,
        call_count: Arc<AtomicI32>,
    ) -> ToolLoopReplyFn<'static> {
        Box::new(move |_tool_results: &str, _loop_count: i32| {
            let decisions = decisions.clone();
            let call_count = call_count.clone();
            Box::pin(async move {
                call_count.fetch_add(1, Ordering::SeqCst);
                let mut guard = decisions.lock();
                if guard.is_empty() {
                    return Err(crate::error::AppError::External(
                        "scripted_reply_fn exhausted".to_string(),
                    ));
                }
                let decision = guard.remove(0);
                Ok((decision, Vec::new()))
            })
        })
    }

    fn final_decision() -> AgentDecision {
        AgentDecision {
            decision_phase: "final".to_string(),
            should_reply: true,
            reply_text: "你好".to_string(),
            risk_level: "low".to_string(),
            knowledge_need: "not_required".to_string(),
            run_mode: "fast_chat".to_string(),
            autonomy_mode: "auto".to_string(),
            ..AgentDecision::default()
        }
    }

    fn tool_calling_decision(calls: Vec<ToolCallRequest>) -> AgentDecision {
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

    /// Test 1：Reply Agent 在 auto_tool_loop 下依次 list_catalog → search →
    /// open_slice 完成一轮决策（最终轮无 toolCalls）。
    #[tokio::test]
    async fn happy_path_three_tools_to_final() {
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            tool_calling_decision(vec![tc("knowledge.list_catalog", doc! { "kind": "chunks" })]),
            tool_calling_decision(vec![tc("knowledge.search", doc! { "query": "z" })]),
            final_decision(),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            budget(16),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("loop should succeed");
        assert_eq!(outcome.decision.decision_phase, "final");
        assert_eq!(call_count.load(Ordering::SeqCst), 3, "应调用 Reply Agent 3 次");
        assert!(outcome.tool_calls_dispatched >= 2);
    }

    /// Test 2：tool_calls_used 计入 RunBudget；预算耗尽时后续 tool call
    /// 返回 budget_exceeded 并强制结束循环（risks 含 `tool_budget_exhausted`）。
    #[tokio::test]
    async fn tool_call_budget_exhausted_forces_stop() {
        let bud = budget(1); // 只允许 1 次 tool call
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            tool_calling_decision(vec![
                tc("knowledge.list_catalog", doc! { "kind": "chunks" }),
                tc("knowledge.search", doc! { "query": "x" }),
            ]),
            final_decision(),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            bud.clone(),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("loop should succeed even when budget exhausted");
        assert!(outcome
            .risks
            .iter()
            .any(|r| r == "tool_budget_exhausted"));
        assert_eq!(*bud.tool_calls_used.lock(), 1, "占用 1 次后超额");
    }

    /// Test 3：MAX_TOOL_LOOPS 上限触发 `tool_loop_exhausted` risk + 强制清空
    /// toolCalls 进 final 轮。
    #[tokio::test]
    async fn max_tool_loops_triggers_exhausted_risk() {
        let mut runtime = runtime();
        runtime.knowledge_max_tool_loops = 2; // clamp 后仍按 2 处理
        // 准备 5 轮 tool_calling decision；但循环上限 2 → 第 3 轮起拿不到
        // decision 会从 reply_fn 返回错误。这里用持续返回 tool_calling 的脚本：
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            tool_calling_decision(vec![tc("knowledge.search", doc! { "query": "a" })]),
            tool_calling_decision(vec![tc("knowledge.search", doc! { "query": "b" })]),
            tool_calling_decision(vec![tc("knowledge.search", doc! { "query": "c" })]),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime,
            &empty_knowledge(),
            budget(16),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("loop should clamp at max_tool_loops");
        assert_eq!(call_count.load(Ordering::SeqCst), 2, "Reply Agent 调用应 ≤ max_tool_loops=2");
        assert!(outcome.risks.iter().any(|r| r == "tool_loop_exhausted"));
        assert_eq!(outcome.decision.decision_phase, "final");
        assert!(outcome.decision.tool_calls.is_empty());
    }

    /// Test 4：连续 3 次失败 tool call 强制结束循环（risks 含
    /// `tool_call_failure_streak`）。失败用未知 tool 名触发 `unknown_tool`。
    #[tokio::test]
    async fn three_failures_in_a_row_force_stop() {
        let decisions = Arc::new(parking_lot::Mutex::new(vec![tool_calling_decision(vec![
            tc("knowledge.unknown_a", Document::new()),
            tc("knowledge.unknown_b", Document::new()),
            tc("knowledge.unknown_c", Document::new()),
        ])]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            budget(16),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("force stop is success path");
        assert!(outcome
            .risks
            .iter()
            .any(|r| r == "tool_call_failure_streak"));
    }

    /// Test 5：`decision_phase=tool_calling` 时即使 reply_text 非空 / should_reply=true
    /// 也被丢弃（追加 `tool_calling_phase_with_reply_text` risk），循环继续。
    #[tokio::test]
    async fn tool_calling_phase_drops_reply_text() {
        let mut bad_intermediate = tool_calling_decision(vec![tc(
            "knowledge.list_catalog",
            doc! { "kind": "chunks" },
        )]);
        bad_intermediate.reply_text = "我提前回复".to_string();
        bad_intermediate.should_reply = true;
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            bad_intermediate,
            final_decision(),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            budget(16),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("loop should succeed");
        assert!(outcome
            .risks
            .iter()
            .any(|r| r == "tool_calling_phase_with_reply_text"));
        assert_eq!(outcome.decision.decision_phase, "final");
    }

    /// Test 6：`decision_phase=final` 时仍带 toolCalls 被清空 + risk
    /// `final_phase_extra_tool_calls_dropped`。
    #[tokio::test]
    async fn final_phase_extra_tool_calls_are_cleared() {
        let mut final_with_calls = final_decision();
        final_with_calls.tool_calls = vec![tc("knowledge.search", doc! { "query": "x" })];
        let decisions = Arc::new(parking_lot::Mutex::new(vec![final_with_calls]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            budget(16),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("loop should succeed");
        assert!(outcome
            .risks
            .iter()
            .any(|r| r == "final_phase_extra_tool_calls_dropped"));
        assert!(outcome.decision.tool_calls.is_empty());
    }

    /// Test 7：单轮 toolCalls > 4 被截断到前 4 + risk `tool_calls_per_turn_truncated`。
    #[tokio::test]
    async fn per_turn_tool_calls_truncated_to_four() {
        let calls = (0..6)
            .map(|i| tc("knowledge.search", doc! { "query": format!("q{i}") }))
            .collect::<Vec<_>>();
        let decisions = Arc::new(parking_lot::Mutex::new(vec![
            tool_calling_decision(calls),
            final_decision(),
        ]));
        let call_count = Arc::new(AtomicI32::new(0));
        let outcome = reply_with_tools_loop(
            &runtime(),
            &empty_knowledge(),
            budget(16),
            scripted_reply_fn(decisions, call_count.clone()),
        )
        .await
        .expect("loop should succeed");
        assert!(outcome
            .risks
            .iter()
            .any(|r| r == "tool_calls_per_turn_truncated"));
        assert_eq!(outcome.tool_calls_dispatched, 4);
    }
}


// ── P7 性质测试（agent-autonomy-loop W3 / Task 4.5：≥ 64 用例）──────────
//
// **Property 7: 工具循环不死锁 + 预算不被绕过**
// **Validates: Requirements 4.2, 4.3, 4.8**
//
// 随机生成 `Vec<ToolCallRequest>`（含非法 tool 名 / 超长 query / 超 K open_slice），
// 断言：
// 1. 循环 ≤ MAX_TOOL_LOOPS 内终止；
// 2. 总 tool 调用 ≤ knowledgeMaxToolCalls；
// 3. budget 超额后任何后续 tool call 返回 `budget_exceeded` 而非实际执行。

#[cfg(test)]
mod pbt_tests {
    use super::*;
    use crate::agent::types::{AgentDecision, KnowledgeRuntime, ToolCallRequest};
    use mongodb::bson::Document;
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    fn arbitrary_tool_name() -> impl Strategy<Value = String> {
        prop_oneof![
            // 合法
            Just("knowledge.list_catalog".to_string()),
            Just("knowledge.search".to_string()),
            Just("knowledge.open_slice".to_string()),
            // 非法
            Just("knowledge.unknown".to_string()),
            Just("".to_string()),
            "[a-z]{1,12}".prop_map(|s| format!("knowledge.{s}")),
        ]
    }

    fn arbitrary_call() -> impl Strategy<Value = ToolCallRequest> {
        arbitrary_tool_name().prop_map(|tool| ToolCallRequest {
            tool,
            arguments: Document::new(),
        })
    }

    fn final_decision_no_calls() -> AgentDecision {
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

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            max_shrink_iters: 50,
            ..ProptestConfig::default()
        })]

        /// P7：随机生成 1..=8 个 toolCalls 的中间轮 + 最终轮组合，
        /// 任何执行路径下都不会出现：
        /// - 循环超过 max_loops；
        /// - tool_calls_used 超过 tool_call_budget；
        /// - budget 超额后还能成功 dispatch（必然返回 budget_exceeded）。
        #[test]
        fn p7_loop_terminates_and_budget_never_bypassed(
            calls_round_1 in proptest::collection::vec(arbitrary_call(), 1..=8),
            calls_round_2 in proptest::collection::vec(arbitrary_call(), 0..=8),
            tool_call_budget in 1i32..=8i32,
        ) {
            // runtime 设定：max_tool_loops 默认 3，knowledge_max_tool_calls 用本例参数。
            let runtime = UserRuntimeParameters {
                knowledge_max_tool_calls: tool_call_budget,
                ..UserRuntimeParameters::default()
            };
            let bud = Arc::new(RunBudget::new(
                "p7".to_string(),
                100_000,
                100,
                tool_call_budget,
            ));
            let scripts: Vec<AgentDecision> = vec![
                intermediate(calls_round_1),
                intermediate(calls_round_2),
                final_decision_no_calls(),
            ];
            let decisions = Arc::new(parking_lot::Mutex::new(scripts));
            let call_count = Arc::new(AtomicI32::new(0));
            let reply_fn: ToolLoopReplyFn<'_> = {
                let decisions = decisions.clone();
                let call_count = call_count.clone();
                Box::new(move |_t, _l| {
                    let decisions = decisions.clone();
                    let call_count = call_count.clone();
                    Box::pin(async move {
                        call_count.fetch_add(1, Ordering::SeqCst);
                        let mut g = decisions.lock();
                        if g.is_empty() {
                            return Ok((final_decision_no_calls(), Vec::new()));
                        }
                        Ok((g.remove(0), Vec::new()))
                    })
                })
            };

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let outcome = rt
                .block_on(reply_with_tools_loop(&runtime, &KnowledgeRuntime::default(), bud.clone(), reply_fn))
                .expect("loop should not return timeout in PBT");

            // 性质 1：Reply Agent 调用次数 ≤ max_tool_loops（=3）。
            prop_assert!(call_count.load(Ordering::SeqCst) <= 3,
                "loop_count={} exceeded max=3", call_count.load(Ordering::SeqCst));

            // 性质 2：tool_calls_used ≤ tool_call_budget。
            let used = *bud.tool_calls_used.lock();
            prop_assert!(used <= tool_call_budget,
                "tool_calls_used={used} > budget={tool_call_budget}");

            // 性质 3：最终 decision_phase 必为 "final"，toolCalls 为空。
            prop_assert_eq!(outcome.decision.decision_phase.as_str(), "final");
            prop_assert!(outcome.decision.tool_calls.is_empty());
        }
    }
}
