//! agent-autonomy-loop W3 / Tasks 4.5 / 4.12 / 4.13 / 4.15：性质测试 P1-P3 + P2。
//!
//! 本文件聚合自治回路相关的性质测试（P1 / P3 / P2），用 `proptest` 在
//! [`RawAgentDecision::validate_and_promote`] 等纯函数上做随机输入验证。
//!
//! 性质对应需求：
//!
//! - **P1 自治字段必填**：R1.3 / R3.5 / R3.9 — 任一 R3.2 必填字段为空 / 类型非法 /
//!   枚举非法时，`validate_and_promote` 输出的 `risks` SHALL 含
//!   `missing_required_field:* / invalid_enum_value:* / invalid_type:*` 之一。
//!
//! - **P2 single-shot revision 上限**：R2.3 / R2.4 / R2.8 — 任意
//!   `(Reply 输出, Review 输出 needsRevision=true)` 组合下，gateway SHALL 调用
//!   Reply Agent 至多 2 次（1 首轮 + 至多 1 次 revision）；若第二轮 review 仍
//!   `needsRevision=true` 或 `approved=false`，则终态 SHALL 为
//!   `gateway_status="revision_failed"` + `decision.should_reply=false`。
//!
//! - **P3 未执行 Reviewer 不发送**：任何 `should_reply=true` 的正文在
//!   `local_decision_review` 都不得获批；预算耗尽保留 `budget_exceeded_no_review` 终态。

use proptest::prelude::*;
use wechatagent::agent::{AgentDecision, RawAgentDecision, UserRuntimeParameters};

// ─────────────────────────────────────────────────────────────────
// P1 自治字段必填（task 4.12）
// ─────────────────────────────────────────────────────────────────

/// 生成"final 轮且必填字段被故意置空 / 取非法值"的 RawAgentDecision。
///
/// 把 9 个自治协议字段全部填满合法长度的中文实质内容（≥ 20 unicode chars
/// for critical-turn 兜底），但随机选 1 个字段 (a) 留空 / (b) 设非法枚举 /
/// (c) 设非法类型；预期是 `validate_and_promote` 输出至少一条
/// `missing_required_field:* / invalid_enum_value:* / invalid_type:*`。
#[derive(Debug, Clone)]
struct ViolationCase {
    /// 0 = missing required field, 1 = invalid enum value, 2 = invalid bool/string type
    kind: u8,
    /// Field index in the required-fields list (0..=6 for R1.3 7 fields, 7..=11 for R3.2 fields).
    field_index: u8,
}

fn violation_case_strategy() -> impl Strategy<Value = ViolationCase> {
    (0u8..=2u8, 0u8..=11u8).prop_map(|(kind, field_index)| ViolationCase { kind, field_index })
}

fn build_baseline_raw() -> RawAgentDecision {
    let long = "这是一段足够长的实质内容用来满足关键变化轮的最低字符数要求"; // > 20 unicode chars
    let mut raw = RawAgentDecision::default();
    raw.user_understanding = Some(long.to_string());
    raw.relationship_read = Some(long.to_string());
    raw.operation_goal = Some(long.to_string());
    raw.knowledge_need_reason = Some(long.to_string());
    raw.memory_update_reason = Some(long.to_string());
    raw.self_critique = Some(long.to_string());
    raw.risk_self_check = Some(long.to_string());
    raw.why_should_reply = Some("因为对话上下文表明用户需要明确的回应".to_string());
    raw.why_skip_reply = Some(String::new());
    raw.run_mode = Some("knowledge_grounded".to_string());
    raw.risk_level = Some("medium".to_string());
    raw.knowledge_need = Some("required".to_string());
    raw.autonomy_mode = Some("assisted".to_string());
    raw.needs_review = Some(true);
    raw.operation_state = Some("relationship_building".to_string());
    raw.consolidation_needed = Some(false);
    raw.should_reply = Some(true);
    raw.reply_text = Some("好的，我来回复你".to_string());
    raw.decision_phase = Some("final".to_string());
    raw
}

