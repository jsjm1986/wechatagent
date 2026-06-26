# 前后端业务对齐修复 批次1（P0+P1）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复审查确认的 1 条 CRITICAL + 13 条 HIGH 前后端对齐缺口，让后端已有能力在前端真正可达。

**Architecture:** 大多数是纯前端缺口（写端点已存在、UI 无入口或路径错配）；3 条需先动后端（B2 请示卡富字段投影、E1 referral $unset 端点、B2/E1 的前端消费）。前端 React 19 + Zustand + CSS Modules，测试用 vitest + @testing-library/react；后端 Axum，测试用 cargo 集成测。每条修复独立可测、独立提交。

**Tech Stack:** 前端 TypeScript / React 19 / Zustand / vitest；后端 Rust 2021 / Axum / MongoDB。

**来源 spec:** `docs/superpowers/specs/2026-06-26-frontend-backend-alignment-fixes-design.md`（条目 A1/B1/B2/B3/C1/C2/C3/C4/C5/D1/D2/D3/E1/E14）。

## Global Constraints

每个 task 隐含遵守，违反即不可合并：

- **前端遵守现有设计系统**：真实 token 在 `frontend/src/components/ui/tokens.css`；CSS 用 `.module.css` + `import styles from "./x.module.css"` 绑定（禁止裸 `import "./x.css"`，会被 tree-shake 删光，见 memory `frontend_css_module_tree_shake_trap`）；4 级层级 / 蓝色仅主操作 / 紫色仅 AI 身份 / 字号纪律。写前端组件前先读 `docs/frontend-design-system.md` 并参照同目录现有 `.module.css`。
- **无人工接管红线**：新增前端/后端代码新行禁含 `人工接管 / 转人工 / takeover / hand-off / 接管 / 人工`（CI 门 `scripts/check-no-human-takeover.{sh,ps1}` 扫 `src/agent|routes|evolution` 与 `frontend/src` 新增行，测试目录豁免）。只用 AI 内部状态名（如「AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文」）。
- **测试基线门**（`scripts/check-baseline.{sh,ps1}`）：`cargo test --lib` ≥350 passed / 0 failed；4 个 PBT 文件累计 ≥33/0；`RUSTFLAGS=-Dwarnings cargo check --tests` 0 error 0 warning。前端 `npm run build` 通过 + `npm test`（vitest run）全绿。本地磁盘紧张：后端只跑 `cargo test --lib` 与单个 PBT，完整集成测交 CI。
- **AI 永不自动验证知识**：新增知识 status=draft + integrity_status=needs_review 红线不破（本批不新增知识写入，但 E14 派工不得绕过）。
- **提交纪律**：只 `git add` 本 task 列出的具体文件，绝不 `git add -A`；commit message 末尾加 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **后端 blast radius**：B2 扩 InboxItem 投影、E1 加 $unset 端点须保 InboxItem 现有 8 处构造点 + escalation 退辅助逻辑不回归。
- **路由前缀**：后端 API 全挂 `/api` 下（中间件已剥前缀）。`outbox`/`principal-escalations`/`ask-human` 带 `/admin`；`evolution`/`evaluation-scenarios`/`accounts`/`conversations`/`decision-reviews` **无** `/admin`。前端 `api.get`/`api.post` 传完整含 `/api` 的路径。

## 文件结构（本批触及）

**纯前端修改：**
- `frontend/src/stores/userOpsStore.ts` — A1 修双死端点路径 + allSettled 加固
- `frontend/src/stores/operationsStore.ts` — C1 catch 接 setError
- `frontend/src/features/operations/index.tsx` — C5 跟进任务操作按钮
- `frontend/src/features/autonomy/index.tsx` + 新建 outbox 子视图 — C2
- `frontend/src/features/evolution/EvolutionCenterTab.tsx` — C3 runtime-flag + C4 审计
- `frontend/src/features/quality/index.tsx` + 新建评测场景子视图 — D1
- `frontend/src/components/review/ProfilePublishCard.tsx` — D3 状态机展示
- 账号 MCP 密钥表单（D2，挂 Shell 或 command-center）
- `frontend/src/features/ask-human/inline/EscalationInline.tsx` — B1 5 verdict + B3 改派
- `frontend/src/lib/inboxApi.ts` — B2 前端富字段
- `frontend/src/features/knowledge/today.tsx` — E14 派工创建入口
- `frontend/src/features/user-ops/legacy.tsx` — E1 撤销引荐动作

**后端修改：**
- `src/routes/ask_human_inbox.rs` — B2 collect_escalations 富字段投影
- `src/routes/contacts.rs` — E1 新增 clear-referral 端点
- `src/routes/mod.rs` — E1 路由注册

---

## Task 1: A1 — 修复驾驶舱 loadMessages 双死端点（CRITICAL）

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts:283-309`（loadMessages 函数）
- Test: `frontend/src/__tests__/stores/userOpsStore.test.ts`（新建）

**背景：** 选中联系人后驾驶舱 5 个面板（对话/运营记忆/记忆候选/决策复盘/运营健康）全空。根因：`loadMessages` 在一个 `Promise.all` 里并发 5 个 `api.get`，其中 2 个路径后端不存在 → `api.get` 对非 2xx 抛错 → `Promise.all` 全有或全无 → catch 只 `setError`，5 个面板的 `set()` 永不执行。

**真实现状（已实证）：**
- `:292` 调 `/api/contacts/${contact.id}/messages?limit=50` — 后端无此路由。消息真实路由 `routes/mod.rs:371 GET /conversations/:contact_id/messages`。
- `:295` 调 `/api/contacts/${contact.id}/decision-reviews?limit=20` — 后端无此路由。决策复盘真实路由 `routes/mod.rs:690 GET /decision-reviews`（query 过滤）。
- 另 3 个端点（operating-memory / memory-candidates / operation-health）路径正确，保留。
- catch 用 `useUiStore.getState().setError(...)`；`api.get<T>(url)` 单参，非 2xx 抛 `parseApiError`。

**Interfaces:**
- Consumes: `api.get<T>(url: string): Promise<T>`（`frontend/src/lib/api.ts:53`）；`useUiStore.getState().setError(msg: string)`。
- Produces: 无（修内部实现，State 字段签名不变）。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/stores/userOpsStore.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../../lib/api", () => ({ api: { get: vi.fn() } }));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError: vi.fn() }) },
}));

import { api } from "../../lib/api";
import { useUserOpsStore } from "../../stores/userOpsStore";

const contact = { id: "C1" } as any;

describe("userOpsStore.loadMessages", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUserOpsStore.setState({
      messages: [], operatingMemory: null, memoryCandidates: [],
      decisionReviews: [], operationHealth: null,
    } as any);
  });

  it("调用正确的会话与决策复盘端点", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url.includes("/messages")) return Promise.resolve({ items: [{ id: "m1" }] });
      if (url.includes("/operating-memory")) return Promise.resolve({ item: { id: "om" } });
      if (url.includes("/memory-candidates")) return Promise.resolve({ items: [] });
      if (url.includes("/decision-reviews")) return Promise.resolve({ items: [{ id: "dr1" }] });
      if (url.includes("/operation-health")) return Promise.resolve({ ok: true });
      return Promise.reject(new Error("unexpected url " + url));
    });

    await useUserOpsStore.getState().loadMessages(contact);

    const calledUrls = (api.get as any).mock.calls.map((c: any[]) => c[0]);
    expect(calledUrls).toContain("/api/conversations/C1/messages?limit=50");
    expect(calledUrls).toContain("/api/decision-reviews?contactId=C1&limit=20");
    // 不再调用已废弃的 contact-scoped 死端点
    expect(calledUrls).not.toContain("/api/contacts/C1/messages?limit=50");
    expect(calledUrls).not.toContain("/api/contacts/C1/decision-reviews?limit=20");

    const st = useUserOpsStore.getState();
    expect(st.messages).toEqual([{ id: "m1" }]);
    expect(st.decisionReviews).toEqual([{ id: "dr1" }]);
  });

  it("单面板失败不拖垮其余面板（allSettled 加固）", async () => {
    (api.get as any).mockImplementation((url: string) => {
      if (url.includes("/operation-health")) return Promise.reject(new Error("health 500"));
      if (url.includes("/messages")) return Promise.resolve({ items: [{ id: "m1" }] });
      if (url.includes("/operating-memory")) return Promise.resolve({ item: { id: "om" } });
      if (url.includes("/memory-candidates")) return Promise.resolve({ items: [] });
      if (url.includes("/decision-reviews")) return Promise.resolve({ items: [{ id: "dr1" }] });
      return Promise.reject(new Error("unexpected"));
    });

    await useUserOpsStore.getState().loadMessages(contact);

    const st = useUserOpsStore.getState();
    expect(st.messages).toEqual([{ id: "m1" }]);      // 成功面板照常填充
    expect(st.operationHealth).toBeNull();             // 失败面板保持默认
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts`
Expected: FAIL —— 第一个用例断言 `calledUrls` 不含 `/api/conversations/C1/messages`（当前代码仍调 `/api/contacts/...`）；第二个用例因 `Promise.all` reject 导致 messages 为空。

