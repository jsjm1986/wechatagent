# 改写失败/超时回退已通过原稿（revision graceful degradation）

日期：2026-07-13
状态：设计待审

## 问题（生产实证）

managed 客户连续追问，收到的永远是同一句兜底占位「这个我帮你确认一下，稍等我给你准信。」，AI 实际生成的优质回复没发出。

根因（全部 file:line 亲验）：

- 改写（single-shot revision）的三个失败分支——LLM 错误（`gateway.rs:1981` `Ok(Err)`）、30s 超时（`gateway.rs:2006` `Err(_)`）、改写后第二轮 review 未过（`gateway.rs:1960` else 分支）——都把 `final_decision.should_reply=false` + `finalize_status=Held`（`derive_revision_failure` 恒返回 `Held(held_by_ai_policy)`，`review/gates.rs:1030-1035`）。
- 非 Approved 终态在 `gateway.rs:2073` 一律 fail-closed：只写审计、不入 outbox（入队白名单仅认 `approved / revision_applied_approved` + `should_reply=true`，`gateway.rs:2469-2472`）。
- 于是 `ensure_customer_acknowledged`（`gateway.rs:904-972`）补一句兜底占位，客户收到的是它，不是被丢弃的真回复。

生产数据：7 月以来 inbound，`revision_failed` 9 条里 **8 条首评本已 approved**；失败原因 `revision_llm_timeout_30s` 占 7 条。即约 20% 本可成功的回复被慢端点的改写超时毙掉。

## 设计基石（已亲验）

**能进改写通道的原稿，finalize 一定已判 `Approved`（安全可发）。** 硬闸失败（hallucination ≥ 阈值、knowledge_grounding < 阈值，`gates.rs:120-141`）一律 `approved=false` → finalize 走 Held → `decide_revision` 返回 `NotEligible`（`gates.rs:981-987`），根本进不了改写。

改写只可能由**软闸**触发（humanLike / pressureRisk / emotionalValue / boundary_privacy_safety，`gates.rs:146-198`）或 `style_diverged`（`gateway.rs:1774`）或双 reviewer 分歧 / insufficient_detail。这些都是「原稿可发、改写只是锦上添花」。

**结论：改写失败/超时 → 无条件回退发送改写前那份已 Approved 的原稿。红线（硬闸）在改写通道之外，回退不触碰它。** 用户已定：不过度严格，该放行的直接放行。

## 方案（最小改动）

改写调用前存一份原稿快照，三个失败分支统一恢复它、走正常发送。

1. **存快照**：在 `RevisionDecision::Proceed` 分支、`decide_reply_with_promote`（`gateway.rs:1842`）调用**之前**，克隆当前已 Approved 的 `final_decision` 为 `pre_revision_decision`（此刻 `final_decision` 还是原稿，未被改写稿覆盖）。

2. **三个失败分支统一回退**（替换现有的 `should_reply=false`+Held）：
   - 恢复 `final_decision = pre_revision_decision.clone()`（超时/LLM 错误分支其实未被覆盖，但统一走克隆最简单、最不易错）。
   - `final_decision.should_reply = true`。
   - `finalize_status = GatewayStatusFinal::Approved`。
   - `review.final_review_status = "revision_applied_approved"`（让原稿过 `gateway.rs:2469-2472` 入队门；语义：改写未成，发的是已批准原稿）。
   - `revision_reason` 保留失败原因（`revision_llm_timeout_30s` / `revision_llm_error:...` / `revision_post_review_failed`）用于审计可观测。
   - 原有的 `write_event_for_account("revision_llm_failure", ...)` 审计事件保留，不动。

3. **不做的事**（避免过度工程）：不新增终态枚举、不改兜底占位机制、不改硬闸、不改 `derive_revision_failure`（它仍供其它调用点用）、不加 fallback 分类逻辑（所有软闸一视同仁回退，含 boundary_privacy_safety）。

## 影响面

- `apply_state_action_gate`（`gateway.rs:1951`）只在第二轮 **通过** 时对改写稿复检 forbidden state。回退发的是原稿，原稿在首轮 finalize 已过 `apply_state_action_gate`（初次动作闸），无需再检。
- outbox 幂等键、去抖中止（`gateway.rs:2504-2513`）、media/namecard 发送逻辑不变——回退后 `final_decision` 是原稿，其 assets/namecard 与首轮一致。

## 测试

- 单测（gateway 或 review 层，就近现有 revision 测试）：
  - 超时分支回退：构造 Proceed + 改写 future 超时 → 断言 `final_decision.should_reply=true`、`finalize_status=Approved`、`final_review_status=revision_applied_approved`、reply_text 等于原稿。
  - LLM 错误分支回退：同上，future 返回 Err。
  - 第二轮 review 未过分支回退：改写稿被第二轮 review 判 fail → 断言发的是**原稿**（reply_text 等于 pre_revision，不是改写稿）。
  - 反向保护：硬闸失败的 run 不进改写通道（既有 `hard_gate_failure_does_not_trigger_revision` 测试已覆盖，确认不回归）。
- 基线门：`cargo test --lib` 不回归，`scripts/check-baseline` 双门绿，`check-no-human-takeover` lint 不踩禁词。

## 验证

改码 + 单测绿后部署 117，用 managed 客户真实发一条会触发软闸改写的消息，观察：改写若超时，客户收到的是真回复原稿而非兜底占位；`agent_run_logs` 该 run `final_review_status=revision_applied_approved` + `revision_reason=revision_llm_timeout_30s` + outbox `status=sent` 内容为原稿。
