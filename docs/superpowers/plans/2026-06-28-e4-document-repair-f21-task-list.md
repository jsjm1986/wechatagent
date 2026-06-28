# E4 文档级批量修复 + F21 任务总览列表 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把下线的 pack repair 重建为「文档级批量修复」（视角聚合复用 PR#49 chunk 闭环），并为知识长任务补「任务总览列表」端点 + 前端列表化。

**Architecture:** E4 后端零改动——前端新增 `DocumentRepairPanel` 调用现成 `GET /operation-knowledge/documents/:id/chunks`、筛 `needs_review`、逐 chunk 内嵌复用现有 `ChunkRepairPanel`。F21 后端新增 `chat_task_list`（`GET /knowledge/chat/tasks`）+ 前端 `TaskRail` 列表化（保留手工输入 fallback）。

**Tech Stack:** 后端 Rust/Axum + MongoDB（`mongodb::bson`）；前端 React 19 + TypeScript + Vite + plain `.css`（knowledge 频道避 tree-shake）；vitest + @testing-library/react；后端测试用无 Docker 依赖的纯逻辑单测（本地 `cargo test --lib` / `--test`，集成测试留 CI）。

## Global Constraints

> 每个 task 隐含包含本节，逐条 verbatim。

- 子 agent 一律 `model:"opus"`；回复中文。
- **AI 永不自动 verify 红线**：本计划不引入任何 worker 自动落库；E4 逐 chunk 落库复用 `applyAiRepairPatch`，其 `thenVerify` 恒 `false`（落 `status=draft + integrity_status=needs_review`）。
- **无人工接管红线**：新增前后端代码新行禁含 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`（`scripts/check-no-human-takeover.{sh,ps1}` CI 门，扫 `src/`+`frontend/src/` 新增行，tests 目录排除）。用 AI 内部状态名。
- **测试基线门**：`cargo test --lib` ≥350/0；4 PBT（state_transition_pbt / memory_card_invariants / wiki_chunk_revision_pbt / llm_retry_jitter）累计 ≥33/0。新工作只增不减。
- **前端设计系统**：真实 token 在 `frontend/src/components/ui/tokens.css`；knowledge 频道用 plain `.css`（`Knowledge.css`，import 在 `index.tsx`）；4 级层级 / 蓝仅主操作 / 紫（`--fill-brand` #5e5ce6）仅 AI 身份。
- **磁盘纪律**：本地只跑 `cargo test --lib` + 单个 `--test <name>`；不跑全量 `--ignored` 集成（留 CI）。需要时先删 `target/debug/incremental`。
- **提交纪律**：只 `git add` 指定文件；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`；**所有 commit 仅在用户明确批准时创建**。
- **wire 契约**：serde camelCase。后端 list 投影逐字段 camelCase，对齐前端 interface。
- **不碰**：PR#49 已闭环的 chunk 修复代码（`ChunkRepairPanel.tsx`/`applyAiRepairPatch.ts`/`repair.rs`，仅复用）；不复活 `operation_knowledge_items`；不改 worker `execute_step` 占位桩。

---

## File Structure

- **后端**：
  - `src/routes/knowledge/chat.rs` — 新增 `ChatTaskListQuery` 结构 + `chat_task_list` handler + lib 单测（投影/limit clamp 纯逻辑）。
  - `src/routes/mod.rs` — `/knowledge/chat/tasks` 路由加 `.get(chat_task_list)`。
- **前端**：
  - `frontend/src/features/knowledge/DocumentRepairPanel.tsx` — 新建。文档级批量修复聚合视图。
  - `frontend/src/features/knowledge/today.tsx` — `TaskRail` 列表化。
  - `frontend/src/features/knowledge/Knowledge.css` — 追加 DocumentRepairPanel + TaskRail 列表样式。
  - `frontend/src/features/knowledge/steward.tsx`（或 document 列表落点）— 挂 DocumentRepairPanel 入口。
  - `frontend/src/__tests__/features/knowledge/DocumentRepairPanel.test.tsx` — 新建。
  - `frontend/src/__tests__/features/knowledge/TaskRailList.test.tsx` — 新建。

---

## Task 1: 后端 `chat_task_list` 端点（GET /knowledge/chat/tasks）

