# F-007 标签候选批量驳回 + 按类型筛选 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给标签候选审核页（`TaxonomyCandidatesAdmin`）加按类型（kind）服务端筛选下拉 + 待审候选的批量驳回（复选框 + 危险确认弹窗 + 前端循环调单条 reject 端点）。

**Architecture:** 改动全部集中在前端单组件 `frontend/src/features/system-strategy/index.tsx` 的 `TaxonomyCandidatesAdmin`。kind 筛选走后端已支持的 `?kind=` 参数；批量驳回复用现有 `POST /api/admin/taxonomy-candidates/:id/reject` 单条端点，前端循环调用；确认走系统现有 `useConfirm()`（`ConfirmProvider` 已包裹 `SystemStrategyFeature`）。零后端改动。

**Tech Stack:** React 19 + TypeScript + Vite + CSS Modules；测试 vitest + @testing-library/react。

## Global Constraints

- 语言/文案：UI 文案中文；标识符/英文技术项保留英文。
- no-human-takeover lint：`frontend/src/` 新增行不得含 `人工接管/takeover/hand-off/人工介入/人工托管/接管/人工` 等禁词。
- 过拟合红线：测试跟随真实行为，不为过测试改业务逻辑/阈值。测试只增量叠加，不改旧断言。
- 零后端改动：不新增后端端点、不改 `admin_taxonomy_candidates.rs`。
- 设计依据：`docs/superpowers/specs/2026-07-11-taxonomy-candidates-batch-filter-design.md`。
- 批量只做**驳回**，不做批量采纳（采纳需逐条人工填 canonicalValue，字典质量红线）。
- 批量驳回只在 `statusFilter === "pending"` 视图启用。

## 亲验的既有事实（实现者请信赖，均已 Read 确认）

- `TaxonomyCandidatesAdmin` 位于 `frontend/src/features/system-strategy/index.tsx`（约 :1031-1140）。
- 组件现有 state：`items` / `loading` / `error` / `statusFilter`（`CandidateStatusFilter`）+ `usePagedList(items)` 返回 `{ pageRows, pageCount, safePage, setPage }`。
- `reload()`（:1038）当前请求：`api.get<{items:TaxonomyCandidate[]}>(\`/api/admin/taxonomy-candidates?status=${encodeURIComponent(statusFilter)}\`)`。
- `useEffect`（:1053）依赖 `[statusFilter]` 触发 `reload()`。
- 单条审核卡 `TaxonomyCandidateReviewCard` 只在 `item.status === "pending"` 渲染（:1119）。
- `TAXONOMY_KIND_LABELS`（`components/review/TaxonomyCandidateReviewCard.tsx:9`）= `{ customer_stage:"客户阶段", intent_level:"意向强度", objection_type:"异议类型", concern_type:"顾虑类型", emotional_state:"情绪状态", relationship_type:"关系类型" }`，已被 index.tsx :10 import。
- 后端 `GET /api/admin/taxonomy-candidates` 已支持 `kind` query 过滤（`src/routes/admin_taxonomy_candidates.rs:101-103`）。
- 后端 `POST /api/admin/taxonomy-candidates/:id/reject` body `{ reason }`（非空），幂等只匹配 `status:"pending"`（同文件 :240-277）。
- `api.post<T>(url, body?)`（`lib/api.ts:58`）非 2xx 抛错。
- `useConfirm()`（`components/ui/ConfirmDialog/ConfirmDialog.tsx:93`）返回 `(opts: ConfirmOptions) => Promise<boolean>`；`ConfirmOptions = { title, body?, confirmText?, cancelText?, tone?, requireText? }`。弹窗真实渲染，确认按钮文案 = `confirmText ?? "确认"`。**body 是只读 ReactNode，不能内嵌回传值的输入框**——故驳回原因用组件内 inline 输入框，不放弹窗里。
- `SystemStrategyFeature`（default export，:2498）用 `<ConfirmProvider>` 包裹 `<SystemStrategyInner>`，故 `TaxonomyCandidatesAdmin` 可用 `useConfirm()`；测试渲染 `<SystemStrategyFeature />` 自带 provider。
- 测试文件 `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`：`api` 被 `vi.mock` 整体替换；`vi.spyOn(api,"get").mockImplementation((url)=>...)` 断言 URL；`api.post` 是 `vi.fn()`；`selectTab("标签与状态")` 切到候选面板 tab；候选 rawValue 落在 `<h3>`（`getByRole("heading",{name})`）；`makeCandidates(n)` 造 n 条 pending 候选（见 :416-432）。

