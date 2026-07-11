# 深度审查批 D（请示配置链）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。**审查工程非写代码**：每 task 派 opus subagent 只读审一条链、产出带 file:line 的 findings；主控逐条亲验后入台账。只审不修 src。

**Goal:** 从请示配置四频道（askHuman/askHumanConfig/llmProviders/systemStrategy）入口穿透后端，按四链（请示裁决→决策人链→provider 热切换→prompt pack）逐链深审逻辑正确性与红线，续入统一台账批 D section。

**Architecture:** 4 链（Explore 已测绘），3 task：Task1 请示裁决 + relay 出站守卫（最高·红线"客户永不知道有领导"）、Task2 决策人链 + 超时改派 + 骚扰门、Task3 provider 热切换（405 坑复发面）+ prompt pack 生效闸 + 批 D 汇总。复用批 A/B/C 方法。

**Tech Stack:** Rust/Axum + MongoDB + LLM + MCP。审查工具 = Read/Grep（117 真跑同前定调：标 PLAUSIBLE，复现留修复阶段）。

## Global Constraints（逐字继承批 A/B/C）
- **只入账不改 src**；**引用必亲验**（file:line 当场 Read/Grep，不靠 memory/不靠 Explore 测绘锚点免验）；**subagent 结论必主控亲验后入账**，凭猜驳回；**严重度跨批一致性校准**（DB-fault/时序触发类默认 Med，不因单批 subagent 定 High 破坏校准）。
- subagent 一律 opus（harness 拒 model:"opus" 时省略继承主会话）。
- 台账续写 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（新增"# 批 D"大节 + 各链 section），finding 编号 `KD-NN`。
- 防假绿：端点/MCP 失败标 BLOCKED；PLAUSIBLE/CONFIRMED 如实标；发现≠修复。
- **权威依据红线**：CLAUDE.md「无人工接管」精确含义（客户永不面对真人、对话始终 AI 在说；relay 是 AI 转述幕后领导结论，不是人工接管）+ 决策请示通道设计 `docs/superpowers/specs/2026-06-05-principal-decision-channel-design.md` + no-human-takeover lint 禁词。
- **改 prompt 不必 bump PROMPT_PACK_VERSION**（生效闸是内容 diff，prompts.rs normalize 内容比对）——审查此闸时不要把"没 bump 版本"当 bug。

## Explore 测绘的四链锚点（审查起点，实现者仍须自己 Read 亲验）
- 请示裁决：should_escalate_held(logic.rs:351)/escalate_held_decision(mod.rs:43)/insert_pending_escalation(ledger.rs:20)/handle_principal_reply(mod.rs:286)/interpret_principal_reply(mod.rs:243)/resolve_principal_escalation(principal_escalations.rs:75)/relay_principal_decision_to_customer(gateway.rs:755)/generate_holding_reply(holding_reply.rs:66)。
- relay 出站守卫（红线核心）：gateway.rs:2480-2519 fail-closed；relay_output_leaks_internal_payload(logic.rs:211)/relay_introduces_unauthorized_number(logic.rs:256)/is_principal_relay_trigger(logic.rs:196 靠 is_synthetic_relay 非客户可控 content)/holding_reply_text_is_safe(holding_reply.rs:11)。
- 决策人链：resolve_ask_human_policy(policy.rs:21)/decider_chain.first()(mod.rs:71)/next_decider_on_timeout(policy.rs:95)/scan_escalation_timeouts(mod.rs:358)/reassign_escalation(ledger.rs:289)/lookup_principal_config(ledger.rs:215)/push_allowed 骚扰门(policy.rs:68)。
- provider 热切换：activate_provider(llm_providers.rs:305)/swap_registry(:552)/test_provider(:439)/base_url trim(:178 create/:229 update，test/swap 不 trim)/mask_api_key(:43)/is_masked_value(:47)。
- prompt pack：reset_system_prompt_pack(prompt_templates.rs:385)/publish_prompt_template(:240 生效主关口三闸)/update_prompt_template(:155)/review_prompt_edit(内容 diff 闸,management_prompt_edit.rs:7 re-export prompt_guard)。
- ✅ Explore 正面结论（仍须主控抽验）：api_key 无明文 echo（View 只出 mask）；provider 全路径 resolve_authorized_workspace + workspaceId 约束；relay 双兜底就位。

---

## Task 1: 请示裁决链 + relay 出站守卫审查（最高优先 · 红线"客户永不知道有领导"）+ 批 D section 骨架