**Files:**
- Modify: `src/routes/knowledge/chat.rs`（在 `chat_task_get`（约 :2014）之前/之后加 `ChatTaskListQuery` + `chat_task_list`；并在文件内 `#[cfg(test)] mod tests` 加纯逻辑单测——若该文件无 tests mod 则新建）
- Modify: `src/routes/mod.rs:673`（`.route("/knowledge/chat/tasks", post(chat_task_create))` → 加 `.get(chat_task_list)`；import 列表 `:202` 加 `chat_task_list`）

**Interfaces:**
- Consumes: `state.db.knowledge_chat_tasks()`（`Collection<KnowledgeChatTask>`）；`admin.current_workspace`；`KnowledgeChatTask` 字段（models.rs:4576-4605：id/session_id/status/planned_steps/completed_steps/created_at/started_at/finished_at/error_kind）；`ALLOWED_TASK_STATUS`（models.rs:4571）。
- Produces: `GET /knowledge/chat/tasks?status=<opt>&limit=<opt>` → `{ "items": [{ taskId, sessionId, status, errorKind, totalSteps, completedStepCount, createdAt, startedAt, finishedAt }] }`，按 `created_at` 倒序，limit clamp 1-200 默认 50。纯函数 `clamp_task_list_limit(Option<i64>) -> i64`。

- [ ] **Step 1: 写失败的纯逻辑单测（limit clamp）**

在 `src/routes/knowledge/chat.rs` 的 `#[cfg(test)] mod tests`（不存在则在文件末尾新建 `#[cfg(test)] mod tests { use super::*; ... }`）加：

```rust
#[test]
fn clamp_task_list_limit_defaults_and_bounds() {
    assert_eq!(clamp_task_list_limit(None), 50, "缺省 50");
    assert_eq!(clamp_task_list_limit(Some(0)), 1, "下界 clamp 到 1");
    assert_eq!(clamp_task_list_limit(Some(-5)), 1, "负数 clamp 到 1");
    assert_eq!(clamp_task_list_limit(Some(10)), 10, "区间内原值");
    assert_eq!(clamp_task_list_limit(Some(9999)), 200, "上界 clamp 到 200");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib clamp_task_list_limit`
Expected: 编译失败 `cannot find function clamp_task_list_limit`。

- [ ] **Step 3: 写 `clamp_task_list_limit` 纯函数 + `ChatTaskListQuery` 结构**

在 `chat.rs`（`chat_task_get` handler 之前）加：

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::routes) struct ChatTaskListQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
}

/// 任务列表 limit clamp：缺省 50，区间 [1, 200]。
fn clamp_task_list_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 200)
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib clamp_task_list_limit`
Expected: PASS（1 passed）。

- [ ] **Step 5: 写 `chat_task_list` handler**

在 `chat.rs` 紧接上面加（投影逐字段 camelCase，列表不带 plannedSteps/cards 全文以控体积，只给 completedStepCount 计数）：

```rust
/// `GET /api/knowledge/chat/tasks`：列出本 workspace 的长任务（F21 任务总览）。
/// 可选 status 过滤（非法值忽略，与现有 chunk 列表 query 宽松风格一致）；
/// limit clamp [1,200] 默认 50；按 created_at 倒序。列表项不带 plannedSteps/cards
/// 全文（控 payload 体积），详情仍走 GET /tasks/:id。
pub(in crate::routes) async fn chat_task_list(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ChatTaskListQuery>,
) -> AppResult<Json<Value>> {
    let mut filter = doc! { "workspace_id": &admin.current_workspace };
    if let Some(status) = query.status.as_ref().filter(|s| !s.trim().is_empty()) {
        filter.insert("status", status.trim());
    }
    let limit = clamp_task_list_limit(query.limit);
    let mut cursor = state
        .db
        .knowledge_chat_tasks()
        .find(
            filter,
            mongodb::options::FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .limit(limit)
                .build(),
        )
        .await?;
    let mut items = Vec::new();
    while let Some(task) = cursor.try_next().await? {
        items.push(json!({
            "taskId": task.id.map(|i| i.to_hex()).unwrap_or_default(),
            "sessionId": task.session_id,
            "status": task.status,
            "errorKind": task.error_kind,
            "totalSteps": task.planned_steps.len() as i32,
            "completedStepCount": task.completed_steps.len() as i32,
            "createdAt": task.created_at.to_string(),
            "startedAt": task.started_at.map(|d| d.to_string()),
            "finishedAt": task.finished_at.map(|d| d.to_string()),
        }));
    }
    Ok(Json(json!({ "items": items })))
}
```

注意：确认 `chat.rs` 顶部已 `use futures::TryStreamExt;`（`try_next` 所需）与 `use serde::Deserialize;`、`use axum::extract::Query;`。若缺则在 import 区补（`chat_task_get` 用了 `Path`/`State`/`Extension`，`load_operation_knowledge_chunks_for_query` 用了 `TryStreamExt`，多半已在 crate 内）。

- [ ] **Step 6: 注册路由 + import**

`src/routes/mod.rs:202` 的 knowledge import 行，把 `chat_task_cancel, chat_task_create, chat_task_get,` 改为含 `chat_task_list`：

```rust
    chat_history, chat_session_stream, chat_task_cancel, chat_task_create, chat_task_get,
    chat_task_list,
