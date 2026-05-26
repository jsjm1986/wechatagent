# Knowledge Digest Workstation — Design

> 配套 `requirements.md` 的实施级设计。读 design 之前先读 requirements R1–R9 + 红线 R7。
> 上一轮已经落地的 KnowledgeChatPanel（`POST /operation-knowledge/chat*`、`knowledge_chat_turns`、`LlmUnavailableError`）作为本轮的 chat 主入口，**不重写**。
>
> **⚠️ Note (2026-05-25)**：本文档示例 JSON / 图中的 `blockReason="fact_risk"` /
> `gateKey="fact_risk_block"` 等字符串是历史稿写作时的写法。运行时已收敛为 3 闸
> （`hallucination` / `knowledge_grounding` / `run_budget`，详见 `src/agent/guards.rs`）；
> evolution layer 仍按 `gate_key` 字符串工作，与运行时 guard 解耦。新版 digest 实现
> 应读 `held_by_ai_policy / blocked_by_safety_guard` 等运行时实际状态。

## 1. Context

当前知识库频道有三大裂缝：
1. **节奏缺失**：AI 不会主动汇总——运营要"想到才看"。
2. **批量消费缺失**：上一轮 chat 是单 chunk 修复入口，没有跨 chunk / 跨日 / 跨数据源的工作面板。
3. **长任务缺失**：chat 只能一来一回，跑 10 条 issue 时被流淹没；运营需要 fire-and-forget。

本设计在不重写 chat 的前提下加三件东西：
- **`KnowledgeDigestWorker`**：每天 09:00 跑一次，吃 4 个数据源吐一份当日 `knowledge_daily_reports`（卡片数组）。
- **三栏布局**：左目录树 25% / 中画布 45%（渲染当日卡片，可勾选派工） / 右 chat 30%（常驻，不再是抽屉）。
- **`KnowledgeTaskWorker`**：把"chat 派 N 条单"落库为 `knowledge_chat_tasks`，串行处理、逐条把进度回写 chat。

加一件不显眼但关键的东西：
- **`knowledge_operator_memory`**：与 chunk / contact / agent 三套 memoryCard **物理隔离**，专门记运营偏好。

## 2. Critical files

只改/新增 11 个文件（节奏 1 第一阶段共两次 commit）：

| 路径 | 改动 | 说明 |
|---|---|---|
| `src/config.rs` | 新增 5 项配置 | `KNOWLEDGE_DIGEST_ENABLED`(默认 false) / `KNOWLEDGE_DIGEST_RUN_HOUR`(默认 9) / `KNOWLEDGE_DIGEST_RUN_TOKEN_BUDGET`(默认 24000) / `KNOWLEDGE_DIGEST_RUN_MAX_LLM_CALLS`(默认 8) / `KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS`(默认 30) |
| `src/models.rs` | 新增 4 struct | `KnowledgeDigestCard` / `KnowledgeDailyReport` / `KnowledgeChatTask` / `KnowledgeOperatorMemory`；扩展现有 `KnowledgeChatTurn` 增 `kind` 与 `tool_calls` 可选字段（向后兼容） |
| `src/db/mod.rs` | 新增 4 collection accessor | `knowledge_daily_reports()` / `knowledge_chat_tasks()` / `knowledge_operator_memory()`（已有 `knowledge_chat_turns()` 不动） |
| `src/db/indexes.rs` | 新增 3 index | 见 R8 |
| `src/knowledge_digest/mod.rs` | 新建模块 | `worker_loop`（cron 调度）+ `generate_today_digest`（合成入口）+ `analyze_*`（4 个数据源扫描器）|
| `src/knowledge_task/mod.rs` | 新建模块 | `worker_loop`（每 30s 取 pending）+ `dispatch_task`（按 card.kind 调对应 chat sub-prompt 并写 turn 进度） |
| `src/prompts.rs` | 新增 3 PromptSpec | `knowledge.digest.compose`（吃 4 数据源摘要 → 卡片数组）/ `knowledge.digest.dispatch`（运营选 N 卡 → 拆 plannedSteps）/ `knowledge.digest.summarize_logs`（24h block/hold log → 一句话 issue） |
| `src/routes/knowledge.rs` | 新增 7 handler | 见 R9 |
| `src/routes/mod.rs` | mount 新路由 | `/api/knowledge/digest/*` 与 `/api/knowledge/chat/tasks/*` |
| `frontend/src/App.tsx` | 重排 Knowledge 频道为三栏 + KnowledgeDigestCanvas 组件 + 把 chat 抽屉换为常驻面板 + 卡片选中派工 + SSE 接收 task 进度 | 见 §4 布局 |
| `frontend/src/styles.css` | 追加 `/* === knowledge digest workstation === */` 段 | 三栏栅格 / 卡片 chip / 派工按钮；卡片复用 `.cardSurface`，气泡复用 `.bubbleRow / .bubble` |