fn apply_violation(raw: &mut RawAgentDecision, case: &ViolationCase) -> &'static str {
    // R1.3 always-required fields (7) — index 0..=6:
    //   0 user_understanding / 1 relationship_read / 2 operation_goal /
    //   3 knowledge_need_reason / 4 memory_update_reason / 5 self_critique /
    //   6 risk_self_check
    // R3.2 enum-required fields — index 7..=11:
    //   7 risk_level / 8 knowledge_need / 9 run_mode / 10 autonomy_mode / 11 operation_state
    let idx = case.field_index;
    let kind = case.kind;

    match kind {
        0 => {
            // missing required field
            match idx {
                0 => raw.user_understanding = Some(String::new()),
                1 => raw.relationship_read = Some(String::new()),
                2 => raw.operation_goal = Some(String::new()),
                3 => raw.knowledge_need_reason = Some(String::new()),
                4 => raw.memory_update_reason = Some(String::new()),
                5 => raw.self_critique = Some(String::new()),
                6 => raw.risk_self_check = Some(String::new()),
                7 => raw.risk_level = Some(String::new()),
                8 => raw.knowledge_need = Some(String::new()),
                9 => raw.run_mode = Some(String::new()),
                10 => raw.autonomy_mode = Some(String::new()),
                _ => raw.operation_state = Some(String::new()),
            }
            "missing_required_field"
        }
        1 => {
            // invalid enum value (only 7..=10 are enum-typed; 11 operation_state is required-only,
            // its membership check happens later in gateway/state-machine guard, not in
            // validate_and_promote).
            match idx {
                7 => raw.risk_level = Some("critical".to_string()),
                8 => raw.knowledge_need = Some("none".to_string()),
                9 => raw.run_mode = Some("manual".to_string()),
                10 => raw.autonomy_mode = Some("manual".to_string()),
                _ => {
                    // for non-enum fields, fall back to "missing"
                    return apply_violation(
                        raw,
                        &ViolationCase {
                            kind: 0,
                            field_index: idx,
                        },
                    );
                }
            }
            "invalid_enum_value"
        }
        _ => {
            // invalid bool type — fall back to coercing risk_level to a clearly invalid value;
            // "invalid_type" only applies to JSON bools and we can't easily inject a String into a
            // serde-derived bool field at runtime, so we substitute a missing field for symmetry.
            return apply_violation(
                raw,
                &ViolationCase {
                    kind: 0,
                    field_index: idx,
                },
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 1 / Task 4.12 / Validates: R1.3, R3.5, R3.9**
    ///
    /// 任一必填字段被设空 / 枚举非法 时，`validate_and_promote` 输出的 risks
    /// SHALL 含一条相应的违规标签。
    #[test]
    fn p1_autonomy_required_fields_violation_always_emits_risk_tag(
        case in violation_case_strategy()
    ) {
        let mut raw = build_baseline_raw();
        let expected_prefix = apply_violation(&mut raw, &case);
        let runtime = UserRuntimeParameters::default();
        let (decision, risks) = raw.validate_and_promote(&runtime);

        prop_assert!(
            risks.iter().any(|r| r.starts_with(expected_prefix)),
            "expected at least one risk starting with `{}`, got risks={:?}, decision.autonomy_mode={:?}, decision.run_mode={:?}",
            expected_prefix,
            risks,
            decision.autonomy_mode,
            decision.run_mode
        );
    }
}

// ─────────────────────────────────────────────────────────────────
// P3 未执行 Reviewer 不发送（task 4.13）
// ─────────────────────────────────────────────────────────────────
//
// 性质本质：`needs_review` / 自报风险不得授权绕过独立 Reviewer。只要存在拟发送正文，
// 本地 fallback 在预算是否耗尽的所有组合下都必须 `approved=false`。
//
// W3 / Task 4.13：`local_decision_review` 与 `RunBudget` 已通过 mod.rs / review.rs
// 提升为 `pub`（仅 PBT 入口需要），其余 `current_run_budget` / `RUN_BUDGET` 仍为
// `pub(crate)`，最小化对外可见面。

use wechatagent::agent::{local_decision_review, RunBudget};

/// 生成 (token_budget, max_llm_calls, force_exceeded, needs_review) 的 PBT 输入。
///
/// `force_exceeded=true` 时通过 `record_call(token_budget + 1)` 把 budget 推过
/// token 阈值；否则保持为 0 用量、未超额。这两条路径覆盖 R3.7 / R3.8 / R3.10。
fn budget_case_strategy() -> impl Strategy<Value = (i64, i32, bool, bool)> {
    (1i64..=100, 1i32..=5, any::<bool>(), any::<bool>())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 3 / Task 4.13 / Validates: R3.7, R3.8, R3.10**
    ///
    /// `local_decision_review` 在 budget 超额 / 未超额、`needs_review` true / false
    /// 的全部组合下都不能批准拟发送正文。
    #[test]
    fn p3_budget_exceeded_no_review_consistent(
        (token_budget, max_llm_calls, force_exceeded, needs_review)
            in budget_case_strategy()
    ) {
        // tool_call_budget 给 i32::MAX：本性质只关心 token / llm_calls 维度。
        let budget = RunBudget::new("run_pbt_p3", token_budget, max_llm_calls, i32::MAX);
        if force_exceeded {
            // 一次记录 token_budget+1 即可在 token 维度跨过阈值。
            budget.record_call(token_budget + 1);
            prop_assert!(budget.is_exceeded(), "force_exceeded 路径必须命中 is_exceeded");
        } else {
            prop_assert!(!budget.is_exceeded(), "未注入用量时不应超额");
        }

        let mut decision = AgentDecision::default();
        decision.should_reply = true;
        decision.reply_text = "sendable body".to_string();
        decision.needs_review = needs_review;
        let runtime = UserRuntimeParameters::default();
        let result = local_decision_review(&decision, &budget, &runtime);

        prop_assert!(!result.approved);
        if force_exceeded {
            prop_assert_eq!(
                result.risks.as_slice(),
                &["budget_exceeded_no_review".to_string()][..],
                "budget exhaustion must preserve the blocked_by_budget contract"
            );
        } else {
            prop_assert!(result.should_hold);
            prop_assert!(
                result.risks.iter().any(|r| r == "required_reviewer_not_executed"),
                "a non-budget local fallback must be an auditable safety hold: {:?}",
                result.risks
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// P2 single-shot revision 上限（task 4.15；28 号裁决 §2.2 后补 fallback 分支）
// ─────────────────────────────────────────────────────────────────
//
// gateway 的 R2 revision 控制流（写作时快照 `run_user_operation_gateway_inner`
// gateway.rs:3237-3585 与 `apply_revision_fallback` gates.rs:1258-1275——行号随
// 代码演进漂移，以符号为准）的纯逻辑映射到下方 `run_revision_loop`。
// 模型边界与 gateway 一一对应，便于审计：
//
//   gateway 行为                                        本测试 model
//   ───────────────────────────────────────────────────────────────
//   `let mut reply_calls = 1;`（首轮 reply）            reply_calls 初值 1
//   `decide_revision(finalize, review, budget)`         本地 decide_revision
//     └─ Approved && needs_revision && !should_hold
//          && !revision_direction.empty
//          && !budget_exceeded                          Proceed
//     └─ revisionDirection 空                           Skip(InvalidDirection)
//     └─ budget 超额                                    Skip(BudgetExceeded)
//     └─ finalize != Approved 或 !needs_revision
//          或 should_hold                               NotEligible
//   `RevisionDecision::Proceed` →
//     `decide_reply_with_promote(... revision_direction)` reply_calls += 1
//     `review_decision(... revised)` →
//     `finalize_review_for_send(second_review)` →
//     `second_passed = Approved && review_passed`        second_passed
//   `if second_passed` → final_review_status =
//      "revision_applied_approved"                       status="approved"
//   `else` → 恢复首轮 review 快照 + `apply_revision_fallback`
//      （gateway.rs:3486-3509；LLM 错误/超时路径同函数）：
//     └─ 首轮 trigger 纯风格（risks 全为 human_like_*/
//        emotional_value_*，或 AllPass 且 style_diverged；
//        无 reviewer_dual_disagree、非硬闸——
//        `revision_fallback_is_safe_style_only`）
//          → 恢复原稿照发：approved=true、
//            final_review_status="revision_applied_approved"、
//            should_reply=true                          status="approved",
//                                                       should_reply=true
//     └─ 其它（安全/边界/压力/双审分歧/未知）
//          → fail-closed：
//            final_review_status="revision_failed"、
//            should_reply=false                         status="revision_failed",
//                                                       should_reply=false
//   `RevisionDecision::Skip` 不走 fallback，直接写
//      `final_review_status="revision_failed"`
//      `final_decision.should_reply=false`              status="revision_failed",
//                                                       should_reply=false
//   `RevisionDecision::NotEligible` →
//      review.approved 决定 should_reply
//      （首轮 finalize 已写好 status）                  保留首轮 should_reply
//
// 性质：
//   1. reply_calls ≤ 2 — 任意输入下都成立（Proceed 至多 +1，Skip / NotEligible 不调）。
//      **范围声明（28 号裁决 A2）**：该上限只覆盖本模型的 revision 子流程；
//      生产 targeted rewrite（证据修复重写）与 revision 串联时 Reply Agent
//      全局可达 3 次，rewrite 路径不在本模型内。
//   2. 进入 Proceed 且第二轮仍失败：
//      2a. 首轮 trigger **非**纯风格 → should_reply == false 且 status == "revision_failed"；
//      2b. 首轮 trigger 纯风格 → **恢复原稿照发**（should_reply == true 且
//          status == "approved"，即 revision_applied_approved）——28 号裁决前的
//          旧模型断言"二轮失败恒 revision_failed"与生产相反，已修正。

#[derive(Debug, Clone, Copy)]
struct ReviewSnapshot {
    /// `review.approved`：finalize 之后的 approved 标记。
    approved: bool,
    /// `review.needs_revision`：Review Agent 是否要求重写。
    needs_revision: bool,
    /// `review.should_hold`：是否走 hold 路径（hold 不进 R2 块）。
    should_hold: bool,
    /// 首轮 finalize 是否仍是 `Approved`；非 Approved 表示已被硬安全门拦截，
    /// gateway 永远不会进入 R2 revision 块（首轮 finalize fail-closed return）。
    finalize_approved: bool,
    /// 是否提供了非空 `revisionDirection`（gateway.rs decide_revision R2.5）。
    revision_direction_non_empty: bool,
    /// 首轮 revision trigger 是否为**纯风格**（`revision_fallback_is_safe_style_only`
    /// 的模型化：soft 失败 risks 全为 human_like_*/emotional_value_*，或 AllPass 且
    /// 唯一触发源是 style_diverged；含 pressure/boundary/硬闸/双审分歧则为 false）。
    /// 只在进入 Proceed 且第二轮失败时被 fallback 消费。
    fallback_safe_style_only: bool,
}

impl ReviewSnapshot {
    /// 等价 `review_passed && finalize == Approved`：判定该 review 在 gateway
    /// 视角下是否"算通过"。本模型不展开 score 维度（fact_risk / human_like 等），
    /// 因为 P2 只关心 revision 控制流；score 路径已被 P1/P4 与
    /// `finalize_review_for_send` 单元测试覆盖。
    fn passed(&self) -> bool {
        self.approved && !self.needs_revision && self.finalize_approved
    }
}

/// 模型化 gateway.rs 的 single-shot revision 控制流。返回
/// `(reply_calls, final_should_reply, final_status)`：
///
/// * `reply_calls`：Reply Agent 调用次数（初值 1，Proceed +1，Skip / NotEligible
///   不增）。性质 1 SHALL `<= 2`。
/// * `final_should_reply`：`final_decision.should_reply` 终值。
/// * `final_status`：终态字面量，对应 gateway 内
///   `review.final_review_status`：`"approved"` / `"revision_failed"` / `"hold"` /
///   `"blocked"`（hold/blocked 走 NotEligible 分支，保留首轮 finalize 状态）。
fn run_revision_loop(
    initial: ReviewSnapshot,
    second: ReviewSnapshot,
    budget_exceeded_for_revision: bool,
) -> (u32, bool, &'static str) {
    let mut reply_calls: u32 = 1;

    // 首轮 finalize 未通过 → gateway 直接 fail-closed return（gateway.rs:937），
    // 永远不进入 revision 块。模型保留首轮 should_reply 与 status。
    if !initial.finalize_approved {
        let status = if initial.should_hold {
            "hold"
        } else {
            "blocked"
        };
        return (reply_calls, false, status);
    }

    // decide_revision：finalize == Approved 之后的三种分支。
    if !initial.needs_revision || initial.should_hold {
        // NotEligible：review 未要求 revision 或 hold → 保留首轮终态。
        let should_reply = initial.passed() && !initial.should_hold;
        let status = if initial.should_hold {
            "hold"
        } else if should_reply {
            "approved"
        } else {
            // approved=false 但 needs_revision=false 也可能发生（safety guard
            // 抢先在 finalize 写过 approved=false）；保守标 "blocked"。
            "blocked"
        };
        return (reply_calls, should_reply, status);
    }

    // 进入 R2 块，但有两种 Skip 前置条件。
    if !initial.revision_direction_non_empty {
        // R2.5：revisionDirection 空 → revision_failed（gateway.rs:735-737）。
        return (reply_calls, false, "revision_failed");
    }
    if budget_exceeded_for_revision {
        // R2.8：budget 超额 → revision_failed（gateway.rs:735-737）。
        return (reply_calls, false, "revision_failed");
    }

    // Proceed：调用第二次 Reply Agent，再走 finalize + review_passed。
    reply_calls += 1;
    let second_passed = second.passed();
    if second_passed {
        // R2.3：revision_applied_approved。
        (reply_calls, true, "approved")
    } else if initial.fallback_safe_style_only {
        // 第二轮仍 fail 且首轮 trigger 纯风格 → `apply_revision_fallback` 恢复
        // 首轮已 Approved 的原稿照发（review.approved=true、
        // final_review_status="revision_applied_approved"、should_reply=true；
        // gates.rs:1264-1269 亲验）。"风格改不动"不是安全事故，扣下原稿才是。
        (reply_calls, true, "approved")
    } else {
        // 第二轮仍 fail 且 trigger 含安全/边界/压力/双审分歧/未知 → fail-closed
        // revision_failed（gates.rs:1270-1273）。
        (reply_calls, false, "revision_failed")
    }
}

fn review_snapshot_strategy() -> impl Strategy<Value = ReviewSnapshot> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(
                approved,
                needs_revision,
                should_hold,
                finalize_approved,
                dir_non_empty,
                fallback_safe_style_only,
            )| {
                ReviewSnapshot {
                    approved,
                    needs_revision,
                    should_hold,
                    finalize_approved,
                    revision_direction_non_empty: dir_non_empty,
                    fallback_safe_style_only,
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// **Property 2 / Task 4.15 / Validates: R2.3, R2.4, R2.8 +
    /// `apply_revision_fallback`（28 号裁决 §2.2 修正）**
    ///
    /// 任意 (首轮 review, 第二轮 review, budget_exceeded) 组合下：
    /// 1. Reply Agent 调用次数 SHALL ≤ 2（仅 revision 子流程口径——生产
    ///    targeted rewrite 串联可达全局 3 次，见模型头部范围声明）；
    /// 2a. 进入 R2 且第二轮仍失败、首轮 trigger **非**纯风格时，终态 SHALL 是
    ///     `should_reply == false` + `status == "revision_failed"`（fail-closed）；
    /// 2b. 进入 R2 且第二轮仍失败、首轮 trigger 纯风格时，SHALL 恢复原稿照发
    ///     （`should_reply == true` + `status == "approved"`，即生产的
    ///     revision_applied_approved 回退路径——gates.rs `apply_revision_fallback`）；
    /// 3. 进入 R2 但被 Skip 前置条件（revisionDirection 空 / 预算超额）拦截
    ///    时，SHALL 写 `revision_failed` 终态且不再调用 Reply Agent（Skip 分支
    ///    不走 fallback——gateway.rs:3316-3343 亲验）。
    #[test]
    fn p2_single_shot_revision_caps_reply_calls_at_two(
        initial in review_snapshot_strategy(),
        second in review_snapshot_strategy(),
        budget_exceeded_for_revision in any::<bool>(),
    ) {
        let (reply_calls, should_reply, status) =
            run_revision_loop(initial, second, budget_exceeded_for_revision);

        // 性质 1：Reply Agent 调用次数硬上限（revision 子流程口径）。
        prop_assert!(
            reply_calls <= 2,
            "reply called {} times, must be ≤ 2 (initial={:?}, second={:?}, budget_exceeded={})",
            reply_calls, initial, second, budget_exceeded_for_revision
        );

        let entered_proceed = initial.finalize_approved
            && initial.needs_revision
            && !initial.should_hold
            && initial.revision_direction_non_empty
            && !budget_exceeded_for_revision;
        let second_failing = !second.passed();

        // 性质 2a：Proceed + 第二轮 fail + 非纯风格 trigger → fail-closed。
        if entered_proceed && second_failing && !initial.fallback_safe_style_only {
            prop_assert_eq!(
                should_reply, false,
                "unsafe trigger + second-pass failing → should_reply must be false"
            );
            prop_assert_eq!(
                status, "revision_failed",
                "unsafe trigger + second-pass failing → status must be revision_failed"
            );
            prop_assert_eq!(
                reply_calls, 2,
                "Proceed branch must invoke Reply Agent exactly 2 times"
            );
        }

        // 性质 2b：Proceed + 第二轮 fail + 纯风格 trigger → 恢复原稿照发
        // （apply_revision_fallback 白名单路径；旧模型在此断言 revision_failed，
        // 与生产相反——28 号裁决修正）。
        if entered_proceed && second_failing && initial.fallback_safe_style_only {
            prop_assert_eq!(
                should_reply, true,
                "style-only trigger + second-pass failing → restore pre-revision draft and send"
            );
            prop_assert_eq!(
                status, "approved",
                "style-only fallback → revision_applied_approved（模型词表 approved）"
            );
            prop_assert_eq!(
                reply_calls, 2,
                "fallback path still consumed the second Reply Agent call"
            );
        }

        // 性质 3：Skip 分支 SHALL 写 revision_failed 终态，且不再调用 Reply Agent。
        let entered_skip = initial.finalize_approved
            && initial.needs_revision
            && !initial.should_hold
            && (!initial.revision_direction_non_empty || budget_exceeded_for_revision);
        if entered_skip {
            prop_assert_eq!(
                should_reply, false,
                "Skip branch (empty direction / budget exceeded) → should_reply=false"
            );
            prop_assert_eq!(
                status, "revision_failed",
                "Skip branch (empty direction / budget exceeded) → status=revision_failed"
            );
            prop_assert_eq!(
                reply_calls, 1,
                "Skip branch must NOT call Reply Agent a second time"
            );
        }
    }
}
