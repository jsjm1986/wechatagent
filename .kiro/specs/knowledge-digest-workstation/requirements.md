# Knowledge Digest Workstation — Requirements

> **节奏**：每日一份 AI 主动产出的知识库日报；运营在固定时间打开页面，逐条勾选交给 AI 处理。
> **形态**：三栏 — 左侧目录树（25%）/ 中间日报画布（45%）/ 右侧 chat（30%）。
> **目标**：把"知识库 = AI agent"落到产品形态。chat 是主入口；画布是当日工作台；目录树是索引。

本 spec 取代当前知识库频道的"三栏并排 + 编辑器跑到页面最下方"布局，并把上一轮已落地的 KnowledgeChatPanel（抽屉式对话补完）升级为常驻右侧 chat。

## R1 节奏与触发

R1.1 SHALL 每天 09:00（运营时区）由 `KnowledgeDigestWorker` 主动跑一次，扫描以下四个数据源并产出**当日**一份 `knowledge_daily_reports` 记录：
- `operation_knowledge_chunks`（缺字段 / `integrityStatus=needs_review` / `status=draft`）
- `knowledge_usage_logs`（最近 24h 的 chunk 命中率 / fallback 率 / route_result）
- `agent_run_logs`（最近 24h `final_review_status` ∈ block/hold 类，反查触发的 chunk）
- `evolution_*` 集合（演化器最近 24h 产出的 release/rollback 候选；只读）

R1.2 SHALL NOT 在节奏 1 阶段引入"事件驱动 push"——webhook 实时不会主动叫醒 chat。所有新增的"AI 主动消息"都汇集到当日日报，运营在固定时间消费。

R1.3 日报生成失败（LLM 不可用 / token 超预算 / Mongo 查询超时）SHALL 写入 `knowledge_daily_reports.status="failed"` 并标 `error_kind`，前端画布显示「AI 没能生成今日日报」+「重试」按钮，**不**降级为空白页。

R1.4 日报生成 SHALL 尊重 `RUN_BUDGET`（单次 worker tick token ≤ 24000；LLM call ≤ 8）。超额即终止本次 tick，已写入的 partial 报告保留并标 `status="partial"`。

## R2 画布形态（紧凑卡片列表）

> **决策**：紧凑卡片列表 vs 叙事周报式 → 选**紧凑卡片列表**。理由：节奏 1 单日 ~20 条 issue，运营需要批量勾选 + 横向对比，叙事周报式无法高吞吐。

R2.1 中间画布 SHALL 渲染 `knowledge_daily_reports.cards: KnowledgeDigestCard[]`，每张卡片包含：
- `kind`: `chunk_missing_field` / `chunk_low_hit_rate` / `chunk_caused_block` / `pack_outdated` / `evolution_pending` / `evolution_released` / `freeform`
- `title`: ≤ 60 字
- `summary`: ≤ 200 字（AI 一句话讲清楚问题）
- `targetRefs`: `Vec<{kind: "chunk"|"pack"|"item"|"run"|"evolution_proposal", id: ObjectId}>`
- `suggestedAction`: `enum { fix_chunk, add_chunk, retag, review_evolution, dismiss, freeform }`
- `severity`: `info | warn | critical`
- `metric`: `Option<{name, value, threshold}>`（命中率 / block 计数 / 缺字段数）

R2.2 卡片左侧 SHALL 有 checkbox；运营勾选 N 张后点页面顶部「💬 让 AI 处理选中的 N 条」按钮 → 把所选 cards 序列化注入右侧 chat 的输入区作为初始 prompt（不直接落库 task；先让运营在 chat 里和 AI 对齐）。

R2.3 单张卡片 SHALL 有快捷动作：「单独让 AI 处理」（=立刻在右侧 chat 起一个 turn）/「忽略」（在 `knowledge_daily_reports.dismissed_card_ids` 追加）/「打开关联对象」（跳转到目录树定位）。

R2.4 SHALL NOT 在画布里直接编辑 chunk —— 编辑入口仍走右侧 chat 或目录树展开后的 `KnowledgeChunkEditor`。画布是**审阅 + 派单**面板，不是编辑器。

R2.5 卡片排序 SHALL 优先级：`severity=critical` > `kind=chunk_caused_block` > `kind=chunk_missing_field` > 其他；同级内按 `targetRefs[0].id` 稳定排序保证刷新后顺序不抖。

## R3 Chat 形态（升级现有 KnowledgeChatPanel）

R3.1 上一轮 KnowledgeChatPanel 抽屉模式 SHALL 改为右侧 30% 常驻面板；session 持久化机制（`knowledge_chat_turns`）保留不变。

