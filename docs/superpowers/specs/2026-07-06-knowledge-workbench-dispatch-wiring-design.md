# 知识库工作台「派工长任务」正规链路补通 — 设计

日期：2026-07-06
分支起点：origin/main

## 背景与问题

知识库工作台（`frontend/src/features/knowledge/index.tsx` 的 workbench 模式）的「派工长任务」当前是**结构性空转**：运营派工后进度条走完、TaskRail 显示 `completed`、summary 报"成功 N 步"，但知识库实际零变化。

根因由三段拼成，均经 `file:line` 亲验：

1. **6 个 action 里 5 个是纯文案桩**（`src/knowledge_task/mod.rs:450-549`）：`analyze_logs`（前端默认选中，`today.tsx:69`）/`add_chunk`/`retag`/`review_evolution`/`dismiss` 全只返回一句成功文案、`details: None`、不改任何库；只有 `fix_chunk` 真接了 `propose_chunk_repair_inner`（`mod.rs:480`）。
2. **唯一真干活的 `fix_chunk` 永远拿不到 `targetChunkId`**：它首行读 `step.targetChunkId`，缺则 fail-soft 返回"缺 targetChunkId 未生成"（`mod.rs:462-470`）。而没有任何入库路径会填这个字段。
3. **派工入口只有一个手打自由文本框**（`today.tsx:412-419`），产的 step 只含 `{stepId, action, description}`（`today.tsx:258-262`），既无 `cardId` 也无 chunk 上下文；后端设计中的「卡片驱动」正规路径（`chat_turn` 返回 `plannedSteps`，`chat.rs:315`）前端从未承接（`ChatTurnResponse` 类型无 `plannedSteps` 字段，`today.tsx:44-56`）。

## 目标

让运营派工后知识库**真正产生结果**：真改知识的动作产 `draft + needs_review` 草稿（红线：worker 永不自动 verify），只读/标记类动作产真实副作用而非假成功文案。

## 三个已定决策（用户拍板）

1. **两条结构化入口都补通**：卡片驱动（今日摘要勾选卡片批量派工）+ 对话驱动（承接 `plannedSteps` 渲染派工确认小卡）。
2. **6 个 action 分层全实现**（见下表）。
3. **删掉手打自由文本框**：它是空转第三源头；补通后两条结构化入口都自带 `cardId`。

## 架构：两条入口汇入同一执行链

```
[入口A 卡片驱动]
今日摘要(DigestCanvas)多选卡片 → 批量派工按钮
  → POST /knowledge/chat/tasks {sessionId, cardIds, plannedSteps(前端按 卡片.cardId+suggestedAction 拼)}
                                              ↓
[入口B 对话驱动]                        chat_task_create (chat.rs:1865):
AI协作输入"把这几张 fix 了" → chat_turn      · 按 step.cardId 反查今日日报卡片 target_refs
  → 返回 plannedSteps(chat.rs:315)            · 解析 kind=="chunk" 的 id → 烤入 step.targetChunkId
  → 前端渲染派工确认小卡 → 确认               · 落库 knowledge_chat_tasks{status=pending}
  → POST 同一端点                                    ↓
                                            worker tick(mod.rs:165) → execute_step 按 action 分派
                                                   ↓
                                            真产 draft+needs_review 草稿 / 只读摘要 / 状态标记
                                                   ↓
                                            TaskRail 显示真实进度 + summary 列待审 chunkId
```

两条入口最终都汇到同一个 `chat_task_create` + 同一个 worker，只是 `plannedSteps` 来源不同（卡片驱动前端直接拼、对话驱动 LLM 产）。后端只维护一套执行逻辑。

## targetChunkId 解析（补通链路的技术核心）

**在 `chat_task_create`（派工落库时）解析，而非 worker。** 依据（均亲验）：
- `chat_task_create` 已在按 `card_ids` 从今日日报反查 `card_snapshots`（`chat.rs:1935-1944`）。
- `KnowledgeDigestCard.target_refs` 确含 `{kind:"chunk", id}`（`models.rs:4626-4629`）。
- 解析模式已有先例：`digest_inbox.rs:346-367` 提取 `kind=="chunk"` 的 id。

**实现**：`chat_task_create` 遍历 `plannedSteps` 时，对每个带 `cardId` 的 step，查对应卡片的 `target_refs`，取第一个 `kind=="chunk"` 的 id 写入 `step.targetChunkId` 再入库。解析一次、确定性、可单测；worker 保持"只读 `step.targetChunkId`"不变。若卡片无 chunk 类 ref（如 `review_evolution`/`analyze_logs` 卡），不写该字段（这些 action 本就不需要）。

## 6 个 action 分层实现

红线：`fix_chunk`/`add_chunk`/`retag` 都只产 `draft + needs_review`，worker 永不 verify。已复核落库路径硬编码：`apply_create_chunk`（`chat.rs:1680-1681`）、`apply_update_chunk`（`chat.rs:1776-1777`）均强制 `status="draft" + integrity_status="needs_review"`。

