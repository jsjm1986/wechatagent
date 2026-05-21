//! 单次 run 的 LLM / Tool 预算计数 (MP-5 / Task 15 + agent-autonomy-loop W3 / Task 4.1)。
//!
//! 通过 `tokio::task_local!` 把 [`RunBudget`] 注入到当前 run 的子 future，
//! 所有 `generate_agent_json` 调用自动累加 token 与调用次数；调用方可用
//! [`RunBudget::is_exceeded`] 判断是否需要进入降级路径（跳过 review、跳过
//! rewrite、跳过二次知识路由等）。
//!
//! agent-autonomy-loop W3 / Task 4.1：
//! - 新增 `tool_call_budget` / `tool_calls_used`（与 LLM 计数独立）。
//! - 新增 [`RunBudget::record_tool_call`]，原子地同时检查并累加 tool_calls
//!   与 tokens；超额返回 [`BudgetError`]，对应 R4.3 的两类硬上限。
//! - `tool_call_budget` 由 `runtime_parameters.knowledge_max_tool_calls`
//!   （默认 6，clamp 到 `[1,16]`，在 `UserRuntimeParameters::from_config`
//!   中完成 clamp）注入；非 tool-loop 的入口（test / fallback）传 `i32::MAX`
//!   表示不限。

use std::sync::Arc;

use parking_lot::Mutex as PlMutex;

/// agent-autonomy-loop W3 / Task 4.1：预算硬上限错误，由
/// [`RunBudget::record_tool_call`] 在 dispatch tool call 前抛出。
///
/// 由 dispatcher 把 `Err(BudgetError::*)` 转换为 R4.3 / R4.8 规定的
/// `{ "error": "budget_exceeded", "detail": "..." }` 工具结果，回传给
/// Reply Agent；调用 LLM 用的 [`RunBudget::record_call`] 不抛错（保留向
/// 兼容）。
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub(crate) enum BudgetError {
    /// `tool_calls_used >= tool_call_budget`：任何后续 tool call SHALL
    /// 立即返回 `budget_exceeded` 而不实际执行（R4.3）。
    #[error(
        "tool_call budget exceeded: tool_calls_used={used} >= tool_call_budget={budget}"
    )]
    ToolCallsExceeded { used: i32, budget: i32 },
    /// `tokens_used + tokens_consumed > token_budget`：本次 tool call 想
    /// 累计的 snippet / body token 数会越过 token 硬上限（R4.3）。
    #[error(
        "token budget exceeded: tokens_used={used} + consumed={consumed} > token_budget={budget}"
    )]
    TokensExceeded {
        used: i64,
        consumed: i64,
        budget: i64,
    },
}

/// 单次 run 的 LLM / Tool 预算计数器。
///
/// 调用方在关键决策点（review、rewrite、router 二次等）前用
/// [`RunBudget::is_exceeded`] 判断是否走降级路径；MCP knowledge tool
/// dispatcher 在每次实际 dispatch 前先调 [`RunBudget::record_tool_call`]
/// 原子地占用 1 次 tool call + N tokens，越界返回 [`BudgetError`]。
pub struct RunBudget {
    pub run_id: String,
    pub token_budget: i64,
    pub max_llm_calls: i32,
    /// agent-autonomy-loop W3 / Task 4.1：单 run tool call 数硬上限。
    /// 由 `runtime_parameters.knowledge_max_tool_calls` 注入（默认 6，
    /// clamp 到 `[1,16]`）；test / 无 tool-loop fallback 路径传 `i32::MAX`。
    pub tool_call_budget: i32,
    pub tokens_used: PlMutex<i64>,
    pub llm_calls_used: PlMutex<i32>,
    /// agent-autonomy-loop W3 / Task 4.1：单 run 已执行的 tool call 累计数；
    /// 与 [`Self::llm_calls_used`] **独立** 计数（R4.3：1 次 LLM = 0 次 tool）。
    pub tool_calls_used: PlMutex<i32>,
    pub degraded_reasons: PlMutex<Vec<String>>,
}

