# 改写失败/超时回退已通过原稿 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 改写（single-shot revision）的三个失败分支（LLM 错误 / 30s 超时 / 改写后第二轮 review 未过）统一回退发送「改写前那份已 Approved 的原稿」，而不是丢弃补兜底占位。

**Architecture:** 在 `run_user_operation_gateway_inner` 的 `RevisionDecision::Proceed` 分支里，改写调用前克隆一份原稿快照；三个失败分支恢复该快照 + 用一个新纯函数 `apply_revision_fallback` 统一把 review/finalize 状态设成「发原稿」态（should_reply=true、finalize=Approved、final_review_status=revision_applied_approved）。纯函数放 `review/gates.rs`（与 `derive_revision_failure` 同文件同层），单测覆盖状态赋值；三个分支的原稿恢复走同一克隆变量。

**Tech Stack:** Rust 2021 / Axum / tokio；`cargo test --lib` 单测。

## Global Constraints

- 红线：改代码前必须 100% 读懂相关代码，file:line 引用必亲验（本计划所有行号基于 2026-07-13 main HEAD 亲验）。
- 不得触碰硬闸（hallucination / knowledge_grounding，`gates.rs:120-141`）、不改 `derive_revision_failure`（其它调用点仍用）、不改兜底占位机制、不新增终态枚举。
- `check-no-human-takeover` lint：新增行/注释不得含 `人工接管/takeover/hand-off/人工介入` 等禁词。
- 基线门：`cargo test --lib` ≥ 350 passed 0 failed；4 个 PBT 累计 ≥ 33 passed 0 failed。新增测试只增量叠加。
- 过拟合红线：测试锚定真实行为，不为调绿改业务逻辑/阈值。

---

## File Structure

- **Modify** `src/agent/review/gates.rs`：新增纯函数 `apply_revision_fallback`（回退状态赋值）+ 其单测。与既有 `derive_revision_failure`（:1030）、`GatewayStatusFinal`（:437）同文件。
- **Modify** `src/agent/gateway.rs`：`RevisionDecision::Proceed` 分支（:1831-2032）——改写调用前存原稿快照；三个失败分支（:1960 else / :1981 `Ok(Err)` / :2006 `Err(_)`）改为恢复快照 + 调 `apply_revision_fallback`。

---

## Task 1: 纯函数 `apply_revision_fallback` + 单测

**Files:**
- Modify: `src/agent/review/gates.rs`（新增函数，放在 `derive_revision_failure` 之后 :1035 附近；测试加入该文件既有 `#[cfg(test)]` 区）
- Test: `src/agent/review/gates.rs`（同文件单测）

**Interfaces:**
- Produces: `pub(crate) fn apply_revision_fallback(review: &mut DecisionReviewResult, finalize_status: &mut GatewayStatusFinal, failure_reason: &str) -> String`
  - 作用：把状态设成「发改写前原稿」——`review.approved=true`、`review.revision_applied=false`、`review.final_review_status="revision_applied_approved"`、`*finalize_status=GatewayStatusFinal::Approved`；返回传入的 `failure_reason`（供调用方赋给 `revision_reason` 审计）。
  - 注意：本函数只改 review/finalize 状态，**不碰 `final_decision`**（原稿恢复与 `should_reply=true` 由调用方在 gateway 侧做，因为 `final_decision` 是 gateway 局部变量）。

- [ ] **Step 1: 写失败测试**

在 `src/agent/review/gates.rs` 的 `#[cfg(test)]` 区（可放入既有 `review_passed_dual_gate_tests` mod 或新建 `revision_fallback_tests` mod）加：

