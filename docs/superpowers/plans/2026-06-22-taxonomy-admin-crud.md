# 字典条目运营编辑（taxonomy admin CRUD 前端）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给系统策略频道「双层标签字典灰度」面板（`TaxonomiesAdmin`）补运营主动新增 / 编辑 / 废弃·恢复字典条目的 inline 表单，接已就绪的后端 `POST/PATCH/DELETE /api/admin/taxonomies`。

**Architecture:** 纯前端，方案 A——扩展现有 `frontend/src/features/system-strategy/index.tsx` 的 `TaxonomiesAdmin` 函数（现 590-688），复用同文件 `TaxonomyCandidatesAdmin` approve 表单（925-967）的 inline 展开 + draft state + 逗号别名解析模式与现有 `.module.css` 类名。先补一个 `api.patch` 基础方法（现 `api` 无 PATCH，而后端编辑/恢复走 PATCH）。

**Tech Stack:** React 19 + TypeScript + Vite + vitest。设计文档：`docs/superpowers/specs/2026-06-22-taxonomy-admin-crud-design.md`。

## Global Constraints

- **纯前端**：零后端改动（CRUD 端点 `admin_taxonomies.rs` 已就绪）。
- **复用不新造**：复用同文件 approve 表单的 inline 模式 + 现有 `.module.css` 类名（`styles.form/field/input/textarea/buttonRow/btnPrimary/btnGhost/inlineError/badgeOk/panelHint/versionedListItem/versionedListHead/versionedListScope`）。**零新样式、零硬编码颜色、不抽子组件、不引入模态弹窗**。
- **前端校验不超过后端（D5）**：id 不前端硬校验格式（仅 placeholder 引导）；**不暴露 `priority_weight` / `is_terminal`**（后端无 create 入参）。
- **废弃语义（D3）**：DELETE 是软删（`status→deprecated`），UI 叫「废弃」；PATCH `{deprecated:false}` 做「恢复」。按 `item.value.status` 切换按钮文案。
- **状态机软提示（D2）**：新增 `kind==="customer_stage"` 时表单内显示一行提示指向状态机面板，**不阻断保存**。
- **409 当 info 不当 error**：新增重复 `(scope,kind,value.id)` 后端返 409，复用 approve 先例用 `api.postRaw` 拿 status，显示 info 而非 error。
- **no-human-takeover 文案**：新增文案走「标签 / 字典 / 阶段 / 废弃 / 恢复 / 业务主题」中性词，禁止 `人工/接管/takeover/hand-off/人工介入` 等词。
- **测试只 append**：加到 `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`，不删既有用例。
- **验证**：`cd frontend && npm run build`（TS 零错误）+ `npm run test`（vitest 全绿）。本地可跑前端。
- **后端契约（已读 `admin_taxonomies.rs` 核实）**：
  - `POST /api/admin/taxonomies` body=`{scope, kind, value:{id, label, aliases:[...], description?}}`；成功返 `{item}`；重复 `(scope,kind,value.id)` 返 **409** `{error:"duplicate_taxonomy", message}`。
  - `PATCH /api/admin/taxonomies/:id` body=`{label?, aliases?, description?, deprecated?}`（至少一个）；成功返 `{item}`；空 label 返 400；id 不存在返 404。
  - `DELETE /api/admin/taxonomies/:id` 软删；成功返 `{ok:true}`；404 not found。
- **前端类型（已存在，`index.tsx:37-48`）**：`TaxonomyEntry` 含 `id/scope/kind/value:{id,label,displayName?,description?,aliases?,status}`。

---

### Task 1: 补 `api.patch` 基础方法

**Files:**
- Modify: `frontend/src/lib/api.ts:67-75`（在 `put` 方法后加 `patch`）
- Test: `frontend/src/__tests__/lib/apiPatch.test.ts`（新建）

**Interfaces:**
- Produces: `api.patch<T>(url: string, body: unknown): Promise<T>` — 发 PATCH 请求，`Content-Type: application/json`，非 2xx 抛 `parseApiError`，2xx 返 `response.json()`。Task 3 的编辑/恢复用它。

**背景**：现有 `api`（`api.ts:52-106`）有 get/post/put/delete/postForm/postRaw，**没有 patch**。后端字典编辑/恢复端点是 `PATCH /api/admin/taxonomies/:id`（`routes/mod.rs:783` `patch(patch_taxonomy)`），所以必须先补一个 `api.patch`，签名/实现镜像现有 `put`（`api.ts:67-75`）。