---

## Task 1: kind 服务端筛选下拉

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx`（`TaxonomyCandidatesAdmin`，约 :1031-1140）
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（`describe("TaxonomyCandidatesAdmin 分页")` 块末尾追加）

**Interfaces:**
- Consumes: 既有 `TAXONOMY_KIND_LABELS`（已 import）、`statusFilter` state、`reload()`。
- Produces: 新增 state `kindFilter: string`（空串 = 全部）；`reload()` 请求 URL 在 `kindFilter` 非空时追加 `&kind=<encoded>`；kind 下拉的 `<select data-testid="candidate-kind-filter">`。

- [ ] **Step 1: 写失败测试（kind 下拉切换触发带 kind= 的请求）**

在 `systemStrategy.test.tsx` 的 `describe("TaxonomyCandidatesAdmin 分页", ...)` 块内、末尾 `});` 之前追加：

```tsx
  it("选择 kind 下拉后重新请求候选列表并带上 kind= 参数", async () => {
    const getSpy = vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(3) } : { items: [] }) as never,
      ),
    );

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");

    // 初次挂载：只带 status=pending，不带 kind=
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());
    expect(
      getSpy.mock.calls.some(
        ([u]) => typeof u === "string" && u.includes("/api/admin/taxonomy-candidates") && !u.includes("kind="),
      ),
    ).toBe(true);

    // 选 kind = objection_type（异议类型）
    const kindSelect = screen.getByTestId("candidate-kind-filter") as HTMLSelectElement;
    fireEvent.change(kindSelect, { target: { value: "objection_type" } });

    // 重新请求带上 kind=objection_type
    await waitFor(() =>
      expect(
        getSpy.mock.calls.some(
          ([u]) => typeof u === "string" && u.includes("/api/admin/taxonomy-candidates") && u.includes("kind=objection_type"),
        ),
      ).toBe(true),
    );
  });
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "选择 kind 下拉"`
Expected: FAIL（`candidate-kind-filter` testid 不存在 → `getByTestId` 抛错）

- [ ] **Step 3: 加 kindFilter state 与请求参数**

在 `TaxonomyCandidatesAdmin`（:1031）内，`statusFilter` state 声明（:1035）之后加：

```tsx
  const [kindFilter, setKindFilter] = useState<string>("");
```

把 `reload()`（:1038-1051）的请求 URL 构建改为：

```tsx
  async function reload() {
    setLoading(true);
    setError(null);
    try {
      let url = `/api/admin/taxonomy-candidates?status=${encodeURIComponent(statusFilter)}`;
      if (kindFilter) url += `&kind=${encodeURIComponent(kindFilter)}`;
      const data = await api.get<{ items: TaxonomyCandidate[] }>(url);
      setItems(data.items ?? []);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }
```

把 `useEffect`（:1053-1056）依赖数组从 `[statusFilter]` 改为 `[statusFilter, kindFilter]`：

```tsx
  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [statusFilter, kindFilter]);
```

- [ ] **Step 4: 加 kind 下拉到工具区**

在 status 筛选按钮 map（:1066-1075）之后、`刷新` 按钮（:1076）之前插入 kind 下拉：

```tsx
          <select
            data-testid="candidate-kind-filter"
            className={styles.profileTab}
            value={kindFilter}
            onChange={(e) => setKindFilter(e.target.value)}
          >
            <option value="">全部类型</option>
            {Object.entries(TAXONOMY_KIND_LABELS).map(([k, label]) => (
              <option key={k} value={k}>
                {label}
              </option>
            ))}
          </select>
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "选择 kind 下拉"`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "feat(taxonomy): 候选审核加按类型(kind)服务端筛选下拉(F-007)"
```

---

## Task 2: 批量驳回（复选框 + 原因输入 + 危险确认弹窗）

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx`（`TaxonomyCandidatesAdmin`）
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`

