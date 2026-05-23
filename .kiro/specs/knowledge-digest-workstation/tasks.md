# Knowledge Digest Workstation — Tasks

> 分 5 个 phase 推进；每 phase 单独 commit，单独可回滚。
> 每 phase 完成后必须跑：`cargo check` → `cargo test --lib` ≥ 78 / 0 → `bash scripts/check-baseline.sh` → `bash scripts/check-no-human-takeover.sh` 0 命中 → `cd frontend && npx tsc --noEmit` 0 错。

任一项 ✗ → 立即停下，按 `docs/agent-policy.md` runbook §5 七步流程定位修复，**不**继续推进下一 phase。

## Phase 1 — worker 骨架 + 数据模型（无前端、无 LLM）

目标：让 `cargo run` 启动后台 `KnowledgeDigestWorker`（默认关停），新增 4 个 collection 与 index，新增 1 个最小路由 `GET /api/knowledge/digest/today`（命中返回；未命中 404，先不触发合成）。

1. **配置**：`src/config.rs` 加 5 项（参考 design §2 表）。`cargo check`。
2. **Models**：`src/models.rs` 加 `KnowledgeDigestCard` / `KnowledgeDailyReport` / `KnowledgeChatTask` / `KnowledgeOperatorMemory` 四个 struct；`KnowledgeChatTurn` 加 `kind: Option<String>` 与 `tool_calls: Option<Vec<...>>`（serde default + skip_serializing_if）。`cargo check`。
3. **DB accessor**：`src/db/mod.rs` 加 3 个 typed accessor（`knowledge_chat_turns()` 已存在不动）。
4. **Indexes**：`src/db/indexes.rs` 加 3 条 index entry（参考 requirements R8）。
5. **路由骨架**：`src/routes/knowledge.rs` 末尾加 `digest_today` handler（仅查 `knowledge_daily_reports` upsert 索引找当日，未命中返回 404 而**不**触发合成）；`src/routes/mod.rs` mount。
6. **Worker 骨架**：新建 `src/knowledge_digest/mod.rs`，导出 `worker_loop(state: AppState)`；启动函数检查 `KNOWLEDGE_DIGEST_ENABLED`，关停就 return；开启时按 `KNOWLEDGE_DIGEST_RUN_HOUR` 计算下次 09:00 时刻，sleep + tick；`generate_today_digest` 内部 `todo!()` 占位。`src/main.rs` `tokio::spawn` 一份。`cargo check`。
7. **测试**：`tests/knowledge_digest_skeleton.rs`（不依赖 testcontainers）—— 测 (a) `KnowledgeDigestCard` JSON 正反序；(b) `KnowledgeDailyReport.status` 必填且为 closed enum；(c) `digest_today` 命中/未命中两路；(d) 配置默认值（`KNOWLEDGE_DIGEST_ENABLED=false`）。

**验收**：`cargo test --lib` ≥ 80 / 0；`scripts/check-baseline.sh` 0；`bash scripts/check-no-human-takeover.sh` 0 命中。

**Commit 1**：`feat(knowledge-digest): phase1 worker skeleton + data models`

## Phase 2 — `generate_today_digest` 真实合成（吃 4 数据源 + LLM compose）

目标：worker 实际可以跑通一次，吐一份 `knowledge_daily_reports`。

1. **PromptSpec**：`src/prompts.rs` 末尾加 3 条 PromptSpec（`knowledge.digest.compose` / `knowledge.digest.dispatch` / `knowledge.digest.summarize_logs`），注册进 `ensure_prompt_pack_v2` seed。
2. **4 个 analyze 函数**：`src/knowledge_digest/mod.rs` 内：
   - `analyze_chunks_health(db) -> Vec<RawSignal>`：扫 `operation_knowledge_chunks` 找 missing field / draft 滞留 ≥ 7 天。
   - `analyze_usage_logs(db, hours=24) -> UsageDigest`：聚合 `knowledge_usage_logs` 命中率 / 落空 query。
   - `analyze_run_logs(db, hours=24) -> Vec<BlockSignal>`：扫 `agent_run_logs.final_review_status ∈ {block,hold}` 反查 `triggered_chunk_ids`；用 `knowledge.digest.summarize_logs` 给每组生成 1 句话。
   - `analyze_evolution(db, hours=24) -> Vec<EvolutionSignal>`：只读 `proposals` 最近 24h `eligible_for_release | rolled_back`。
