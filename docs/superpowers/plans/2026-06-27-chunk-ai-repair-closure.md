# chunk AI 修复落库闭环（F22 + F12-provenance）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `integrity_status = needs_review` 的知识切片接上 AI 修复闭环——把后端三个空转端点（propose/answer/applied）在前端兑现，运营能让 AI 提议 patch、多轮追问、逐字段勾选落库为 draft、闭账上报；顺带渲染 chunk provenance 来源。

**Architecture:** 纯前端。后端 propose/answer/applied 三端点 + 落库 PUT + verify 全部现成（零后端改动）。新增落库工具 `applyAiRepairPatch.ts`（拼 PUT body + 闭账，照 `useGoLive.ts` 的 `runGoLive` 样板返回 `{ok,reason}` 不抛错）+ 面板组件 `ChunkRepairPanel.tsx`（propose→reviewing→answer 多轮→落库状态机），挂在 `ChunkInspectorPane`（shared.tsx）内，needs_review chunk 才显入口。

**Tech Stack:** React 19 + TypeScript + Vite + CSS（knowledge 频道用 plain `.css` 非 module，避 tree-shake）；vitest + @testing-library/react；后端 Rust/Axum（仅消费现有 `src/routes/knowledge/repair.rs` 端点，不改）。

## Global Constraints

> 每个 task 隐含包含本节，逐条 verbatim 来自 spec 第六节。

