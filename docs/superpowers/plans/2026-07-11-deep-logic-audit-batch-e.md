# 深度审查批 E（其余频道）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。**审查工程非写代码**：每 task 派 opus subagent 只读审一块、产出带 file:line 的 findings；主控逐条亲验后入台账。只审不修 src。

**Goal:** 审查批 E 五块（evolution/account/overview/operations/referral），聚焦 evolution "AI 提议+人工发布"红线（含唯一 AI 自动放量路径 auto_release）+ referral 名片引荐红线受控例外，续入统一台账批 E section，收官全五批。

**Architecture:** 5 块（Explore 已测绘），3 task：Task1 evolution 自优化（最高·auto_release 红线）、Task2 referral 名片引荐（红线受控例外+三闸让位）、Task3 account+overview+operations+批 E 汇总+全五批终评。复用批 A/B/C/D 方法。

**Tech Stack:** Rust/Axum + MongoDB + LLM + MCP。审查工具 = Read/Grep（117 真跑同前定调：PLAUSIBLE，复现留修复阶段）。

## Global Constraints（逐字继承批 A/B/C/D）
- **只入账不改 src**；**引用必亲验**（file:line 当场 Read/Grep，不靠 memory/不靠 Explore 测绘锚点免验）；**subagent 结论必主控亲验后入账/校准/推翻**，凭猜驳回；**严重度跨批一致性校准**（DB-fault/时序/误配触发默认 Med；只有"推荐配置下确定性发生的核心交互/红线破坏"够 High——见批 D KD-04 基准）。
- subagent 一律 opus（harness 拒 model:"opus" 时省略继承主会话）。
- 台账续写 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（新增"# 批 E"大节 + 各块 section），finding 编号 `KE-NN`。
- 防假绿：端点/MCP 失败标 BLOCKED；PLAUSIBLE/CONFIRMED 如实标；发现≠修复。
- **权威依据红线**：CLAUDE.md「AI 提议+人工发布，AI 绝不自动放量」（evolution）+「名片引荐是辅助模式受控例外，仍是 AI 发起+辅助，对话始终 AI 在说；台前顾问≠幕后领导；不改全自治红线」（referral 设计 2026-06-21）+「AI 永不自我核验」。
- **memory 基线**：evolution 三阶段「AI 提议+人工发布」已收紧（PR#77 第三闸模糊 LLM 响应→NeedsHumanConfirm）；referral 引荐设计已交付。批 E 审该基线后业务流 + auto_release 红线受控例外面。

## Explore 测绘的五块锚点（审查起点，实现者仍须自己 Read 亲验）
- evolution：release_evolution_proposal(evolution.rs:142 人工发布入口，confirmation=="RELEASE")/release_threshold(release.rs:36)/release_prompt(release.rs:198 红线三闸)/auto_release_gate_open(auto_release.rs:39 双闸默认关)/auto_release_eligible_thresholds(:48 仅 threshold)/decide_auto_release(:207)/decide_negative_reaction_block(:226)/release_threshold auto 调用(:187 唯一 AI 自动写生产)/is_evolution_enabled_for(runtime_flag.rs:80)/put_evolution_runtime_flag(evolution.rs:587)。
- account：list_accounts(accounts.rs:32 mcpKeyConfigured 布尔不泄漏)/sync_accounts(:64 online 来自 MCP)/update_account_mcp_key(:171)/login_begin(:244)。无删除端点。
- overview/operations：active_view(operation_view.rs:27 只读)/send_ledger overview(send_ledger.rs:115 仅 workspace 不分 account)。无单一首屏聚合端点。
- referral：assist_mode_active(referral.rs:17 默认关)/render_referral_lines(:52 send_trigger_hint)/send_outbound_namecard(:99 workspace scope+二次准入)/decision assist_on 判定(decision.rs:395)/referral_block(decision.rs:399 仅 include_business&&assist_on)/gateway assist 二次门(gateway.rs:2824)/准入三次校验(gateway.rs:2837)/reviewer 让位(review/mod.rs:259/348)/名片库 create 强制 enabled=false+draft(referral_cards.rs:80)。
- ✅ Explore 正面结论（仍须主控抽验）：无账号删除端点；密钥不泄漏（accounts 只出布尔）；全 api 经 require_session；auto_release 仅 threshold 不碰 prompt + 双闸默认关。

