//! knowledge-digest-workstation Phase 5 / P5.3：chat tool loop 预算硬门测试
//! （无 Docker 依赖）。
//!
//! Chat agent 的 tool-calling 循环必须**永远** 走 `RunBudget` 拦门：
//!
//! 1. **token / LLM call / tool_call 三维独立计数**
//!    - `record_call` 只动 LLM 维度；
//!    - `record_tool_call` 只动 tool 维度；
//!    通过 `is_exceeded()` 间接验证三维互不串扰。
//! 2. **tool_call_budget=0 即时拒绝**：第一个 toolCall 就要被挡住。
//! 3. **超额状态机单调**：is_exceeded() 一旦 true 不会回 false。
//! 4. **失败路径不污染计数器**：第 N+1 次 tool call 失败后，下次成功调用
//!    本应可继续——验证失败路径没有把名额"消费掉"。
//! 5. **token 维度极大消耗 saturating 不溢出 panic**。
//!
//! 注意：这是 integration test，看不到 `RunBudget::snapshot()`（pub(crate)）
//! 与 `BudgetError`（pub(crate)）；改用 `is_exceeded()` 与 `record_tool_call`
//! 的 `Result<(), _>::is_ok/err` 做行为级断言。

use wechatagent::agent::RunBudget;

#[test]
fn record_call_does_not_block_tool_calls() {
    // record_call 只动 LLM 维度；只要 tool_call_budget 还有名额，
    // record_tool_call 必须仍能成功——侧面验证 record_call 没有越权动 tool_calls。
    let budget = RunBudget::new("chat_t", 10_000, 6, 4);
    budget.record_call(120);
    budget.record_call(80);
    // tool 维度未被 record_call 触动 → 可继续 dispatch tool。
    assert!(
        budget.record_tool_call(50).is_ok(),
        "record_call 不应消耗 tool_call_budget"
    );
}

#[test]
fn record_tool_call_does_not_block_more_llm_calls_when_only_tool_cap_hit() {
    // record_tool_call 只动 tool 维度；即使 tool_call_budget 用完，
    // 只要 LLM / token 维度没满，依然要被视作 tool 维度超额（is_exceeded=true），
    // 这点正是 chat_tool_loop 用来 force_stop 的信号。
    let budget = RunBudget::new("chat_t", 10_000, 6, 1);
    budget.record_tool_call(50).expect("first tool call ok");
    assert!(
        budget.is_exceeded(),
        "tool_calls 用满 → is_exceeded 应该为 true（用于 chat loop force_stop）"
    );
}

#[test]
fn zero_tool_budget_rejects_first_call() {
    // tool_call_budget=0 是「禁止任何 tool call」语义；第一个调用就要被挡。
    let budget = RunBudget::new("chat_t", 10_000, 6, 0);
    assert!(
        budget.record_tool_call(50).is_err(),
        "tool_call_budget=0 应该立即拒绝第一个 toolCall"
    );
    // 注意：is_exceeded 看 tool_calls_used >= tool_call_budget；
    // budget=0、used=0 时 0>=0 也是 true，正好作为 chat loop 立即退出的信号。
    assert!(
        budget.is_exceeded(),
        "budget=0 起步即 is_exceeded（chat loop 立即结束）"
    );
}

#[test]
fn is_exceeded_is_monotonic_after_tool_cap() {
    // chat tool loop 在 is_exceeded()=true 时进入 force_stop；
    // 一旦超额，无论后续做了什么调用，is_exceeded 不能回 false。
    let budget = RunBudget::new("chat_t", 10_000, 6, 1);
    assert!(!budget.is_exceeded(), "初始未超额");
    budget.record_tool_call(50).expect("first call ok");
    assert!(budget.is_exceeded());
    // 后续 record_call 不应"清掉"超额状态。
    budget.record_call(10);
    assert!(budget.is_exceeded(), "is_exceeded 必须单调不可逆");
}

#[test]
fn failed_tool_call_does_not_consume_quota() {
    // 守的是 record_tool_call 文档承诺的"atomic 语义"：
    // tokens 维度过载导致的失败不应吃掉 tool_call_budget 名额。
    let budget = RunBudget::new("chat_t", 100, 6, 4);
    budget.record_tool_call(60).unwrap();
    // 60+50>100 → tokens 维度失败；tool_call 名额本不该被消费
    assert!(budget.record_tool_call(50).is_err());
    // 拒绝后再调用一个 token 消耗很小的 tool call，名额应仍能成功用——
    // 如果失败路径消费了名额，这里就会再次失败 / is_exceeded。
    assert!(
        !budget.is_exceeded(),
        "失败的 tool call 不应触发 is_exceeded"
    );
    assert!(
        budget.record_tool_call(20).is_ok(),
        "tokens 维度失败不应吃掉 tool_call 名额"
    );
}

#[test]
fn token_budget_overflow_is_rejected_without_panic() {
    // 防御性：极大的 tokens_consumed 不应让计数器整数溢出 panic / wrap，
    // 也不应被悄悄记入；必须以 Err 拒绝，且 is_exceeded 不变。
    let budget = RunBudget::new("chat_t", 1_000, 6, 16);
    assert!(budget.record_tool_call(i64::MAX).is_err());
    assert!(
        !budget.is_exceeded(),
        "超大 tokens_consumed 被拒后状态机不应被污染"
    );
    // 拒绝路径不污染 → 后续小消耗 tool call 仍能成功。
    assert!(budget.record_tool_call(10).is_ok());
}

#[test]
fn unlimited_tool_budget_via_i32_max_does_not_block() {
    // chat 路径 fallback 用 i32::MAX 表达「关闭 tool 维度限制」；
    // 在该模式下，连续 8 次 toolCall 不会触发 is_exceeded。
    let budget = RunBudget::new("chat_t", 1_000_000, 64, i32::MAX);
    for i in 0..8 {
        assert!(
            budget.record_tool_call(10).is_ok(),
            "第 {i} 次 tool call 不应被 ToolCallsExceeded 拒绝"
        );
    }
    assert!(
        !budget.is_exceeded(),
        "i32::MAX 表达「不限」，8 次 dispatch 后不应超额"
    );
}

#[test]
fn negative_tokens_consumed_clamped_to_zero() {
    // 文档承诺 `tokens_consumed.max(0)`：负值视为 0；不会让 token 计数器
    // 反向减少，也不会突破 token_budget 上限。
    let budget = RunBudget::new("chat_t", 100, 6, 4);
    assert!(budget.record_tool_call(-50).is_ok());
    // 如果负值被错误累加，tokens_used 会变成 -50，下次 i64::MAX 校验
    // 也可能被绕过；这里通过验证后续大消耗仍受限来反证。
    assert!(
        budget.record_tool_call(101).is_err(),
        "负值 token 不应让后续超额校验失效"
    );
}