R3.2 chat 接受三类输入：
- 运营自由对话（如"再加一条针对宝妈的反对话术"）— 走现有 intent 分类
- 卡片注入（R2.2/R2.3 → 自动生成 `attachments`）— 新增 intent `digest_action`
- long-running task 进度回执（worker → chat 系统消息）— 见 R4

R3.3 chat 仍走 `RUN_BUDGET.scope`（per-turn ≤ 16000 token / ≤ 6 LLM call；per-session ≤ 8 turn）。LLM 不可用 SHALL 走上一轮已落地的 `LlmUnavailableError` 路径——红色 banner + 「AI 重试」按钮，**不**降级为静默失败。

R3.4 chat 输出的 chunk 草稿 SHALL 强制 `status="draft", integrityStatus="needs_review"`（与现有 chat apply 规则一致）；运营在 `KnowledgeChunkEditor` 二次审核走 #329 sourceQuote → anchor 模糊匹配 gate 才能进 verified 池。AI 永不自动 verify。

## R4 Long-running Task

> **决策**：完成边界 = AI 直接落 `draft + needs_review`，不要求每条都在 chat 里点确认。理由：与 R3.4 一致；批量处理 10 条以上时"逐条确认"会让运营被 chat 流淹没。

R4.1 当 chat 接到一次任务 ≥ 3 个 cards 或预估 LLM call > 6 时，SHALL 落库为 `knowledge_chat_tasks` 记录而不是单 turn 同步执行：
```text
{ _id, sessionId, accountId, operatorId, cards: [...], status, plannedSteps, completedSteps, results: [...], createdAt, startedAt?, finishedAt?, errorKind? }
```

R4.2 后台 `KnowledgeTaskWorker`（独立于现有 follow-up task worker）SHALL 串行执行同 sessionId 内的 task，保证 chat 上下文不被并发污染。

R4.3 worker 每完成一条 card SHALL 写一条 `knowledge_chat_turns { role: "system", kind: "task_progress", content: "已起草第 N/M 条草稿，chunkId=..." }`；前端 chat 长轮询或 SSE 拉到新 turn 后追加显示。

R4.4 task 整体完成 SHALL 写一条 summary turn `{ role: "assistant", kind: "task_summary", content: "本次共起草 3 条草稿、修订 2 条；请去 chunk 编辑器审核 5 条 needs_review。" }`，并把生成的 chunk 列表附在 `task_summary.attachments`。

R4.5 task 失败 / 超预算 SHALL 走 `LlmUnavailableError` 路径（与 R3.3 一致），并在 `knowledge_chat_tasks.errorKind` 留分类；前端可点「AI 重试」重新派工。

## R5 Operator Memory（与 chunk memoryCard 物理隔离）

R5.1 SHALL 新增独立 collection `knowledge_operator_memory`，schema：
```text
{ _id, accountId, operatorId, kind: "preference"|"rejection"|"context", content: String, createdAt, lastUsedAt }
```
**禁止**复用现有 `contacts.memory_card`（contact 维度）/ `agents.soul.memory`（agent 维度）。运营偏好不能污染对客户/agent 的记忆。

R5.2 chat 在 intent 分类前 SHALL 注入运营当日相关的 `knowledge_operator_memory.content`（≤ 5 条，按 `lastUsedAt` 倒序），让 AI"记得运营昨天说过不要再起带绝对承诺词的草稿"。

R5.3 运营在 chat 里说「以后不要再起带『100%回奶』这种话术」SHALL 触发 AI 写入一条 `kind=rejection` 的 operator memory（不是 chunk 修改）。识别由 `knowledge.chat.intent` 增加 `intent=update_operator_memory` 分支。

R5.4 SHALL NOT 让 AI 静默写运营记忆 —— 任何写入都在 chat 里有一条 assistant turn 显式说明「我已记下这条偏好」并附 memoryId，让运营随时可以撤销。

## R6 Tool-calling 扩展

R6.1 chat 内 LLM SHALL 启用 4 个新工具（基于现有 `agent::tool_loop` 注入模式，不引入新框架）：
- `audit_completeness({chunkId|packId})` — 调现有 `/api/operation-knowledge/chunks/:id/audit-completeness` 计算缺字段
- `search_chunks({query, topK})` — 走现有 BM25 + tag 检索
- `propose_repair({chunkId, hints})` — 调现有 `/api/operation-knowledge/chunks/:id/repair`
- `analyze_logs({contactWxid?, hours})` — 新增只读路由 `/api/operation-knowledge/logs/analyze`，反查 24h 内 `agent_run_logs.final_review_status` 为 block/hold 的 run，关联到 chunk 命中率

R6.2 工具调用 SHALL 受 `RUN_BUDGET` 限制；单 turn 内 tool call ≤ 6 次。超额 LLM 必须 fail-fast 写入 turn `kind=tool_budget_exceeded` 并停止本 turn。