- 子 agent 一律 `model:"opus"`；回复中文。
- **AI 永不自动 verify 红线**：落库 PUT 落 `status=draft + integrity_status=needs_review`（后端强制），面板**不传 verified**，`thenVerify` 恒传 false。运营放行另走现有 verify 按钮。
- **防清空**：PUT body 从 chunk 原值出发，只用勾选字段覆盖（复用批次3 E6 防清空模式，避免"表单驱动整 body 清空 rawContent"）。
- **serde 命门**：前端提交 wire 键 camelCase 对齐 `repair.rs` 的 `rename_all`：`sessionId`/`previousPatch`/`answers`(每项 `{id,field,text}`)/`turn`/`targetKind`/`targetId`/`acceptedFields`/`skippedFields`/`confidenceHint`/`extras`/`thenVerify`。
- **无人工接管 lint**：新增行（含注释/JSX 文案）禁词 `人工/接管/takeover/hand-off/转人工/人工介入/人工托管`。用业务语义："AI 修复建议"/"落库为草稿"/"去核验"。
- **设计系统**：knowledge 频道，复用 wiki* class（plain css 非 module）；蓝（主操作）仅"落库"按钮，AI 提议区可用紫系标 AI 身份，其余中性。读 `frontend/src/features/knowledge/Knowledge.css` 现有 wiki* class 后复用，补新 class 也落 Knowledge.css。
- **git**：只 `git add` 具名文件，绝不 `git add -A`（工作区有并行会话未提交 scripts/biz-test/* 等）；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`。
- **测试基线**：后端无改动，`cargo test --lib` 不回退；前端 `npx vitest run` 全绿 + `npx tsc --noEmit` 0。**本项目前端未配 eslint**，真类型门是 tsc，不跑 eslint。本地只跑前端测。

## 已实证修正（spec 表述与真实代码的差异，implementer 以此为准）

- **propose/answer 返回体**（repair.rs:365-377 实证）：`{ chunkId, sessionId, turn, promptKey, interpretation, patch, missingFields, followupQuestions, stillMissing, confidenceHint, budget }`。sessionId 在返回体里（非 header）。
- **BudgetExceeded 是 HTTP 错误不是 200+字段**：propose/answer 内 `generate_agent_json(...).await?` 失败（含超预算）走 `AppError` → 前端按**非 2xx** 处理（`!resp.ok`），不是解析 200 body 里的字段。错误横幅显友好提示即可。
- **ChunkProvenanceView 当前只定型 2 字段**（trustTypes.ts:154：`source`/`llmModelAlias`），后端 `ChunkProvenance`（models.rs:1472）有 6 字段（source/source_doc_id/source_quote/llm_model_alias/edited_at/edited_by）。Task 5 须把 type 补全到后端形态（camelCase：source/sourceDocId/sourceQuote/llmModelAlias/editedAt/editedBy）再渲染。
- **落库工具样板**：`frontend/src/features/knowledge/cockpit/useGoLive.ts` 的 `runGoLive`——已封装"PUT/verify + 4xx/5xx 分类 + fetch reject 归一为 server_error 不冒泡"。`applyAiRepairPatch` 照此返回 `{ok, reason, message}`，不抛错。

---

## Task 1: applyAiRepairPatch 落库工具

**Files:**
- Create: `frontend/src/lib/applyAiRepairPatch.ts`
- Test: `frontend/src/__tests__/lib/applyAiRepairPatch.test.ts`

**Interfaces:**
- Consumes: 现有 `PUT /api/operation-knowledge/chunks/:id`（落库）+ `POST /api/operation-knowledge/repair/applied`（闭账，RepairApplyBody camelCase）。
- Produces:
  ```ts
  export interface ApplyRepairInput {
    chunkId: string;
    originalChunk: Record<string, unknown>;  // 落库基线（防清空：从原值出发）
    patch: Record<string, unknown>;          // AI 提议的全部字段
    acceptedFieldNames: string[];            // 勾选接受的字段名
    sessionId: string;
    turn: number;
    confidenceHint: number;
    extras?: unknown;                        // patch.extras 透传闭账
  }
  export interface ApplyRepairResult {
    ok: boolean;
    reason?: "apply_failed" | "audit_failed" | "server_error";
    message?: string;
  }
  export function applyAiRepairPatch(input: ApplyRepairInput): Promise<ApplyRepairResult>;
  ```
  语义：`apply_failed`=PUT 落库失败（不发 applied）；`audit_failed`=落库成功但闭账失败（**ok 仍为 false 但 message 提示"已落库，审计写入失败"**，不回滚）；`server_error`=fetch reject。落库成功+闭账成功才 `ok:true`。

- [ ] **Step 1: 写失败的核心命门测**

`frontend/src/__tests__/lib/applyAiRepairPatch.test.ts`：

```ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { applyAiRepairPatch } from "../../lib/applyAiRepairPatch";

const okResp = (body: unknown = {}) => ({ ok: true, status: 200, json: async () => body }) as Response;
const errResp = (status: number) => ({ ok: false, status, json: async () => ({}) }) as Response;

let fetchSpy: ReturnType<typeof vi.fn>;
beforeEach(() => { fetchSpy = vi.fn(); vi.stubGlobal("fetch", fetchSpy); });
afterEach(() => { vi.unstubAllGlobals(); });

const BASE = {
  chunkId: "c1",
  originalChunk: { title: "原标题", body: "原body", summary: "原summary", sourceQuote: "原quote" },
  patch: { summary: "AI改的summary", sourceQuote: "AI脑补的quote", title: "AI改的标题", body: "AI改的body" },
  sessionId: "s1", turn: 1, confidenceHint: 70,
};

describe("applyAiRepairPatch", () => {
  it("勾选2/4字段：PUT body只含勾选字段覆盖+原值其余保留（防清空），applied分组正确", async () => {
    fetchSpy.mockResolvedValueOnce(okResp()).mockResolvedValueOnce(okResp());
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary", "title"] });
    expect(r.ok).toBe(true);
    // 第一个 fetch = PUT 落库
    const [putUrl, putInit] = fetchSpy.mock.calls[0];
    expect(putUrl).toContain("/api/operation-knowledge/chunks/c1");
    expect(putInit.method).toBe("PUT");
    const putBody = JSON.parse(putInit.body);
    expect(putBody.summary).toBe("AI改的summary");   // 勾选→用patch值
    expect(putBody.title).toBe("AI改的标题");          // 勾选→用patch值
    expect(putBody.body).toBe("原body");               // 没勾→保留原值（防清空）
    expect(putBody.sourceQuote).toBe("原quote");       // 没勾→保留原值（防清空，不被AI脑补覆盖）
    // 第二个 fetch = applied 闭账
    const [appliedUrl, appliedInit] = fetchSpy.mock.calls[1];
    expect(appliedUrl).toContain("/api/operation-knowledge/repair/applied");
    const aBody = JSON.parse(appliedInit.body);
    expect(aBody.targetKind).toBe("chunk");
    expect(aBody.targetId).toBe("c1");
    expect(aBody.sessionId).toBe("s1");
    expect(aBody.acceptedFields.sort()).toEqual(["summary", "title"]);
    expect(aBody.skippedFields.sort()).toEqual(["body", "sourceQuote"]); // patch有但没勾
    expect(aBody.thenVerify).toBe(false);              // 红线：恒false
    expect(aBody).not.toHaveProperty("then_verify");   // serde命门：camelCase
  });

  it("PUT落库失败：不发applied，返回apply_failed", async () => {
    fetchSpy.mockResolvedValueOnce(errResp(400));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("apply_failed");
    expect(fetchSpy).toHaveBeenCalledTimes(1); // 没发 applied
  });

  it("落库成功但闭账失败：返回audit_failed（不误报落库失败），message提示已落库", async () => {
    fetchSpy.mockResolvedValueOnce(okResp()).mockResolvedValueOnce(errResp(500));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("audit_failed");
    expect(r.message).toMatch(/已落库/);
  });

  it("fetch reject 归一为 server_error 不冒泡", async () => {
    fetchSpy.mockRejectedValueOnce(new Error("network"));
    const r = await applyAiRepairPatch({ ...BASE, acceptedFieldNames: ["summary"] });
    expect(r.ok).toBe(false);
    expect(r.reason).toBe("server_error");
  });
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/lib/applyAiRepairPatch.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现 applyAiRepairPatch**

`frontend/src/lib/applyAiRepairPatch.ts`：

```ts
// AI 修复 patch 落库 + 闭账。照 useGoLive.ts 的 runGoLive 形态返回 {ok,reason}，不抛错。
// 防清空：PUT body 从 originalChunk 出发，只用勾选字段覆盖。
// 红线：thenVerify 恒 false（落库只到 draft+needs_review，AI 永不自动 verify）。
export interface ApplyRepairInput {
  chunkId: string;
  originalChunk: Record<string, unknown>;
  patch: Record<string, unknown>;
  acceptedFieldNames: string[];
  sessionId: string;
  turn: number;
  confidenceHint: number;
  extras?: unknown;
}
export interface ApplyRepairResult {
  ok: boolean;
  reason?: "apply_failed" | "audit_failed" | "server_error";
  message?: string;
}

export async function applyAiRepairPatch(input: ApplyRepairInput): Promise<ApplyRepairResult> {
  const accepted = new Set(input.acceptedFieldNames);
  // 防清空：从原 chunk 值出发，只覆盖勾选字段。
  const putBody: Record<string, unknown> = { ...input.originalChunk };
  for (const name of input.acceptedFieldNames) {
    if (name in input.patch) putBody[name] = input.patch[name];
  }
  // skipped = patch 里有、但没勾选的字段名。
  const skippedFields = Object.keys(input.patch).filter((k) => k !== "extras" && !accepted.has(k));

  try {
    const putResp = await fetch(
      `/api/operation-knowledge/chunks/${encodeURIComponent(input.chunkId)}`,
      { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(putBody) },
    );
    if (!putResp.ok) return { ok: false, reason: "apply_failed" };

    const appliedResp = await fetch(
      `/api/operation-knowledge/repair/applied`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          targetKind: "chunk",
          targetId: input.chunkId,
          sessionId: input.sessionId,
          turn: input.turn,
          acceptedFields: input.acceptedFieldNames,
          skippedFields,
          confidenceHint: input.confidenceHint,
          extras: input.extras ?? null,
          thenVerify: false,
        }),
      },
    );
    if (!appliedResp.ok) {
      return { ok: false, reason: "audit_failed", message: "已落库为草稿，但审计记录写入失败" };
    }
    return { ok: true };
  } catch {
    return { ok: false, reason: "server_error" };
  }
}
```

- [ ] **Step 4: 跑测确认通过**

Run: `cd frontend && npx vitest run src/__tests__/lib/applyAiRepairPatch.test.ts`
Expected: PASS（4 用例）

- [ ] **Step 5: tsc + commit**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 错误

```bash
git add frontend/src/lib/applyAiRepairPatch.ts frontend/src/__tests__/lib/applyAiRepairPatch.test.ts
git commit -m "feat(knowledge): F22 applyAiRepairPatch 落库工具(防清空+逐字段分组+thenVerify恒false红线)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: ChunkRepairPanel propose + reviewing UI（含类型）

**Files:**
- Create: `frontend/src/features/knowledge/ChunkRepairPanel.tsx`
- Modify: `frontend/src/features/knowledge/trustTypes.ts`（新建 ChunkRepairProposal 等类型）
- Test: `frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`

**Interfaces:**
- Consumes: `POST /api/operation-knowledge/chunks/:id/repair`（propose，无 body，返回见下）。
- Produces:
  ```ts
  // trustTypes.ts 新建（前端无 repair 类型，今 today.tsx 的 missingFields:string[] 形态不同不可复用）
  export interface RepairMissingField { field: string; reason?: string | null; }
  export interface RepairFollowupQuestion { id: string; field?: string | null; question: string; }
  export interface ChunkRepairProposal {
    chunkId: string;
    sessionId: string;
    turn: number;
    interpretation: Record<string, unknown>;
    patch: Record<string, unknown>;
    missingFields: RepairMissingField[];
    followupQuestions: RepairFollowupQuestion[];
    stillMissing: RepairMissingField[];
    confidenceHint: number;
  }
  // ChunkRepairPanel 组件 props
  export function ChunkRepairPanel(props: {
    chunkId: string;
    originalChunk: Record<string, unknown>;
    onApplied: () => void;  // 落库成功回调（Task4 接入，本 task 先留空 noop 走通）
  }): JSX.Element;
  ```
- Produces（给 Task 3/4）：面板内 `proposal` 状态（ChunkRepairProposal | null）、`accepted` 勾选集（Set<string>）、`status`（"idle"|"proposing"|"reviewing"|"answering"|"applying"|"done"|"error"）。

- [ ] **Step 1: 写失败的组件测**

`frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ChunkRepairPanel } from "../../../features/knowledge/ChunkRepairPanel";

const PROPOSAL = {
  chunkId: "c1", sessionId: "s1", turn: 1,
  interpretation: { domain: "B2B SaaS", audience: "采购决策人" },
  patch: { summary: "AI改的summary", sourceQuote: "AI脑补quote" },
  missingFields: [{ field: "sourceQuote", reason: "原文无可核验出处" }],
  followupQuestions: [],
  stillMissing: [],
  confidenceHint: 65,
};
const okResp = (body: unknown) => ({ ok: true, status: 200, json: async () => body }) as Response;

let fetchSpy: ReturnType<typeof vi.fn>;
beforeEach(() => { fetchSpy = vi.fn(); vi.stubGlobal("fetch", fetchSpy); });
afterEach(() => { vi.unstubAllGlobals(); });

describe("ChunkRepairPanel", () => {
  it("点AI修复建议→propose→展示patch逐字段+复选框+confidenceHint", async () => {
    fetchSpy.mockResolvedValueOnce(okResp(PROPOSAL));
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{ summary: "原" }} onApplied={vi.fn()} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/AI改的summary/));
    // patch 两字段各一复选框
    expect(screen.getByText("summary")).toBeInTheDocument();
    expect(screen.getByText("sourceQuote")).toBeInTheDocument();
    expect(screen.getAllByRole("checkbox").length).toBe(2);
    // confidenceHint 展示
    expect(screen.getByText(/65/)).toBeInTheDocument();
    // propose 端点对
    expect(fetchSpy.mock.calls[0][0]).toContain("/api/operation-knowledge/chunks/c1/repair");
  });

  it("propose失败（含BudgetExceeded走非2xx）显错误横幅不崩", async () => {
    fetchSpy.mockResolvedValueOnce({ ok: false, status: 429, json: async () => ({}) } as Response);
    render(<ChunkRepairPanel chunkId="c1" originalChunk={{}} onApplied={vi.fn()} />);
    fireEvent.click(screen.getByText(/AI 修复建议/));
    await waitFor(() => screen.getByText(/修复.*失败|预算|重试/));
  });
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 补 trustTypes.ts 类型**

在 `trustTypes.ts` append（ChunkProvenanceView 之后）：上面 Interfaces 块的 `RepairMissingField` / `RepairFollowupQuestion` / `ChunkRepairProposal` 三个 interface（verbatim）。

- [ ] **Step 4: 实现 ChunkRepairPanel（propose + reviewing）**

`ChunkRepairPanel.tsx`。本 task 只做 idle→proposing→reviewing（answer 留 Task3、落库留 Task4）：

```tsx
import { useState } from "react";
import type { ChunkRepairProposal } from "./trustTypes";

type RepairStatus = "idle" | "proposing" | "reviewing" | "answering" | "applying" | "done" | "error";

export function ChunkRepairPanel({
  chunkId,
  originalChunk,
  onApplied,
}: {
  chunkId: string;
  originalChunk: Record<string, unknown>;
  onApplied: () => void;
}) {
  const [status, setStatus] = useState<RepairStatus>("idle");
  const [proposal, setProposal] = useState<ChunkRepairProposal | null>(null);
  const [accepted, setAccepted] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  // onApplied / originalChunk 在 Task4 落库时使用；此处先标记避免 tsc unused
  void onApplied; void originalChunk;

  async function propose() {
    setStatus("proposing");
    setError(null);
    try {
      const r = await fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/repair`, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: "{}",
      });
      if (!r.ok) {
        setError("AI 修复建议生成失败（可能预算用尽），请稍后重试");
        setStatus("error");
        return;
      }
      const data = (await r.json()) as ChunkRepairProposal;
      setProposal(data);
      setAccepted(new Set(Object.keys(data.patch ?? {}).filter((k) => k !== "extras"))); // 默认全勾，运营可取消
      setStatus("reviewing");
    } catch {
      setError("AI 修复建议生成失败，请稍后重试");
      setStatus("error");
    }
  }

  function toggleField(name: string) {
    setAccepted((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name); else next.add(name);
      return next;
    });
  }

  if (status === "idle") {
    return (
      <button type="button" className="wikiBtn" onClick={() => void propose()}>
        AI 修复建议
      </button>
    );
  }
  if (status === "proposing") return <div className="wikiHint">AI 正在分析这条切片…</div>;
  if (status === "error") return (
    <div className="wikiAlert error">
      {error}
      <button type="button" className="wikiBtn" onClick={() => void propose()}>重试</button>
    </div>
  );

  // reviewing（answer 区 Task3 补、落库按钮 Task4 补）
  const patchEntries = proposal ? Object.entries(proposal.patch ?? {}).filter(([k]) => k !== "extras") : [];
  return (
    <div className="wikiRepairPanel">
      {proposal?.interpretation ? (
        <div className="wikiRepairInterp">
          {Object.entries(proposal.interpretation).map(([k, v]) => (
            <span key={k} className="wikiArchiveTag">{k}: {String(v)}</span>
          ))}
        </div>
      ) : null}
      <div className="wikiRepairConfidence">AI 自评可信度：{proposal?.confidenceHint ?? 0}</div>
      <div className="wikiRepairFields">
        {patchEntries.map(([field, value]) => (
          <label key={field} className="wikiRepairField">
            <input type="checkbox" checked={accepted.has(field)} onChange={() => toggleField(field)} />
            <span className="wikiRepairFieldName">{field}</span>
            <span className="wikiRepairFieldValue">{typeof value === "string" ? value : JSON.stringify(value)}</span>
          </label>
        ))}
      </div>
      {proposal && proposal.missingFields.length > 0 ? (
        <div className="wikiRepairMissing">
          仍缺：{proposal.missingFields.map((m) => m.field).join("、")}
        </div>
      ) : null}
    </div>
  );
}
```

- [ ] **Step 5: 跑测确认通过 + tsc + commit**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx && npx tsc --noEmit`
Expected: PASS（2 用例）+ 0 错误

```bash
git add frontend/src/features/knowledge/ChunkRepairPanel.tsx frontend/src/features/knowledge/trustTypes.ts frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx
git commit -m "feat(knowledge): F22 ChunkRepairPanel propose+reviewing(patch逐字段复选+类型)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: answer 多轮追问

**Files:**
- Modify: `frontend/src/features/knowledge/ChunkRepairPanel.tsx`
- Test: `frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`（append）

**Interfaces:**
- Consumes: `POST /api/operation-knowledge/chunks/:id/repair/answer`（ChunkRepairAnswerBody camelCase：sessionId/previousPatch/answers[{id,field,text}]/turn，返回同 propose 形态）。
- Produces: 面板 reviewing 态在 followupQuestions 非空时显追问输入；提交后 answering→刷新 proposal。

- [ ] **Step 1: 写失败的 answer 多轮测**

append 到 `ChunkRepairPanel.test.tsx`：

```tsx
it("有followupQuestions→填答→answer→刷新patch", async () => {
  const withFollowup = { ...PROPOSAL, followupQuestions: [{ id: "q1", field: "sourceQuote", question: "原文哪段支持？" }] };
  const answered = { ...PROPOSAL, turn: 2, patch: { summary: "改进后summary", sourceQuote: "运营补充的quote" }, followupQuestions: [] };
  fetchSpy.mockResolvedValueOnce(okResp(withFollowup)).mockResolvedValueOnce(okResp(answered));
  render(<ChunkRepairPanel chunkId="c1" originalChunk={{}} onApplied={vi.fn()} />);
  fireEvent.click(screen.getByText(/AI 修复建议/));
  await waitFor(() => screen.getByText(/原文哪段支持/));
  // 填答
  fireEvent.change(screen.getByPlaceholderText(/回答/), { target: { value: "第三段" } });
  fireEvent.click(screen.getByText(/提交回答/));
  // answer 端点对 + 刷新 patch
  await waitFor(() => screen.getByText(/运营补充的quote/));
  const [answerUrl, answerInit] = fetchSpy.mock.calls[1];
  expect(answerUrl).toContain("/chunks/c1/repair/answer");
  const body = JSON.parse(answerInit.body);
  expect(body.sessionId).toBe("s1");
  expect(body.turn).toBe(1);
  expect(body.answers[0]).toMatchObject({ id: "q1", field: "sourceQuote", text: "第三段" });
  expect(body.previousPatch).toBeDefined();
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`
Expected: FAIL（无追问输入 / 提交回答按钮）

- [ ] **Step 3: 加 answer 状态与 UI**

在 ChunkRepairPanel 加 `answerDrafts` 状态（`Record<string,string>`，followup id→回答文本）+ `answer()` 函数：

```tsx
  const [answerDrafts, setAnswerDrafts] = useState<Record<string, string>>({});

  async function answer() {
    if (!proposal) return;
    setStatus("answering");
    setError(null);
    try {
      const answers = proposal.followupQuestions.map((q) => ({
        id: q.id, field: q.field ?? null, text: answerDrafts[q.id] ?? "",
      }));
      const r = await fetch(`/api/operation-knowledge/chunks/${encodeURIComponent(chunkId)}/repair/answer`, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          sessionId: proposal.sessionId, previousPatch: proposal.patch, answers, turn: proposal.turn,
        }),
      });
      if (!r.ok) { setError("追问应答失败，请稍后重试"); setStatus("error"); return; }
      const data = (await r.json()) as ChunkRepairProposal;
      setProposal(data);
      setAccepted(new Set(Object.keys(data.patch ?? {}).filter((k) => k !== "extras")));
      setAnswerDrafts({});
      setStatus("reviewing");
    } catch { setError("追问应答失败，请稍后重试"); setStatus("error"); }
  }
```

在 reviewing 渲染（missingFields 区之后）加追问块：

```tsx
      {proposal && proposal.followupQuestions.length > 0 ? (
        <div className="wikiRepairFollowup">
          {proposal.followupQuestions.map((q) => (
            <label key={q.id} className="wikiRepairFollowupItem">
              <span>{q.question}</span>
              <input
                type="text"
                placeholder="回答 AI 的追问（可留空）"
                value={answerDrafts[q.id] ?? ""}
                onChange={(e) => setAnswerDrafts((p) => ({ ...p, [q.id]: e.target.value }))}
              />
            </label>
          ))}
          <button type="button" className="wikiBtn" onClick={() => void answer()}>提交回答</button>
        </div>
      ) : null}
```

- [ ] **Step 4: 跑测确认通过 + tsc + commit**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx && npx tsc --noEmit`
Expected: PASS（3 用例）+ 0 错误

```bash
git add frontend/src/features/knowledge/ChunkRepairPanel.tsx frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx
git commit -m "feat(knowledge): F22 answer 多轮追问(followup填答→刷新patch,sessionId串审计链)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: 逐字段勾选落库 + 闭账

**Files:**
- Modify: `frontend/src/features/knowledge/ChunkRepairPanel.tsx`
- Test: `frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`（append）

**Interfaces:**
- Consumes: Task1 的 `applyAiRepairPatch`（`import { applyAiRepairPatch } from "../../lib/applyAiRepairPatch"`）。
- Produces: 面板 reviewing 态加"落库勾选字段"按钮→applying→done；done 显"已落库为草稿，可去核验"+调 onApplied。

- [ ] **Step 1: 写失败的落库测**

append：

```tsx
import { applyAiRepairPatch } from "../../../lib/applyAiRepairPatch";
vi.mock("../../../lib/applyAiRepairPatch", () => ({ applyAiRepairPatch: vi.fn() }));

it("落库：勾选字段调applyAiRepairPatch→done+onApplied", async () => {
  fetchSpy.mockResolvedValueOnce(okResp(PROPOSAL));
  (applyAiRepairPatch as ReturnType<typeof vi.fn>).mockResolvedValue({ ok: true });
  const onApplied = vi.fn();
  render(<ChunkRepairPanel chunkId="c1" originalChunk={{ summary: "原" }} onApplied={onApplied} />);
  fireEvent.click(screen.getByText(/AI 修复建议/));
  await waitFor(() => screen.getByText(/AI改的summary/));
  fireEvent.click(screen.getByText(/落库/));
  await waitFor(() => screen.getByText(/已落库/));
  expect(applyAiRepairPatch).toHaveBeenCalledWith(expect.objectContaining({
    chunkId: "c1", sessionId: "s1", turn: 1, confidenceHint: 65,
    acceptedFieldNames: expect.arrayContaining(["summary", "sourceQuote"]),
  }));
  expect(onApplied).toHaveBeenCalled();
});

it("落库失败显错误不调onApplied", async () => {
  fetchSpy.mockResolvedValueOnce(okResp(PROPOSAL));
  (applyAiRepairPatch as ReturnType<typeof vi.fn>).mockResolvedValue({ ok: false, reason: "apply_failed" });
  const onApplied = vi.fn();
  render(<ChunkRepairPanel chunkId="c1" originalChunk={{}} onApplied={onApplied} />);
  fireEvent.click(screen.getByText(/AI 修复建议/));
  await waitFor(() => screen.getByText(/AI改的summary/));
  fireEvent.click(screen.getByText(/落库/));
  await waitFor(() => screen.getByText(/落库失败|失败/));
  expect(onApplied).not.toHaveBeenCalled();
});
```

- [ ] **Step 2: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx`
Expected: FAIL（无落库按钮）

- [ ] **Step 3: 加落库逻辑**

import + `apply()` 函数：

```tsx
import { applyAiRepairPatch } from "../../lib/applyAiRepairPatch";
// ……
  async function apply() {
    if (!proposal) return;
    setStatus("applying");
    setError(null);
    const r = await applyAiRepairPatch({
      chunkId,
      originalChunk,
      patch: proposal.patch,
      acceptedFieldNames: [...accepted],
      sessionId: proposal.sessionId,
      turn: proposal.turn,
      confidenceHint: proposal.confidenceHint,
      extras: (proposal.patch as Record<string, unknown>).extras,
    });
    if (r.ok) {
      setStatus("done");
      onApplied();
    } else {
      setError(r.message ?? (r.reason === "apply_failed" ? "落库失败，请重试" : "操作失败，请重试"));
      setStatus("error");
    }
  }
```

reviewing 态加落库按钮（追问块之后）：

```tsx
      <button type="button" className="primary" disabled={accepted.size === 0} onClick={() => void apply()}>
        落库勾选字段（{accepted.size}）
      </button>
```

applying / done 态分支（放在 proposing 分支附近）：

```tsx
  if (status === "applying") return <div className="wikiHint">正在落库…</div>;
  if (status === "done") return <div className="wikiAlert ok">已落库为草稿，可在上方「确认放行」按钮去核验。</div>;
```

> 注意：`apply()` 内不再 try/catch（applyAiRepairPatch 已归一 fetch reject 为 server_error，不抛错）。

- [ ] **Step 4: 跑测确认通过 + tsc + commit**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx && npx tsc --noEmit`
Expected: PASS（5 用例）+ 0 错误

```bash
git add frontend/src/features/knowledge/ChunkRepairPanel.tsx frontend/src/__tests__/features/knowledge/ChunkRepairPanel.test.tsx
git commit -m "feat(knowledge): F22 逐字段勾选落库+闭账(调applyAiRepairPatch,done态提示去核验)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5: provenance 展示 + 接入 ChunkInspectorPane

**Files:**
- Modify: `frontend/src/features/knowledge/trustTypes.ts:154`（ChunkProvenanceView 补全 6 字段）
- Modify: `frontend/src/features/knowledge/shared.tsx`（ChunkInspectorPane :162-340 区：挂 ChunkRepairPanel + provenance 展示）
- Modify: `frontend/src/features/knowledge/Knowledge.css`（补 wikiRepair* + provenance class）
- Test: `frontend/src/__tests__/features/knowledge/knowledge.test.tsx`（append 或新建 chunkInspectorRepair 测）

**Interfaces:**
- Consumes: Task2-4 的 `<ChunkRepairPanel>`；`chunk.provenance`（补全后的 ChunkProvenanceView）。
- Produces: ChunkInspectorPane 在 needs_review chunk 显修复面板入口；provenance 非空显来源区。

- [ ] **Step 1: 补全 ChunkProvenanceView**

`trustTypes.ts:154` 改为（对齐后端 ChunkProvenance camelCase）：

```ts
export interface ChunkProvenanceView {
  source?: string;
  sourceDocId?: string | null;
  sourceQuote?: string | null;
  llmModelAlias?: string | null;
  editedAt?: string | null;
  editedBy?: string | null;
}
```

- [ ] **Step 2: 写失败的接入测**

append 到 `knowledge.test.tsx`（沿用既有 ChunkInspector 渲染套路；若无则新建 `chunkInspectorRepair.test.tsx`）：

```tsx
it("F22: needs_review chunk 显 AI 修复建议入口", async () => {
  // mock GET /chunks 返回一条 integrityStatus=needs_review 的 chunk，
  // 渲染 ChunkInspectorPane，断言出现「AI 修复建议」按钮
});
it("F12: chunk.provenance 非空时渲染来源区（source/editedBy）", async () => {
  // chunk.provenance = { source:"ai_repair", editedBy:"admin1", editedAt:"..." }
  // 断言来源文案渲染；provenance 为 null 时不崩、不显来源区
});
```

- [ ] **Step 3: 跑测确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/knowledge.test.tsx`
Expected: FAIL（无修复入口 / 无 provenance 区）

- [ ] **Step 4: ChunkInspectorPane 挂面板 + provenance**

`shared.tsx` ChunkInspectorPane chunk 详情渲染区（:316 integrityStatus 那块附近）：
- needs_review 时挂 `<ChunkRepairPanel chunkId={chunk.id} originalChunk={chunk as Record<string,unknown>} onApplied={() => { reload(); window.dispatchEvent(new CustomEvent("wikiChunkRevised", { detail: { chunk_id: chunk.id } })); }} />`（落库成功后刷新本面板 + 通知跨端）。
- provenance 区（dd 列表区加一项）：

```tsx
{chunk.provenance ? (
  <>
    <dt>来源</dt>
    <dd className="wikiProvenance">
      {chunk.provenance.source ? <span className="wikiArchiveTag">{chunk.provenance.source}</span> : null}
      {chunk.provenance.editedBy ? <span className="wikiProvBy">编辑者：{chunk.provenance.editedBy}</span> : null}
      {chunk.provenance.llmModelAlias ? <span className="wikiProvModel">{chunk.provenance.llmModelAlias}</span> : null}
    </dd>
  </>
) : null}
```

import 顶部加 `import { ChunkRepairPanel } from "./ChunkRepairPanel";`。确认 TreeChunkItem 类型含 provenance（来自 TrustChunkFields，已有）。

- [ ] **Step 5: 补 CSS**

`Knowledge.css` append `.wikiRepairPanel`/`.wikiRepairInterp`/`.wikiRepairConfidence`/`.wikiRepairFields`/`.wikiRepairField`/`.wikiRepairFieldName`/`.wikiRepairFieldValue`/`.wikiRepairMissing`/`.wikiRepairFollowup`/`.wikiRepairFollowupItem`/`.wikiProvenance`/`.wikiProvBy`/`.wikiProvModel`，走 tokens 变量；AI 提议区（wikiRepair*）可用紫系标 AI 身份，落库按钮 .primary 蓝，其余中性。读现有 wiki* class 配色后对齐。

- [ ] **Step 6: 全前端测 + tsc + 禁词 + commit**

Run: `cd frontend && npx vitest run && npx tsc --noEmit`
Run: `bash scripts/check-no-human-takeover.sh <BASE> HEAD`（BASE=本 task 起点 commit）
Expected: PASS + 0 错误 + 0 禁词

```bash
git add frontend/src/features/knowledge/trustTypes.ts frontend/src/features/knowledge/shared.tsx frontend/src/features/knowledge/Knowledge.css frontend/src/__tests__/features/knowledge/knowledge.test.tsx
git commit -m "feat(knowledge): F22 接入ChunkInspector修复入口+F12 provenance来源展示(补全6字段)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 完工后

全 5 task 完成后：
1. **whole-branch 终审**（最强模型，range = `git merge-base main HEAD`..HEAD）：重点查——applyAiRepairPatch 防清空（PUT 从原值出发只覆盖勾选）+ thenVerify 恒 false 红线 + serde camelCase 对齐 repair.rs（sessionId/previousPatch/answers/acceptedFields/skippedFields/thenVerify）+ 状态机闭合（propose→answer→落库→done 无死态）+ 闭账失败不回滚落库 + provenance 渲染 + 禁词。累积 Minor triage。
2. 终审有 Critical/Important → 派 ONE fix subagent（完整 findings）。
3. `superpowers:finishing-a-development-branch`：push + 创建 PR（**不自动合并**，除非用户确认）。本批纯前端，CI 按 paths-ignore 不触发后端 job（与批次1-4 同），靠本地 vitest+tsc 把关。

## 验收清单（浏览器人肉验收）

- F22：Explore 选中一条 needs_review chunk → 显「AI 修复建议」→ 点击出 patch 逐字段 + 可信度 → 勾选部分字段 → 落库 → chunk 刷新为 draft（仍 needs_review）。
- F22 多轮：propose 返回 followupQuestions 时可填答 → 提交 → patch 刷新。
- F22 红线：落库后 chunk 仍 needs_review（不自动 verified），需另点「确认放行」才核验。
- F12：chunk 详情显 provenance 来源（source/editedBy/model）。

## Self-Review

**1. Spec 覆盖**：spec 第四节 5 文件 → Task1（applyAiRepairPatch）/Task2（ChunkRepairPanel propose+类型）/Task3（answer）/Task4（落库闭账）/Task5（provenance+接入 ChunkInspector + Knowledge.css）。spec 第三节边界 4 条：多轮(Task3)/逐字段勾选(Task2 复选+Task4 落库)/不一键verify(Task1 thenVerify 恒false + Task4 done 提示去核验)/provenance(Task5)。spec 第六节约束 → Global Constraints。无遗漏。

**2. Placeholder 扫描**：各 step 含实际代码/命令。Task5 Step2 测试用注释骨架（沿用既有 ChunkInspector 测套路），属合理留白（implementer 读既有测试文件 mock 形态）。

**3. 类型一致性**：`ChunkRepairProposal`/`RepairMissingField`/`RepairFollowupQuestion`（Task2 定义）→ Task3/4 一致用。`ApplyRepairInput`/`applyAiRepairPatch`（Task1 定义）→ Task4 一致调（chunkId/originalChunk/patch/acceptedFieldNames/sessionId/turn/confidenceHint/extras 字段名对齐）。`ChunkProvenanceView`（Task5 补全）→ Task5 渲染一致用。

**4. 已实证修正**：propose/answer 返回体（repair.rs:365-377）、BudgetExceeded 走 HTTP 错误（非 200 字段）、ChunkProvenanceView 只有 2 字段需补全、useGoLive.ts 样板——均在"已实证修正"节列明，implementer 以此为准而非 spec 旧表述。