```rust
#[cfg(test)]
mod revision_fallback_tests {
    use super::{apply_revision_fallback, GatewayStatusFinal, HOLD_CATEGORY_HELD_BY_AI_POLICY};
    use crate::agent::types::DecisionReviewResult;

    #[test]
    fn fallback_sets_approved_draft_state() {
        let mut review = DecisionReviewResult::default();
        review.approved = false;
        review.revision_applied = false;
        review.final_review_status = String::new();
        let mut status = GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());

        let reason = apply_revision_fallback(&mut review, &mut status, "revision_llm_timeout_30s");

        assert!(review.approved, "回退后原稿应视为已批准");
        assert!(!review.revision_applied, "改写未真正应用");
        assert_eq!(review.final_review_status, "revision_applied_approved");
        assert!(matches!(status, GatewayStatusFinal::Approved), "finalize 应回到 Approved 走发送");
        assert_eq!(reason, "revision_llm_timeout_30s", "失败原因应原样返回供审计");
    }

    #[test]
    fn fallback_preserves_arbitrary_reason() {
        let mut review = DecisionReviewResult::default();
        let mut status = GatewayStatusFinal::Held(HOLD_CATEGORY_HELD_BY_AI_POLICY.to_string());
        let reason = apply_revision_fallback(&mut review, &mut status, "revision_post_review_failed");
        assert_eq!(reason, "revision_post_review_failed");
        assert_eq!(review.final_review_status, "revision_applied_approved");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib apply_revision_fallback`
Expected: 编译失败 `cannot find function apply_revision_fallback`（函数尚未定义）。

- [ ] **Step 3: 实现纯函数**

在 `src/agent/review/gates.rs` `derive_revision_failure`（:1035 结尾）之后加：

```rust
/// 改写失败/超时的优雅降级：回退发送「改写前那份已 Approved 的原稿」。
///
/// 前提（已亲验）：能进改写通道的原稿在 finalize 阶段一定已判 Approved——硬闸
/// 失败（hallucination / knowledge_grounding）走 approved=false → Held，
/// `decide_revision` 返回 NotEligible，根本进不了改写。因此改写只由软闸 / style /
/// 双审分歧触发，原稿本就安全可发；改写只是锦上添花。改写因下游 LLM 超时/错误没做成，
/// 应回退发原稿而非毙掉补兜底。
///
/// 本函数只设置 review/finalize 状态；原稿恢复与 should_reply=true 由 gateway 调用方
/// 负责（final_decision 是 gateway 局部变量）。返回 failure_reason 供调用方写 revision_reason。
pub(crate) fn apply_revision_fallback(
    review: &mut DecisionReviewResult,
    finalize_status: &mut GatewayStatusFinal,
    failure_reason: &str,
) -> String {
    review.approved = true;
    review.revision_applied = false;
    review.final_review_status = "revision_applied_approved".to_string();
    *finalize_status = GatewayStatusFinal::Approved;
    failure_reason.to_string()
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib revision_fallback_tests`
Expected: 2 passed。

> 若 `DecisionReviewResult` 无 `Default` 派生或字段名不符：先 Read `src/agent/types.rs` 确认 `DecisionReviewResult` 的构造方式与 `approved`/`revision_applied`/`final_review_status` 字段名，按实际调整测试构造（既有测试 `full_pass_review()` 在 gates.rs:1052 是现成构造范例，可复用）。`HOLD_CATEGORY_HELD_BY_AI_POLICY` 常量已在 gates.rs 使用（:1033），确认其可见性；不可见则直接用 `GatewayStatusFinal::Held("held_by_ai_policy".to_string())`。

- [ ] **Step 5: 提交**

