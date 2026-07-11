# 深度审查批 B（知识链）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。**审查工程非写代码**：每 task 派 opus subagent 只读审一条知识业务链、产出带 file:line 的 findings；主控逐条亲验后入台账。只审不修 src。

**Goal:** 从知识子系统三频道（knowledgeWiki/content/quality）入口穿透到后端，按**业务链**逐条深审逻辑正确性，产出主控亲验的 findings，续入统一台账批 B section。

**Architecture:** 7 条业务链（Explore 已测绘），按优先级组织：链3 grounding 召回（最高）/ 链4 修复 / 链2 审核 为重心，链1录入/链5修订/链6质量/链7 catalog 各一 task。复用批 A 方法（subagent 只读审 + 主控亲验 + 只入账不修）。

**Tech Stack:** Rust/Axum + MongoDB + LLM。审查工具 = Read/Grep（117 真跑同批 A 定调：多数触发前提生产无法安全构造，标 PLAUSIBLE，复现留修复阶段写测试）。

## Global Constraints（逐字继承批 A 设计文档）
- **只入账不改 src**；**引用必亲验**（file:line 当场 Read/Grep，不靠 memory 旧描述/不靠 Explore 测绘锚点免验）；**subagent 结论必主控亲验后入账**，凭猜驳回。
- subagent 一律 opus（harness 拒 model:"opus" 时省略继承主会话）。
- **对照 2026-06-30 基线**（wiki 全覆盖结构审查，PR#74 修 2 High）：批 B 审**业务链/命脉视角**（结构正确性已覆盖，不重复）；聚焦该基线后演进 + 业务流缺陷。
- 台账续写 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（新增"## 批 B"大节 + 各链 section），finding 编号用 `KB-NN`。
- 防假绿：端点/MCP 失败标 BLOCKED；PLAUSIBLE/CONFIRMED 如实标；发现≠修复。
- 权威依据：CLAUDE.md「AI 永不自动核实」「产品声明须 verified knowledge 背书」红线、`docs/agent-policy.md`。

## Explore 测绘的链锚点（审查起点，实现者仍须自己 Read 亲验）
- 链1 录入：`routes/knowledge/import.rs`（preview:127/pdf:496/image:678/`ingest_chunked_text`:760）+ `media_assets.rs` + `knowledge_wiki/ingest_worker.rs`。恒 draft+needs_review（import.rs:315/362/824/880 多处压回）。
- 链2 审核：`routes/knowledge/verify.rs`（verify:66/reject:126/auto_verify:159）+ `wiki_edit.rs:934 batch_verify`。`clamp_sample_rate` 硬下限。
- 链3 grounding：`routes/knowledge/chat.rs`（chat_turn:42/chat_apply:367/apply_create_chunk:1668，:1050-1076 仅取 verified）+ `agent/domain_profile.rs:53`（product_fact 仅 verified 背书）+ `agent/review/gates.rs`（硬门）。
- 链4 修复：`routes/knowledge/repair.rs`（propose_chunk_repair:379/propose_pack_repair:576 死桩仍注册路由/record_repair_apply:629）+ `knowledge_wiki/gap_signals.rs`（2157 行最大）+ `structural_proposals.rs:118`。
- 链5 修订：`routes/knowledge/wiki_edit.rs` + `knowledge_wiki/chunk_revisions.rs:149 apply_chunk_revision`（唯一编辑落库入口）+ `page_merge.rs`（union/locked/patch）。
- 链6 质量：`routes/outcome_metrics.rs:30` + `routes/outcomes_autonomy.rs:219/443` + `knowledge_wiki/reviewer_stats.rs:49`。
- 链7 catalog：`routes/knowledge/catalog.rs`（catalog:45/completeness:105/refresh:136，F-013 TTL 缓存已挡；缓存 key 维度 :116 值得看）+ `catalog_rebuild.rs`。

---

## Task 1: 链3 grounding 召回链审查（最高优先）+ 批 B section 骨架

**Files:** 审查(只读) `routes/knowledge/chat.rs` + `agent/domain_profile.rs:53` + `agent/review/gates.rs`（grounding 相关）。台账新建"## 批 B"大节 + "### 链3 grounding" section。

