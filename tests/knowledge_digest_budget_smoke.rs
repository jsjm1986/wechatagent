//! knowledge-digest-workstation Phase 2：预算 / Budget 隔离 smoke 测试。
//!
//! 1. `RunBudget::new(run_id, 24000, 8, i32::MAX)` 必须能正常构造（Phase 2 默认值）。
//! 2. `RunBudget::record_call` 累计 token 与 LLM call 后，`is_exceeded` 必须正确判定
//!    达到上限后触发降级。
//! 3. `tool_call_budget=i32::MAX` 时不会被 tool-call 维度误判超额（digest 不走 tool-loop）。
//! 4. `RunBudget::mark_degraded` 不抛异常（运行时 fallible 路径稳定）。
//!
//! 注意：`RunBudgetSnapshot` 是 `pub(crate)`，集成测试 crate 看不到字段；这里仅用公开方法
//! 守住对外契约。字段级断言由 `src/agent/budget.rs` 内的 `mod tests` 覆盖。

use wechatagent::agent::RunBudget;

#[test]
fn digest_default_budget_constructs_without_panic() {
    let budget = RunBudget::new("digest_smoke", 24_000_i64, 8_i32, i32::MAX);
    assert!(!budget.is_exceeded(), "新建时不应超额");
}

#[test]
fn digest_budget_exceeded_when_llm_calls_reach_cap() {
    let budget = RunBudget::new("digest_smoke", 24_000_i64, 3_i32, i32::MAX);
    assert!(!budget.is_exceeded());
    budget.record_call(100);
    budget.record_call(100);
    assert!(!budget.is_exceeded(), "2 < 3, 不应超额");
    budget.record_call(100);
    assert!(budget.is_exceeded(), "llm_calls_used=3 >= max_llm_calls=3 必须触发");
}

#[test]
fn digest_budget_exceeded_when_tokens_reach_cap() {
    let budget = RunBudget::new("digest_smoke", 1000_i64, 8_i32, i32::MAX);
    budget.record_call(600);
    assert!(!budget.is_exceeded());
    budget.record_call(600);
    assert!(budget.is_exceeded(), "tokens_used=1200 >= 1000 必须触发");
}

#[test]
fn digest_budget_tool_calls_uncapped_does_not_trip_exceeded() {
    // Phase 2: digest 不走 tool-loop，传 i32::MAX 表示"不限"。
    let budget = RunBudget::new("digest_smoke", 24_000_i64, 8_i32, i32::MAX);
    // 只累 LLM 调用，不动 tool_calls；is_exceeded 仅从 token / LLM 维度判定。
    budget.record_call(1_000);
    assert!(!budget.is_exceeded(), "i32::MAX 维度不应触发超额");
}

#[test]
fn digest_budget_mark_degraded_is_callable() {
    let budget = RunBudget::new("digest_smoke", 24_000_i64, 8_i32, i32::MAX);
    budget.mark_degraded("compose_skipped_due_to_no_signals");
    // 仅守住「不 panic、不影响 is_exceeded」契约；字段级断言在内部 mod tests 覆盖。
    assert!(!budget.is_exceeded());
}