```

`src/routes/mod.rs:673`：

```rust
        .route(
            "/knowledge/chat/tasks",
            get(chat_task_list).post(chat_task_create),
        )
```

（注意 `get` 已在本文件 import；原 `post(chat_task_create)` 单独一行改为 `.route(... get().post())` 形态。）

- [ ] **Step 7: 编译 + lib 测试全绿**

Run: `cargo test --lib`
Expected: ≥350 passed, 0 failed（含新增 clamp 单测）。
Run: `RUSTFLAGS=-Dwarnings cargo check --tests`
Expected: 0 error 0 warning（CI baseline step2 复刻）。

- [ ] **Step 8: 提交（需用户批准）**

```bash
git add src/routes/knowledge/chat.rs src/routes/mod.rs
git commit -m "feat(knowledge): F21 新增 chat_task_list 端点(GET /knowledge/chat/tasks)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: 前端 TaskRail 列表化

**Files:**
- Modify: `frontend/src/features/knowledge/today.tsx`（`TaskRail` 约 :785-952）
- Modify: `frontend/src/features/knowledge/Knowledge.css`（追加列表项 class）
- Test: `frontend/src/__tests__/features/knowledge/TaskRailList.test.tsx`（新建）

**Interfaces:**
- Consumes: `GET /api/knowledge/chat/tasks`（Task 1 产出）→ `{ items: ChatTaskListItem[] }`，`ChatTaskListItem = { taskId, sessionId, status, errorKind?, totalSteps, completedStepCount, createdAt?, startedAt?, finishedAt? }`；现有 `loadTask(taskId)`、`taskStatusLabel`、`EmptyState`、`parseApiError`。
- Produces: TaskRail 渲染任务列表，点选项触发现有 `loadTask`；保留手工输入 fallback。

- [ ] **Step 1: 写失败的组件测试**

新建 `frontend/src/__tests__/features/knowledge/TaskRailList.test.tsx`（参照 `knowledge.test.tsx` 的 fetchMock 风格）：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TaskRail } from "../../../features/knowledge/today";

function mockFetch(handler: (url: string, init?: RequestInit) => unknown) {
  globalThis.fetch = vi.fn(async (url: unknown, init?: RequestInit) => {
    const body = handler(String(url), init);
    return {
      ok: true,
      status: 200,
      async json() { return body; },
      async text() { return JSON.stringify(body); },
    } as unknown as Response;
  }) as typeof fetch;
}