- [ ] **Step 1: 建批 B section 骨架 + 读透 chat 召回过滤**

台账加"## 批 B（知识链）"大节 + 7 条链空 section 占位。Read `chat.rs` 召回快照（:1050-1076 起）：核 `integrity_status="verified"` 过滤是否覆盖**所有**召回入口（chat_turn / chat_apply / 命脉链 knowledge_router 复用的召回）——有无某入口漏过滤让 needs_review/draft 切片泄漏进背书？

- [ ] **Step 2: 三处协同核验（快照过滤 + prompt 约束 + 硬门）**

grounding 红线依赖三处独立协同：`chat.rs:1076` 快照过滤、`domain_profile.rs:53` product_fact 仅 verified、`gates.rs` blocked_unverified_product_claim 硬门。核三处判定的"verified"口径是否一致（同一 integrity_status 值？有无一处用旧字段/旧枚举）。任一处漏判即泄漏。

- [ ] **Step 3: apply_create_chunk 溯源锚定**（对照 memory D2/空 anchor 历史）

Read `chat.rs:1668 apply_create_chunk`：运营对话陈述转知识时，新 chunk 的 source_anchors/溯源是否正确锚定、初值是否 draft+needs_review（不能因为"是 AI 问答产出"就跳过人审红线）。

- [ ] **Step 4: 派 subagent 复审 + 主控亲验 + 入账 + Commit**

派 opus subagent 独立复审链3（指令：先读懂再断言、带 file:line、凭猜驳回）。主控逐条亲验 file:line 后写入台账"链3"section（含 ✅ 通过点）。`git commit -m "audit(batch-b): 链3 grounding召回链(最高优先)"`

---

## Task 2: 链4 修复链审查（次高优先）

**Files:** 审查(只读) `routes/knowledge/repair.rs` + `knowledge_wiki/gap_signals.rs` + `knowledge_wiki/structural_proposals.rs` + `knowledge_task/mod.rs`（execute_step Phase4）。

- [ ] **Step 1: "AI 只提议不落库主集合"红线三路径核验**

红线散落三处：`repair.rs:379 propose_chunk_repair_inner`、worker `mod.rs:458 execute_step fix_chunk`、`structural_proposals.rs:118`。逐一核：是否**真的零写 operation_knowledge_chunks 主集合**（只产草稿/信号/pending_review）？有无某路径直接改主集合绕过人审？

- [ ] **Step 2: 死桩误导核验**

`repair.rs:576 propose_pack_repair` 仍是死桩（返 400）但路由 `/items/:id/repair` 仍注册——前端调用即 400。核：这是死代码误导（应删路由）还是有意保留？记为 finding（严重度视前端是否真调）。

- [ ] **Step 3: gap_signals 业务逻辑**（2157 行最大单文件）

Read `gap_signals.rs` 核心：gap 信号生成/消解逻辑是否正确（不误报/不漏报致知识缺口被忽略）；信号状态流转；与 repair 的衔接。

- [ ] **Step 4: 主控亲验 + 入账 + Commit**

派 subagent 复审重点函数，主控亲验。写入台账"链4"。`git commit -m "audit(batch-b): 链4修复链(AI只提议不落库红线三路径)"`

---

## Task 3: 链2 审核链审查（第三优先）

**Files:** 审查(只读) `routes/knowledge/verify.rs`（verify:66/reject:126/auto_verify:159）+ `wiki_edit.rs:934 batch_verify`。

- [ ] **Step 1: auto-verify 红线对抗面核验**

"AI 永不自动 verify"的最大对抗面。核 `auto_verify_..._inner`（verify.rs:159）：`clamp_sample_rate` 硬下限是否真禁 0（=100% 无人审）？product_fact 类是否真全量强制人审？**分类判定有无边界漏洞**（如 product_fact 误判成其他类走 5% 抽样→产品事实漏过人审）？

- [ ] **Step 2: verify/reject/batch 状态流转**

核 verify(:66)→verified、reject(:126)、batch_verify(wiki_edit.rs:934) 的状态流转是否正确、幂等；batch 部分失败处理；越权（workspace/account 隔离）。