只读引用（不改）：
- `src/agent/budget.rs:71-188` `RunBudget::new` + `RUN_BUDGET.scope` + `current_run_budget`
- `src/agent/mod.rs:141-149` `generate_agent_json`（唯一 LLM JSON 入口）
- `src/agent/tool_loop.rs` `reply_with_tools_loop`（tool-calling 注入模式蓝本）
- `src/routes/knowledge.rs` 现有 `chat_turn` / `chat_apply` / `chat_history` / `chat_discard`（本轮 chat 派工复用）
- `src/llm.rs` `classify_llm_error_for_user`（失败 → `AppError::LlmUnavailable` 已就位）

## 3. 数据流（一次完整闭环）

```
[09:00 每日 cron tick]
    │
    ▼
KnowledgeDigestWorker::generate_today_digest(account_id)
    │  RUN_BUDGET.scope(token=24000, calls=8, _ /*no rewrite gate*/) {
    │    1. analyze_chunks_health(db) ─┐
    │    2. analyze_usage_logs(db, 24h) ┤
    │    3. analyze_run_logs(db, 24h) ──┤── Vec<RawSignal>
    │    4. analyze_evolution(db, 24h) ─┘
    │    5. compose_cards(signals)  → call LLM `knowledge.digest.compose`
    │       ├─ LLM 输出 schema: [{kind, title, summary, targetRefs, suggestedAction, severity, metric}]
    │       ├─ 后端校验 + sort（R2.5）
    │       └─ 失败：partial 报告 + status="failed" / "partial"
    │    6. write knowledge_daily_reports{accountId, reportDate, cards, ...}
    │    7. write KnowledgeUsageLog{kind="digest_compose"}
    │    8. write AgentEvent{kind="knowledge_digest_generated", summary="今日 N 条 issue"}
    │  }
    │  ── 失败/超预算走 LlmUnavailable 路径，写一条 status=failed 的报告
    │
    ▼
[运营 09:30 打开 Knowledge 频道]
    │
    ▼
GET /api/knowledge/digest/today
    ├─ 命中 → 直接渲染
    └─ 未命中 → 同步触发一次 generate_today_digest（同入口、同 budget）
    │
    ▼
[画布渲染 N 张卡片；运营勾选 3 张点「💬 让 AI 处理选中的 3 条」]
    │
    ▼  前端把 selected cards 序列化成 chat 系统 turn 写入输入区
    │  运营可微调话术（如"先做 chunk 缺字段那条；命中率低的先放着"）
    │  → 发送
    │
    ▼
POST /api/operation-knowledge/chat
   intent → 命中新分支 "digest_action"
   sub-prompt: knowledge.digest.dispatch
   返回: { plannedSteps: [...], naturalReply: "我会按以下步骤处理 3 条 issue..." }
    │
    ▼  前端拿到 plannedSteps 后弹「派工确认」小卡，运营点确认
    │
    ▼
POST /api/knowledge/chat/tasks  body={sessionId, cardIds, plannedSteps}
    │
    ▼  服务端：
    │  1. 写 knowledge_chat_tasks{status="pending", plannedSteps, completedSteps:[]}
    │  2. 写 knowledge_chat_turns{role="system", kind="task_progress", content="任务已派出 (taskId=...)"}
    │  3. 立刻返回 taskId
    │
    ▼
[KnowledgeTaskWorker 每 30s 取 pending → status="running"]
    │
    ▼  for step in plannedSteps:
    │    RUN_BUDGET.scope(per-step token=8000, calls=4, _) {
    │      match step.action {
    │        fix_chunk(chunkId)  → 调现有 chunk repair propose + apply (强制 draft + needs_review)
    │        add_chunk(seed)     → 调现有 operation_knowledge_chunk_from_request（强制 draft + needs_review）
    │        retag(chunkId)      → 调现有 /chunks/:id/extract-tags
    │        review_evolution    → 不改 evolution，写 turn 让运营自己去 EvolutionCenterTab 决定
    │        dismiss             → 把 cardId 写 dismissedCardIds
    │      }
    │      // 每完成一条 → 写 turn{kind="task_progress", content="第 N/M 条已完成 ..."}
    │      // → 写 completedSteps
    │      // 失败 → 写 turn{kind="task_progress", content="第 N 条失败：..."} 并继续下一条（fail-soft）
    │    }
    │
    ▼  全部跑完 → status="finished" + 写 turn{kind="task_summary"} 列出 5 条 needs_review chunkId
    │
    ▼
[前端 chat 通过 SSE GET /api/knowledge/chat/sessions/:sid/stream 实时收到 progress turn]
    │
    ▼
[运营在右侧 chat 看到逐条进度；点 task_summary 里的 chunkId 跳转到目录树定位 + KnowledgeChunkEditor]
    │
    ▼  运营在 Editor 二次审核 → 现有 #329 sourceQuote → anchor gate → verify
    │
    ▼
[chunk 进入 verified 池，下次 inbound 检索可用]
```