- [ ] **Step 1: 写失败测试**

新建 `frontend/src/__tests__/lib/apiPatch.test.ts`：

```ts
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../lib/api";

describe("api.patch", () => {
  afterEach(() => vi.restoreAllMocks());

  it("发 PATCH 请求，带 JSON body 和 Content-Type，返回解析后的 JSON", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ item: { id: "x" } }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const result = await api.patch<{ item: { id: string } }>("/api/admin/taxonomies/abc", {
      label: "新名",
    });

    expect(fetchMock).toHaveBeenCalledWith("/api/admin/taxonomies/abc", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ label: "新名" }),
    });
    expect(result).toEqual({ item: { id: "x" } });
  });

  it("非 2xx 抛错", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 400,
        headers: { get: () => "application/json" },
        text: async () => JSON.stringify({ error: "label 不能为空" }),
      })
    );
    await expect(api.patch("/api/admin/taxonomies/abc", {})).rejects.toThrow("label 不能为空");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/lib/apiPatch.test.ts`
Expected: FAIL（`api.patch is not a function`）。

- [ ] **Step 3: 实现 `api.patch`**

`frontend/src/lib/api.ts`，在 `put` 方法（:67-75）后、`delete` 前插入：

```ts
  async patch<T>(url: string, body: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    if (!response.ok) throw await parseApiError(response);
    return response.json();
  },
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/lib/apiPatch.test.ts`
Expected: 2 PASS。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/lib/api.ts frontend/src/__tests__/lib/apiPatch.test.ts
git commit -m "feat(taxonomy-admin): api 加 patch 方法(镜像 put,接后端 PATCH 字典编辑端点)"
```

---

### Task 2: `TaxonomiesAdmin` 新增条目表单（create）

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx`（`TaxonomiesAdmin` 函数，现 590-688）
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（append）

**Interfaces:**
- Consumes: `api.postRaw`（`api.ts:89`，已存在）。
- Produces: 面板顶部「新增条目」展开表单，提交 POST `/api/admin/taxonomies`。Task 3 在同函数加编辑/废弃。

**背景**：`TaxonomiesAdmin`（590-688）目前只读。本 task 加新增能力。复用同文件 `TaxonomyCandidatesAdmin` approve 表单（925-967）的结构：`styles.form` 容器 + `styles.field`/`styles.input`/`styles.textarea` 字段 + `styles.buttonRow` + `styles.btnPrimary`/`styles.btnGhost`，逗号别名解析 `split(/[,，]/).map(trim).filter(Boolean)`（见 `submitApprove` :796-799）。

- [ ] **Step 1: 加 draft 类型 + state**

在 `TaxonomiesAdmin` 函数体顶部（现有 `const [items...]` 等 state 旁，:592-596）加：

```ts
  const [showCreate, setShowCreate] = useState(false);
  const [createDraft, setCreateDraft] = useState<TaxonomyDraft>({
    scope: "global",
    kind: "customer_stage",
    id: "",
    label: "",
    aliases: "",
    description: "",
  });
  const [acting, setActing] = useState(false);
  const [info, setInfo] = useState<string | null>(null);
```

在文件内 `type TaxonomyEntry` 定义（:37-48）后加类型（与 `ApproveDraft` :706-711 同为本地类型，不进 types/index.ts）：

```ts
type TaxonomyDraft = {
  scope: string;
  kind: string;
  id: string;
  label: string;
  aliases: string;
  description: string;
};
```

- [ ] **Step 2: 写失败测试**

append 到 `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（先读文件头部看现有 import / mock 风格，对齐 `api` mock 与 render 方式）。本测试断言新增提交形态：

```ts
describe("TaxonomiesAdmin 新增条目", () => {
  it("新增提交 POST /api/admin/taxonomies，body 形态正确，别名中英文逗号都 split", async () => {
    const postRaw = vi.spyOn(api, "postRaw").mockResolvedValue({ ok: true, status: 200, data: { item: {} } });
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);

    render(<SystemStrategyView /* 与现有用例同款 props */ />);

    // 切到 taxonomies 面板（与现有用例切面板方式一致）+ 点「新增条目」
    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "need_discovery" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.change(screen.getByPlaceholderText(/别名/i), { target: { value: "挖需求，需求探索, 探需" } });
    fireEvent.click(screen.getByText("保存"));

    await waitFor(() => expect(postRaw).toHaveBeenCalled());
    expect(postRaw).toHaveBeenCalledWith("/api/admin/taxonomies", {
      scope: "global",
      kind: "customer_stage",
      value: { id: "need_discovery", label: "需求挖掘", aliases: ["挖需求", "需求探索", "探需"], description: undefined },
    });
  });
});
```

> 实现者：测试的「切面板 / render」方式必须对齐文件内现有 `systemStrategy.test.tsx` 用例（先读现有测试看 SystemStrategyView 怎么 render、面板怎么切换、`screen.getByText` 用什么文案）。上面的选择器（placeholder 文案）以 Step 3 实际写的 placeholder 为准——两边对齐。

- [ ] **Step 3: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "新增条目"`
Expected: FAIL（找不到「新增条目」按钮）。