**Interfaces:**
- Consumes: Task 1 的 `kindFilter`/`statusFilter`/`reload()`；`useConfirm()`；`api.post`。
- Produces:
  - state `selectedIds: Set<string>`、`bulkReason: string`、`bulkBusy: boolean`、`bulkResult: string`。
  - `useConfirm` 引入：组件顶部 `const confirm = useConfirm();`（`useConfirm` 从 `../../components/ui/ConfirmDialog` import）。
  - 仅 `statusFilter === "pending"` 时：每条候选头部复选框 `<input type="checkbox" data-testid={\`candidate-check-${item.id}\`}>`；工具区批量条含原因输入 `data-testid="bulk-reject-reason"` + 按钮 `data-testid="bulk-reject-btn"`（`selectedIds.size===0 || !bulkReason.trim()` 时 disabled）。
  - 切换 `statusFilter`/`kindFilter`/翻页时清空 `selectedIds`。
  - `runBulkReject()`：确认后 `for (const id of selectedIds) await api.post(\`/api/admin/taxonomy-candidates/${id}/reject\`, { reason })`，失败计数不中断，末尾写 `bulkResult`，清空 `selectedIds` + `reload()`。

- [ ] **Step 1: 写失败测试（勾选 2 条 + 填原因 + 确认 → 2 次 reject POST + reload）**

在 `describe("TaxonomyCandidatesAdmin 分页", ...)` 块内追加：

```tsx
  it("批量驳回：勾选 2 条 pending + 填原因 + 确认 → 发 2 次 reject 请求", async () => {
    let getCalls = 0;
    vi.spyOn(api, "get").mockImplementation((url: string) => {
      if (url.includes("/api/admin/taxonomy-candidates")) getCalls += 1;
      return Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(3) } : { items: [] }) as never,
      );
    });
    const postSpy = vi.spyOn(api, "post").mockResolvedValue({} as never);

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());

    // 勾选 cand0、cand1
    fireEvent.click(screen.getByTestId("candidate-check-cand0"));
    fireEvent.click(screen.getByTestId("candidate-check-cand1"));

    // 填驳回原因
    fireEvent.change(screen.getByTestId("bulk-reject-reason"), { target: { value: "无业务相关性" } });

    // 点批量驳回 → 弹确认窗
    fireEvent.click(screen.getByTestId("bulk-reject-btn"));

    // 确认弹窗（useConfirm 渲染 confirmText="确认驳回"）
    fireEvent.click(await screen.findByText("确认驳回"));

    // 发出 2 次 reject POST，带 reason
    await waitFor(() => expect(postSpy).toHaveBeenCalledTimes(2));
    expect(postSpy).toHaveBeenCalledWith(
      "/api/admin/taxonomy-candidates/cand0/reject",
      { reason: "无业务相关性" },
    );
    expect(postSpy).toHaveBeenCalledWith(
      "/api/admin/taxonomy-candidates/cand1/reject",
      { reason: "无业务相关性" },
    );
  });

  it("批量驳回按钮：未勾选或未填原因时 disabled", async () => {
    vi.spyOn(api, "get").mockImplementation((url: string) =>
      Promise.resolve(
        (url.includes("/api/admin/taxonomy-candidates") ? { items: makeCandidates(3) } : { items: [] }) as never,
      ),
    );
    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());

    // 未勾选 → disabled
    expect(screen.getByTestId("bulk-reject-btn")).toBeDisabled();

    // 勾一条但没填原因 → 仍 disabled
    fireEvent.click(screen.getByTestId("candidate-check-cand0"));
    expect(screen.getByTestId("bulk-reject-btn")).toBeDisabled();

    // 填原因 → enabled
    fireEvent.change(screen.getByTestId("bulk-reject-reason"), { target: { value: "重复" } });
    expect(screen.getByTestId("bulk-reject-btn")).not.toBeDisabled();
  });
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "批量驳回"`
Expected: FAIL（`candidate-check-cand0` / `bulk-reject-reason` / `bulk-reject-btn` testid 不存在）

- [ ] **Step 3: import useConfirm + 加批量相关 state**

在 index.tsx 顶部已有 `import { ConfirmProvider } from "../../components/ui/ConfirmDialog";`（:11），改为同时引入 `useConfirm`：

```tsx
import { ConfirmProvider, useConfirm } from "../../components/ui/ConfirmDialog";
```

在 `TaxonomyCandidatesAdmin` 内（Task 1 的 `kindFilter` 之后）加：