```bash
git add src/agent/review/gates.rs
git commit -m "feat(gateway): 加 apply_revision_fallback 纯函数（改写失败回退原稿状态）

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: gateway 三分支接回退 + 原稿快照

**Files:**
- Modify: `src/agent/gateway.rs`：`RevisionDecision::Proceed` 分支 :1831-2032

**Interfaces:**
- Consumes: `apply_revision_fallback`（Task 1）
- Produces: 无（改内联控制流）

- [ ] **Step 1: 存原稿快照**

在 `RevisionDecision::Proceed => {` 分支内、`let revision_direction = ...`（:1832）之后、`pre_revision_summary = Some(...)`（:1835）之前，插入：

```rust
            // 改写失败/超时回退用：此刻 final_decision 仍是改写前那份已 Approved 的原稿。
            let pre_revision_decision = final_decision.clone();
```

> 亲验：`final_decision` 在 :1887 `final_decision = revised_decision;` 才被改写稿覆盖。快照点在 :1832 之后，克隆到的确是原稿。`AgentDecision` 是否派生 `Clone`：Read `src/agent/types.rs` 确认；若未派生，用既有的 `to_document`/重建方式，或给该结构加 `#[derive(Clone)]`（它是纯数据结构，加 Clone 无副作用）。

- [ ] **Step 2: 改「第二轮 review 未过」分支（现 :1960-1979 else 块）**

把该 else 块（现内容：`revision_applied=true; review.approved=false; final_review_status="revision_failed"; should_reply=false; derive_revision_failure("revision_post_review_failed"); ...`）整体替换为：

```rust
                    } else {
                        // 改写稿第二轮 review 未过 → 回退发改写前已 Approved 的原稿（graceful degradation）。
                        // 原稿在首轮 finalize 已过 apply_state_action_gate，无需再检。
                        final_decision = pre_revision_decision.clone();
                        final_decision.should_reply = true;
                        revision_applied = false;
                        revision_reason = super::review::apply_revision_fallback(
                            &mut review,
                            &mut finalize_status,
                            "revision_post_review_failed",
                        );
                        post_revision_summary = Some(format!(
                            "fallback_to_pre_revision reply_text_len={} reason=revision_post_review_failed",
                            final_decision.reply_text.chars().count()
                        ));
                    }
```

> 注意：`apply_revision_fallback` 的模块路径按 gates.rs 在 review 模块内的实际 re-export 调整（既有代码 :1801 用 `decide_revision(...)`、:1810 用 `derive_revision_failure(...)` 的引用方式即为同模块可见范例——沿用相同前缀，不要臆造 `super::review::`；亲验现有调用点的写法后对齐）。

- [ ] **Step 3: 改「LLM 错误」分支（现 :1981-2005 `Ok(Err(err))`）**

把该分支体（现：`review.approved=false; revision_applied=false; final_review_status="revision_failed"; should_reply=false; derive_revision_failure(format!("revision_llm_error:{}",err)); finalize_status=status; write_event(...)`）替换为：

```rust
                Ok(Err(err)) => {
                    // 改写 LLM 调用失败 → 回退发原稿（原稿此分支未被覆盖，仍安全；统一走快照恢复最省心）。
                    final_decision = pre_revision_decision.clone();
                    final_decision.should_reply = true;
                    revision_applied = false;
                    revision_reason = apply_revision_fallback_ref(
                        &mut review,
                        &mut finalize_status,
                        &format!("revision_llm_error:{}", err),
                    );
                    write_event_for_account(
                        state,
                        &contact.account_id,
                        Some(&contact.wxid),
                        "revision_llm_failure",
                        "info",
                        "Reply Agent revision 调用失败：回退发送改写前已批准原稿",
                        Some(doc! {
                            "run_id": &run_id,
                            "error": err.to_string(),
                        }),
                    )
                    .await?;
                }
```

> `apply_revision_fallback_ref` 是占位名——实际用 Task 1 定义的 `apply_revision_fallback`，模块前缀按 Step 2 亲验后的写法统一。事件 severity 从 `"blocked"` 改 `"info"`（不再是拦截，是降级放行）。

- [ ] **Step 4: 改「30s 超时」分支（现 :2006-2029 `Err(_)`）**

替换为：

```rust
                Err(_) => {
                    // 改写 30s 超时 → 回退发改写前已 Approved 的原稿（慢端点下最常见路径）。
                    final_decision = pre_revision_decision.clone();
                    final_decision.should_reply = true;
                    revision_applied = false;
                    revision_reason = apply_revision_fallback(
                        &mut review,
                        &mut finalize_status,
                        "revision_llm_timeout_30s",
                    );
                    write_event_for_account(
                        state,
                        &contact.account_id,
                        Some(&contact.wxid),
                        "revision_llm_failure",
                        "info",
                        "Reply Agent revision 调用超时（30s）：回退发送改写前已批准原稿",
                        Some(doc! {
                            "run_id": &run_id,
                            "latency_ms": 30000_i64,
                        }),
                    )
                    .await?;
                }
```

- [ ] **Step 5: 编译**

Run: `cargo check --lib`
Expected: 通过。若报模块路径/Clone 错误，按 Step 1-2 的旁注（确认 `AgentDecision: Clone`、`apply_revision_fallback` 可见路径）修正。

- [ ] **Step 6: 全量 lib 测试 + 基线门**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed（不回归）。

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter`
Expected: 累计 ≥ 33 passed, 0 failed。

- [ ] **Step 7: no-human-takeover lint**

Run: `bash scripts/check-no-human-takeover.sh`（Windows 用 `pwsh scripts/check-no-human-takeover.ps1`）
Expected: 0 violations（新增注释里没有禁词——已用「回退发送/降级放行/已批准原稿」等 AI 内部措辞）。

- [ ] **Step 8: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "fix(gateway): 改写失败/超时回退已批准原稿而非毙掉补兜底

三个失败分支（LLM错误/30s超时/二轮review未过）统一恢复改写前
已Approved原稿并走正常发送。慢端点下约20%本可成功回复曾被改写
超时误毙成兜底占位，此修根治。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: 集成验证（部署 117 真发）

**Files:** 无（运行时验证）

- [ ] **Step 1: 部署两文件到 117**

按既有部署方式（file-patch + `_remote_run_direct.py`）把 `gates.rs`+`gateway.rs` 的 diff 应用到 117，`cargo build --release`（前台流式，避免 nohup 被会话带走），`systemctl restart wechatagent.service`。重启前红线确认：无 managed 私聊 inbound 在等（排除 @chatroom / gh_）。

- [ ] **Step 2: 真实触发软闸改写**

用一个 managed 客户发一条会触发软闸改写、且端点可能超时的消息。观察生产：

```
mongosh --quiet wechatagent --eval '...'
# 断言该 run: final_review_status=revision_applied_approved
#            revision_reason=revision_llm_timeout_30s（或 error/post_review_failed）
#            outbox status=sent 且 content 是真回复原稿（不是"稍等我给你准信"兜底）
```

Expected: 客户收到真回复原稿，不再是兜底占位。

---

## Self-Review

**1. Spec coverage:**
- 设计「三个失败分支统一回退」→ Task 2 Step 2/3/4 覆盖三分支。✓
- 设计「存原稿快照」→ Task 2 Step 1。✓
- 设计「用 revision_applied_approved 过入队门」→ Task 1 纯函数设该值 + Task 2 各分支调用。✓
- 设计「不新增终态、不改兜底/硬闸/derive_revision_failure」→ 计划未触碰这些。✓
- 设计「所有软闸一视同仁回退含 boundary_privacy_safety」→ 回退在改写失败分支统一发生，与软闸类别无关。✓
- 设计测试四项 → Task 1 两纯函数测 + Task 2 Step 6 基线 + Task 3 真发验证。⚠ 说明：三分支的完整 async 流程无法纯函数单测（需 DB），故用「纯函数覆盖状态赋值 + 生产真发验证」组合，符合项目「本地只跑 --lib、集成走 CI/真发」纪律。

**2. Placeholder scan:** Step 3 的 `apply_revision_fallback_ref` 已显式标注为占位名并指向真名；模块前缀处均要求「亲验现有调用点写法后对齐」而非留空。无 TBD/TODO。

**3. Type consistency:** `apply_revision_fallback(review:&mut DecisionReviewResult, finalize_status:&mut GatewayStatusFinal, failure_reason:&str) -> String` 在 Task 1 定义、Task 2 三处调用签名一致。返回值赋给 `revision_reason`（既有 `String` 变量，:1745）。`final_decision.clone()` 依赖 `AgentDecision: Clone`（Task 2 Step 1 要求亲验/按需加派生）。

## Execution Handoff

计划已保存到 `docs/superpowers/plans/2026-07-13-revision-fallback-to-approved-draft.md`。
