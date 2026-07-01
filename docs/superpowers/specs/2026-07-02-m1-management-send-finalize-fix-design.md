# M1 管理 Agent 发送网关缺 finalize 硬门修复设计

> 日期：2026-07-02
> 分支：`fix/m1-management-send-finalize`（从 origin/main 0fb31b7 切，含 H3/H10/H8/H7/H1/H11+M9+L1/M13）
> 来源：终极审判审计 M1（UPHELD Medium）

## 1. 漏洞描述（对最新代码 100% 亲验）

管理 Agent 的 `wechatagent.send_contact_message` 工具（`execute_management_tool`，management.rs:1513）调 `agent::send_contact_message_gateway`（gateway.rs:161）发私聊。该函数的发送放行判定**只调 `review_passed`**（gateway.rs:274）：

```rust
if !review_passed(&review, &runtime) { /* block */ }
```

而客户主链路（webhook）的 `run_user_operation_gateway_inner` 走 `finalize_review_for_send`（gateway.rs:1524）——W2 设计的「最终安全汇总层，所有硬门汇于一处」。两条发送路径的安全门**不对称**。

### `review_passed`（gates.rs:20-40）缺失、而 `finalize_review_for_send` 独有的硬门

1. **R5.4 verified-knowledge 确定性硬门**（gates.rs:653-686）：产品声明必须由 `operation_knowledge_chunks` 中 verified 知识或 `priced_from_catalog` 背书，否则 `blocked_unverified_product_claim`。`review_passed` 只有 reviewer 自报的 `knowledge_grounding_score` **软闸**（LLM 可高估）。这正是 CLAUDE.md 红线「产品声明必须 verified 背书」的确定性兜底——管理发送路径完全没有。
2. **R3.5/R3.6 协议违规门**（gates.rs:555）→ `blocked_by_required_field`。
3. **R3.7 预算门**（gates.rs:616）→ `blocked_by_budget`。
4. **R2.6 should_hold + holdCategory 校验**（gates.rs:757-791）。

### 二次安全门不兜底

`check_second_safety_gate_pure`（outbox.rs:497-519）只查 `contact_cooldown_active` / `user_stop_requested_after_decision` / `outbox_stale_30min`，**不重跑内容安全门**。outbox dispatcher 发送前的唯一复检就是这道，不含 R5.4。

### 内容可 AI 生成

management.rs:1514 的 `content = string_arg(&planned.arguments, "content")` 来自管理 Agent 的 tool-call 参数，可由 LLM 生成（非仅运营手打）。

### 后果

管理 Agent 发送私聊时，若 reviewer LLM 对一条无 verified 背书的产品声明给了高 `knowledge_grounding_score`，`review_passed` 放行，而客户主链路的 R5.4 会确定性拦截。真实的安全门不对称——同一条危险内容，走客户回复被拦、走管理发送被放行。

## 2. 根因

`send_contact_message_gateway` 是先于 W2 finalize 层设计的旧发送入口，从未接入 `finalize_review_for_send`。它把 `review_passed`（软/硬闸折叠成一个 bool）当作最终放行判定，缺少 finalize 层独有的四道确定性硬门。

## 3. 方案选型

### 方案 A（选定，用户确认）：管理发送路径接入 finalize_review_for_send + review_passed guard

把 `send_contact_message_gateway` 的 `review_decision` 之后、发送之前的判定，从「仅 `review_passed`」改为「`finalize_review_for_send` + `matches!(status, Approved) && review_passed`」，与客户主链路的 `second_passed`（gateway.rs:1814）判定完全一致。

**关键：放行条件必须带 `&& review_passed(&review, &runtime)` guard，不能是裸 `matches!(Approved)`。** 原因（已逐行核实）：`review_decision` 内部走 `route_dual_gate`（review/mod.rs:520），对**软闸失败**（humanLike/emotionalValue/pressureRisk 不达标）会设 `needs_revision=true`；finalize 随后在 gates.rs:801-820 把 `approved` **翻成 true、status=Approved**，指望调用方后续跑 single-shot revision 循环补救。但管理发送路径**没有 revision 循环**。若裸判 `matches!(Approved)` 就发，软闸失败的内容会被**未经改写直接发出**——而当前 `review_passed` 对软闸失败是直接 block。加 `&& review_passed` guard 后：finalize 只 mutate `approved`、不改分数，软闸失败时 `review_passed` 仍返 false，那道 guard 挡住「软闸失败被当 Approved 发出」。

