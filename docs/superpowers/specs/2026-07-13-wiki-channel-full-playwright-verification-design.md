# Wiki 频道全功能 Playwright 真实验证方案

日期：2026-07-13
范围：知识库 Wiki 频道（3 模式 × 21 视图）全功能真实浏览器验证
环境：生产 117（`http://117.72.54.28:3003`），库内为 2026-07-13 重导的 95 条星零感知识（全 `draft`）

## 目标

用 Playwright 真实浏览器点击，走完整前端交互（含反馈/弹窗/轮询），逐一核对 wiki 频道 21 个视图的功能是否正确。所有断言基于真实 API 响应（`page.on("response")` 捕获）+ 页面可见文案，不做脚本直连绕过 UI。

## 前置：已亲验的业务边界（每条附 file:line，非猜测/非 subagent 转述）

主会话逐条 Read 亲验的关键安全事实：

1. **对话工作台物理隔离**：`src/routes/knowledge/chat.rs:989-990` 明确「永不写库、永不进 outbox、永不进 mcp（与 user-ops gateway 物理隔离）；AI 永不自动 verify：chat 落库由 chat_apply 强制 status=draft + needs_review」。→ 对话发测试消息**不会**触达任何真实客户。
2. **auto-verify 绝不真放行进池**：`verify.rs:401` `enforce_verified_needs_human_audit` 对**所有** chunk_type 把 `verified` 强制降级 `needs_human_audit`（:554-559），仅「预审分诊」。→ 点自动核实不会让知识 active 进决策池。
3. **单条 verify 才是进池动作**：`verify.rs:104-122` 置 `integrity_status=verified` + `confidence_score=100` + `status=active`；且有 D2 硬闸（:88-96）——`source_quote` 非空且 `source_anchors` 非空才放行，否则 `AppError::BadRequest`。→ Tier 3 造的测试 chunk **必须**带 source_quote + 可锚定 source_anchors，否则 verify 返回 400。
4. **删除文档级联硬删**：`crud.rs:131-159` 先 `delete_one` 文档，再 `delete_many` 其下全部 chunks（`document_id` 匹配）。不可逆。→ Tier 3 只点到确认弹窗。
5. **新建切片经 D2 闸 + coerce**：`crud.rs:192-209` `create_operation_knowledge_chunk` 走 `coerce_integrity_against_d2_gate` + `operation_knowledge_chunk_from_request`（#186 挂 coerce 的必经点）。新建切片强制 draft。
6. **reset-system-pack 高危但本频道 UI 无入口**：后端 `mod.rs:809` / `management.rs:1192` 标 `Irreversible` 物理删重建，知识库 console 11 视图无任何按钮调用它 → Playwright 不会误触。

## 视图分级（21 视图 × 3 模式）

### Tier 1 — 纯只读，真实点击全验（10 个）

| 视图 | 模式 | 主要端点 | 验证点 |
|---|---|---|---|
| 知识收件箱 KnowledgeInbox | 工作台 | GET inbox | 列表渲染、深链跳转 |
| 问答 AskView | 知识库 | POST tools/search | 检索返回、命中渲染 |
| 知识树 KnowledgeTreeView | 知识库 | GET chunks | 树结构、分类分组 |
| 修订历史 ChunkRevisionsDrawer | 知识库 | GET revisions | 历史列表 |
| 概览 Cockpit | 控制台 | GET completeness/integrity-report/gap-signals | 看板卡片、维度按钮跳转 |
| 指标 MetricsTab | 控制台 | GET knowledge/metrics | 指标渲染、刷新 |
| 运营记忆 MemoryDrawer | 控制台 | GET operator-memory | kind 筛选、刷新 |
| 关系图谱 ChunkGraphView | 控制台 | GET chunks | 布局/染色/节点跳转 |
| 试召诊断 TryRecall | 控制台 | POST tools/search + open-slice | 检索/切片（POST 语义但不写库） |
| 诊断仪表 Observability | 控制台 | 9 路 GET + POST gap-signals/sweep（幂等） | 面板渲染、立即扫描（幂等安全） |

### Tier 2 — 有写操作但低风险/可清理，用一次性测试数据真验（7 个）