- [ ] **Step 3: 主控亲验 + 入账 + Commit**

`git commit -m "audit(batch-b): 链2审核链(auto-verify红线对抗面)"`

---

## Task 4: 链1录入 + 链5修订 + 链7 catalog 审查（合并·结构较稳）

**Files:** 审查(只读) `import.rs`/`media_assets.rs`/`ingest_worker.rs`（链1）+ `wiki_edit.rs`/`chunk_revisions.rs`/`page_merge.rs`（链5）+ `catalog.rs`/`catalog_rebuild.rs`（链7）。

- [ ] **Step 1: 链1 录入红线（恒 draft+needs_review）**

核 import 全入口（preview/apply/pdf/image + media_assets + ingest_worker）录入初值是否**恒** draft+needs_review（Explore 说 import.rs:315/362/824/880 多处压回，亲验是否**所有**入口都压回、有无遗漏入口）。

- [ ] **Step 2: 链5 修订统一入口 + 锁字段**

核所有 chunk 编辑是否**统一走** `chunk_revisions.rs:149 apply_chunk_revision`（有无旁路直改主集合）；`page_merge.rs` 的 union_array/enforce_locked/apply_field_patch 锁字段保护是否有效（不被 patch 绕过）。

- [ ] **Step 3: 链7 catalog 缓存 key 维度**

核 `catalog.rs:116 completeness_cache` 的 key 是否含 account_id/workspace_id 维度（Explore 提示：缺维度可能跨账号串味）——多账号下会不会 A 账号看到 B 账号的 completeness？F-013 TTL 缓存本身已确认挡住慢查询。

- [ ] **Step 4: 主控亲验 + 入账 + Commit**

`git commit -m "audit(batch-b): 链1录入+链5修订+链7catalog"`

---

## Task 5: 链6 质量链审查 + 批 B 汇总 + PR

**Files:** 审查(只读) `outcome_metrics.rs`/`outcomes_autonomy.rs`/`reviewer_stats.rs`（链6）。台账批 B 总评。

- [ ] **Step 1: 链6 质量聚合红线 + 口径**

核 outcome 审核链（mod.rs:885 list/reject 仅改信号、approve 落 staff_confirmed，**AI 永不直写 outcome**）；reviewer_stats pass_rate/misjudge_rate 计算口径；聚合读写 workspace_id 一致（批 A 环节⑧已核 outcome_metrics 一致，此处看 reviewer_stats/autonomy 侧）。

- [ ] **Step 2: 主控亲验 + 入账链6 findings**

- [ ] **Step 3: 批 B 总评**

台账加"批 B 总评"：finding 计数按严重度 + 跨链根因家族（若有）+ 修复优先级建议 + 与批 A 的关联（如是否有共性错误处理家族）。

- [ ] **Step 4: 自检防假绿 + Commit + PR**

复核每条 finding file:line 亲验、无夸大、无把设计当 bug。`git commit` + `git push` + `gh pr create`（纯审查台账无 src 改动，PR body 说明 findings 汇总 + 待修复）。

---

## Self-Review 结论
- **Spec coverage**：Explore 测绘 7 链 ↔ Task1(链3)/Task2(链4)/Task3(链2)/Task4(链1+5+7)/Task5(链6)，全覆盖；优先级（grounding>修复>审核）↔ 独立 task 顺序；只入账不修 ↔ 全 task 无 src 改动 + Global Constraints。
- **Placeholder scan**：无 TBD；每 Step 给具体审查问题 + Explore 锚点（file:line，实现者仍须亲验）。
- **一致性**：台账文件名/finding 编号(KB-NN)/链锚点跨 task 一致；2026-06-30 基线与 memory 历史点（propose_pack_repair 死桩/F-013 缓存/D2 anchor）在对应 task 作为"更深/复核"基线引用。
- **审查工程适配**：无 TDD；用"读码→对照红线→主控亲验→入账"替代；每 task 独立可提交 deliverable（台账 section）；117 真跑同批 A 定调（PLAUSIBLE，留修复阶段）。