---

## Task 1: evolution 自优化审查（最高优先 · auto_release 红线唯一 AI 自动写生产路径）+ 批 E section 骨架

**Files:** 审查(只读) `evolution/auto_release.rs`（gate_open:39 / eligible:48 / decide_auto_release:207 / decide_negative_reaction_block:226 / release_threshold 调用:187）+ `evolution/release.rs`（release_threshold:36 / release_prompt:198 红线三闸）+ `routes/evolution.rs`（release_evolution_proposal:142 人工发布）+ `evolution/runtime_flag.rs`（is_evolution_enabled_for:80）。台账建"# 批 E"大节 + "## evolution 自优化" section。

- [ ] **Step 1: 建批 E section 骨架 + auto_release 红线双闸核验（最敏感）**

台账加"# 批 E（其余频道）"大节 + 5 块空 section。Read auto_release.rs:39-64 双闸：env 总闸 evolution_auto_release_enabled AND per-workspace 子闸 threshold_auto_release_enabled，缺失/读失败是否**默认关**（:40）。核 auto_release_eligible_thresholds(:48-79) 是否**仅** proposal_kind="threshold"（prompt 绝不走此路径）。核 :187 release_threshold(admin_id="evolution_auto_release") 确是唯一 AI 自动写生产 threshold_overrides 的路径——受双闸+band+负反应门+per-tick cap 约束是否闭合。

- [ ] **Step 2: 人工发布红线三闸 + prompt 绝不自动放量**

核 release_prompt(release.rs:198)：事务内红线三闸（compose_appended_content:258 / validate_prompt_edit:260 字面禁词+锚 / review_prompt_edit:263 LLM 语义），LLM 不可用→NeedsHumanConfirm 中止（:278）不 fail-open。核 release_evolution_proposal(evolution.rs:142) 人工发布入口 confirmation=="RELEASE" 精确串校验(:149)+workspace scope。确认 prompt 候选无任何自动放量路径（只人工 release）。

- [ ] **Step 3: decide_auto_release band 判定 + 负反应门 + 无样本保守**

核 decide_auto_release(:207)：命中率落 band 外才放行、observed=None（无样本）保守拒放(:209)。decide_negative_reaction_block(:226)：放行后窗口负反应率超阈值强制改判 SKIP 退回 admin（非回滚）。post_release 仅观测不自动回滚（Req 9.7）。

- [ ] **Step 4: 派 subagent 复审 + 主控亲验 + 入账 + Commit**

派 opus subagent 独立复审 evolution（指令：先读懂再断言、带 file:line、凭猜驳回、红线最敏感）。主控逐条亲验后写台账"evolution 自优化"section（含 ✅ 通过点 + auto_release 双闸默认关的正面结论）。`git commit -m "audit(batch-e): evolution自优化(最高优先·auto_release红线)"`

---

## Task 2: referral 名片引荐审查（红线受控例外 + 三闸让位）

**Files:** 审查(只读) `agent/referral.rs`（assist_mode_active:17 / render_referral_lines:52 / send_outbound_namecard:99）+ `agent/decision.rs`（assist_on:395 / referral_block:399）+ `agent/gateway.rs`（assist 二次门:2824 / 准入三次校验:2837 / 不走 escalation:2813）+ `agent/review/mod.rs`（reviewer 让位:259/348）+ `routes/referral_cards.rs`（create:51 强制 draft / review:122 / delete:212）。

- [ ] **Step 1: 辅助模式默认关 + assist_on 三处判定一致**

核 assist_mode_active(referral.rs:17)：force_on/force_off override 优先，否则 account_enabled.unwrap_or(false)=**默认关**；脏值 override 视为无覆盖。核 assist_on 三处独立判定同一 assist_mode_active——decision.rs:399（prompt 注入）、gateway.rs:2824（入队）、review/mod.rs:353（reviewer 让位）——关时是否全空/全跳（即便 LLM 幻觉 namecard_to_send，assist_on=false 整段跳过 gateway.rs:2828）。

- [ ] **Step 2: 红线受控例外——引荐≠转人工 + 让位段不被滥用**