所有测试数据带可识别前缀 `[E2E验证]`，验证完清理。

| 视图 | 模式 | 写操作 | 落库性质 |
|---|---|---|---|
| 对话工作台 ChatWorkbench | 工作台 | POST chat（LLM 工具调用）、apply | apply 强制 draft，**不发客户** |
| 今日速览 DigestCanvas | 工作台 | POST regenerate（LLM）、dismiss | 生成/忽略，不进池 |
| TaskRail | 工作台 | tasks list/cancel | 任务态变更 |
| LintView | 知识库 | POST gap-signals/sweep、dismiss | 扫描/忽略信号，幂等 |
| 文档目录 DocumentsView | 控制台 | POST 新建文档/切片 | 新建强制 draft |
| 外部源 IngestSourcesView | 控制台 | POST 新增源、PATCH 重激活 | 源配置，填无害 URL |
| Inspector ChunkInspectorPane | 共享 | repair/relate | chunk 关联/修复 |

### Tier 3 — 危险/不可逆，只点到确认弹窗断言其出现（6 类），外加 1 条造数据全链

**只到弹窗、绝不确认**：
- 文档目录删除文档（crud.rs:131 级联硬删）
- Schema 激活（atlas.tsx:525 切全局判断口径）
- 治理 rollout 发布给全部客户（atlas.tsx:1002 不可逆）
- 治理 publish / rollback
- Inspector rollback（shared.tsx:1093）
- 外部源删除

**造数据全链真验（用户拍板，最彻底）**：
1. 用 POST 新建 1 条测试文档（带 raw_content 供锚定）+ 1 条测试 chunk（`[E2E验证]` 前缀，body 取材料原文片段，填 source_quote = 该原文片段，确保 D2 锚定可命中）。
2. 到评审队列真实点「核实」→ 期望 verify.rs D2 闸放行 → chunk 变 `active`+`verified`+`confidence=100`。
3. 断言：该 chunk 确实进入决策池（列表 integrity_status=verified/status=active）。
4. **清理**：验证完把该测试 chunk supersede（软废弃，复用 `superseded_by`，不新造 status）或直接 delete，测试文档 delete。确保生产库回到 95 条纯净态。

## Playwright 执行策略

- headed 可见 + slow_mo=350 + 全程截图（沿用 `scripts/e2e/wiki_reimport_*.py` 范式）。
- `page.on("response")` 捕获所有 `/api/` 响应，断言 HTTP 状态 + JSON 结构。
- 危险操作：定位按钮 → 点击 → 断言 `useConfirm` 弹窗出现（文案含「确认/删除/发布」）→ **截图 → 关闭弹窗**，不点确认。rollout 额外断言 requireText 输入框出现。
- 每个 Tier 分独立脚本，可单独重跑：`wiki_verify_T1_readonly.py`、`wiki_verify_T2_writes.py`、`wiki_verify_T3_danger_and_chain.py`。
- 脚本产出结构化 JSON 结果（每视图 pass/fail + 证据），汇总成一份核对报告。

## 清理与安全网

- 造的所有测试数据带 `[E2E验证]` 前缀，脚本结束自动清理 + 查库确认生产库回到 95 chunks/1 doc 纯净态。
- 真发红线：验证期间**绝不**与任何真实客户消息/发送套件并发。对话工作台经 chat.rs:989 隔离，本身不发客户。
- 生产 LLM 端点仅 2 线程：涉及 LLM 的操作（对话/regenerate/造数据）串行，不并发。

## 验证顺序

1. Tier 1 全只读（无副作用，先跑建立基线）。
2. Tier 3 危险操作只到弹窗（不产生副作用）。
3. Tier 2 写操作（造 `[E2E验证]` 测试数据）。
4. Tier 3 造数据全链（verify→active→池→清理）。
5. 查库终验：生产库回到 95 chunks/1 doc 全 draft 纯净态。

## 不做的事

- 不点任何真实确认的危险操作（删真实文档/激活 schema/rollout 给全部客户）。
- 不改任何业务代码/prompt/阈值（本轮是验证，非修改；若发现 bug 单独记录，另开修复流程）。
- 不碰 taxonomy/prompts/联系人集合。