- [ ] **Step 3: 改实现（路径修正 + allSettled 加固）**

把 `frontend/src/stores/userOpsStore.ts:283-309` 的 loadMessages 改为：

```ts
loadMessages: async (contact) => {
  const [messagesR, memoryR, candidateR, reviewsR, healthR] = await Promise.allSettled([
    api.get<{ items: Message[] }>(`/api/conversations/${contact.id}/messages?limit=50`),
    api.get<{ item: OperatingMemory }>(`/api/contacts/${contact.id}/operating-memory`),
    api.get<{ items: MemoryCandidateItem[] }>(`/api/contacts/${contact.id}/memory-candidates?limit=30`),
    api.get<{ items: DecisionReview[] }>(`/api/decision-reviews?contactId=${contact.id}&limit=20`),
    api.get<OperationHealth>(`/api/contacts/${contact.id}/operation-health`),
  ]);
  set({
    messages: messagesR.status === "fulfilled" ? messagesR.value.items : [],
    operatingMemory: memoryR.status === "fulfilled" ? memoryR.value.item : null,
    memoryCandidates: candidateR.status === "fulfilled" ? candidateR.value.items : [],
    decisionReviews: reviewsR.status === "fulfilled" ? reviewsR.value.items : [],
    operationHealth: healthR.status === "fulfilled" ? healthR.value : null,
  });
  const firstErr = [messagesR, memoryR, candidateR, reviewsR, healthR]
    .find((r) => r.status === "rejected") as PromiseRejectedResult | undefined;
  if (firstErr) {
    useUiStore.getState().setError(
      firstErr.reason instanceof Error ? firstErr.reason.message : String(firstErr.reason),
    );
  }
},
```

注意：`OperationHealth` 类型已在 State 接口（`userOpsStore.ts:34-39`）使用，import 应已存在；若 `healthData` 原为 `any`，改 `OperationHealth` 后若编译报错则保留 `api.get<any>` 不影响测试。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts`
Expected: PASS（2 个用例）。

- [ ] **Step 5: 类型检查 + 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: build 成功；vitest 全绿（不引入回归）。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/__tests__/stores/userOpsStore.test.ts
git commit -m "$(cat <<'EOF'
fix(user-ops): 修复驾驶舱 loadMessages 双死端点(A1 critical)

:292 /contacts/:id/messages → /conversations/:id/messages
:295 /contacts/:id/decision-reviews → /decision-reviews?contactId=
Promise.all → allSettled,单面板失败不再拖垮其余四面板。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: C1 — Operations 域加载失败接全局错误横幅

**Files:**
- Modify: `frontend/src/stores/operationsStore.ts:24-52`（loadOperationsData catch）
- Test: `frontend/src/__tests__/stores/operationsStore.test.ts`（新建）

**背景：** 4 个端点齐故障时管理员看到「暂无跟进任务」，误判 Agent 无活动。根因：`operationsStore.ts:42-51` catch 仅 `console.error` 后把 events/tasks/decisionReviews/llmUsage 全置空，不触发任何 UI 错误提示。

**真实现状（已实证）：**
- `OperationsState`（:5-13）无 loading/error 字段。
- `loadOperationsData`（:24-52）Promise.all（:29-34）调 `/api/events|tasks|decision-reviews|llm-usage?{accountParam}`；catch（:42-51）`console.error("Failed to load operations data:", error)` 后置空。
- 全局错误机制：`uiStore.ts` 有 `setError(error: string)`（实现 `set({ error })`），`GlobalErrorBanner.tsx` 已在 `App.tsx:150` 挂载消费 `error`。operationsStore 当前**未** import uiStore。

**Interfaces:**
- Consumes: `useUiStore.getState().setError(msg: string)`（`frontend/src/stores/uiStore.ts`）。
- Produces: 无（不改 State 字段）。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/stores/operationsStore.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const setError = vi.fn();
vi.mock("../../lib/api", () => ({ api: { get: vi.fn() } }));
vi.mock("../../stores/uiStore", () => ({
  useUiStore: { getState: () => ({ setError }) },
}));

import { api } from "../../lib/api";
import { useOperationsStore } from "../../stores/operationsStore";

describe("operationsStore.loadOperationsData", () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it("加载失败时上报全局错误横幅而非静默吞错", async () => {
    (api.get as any).mockRejectedValue(new Error("events 500"));
    await useOperationsStore.getState().loadOperationsData("acc-1");
    expect(setError).toHaveBeenCalledWith(expect.stringContaining("events 500"));
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/stores/operationsStore.test.ts`
Expected: FAIL —— 当前 catch 只 console.error，`setError` 从未被调用。

- [ ] **Step 3: 改实现**

在 `frontend/src/stores/operationsStore.ts` 顶部 import 区加（若未有）：

```ts
import { useUiStore } from "./uiStore";
```

把 catch 块（:42-51）改为：

```ts
  } catch (error) {
    console.error("Failed to load operations data:", error);
    useUiStore.getState().setError(
      error instanceof Error ? error.message : String(error),
    );
    set({ events: [], tasks: [], decisionReviews: [], llmUsage: null });
  }
```

（保留原置空逻辑——错误横幅负责"区分错误态"，置空保持渲染不崩。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/stores/operationsStore.test.ts`
Expected: PASS。

- [ ] **Step 5: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: build 成功；vitest 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/stores/operationsStore.ts frontend/src/__tests__/stores/operationsStore.test.ts
git commit -m "$(cat <<'EOF'
fix(operations): 加载失败接全局错误横幅,区分错误态vs空态(C1)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: C5 — 跟进任务行加「立即复核 / 取消」操作

**Files:**
- Modify: `frontend/src/features/operations/index.tsx:116-138`（跟进任务 tab tbody）
- Test: `frontend/src/__tests__/features/operations/operations.test.tsx`（扩充现有文件）

**背景：** 跟进任务 tab 仅三只读列（StatusBadge / content / runAt），tbody 无任何 button/onClick。后端 `POST /agent-tasks/:id/review-now`（mod.rs:376）与 `POST /agent-tasks/:id/cancel`（mod.rs:377）存在，前端无入口，运营无法干预。

**真实现状（已实证）：**
- `operations/index.tsx:116-138` tasks tab 表格，tbody（:128-136）每行三 `<td>`，无操作列。
- 端点：`/api/agent-tasks/:id/review-now`、`/api/agent-tasks/:id/cancel`（均 POST，无 /admin 前缀）。
- 该 feature 用 `api`（`lib/api`）+ `useOperationsStore` + `useAccountStore`，无独立 store。
- 现有测试 `operations.test.tsx` mock `useOperationsStore`/`useAccountStore`，可扩充。

**Interfaces:**
- Consumes: `api.post<T>(url, body?)`；`useOperationsStore().loadOperationsData(accountId)`（复核/取消后刷新）。
- Produces: 无。

- [ ] **Step 1: 写失败测试（扩充现有文件）**

在 `frontend/src/__tests__/features/operations/operations.test.tsx` 末尾、`describe` 块内追加用例。先在文件顶部 mock api：

```ts
vi.mock("../../../lib/api", () => ({
  api: { post: vi.fn().mockResolvedValue({ ok: true }), get: vi.fn() },
}));
import { api } from "../../../lib/api";
```

追加用例：

```ts
  it("跟进任务行点击「取消」调用 cancel 端点", async () => {
    render(<OperationsFeature />);
    const { fireEvent, waitFor } = await import("@testing-library/react");
    fireEvent.click(screen.getByText("取消"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith("/api/agent-tasks/1/cancel"),
    );
  });
```

（mock 的 tasks 数据 `id: "1"` 已在现有 beforeEach 中。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx`
Expected: FAIL —— 当前无「取消」按钮，`getByText("取消")` 抛 not found。

- [ ] **Step 3: 改实现**

在 `frontend/src/features/operations/index.tsx` 跟进任务 tbody 每行加操作列。表头加一列 `<th>操作</th>`，每行末加：

```tsx
<td>
  <button
    type="button"
    className={styles.linkBtn}
    onClick={async () => {
      await api.post(`/api/agent-tasks/${task.id}/review-now`);
      loadOperationsData(currentAccountId());
    }}
  >
    立即复核
  </button>
  <button
    type="button"
    className={styles.linkBtn}
    onClick={async () => {
      await api.post(`/api/agent-tasks/${task.id}/cancel`);
      loadOperationsData(currentAccountId());
    }}
  >
    取消
  </button>
</td>
```

注意：`loadOperationsData` 与 `currentAccountId` 从现有 store hook 取（文件已解构使用）；`styles.linkBtn` 若不存在则在同目录 `.module.css` 加一个文字按钮样式类（参照设计系统，文字链接型、非蓝色实心按钮）。`api` 从 `../../lib/api` import（若未 import 则加）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/operations/operations.test.tsx`
Expected: PASS（含原有 3 用例 + 新用例）。

- [ ] **Step 5: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/operations/index.tsx frontend/src/__tests__/features/operations/operations.test.tsx
# 若改了 css module 一并 add
git commit -m "$(cat <<'EOF'
feat(operations): 跟进任务行加立即复核/取消操作(C5)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: C2 — outbox 发件箱逐条列表 + 取消

**Files:**
- Create: `frontend/src/features/autonomy/OutboxPanel.tsx`
- Create: `frontend/src/features/autonomy/OutboxPanel.module.css`
- Modify: `frontend/src/features/autonomy/index.tsx`（挂载 OutboxPanel）
- Test: `frontend/src/__tests__/features/autonomy/OutboxPanel.test.tsx`（新建）

**背景：** outbox 是 approved 决策发送链路的真相源（CLAUDE.md 硬规则），无法逐条排障或取消卡住的待发条目。前端仅 autonomy/index.tsx 展示 `/outcomes/autonomy` 聚合比率，无逐条记录、无取消。

**真实现状（已实证）：**
- 后端 `GET /api/admin/outbox`（mod.rs:883，handler `list_outbox`）+ `POST /api/admin/outbox/:id/cancel`（mod.rs:884，`cancel_outbox`）。**带 /admin 前缀。**
- autonomy feature 无独立 store，组件内用 `useState` + `api.get`（index.tsx:100-106 调 `/api/outcomes/autonomy`、`/api/outcomes/autonomy/revisions`）；catch 用本地 `setErr`。
- `outboxLink` 字段（index.tsx:61-69）是无关的类型定义，非 outbox 列表。

**Interfaces:**
- Consumes: `api.get<{ items: OutboxItem[] }>(url)`、`api.post(url)`。
- Produces: `OutboxPanel` React 组件（无 props，自取 accountId）。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/features/autonomy/OutboxPanel.test.tsx`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({
      items: [{ id: "OB1", status: "pending", content: "待发文本", createdAt: null }],
    }),
    post: vi.fn().mockResolvedValue({ ok: true }),
  },
}));
vi.mock("../../../stores/accountStore", () => ({
  useAccountStore: (sel?: any) => {
    const st = { currentAccountId: () => "acc-1" };
    return typeof sel === "function" ? sel(st) : st;
  },
}));