**关键不变量**：
- AI 永不写 verified（与现有 chat apply 一致）；
- 任何 LLM 错误统一走 `AppError::LlmUnavailable` → 前端 `LlmErrorBanner`（不允许第二套样式）；
- worker tick / chat turn / task step 三层都被 `RUN_BUDGET` 卡死；
- 失败的 task step **不**阻塞后续 step（fail-soft），整个 task 总是能给一个 summary。

## 4. 三栏布局（Knowledge 频道）

```
┌─ Knowledge 频道（取代上一轮的抽屉式 chat）─────────────────────────────────────┐
│                                                                                │
│  ┌─ 25% 索引（左）─┐  ┌─ 45% 画布（中）────────────┐  ┌─ 30% Chat（右）─────┐│
│  │  📚 知识库目录   │  │  📅 2026-05-24 日报         │  │  💬 与 AI 对话补完  ││
│  │  ▸ 销售口径      │  │  generated 09:00 · 18 条 issue│  │  session: ...       ││
│  │    • chunk-003   │  │  [🔄 重算今日]              │  │                     ││
│  │  ▸ 反对话术      │  │                              │  │  [消息流]           ││
│  │  ▸ 产品事实      │  │  □ critical  chunk_caused   │  │                     ││
│  │  ...             │  │     _block (5)               │  │                     ││
│  │                  │  │     summary：2 个 chunk      │  │                     ││
│  │  [📄 手动新建]   │  │     在 24h 内被引用 5 次但   │  │                     ││
│  │                  │  │     最终发送被 fact_risk     │  │                     ││
│  │                  │  │     拦截 → [让AI 单独处理]   │  │                     ││
│  │                  │  │  □ warn  chunk_missing_field │  │                     ││
│  │                  │  │     (12)                     │  │                     ││
│  │                  │  │     ...                      │  │                     ││
│  │                  │  │  □ info  pack_outdated (1)   │  │                     ││
│  │                  │  │  ───────────────────────────  │  │                     ││
│  │                  │  │  [💬 让 AI 处理选中的 N 条]   │  │  [输入框]           ││
│  │                  │  │  [全选] [反选] [一键忽略 info]│  │                     ││
│  └─────────────────┘  └──────────────────────────────┘  └─────────────────────┘│
│                                                                                │
└────────────────────────────────────────────────────────────────────────────────┘
```

CSS 栅格（简化）：

```css
.knowledgeWorkstation {
  display: grid;
  grid-template-columns: minmax(220px, 25%) minmax(360px, 1fr) minmax(320px, 30%);
  gap: 12px;
  padding: 12px;
  height: calc(100vh - var(--header-h));
}
.knowledgeWorkstation > * {
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
  overflow: auto;
}
```

卡片复用 `.cardSurface`：