| action | 层级 | 真正做什么 | 地基（亲验） |
|---|---|---|---|
| `fix_chunk` | 真产草稿 | 调 `propose_chunk_repair_inner` 生成修复草稿塞进 step details | 已实现（`mod.rs:480`），只差 `targetChunkId` |
| `add_chunk` | 真产草稿 | 用卡片上下文起草新条目 → 落 draft+needs_review | 落库：`apply_create_chunk`（`chat.rs:1668`，pub，已强制 draft）现成；起草 LLM 环节需小重构（见下） |
| `retag` | 真产草稿 | 对 targetChunkId 重抽 productTags/businessTopics → 写回该 chunk 草稿（不 verify） | 抽标签逻辑在 `extract_operation_knowledge_tags`（`import.rs:193`，是路由 handler）；写回可用 `apply_update_chunk` 语义（`chat.rs:1711`，已支持 product_tags/business_topics 字段 + 强制 draft）。两者都需抽成可从 worker 调的 inner |
| `analyze_logs` | 只读报告 | 真查 events 集合 24h 内 block/hold 事件 → 聚合摘要写进 turn details | 新增只读查询（读 `AgentEvent` kind/status，取值空间已全量枚举） |
| `dismiss` | 状态标记 | 真把对应 digest 卡片标记 dismissed | 复用 `digest_dismiss_card` 的 `$addToSet dismissed_card_ids`（`digest_inbox.rs:143-145`）by workspace+report_date+cards.cardId |
| `review_evolution` | 跳转指引 | 产一条"请去自优化中心评估候选"的 turn（不假装自动评估） | 纯文案 turn |

### add_chunk / retag 的小重构（不是纯复用）

- **add_chunk**：`draft_chunk_for_chat`（`chat.rs:1350`）是 private 且依赖 chat history。worker 里没有对话 history，改为 worker 直接用卡片 summary/title 作为上下文调 `generate_agent_json`（`knowledge.chat.draft_chunk` prompt 复用），拿到 patch 后调已 pub 的 `apply_create_chunk` 落库。
- **retag**：`extract_operation_knowledge_tags`（`import.rs:193`）是收 `Json` 的路由 handler，不能内联调；抽出其 LLM 调用 + normalize 逻辑成 `pub(crate)` inner（输入 title+body → productTags/businessTopics），worker 调 inner 拿标签，再走 `apply_update_chunk` 的 `$set{product_tags, business_topics, status:draft, integrity_status:needs_review}` 写回。
- **fix_chunk**：`propose_chunk_repair_inner` 已是 `pub(crate)`（`repair.rs:218`），worker 已在调，无需重构，仅靠 targetChunkId 解析即打通。

## 前端改动

1. **DigestCanvas（`today.tsx:638`）**：给卡片加多选态 + "批量派工"按钮；选中卡片按 `{cardId, action: suggestedAction}` 拼 `plannedSteps`，POST `/knowledge/chat/tasks`。
   - **闭集对齐（自查发现，已亲验）**：卡片 `suggestedAction` 闭集（`knowledge_digest/mod.rs:624`）含 `freeform`，但派工校验闭集 `ALLOWED_TASK_ACTIONS`（`chat.rs:1894`）**不含** `freeform`。因此 `suggestedAction=="freeform"` 的卡片**不可批量派工**（freeform 语义就是"仅查看"，无对应执行动作）——前端多选时必须排除/禁选这类卡，否则会被 `chat_task_create` 400 拦。其余 5 个 suggestedAction 值都在 ALLOWED_TASK_ACTIONS 内，可直接映射。反向：`analyze_logs` 在 ALLOWED_TASK_ACTIONS 内但不是任何卡片的 suggestedAction，只能经对话驱动路径由 LLM 产出，卡片驱动路径不产它。
2. **ChatWorkbench（`today.tsx:58`）**：`ChatTurnResponse` 类型补 `plannedSteps` 字段；`submit()` 拿到响应后，若含非空 `plannedSteps` 则渲染「派工确认小卡」（展示步骤 + action），运营确认后 POST 同一端点。
3. **删除手打自由文本框**：移除 `stepsText`/`stepAction`/`dispatchTask` 相关 UI 与状态（`today.tsx:67-69, 249-294, 407-445`）。派工只能从卡片驱动或对话驱动发起。

## 错误处理与边界

- **fail-soft 保持**：单 step 失败不阻断后续（`run_task` 现有，`mod.rs:290-302`），失败文案变准确（不再假成功）。
- **解析失败诚实标注**：`chat_task_create` 若某需 chunk 的 action（fix_chunk/retag）的 cardId 查不到 chunk 类 ref，该 step 落库时标记原因；worker 执行到时返回明确文案而非假成功，但不阻断整批。
- **预算**：每 step 仍在 `RUN_BUDGET.scope`（`STEP_TOKEN_BUDGET`，`mod.rs:251-256`），LLM 类 action 超额 fail-soft。
- **ALLOWED_TASK_ACTIONS 闭集校验保留**（`chat.rs:1894-1917`）。
- **红线**：worker 严禁引用 gateway/outbox/mcp 写入路径（`mod.rs:12-15`）；所有 chunk 写入强制 draft+needs_review。

## 测试

- **后端（本地 lib，不需 Docker）**：
  - `chat_task_create` 的 cardId→targetChunkId 解析纯函数单测（卡片有/无 chunk 类 ref 的分支）。
  - `execute_step` 各 action 的 fail-soft 单测（缺 targetChunkId、正常、LLM 失败）。
  - retag inner（title+body → tags）纯逻辑单测。
- **前端（vitest）**：
  - DigestCanvas 多选 + 批量派工 POST 契约（含 plannedSteps camelCase 键）。
  - ChatWorkbench 承接 plannedSteps 渲染派工确认小卡。
  - 删手打框后同步相关断言。
- **三关**：`npm run build` + `npx vitest run` + `bash scripts/check-no-human-takeover.sh`。

## 非目标

- 不改 worker 的串行/SSE/清理机制（现有健全）。
- 不复活 `operation_knowledge_items`（pack 层已物理删除）。
- 不动 `propose_pack_repair` 死桩。
- 不改 TaskRail 的跟踪/取消机制（现有真实）。
