//! GATE-1:single-shot revision 改写后若 operation_state 迁入"禁止 reply"的态,
//! 动作闸须在二次 finalize 后复检,把结果置 held_by_ai_policy(而非放行进 outbox)。
//!
//! 验证点(代码审查 + 本 #[ignore] 集成骨架共同保证):
//! 1. 初次 finalize 的动作闸已抽成 `apply_state_action_gate`,语义与抽取前等价。
//! 2. revision 通过(second_passed=true、外层 finalize_status 被置回 Approved)后,
//!    对改写后的 final_decision 复检一次动作闸:
//!    - load_operation_state_policy_for_contact(改写后 operation_state)
//!    - classify_decision_action(改写后 final_decision)
//!    - enforce_state_action_policy 命中 forbidden → 置外层 finalize_status=Held,
//!      review.approved=false、final_decision.should_reply=false、追加
//!      "state_action_policy_blocked" risk、落 state_action_policy_blocked 审计事件。
//! 3. 外层 finalize_status 被改成 Held 后,下游 `if !matches!(finalize_status, Approved)`
//!    分支(gateway.rs ~:1835)走 fail-closed:写 decision_review/审计/run_log、取消任务、
//!    return Ok(()) —— 不进 precheck_send_gateway、不 enqueue agent_send_outbox、不发送。
//!
//! 注:revision 路径调完整 Reply Agent(decide_reply_with_promote → 真实 LLM),无 mock
//! 注入 seam,难在本地确定性复现。故作 #[ignore] CI 骨架;复检逻辑正确性由 Step 3 代码
//! 审查 + lib 基线保证。
#![cfg(test)]

mod common;

#[tokio::test]
#[ignore = "需要 Docker testcontainers MongoDB + 真实 LLM(revision 路径)"]
async fn revision_into_forbidden_state_is_held() {
    // 播种一个 operation_state_policies:把某 state 的 reply 动作列 forbidden
    // (参 tests/workspace_isolation.rs:554 的 forbidden/allowed/status=active 行形态)。
    // 构造一次会触发 single-shot revision(reviewer needs_revision=true、direction 非空、
    // 预算未超)且 revision 后 operation_state 迁入该 forbidden state 的 run。
    // 断言:
    //   * 最终 gateway 状态为 held_by_ai_policy(decision_review / agent_run_log 落库值);
    //   * agent_send_outbox 无本 run 的 pending 条目(未发送);
    //   * events 集合存在一条 kind="state_action_policy_blocked"、status="blocked" 的审计。
    // 具体搭建按 tests/ 现有 operation_state_policies + revision 集成测范式。
    // 此路径依赖真实 LLM(revision 调完整 Reply Agent),作为 #[ignore] CI 验证。
}