```tsx
<div className="cardSurface knowledgeDigestCard">
  <div className="knowledgeDigestCard__head">
    <input type="checkbox" />
    <span className={`severityChip severityChip--${card.severity}`}>{card.severity}</span>
    <span className="knowledgeDigestCard__title">{card.title}</span>
  </div>
  <p className="knowledgeDigestCard__summary">{card.summary}</p>
  {card.metric && <span className="metricChip">{card.metric.name}: {card.metric.value}/{card.metric.threshold}</span>}
  <div className="knowledgeDigestCard__actions">
    <button>让 AI 单独处理</button>
    <button>忽略</button>
    <button>打开</button>
  </div>
</div>
```

不新增第三套气泡；chat 区直接复用 `.bubbleRow / .bubble`，task 进度的 system turn 给一个 `.bubble--system` 灰底变体即可（用现有色 `--surface-quiet`）。

## 5. 后端 PromptSpec

### `knowledge.digest.compose`

**输入**（system + user）：
- system：约 800 字 —— 角色 = 知识库巡检助手；输出 schema；红线（不要捏造数字、不要写运营建议、不主动 verify）；不输出"接管/人工"等禁词。
- user：4 个数据源的摘要 JSON，每条限长，例如：
  ```json
  {
    "asOf": "2026-05-24T09:00:00+08:00",
    "windowHours": 24,
    "chunkHealth": [{"chunkId":"...","missing":["sourceQuote"],"draftAgeDays":7}],
    "usageLogs": {"totalQueries":312,"hitRate":0.68,"topMissedQueries":[...]},
    "runLogs": [{"runId":"...","blockReason":"fact_risk","triggeredChunkIds":["..."]}],
    "evolution": [{"proposalId":"...","gateKey":"fact_risk_block","status":"eligible_for_release"}]
  }
  ```

**输出 schema**：
```json
{
  "cards": [{
    "kind": "chunk_caused_block",
    "title": "...",
    "summary": "...",
    "targetRefs": [{"kind":"chunk","id":"..."}],
    "suggestedAction": "fix_chunk",
    "severity": "critical",
    "metric": {"name":"blocked_runs_24h","value":5,"threshold":1}
  }]
}
```

后端 `compose_cards` 拿到后做：
1. JSON schema 校验（缺字段 → reject）；
2. `targetRefs` 的 `id` 反查存在性（外键校验，不存在的 ref 整张卡片丢弃）；
3. R2.5 排序；
4. 截断 `cards.len() > 50` → 留前 50 + 写 partial。

### `knowledge.digest.dispatch`

吃运营选中的 cards + 对话上下文，输出 `plannedSteps`：
```json
{
  "naturalReply": "我会按以下步骤处理这 3 条问题…",
  "plannedSteps": [
    {"cardId":"...","action":"fix_chunk","targetChunkId":"...","hint":"..."}
  ],
  "estimatedLlmCalls": 7
}
```

`estimatedLlmCalls > 6` 时前端把这次派工降级为「分两批」（避免单 task 一次撑爆 budget）。

### `knowledge.digest.summarize_logs`

只在 `analyze_run_logs` 内部调用。把 24h block/hold runs 按 `triggeredChunkIds` group → 给 LLM 让它产 1 句话 issue summary（不让 worker 直接拼字符串，因为运营读起来像日志）。

3 条 PromptSpec 通过 `ensure_prompt_pack_v2` 末尾追加 seed；版本号挂在 `knowledge_daily_reports.promptVersions`。

## 6. 路由签名