**逐场景验证（每条已核实）：**

| 场景 | 当前（仅 review_passed） | 方案 A（finalize + guard） |
|---|---|---|
| 全通过 | 发送 | `Approved && review_passed=true` → 发送 ✓ 不变 |
| 软闸失败 | block | finalize 标 Approved，但 review_passed（软闸分不达标）=false → 不发 ✓ 不变 |
| 硬闸失败（hallucination/grounding） | block | status=Held → 不发 ✓ 不变 |
| **R5.4 无背书产品声明** | **误发（bug）** | status=BlockedUnverifiedProductClaim → 不发 ✓ **新增保护** |
| 协议违规 / 预算超额 / should_hold | 部分漏 | finalize 各 return 对应 blocked 状态 → 不发 ✓ 新增保护 |

方案 A 严格**只增不减**：补 R5.4/协议/预算/should_hold 四门，不动任何现有 block 行为、不需 revision 循环。

**否决方案 B（只补 R5.4 一道）**：手写一段 R5.4 判定塞进管理路径，重复 finalize 已有逻辑、两处易漂移，且漏掉协议/预算/should_hold。既然 finalize 是现成的统一安全层，直接复用。否决。

### 明确不做（范围边界，避免超出用户批准的「四道门」）

- **不接 `apply_state_action_gate`**（operation_state_policies 动作门）：主链路在 finalize 之后单独调它（gateway.rs:1552），它不是 finalize 的一部分。管理发送是**运营显式发起**的动作，被 operation_state policy 拦截与否是一个产品语义决策（AI 自主回复受状态机约束合理，运营手动发送是否也受约束存疑），不在本次「四道门对齐」范围。保持现状不接入。
- 不改 outbox / 二次安全门 / precheck / 前端。

## 4. 核心改动

落点：`src/agent/gateway.rs` 的 `send_contact_message_gateway`。

### 4.1 decision 改可变（gateway.rs:246）
`let decision = AgentDecision { ... }` → `let mut decision = AgentDecision { ... }`（finalize 需 `&mut decision`）。

### 4.2 review_decision 之后、发送判定之前，插入 finalize（替换 gateway.rs:274 的 `if !review_passed` 判定）

```rust
// M1：与客户主链路对齐——管理发送也走 finalize_review_for_send 汇总所有硬门
// （R5.4 verified-knowledge / 协议 / 预算 / should_hold），不再仅凭 review_passed
// 的软闸折叠 bool 放行。放行条件带 `&& review_passed` guard：finalize 对软闸失败
// 会标 Approved+needs_revision 指望 revision 循环，而管理发送无 revision 通道，故
// 必须用 review_passed 二次确认软闸达标（镜像主链路 gateway.rs 的 second_passed）。
let active_profile =
    crate::agent::domain_profile::load_active_domain_profile(&state.db, &contact.workspace_id).await;
let priced_from_catalog = if active_profile.transaction_facts_enabled {
    let active_products =
        super::entitlements::load_active_products(&state.db, &contact.workspace_id).await;
    super::entitlements::priced_from_active_catalog(
        &decision.quoted_product_ids,
        &active_products,
    )
} else {
    false
};
let outcome = finalize_review_for_send(
    review,
    &mut decision,
    &runtime,
    &contact,
    &selected_chunks,
    Vec::new(), // 管理发送 decision 直接构造、非 LLM raw output，无 promote_risks
    synthetic_inbound.content.as_str(),
    &active_profile.commitment_markers,
    priced_from_catalog,
);
let FinalizeOutcome { review, status: finalize_status, pending_events } = outcome;
persist_finalize_pending_events(state, &contact, &pending_events).await?;
let passed = matches!(finalize_status, GatewayStatusFinal::Approved)
    && review_passed(&review, &runtime);
if !passed {
    // 用 finalize_status 的精确状态串作 gateway_status（blocked_unverified_product_claim /
    // blocked_by_required_field / blocked_by_budget / held_by_ai_policy 等），
    // 而非笼统 "review_blocked"。写 decision_review + event，返回 ContactSendResult。
    ...
}
```

块状态串取 `finalize_status` 的 as-string 映射（gates.rs:454 的方法，实现时读准方法名）。原 274-318 的 block 分支保留其 `write_decision_review` + `write_event_for_account` + 返回 `ContactSendResult` 结构，仅把写死的 `"review_blocked"` 改成 finalize 精确状态串。