describe("TaskRail 任务总览列表", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("挂载时拉取任务列表并渲染列表项", async () => {
    mockFetch((url) => {
      if (url.includes("/knowledge/chat/tasks/")) {
        return { taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedSteps: [1], cards: [] };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return { items: [
          { taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedStepCount: 1, createdAt: "2026-06-28" },
          { taskId: "T2", sessionId: "S2", status: "completed", totalSteps: 2, completedStepCount: 2, createdAt: "2026-06-27" },
        ] };
      }
      return {};
    });
    render(<TaskRail />);
    await waitFor(() => expect(screen.getByText("S1")).toBeInTheDocument());
    expect(screen.getByText("S2")).toBeInTheDocument();
  });

  it("点选列表项触发 loadTask 拉详情", async () => {
    const user = userEvent.setup();
    const calls: string[] = [];
    mockFetch((url) => {
      calls.push(url);
      if (url.includes("/knowledge/chat/tasks/")) {
        return { taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedSteps: [1], cards: [] };
      }
      if (url.includes("/knowledge/chat/tasks")) {
        return { items: [{ taskId: "T1", sessionId: "S1", status: "running", totalSteps: 3, completedStepCount: 1, createdAt: "2026-06-28" }] };
      }
      return {};
    });
    render(<TaskRail />);
    await waitFor(() => expect(screen.getByText("S1")).toBeInTheDocument());
    await user.click(screen.getByText("S1"));
    await waitFor(() =>
      expect(calls.some((u) => u.includes("/knowledge/chat/tasks/T1"))).toBe(true)
    );
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/TaskRailList.test.tsx`
Expected: FAIL（列表未渲染，`getByText("S1")` 找不到——当前 TaskRail 无列表）。

- [ ] **Step 3: TaskRail 加列表 state + 拉取**

`today.tsx` 的 `TaskRail`，在现有 `const [sessionId, setSessionId] = useState("")` 等 state 后加列表 state，并加挂载拉取（保留所有现有逻辑：手工 input、wikiTrackTask、loadTask、cancelTask、SSE）：

```tsx
  const [taskList, setTaskList] = useState<ChatTaskListItem[]>([]);

  async function loadTaskList() {
    try {
      const r = await fetch("/api/knowledge/chat/tasks");
      if (!r.ok) return; // 列表失败不阻塞手工跟踪，静默降级
      const data = (await r.json()) as { items?: ChatTaskListItem[] };
      setTaskList(data.items ?? []);
    } catch {
      /* 列表拉取失败：保留手工输入 fallback，不弹错 */
    }
  }

  useEffect(() => { void loadTaskList(); }, []);
```

在 `today.tsx` 的 `ChatTaskView` interface 附近加列表项类型：

```tsx
interface ChatTaskListItem {
  taskId: string;
  sessionId: string;
  status: string;
  errorKind?: string | null;
  totalSteps: number;
  completedStepCount: number;
  createdAt?: string;
  startedAt?: string | null;
  finishedAt?: string | null;
}
```

- [ ] **Step 4: 渲染列表 UI（插在手工 form 之后、task 详情之前）**

在 `TaskRail` 的 `return` 里，`<div className="wikiTaskRailForm">...</div>` 之后插入：

```tsx
      {taskList.length > 0 ? (
        <ul className="wikiTaskRailList">
          {taskList.map((t) => (
            <li key={t.taskId}>
              <button
                type="button"
                className={`wikiTaskRailListItem${task?.taskId === t.taskId ? " active" : ""}`}
                onClick={() => { setSessionId(t.taskId); void loadTask(t.taskId); }}
              >
                <span className={`wikiTaskStatus s-${t.status}`}>{taskStatusLabel(t.status)}</span>
                <span className="wikiTaskRailListSess">{t.sessionId}</span>
                <span className="wikiTaskRailListSteps">{t.completedStepCount}/{t.totalSteps}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
```

- [ ] **Step 5: 追加 CSS**

`Knowledge.css` 末尾追加（plain css，2 空格缩进，复用既有 token 变量，不引新颜色）：

```css
  .wikiTaskRailList {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 240px;
    overflow-y: auto;
  }
  .wikiTaskRailListItem {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
    background: var(--fill-surface);
    cursor: pointer;
    text-align: left;
    font-size: 12px;
  }
  .wikiTaskRailListItem.active {
    border-color: var(--border-strong);
    background: var(--fill-subtle);
  }
  .wikiTaskRailListSess {
    flex: 1;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wikiTaskRailListSteps {
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }
```

注意：用前先 grep `Knowledge.css` 确认 `--border-subtle`/`--fill-surface`/`--fill-subtle`/`--text-secondary`/`--text-tertiary`/`--border-strong` 真实存在；若命名不符，用文件内 TaskRail 既有 class（如 `.wikiTaskCard`）实际引用的变量替换。

- [ ] **Step 6: 运行测试确认通过 + tsc + 禁词**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/TaskRailList.test.tsx`
Expected: PASS（2 passed）。
Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。
Run: `bash scripts/check-no-human-takeover.sh`（或 `.ps1`）
Expected: 0 命中。

- [ ] **Step 7: 全前端回归**

Run: `cd frontend && npx vitest run`
Expected: 全绿（现有 311 + 新增，0 failed）。

- [ ] **Step 8: 提交（需用户批准）**

```bash
git add frontend/src/features/knowledge/today.tsx frontend/src/features/knowledge/Knowledge.css frontend/src/__tests__/features/knowledge/TaskRailList.test.tsx
git commit -m "feat(knowledge): F21 TaskRail 任务总览列表(GET list + 点选跟踪, 保留手工 fallback)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: 前端 DocumentRepairPanel（文档级批量修复）

**Files:**
- Create: `frontend/src/features/knowledge/DocumentRepairPanel.tsx`
- Modify: `frontend/src/features/knowledge/steward.tsx`（document 列表/详情项挂入口；落点按现有 document UI 结构定）
- Modify: `frontend/src/features/knowledge/Knowledge.css`（追加 DocumentRepairPanel class）
- Test: `frontend/src/__tests__/features/knowledge/DocumentRepairPanel.test.tsx`（新建）

**Interfaces:**
- Consumes: `GET /api/operation-knowledge/documents/:id/chunks` → `{ items: ChunkView[] }`，每个 ChunkView 含 `id`/`documentId`/`integrityStatus`/`title` 等（mod.rs:282 `operation_knowledge_chunk_json` 全字段）；现有组件 `ChunkRepairPanel`（props `{ chunkId: string; originalChunk: Record<string, unknown>; onApplied: () => void }`，from `./ChunkRepairPanel`）。
- Produces: `DocumentRepairPanel`（props `{ documentId: string; documentTitle?: string; onClose?: () => void }`），导出供 steward 挂载。

- [ ] **Step 1: 写失败的组件测试**

新建 `frontend/src/__tests__/features/knowledge/DocumentRepairPanel.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { DocumentRepairPanel } from "../../../features/knowledge/DocumentRepairPanel";

function mockChunks(items: unknown[]) {
  globalThis.fetch = vi.fn(async (url: unknown) => {
    const u = String(url);
    const body = u.includes("/documents/") && u.includes("/chunks") ? { items } : {};
    return { ok: true, status: 200, async json() { return body; }, async text() { return JSON.stringify(body); } } as unknown as Response;
  }) as typeof fetch;
}

describe("DocumentRepairPanel 文档级批量修复", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("只渲染 needs_review 的 chunk（过滤 verified/draft）", async () => {
    mockChunks([
      { id: "c1", documentId: "d1", title: "待修切片A", integrityStatus: "needs_review" },
      { id: "c2", documentId: "d1", title: "已核验切片B", integrityStatus: "verified" },
      { id: "c3", documentId: "d1", title: "待修切片C", integrityStatus: "needs_review" },
    ]);
    render(<DocumentRepairPanel documentId="d1" documentTitle="文档一" />);
    await waitFor(() => expect(screen.getByText("待修切片A")).toBeInTheDocument());
    expect(screen.getByText("待修切片C")).toBeInTheDocument();
    expect(screen.queryByText("已核验切片B")).not.toBeInTheDocument();
  });

  it("无 needs_review chunk 时显示空态，不崩", async () => {
    mockChunks([{ id: "c2", documentId: "d1", title: "B", integrityStatus: "verified" }]);
    render(<DocumentRepairPanel documentId="d1" />);
    await waitFor(() => expect(screen.getByText(/无待修切片/)).toBeInTheDocument());
  });

  it("加载失败显示错误态，不静默空", async () => {
    globalThis.fetch = vi.fn(async () => ({
      ok: false, status: 500, async json() { return { error: "boom" }; }, async text() { return "boom"; },
    } as unknown as Response)) as typeof fetch;
    render(<DocumentRepairPanel documentId="d1" />);
    await waitFor(() => expect(screen.getByText(/加载失败/)).toBeInTheDocument());
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/DocumentRepairPanel.test.tsx`
Expected: FAIL（`DocumentRepairPanel` 模块不存在 / import 报错）。

- [ ] **Step 3: 创建 DocumentRepairPanel 组件**

新建 `frontend/src/features/knowledge/DocumentRepairPanel.tsx`：

```tsx
import { useEffect, useState, useCallback } from "react";
import { ChunkRepairPanel } from "./ChunkRepairPanel";

interface ChunkView {
  id: string;
  documentId?: string | null;
  title?: string;
  integrityStatus?: string;
  [k: string]: unknown;
}

export function DocumentRepairPanel({
  documentId,
  documentTitle,
  onClose,
}: {
  documentId: string;
  documentTitle?: string;
  onClose?: () => void;
}) {
  const [chunks, setChunks] = useState<ChunkView[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [doneIds, setDoneIds] = useState<Set<string>>(new Set());

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await fetch(
        `/api/operation-knowledge/documents/${encodeURIComponent(documentId)}/chunks`,
      );
      if (!r.ok) throw new Error("加载失败");
      const data = (await r.json()) as { items?: ChunkView[] };
      setChunks(data.items ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, [documentId]);

  useEffect(() => { void reload(); }, [reload]);

  const needsReview = chunks.filter(
    (c) => c.integrityStatus === "needs_review" && !doneIds.has(c.id),
  );

  return (
    <section className="wikiDocRepair">
      <header className="wikiDocRepairHead">
        <h3>批量 AI 修复{documentTitle ? `：${documentTitle}` : ""}</h3>
        {onClose ? (
          <button type="button" className="wikiDocRepairClose" onClick={onClose}>关闭</button>
        ) : null}
      </header>
      {error ? (
        <div className="wikiAlert error">{error}</div>
      ) : loading ? (
        <div className="wikiDocRepairHint">加载待修切片…</div>
      ) : needsReview.length === 0 ? (
        <div className="wikiDocRepairHint">该文档无待修切片（needs_review）。</div>
      ) : (
        <ul className="wikiDocRepairList">
          {needsReview.map((chunk) => (
            <li className="wikiDocRepairItem" key={chunk.id}>
              <button
                type="button"
                className="wikiDocRepairItemHead"
                onClick={() => setExpandedId(expandedId === chunk.id ? null : chunk.id)}
              >
                <span className="wikiDocRepairItemTitle">{chunk.title || chunk.id}</span>
                <span className="wikiDocRepairItemTag">needs_review</span>
              </button>
              {expandedId === chunk.id ? (
                <ChunkRepairPanel
                  chunkId={chunk.id}
                  originalChunk={chunk as unknown as Record<string, unknown>}
                  onApplied={() => {
                    setDoneIds((prev) => new Set(prev).add(chunk.id));
                    setExpandedId(null);
                    window.dispatchEvent(
                      new CustomEvent("wikiChunkRevised", { detail: { chunk_id: chunk.id } }),
                    );
                  }}
                />
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/DocumentRepairPanel.test.tsx`
Expected: PASS（3 passed）。

- [ ] **Step 5: 追加 CSS**

`Knowledge.css` 末尾追加（紫 `--fill-brand` 仅用于"批量 AI 修复"标题/AI 身份；其余用中性 token；用前 grep 确认变量名存在，不存在则替换为文件内既有变量）：

```css
  .wikiDocRepair {
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
    padding: 12px;
    margin-top: 12px;
  }
  .wikiDocRepairHead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .wikiDocRepairHead h3 {
    margin: 0;
    font-size: 14px;
    color: var(--fill-brand);
  }
  .wikiDocRepairClose {
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    cursor: pointer;
    font-size: 12px;
  }
  .wikiDocRepairHint {
    color: var(--text-tertiary);
    font-size: 12px;
    padding: 8px 0;
  }
  .wikiDocRepairList {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .wikiDocRepairItem {
    border: 1px solid var(--border-subtle);
    border-radius: 6px;
    overflow: hidden;
  }
  .wikiDocRepairItemHead {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 10px;
    background: var(--fill-surface);
    border: none;
    cursor: pointer;
    text-align: left;
  }
  .wikiDocRepairItemTitle {
    font-size: 13px;
    color: var(--text-primary);
  }
  .wikiDocRepairItemTag {
    font-size: 11px;
    color: var(--text-tertiary);
  }
```

- [ ] **Step 6: 挂入口到 steward（document 落点）**

先读 `frontend/src/features/knowledge/steward.tsx`，找到 document 列表/详情项渲染处。在 document 项加一个"批量 AI 修复"按钮，点击 setState 打开 `DocumentRepairPanel`（传 `documentId={doc.id}` `documentTitle={doc.title}` `onClose`）。具体落点与 state 接法按 steward 现有结构定——核心要求：

1. import：`import { DocumentRepairPanel } from "./DocumentRepairPanel";`
2. 入口按钮文案不含禁词；遵守设计系统（次级按钮，非蓝色主操作）。
3. 仅在文档存在 needs_review chunk 时显示入口（若 steward 已有 chunk 计数则复用；无则始终显示，点开后由面板自身空态兜底）。

读 steward.tsx 后，按其 document 项的实际结构写最小改动 patch（此步无法预写精确代码——steward.tsx 结构需现场读，但改动限于：1 行 import + 1 个 state + 1 个按钮 + 1 处条件渲染 panel）。

- [ ] **Step 7: tsc + 禁词 + 全前端回归**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。
Run: `bash scripts/check-no-human-takeover.sh`
Expected: 0 命中。
Run: `cd frontend && npx vitest run`
Expected: 全绿（0 failed）。

- [ ] **Step 8: 提交（需用户批准）**

```bash
git add frontend/src/features/knowledge/DocumentRepairPanel.tsx frontend/src/features/knowledge/steward.tsx frontend/src/features/knowledge/Knowledge.css frontend/src/__tests__/features/knowledge/DocumentRepairPanel.test.tsx
git commit -m "feat(knowledge): E4 文档级批量修复(DocumentRepairPanel 聚合复用 ChunkRepairPanel)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: whole-branch 终审

**Files:** 无（审查 only）

- [ ] **Step 1: 派最强模型 subagent 做整分支终审**

审查范围（`git diff main..HEAD` 全量）：
1. **红线**：grep 确认无任何新增 worker 自动落库；`applyAiRepairPatch` 的 `thenVerify` 仍 false（未被 E4 复用路径绕过）；DocumentRepairPanel 落库全程走 ChunkRepairPanel 既有链路，无新 PUT/verify 旁路。
2. **禁词**：`git diff main..HEAD` 新增行跑 `check-no-human-takeover`，0 命中。
3. **baseline**：`cargo test --lib` ≥350/0；4 PBT ≥33/0；`RUSTFLAGS=-Dwarnings cargo check --tests` 0/0。
4. **复用正确性**：DocumentRepairPanel 传给 ChunkRepairPanel 的 props 类型/字段名与 ChunkRepairPanel 实际签名一致（chunkId/originalChunk/onApplied）；list 端点 wire 投影 camelCase 与前端 ChatTaskListItem interface 字段一一对应。
5. **F21 端点**：`chat_task_list` 的 workspace 过滤存在（不跨租户泄漏）；status 闭集外的值不报错（宽松忽略）；limit clamp 生效。
6. **设计系统**：新增 CSS 用真实 token；紫仅 AI 身份；plain .css 无 module 副作用导入。

- [ ] **Step 2: 修复终审发现的问题（如有），回到对应 Task 重跑其测试**

- [ ] **Step 3: 全量回归 + 报告**

Run: `cargo test --lib && cd frontend && npx vitest run && npx tsc --noEmit`
Expected: 后端 ≥350/0、前端全绿、tsc 0。
报告：列出每条红线 HOLD/BROKEN + baseline 数字 + 禁词扫描结果。

---

## Self-Review（计划自审记录）

- **Spec coverage**：E4 文档级批量修复 → Task 3；F21 list 端点 → Task 1，TaskRail 列表化 → Task 2；红线/不变量 → 各 Task 测试步 + Task 4 终审。spec "不做 worker 落库/批量大表/复活 pack" → 计划 Global Constraints + 未出现相关 task。全覆盖。
- **Placeholder scan**：Task 3 Step 6（steward 挂载）无法预写精确代码——已明确标注"读 steward.tsx 后按实际结构写最小改动"并界定改动范围（1 import + 1 state + 1 按钮 + 1 条件渲染），非占位符而是有界的现场适配。其余步骤均给完整代码。
- **Type consistency**：`ChatTaskListItem`（前端 Task 2）字段与 `chat_task_list` 投影（后端 Task 1）一一对应：taskId/sessionId/status/errorKind/totalSteps/completedStepCount/createdAt/startedAt/finishedAt。`ChunkRepairPanel` props（chunkId/originalChunk/onApplied）与 Task 3 调用一致。`clamp_task_list_limit` 命名前后一致。