```tsx
  const confirm = useConfirm();
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [bulkReason, setBulkReason] = useState("");
  const [bulkBusy, setBulkBusy] = useState(false);
  const [bulkResult, setBulkResult] = useState("");
```

- [ ] **Step 4: filter/翻页变化时清空选择**

把 Task 1 的 `useEffect` 依赖块扩展为切换时清空选择（在 `void reload()` 前清空）：

```tsx
  useEffect(() => {
    setSelectedIds(new Set());
    setBulkResult("");
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [statusFilter, kindFilter]);
```

翻页清空：把渲染里的 `<Pager ... setPage={setPage} />`（:1137）包一层清空选择的回调。改为：

```tsx
      <Pager
        pageCount={pageCount}
        safePage={safePage}
        setPage={(p) => {
          setSelectedIds(new Set());
          setPage(p);
        }}
      />
```

- [ ] **Step 5: 加 runBulkReject + toggle helper**

在 `reload()` 之后加两个函数：

```tsx
  function toggleSelect(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function runBulkReject() {
    const reason = bulkReason.trim();
    if (selectedIds.size === 0 || !reason) return;
    const ids = Array.from(selectedIds);
    const ok = await confirm({
      title: "批量驳回候选",
      body: `将驳回选中的 ${ids.length} 条候选，操作不可撤销。`,
      confirmText: "确认驳回",
      tone: "danger",
    });
    if (!ok) return;
    setBulkBusy(true);
    setBulkResult("");
    let done = 0;
    let failed = 0;
    for (const id of ids) {
      try {
        await api.post(`/api/admin/taxonomy-candidates/${id}/reject`, { reason });
        done += 1;
      } catch {
        failed += 1;
      }
    }
    setBulkBusy(false);
    setBulkResult(failed === 0 ? `已驳回 ${done} 条候选。` : `已驳回 ${done} 条，${failed} 条失败。`);
    setBulkReason("");
    setSelectedIds(new Set());
    void reload();
  }
```

- [ ] **Step 6: 渲染复选框 + 批量工具条**

在候选列表 `pageRows.map`（:1087）的每个 `versionedListHead`（:1089）里，`<div>`（含 scope/rawValue）之前加复选框（仅 pending 视图）。把 :1089-1097 的 head 结构改为：

```tsx
            <div className={styles.versionedListHead}>
              <div style={{ display: "flex", alignItems: "flex-start", gap: 10 }}>
                {statusFilter === "pending" && (
                  <input
                    type="checkbox"
                    data-testid={`candidate-check-${item.id}`}
                    checked={selectedIds.has(item.id)}
                    onChange={() => toggleSelect(item.id)}
                  />
                )}
                <div>
                  <span className={styles.versionedListScope}>
                    账号 {item.scope} · {labelOf(TAXONOMY_KIND_LABELS, item.kind)}
                  </span>
                  <h3>{item.rawValue}</h3>
                </div>
              </div>
              <span className={candidateStatusBadgeClass(item.status)}>{CANDIDATE_STATUS_LABEL[item.status as CandidateStatusFilter] ?? item.status}</span>
            </div>
```

在 `panelHint`（:1081-1083）之后、`{error && ...}`（:1084）之前插入批量工具条（仅 pending 视图）：

```tsx
      {statusFilter === "pending" && (
        <div className={styles.bulkBar} data-testid="bulk-reject-bar">
          <span className={styles.panelHint}>已选 {selectedIds.size} 条</span>
          <input
            className={styles.input}
            data-testid="bulk-reject-reason"
            placeholder="批量驳回原因（如：无业务相关性 / 与现有条目重复）"
            value={bulkReason}
            onChange={(e) => setBulkReason(e.target.value)}
            disabled={bulkBusy}
          />
          <button
            type="button"
            className={styles.btnGhost}
            data-testid="bulk-reject-btn"
            onClick={() => void runBulkReject()}
            disabled={bulkBusy || selectedIds.size === 0 || !bulkReason.trim()}
          >
            {bulkBusy ? "驳回中" : "批量驳回"}
          </button>
          {bulkResult && <span className={styles.panelHint}>{bulkResult}</span>}
        </div>
      )}
```

- [ ] **Step 7: 加 .bulkBar 样式**