核 gateway.rs:2813-2814 名片**不走** escalation 分支（被推真人=台前顾问≠幕后 principal_decider，D9 解耦）。核 reviewer 让位段（review/mod.rs:259 REVIEWER_ASSIST_YIELD_NOTE，仅 assist_on:348 注入）——让位段关闭了"第三方人类角色"与"产品声明"红线，核它是否可能被 LLM 滥用绕过 grounding/factRisk（让位仅对引荐这一动作、不应架空产品声明硬闸）。措辞守卫"我仍在场辅助"非转人工。

- [ ] **Step 3: 名片准入三次校验 + 库 CRUD 红线**

核 send_outbound_namecard(referral.rs:99)：查名片 workspace scope(:111 防 IDOR)+发送前 validate_card_sendable(:118)+MCP 成功后落库/置态失败只 error 不返 Err（防重发）。核 gateway.rs:2837 准入三次校验（card 存在+enabled+approved+workspace scope），幻觉 card_id 被拒写 referral_card_rejected。核 create_referral_card(referral_cards.rs:80) 强制 enabled=false+review_status=draft（AI 不自我核验红线）。

- [ ] **Step 4: 主控亲验 + 入账 + Commit**

派 subagent 复审 referral，主控亲验。写台账"referral 名片引荐"section。`git commit -m "audit(batch-e): referral名片引荐(红线受控例外+三闸让位)"`

---

## Task 3: account + overview/operations 审查（合并）+ 批 E 汇总 + 全五批终评 + PR

**Files:** 审查(只读) `routes/accounts.rs`（list:32 / sync:64 / update_mcp_key:171 / login:244）+ `routes/operation_view.rs`（active_view:27）+ `routes/send_ledger.rs`（overview:115 account 口径）。台账批 E 总评 + 全五批终评。

- [ ] **Step 1: account CRUD 鉴权/隔离/密钥 + 无破坏性删除**

核 list_accounts(:32) workspace scope + mcpKeyConfigured **只下发布尔不泄漏 mcp_api_key 明文**(:56)；sync_accounts(:64) online 来自 MCP、mcp_api_key $setOnInsert 不覆盖既有；update_account_mcp_key(:171) _id+workspace_id 双过滤。确认无账号删除端点（无破坏性操作）。login alias 查账号带 workspace scope。

- [ ] **Step 2: overview/operations 口径 + workspace/account 隔离**

核 active_view(operation_view.rs:27) 只读不写；send_ledger overview(:115) 仅 workspace 不分 account 口径（Explore 提示：多账号 workspace 下跨账号汇总，evolution 系列用 default_account_id）——核是否符合单 default account 部署模型、多账号下是否串数据/错账号放量（就绪债 vs bug 判定）。

- [ ] **Step 3: 主控亲验 + 入账 + Commit**

派 subagent 复审 account+overview，主控亲验。写台账"account"+"overview/operations"section。`git commit -m "audit(batch-e): account+overview/operations"`

- [ ] **Step 4: 批 E 总评 + 全五批终评 + PR**

台账加"批 E 总评"（计数/根因家族/优先级/与前四批关联）+ "全五批终评"（A~E 累计 finding 计数、跨批元家族、红线总结论、修复路线图）。复核每条 finding file:line 亲验、无夸大、严重度跨批校准。`git commit` + `git push` + 更新 PR#178（批 A~E 全量台账）。

---

## Self-Review 结论
- **Spec coverage**：Explore 测绘五块 ↔ Task1(evolution)/Task2(referral)/Task3(account+overview+operations)，全覆盖；优先级（auto_release 红线最高）↔ 独立 task 顺序；只入账不修 ↔ 全 task 无 src 改动 + Global Constraints。
- **Placeholder scan**：无 TBD；每 Step 给具体审查问题 + Explore 锚点（file:line，实现者仍须亲验）+ Explore top-3 已分派到对应 Step。
- **一致性**：台账文件名/finding 编号(KE-NN)/块锚点跨 task 一致；evolution「AI 提议+人工发布」+ referral「辅助模式受控例外」红线 + memory 基线（PR#77 三阶段收紧）作为审查基准写进 Global Constraints；严重度校准以批 D KD-04（确定性核心交互破坏才 High）为基准。
- **审查工程适配**：无 TDD；用"读码→对照红线→主控亲验→入账"替代；每 task 独立可提交 deliverable；117 真跑同前定调（PLAUSIBLE，留修复阶段）；Task3 含全五批终评收官。