R6.3 工具调用结果 SHALL 写入 `knowledge_chat_turns.attachments.tool_calls: [{name, params, result, latency_ms, tokens}]`，让运营在 chat 里能展开查看 AI 看到了什么。

## R7 红线（CI 守门 / 不可降级）

R7.1 R11.6 baseline 不能跌：`cargo test --lib` ≥ 78 / 0；4 PBT 累计 ≥ 33 / 0。本 spec 实施 SHALL 新增 ≥ 6 个单元测试覆盖：digest worker 卡片生成 / 卡片排序 / task worker 串行 / operator memory 隔离 / tool-calling budget / digest LlmUnavailable 失败。

R7.2 `bash scripts/check-no-human-takeover.sh` 0 命中。本 spec 文案统一：「AI 主动汇总」「让 AI 处理选中」「AI 起草草稿」「AI 记下偏好」「AI 没能生成日报」；**不写**「人工审核 / 人工接管 / takeover / hand-off」。

R7.3 设计 token 强制：bg `#f6f8fb` / surface `#ffffff` / ink `#111827` / muted `#64748b` / accent `#2563eb` / AI `#0f766e`；圆角 8/6；阴影 `0 6px 18px rgba(15,23,42,0.06)`。卡片复用现有 `.cardSurface` 类；chat 气泡复用现有 `.bubbleRow / .bubble`，**不**新增第三套气泡样式。

R7.4 零新依赖。React 19 + lucide-react + 单一 `styles.css`；后端无新 crate。SSE / 长轮询用现有 `tokio::sync::watch` + `axum::response::sse`，不引 redis-pubsub。

R7.5 AI 永不自动 verify（同 R3.4）。AI 永不直接修改 `prompt_templates` / `threshold_overrides`（演化器红线，本 spec 不触碰）。

R7.6 LLM 错误统一走上一轮 `LlmUnavailableError` 路径，不允许新增第二套错误样式。

## R8 数据模型

```text
knowledge_daily_reports
  _id, accountId, reportDate (YYYY-MM-DD), generatedAt, generatedBy: "worker"|"manual",
  status: "ok"|"partial"|"failed",
  errorKind?: String,
  budgetSnapshot: { tokensUsed, llmCalls },
  cards: [KnowledgeDigestCard],
  dismissedCardIds: [ObjectId],
  promptVersions: { intent, draft, ... }
索引: { accountId: 1, reportDate: -1 }（unique compound 防止同日重复）

knowledge_chat_tasks
  _id, sessionId, accountId, operatorId,
  cards: [KnowledgeDigestCard],  // 任务起源的 cards 快照
  plannedSteps: [{cardId, action: "fix_chunk"|"add_chunk"|...}],
  completedSteps: [{cardId, action, chunkId?, error?}],
  status: "pending"|"running"|"finished"|"failed"|"cancelled",
  errorKind?: String,
  createdAt, startedAt?, finishedAt?
索引: { sessionId: 1, status: 1 }, { status: 1, createdAt: 1 }（worker 取 pending）

knowledge_operator_memory
  _id, accountId, operatorId, kind: "preference"|"rejection"|"context",
  content: String, createdAt, lastUsedAt, expiresAt?
索引: { accountId: 1, operatorId: 1, lastUsedAt: -1 }

knowledge_chat_turns（已存在，扩展）
  追加字段: kind?: "task_progress"|"task_summary"|"tool_call_log"|null
  追加字段: attachments.tool_calls?: [{name, params, result, latency_ms, tokens}]
```

## R9 路由

```
GET  /api/knowledge/digest/today                  当日日报（无则触发一次合成）
POST /api/knowledge/digest/regenerate             手动重算（运营点画布的「🔄 重算今日」）
POST /api/knowledge/digest/cards/:cardId/dismiss  忽略一张卡片
POST /api/knowledge/chat/tasks                    chat 派工（body: {sessionId, cardIds[]}）
GET  /api/knowledge/chat/tasks/:taskId            轮询任务进度
POST /api/knowledge/chat/tasks/:taskId/cancel     取消未开始的任务
GET  /api/knowledge/chat/sessions/:sid/stream     SSE 长连接，推送新 turn
GET  /api/operation-knowledge/logs/analyze        R6.1 第 4 个工具的只读后端
```

`POST /api/operation-knowledge/chat` / `chat/:sid/apply` 等已有路由不变。

## Out of scope（节奏 1 明确不做）

- 事件驱动 push（webhook 实时叫醒）
- 移动端适配
- 跨 session 全文搜索
- chat 分支历史（fork session）
- 多人同时编辑同一 sessionId 的冲突解决（保留现有 expectedVersion 乐观锁）
- 把日报扩展到 user-ops / group / moments（仅知识库）