- [ ] **Step 4: 实现新增按钮 + 表单**

在 `TaxonomiesAdmin` 的 panelHead buttonRow（现「刷新」按钮旁，:639-641）加「新增条目」按钮：

```tsx
          <button type="button" className={styles.btnGhost} onClick={() => { setShowCreate((v) => !v); setInfo(null); setError(null); }} disabled={busy || loading}>
            {showCreate ? "收起新增" : "新增条目"}
          </button>
```

在 panelHead `</div>` 后、`{error && ...}` 前（:643 附近）加 info 行 + 新增表单：

```tsx
      {info && <div className={styles.badgeOk} style={{ display: "inline-block", marginBottom: 8 }}>{info}</div>}
      {showCreate && (
        <div className={styles.form} style={{ marginBottom: 14 }}>
          <label className={styles.field}>
            <span>scope（global = 全局，填 accountId = 仅该账号）</span>
            <input className={styles.input} placeholder="global" value={createDraft.scope}
              onChange={(e) => setCreateDraft({ ...createDraft, scope: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>kind（维度，如 customer_stage / intent_level / objection_type）</span>
            <input className={styles.input} placeholder="customer_stage" value={createDraft.kind}
              onChange={(e) => setCreateDraft({ ...createDraft, kind: e.target.value })} />
          </label>
          {createDraft.kind.trim() === "customer_stage" && (
            <p className={styles.panelHint}>
              新增客户阶段后，需到上方「状态机灰度」面板同步配置对应 state，否则该阶段的 operation_state 流转校验会被跳过。
            </p>
          )}
          <label className={styles.field}>
            <span>canonical id（建议英文 snake_case，如 need_discovery）</span>
            <input className={styles.input} placeholder="canonical id（如 need_discovery）" value={createDraft.id}
              onChange={(e) => setCreateDraft({ ...createDraft, id: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>显示名</span>
            <input className={styles.input} placeholder="显示名（如 需求挖掘）" value={createDraft.label}
              onChange={(e) => setCreateDraft({ ...createDraft, label: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>别名（逗号分隔，可空）</span>
            <input className={styles.input} placeholder="别名（逗号分隔，可空）" value={createDraft.aliases}
              onChange={(e) => setCreateDraft({ ...createDraft, aliases: e.target.value })} />
          </label>
          <label className={styles.field}>
            <span>描述（可空）</span>
            <textarea className={styles.textarea} value={createDraft.description}
              onChange={(e) => setCreateDraft({ ...createDraft, description: e.target.value })} />
          </label>
          <div className={styles.buttonRow}>
            <button type="button" className={styles.btnPrimary} onClick={() => void submitCreate()} disabled={acting}>保存</button>
            <button type="button" className={styles.btnGhost} onClick={() => setShowCreate(false)} disabled={acting}>取消</button>
          </div>
        </div>
      )}
```

在 `reload` 函数后加 `submitCreate`（校验不超后端：scope/kind/id/label 必填，description 可空）：

```ts
  async function submitCreate() {
    if (!createDraft.scope.trim() || !createDraft.kind.trim() || !createDraft.id.trim() || !createDraft.label.trim()) {
      setError("scope / kind / canonical id / 显示名 均不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliases = createDraft.aliases.split(/[,，]/).map((a) => a.trim()).filter((a) => a.length > 0);
      const res = await api.postRaw<{ error?: string; message?: string }>("/api/admin/taxonomies", {
        scope: createDraft.scope.trim(),
        kind: createDraft.kind.trim(),
        value: {
          id: createDraft.id.trim(),
          label: createDraft.label.trim(),
          aliases,
          description: createDraft.description.trim() || undefined,
        },
      });
      if (res.status === 409) {
        setInfo(res.data?.message ?? "该字典条目已存在。");
      } else if (!res.ok) {
        setError(res.data?.message ?? res.data?.error ?? `HTTP ${res.status}`);
        return;
      } else {
        setInfo(`已新增：${createDraft.id.trim()}`);
        setShowCreate(false);
        setCreateDraft({ scope: "global", kind: "customer_stage", id: "", label: "", aliases: "", description: "" });
      }
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }
```