**Files:** 审查(只读) `agent/escalation/logic.rs`（should_escalate_held/relay_output_leaks_internal_payload:211/relay_introduces_unauthorized_number:256/is_principal_relay_trigger:196）+ `agent/gateway.rs`（relay_principal_decision_to_customer:755 / relay 出站守卫:2480-2519）+ `agent/escalation/mod.rs`（interpret_principal_reply:243 / handle_principal_reply:286）+ `agent/escalation/holding_reply.rs`。台账建"# 批 D"大节 + "## 请示裁决链" section。

- [ ] **Step 1: 建批 D section 骨架 + 读透 relay 出站红线守卫**

台账加"# 批 D（请示配置链）"大节 + 4 链空 section。Read gateway.rs:2480-2519 relay 出站守卫全段：核 fail-closed 是否真在**入 outbox 前**拦截、命中是否记 blocked_by_safety_guard 不发；relay_output_leaks_internal_payload(:211) 哨兵/字段标记检测是否完整（`__PRINCIPAL_RELAY__`/verdict=/substance=/constraints=）；relay_introduces_unauthorized_number(:256) 授权外数字护栏——Explore 提示 FollowUp 分支授权源空串 `""` 时是否误杀/漏检、守卫是否仅 is_principal_relay_trigger 为真时启用（非 relay run 常规回复的授权外数字有无覆盖盲区）。

- [ ] **Step 2: interpret 侧对称 + relay 身份判定防伪造**

核 interpret_principal_reply(mod.rs:243)：LLM 把领导自然语言解读成结构化裁决，是否**绝不原话转发客户**、解析失败/越界回落 deferred（sanitize_verdict:411 闭集）。is_principal_relay_trigger(logic.rs:196) 是否靠来源标记 is_synthetic_relay 而非客户可控 content 前缀（H10 修复防伪造哨兵劫持）——核现码仍是来源标记判定。

- [ ] **Step 3: holding_reply 出站守卫 + 客户永不被晾死**

核 generate_holding_reply(holding_reply.rs:66)：独立预算旁路 + 出站守卫 holding_reply_text_is_safe(:11 非空+禁词+授权外数字三查) + 硬编码降级兜底。expired_authorization 走中性收尾（logic.rs:100）清 awaiting 不永久压制自主回复（gateway.rs:798 clear_awaiting）。

- [ ] **Step 4: 派 subagent 复审 + 主控亲验 + 入账 + Commit**

派 opus subagent 独立复审请示裁决+relay 守卫（指令：先读懂再断言、带 file:line、凭猜驳回、红线核心）。主控逐条亲验后写台账"请示裁决链"section（含 ✅ 通过点）。`git commit -m "audit(batch-d): 请示裁决链+relay出站守卫(最高优先·红线)"`

---

## Task 2: 决策人链 + 超时改派 + 骚扰门审查

**Files:** 审查(只读) `agent/escalation/policy.rs`（resolve_ask_human_policy:21 / next_decider_on_timeout:95 / push_allowed:68 / in_quiet_hours:55）+ `agent/escalation/mod.rs`（scan_escalation_timeouts:358 / decider_chain.first:71）+ `agent/escalation/ledger.rs`（reassign_escalation:289 / lookup_principal_config:215 / count_pushes_today:338）+ `routes/principal_escalations.rs`（resolve:75 / reassign:122）。

- [ ] **Step 1: decider chain 解析 + 决策人≠客户 + workspace 隔离**

核 resolve_ask_human_policy(:21) 优先 decider_chain、回落旧 principal_decider 字节等价；decider_chain.first() 取当前决策人、链空=未启用请示（mod.rs:71-74）、拒绝决策人==客户 wxid(:75)。lookup_principal_config(ledger.rs:215) 必须用入站消息自己的 workspace_id 约束防跨域串扰。reassign(principal_escalations.rs:122) 校验 to_wxid 在 decider_chain 内 + workspace 防 IDOR。

- [ ] **Step 2: 超时改派幂等 + 骚扰门口径（Explore top-3 观察点）**

核 scan_escalation_timeouts(mod.rs:358)：age>timeout 且非链尾→推 next→推成功才 reassign 落库；链尾发客户延期安抚不困死台账。Explore 疑点：并发多 tick（多实例部署）是否同一台账被两个 next 同时推卡；count_pushes_today/latest_push_ms 以 created_at 近似推送时刻、改派不改 created_at 是否致 next 骚扰门统计口径漂移；推卡成功但 reassign 落库失败下一 tick 重推是否真幂等。push_allowed(policy.rs:68) daily_cap+dedupe_window+quiet_hours 逻辑正确性。