3. **`compose_cards`**：把 4 路信号 → JSON → 调 `generate_agent_json("knowledge.digest.compose", ...)` → 校验 schema → 外键校验 → 排序 → 截断 50。
4. **`generate_today_digest`** 实装：upsert `knowledge_daily_reports`；写 `KnowledgeUsageLog{kind="digest_compose"}` + `AgentEvent{kind="knowledge_digest_generated"}`；失败统一走 `AppError::LlmUnavailable` 标 `status="failed"`。
5. **`digest_today` 升级**：未命中时调用 `generate_today_digest` 同步合成（带 `RUN_BUDGET.scope`）。
6. **`digest_regenerate`**：手动重算路由；`force=true` 跳过缓存。
7. **`digest_dismiss_card`**：路由实装（追加到 `dismissedCardIds` 数组）。
8. **测试**：
   - `tests/knowledge_digest_compose_smoke.rs`（mock `LlmGenerator` 返回 fixed cards）：(a) compose 成功路径；(b) LLM 返回非法 JSON 走 partial；(c) targetRefs 指向不存在 chunk → 整张卡片丢弃；(d) `cards.len() > 50` 截断；(e) 卡片排序 R2.5 稳定。
   - `tests/knowledge_digest_budget_smoke.rs`：(f) `RUN_BUDGET` 超额 → status="failed"；(g) `LlmUnavailable` 路径不污染 collection。

**验收**：`cargo test --lib` ≥ 86 / 0；CI 双 lint；`cargo run` + 临时设 `KNOWLEDGE_DIGEST_ENABLED=true` + `KNOWLEDGE_DIGEST_RUN_HOUR=$(date +%H + 1 in 5min)` 观察一次完整 tick；mongosh 查 `knowledge_daily_reports` 出现今日记录。

**Commit 2**：`feat(knowledge-digest): phase2 generate_today_digest with 4 analyzers`

## Phase 3 — 三栏布局 + 画布 UI

目标：把 Knowledge 频道从抽屉式 chat 切换到三栏常驻；画布渲染 cards；checkbox 派工初步可用（落到 chat 输入区，**不**真起 task worker，留 phase 4）。

1. **fetch handlers + state**：`frontend/src/App.tsx` 顶层加 5 个 fetch handler（`getDigestToday` / `regenerateDigest` / `dismissDigestCard` / `createChatTask` / `getChatTask`）+ 选中 cards state + chat 常驻 state。`tsc --noEmit`。
2. **`<KnowledgeWorkstation>`**：拆掉旧的两栏 + 抽屉，按 design §4 三栏栅格重排；左侧仍是现有目录树组件（不动）；中间嵌新组件 `<KnowledgeDigestCanvas>`；右侧把上一轮 `<KnowledgeChatPanel>` 从抽屉模式切到常驻面板（加 `mode: "drawer" | "docked"` prop）。
3. **`<KnowledgeDigestCanvas>`**：渲染 `report.cards`，每张卡片 checkbox + severityChip + 三按钮（让 AI 单独处理 / 忽略 / 打开）；顶部 4 个聚合按钮（重算今日 / 全选 / 反选 / 一键忽略 info）；底部「💬 让 AI 处理选中的 N 条」按钮（点击：拼成 chat 系统 turn 注入 chat 输入区）。
4. **错误态**：`status="failed"` 时显示 `<LlmErrorBanner>`（复用上一轮组件）+「重试」按钮 → 调 `regenerateDigest`。
5. **CSS**：`frontend/src/styles.css` 末尾追加 `/* === knowledge digest workstation === */` 段；卡片复用 `.cardSurface`，气泡复用 `.bubbleRow / .bubble`，仅追加：栅格、severityChip、metricChip、卡片头部布局。
6. **lint + 构建**：`bash scripts/check-no-human-takeover.sh` 0 命中；`cd frontend && npx tsc --noEmit && npm run build` 干净。