- [ ] **Step 5: 跑测试确认通过 + build**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "新增条目" && npm run build`
Expected: 测试 PASS；build 零 TS 错误。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "feat(taxonomy-admin): 字典面板加新增条目表单(409当info+状态机软提示+别名逗号解析)"
```

---

### Task 3: 行内编辑 + 废弃/恢复（patch / delete）

**Files:**
- Modify: `frontend/src/features/system-strategy/index.tsx`（`TaxonomiesAdmin` 函数，Task 2 已扩展过）
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（append）

**Interfaces:**
- Consumes: `api.patch`（Task 1 新增）、`api.delete`（`api.ts:76`，已存在）、Task 2 的 `acting`/`info`/`reload`。
- Produces: 每个条目行的「编辑」inline 表单（PATCH label/aliases/description）+「废弃」/「恢复」按钮。

**背景**：在 Task 2 扩展过的 `TaxonomiesAdmin` 里，给每个 `versionedListItem` 加行内编辑与废弃/恢复。编辑只改 label/aliases/description（**不含 id/scope/kind**，主键不可改，对应后端 PATCH 只接这几个字段）。废弃/恢复按 `item.value.status` 切换文案（D3）。一次只展开一个编辑行（`editingId` 单值），且与新增表单互斥（打开编辑时 `setShowCreate(false)`）。

- [ ] **Step 1: 加编辑 state + draft 类型**

在 `TaxonomiesAdmin` 的 state 区（Task 2 加的 state 旁）加：

```ts
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState<EditDraft>({ label: "", aliases: "", description: "" });
```

在 `TaxonomyDraft` 类型（Task 2 加的）旁加：

```ts
type EditDraft = { label: string; aliases: string; description: string };
```

- [ ] **Step 2: 写失败测试**

append 到 `systemStrategy.test.tsx`（seed 一条 active + 一条 deprecated 条目）：

```ts
describe("TaxonomiesAdmin 编辑与废弃恢复", () => {
  const activeItem = {
    id: "id_active", scope: "global", kind: "customer_stage",
    value: { id: "need_discovery", label: "需求挖掘", aliases: ["挖需求"], description: "", status: "active" },
    version: 1, currentVersion: true, previousVersion: null, seededBy: "manual", updatedAt: "",
  };
  const deprecatedItem = {
    id: "id_dep", scope: "global", kind: "intent_level",
    value: { id: "low", label: "低意向", aliases: [], description: "", status: "deprecated" },
    version: 1, currentVersion: true, previousVersion: null, seededBy: "manual", updatedAt: "",
  };

  it("编辑提交 PATCH /:id，body 仅 label/aliases/description（无 id/scope/kind）", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [activeItem] } as never);
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item: activeItem } as never);
    render(<SystemStrategyView /* 同款 */ />);
    fireEvent.click(await screen.findByText("编辑"));
    fireEvent.change(screen.getByDisplayValue("需求挖掘"), { target: { value: "需求探索阶段" } });
    fireEvent.click(screen.getByText("保存编辑"));
    await waitFor(() => expect(patch).toHaveBeenCalled());
    expect(patch).toHaveBeenCalledWith("/api/admin/taxonomies/id_active", {
      label: "需求探索阶段", aliases: ["挖需求"], description: "",
    });
  });

  it("active 条目显示「废弃」，点击调 api.delete", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [activeItem] } as never);
    const del = vi.spyOn(api, "delete").mockResolvedValue({ ok: true } as never);
    render(<SystemStrategyView /* 同款 */ />);
    fireEvent.click(await screen.findByText("废弃"));
    await waitFor(() => expect(del).toHaveBeenCalledWith("/api/admin/taxonomies/id_active"));
  });

  it("deprecated 条目显示「恢复」，点击调 api.patch {deprecated:false}", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [deprecatedItem] } as never);
    const patch = vi.spyOn(api, "patch").mockResolvedValue({ item: deprecatedItem } as never);
    render(<SystemStrategyView /* 同款 */ />);
    // deprecated 条目需勾「显示已废弃」才可见（现有 includeDeprecated checkbox）
    fireEvent.click(screen.getByText("显示已废弃"));
    fireEvent.click(await screen.findByText("恢复"));
    await waitFor(() => expect(patch).toHaveBeenCalledWith("/api/admin/taxonomies/id_dep", { deprecated: false }));
  });
});
```