impl RunBudget {
    pub fn new(
        run_id: impl Into<String>,
        token_budget: i64,
        max_llm_calls: i32,
        tool_call_budget: i32,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            token_budget,
            max_llm_calls,
            tool_call_budget,
            tokens_used: PlMutex::new(0),
            llm_calls_used: PlMutex::new(0),
            tool_calls_used: PlMutex::new(0),
            degraded_reasons: PlMutex::new(Vec::new()),
        }
    }

    /// 累计 1 次 LLM 调用与本次返回的 token 数。
    /// 不会检查上限——is_exceeded() 由调用方在下一个降级点 / revision
    /// 入口负责检查。
    pub fn record_call(&self, tokens: i64) {
        *self.tokens_used.lock() += tokens.max(0);
        *self.llm_calls_used.lock() += 1;
    }

    /// agent-autonomy-loop W3 / Task 4.1：原子地检查并累计 1 次 tool call
    /// 以及它消费的 token 数（snippet / body 字符长度）。
    ///
    /// 行为对齐 R4.3：
    /// 1. `tool_calls_used >= tool_call_budget` → `Err(ToolCallsExceeded)`，
    ///    **不**累加任何计数；
    /// 2. `tokens_used + tokens_consumed > token_budget` → `Err(TokensExceeded)`，
    ///    **不**累加任何计数；
    /// 3. 否则同时增加 `tool_calls_used += 1` 与 `tokens_used += tokens_consumed`，
    ///    返回 `Ok(())`。
    ///
    /// 两把锁按固定顺序（tokens → tool_calls）抢占，避免与其它路径死锁。
    /// 任何 LLM 调用入口（[`Self::record_call`]）只锁 `tokens_used` 与
    /// `llm_calls_used`，不与本方法竞争 `tool_calls_used`，所以不会形成环。
    ///
    /// `dead_code` allow：MCP knowledge.* dispatcher 在 Task 4.2 才接入，
    /// 当前 lib build 中暂无调用方，但 Task 4.1 单测已覆盖三条主路径。
    #[allow(dead_code)]
    pub fn record_tool_call(&self, tokens_consumed: i64) -> Result<(), BudgetError> {
        let consumed = tokens_consumed.max(0);
        let mut tokens = self.tokens_used.lock();
        let mut tool_calls = self.tool_calls_used.lock();
        if *tool_calls >= self.tool_call_budget {
            return Err(BudgetError::ToolCallsExceeded {
                used: *tool_calls,
                budget: self.tool_call_budget,
            });
        }
        if (*tokens).saturating_add(consumed) > self.token_budget {
            return Err(BudgetError::TokensExceeded {
                used: *tokens,
                consumed,
                budget: self.token_budget,
            });
        }
        *tokens += consumed;
        *tool_calls += 1;
        Ok(())
    }

    /// agent-autonomy-loop W3 / Task 4.1：在原 token / LLM 双维度的
    /// "超额"判定基础上，叠加 tool_calls 维度——任一硬上限达到即视为
    /// 超额，触发降级路径（跳过 review / rewrite / 二次知识路由）。
    pub fn is_exceeded(&self) -> bool {
        *self.tokens_used.lock() >= self.token_budget
            || *self.llm_calls_used.lock() >= self.max_llm_calls
            || *self.tool_calls_used.lock() >= self.tool_call_budget
    }

    pub fn mark_degraded(&self, reason: impl Into<String>) {
        self.degraded_reasons.lock().push(reason.into());
    }

    pub fn snapshot(&self) -> RunBudgetSnapshot {
        RunBudgetSnapshot {
            run_id: self.run_id.clone(),
            token_budget: self.token_budget,
            max_llm_calls: self.max_llm_calls,
            tool_call_budget: self.tool_call_budget,
            tokens_used: *self.tokens_used.lock(),
            llm_calls_used: *self.llm_calls_used.lock(),
            tool_calls_used: *self.tool_calls_used.lock(),
            degraded_reasons: self.degraded_reasons.lock().clone(),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct RunBudgetSnapshot {
    pub run_id: String,
    pub token_budget: i64,
    pub max_llm_calls: i32,
    pub tool_call_budget: i32,
    pub tokens_used: i64,
    pub llm_calls_used: i32,
    pub tool_calls_used: i32,
    pub degraded_reasons: Vec<String>,
}

tokio::task_local! {
    /// 当前 run 的 budget；只在 `run_user_operation_gateway` /
    /// `simulate_user_dialogue` / `consolidate_contact_memory` 等入口
    /// 通过 `RUN_BUDGET.scope(...)` 设置。
    pub(crate) static RUN_BUDGET: Arc<RunBudget>;
}

/// 安全获取当前 task-local 的 budget；不在 scope 内时返回 None。
pub(crate) fn current_run_budget() -> Option<Arc<RunBudget>> {
    RUN_BUDGET.try_with(Arc::clone).ok()
}

#[cfg(test)]
mod tests {
    //! agent-autonomy-loop W3 / Task 4.1：单元测试覆盖 `record_tool_call`
    //! 的三条路径——成功累加、tool_call 超额、token 超额；并断言失败路径
    //! 不污染计数器（atomic 语义）。`is_exceeded()` 维度叠加由 mod.rs
    //! 的既有测试 + 这里的 tool_calls_used 上限测试共同覆盖。

    use super::{BudgetError, RunBudget};

    #[test]
    fn record_tool_call_increments_both_counters_on_success() {
        let budget = RunBudget::new("run_t", 10_000, 10, 4);
        budget.record_tool_call(120).expect("first call ok");
        budget.record_tool_call(80).expect("second call ok");
        let snap = budget.snapshot();
        assert_eq!(snap.tool_calls_used, 2);
        assert_eq!(snap.tokens_used, 200);
        // 不应触动 LLM 计数器（R4.3：tool != LLM）。
        assert_eq!(snap.llm_calls_used, 0);
    }

    #[test]
    fn record_tool_call_returns_tool_calls_exceeded_at_cap() {
        let budget = RunBudget::new("run_t", 10_000, 10, 2);
        budget.record_tool_call(50).unwrap();
        budget.record_tool_call(50).unwrap();
        let err = budget
            .record_tool_call(50)
            .expect_err("third call SHALL hit ToolCallsExceeded");
        match err {
            BudgetError::ToolCallsExceeded { used, budget: cap } => {
                assert_eq!(used, 2);
                assert_eq!(cap, 2);
            }
            other => panic!("expected ToolCallsExceeded, got {other:?}"),
        }
        // 失败路径不污染计数器。
        let snap = budget.snapshot();
        assert_eq!(snap.tool_calls_used, 2);
        assert_eq!(snap.tokens_used, 100);
    }

    #[test]
    fn record_tool_call_returns_tokens_exceeded_when_consumption_overflows_budget() {
        let budget = RunBudget::new("run_t", 100, 10, 16);
        budget.record_tool_call(60).unwrap();
        let err = budget
            .record_tool_call(50)
            .expect_err("60+50>100 SHALL hit TokensExceeded");
        match err {
            BudgetError::TokensExceeded {
                used,
                consumed,
                budget: cap,
            } => {
                assert_eq!(used, 60);
                assert_eq!(consumed, 50);
                assert_eq!(cap, 100);
            }
            other => panic!("expected TokensExceeded, got {other:?}"),
        }
        // 失败路径不污染计数器（atomic 语义）。
        let snap = budget.snapshot();
        assert_eq!(snap.tool_calls_used, 1);
        assert_eq!(snap.tokens_used, 60);
    }

    #[test]
    fn is_exceeded_now_includes_tool_calls_dimension() {
        let budget = RunBudget::new("run_t", 1_000_000, 100, 1);
        assert!(!budget.is_exceeded());
        budget.record_tool_call(0).unwrap();
        assert!(
            budget.is_exceeded(),
            "tool_calls_used 1 >= tool_call_budget 1 SHALL trigger is_exceeded"
        );
    }

    #[test]
    fn record_tool_call_negative_tokens_clamp_to_zero() {
        let budget = RunBudget::new("run_t", 100, 10, 16);
        // 负数 tokens 视为 0，仅占用 1 次 tool call slot。
        budget.record_tool_call(-50).unwrap();
        let snap = budget.snapshot();
        assert_eq!(snap.tool_calls_used, 1);
        assert_eq!(snap.tokens_used, 0);
    }
}