```rust
// src/routes/knowledge.rs

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DigestTodayResponse {
    report: KnowledgeDailyReportView,
    generated_just_now: bool, // 命中缓存=false / 同步合成=true
}

async fn digest_today(State(s): State<AppState>) -> AppResult<Json<DigestTodayResponse>>;

#[derive(Deserialize)]
struct DigestRegenerateRequest { force: Option<bool> }

async fn digest_regenerate(
    State(s): State<AppState>,
    Json(b): Json<DigestRegenerateRequest>,
) -> AppResult<Json<DigestTodayResponse>>;

async fn digest_dismiss_card(
    State(s): State<AppState>,
    Path(card_id): Path<String>,
) -> AppResult<Json<Value>>;

#[derive(Deserialize)]
struct ChatTaskCreateRequest {
    session_id: String,
    card_ids: Vec<String>,
    planned_steps: Vec<PlannedStep>,
}

async fn chat_task_create(
    State(s): State<AppState>,
    Json(b): Json<ChatTaskCreateRequest>,
) -> AppResult<Json<KnowledgeChatTaskView>>;

async fn chat_task_get(
    State(s): State<AppState>,
    Path(task_id): Path<String>,
) -> AppResult<Json<KnowledgeChatTaskView>>;

async fn chat_task_cancel(
    State(s): State<AppState>,
    Path(task_id): Path<String>,
) -> AppResult<Json<Value>>;

// SSE：基于 tokio::sync::watch 推 turn id；前端拿到 id 后 GET history 拉新 turn
async fn chat_session_stream(
    State(s): State<AppState>,
    Path(session_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>>;
```

**每个 handler 都必须**：
1. 加 `RunBudget` scope；
2. 失败统一走 `AppError::LlmUnavailable`（已有分类）；
3. 写 `KnowledgeUsageLog` + `AgentEvent`；
4. 不允许向上游返回 raw reqwest 错误；
5. 不允许动 `prompt_templates` / `threshold_overrides`（演化器红线）。

## 7. 边界与失败模式

| 场景 | 处理 |
|---|---|
| worker 跑到一半崩了 | 已写 partial 报告保留 + status="failed" + errorKind；前端画布显示「AI 没能生成今日日报」+「重试」 |
| 运营手动重算 | 同入口 `generate_today_digest`，按 (accountId, reportDate) upsert；不会出现 2 份当日报告 |
| LLM compose 输出非法 JSON | `repair_loose_json` 兜一次；仍失败 → 整张报告 status="failed"，错误进 `LlmErrorBanner` |
| 卡片 targetRefs 引用了已删除 chunk | compose 后做存在性校验，整张卡片丢弃，写 KnowledgeUsageLog 提醒 |
| 派工 plannedSteps 估算 > 6 LLM call | 前端弹「分批确认」对话框，分两批落 task |
| task worker 取不到 pending | 正常，下一轮 30s tick 再取 |
| 同一 sessionId 多 task 并发 | worker 按 sessionId 串行（同 sessionId pending 任意时刻只 1 跑） |
| task 中途失败 | fail-soft，写 progress turn 标该步失败，继续后续 step；最后 summary 列出失败步 |
| 运营 cancel 已 running 的 task | 设 status="cancelled"；worker 在每步开始前检查 status，非 "running" 即停下 |
| operator 在 chat 说"以后别再起带 100% 回奶" | intent=update_operator_memory → 写 `knowledge_operator_memory{kind="rejection"}` → assistant turn 显式确认（R5.4） |
| SSE 断开 | 前端 reconnect with last_turn_seq；服务端按 seq 回放未送达的 turn |
| LlmUnavailable 错误 | 不写 partial 卡片，整体 status="failed" + errorKind；前端复用上一轮 `LlmErrorBanner` |
| 演化器同时也在修同一个 chunk | 不可能 —— 演化器只动 `prompt_templates` / `threshold_overrides`，不动 `operation_knowledge_chunks`（隔离已 CI 守门） |

## 8. 文案防御（lint）

新增字符串预先自检禁词（`bash scripts/check-no-human-takeover.sh`）。统一用语：

| 位置 | 文案 |
|---|---|
| 顶部按钮 | 💬 让 AI 处理选中的 N 条 / 🔄 重算今日 / 📄 手动新建 |
| 卡片动作 | 让 AI 单独处理 / 忽略 / 打开 |
| chat 系统消息 | "AI 已起草第 N/M 条草稿，chunkId=..." / "本次共起草 X 条、修订 Y 条；请去 chunk 编辑器审核" |
| 错误 banner | "AI 没能生成今日日报，已多次重试。" / "AI 派工被预算限制中断，请稍后再试。" |
| operator memory 确认 | "我已记下这条偏好，下次起草草稿会避开。" |
| **不写** | 人工接管 / 人工修改 / 人工补完 / takeover / hand-off / 人工审核（→ 改用「请去 chunk 编辑器审核」）|

每段新文案 commit 前 `bash scripts/check-no-human-takeover.sh` 扫一遍。