- [ ] **Step 3: 主控亲验 + 入账 + Commit**

派 subagent 复审决策人链+超时改派，主控亲验。写台账"决策人链"section。`git commit -m "audit(batch-d): 决策人链+超时改派+骚扰门"`

---

## Task 3: provider 热切换 + prompt pack 生效闸审查 + 批 D 汇总 + PR

**Files:** 审查(只读) `routes/llm_providers.rs`（activate_provider:305 / swap_registry:552 / test_provider:439 / base_url trim:178/229 / mask_api_key:43 / is_masked_value:47）+ `routes/prompt_templates.rs`（publish_prompt_template:240 / update_prompt_template:155 / reset_system_prompt_pack:385）+ `prompt_guard`（review_prompt_edit / validate_prompt_edit）。台账批 D 总评。

- [ ] **Step 1: provider 热切换正确性 + base_url 规范化一致性（405 坑复发面）**

核 activate_provider(:305)：DB update_many 清旧 active→update_one 置新→swap_registry(:552) 热替换运行时 LlmRegistry（改 DB+热重载双写），自治 agent 下次取 registry 即用新 provider。**Explore 重点**：create/update 写库 trim_end_matches('/')(:178/:229)，但 test_provider 用 inline base_url(:470/485) 与 swap_registry(:558) 均**不 trim**——"编辑未保存即 test"带尾斜杠/缺 /v1 时 test 与实际 activate 后行为不一致；405 根因（缺 /v1）不在任何规范化覆盖内，核是否应在 LlmClient::with_format 或 swap 层补路径校验。api_key mask 守卫（:43/:47/:220 占位沿用旧值/:461 test 回退真值）无明文 echo。

- [ ] **Step 2: prompt pack 生效闸三闸 + reset 语义**

核 publish_prompt_template(:240) draft→active 生效主关口：闸1+2 字面双闸（validate_prompt_edit 禁词+锚完整性，force 不可绕）→闸3 LLM 语义审查（review_prompt_edit 审 old↔new diff 增量，force 可跳）→保留 evolution 历史行→置 active。生效闸是**内容 diff 非版本号**（不把没 bump 版本当 bug）。reset_system_prompt_pack(:385) 显式销毁性 reseed + bump prompt_pack_version 失效 LRU cache——核 reset 不是每启动幂等覆写（会clobber 运营编辑）。publish 时 delete_many 清同 key 非 evolution 旧行(:330) 是否误删 evolution_release（rollback 保护）。

- [ ] **Step 3: 主控亲验 + 入账 + Commit**

派 subagent 复审 provider+prompt pack，主控亲验。写台账"provider 热切换"+"prompt pack"section。`git commit -m "audit(batch-d): provider热切换+prompt pack生效闸"`

- [ ] **Step 4: 批 D 总评 + 自检防假绿 + PR**

台账加"批 D 总评"：finding 计数按严重度 + 跨链根因家族 + 修复优先级 + 与批 A/B/C 关联。复核每条 finding file:line 亲验、无夸大、无把设计当 bug、严重度跨批校准。`git commit` + `git push` + 更新 PR#178（批 A+B+C+D 合并台账）。

---

## Self-Review 结论
- **Spec coverage**：Explore 测绘四链 ↔ Task1(请示裁决+relay)/Task2(决策人链+超时)/Task3(provider+prompt pack)，全覆盖；优先级（红线 relay 守卫最高）↔ 独立 task 顺序；只入账不修 ↔ 全 task 无 src 改动 + Global Constraints。
- **Placeholder scan**：无 TBD；每 Step 给具体审查问题 + Explore 锚点（file:line，实现者仍须亲验）+ Explore top-3 观察点已分派到对应 Step。
- **一致性**：台账文件名/finding 编号(KD-NN)/链锚点跨 task 一致；「无人工接管」红线精确含义 + 决策请示通道设计文档 + prompt 生效闸是内容 diff 非版本号，作为审查基准写进 Global Constraints。
- **审查工程适配**：无 TDD；用"读码→对照红线→主控亲验→入账"替代；每 task 独立可提交 deliverable；117 真跑同前定调（PLAUSIBLE，留修复阶段）。