在 `frontend/src/features/system-strategy/SystemStrategy.module.css` 找到 `.buttonRow` 或同类布局类附近，追加：

```css
.bulkBar {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
```

（已亲验 `.input`(:97) / `.btnGhost`(:133) / `.panelHint`(:26) / `.buttonRow`(:122) 均存在于该文件，直接复用；本 task 只新增 `.bulkBar`。）

- [ ] **Step 8: 运行测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "批量驳回"`
Expected: PASS（2 个用例）

- [ ] **Step 9: 提交**

```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/features/system-strategy/SystemStrategy.module.css frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "feat(taxonomy): 待审候选批量驳回(复选框+原因+危险确认弹窗,前端循环调单条端点)(F-007)"
```

---

## Task 3: 非 pending 视图不出现批量入口 + 全量回归

**Files:**
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`

**Interfaces:**
- Consumes: Task 2 的复选框/工具条（仅 pending 渲染）。
- Produces: 无新代码，仅补边界测试 + 跑全套门。

- [ ] **Step 1: 写测试（切到 approved 视图不出现复选框/批量入口）**

在 `describe("TaxonomyCandidatesAdmin 分页", ...)` 块内追加：

```tsx
  it("非 pending 视图（已采纳）不渲染复选框与批量驳回入口", async () => {
    vi.spyOn(api, "get").mockImplementation((url: string) => {
      if (url.includes("/api/admin/taxonomy-candidates")) {
        // approved filter：返回 1 条 approved 候选
        const items = makeCandidates(1).map((c) => ({ ...c, status: "approved" }));
        return Promise.resolve({ items } as never);
      }
      return Promise.resolve({ items: [] } as never);
    });

    render(<SystemStrategyFeature />);
    selectTab("标签与状态");
    await waitFor(() => expect(screen.getByRole("heading", { name: "候选词0" })).toBeInTheDocument());

    // 切到「已采纳」status filter
    fireEvent.click(screen.getByRole("button", { name: "已采纳" }));

    await waitFor(() => {
      expect(screen.queryByTestId("bulk-reject-bar")).toBeNull();
    });
    expect(screen.queryByTestId("candidate-check-cand0")).toBeNull();
  });
```

- [ ] **Step 2: 运行该测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "非 pending"`
Expected: PASS（Task 2 的 `statusFilter === "pending"` 守卫已保证）

- [ ] **Step 3: 跑候选面板整组测试**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx`
Expected: PASS（含既有分页用例 + 本批新增用例，无回归）

- [ ] **Step 4: tsc + 前端全量测试（前端契约门本地复刻）**

Run: `cd frontend && npx tsc --noEmit && npx vitest run`
Expected: tsc 0 error；全量 vitest 全 PASS

- [ ] **Step 5: no-human-takeover lint 自查**

Run: `git diff origin/main -- frontend/src/ | grep -nE '^\+' | grep -inE 'human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工' || echo "无禁词命中"`
Expected: `无禁词命中`

- [ ] **Step 6: 提交**

```bash
git add frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "test(taxonomy): 非 pending 视图无批量入口边界用例(F-007)"
```

---

## Self-Review 结论

- **Spec coverage**：kind 服务端筛选（Task 1）✓；批量驳回复选框+原因+确认弹窗+循环调单条端点（Task 2）✓；只 pending 启用（Task 2 守卫 + Task 3 边界测试）✓；切 filter/翻页清空选择（Task 2 Step 4）✓；测试增量叠加（三 Task 全部 append）✓；不做批量采纳/不加后端端点/不改分页机制（全程未触碰）✓。
- **Placeholder scan**：无 TBD/TODO；每个代码步骤含完整代码与真实 testid。
- **Type consistency**：`kindFilter:string`、`selectedIds:Set<string>`、`bulkReason:string`、`confirm(opts):Promise<boolean>`、`api.post(url,{reason})` 全对齐亲验签名；testid（`candidate-kind-filter`/`candidate-check-${id}`/`bulk-reject-reason`/`bulk-reject-btn`/`bulk-reject-bar`）跨 Task 一致。
- CSS 类名已全部亲验存在（`.input`/`.btnGhost`/`.panelHint`/`.buttonRow`），Task 2 只新增 `.bulkBar`，无实现期不确定点。