> 实现者：render / 切面板 / 勾选「显示已废弃」的方式对齐文件内现有用例。上面选择器文案以 Step 4 实际实现为准（两边对齐）。

- [ ] **Step 3: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "编辑与废弃恢复"`
Expected: FAIL（找不到「编辑」按钮）。

- [ ] **Step 4: 实现编辑/废弃/恢复**

在条目行 map 里、`versionedListBody` 的 `</div>` 之后（现 :683 附近）加操作按钮区与编辑表单：

```tsx
            {editingId !== item.id && (
              <div className={styles.buttonRow}>
                <button type="button" className={styles.btnGhost}
                  onClick={() => { setShowCreate(false); setEditingId(item.id); setEditDraft({ label: item.value.label, aliases: (item.value.aliases ?? []).join("，"), description: item.value.description ?? "" }); setInfo(null); setError(null); }}
                  disabled={busy || acting}>编辑</button>
                {item.value.status === "active" ? (
                  <button type="button" className={styles.btnGhost} onClick={() => void deprecateEntry(item.id)} disabled={busy || acting}>废弃</button>
                ) : (
                  <button type="button" className={styles.btnGhost} onClick={() => void restoreEntry(item.id)} disabled={busy || acting}>恢复</button>
                )}
              </div>
            )}
            {editingId === item.id && (
              <div className={styles.form} style={{ marginTop: 12 }}>
                <label className={styles.field}>
                  <span>显示名</span>
                  <input className={styles.input} value={editDraft.label}
                    onChange={(e) => setEditDraft({ ...editDraft, label: e.target.value })} />
                </label>
                <label className={styles.field}>
                  <span>别名（逗号分隔，可空）</span>
                  <input className={styles.input} value={editDraft.aliases}
                    onChange={(e) => setEditDraft({ ...editDraft, aliases: e.target.value })} />
                </label>
                <label className={styles.field}>
                  <span>描述（可空）</span>
                  <textarea className={styles.textarea} value={editDraft.description}
                    onChange={(e) => setEditDraft({ ...editDraft, description: e.target.value })} />
                </label>
                <div className={styles.buttonRow}>
                  <button type="button" className={styles.btnPrimary} onClick={() => void submitEdit(item.id)} disabled={acting}>保存编辑</button>
                  <button type="button" className={styles.btnGhost} onClick={() => setEditingId(null)} disabled={acting}>取消</button>
                </div>
              </div>
            )}