**验收**：手动 cargo run + 浏览器进 Knowledge 频道：(a) 09:00 之前空 → 同步合成卡片渲染；(b) 勾选 3 张点 CTA → 看见 chat 输入区被注入 cards 序列化文本；(c) 点单卡「让 AI 单独处理」→ 同样注入 chat。

**Commit 3**：`feat(knowledge-digest): phase3 three-column layout + digest canvas`

## Phase 4 — long-running task + SSE 进度

目标：派工真的落到 `knowledge_chat_tasks`，由 `KnowledgeTaskWorker` 串行处理，进度 SSE 推回 chat。

1. **路由**：`chat_task_create` / `chat_task_get` / `chat_task_cancel` / `chat_session_stream` 四 handler 实装（design §6 签名）。
2. **`KnowledgeTaskWorker`**：新建 `src/knowledge_task/mod.rs`；30s tick 取 `status=pending` task；按 sessionId 串行（同 sessionId 多 task 排队）；逐步执行 plannedSteps（fail-soft）；每完成一步写 `knowledge_chat_turns{kind="task_progress"}`；全部完成写 `kind="task_summary"` 列出 needs_review chunkIds。
3. **`src/main.rs`** `tokio::spawn` worker；可由 `KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS=0` 停掉（调试便利）。
4. **chat dispatch intent**：`src/routes/knowledge.rs` 现有 chat_turn 增加 `intent="digest_action"` 分支；调 `knowledge.digest.dispatch` prompt 出 plannedSteps；前端拿到 plannedSteps 后弹「派工确认」小卡。
5. **SSE**：`tokio::sync::watch` per sessionId（`HashMap<SessionId, watch::Sender<u64>>`）；`chat_session_stream` 订阅；前端 `EventSource('/api/knowledge/chat/sessions/:sid/stream')` 收到新 turn id 后 GET history 拉新 turn 追加。
6. **测试**：
   - `tests/knowledge_task_worker.rs`：(a) sessionId 串行；(b) cancel 在 running 中即时退出当前步前；(c) fail-soft 一步失败不阻塞后续；(d) summary turn 列出所有 needs_review chunkId。
   - `tests/knowledge_chat_dispatch.rs`：(e) `intent=digest_action` → plannedSteps 估算 > 6 → 前端拆批（这条主要前端，后端测路径返回 estimatedLlmCalls）。

**验收**：`cargo test --lib` ≥ 92 / 0；浏览器跑全链路：勾选 3 卡 → 派工确认 → chat 实时显示 3 条 progress + 1 条 summary；mongosh 查 `knowledge_chat_tasks` 状态完整 + `knowledge_chat_turns` 多出 progress/summary 类型 turn。

**Commit 4**：`feat(knowledge-digest): phase4 long-running task worker + SSE`

## Phase 5 — operator memory + tool-calling 扩展

目标：chat 在 intent 分类前注入运营记忆；4 个 tool-calling 工具上线；`analyze_logs` 只读路由可被 tool 调用。

1. **operator memory**：
   - `KnowledgeOperatorMemory` collection accessor + index 已在 phase 1 落地，本 phase 仅实装 read/write 路径。
   - `src/agent/memory.rs` 加 `load_operator_memory(db, accountId, operatorId, top_n=5)`，注入 chat system prompt 头部。
   - `src/prompts.rs` `knowledge.chat.intent` 增加 `intent="update_operator_memory"` 分支输出 schema：`{kind, content}`；命中时 `chat_turn` handler 写一条 memory + assistant turn 显式确认。