### 4.3 review 变量遮蔽
finalize 返回的 `review`（已被 finalize mutate，如 R5.4 命中时 `hallucination_score.max(6)`、追加 risks、写 final_review_status）遮蔽原 `review`，下游 `write_decision_review` / `review_passed` 使用遮蔽后的值。

**不动**：precheck_send_gateway（发送前/后各一次，保留）、outbox enqueue、synthetic_inbound、planner、返回结构字段。`decision` 从 `..Default::default()` 构造 → `quoted_product_ids` 为空 → 管理发送 `priced_from_catalog` 恒 false（内容是 admin 自由文本、无结构化报价），R5.4 纯靠 verified_chunks 判定，行为正确且严格。

## 5. 行为验证（改动后）

见 §3 逐场景表。核心新增：R5.4 无背书产品声明从「误发」→「blocked_unverified_product_claim 拦截」，与客户主链路一致。其余场景（全通过/软闸失败/硬闸失败）行为字节等价。

## 6. 测试设计

### 核心红线测试（hermetic 纯函数对比，锁 M1 不变量）

新增 lib 单测到 `src/agent/review/gates.rs` 的 `#[cfg(test)] mod tests`（与现有 finalize_review_for_send 测试同档、无需 Docker/LLM）：

**测试：`review_passed_lets_unverified_product_claim_through_but_finalize_blocks`**
- 构造一个 review：`approved=true`、五闸分数全达标（`review_passed` 会返 **true**）、但 `claim_analysis` 显式 `requiresProductKnowledge=true`；decision `used_knowledge_ids` 引用一个 chunk，`knowledge_chunks` 传空（无 verified 背书）；`priced_from_catalog=false`。
- 断言 1：`review_passed(&review, &runtime) == true`（证明**旧代码**——管理路径仅凭 review_passed——会放行这条危险内容）。
- 断言 2：`finalize_review_for_send(...).status == GatewayStatusFinal::BlockedUnverifiedProductClaim`（证明**新代码**——接入 finalize 后——确定性拦截）。

这道对比测试直接钉死 M1 的核心：`review_passed` 漏、`finalize` 补的正是同一条无背书产品声明。纯函数、无 LLM/DB，本地可跑，且**在旧代码语义下断言 1 成立、断言 2 是新增保护的证据**。

现有 finalize R5.4 / 协议 / 预算 / should_hold 测试（gates.rs:1663+，含 BlockedUnverifiedProductClaim @ :1945/:2097）已覆盖各门本身行为，本次复用不重测。

### 软闸 guard 回归（锁「软闸失败仍不发」不因接入 finalize 而松动）

**测试：`soft_gate_failure_stays_blocked_via_review_passed_guard`**
- 构造 review：软闸失败（如 `human_like` < 阈值）、硬闸通过、无产品声明 → `route_dual_gate` 标 needs_revision → finalize 返 `Approved`（approved 被翻 true）。
- 断言：`matches!(status, Approved) == true` **但** `review_passed(&review, &runtime) == false` → `passed`（两者与）= false。证明管理发送的 `&& review_passed` guard 挡住了「软闸失败被 finalize 标 Approved 后误发」。

### 验证

- `cargo test --lib`（含上面两个新单测）≥ 350 passed / 0 failed。
- `cargo build --lib` 无 error。
- **接线（send_contact_message_gateway 真调 finalize）的端到端行为**依赖 `review_decision` 的真实 LLM reviewer，本地无法 hermetic 驱动；由代码审查 + 上面纯函数对比测试（证明 finalize 拦得住、review_passed 拦不住）+ 现有 real-LLM CI 管理发送场景兜底。诚实声明：不新增 real-LLM 集成测试（那需 LLM 端点，非本地 lib 门范围）。

## 7. 范围边界

- **不做（YAGNI）**：不接 state-action gate（§3 已论证，非「四道门」范围、涉产品语义决策）、不改 outbox/二次安全门/precheck/前端、不动 review_passed/finalize 本身逻辑。
- **过拟合红线**：测试锁「review_passed 漏而 finalize 补无背书产品声明」「软闸失败经 guard 仍不发」两个真实不变量，不为过测试改任何阈值/门逻辑。修的是让管理发送复用已有的确定性安全层，让两条发送路径的安全门对齐。
- **禁词 lint**：改动用 AI-internal 状态名（blocked_unverified_product_claim / held_by_ai_policy 等，均为现有闭集），不涉禁词。
- **多租户**：`load_active_domain_profile` / `load_active_products` 均按 `contact.workspace_id` 取，不跨租户。