```

在 `submitCreate` 后加三个函数：

```ts
  async function submitEdit(id: string) {
    if (!editDraft.label.trim()) {
      setError("显示名不能为空。");
      return;
    }
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      const aliases = editDraft.aliases.split(/[,，]/).map((a) => a.trim()).filter((a) => a.length > 0);
      await api.patch(`/api/admin/taxonomies/${id}`, {
        label: editDraft.label.trim(),
        aliases,
        description: editDraft.description.trim(),
      });
      setInfo("已更新。");
      setEditingId(null);
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  async function deprecateEntry(id: string) {
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      await api.delete(`/api/admin/taxonomies/${id}`);
      setInfo(includeDeprecated ? "已废弃。" : "已废弃，勾选「显示已废弃」可查看。");
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }

  async function restoreEntry(id: string) {
    setActing(true);
    setError(null);
    setInfo(null);
    try {
      await api.patch(`/api/admin/taxonomies/${id}`, { deprecated: false });
      setInfo("已恢复为启用。");
      await reload();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setActing(false);
    }
  }
```

> 注意：`includeDeprecated` 是现有 state（控制列表是否含 deprecated），`deprecateEntry` 的 info 据它给提示。

- [ ] **Step 5: 跑测试确认通过 + build**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "编辑与废弃恢复" && npm run build`
Expected: 3 PASS；build 零 TS 错误。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/features/system-strategy/index.tsx frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "feat(taxonomy-admin): 字典面板加行内编辑+废弃/恢复(PATCH/DELETE,按status切文案)"
```

---

### Task 4: 校验/软提示/409 边界用例收口 + 全量验证

**Files:**
- Test: `frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx`（append 边界用例）

**Interfaces:**
- Consumes: Task 2/3 已实现的 `submitCreate` 行为（409 当 info / 前端校验 / 状态机软提示条件渲染）。
- Produces: 无（纯补测试 + 全量验证）。

**背景**：Task 2 的实现已写了 409 当 info、前端校验、状态机软提示条件渲染的逻辑，但 Task 2 测试只覆盖了主路径（提交形态）。本 task 补 spec §7 剩余边界用例（#4 409、#5 校验、#6 软提示条件渲染），确保这些分支真被断言，再跑全量 build + test。

- [ ] **Step 1: 写边界测试**

append 到 `systemStrategy.test.tsx`：

```ts
describe("TaxonomiesAdmin 边界", () => {
  it("新增重复条目(409) 显示 info 不显示 error", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    vi.spyOn(api, "postRaw").mockResolvedValue({ ok: false, status: 409, data: { message: "(scope=global, kind=customer_stage, value.id=need_discovery) 已存在" } } as never);
    render(<SystemStrategyView /* 同款 */ />);
    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/canonical id/i), { target: { value: "need_discovery" } });
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.click(screen.getByText("保存"));
    expect(await screen.findByText(/已存在/)).toBeInTheDocument();
  });

  it("新增缺 canonical id 时本地校验拦下，不发请求", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    const postRaw = vi.spyOn(api, "postRaw");
    render(<SystemStrategyView /* 同款 */ />);
    fireEvent.click(await screen.findByText("新增条目"));
    fireEvent.change(screen.getByPlaceholderText(/显示名/i), { target: { value: "需求挖掘" } });
    fireEvent.click(screen.getByText("保存"));
    expect(await screen.findByText(/均不能为空/)).toBeInTheDocument();
    expect(postRaw).not.toHaveBeenCalled();
  });

  it("kind=customer_stage 显示状态机软提示，改成 intent_level 后不显示", async () => {
    vi.spyOn(api, "get").mockResolvedValue({ items: [] } as never);
    render(<SystemStrategyView /* 同款 */ />);
    fireEvent.click(await screen.findByText("新增条目"));
    expect(screen.getByText(/状态机灰度.*同步配置/)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("customer_stage"), { target: { value: "intent_level" } });
    expect(screen.queryByText(/状态机灰度.*同步配置/)).not.toBeInTheDocument();
  });
});
```

> 实现者：选择器文案以 Task 2 实际实现为准（软提示文案、placeholder、校验错误文案三处对齐）。「不显示 error」按现有 `styles.inlineError` 渲染方式选合适 query。

- [ ] **Step 2: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/system-strategy/systemStrategy.test.tsx -t "边界"`
Expected: 3 PASS（实现已在 Task 2 完成；若失败说明文案不符，对齐文案）。

- [ ] **Step 3: 全量前端验证**

Run: `cd frontend && npm run test && npm run build`
Expected: vitest 全绿（含新增 + 既有，无回归）；build 零 TS 错误。

- [ ] **Step 4: no-human-takeover 文案自查**

Run: `cd frontend && grep -rnE "人工|接管|takeover|hand[ -]?off|人工介入" src/features/system-strategy/index.tsx src/__tests__/features/system-strategy/systemStrategy.test.tsx || echo "无禁词"`
Expected: `无禁词`。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/__tests__/features/system-strategy/systemStrategy.test.tsx
git commit -m "test(taxonomy-admin): 补409当info/前端校验/状态机软提示条件渲染边界用例"
```

---

## 执行顺序与依赖

- **Task 1**（api.patch）独立基础设施，Task 3 依赖它——必须先做。
- **Task 2**（新增表单）用已有 `api.postRaw`；与 Task 3 改同一函数，先做新增。
- **Task 3**（编辑/废弃/恢复）依赖 Task 1（api.patch）+ Task 2（共享 state/reload/info/acting + draft 类型旁加 EditDraft）。
- **Task 4**（边界用例 + 全量验证）依赖 Task 2/3 实现完成。

顺序：1 → 2 → 3 → 4。Task 1 是独立 api 基础设施（lib 测）；Task 2/3 改同一 `TaxonomiesAdmin` 函数但可独立 reviewer 判（新增 vs 编辑废弃是不同交付）；Task 4 是测试收口 + 全量门。每 task 末尾 `npm run build` + 相关 vitest，Task 4 跑全量。