2. **tool-calling**：
   - `src/agent/knowledge_tools.rs` 内增加 4 个 tool：`audit_completeness` / `search_chunks` / `propose_repair` / `analyze_logs`；走现有 `tool_loop` 注入模式。
   - `src/routes/knowledge.rs` 增加 `GET /api/operation-knowledge/logs/analyze` 只读路由（24h 内 block/hold runs 反查 chunkIds，与 phase 2 `analyze_run_logs` 复用同一查询函数）。
   - chat per-turn tool call ≤ 6（`RUN_BUDGET` 已就位，仅在 tool_loop 加计数 + 超额抛错）。
3. **测试**：
   - `tests/knowledge_operator_memory_isolation.rs`：(a) 写 operator memory **不**触达 `contacts.memory_card`；(b) `intent=update_operator_memory` 走 chat 显式 assistant turn；(c) operator memory `lastUsedAt` 正确更新。
   - `tests/knowledge_tools_budget.rs`：(d) tool call ≤ 6；(e) `analyze_logs` 路由只读不写 outbox。

**验收**：`cargo test --lib` ≥ 96 / 0；浏览器：(a) chat 输入"以后别再起带 100% 回奶" → 看到 assistant turn 确认 + mongosh 看到 `knowledge_operator_memory` 新条目；(b) chat 输入"24h 内 fact_risk 拦截了哪些 chunk" → AI 调 `analyze_logs` 工具 → 在 chat 里展开 tool_calls 看到 raw 结果。

**Commit 5**：`feat(knowledge-digest): phase5 operator memory + tool-calling expansion`

## 全局验收（commit 5 之后）

| # | 命令 | 期望 |
|---|---|---|
| 1 | `cargo test --lib` | ≥ 96 / 0 |
| 2 | `bash scripts/check-baseline.sh` | exit 0（lib ≥ 78, PBT ≥ 33） |
| 3 | `bash scripts/check-no-human-takeover.sh` | 0 命中 |
| 4 | `cd frontend && npx tsc --noEmit && npm run build` | 0 错误 + dist 产出 |
| 5 | 真实 cargo run，浏览器 9:00 看日报 | 三栏齐全；卡片 ≤ 50；勾选派工 → 3 条 progress + 1 summary |
| 6 | mongosh 查 `knowledge_daily_reports` `knowledge_chat_tasks` `knowledge_operator_memory` `knowledge_chat_turns`| 4 个 collection 索引就位、有数据 |
| 7 | mongosh 查 `agent_events` | 出现 `kind=knowledge_digest_generated` / `knowledge_chat_task_finished` / `knowledge_operator_memory_added` |
| 8 | mongosh 查 `llm_call_logs` | 出现 `prompt_key="knowledge.digest.compose"` / `.dispatch` / `.summarize_logs` |
| 9 | 拔网线模拟 LLM 不可用 | digest 整体 `status="failed"`；前端显示 `<LlmErrorBanner>` + 重试按钮；点重试 → 重新合成 |
| 10 | 临时 `KNOWLEDGE_DIGEST_ENABLED=false` 重启 | worker 不起 tick；前端 `digest_today` 404 → 显示「AI 没能生成今日日报」+ 重试 |

任一 ✗ → 停下，按 `docs/agent-policy.md` runbook §5 七步流程定位修复。

## Out of scope（节奏 1 明确不做；下一轮再考虑）

- 事件驱动 push（webhook 实时叫醒 chat）
- 移动端三栏适配
- chat 跨 session 全文搜索 / 分支历史 fork
- 把日报扩展到 user-ops / group / moments
- digest 国际化 / 多语言文案

## Rollback

每 phase 单独 commit，倒序 `git revert`：
1. revert phase 5（operator memory + tool-calling） → revert phase 4（task worker + SSE） → revert phase 3（前端三栏） → revert phase 2（compose）→ revert phase 1（骨架）。
2. 每 revert 一步跑 `cargo test --lib` + `cd frontend && npm run build`，确保下一 revert 之前 baseline 仍绿。
3. Mongo 新增的 4 个 collection 留着不动；旧版本不读不影响。索引同样保留。
4. 不需要 mongosh 手动操作。