import { api } from "../../../lib/api";
import { OutboxPanel } from "../../../features/autonomy/OutboxPanel";

describe("OutboxPanel", () => {
  beforeEach(() => vi.clearAllMocks());

  it("拉取并渲染 outbox 逐条记录", async () => {
    render(<OutboxPanel />);
    await waitFor(() => expect(screen.getByText("待发文本")).toBeInTheDocument());
    expect(api.get).toHaveBeenCalledWith(expect.stringContaining("/api/admin/outbox"));
  });

  it("点击取消调用 cancel 端点", async () => {
    render(<OutboxPanel />);
    await waitFor(() => screen.getByText("待发文本"));
    fireEvent.click(screen.getByText("取消"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith("/api/admin/outbox/OB1/cancel"),
    );
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/autonomy/OutboxPanel.test.tsx`
Expected: FAIL —— `OutboxPanel` 模块不存在。

- [ ] **Step 3: 建组件**

`frontend/src/features/autonomy/OutboxPanel.tsx`：

```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { useAccountStore } from "../../stores/accountStore";
import styles from "./OutboxPanel.module.css";

type OutboxItem = {
  id: string;
  status: string;
  content: string;
  createdAt: string | null;
};

export function OutboxPanel() {
  const accountId = useAccountStore((s) => s.currentAccountId());
  const [items, setItems] = useState<OutboxItem[]>([]);

  const load = async () => {
    const qs = accountId ? `?accountId=${accountId}` : "";
    const data = await api.get<{ items: OutboxItem[] }>(`/api/admin/outbox${qs}`);
    setItems(data.items);
  };

  useEffect(() => { void load(); }, [accountId]);

  return (
    <div className={styles.panel}>
      <h3 className={styles.title}>发件箱（outbox）</h3>
      <table className={styles.table}>
        <thead>
          <tr><th>状态</th><th>内容</th><th>操作</th></tr>
        </thead>
        <tbody>
          {items.map((it) => (
            <tr key={it.id}>
              <td>{it.status}</td>
              <td>{it.content}</td>
              <td>
                <button
                  type="button"
                  className={styles.linkBtn}
                  onClick={async () => {
                    await api.post(`/api/admin/outbox/${it.id}/cancel`);
                    await load();
                  }}
                >
                  取消
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
```

`frontend/src/features/autonomy/OutboxPanel.module.css`（参照设计系统 tokens，文字按钮非实心蓝）：

```css
.panel { margin-top: var(--space-6, 24px); }
.title { font-size: var(--text-base, 15px); font-weight: 600; margin-bottom: var(--space-3, 12px); }
.table { width: 100%; border-collapse: collapse; }
.table th, .table td { text-align: left; padding: var(--space-2, 8px); border-bottom: 1px solid var(--border-subtle, #eceef1); }
.linkBtn { background: none; border: none; color: var(--text-link, #2563eb); cursor: pointer; padding: 0; font: inherit; }
```

（先确认 tokens.css 里实际的变量名，对不上则用现有同类组件的变量；fallback 值保证不报错。）

- [ ] **Step 4: 挂载到 autonomy feature**

在 `frontend/src/features/autonomy/index.tsx` 顶部 import：

```tsx
import { OutboxPanel } from "./OutboxPanel";
```

在 autonomy 主体渲染区合适位置（聚合比率下方）加 `<OutboxPanel />`。

- [ ] **Step 5: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/autonomy/OutboxPanel.test.tsx`
Expected: PASS（2 用例）。

- [ ] **Step 6: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 7: 提交**

```bash
git add frontend/src/features/autonomy/OutboxPanel.tsx frontend/src/features/autonomy/OutboxPanel.module.css frontend/src/features/autonomy/index.tsx frontend/src/__tests__/features/autonomy/OutboxPanel.test.tsx
git commit -m "$(cat <<'EOF'
feat(autonomy): outbox 发件箱逐条列表+取消(C2)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: C3 — 演化中心 runtime-flag 灰度开关 + rollout 比例

**Files:**
- Modify: `frontend/src/features/evolution/EvolutionCenterTab.tsx`
- Test: `frontend/src/__tests__/EvolutionCenterTab.test.tsx`（扩充现有）

**背景：** 管理员无法在 UI 调灰度比例，只能改 env 全开/全关。后端 workspace 级 runtime-flag 已存在。

**真实现状（已实证）：**
- 后端 `GET/PUT /api/evolution/runtime-flag`（mod.rs:984-987，handler `get_evolution_runtime_flag`/`put_evolution_runtime_flag`）。**无 /admin 前缀。**
- PUT body struct `UpdateRuntimeFlagRequest`（evolution.rs:548-559，camelCase）：`enabled: bool`、`rolloutPercent: u32`（server 钳 ≤100）、`updatedBy?`、`thresholdAutoReleaseEnabled?`。
- GET 返回同形（含 enabled / rolloutPercent）。
- `EvolutionCenterTab.tsx` 现唯一端点 :127 `apiGet<ExperimentsResponse>("/api/evolution/experiments?limit=20")`；:143-144 静态文案。组件用 `apiGet`（核对其 import 来源——可能是 `lib/api` 的别名）。
- 现有测试 `EvolutionCenterTab.test.tsx` 存在，可扩充。

**Interfaces:**
- Consumes: `api.get<RuntimeFlag>("/api/evolution/runtime-flag")`、`api.put("/api/evolution/runtime-flag", body)`。
- Produces: 无（组件内状态）。

- [ ] **Step 1: 确认 api 调用别名**

Read `frontend/src/features/evolution/EvolutionCenterTab.tsx` 顶部 import，确认 `apiGet` 是什么（`import { api } from ...` 还是具名 `apiGet`）。后续用同一形态。确认 `api` 是否有 `put` 方法（Read `frontend/src/lib/api.ts`）。

- [ ] **Step 2: 写失败测试**

在 `frontend/src/__tests__/EvolutionCenterTab.test.tsx` 追加（沿用文件现有 mock 形态）：

```ts
  it("渲染 runtime-flag 灰度控件并 PUT 保存", async () => {
    const { fireEvent, waitFor, screen } = await import("@testing-library/react");
    // 假设文件已 mock api；runtime-flag GET 返回 enabled:false rolloutPercent:0
    // 渲染后改 rolloutPercent 输入为 50，点保存，断言 PUT body
    // （具体 mock 注入沿用本文件顶部既有 vi.mock("...lib/api") 结构）
    render(<EvolutionCenterTab enabled={true} />);
    const input = await screen.findByLabelText(/灰度比例/);
    fireEvent.change(input, { target: { value: "50" } });
    fireEvent.click(screen.getByText("保存灰度"));
    await waitFor(() =>
      expect(api.put).toHaveBeenCalledWith(
        "/api/evolution/runtime-flag",
        expect.objectContaining({ rolloutPercent: 50 }),
      ),
    );
  });
```

（注：实现 Step 3 时把测试里的 mock 形态对齐文件顶部既有结构——若现有文件 mock 的是 `apiGet` 具名导出，则 api.put 也需在该 mock 中暴露。）

- [ ] **Step 3: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/EvolutionCenterTab.test.tsx`
Expected: FAIL —— 无「灰度比例」控件 / 无「保存灰度」按钮。

- [ ] **Step 4: 改实现**

在 `EvolutionCenterTab.tsx` 加 runtime-flag 区块：useEffect GET `/api/evolution/runtime-flag` 初始化 `enabled`/`rolloutPercent` 两个 state；渲染一个 `enabled` 开关（checkbox）+ 一个 `rolloutPercent` 数字输入（`<label>灰度比例</label>`，min 0 max 100）+「保存灰度」按钮，onClick PUT：

```tsx
await api.put("/api/evolution/runtime-flag", {
  enabled: flagEnabled,
  rolloutPercent: Number(rollout),
});
```

样式用现有 `EvolutionCenterTab` 的 CSS module，文案不含禁词。

- [ ] **Step 5: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/EvolutionCenterTab.test.tsx`
Expected: PASS。

- [ ] **Step 6: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 7: 提交**

```bash
git add frontend/src/features/evolution/EvolutionCenterTab.tsx frontend/src/__tests__/EvolutionCenterTab.test.tsx
git commit -m "$(cat <<'EOF'
feat(evolution): runtime-flag 灰度开关+rollout比例UI(C3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: C4 — 演化中心阈值变更不可变审计日志视图

**Files:**
- Modify: `frontend/src/features/evolution/EvolutionCenterTab.tsx`
- Test: `frontend/src/__tests__/EvolutionCenterTab.test.tsx`（扩充）

**背景：** 自演化合规追溯/可观测性缺失。后端审计端点已存在但前端 0 调用。

**真实现状（已实证）：**
- 后端 `GET /api/evolution/threshold-overrides/audit`（mod.rs:978-981，handler `list_threshold_override_audit`）。返回 release/rollback/auto-release 历史行。**无 /admin 前缀。**
- 前端 grep `threshold-overrides` 0 命中。

**Interfaces:**
- Consumes: `api.get<{ items: AuditRow[] }>("/api/evolution/threshold-overrides/audit")`。
- Produces: 无。

- [ ] **Step 1: 写失败测试**

在 `EvolutionCenterTab.test.tsx` 追加：

```ts
  it("渲染阈值变更审计日志", async () => {
    const { waitFor, screen } = await import("@testing-library/react");
    // mock api.get 对 /threshold-overrides/audit 返回 [{ action:"release", at:"...", by:"admin" }]
    render(<EvolutionCenterTab enabled={true} />);
    await waitFor(() => expect(screen.getByText(/审计/)).toBeInTheDocument());
    expect(api.get).toHaveBeenCalledWith(
      expect.stringContaining("/api/evolution/threshold-overrides/audit"),
    );
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/EvolutionCenterTab.test.tsx`
Expected: FAIL —— 无审计端点调用。

- [ ] **Step 3: 改实现**

在 `EvolutionCenterTab.tsx` useEffect 加 GET `/api/evolution/threshold-overrides/audit`，存 `auditRows` state；渲染一个「阈值变更审计」表格（action / at / by / detail 列）。空时显示「暂无审计记录」。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/EvolutionCenterTab.test.tsx`
Expected: PASS。

- [ ] **Step 5: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/evolution/EvolutionCenterTab.tsx frontend/src/__tests__/EvolutionCenterTab.test.tsx
git commit -m "$(cat <<'EOF'
feat(evolution): 阈值变更不可变审计日志视图(C4)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: D1 — 评测场景 CRUD 入口

**Files:**
- Create: `frontend/src/features/quality/EvaluationScenariosPanel.tsx`
- Create: `frontend/src/features/quality/EvaluationScenariosPanel.module.css`
- Modify: `frontend/src/features/quality/index.tsx`（挂载）
- Test: `frontend/src/__tests__/features/quality/EvaluationScenariosPanel.test.tsx`（新建）

**背景：** formula-adherence 评测完全依赖 active evaluation_scenarios，但管理员无法自助配置评测基准，只能后端写库。

**真实现状（已实证）：**
- 后端 `GET/POST /api/evaluation-scenarios`（mod.rs:702-705，`list_evaluation_scenarios`/`create_evaluation_scenario`）；`PUT/DELETE /api/evaluation-scenarios/:id`（mod.rs:706-709）。**无 /admin 前缀。**
- POST/PUT body struct `EvaluationScenarioRequest`（evaluations.rs:26-43，camelCase）：`scenarioId`、`title`（必填）、`description`（default）、`accountId?`、`contactSeed: Document`（default）、`inboundMessages: string[]`（default）、`groundTruth: Document`（default）、`tags: string[]`（default）、`status?`。
- quality feature 无独立 store，本地 useState + `api`。

**Interfaces:**
- Consumes: `api.get<{ items: Scenario[] }>("/api/evaluation-scenarios")`、`api.post`、`api.put`、`api.delete`（确认 lib/api 有 delete；若无则用 `api.request("DELETE", url)` 形态——Step 1 核对）。
- Produces: `EvaluationScenariosPanel` 组件。

- [ ] **Step 1: 核对 api 方法**

Read `frontend/src/lib/api.ts`，确认有 `post`/`put`/`delete` 方法及签名。后续按实际形态写。

- [ ] **Step 2: 写失败测试**

新建 `frontend/src/__tests__/features/quality/EvaluationScenariosPanel.test.tsx`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../../lib/api", () => ({
  api: {
    get: vi.fn().mockResolvedValue({ items: [{ scenarioId: "S1", title: "议价场景", status: "active" }] }),
    post: vi.fn().mockResolvedValue({ ok: true }),
    delete: vi.fn().mockResolvedValue({ ok: true }),
  },
}));

import { api } from "../../../lib/api";
import { EvaluationScenariosPanel } from "../../../features/quality/EvaluationScenariosPanel";

describe("EvaluationScenariosPanel", () => {
  beforeEach(() => vi.clearAllMocks());

  it("渲染评测场景列表", async () => {
    render(<EvaluationScenariosPanel />);
    await waitFor(() => expect(screen.getByText("议价场景")).toBeInTheDocument());
  });

  it("新建场景提交 POST", async () => {
    render(<EvaluationScenariosPanel />);
    await waitFor(() => screen.getByText("议价场景"));
    fireEvent.change(screen.getByPlaceholderText(/场景标识/), { target: { value: "S2" } });
    fireEvent.change(screen.getByPlaceholderText(/场景标题/), { target: { value: "退款场景" } });
    fireEvent.click(screen.getByText("新建场景"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/evaluation-scenarios",
        expect.objectContaining({ scenarioId: "S2", title: "退款场景" }),
      ),
    );
  });
});
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/quality/EvaluationScenariosPanel.test.tsx`
Expected: FAIL —— 模块不存在。

- [ ] **Step 4: 建组件**

`frontend/src/features/quality/EvaluationScenariosPanel.tsx`：列表（GET）+ 新建表单（scenarioId / title / description / inboundMessages 文本域逐行）+ 每行删除按钮（DELETE `/api/evaluation-scenarios/:id`）。提交 POST body 用 camelCase `{ scenarioId, title, description, inboundMessages }`。新建/删除后 reload。CSS module 参照设计系统。

最小可过测试的骨架（删除/编辑可在此结构上扩）：

```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import styles from "./EvaluationScenariosPanel.module.css";

type Scenario = { scenarioId: string; title: string; status?: string };

export function EvaluationScenariosPanel() {
  const [items, setItems] = useState<Scenario[]>([]);
  const [scenarioId, setScenarioId] = useState("");
  const [title, setTitle] = useState("");

  const load = async () => {
    const d = await api.get<{ items: Scenario[] }>("/api/evaluation-scenarios");
    setItems(d.items);
  };
  useEffect(() => { void load(); }, []);

  const create = async () => {
    await api.post("/api/evaluation-scenarios", { scenarioId, title });
    setScenarioId(""); setTitle("");
    await load();
  };

  return (
    <div className={styles.panel}>
      <h3 className={styles.title}>评测场景</h3>
      <ul>{items.map((s) => <li key={s.scenarioId}>{s.title}</li>)}</ul>
      <input placeholder="场景标识(scenarioId)" value={scenarioId} onChange={(e) => setScenarioId(e.target.value)} />
      <input placeholder="场景标题" value={title} onChange={(e) => setTitle(e.target.value)} />
      <button type="button" onClick={create}>新建场景</button>
    </div>
  );
}
```

（实施时按设计系统补样式 + 删除/编辑/inboundMessages 字段；骨架先让测试通过。）

- [ ] **Step 5: 挂载到 quality feature**

`frontend/src/features/quality/index.tsx` import 并在评测相关区块渲染 `<EvaluationScenariosPanel />`（靠近 :276-280 formula-adherence 文案处）。

- [ ] **Step 6: 跑测试 + 全量**

Run: `cd frontend && npx vitest run src/__tests__/features/quality/EvaluationScenariosPanel.test.tsx`
Expected: PASS。
Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 7: 提交**

```bash
git add frontend/src/features/quality/EvaluationScenariosPanel.tsx frontend/src/features/quality/EvaluationScenariosPanel.module.css frontend/src/features/quality/index.tsx frontend/src/__tests__/features/quality/EvaluationScenariosPanel.test.tsx
git commit -m "$(cat <<'EOF'
feat(quality): 评测场景CRUD入口(D1)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: D2 — 账号 MCP 密钥配置表单

**Files:**
- Create: `frontend/src/features/command-center/McpKeyForm.tsx`
- Create: `frontend/src/features/command-center/McpKeyForm.module.css`
- Modify: `frontend/src/features/command-center/index.tsx`（挂载，该页已用 mcpKeyConfigured）
- Test: `frontend/src/__tests__/features/command-center/McpKeyForm.test.tsx`（新建）

**背景：** 新账号接入 WeChat 工具链只能后端改库，管理员无法自助开通。

**真实现状（已实证）：**
- 后端 `PUT /api/accounts/:id/mcp-key`（mod.rs:312，handler `update_account_mcp_key`）。**无 /admin 前缀。**
- body struct `UpdateAccountMcpKeyRequest`（accounts.rs:27-30）：`mcp_api_key: String`、`mcp_base_url: Option<String>`。**非 camelCase——JSON 键就是 snake_case `mcp_api_key` / `mcp_base_url`。**
- 前端 `types/index.ts:37 mcpKeyConfigured?: boolean` 只读布尔，command-center/index.tsx:100 用它做状态色。无写表单。accountStore 纯本地 state。

**安全约束：** 密钥是敏感值——输入框用 `type="password"`，不回显已存值，仅显示「已配置」布尔。提交后清空输入框。

**Interfaces:**
- Consumes: `api.put(url, body)`，body 键为 snake_case。
- Produces: `McpKeyForm` 组件（props: `{ accountId: string; configured: boolean }`）。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/features/command-center/McpKeyForm.test.tsx`：

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../../../lib/api", () => ({ api: { put: vi.fn().mockResolvedValue({ ok: true }) } }));
import { api } from "../../../lib/api";
import { McpKeyForm } from "../../../features/command-center/McpKeyForm";

describe("McpKeyForm", () => {
  beforeEach(() => vi.clearAllMocks());

  it("提交以 snake_case 键 PUT 密钥,且不回显明文已存值", async () => {
    render(<McpKeyForm accountId="acc-1" configured={true} />);
    const input = screen.getByLabelText(/MCP 密钥/);
    expect((input as HTMLInputElement).type).toBe("password");
    expect((input as HTMLInputElement).value).toBe(""); // 不回显已存值
    fireEvent.change(input, { target: { value: "secret-key-123" } });
    fireEvent.click(screen.getByText("保存密钥"));
    await waitFor(() =>
      expect(api.put).toHaveBeenCalledWith(
        "/api/accounts/acc-1/mcp-key",
        expect.objectContaining({ mcp_api_key: "secret-key-123" }),
      ),
    );
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/command-center/McpKeyForm.test.tsx`
Expected: FAIL —— 模块不存在。

- [ ] **Step 3: 建组件**

`frontend/src/features/command-center/McpKeyForm.tsx`：

```tsx
import { useState } from "react";
import { api } from "../../lib/api";
import styles from "./McpKeyForm.module.css";

export function McpKeyForm({ accountId, configured }: { accountId: string; configured: boolean }) {
  const [key, setKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [saved, setSaved] = useState(false);

  const save = async () => {
    await api.put(`/api/accounts/${accountId}/mcp-key`, {
      mcp_api_key: key,
      ...(baseUrl ? { mcp_base_url: baseUrl } : {}),
    });
    setKey(""); setSaved(true);
  };

  return (
    <div className={styles.form}>
      <label htmlFor="mcpKey">MCP 密钥{configured ? "（已配置，留空不变）" : ""}</label>
      <input id="mcpKey" type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" />
      <label htmlFor="mcpBase">MCP Base URL（可选）</label>
      <input id="mcpBase" type="text" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
      <button type="button" onClick={save}>保存密钥</button>
      {saved && <span className={styles.ok}>已保存</span>}
    </div>
  );
}
```

`McpKeyForm.module.css` 参照设计系统（表单纵向布局）。

- [ ] **Step 4: 挂载**

`frontend/src/features/command-center/index.tsx` 在账号信息区（:100 用 mcpKeyConfigured 处附近）渲染 `<McpKeyForm accountId={currentAccount.id} configured={!!currentAccount?.mcpKeyConfigured} />`。

- [ ] **Step 5: 跑测试 + 全量**

Run: `cd frontend && npx vitest run src/__tests__/features/command-center/McpKeyForm.test.tsx`
Expected: PASS。
Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/command-center/McpKeyForm.tsx frontend/src/features/command-center/McpKeyForm.module.css frontend/src/features/command-center/index.tsx frontend/src/__tests__/features/command-center/McpKeyForm.test.tsx
git commit -m "$(cat <<'EOF'
feat(command-center): 账号MCP密钥配置表单(D2)

密钥输入用password型不回显已存值;body键为snake_case。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: D3 — AI 生成状态机本体人审展示

**Files:**
- Modify: `frontend/src/components/review/ProfilePublishCard.tsx`
- Modify: `frontend/src/types/index.ts`（扩 ProfileLite / 加 GeneratedStateMachine 类型）
- Test: `frontend/src/__tests__/components/review/ProfilePublishCard.test.tsx`（新建）

**背景：** AI 生成状态机走 draft + 人审红线，但管理员激活前无界面审阅其 states/goal/advanceSignals/riskRules。

**真实现状（已实证）：**
- `ProfilePublishCard.tsx` 数据模型 `ProfileLite`（:17-22）只有 `id/display_name/is_active/current_version`；渲染体（:118-136）仅 display_name + 状态文案。
- 前端无 `GeneratedStateMachine` 类型，grep generated_state_machine 0 命中。
- 后端 `guide_profile.rs` 落 draft、`domain_profiles.rs` activate 时 validate_state_machine。状态机字段 states/goal/advanceSignals/riskRules（核对后端 generated_state_machine 的实际 JSON 形态——Step 1）。

**Interfaces:**
- Consumes: `ProfileLite` 扩展后含可选 `generated_state_machine`。
- Produces: 扩展的 `ProfileLite` 类型 + 渲染。

- [ ] **Step 1: 核对后端状态机字段名**

Read `src/agent/.../guide_profile.rs` 或 `src/models.rs` 中 generated_state_machine / state machine 的 struct（grep `generated_state_machine` / `GeneratedStateMachine` / `state_machine` in src/），确认 states/goal/advanceSignals/riskRules 的真实 JSON 键名与嵌套（决定前端类型形态）。**这是 Task 9 的硬前置——类型不对测试白写。**

- [ ] **Step 2: 写失败测试**

新建 `frontend/src/__tests__/components/review/ProfilePublishCard.test.tsx`：

```ts
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ProfilePublishCard } from "../../../components/review/ProfilePublishCard";

// 按 Step1 核对结果填充真实字段名；下例假设 states[].name / goal
const profile = {
  id: "P1", display_name: "母婴顾问", is_active: false, current_version: 1,
  generated_state_machine: {
    goal: "促成到店",
    states: [{ name: "new_contact" }, { name: "engaged" }],
    advance_signals: ["明确询价"],
    risk_rules: ["禁止虚假承诺"],
  },
} as any;

describe("ProfilePublishCard 状态机人审", () => {
  it("激活前展示状态机 goal/states/advanceSignals/riskRules", () => {
    render(<ProfilePublishCard profile={profile} onPublish={vi.fn()} onActivate={vi.fn()} />);
    expect(screen.getByText("促成到店")).toBeInTheDocument();
    expect(screen.getByText("new_contact")).toBeInTheDocument();
    expect(screen.getByText("engaged")).toBeInTheDocument();
  });
});
```

（render props 按 ProfilePublishCard 真实 props 形态调整——Step 2 实施时先读组件 props。）

- [ ] **Step 3: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/components/review/ProfilePublishCard.test.tsx`
Expected: FAIL —— 状态机内容未渲染。

- [ ] **Step 4: 扩类型 + 渲染**

`types/index.ts`：给 `ProfileLite`（或 ProfilePublishCard 的 props 类型）加可选 `generated_state_machine?: { goal?: string; states?: {name:string}[]; advance_signals?: string[]; risk_rules?: string[] }`（字段名以 Step 1 核对为准）。
`ProfilePublishCard.tsx` 渲染体加状态机审阅区：goal 一行、states 列表、advance_signals 列表、risk_rules 列表。仅当 `generated_state_machine` 存在时渲染。

- [ ] **Step 5: 跑测试 + 全量**

Run: `cd frontend && npx vitest run src/__tests__/components/review/ProfilePublishCard.test.tsx`
Expected: PASS。
Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/components/review/ProfilePublishCard.tsx frontend/src/types/index.ts frontend/src/__tests__/components/review/ProfilePublishCard.test.tsx
git commit -m "$(cat <<'EOF'
feat(review): AI生成状态机本体人审展示(D3)

激活前展示 generated_state_machine 的 goal/states/advanceSignals/riskRules。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: B1 — 请示裁决扩 5 种 verdict + 条件授权窗录入

**Files:**
- Modify: `frontend/src/features/ask-human/inline/EscalationInline.tsx`
- Test: `frontend/src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`（扩充现有）

**背景：** UI 仅批准/驳回两按钮，后端 5 值闭集 + 条件授权窗语义前端不可达，3 种裁决类型不可用。

**真实现状（已实证）：**
- `EscalationInline.tsx`（全 42 行）props `{ item, ctx }: { item: InboxItem; ctx: RowCtx }`，`code = item.id`（:8）。
- resolve（:10）`function resolve(verdict: "approved" | "rejected")`，POST `/api/admin/principal-escalations/${code}/resolve`，body（:14-18）`{ verdict, substance, constraints: [], authorizationWindowHours: null }`。
- 按钮（:33-38）仅「批准」「驳回」。`substance` 来自一个 textarea（:27，placeholder 含「裁决意见」）。
- 后端 `ResolveBody`（principal_escalations.rs:61-71，camelCase）：`verdict`、`substance`、`constraints: string[]`、`authorizationWindowHours: number?`。`ALLOWED_PRINCIPAL_VERDICT`（models.rs:3253-3257）= approved/rejected/conditional/deferred/delegated_back。
- ctx 形态：`{ busy: boolean; runAction: (fn) => Promise<void> }`（现有测试 :21-24 已用）。

**Interfaces:**
- Consumes: `api.post(url, body)`；`ctx.runAction`。
- Produces: 无（组件内）。

- [ ] **Step 1: 写失败测试（扩充现有文件）**

在 `EscalationInline.test.tsx` 追加：

```ts
  it("选 conditional 时显示授权窗输入,提交 body 含 constraints+authorizationWindowHours", async () => {
    const runAction = vi.fn(async (fn: () => Promise<unknown>) => { await fn(); });
    render(<EscalationInline item={item} ctx={{ busy: false, runAction }} />);
    // 选择 conditional 裁决
    fireEvent.change(screen.getByLabelText(/裁决类型/), { target: { value: "conditional" } });
    // 授权窗输入出现
    const win = screen.getByLabelText(/授权窗/);
    fireEvent.change(win, { target: { value: "48" } });
    fireEvent.change(screen.getByPlaceholderText(/约束条款/), { target: { value: "仅限本月" } });
    fireEvent.change(screen.getByPlaceholderText(/裁决意见/), { target: { value: "有条件同意" } });
    fireEvent.click(screen.getByText("提交裁决"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/principal-escalations/ESC1/resolve",
        expect.objectContaining({
          verdict: "conditional",
          authorizationWindowHours: 48,
          constraints: ["仅限本月"],
        }),
      ),
    );
  });
```

注意：保留现有「批准」用例兼容——若改成下拉+提交按钮，需同步更新旧用例（旧用例点「批准」按钮，新 UI 改为选 approved + 点「提交裁决」）。**Step 4 须把现有 :20-33 用例改为新交互**，避免旧用例失败。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`
Expected: FAIL（新用例找不到「裁决类型」下拉）+ 旧用例可能因 UI 改动失败（预期，Step 4 修）。

- [ ] **Step 3: 改实现**

`EscalationInline.tsx` 改造：
- resolve 形参改 `verdict: string`（5 值之一）。
- 加 state：`verdict`（默认 "approved"）、`windowHours`（string）、`constraintText`（string）。
- 渲染：`<label>裁决类型</label>` + `<select>` 5 个 option（approved/rejected/conditional/deferred/delegated_back，中文标签）；当 `verdict === "conditional"` 时显示 `<label>授权窗(小时)</label><input type="number">` + `<input placeholder="约束条款">`。
- 一个「提交裁决」按钮，POST body：

```tsx
{
  verdict,
  substance,
  constraints: constraintText ? [constraintText] : [],
  authorizationWindowHours: verdict === "conditional" && windowHours ? Number(windowHours) : null,
}
```

- 中文 verdict 标签 map（闭集 5 值）：approved=批准 / rejected=驳回 / conditional=有条件批准 / deferred=暂缓 / delegated_back=退回再议。文案不含禁词。

- [ ] **Step 4: 更新旧用例 + 跑测试确认通过**

把现有「批准」用例改为：选 approved（下拉）+ 填 substance + 点「提交裁决」，断言 body `verdict: "approved"`。

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`
Expected: PASS（旧用例改造后 + 新 conditional 用例）。

- [ ] **Step 5: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/ask-human/inline/EscalationInline.tsx frontend/src/__tests__/features/ask-human/inline/EscalationInline.test.tsx
git commit -m "$(cat <<'EOF'
feat(ask-human): 请示裁决扩5种verdict+条件授权窗录入(B1)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: B3 — 请示改派 reassign 入口

**Files:**
- Modify: `frontend/src/features/ask-human/inline/EscalationInline.tsx`
- Test: `frontend/src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`（扩充）

**背景：** 决策人链超时无法转备选，请示卡死在原决策人。后端端点已存在，前端 0 入口。

**真实现状（已实证）：**
- 后端 `POST /api/admin/principal-escalations/:short_code/reassign`（mod.rs:851-854，`reassign_principal_escalation`）。body struct `ReassignBody`（principal_escalations.rs:115-119）= `{ to_wxid: String }`。**注意 snake_case 键 `to_wxid`。**
- 前端 grep reassign / 改派 / toWxid 全 0 命中。

**Interfaces:**
- Consumes: `api.post(url, { to_wxid })`；`ctx.runAction`。
- Produces: 无。

依赖：Task 10 已先改 EscalationInline。本 task 在其基础上加改派动作。

- [ ] **Step 1: 写失败测试（扩充）**

```ts
  it("改派提交 to_wxid 到 reassign 端点", async () => {
    const runAction = vi.fn(async (fn: () => Promise<unknown>) => { await fn(); });
    render(<EscalationInline item={item} ctx={{ busy: false, runAction }} />);
    fireEvent.change(screen.getByPlaceholderText(/备选决策人/), { target: { value: "wxid_backup" } });
    fireEvent.click(screen.getByText("改派"));
    await waitFor(() =>
      expect(api.post).toHaveBeenCalledWith(
        "/api/admin/principal-escalations/ESC1/reassign",
        { to_wxid: "wxid_backup" },
      ),
    );
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`
Expected: FAIL —— 无「改派」按钮 / 「备选决策人」输入。

- [ ] **Step 3: 改实现**

`EscalationInline.tsx` 加：state `reassignWxid`（string）；渲染一个 `<input placeholder="备选决策人 wxid">` + 「改派」按钮，onClick 经 `ctx.runAction` POST `/api/admin/principal-escalations/${code}/reassign`，body `{ to_wxid: reassignWxid }`。文案不含禁词（用「改派」非「转人工」）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`
Expected: PASS。

- [ ] **Step 5: 全量前端测试**

Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/ask-human/inline/EscalationInline.tsx frontend/src/__tests__/features/ask-human/inline/EscalationInline.test.tsx
git commit -m "$(cat <<'EOF'
feat(ask-human): 请示改派reassign入口(B3)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: B2（后端）— InboxItem 富字段投影

**Files:**
- Modify: `src/routes/ask_human_inbox.rs:16-29`（InboxItem struct）+ `:38-60`（collect_escalations）+ 其余 7 处构造点
- Test: `tests/ask_human_inbox_projection.rs`（新建集成测，或加进现有 ask-human 测试文件）

**背景：** 决策人在不知道客户是谁、请示什么问题、属哪类的情况下盲裁。根因后端聚合器把富数据压扁成 title+summary。

**真实现状（已实证）：**
- `InboxItem` struct（ask_human_inbox.rs:16-29，`#[serde(rename_all="camelCase")]`）：source/id/title/summary/severity: String，created_at: Option<DateTime>，age_hours: f64，action_kind: String，rich_component: Option<String>（skip if none），rich_params: Option<Document>（skip if none）。
- `collect_escalations`（:38-60）映射时 title=`format!("请示 #{}", e.short_code)`、summary=`e.reason.clone()`，丢弃 category/question_for_principal/contact_wxid/principal_wxid。
- 源 struct `AgentPrincipalEscalation`（models.rs:3311-3339+）可用字段：category/reason/question_for_principal/contact_wxid/principal_wxid（均 String）。
- **InboxItem 其余 7 处构造点**（必须不回归）：knowledge_review(:82-93)、taxonomy_candidate(:119-130)、relationship_suggestion(:155-166)、gap_signal(:191-202)、profile_risky(:227-238)、evolution_proposal(:263-274)、lessons_learned(:303-314)。所有构造点用具名字段初始化**全部**字段。

**关键约束：** 新增字段须 `Option<String>` + `#[serde(skip_serializing_if = "Option::is_none")]`，且**每处构造点显式填 `None`**（Rust struct 初始化不支持 `..Default`，除非 InboxItem 实现 Default——本 task 不引入 Default 以免掩盖遗漏）。

**Interfaces:**
- Produces: InboxItem 加 4 可选字段 `category` / `question_for_principal` / `contact_wxid` / `principal_wxid`（camelCase 序列化为 category/questionForPrincipal/contactWxid/principalWxid）。

- [ ] **Step 1: 写失败测试**

新建 `tests/ask_human_inbox_projection.rs`（参照现有 tests/ 里 ask-human 相关测试的 setup；若现有有 ask_human 集成测文件，加用例进去）。核心断言：collect_escalations 对一个含 category/question_for_principal/contact_wxid 的 escalation，产出的 InboxItem 序列化 JSON 含 `questionForPrincipal` 等字段。

由于该投影函数可能是 `pub(crate)`，优先写**单元测试**放在 `ask_human_inbox.rs` 文件底部 `#[cfg(test)] mod tests`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalation_projection_carries_rich_fields() {
        let esc = AgentPrincipalEscalation {
            // 用最小构造：必填字段给值,category/question_for_principal/contact_wxid 给真实值
            // (按 models.rs:3311 实际字段补全)
            ..test_escalation_fixture()
        };
        let item = escalation_to_inbox_item(&esc); // 若现有是闭包,重构出具名函数
        assert_eq!(item.question_for_principal.as_deref(), Some("能否给折扣"));
        assert_eq!(item.contact_wxid.as_deref(), Some("wxid_cust"));
    }
}
```

注：若 collect_escalations 内是内联 map 闭包，Step 3 先抽出具名函数 `fn escalation_to_inbox_item(e: &AgentPrincipalEscalation) -> InboxItem` 再测（可测性改造）。`test_escalation_fixture()` 按 models.rs:3311 真实字段写。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib escalation_projection 2>&1 | tail -20`（若放 routes 模块单测）或对应 `cargo test --test ask_human_inbox_projection`
Expected: FAIL —— InboxItem 无 question_for_principal 字段（编译错误即"失败"）。

- [ ] **Step 3: 改实现**

① InboxItem struct（:16-29）加 4 字段：

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub category: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub question_for_principal: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub contact_wxid: Option<String>,
#[serde(skip_serializing_if = "Option::is_none")]
pub principal_wxid: Option<String>,
```

② collect_escalations（:38-60）填充这 4 字段（从 e.category / e.question_for_principal / e.contact_wxid / e.principal_wxid，空串则 None）。
③ **其余 7 处构造点**（knowledge_review 等）每处加 `category: None, question_for_principal: None, contact_wxid: None, principal_wxid: None,`。

- [ ] **Step 4: 跑测试 + 编译门**

Run: `cargo test --lib escalation_projection 2>&1 | tail -20`
Expected: PASS。
Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -20`
Expected: 0 error 0 warning（验证 7 处构造点全补齐，无遗漏导致的编译错误）。

- [ ] **Step 5: 基线门**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥350 passed, 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/routes/ask_human_inbox.rs tests/ask_human_inbox_projection.rs
git commit -m "$(cat <<'EOF'
feat(ask-human): InboxItem 富字段投影,请示卡不再压扁(B2后端)

加 category/questionForPrincipal/contactWxid/principalWxid 可选字段,
collect_escalations 填充;其余7处InboxItem构造点显式填None不回归。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: B2（前端）— 请示卡富展示

**Files:**
- Modify: `frontend/src/lib/inboxApi.ts:3-14`（InboxItem 接口）
- Modify: `frontend/src/features/ask-human/inline/EscalationInline.tsx`（富展示）
- Test: `frontend/src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`（扩充）

**背景：** 前端 InboxItem 接口无富字段，EscalationInline 仅渲染 title/summary。依赖 Task 12 后端先投影。

**真实现状（已实证）：**
- `inboxApi.ts:3-14` InboxItem 接口：source/id/title/summary/severity/createdAt/ageHours/actionKind/richComponent?/richParams?，无富字段。
- EscalationInline 仅渲染 title/summary。

**Interfaces:**
- Consumes: 后端 Task 12 投影的 camelCase 字段。

- [ ] **Step 1: 写失败测试（扩充）**

在 `EscalationInline.test.tsx` 顶部 `item` fixture 加富字段，追加用例：

```ts
  it("富展示客户/问题/类别", () => {
    const richItem = {
      ...item,
      contactWxid: "wxid_cust",
      questionForPrincipal: "能否给折扣",
      category: "pricing",
    };
    const runAction = vi.fn();
    render(<EscalationInline item={richItem} ctx={{ busy: false, runAction }} />);
    expect(screen.getByText(/能否给折扣/)).toBeInTheDocument();
    expect(screen.getByText(/wxid_cust/)).toBeInTheDocument();
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`
Expected: FAIL —— 富字段未渲染。

- [ ] **Step 3: 改实现**

① `inboxApi.ts` InboxItem 接口加可选字段：

```ts
category?: string;
questionForPrincipal?: string;
contactWxid?: string;
principalWxid?: string;
```

② EscalationInline 在 title/summary 下加富信息区：客户（contactWxid）、具体问题（questionForPrincipal）、类别（category），仅在字段存在时渲染。

- [ ] **Step 4: 跑测试 + 全量**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human/inline/EscalationInline.test.tsx`
Expected: PASS。
Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/lib/inboxApi.ts frontend/src/features/ask-human/inline/EscalationInline.tsx frontend/src/__tests__/features/ask-human/inline/EscalationInline.test.tsx
git commit -m "$(cat <<'EOF'
feat(ask-human): 请示卡富展示客户/问题/类别(B2前端)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: E1（后端）— referral 撤销引荐 $unset 端点

**Files:**
- Modify: `src/routes/contacts.rs`（新增 clear_referral handler，仿 update_assist_override :562-590）
- Modify: `src/routes/mod.rs`（注册路由，contacts 子路由块 :313-354）
- Test: `tests/referral_clear.rs`（新建集成测）或 contacts.rs 单元测

**背景（红线）：** 设计 §6.3 承诺「已引荐」态可撤销，当前一旦引荐永久锁定被动答疑，无法让客户回主动运营。这是红线缺陷，本 task 最高验收权重。

**真实现状（已实证）：**
- $set 写入：`referral.rs:79-86 build_referred_set_doc` 写 `domain_attributes.referred_specialist_at` + `domain_attributes.referred_card_id`（dotted-key），update 调用 :166-174。
- 常量：`models.rs:3246 REFERRED_SPECIALIST_AT_ATTR="referred_specialist_at"`、`:3248 REFERRED_CARD_ID_ATTR="referred_card_id"`。
- 退辅助注入判定：`escalation/logic.rs:309-319` 判 `contact.domain_attributes.contains_key(REFERRED_SPECIALIST_AT_ATTR)` —— **判的是 specialist_at 键**。clear 须 $unset **两个**键才彻底退态。
- 范本 `update_assist_override`（contacts.rs:562-590）：签名 `(State<AppState>, Extension<AuthenticatedAdmin>, Path<String>, Json<AssistOverrideRequest>)`；$unset 写法见 :579；update filter 含 `workspace_id`（:587，workspace 隔离不可省）。

**Interfaces:**
- Produces: `POST /api/contacts/:id/clear-referral`，handler `clear_referral`。无 body（或空 body）。$unset 两键 + $set updated_at。

- [ ] **Step 1: 写失败测试**

新建 `tests/referral_clear.rs`（参照现有 tests/ contacts 相关集成测的 TestApp setup）。核心：
1. 造一个 contact，先 $set referred_specialist_at + referred_card_id（模拟已引荐）。
2. 调 `POST /api/contacts/:id/clear-referral`。
3. 断言 reload 后 contact.domain_attributes **不含** referred_specialist_at / referred_card_id。
4. 断言 escalation/logic 对该 contact **不再**注入退辅助指引（调 logic.rs 的判定函数或检查其输出）。

若集成测 setup 重（需 Docker），改为 contacts.rs 单元测验证 $unset doc 形态：

```rust
#[test]
fn clear_referral_unset_doc_drops_both_keys() {
    let doc = build_clear_referral_unset_doc(); // 新抽函数
    let unset = doc.get_document("$unset").unwrap();
    assert!(unset.contains_key("domain_attributes.referred_specialist_at"));
    assert!(unset.contains_key("domain_attributes.referred_card_id"));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib clear_referral 2>&1 | tail -20`
Expected: FAIL —— 函数 / 端点不存在。

- [ ] **Step 3: 改实现**

① contacts.rs 加 handler（仿 update_assist_override）：

```rust
pub async fn clear_referral(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let update = doc! {
        "$unset": {
            format!("domain_attributes.{}", REFERRED_SPECIALIST_AT_ATTR): "",
            format!("domain_attributes.{}", REFERRED_CARD_ID_ATTR): "",
        },
        "$set": { "updated_at": now_bson() },  // 用项目现有时间戳 helper
    };
    // update_one filter 含 workspace_id(admin.workspace_id) + _id/contact id,仿 :587
    // ...
    Ok(Json(json!({ "ok": true })))
}
```

（time helper、filter 形态、AppError 返回均仿 update_assist_override 的真实写法；REFERRED_*_ATTR 从 models.rs import。）

② mod.rs contacts 子路由块（:313-354）加：

```rust
.route("/contacts/:id/clear-referral", post(clear_referral))
```

- [ ] **Step 4: 跑测试 + 门**

Run: `cargo test --lib clear_referral 2>&1 | tail -20`
Expected: PASS。
Run: `RUSTFLAGS=-Dwarnings cargo check --tests 2>&1 | tail -20`
Expected: 0/0。
Run: `cargo test --lib 2>&1 | tail -5`
Expected: ≥350/0。

- [ ] **Step 5: 无人工接管 lint**

Run: `bash scripts/check-no-human-takeover.sh 2>&1 | tail -10`（或 ps1）
Expected: 通过——clear-referral / 撤销引荐 / 恢复主动运营 均非禁词。确认新增行无 `人工/接管/转人工`。

- [ ] **Step 6: 提交**

```bash
git add src/routes/contacts.rs src/routes/mod.rs tests/referral_clear.rs
git commit -m "$(cat <<'EOF'
feat(referral): 撤销引荐clear-referral端点,$unset两键退态(E1后端)

红线§6.3:已引荐态可撤销。$unset referred_specialist_at+referred_card_id,
让 escalation 不再注入退辅助指引,客户回主动运营。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: E1（前端）— 撤销引荐动作

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（联系人详情面板，assist_mode 区附近 :425-441）
- Modify: `frontend/src/stores/userOpsStore.ts`（加 clearReferral action）
- Test: `frontend/src/__tests__/stores/userOpsStore.test.ts`（扩充 Task 1 建的文件）

**背景：** 联系人详情无撤销引荐入口。依赖 Task 14 后端端点。

**真实现状（已实证）：**
- legacy.tsx 联系人详情：assist_mode_override 下拉（:425-441）、relationship_type 下拉（:443-463），均编辑型 select + 保存。无引荐状态展示/撤销。
- userOpsStore 有 saveRelationshipType（:479-499）等 action 范本。

**Interfaces:**
- Consumes: `api.post("/api/contacts/:id/clear-referral")`。
- Produces: `clearReferral(contactId: string)` store action。

- [ ] **Step 1: 写失败测试（扩充 userOpsStore.test.ts）**

```ts
  it("clearReferral 调用撤销引荐端点", async () => {
    (api as any).post = vi.fn().mockResolvedValue({ ok: true });
    await useUserOpsStore.getState().clearReferral("C1");
    expect((api as any).post).toHaveBeenCalledWith("/api/contacts/C1/clear-referral");
  });
```

（顶部 mock 的 api 需加 `post: vi.fn()`。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts`
Expected: FAIL —— clearReferral 不存在。

- [ ] **Step 3: 改实现**

① userOpsStore.ts 加 action（State 接口 + 实现）：

```ts
clearReferral: async (contactId: string) => {
  await api.post(`/api/contacts/${contactId}/clear-referral`);
},
```

② legacy.tsx 联系人详情 assist_mode 区附近加「撤销引荐 / 恢复主动运营」按钮，onClick 调 `clearReferral(contact.id)` 后刷新详情。文案不含禁词。

- [ ] **Step 4: 跑测试 + 全量**

Run: `cd frontend && npx vitest run src/__tests__/stores/userOpsStore.test.ts`
Expected: PASS。
Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/user-ops/legacy.tsx frontend/src/stores/userOpsStore.ts frontend/src/__tests__/stores/userOpsStore.test.ts
git commit -m "$(cat <<'EOF'
feat(user-ops): 撤销引荐/恢复主动运营动作(E1前端)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 16: E14 — 知识长任务派工创建入口

**Files:**
- Modify: `frontend/src/features/knowledge/today.tsx`（ChatWorkbench，加派工创建）
- Test: `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（扩充现有）

**背景：** 长任务队列从 UI 不可达，只能外部拿 taskId 手工粘贴跟踪。**TaskRail 跟踪 + cancel 已存在**（today.tsx:705/760），本 task 只补「派工创建」入口。

**真实现状（已实证）：**
- 后端 `POST /api/knowledge/chat/tasks`（mod.rs:665，handler `chat_task_create`）。body struct `ChatTaskCreateRequest`（knowledge/chat.rs:1858-1866）：`session_id: String`、`account_id?`、`operator_id?`、`card_ids: Vec<String>`（default）、`planned_steps: Vec<Value>`（default）。**snake_case 键。**
- ChatWorkbench（today.tsx:57）用裸 `fetch` + parseApiError（非 lib/api）；单轮 POST `/api/operation-knowledge/chat`（:173-177）。
- TaskRail（:705）已存在，GET `/api/knowledge/chat/tasks/{taskId}`（:743）+ POST cancel（:760）。
- grep plannedSteps / cardIds 0 命中。

**Interfaces:**
- Consumes: `fetch("/api/knowledge/chat/tasks", { method:"POST", body })`（沿用 ChatWorkbench 裸 fetch 形态）。
- Produces: 派工创建后把返回 taskId 交给现有 TaskRail 跟踪。

- [ ] **Step 1: 写失败测试（扩充 knowledge.test.tsx）**

mock `globalThis.fetch`，断言点「派工」时 POST `/api/knowledge/chat/tasks`，body 含 `session_id` + `planned_steps`：

```ts
  it("派工创建 POST chat/tasks 含 plannedSteps", async () => {
    const { fireEvent, waitFor, screen } = await import("@testing-library/react");
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ taskId: "T1" }) });
    (globalThis as any).fetch = fetchMock;
    // 渲染 ChatWorkbench(已有 sessionId),填写步骤,点「派工」
    // 断言:
    await waitFor(() => {
      const call = fetchMock.mock.calls.find((c: any[]) => String(c[0]).includes("/knowledge/chat/tasks"));
      expect(call).toBeTruthy();
      const body = JSON.parse(call[1].body);
      expect(body).toHaveProperty("session_id");
      expect(body).toHaveProperty("planned_steps");
    });
  });
```

（具体渲染入口按 knowledge.test.tsx 现有 ChatWorkbench mount 方式调整。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx`
Expected: FAIL —— 无「派工」入口。

- [ ] **Step 3: 改实现**

ChatWorkbench 加「派工长任务」区：一个多行输入（每行一个步骤 → `planned_steps`）+「派工」按钮，POST `/api/knowledge/chat/tasks`，body：

```ts
{
  session_id: sessionId,
  planned_steps: stepsText.split("\n").filter(Boolean).map((s) => ({ description: s })),
  card_ids: selectedCardIds ?? [],
}
```

成功后用返回的 taskId 触发现有 TaskRail 显示（沿用 today.tsx 既有 task 状态机制）。沿用裸 fetch + parseApiError 形态（与 ChatWorkbench 一致，非 lib/api）。

- [ ] **Step 4: 跑测试 + 全量**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx`
Expected: PASS。
Run: `cd frontend && npm run build && npm test`
Expected: 全绿。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/knowledge/today.tsx frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "$(cat <<'EOF'
feat(knowledge): 知识长任务派工创建入口(E14)

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review 结果

**Spec coverage（批次1 共 14 条 spec 条目 → 16 个 plan task）：**
- A1→T1 / C1→T2 / C5→T3 / C2→T4 / C3→T5 / C4→T6 / D1→T7 / D2→T8 / D3→T9 / B1→T10 / B3→T11 / B2→T12+T13（后端+前端）/ E1→T14+T15（后端+前端）/ E14→T16。全覆盖。

**待实施时核对（plan 标注的硬前置，非占位符——是「实证后落码」的诚实标记）：**
- T5/T6：EvolutionCenterTab 的 `apiGet` 真实 import 形态 + `api.put` 是否存在（Step 1 已列为动作）。
- T7：lib/api 是否有 `delete` 方法（Step 1）。
- T9：后端 generated_state_machine 真实 JSON 键名（Step 1，类型硬依赖）。
- T3/T4/T8：CSS module 变量名对齐 tokens.css。

这些是「读一个文件就能定」的局部确认，已写进各 task 的 Step 1，不是设计层空白。

**类型一致性：** B2 后端字段 snake_case（question_for_principal）↔ 序列化 camelCase（questionForPrincipal）↔ 前端接口 camelCase，三处已对齐。D2/B3/E14 的 snake_case body 键（mcp_api_key / to_wxid / session_id）已在各 task 显式标注，不与 camelCase 混淆。

**测试基线：** 后端 task（T12/T14）含 `cargo test --lib ≥350/0` + `RUSTFLAGS=-Dwarnings cargo check --tests 0/0` + 无人工接管 lint；前端每 task 含 `npm run build && npm test`。

