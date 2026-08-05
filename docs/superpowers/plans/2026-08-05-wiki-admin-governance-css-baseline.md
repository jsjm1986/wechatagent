# 知识库 Wiki · 治理工坊样式基线修复 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修掉知识库 Wiki「治理工坊」四个面板的五处渲染缺陷，其中「发布给全部」按钮白底白字是功能缺陷（不可逆高危操作的按钮当前不可见）。

**Architecture:** 五处缺陷分属三层，各修在正确的层上：`input{width}` 是全局元素基线漏排除 checkbox → 改 `styles.css`；白底白字与按钮尺度是 `.wikiAdmin*` 漏覆盖全局值 → 补 `Knowledge.css`；表格列宽用 `table-layout: fixed` + JSX `<colgroup>`；ISO 时间跟同频道既有惯例 `new Date(x).toLocaleString()`。

**Tech Stack:** React 19 + TypeScript + Vite；plain CSS（`Knowledge.css` 不是 CSS module）；vitest 4 + @testing-library/react + jsdom。

**依据 spec:** `docs/superpowers/specs/2026-08-05-wiki-admin-governance-css-baseline-design.md`（commit `1bc50e4`）

## Global Constraints

- 分支：`fix/wiki-admin-governance-css-baseline-20260805`（已存在，spec 已提交在其上）
- 工作树中有**他人未提交的后端改动**（`src/agent/*.rs`、`tests/*.rs`、`AGENTS.md`、`CODE_REVIEW_FINDINGS.md`）。每次 `git add` 只 stage 本计划明确列出的文件，**禁止 `git add .` / `git add -A`**。
- CI 门禁 `scripts/check-no-human-takeover.sh` 扫描 `frontend/src/` 新增行中的禁用词，含「**人工**」。新增测试名、注释、字符串一律用「目视确认 / 视觉核验」，不得出现「人工」二字。
- `Knowledge.css` 必须保持 plain `.css`。改成 `.module.css` 会被 Rollup tree-shake 掉整份样式导致频道裸奔（见该文件头注释）。
- `Knowledge.css` 中 `.wikiAdmin*` 规则均带 `.knowledgeWiki` 前缀且缩进 2 空格，新增规则遵循同一格式。
- 测试命令：`cd frontend && npx vitest run <path>`（`package.json` 的 `test` 脚本是 `vitest run`）。
- CSS 变量已在 `src/components/ui/tokens.css` 定义：`--ink-2:#515156`、`--ink-3:#76767b`、`--surface-card:#ffffff`、`--surface-page:#eef1f5`、`--hairline:rgba(0,0,0,.08)`。
- 不做的事：不清理 6 处冗余的 checkbox 宽度局部覆盖；不重构四个面板的同构重复；不动 `today.tsx` / `steward.tsx` 里同样继承全局蓝按钮的 `.wikiArchiveHeaderActions`（超出本 spec 范围）。

## File Structure

| 文件 | 职责 | 本次改动 |
| --- | --- | --- |
| `frontend/src/styles.css` | 全局元素基线与设计 token | 1 行选择器：width 规则排除 checkbox/radio |
| `frontend/src/features/knowledge/Knowledge.css` | 知识库频道全部样式（plain CSS，3641 行） | 新增 `.wikiActionBtn--neutral`；新增两条工具栏按钮规则；`.wikiAdminTable` 加 `table-layout`；`tbody td` 加折行 |
| `frontend/src/features/knowledge/atlas.tsx` | 图谱/治理工坊组件（含 4 个治理面板 + PublishBar） | 「发布给全部」加 class；3 张表加 `<colgroup>`；3 处 `updatedAt` 改本地化渲染 |
| `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx` | **Task 1 新建**，Task 4 / 5 追加 | 共 8 个用例；helper 由 Task 1 一次定义 |

**任务顺序理由：** Task 1（功能缺陷，最高优先级，同时建立测试文件与 helper）→ Task 2（全局基线，唯一跨频道影响）→ Task 3（按钮尺度）→ Task 4（表格列宽）→ Task 5（时间格式化）。

Task 2、3 是纯 CSS 且无 DOM 变化，jsdom 无布局引擎故无法断言（见 spec 第四节），只做改动 + 目视清单。Task 1、4、5 都改 JSX，可断言结构与文本——Task 1 的白底白字虽是 CSS 现象，但修法是加 class，class 的存在可断言。

---

### Task 1: 修 PublishBar 白底白字（功能缺陷）

**Files:**
- Modify: `frontend/src/features/knowledge/atlas.tsx:1051`（给「发布给全部」按钮加 class）
- Modify: `frontend/src/features/knowledge/Knowledge.css`（在 `.wikiActionBtn--reject:not(:disabled):hover` 规则后新增 `--neutral` 规则）
- Create: `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`

**Interfaces:**
- Consumes: 无
- Produces:
  - CSS class `wikiActionBtn--neutral`（全库原先未占用，已核验）
  - 测试文件内的 helper：`response(body, ok?)`、`mockGovernanceApi()`、`renderGovernance()`、`openTab(tabName)` —— Task 4 与 Task 5 在同一文件追加测试时**直接复用，不要重复定义**

**背景（实施者必读）：** `atlas.tsx:1051` 的「发布给全部」按钮没有 class，`.wikiPublishBar button`（`Knowledge.css:3177`）覆盖了 `background: var(--surface-card)`（白）却没声明 `color`，于是继承 `styles.css:71` 的 `color: #fff` → 白底白字，渲染成一个看不见的空白框。同排另两个按钮可见是因为 `wikiActionBtn--verify`（`:2619`）和 `wikiActionBtn--reject`（`:2627`）各自带 `color`。该按钮触发的是不可逆操作（确认文案：「立即对所有客户生效，且不可逆」），所以这条是功能缺陷而非美观问题。

**特异性陷阱（必读，别抄近路）：** 直觉做法是在 `.wikiPublishBar button` 规则里补一行 `color`。**那是错的**，会造成两个回归：

| 选择器 | 特异性 | |
| --- | --- | --- |
| `.knowledgeWiki .wikiPublishBar button` | (0,2,1) | 2 类 + 1 元素 |
| `.knowledgeWiki .wikiActionBtn--verify` | (0,2,0) | 2 类 |

类计数打平在 2，元素选择器让 `.wikiPublishBar button` **胜出**，于是那行 `color` 会把蓝色的「发布新版」和红色的「回退上版」一起刷成灰。正确做法是给这个按钮补一个与 `--verify`/`--reject` **同级**的语义 class。

也不要用 `.wikiPublishBar button:not([class])`：它当下能只命中这一个按钮，但语义是「没有任何 class 的按钮」，将来谁给它加个无关 class（埋点标记之类）颜色就再次消失，是个潜伏陷阱。

- [ ] **Step 1: 写失败测试 — 三个 PublishBar 按钮都必须有兜色 class**

创建 `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`。这个文件后续 Task 4 / 5 会继续追加，helper 定义在此处一次到位。

```tsx
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConfirmProvider } from "../../../components/ui/ConfirmDialog";
import { ToastProvider } from "../../../components/ui/Toast";
import { AdminGovernanceView } from "../../../features/knowledge/atlas";

const realFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = realFetch;
  vi.restoreAllMocks();
});

function response(body: unknown, ok = true): Response {
  return {
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
    text: async () => JSON.stringify(body),
  } as Response;
}

const ISO_FIXTURE = "2026-06-26T07:25:11.049Z";

// 各治理面板的 GET 端点 → 固定 fixture。PublishBar 的 POST 不在本文件覆盖。
function mockGovernanceApi() {
  globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.startsWith("/api/admin/taxonomies")) {
      return response({
        items: [{
          id: "tax-a",
          scope: "global",
          kind: "churn_reason",
          value: { id: "need_changed", displayName: "需求变化", status: "active" },
          version: 1,
          currentVersion: true,
          updatedAt: ISO_FIXTURE,
        }],
      });
    }
    if (url.startsWith("/api/admin/operation-state-policies")) {
      return response({
        items: [{
          id: "policy-a",
          domain: "DEFAULT",
          version: 2,
          currentVersion: true,
          updatedAt: ISO_FIXTURE,
          states: [{ id: "new_contact" }],
        }],
      });
    }
    if (url.startsWith("/api/operation-domains")) {
      return response({
        items: [{
          id: "domain-a",
          domain: "DEFAULT",
          version: 3,
          currentVersion: true,
          updatedAt: ISO_FIXTURE,
        }],
      });
    }
    return response({ items: [] });
  }) as typeof fetch;
}

function renderGovernance() {
  return render(
    <ToastProvider>
      <ConfirmProvider>
        <AdminGovernanceView />
      </ConfirmProvider>
    </ToastProvider>,
  );
}

// 切到指定 tab。治理工坊四个 tab 同时只渲染一个面板，故全局查询不会串台。
async function openTab(tabName: string): Promise<HTMLTableElement> {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: tabName }));
  return waitFor(() => {
    const found = document.querySelector("table.wikiAdminTable");
    if (!found) throw new Error(`${tabName} 面板未渲染表格`);
    return found as HTMLTableElement;
  });
}

describe("治理工坊 PublishBar 按钮兜色", () => {
  // 三个按钮都在白底(.wikiPublishBar button 的 background)上。全局 button 基线
  // 是 color:#fff，任何一个按钮少了显式 color 就会白底白字彻底不可见。
  // 「发布给全部」触发不可逆的全量推送，尤其不能隐形。
  // jsdom 无 CSS 层叠，无法断言实际颜色，故断言每个按钮都带兜色 class——
  // 类名到颜色的映射由 Knowledge.css 保证，实际颜色需目视确认。
  it("三个按钮各自带兜色 class，无裸 button", async () => {
    mockGovernanceApi();
    renderGovernance();
    await openTab("分类系统");

    const bar = document.querySelector(".wikiPublishBar");
    expect(bar).not.toBeNull();

    const buttons = Array.from(bar!.querySelectorAll("button"));
    expect(buttons).toHaveLength(3);

    buttons.forEach((button) => {
      expect(button.className.trim()).not.toBe("");
    });

    expect(
      screen.getByRole("button", { name: /发布新版/ }).className,
    ).toContain("wikiActionBtn--verify");
    expect(
      screen.getByRole("button", { name: /发布给全部/ }).className,
    ).toContain("wikiActionBtn--neutral");
    expect(
      screen.getByRole("button", { name: /回退上版/ }).className,
    ).toContain("wikiActionBtn--reject");
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/adminGovernance.test.tsx`

Expected: 1 个用例 FAIL，报「发布给全部」的 className 不含 `wikiActionBtn--neutral`（实际为空串）。

若报错是「分类系统 面板未渲染表格」或 provider 相关异常，说明 fixture 或包裹有问题，先修好再往下走——不要跳过红灯。

- [ ] **Step 3: 给按钮加 class**

打开 `frontend/src/features/knowledge/atlas.tsx`，找到第 1051 行。当前内容：

```tsx
      <button type="button" onClick={() => void call("rollout")} disabled={busy !== ""}>
        <ArrowRight size={12} /> {busy === "rollout" ? "发布中…" : "发布给全部"}
      </button>
```

改为：

```tsx
      <button
        type="button"
        onClick={() => void call("rollout")}
        disabled={busy !== ""}
        className="wikiActionBtn--neutral"
      >
        <ArrowRight size={12} /> {busy === "rollout" ? "发布中…" : "发布给全部"}
      </button>
```

- [ ] **Step 4: 加 CSS 规则**

打开 `frontend/src/features/knowledge/Knowledge.css`，找到 `.knowledgeWiki .wikiActionBtn--reject:not(:disabled):hover` 规则（`:2631`）的结束大括号，在其后插入：

```css
  /* 中性款操作按钮。必须与 --verify / --reject 同级（2 类）而不是写进
     .wikiPublishBar button（2 类 + 1 元素，特异性更高）——写在那里会把
     蓝色的「发布新版」和红色的「回退上版」一起刷成灰。
     没有这条兜色，按钮就继承全局 button 基线的 color:#fff，
     在 .wikiPublishBar button 的白底上完全不可见。 */
  .knowledgeWiki .wikiActionBtn--neutral {
    color: var(--ink-2);
  }
```

- [ ] **Step 5: 运行测试，确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/adminGovernance.test.tsx`
Expected: 1 passed。

- [ ] **Step 6: 构建并目视确认三个按钮颜色**

Run: `cd frontend && npm run build`
Expected: 构建成功，无 CSS 报错。

然后 `npm run dev`，打开「知识库 Wiki → 控制台 → 治理工坊 → 分类系统」，确认「操作」列三个按钮**全部可见**且颜色互不干扰：

| 按钮 | 期望 |
| --- | --- |
| `发布新版` | 蓝色描边 + 蓝字（**没变灰**） |
| `发布给全部` | 灰字可见（本次修复目标） |
| `回退上版` | 红色描边 + 红字（**没变灰**） |

前后两个变灰就说明 class 被写进了 `.wikiPublishBar button`，回到 Step 4 检查规则位置。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/knowledge/atlas.tsx frontend/src/features/knowledge/Knowledge.css frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx
git commit -m "fix(ui): 治理工坊「发布给全部」按钮不再白底白字

.wikiPublishBar button 覆盖了 background 但未声明 color，无 class 的
「发布给全部」于是继承全局 button 基线的 color:#fff，渲染为不可见的
空白框。该按钮触发不可逆的全量推送操作，不应隐形。

兜色走与 --verify / --reject 同级的新 class，而非写进
.wikiPublishBar button——后者特异性 (0,2,1) 高于那两个 class 的
(0,2,0)，会把蓝/红两个按钮一起刷成灰。"
```

---

### Task 2: 全局基线 — width 规则排除 checkbox / radio

**Files:**
- Modify: `frontend/src/styles.css:563-570`

**Interfaces:**
- Consumes: 无
- Produces: 全局 `input` 的 `width:100%` 不再作用于 checkbox/radio，`.wikiAdminToolbarLabel` 内的 checkbox 恢复原生尺寸

**背景（实施者必读）：** `styles.css:563` 的 `input, textarea { width: 100% }` 对 checkbox 同样生效，导致 `.wikiAdminToolbarLabel`（`inline-flex`）内的 checkbox flex-basis 变成 100%，与文本争空间，中文逐字换行 → 「显示历史版本」竖排成一列。紧邻的 `:572` 已经用 `:not()` 把 `height` 排除了 checkbox/radio，`width` 漏了。

**安全性已核验（spec 第三节 1）：** 全库 38 处 `type="checkbox"`，其中 6 处已显式写 `width: 16px/15px/auto` 与全局对抗，**0 处依赖 `width:100%`**；radio 全库 0 处使用；`appearance:none` 的元素全是 `<select>`；`RosterView` 的 `.checkbox` 作用于 `<div>` 不是 input。

- [ ] **Step 1: 修改 width 规则的选择器**

打开 `frontend/src/styles.css`，找到第 563 行。当前内容：

```css
input,
textarea {
  width: 100%;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--ink);
}
```

改为（拆出 width，与下方 height 规则口径一致）：

```css
input,
textarea {
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--ink);
}

/* width 与紧随的 height 规则同口径排除 checkbox/radio：设计规范
   docs/frontend-design-system.md:162「Inputs are full width」指文本输入框，
   勾选框被撑满会挤压同行 label 文本，导致中文逐字换行。 */
input:not([type="checkbox"]):not([type="radio"]),
textarea {
  width: 100%;
}
```

- [ ] **Step 2: 确认 height 规则仍在其后且未被破坏**

Run: `cd frontend && sed -n '560,585p' src/styles.css`

Expected: 依次看到 `input, textarea` 基础规则（无 width）、新增的 width 规则、原有的 `input:not([type="checkbox"]):not([type="radio"]) { height: var(--control-h); }`、`textarea { min-height: 132px; ... }`。`textarea` 仍在 width 规则中（textarea 需要满宽）。

- [ ] **Step 3: 构建**

Run: `cd frontend && npm run build`
Expected: 构建成功。

- [ ] **Step 4: 目视抽查含 checkbox 的其它频道**

`npm run dev` 后逐一打开，确认 checkbox 为方形小框、同行 label 单行显示、布局未变形：

- 「系统策略」频道（`system-strategy`，21 处 `.inlineCheckbox`，最大用量）
- 「内容资产」频道（`content-assets`）
- 「AI 总控」频道（`command-center`，其 `.dryRunToggle input` 写的是 `width:auto`）
- 「自主进化」频道（`evolution`，`.flagToggle`）
- 「知识库 Wiki → 控制台 → 治理工坊 → 分类系统」：「显示历史版本」应单行显示

- [ ] **Step 5: Commit**

```bash
git add frontend/src/styles.css
git commit -m "fix(ui): 全局 input width 基线排除 checkbox/radio

input{width:100%} 对勾选框同样生效，会挤压同行 label 使中文逐字换行。
紧邻的 height 规则早已用 :not() 排除 checkbox/radio，width 漏了同样处理。
全库 38 处 checkbox 中 6 处已各自写 width 覆盖与之对抗，0 处依赖 100%。"
```

---

### Task 3: 工具栏「刷新」按钮尺度

**Files:**
- Modify: `frontend/src/features/knowledge/Knowledge.css`（在 Task 1 修改的 `.wikiPublishBar button` 规则之前插入，即原 `:3145` 与 `:3146` 之间）

**Interfaces:**
- Consumes: Task 1 确立的 `.wikiPublishBar button` 视觉规格（11px / `padding:4px 8px` / `background: var(--surface-card)` / `color: var(--ink-2)`）
- Produces: 无（终端样式）

**背景（实施者必读）：** 四个治理面板的「刷新」按钮都没有 class，完整继承 `styles.css:71` 的全局 button 基线：`min-height:38px`、`background:#175cd3`（纯蓝）、`padding:8px 13px`、`font-size:13px`、`font-weight:680`。同屏 PublishBar 按钮是 11px 白底描边款，尺度冲突明显。

**关键事实（spec 未覆盖，实施时必须按此处理）：** 四个「刷新」按钮的容器**不同**：

| 面板 | 按钮位置 | 容器 class |
| --- | --- | --- |
| `TaxonomiesGovernance` | `atlas.tsx:1167` | `.wikiAdminToolbar` |
| `StatePoliciesGovernance` | `atlas.tsx:1257` | `.wikiAdminToolbar` |
| `DomainGovernance` | `atlas.tsx:1338` | `.wikiAdminToolbar` |
| `MetadataDashboard` | `atlas.tsx:865` | `.wikiArchiveHeaderActions` |

所以单靠 `.wikiAdminToolbar button` 只覆盖前三个。第四个不能用 `.wikiArchiveHeaderActions button`——该 class 在 `today.tsx:376`、`today.tsx:581`、`steward.tsx:1732`、`steward.tsx:2419` 也在用，会溢出到本 spec 范围外。改用 `.wikiMetadataDashboard`（`atlas.tsx:859` 唯一使用处）精确限定。

不复用全局 `button.secondary`（`styles.css:108`）：它只改 `border-color`/`background`/`color`，不改 `min-height`，38px 仍会压着 11px 的表格行。

- [ ] **Step 1: 新增两条工具栏按钮规则**

打开 `frontend/src/features/knowledge/Knowledge.css`，找到 `.knowledgeWiki .wikiAdminToolbarLabel` 规则的结束大括号（原第 3145 行），在其后、`.knowledgeWiki .wikiAdminTable` 之前插入：

```css
  /* 治理工坊工具栏/表头的「刷新」按钮：全局 button 基线（styles.css:71）是
     38px 高的纯蓝实心款，压在 11px 的 PublishBar 按钮和表格行上尺度失调。
     此处对齐同面板 .wikiPublishBar button 的规格，并显式压掉 min-height
     与 background/color。MetadataDashboard 的刷新按钮在 .wikiArchiveHeaderActions
     内，而该 class 亦被 today.tsx / steward.tsx 使用，故用唯一的
     .wikiMetadataDashboard 精确限定，避免溢出到本次范围外的面板。 */
  .knowledgeWiki .wikiAdminToolbar button,
  .knowledgeWiki .wikiMetadataDashboard .wikiArchiveHeaderActions button {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    min-height: auto;
    border: 1px solid var(--surface-page);
    background: var(--surface-card);
    color: var(--ink-2);
    padding: 4px 8px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .knowledgeWiki .wikiAdminToolbar button:hover:not(:disabled),
  .knowledgeWiki .wikiMetadataDashboard .wikiArchiveHeaderActions button:hover:not(:disabled) {
    background: var(--surface-page);
  }
```

- [ ] **Step 2: 确认插入位置正确**

Run: `cd frontend && grep -n "wikiAdminToolbarLabel\|wikiAdminToolbar button\|wikiAdminTable {" src/features/knowledge/Knowledge.css | head -8`

Expected: 行号顺序为 `.wikiAdminToolbarLabel` → `.wikiAdminToolbar button` → `.wikiAdminTable {`。新规则夹在两者之间。

- [ ] **Step 3: 构建**

Run: `cd frontend && npm run build`
Expected: 构建成功。

- [ ] **Step 4: 目视确认四个面板**

`npm run dev`，打开「知识库 Wiki → 控制台 → 治理工坊」，逐个切换四个 tab（元信息 / 分类系统 / 状态策略 / 域配置），确认「刷新」按钮均为白底描边小号款，与 PublishBar 按钮尺度协调，不再是 38px 纯蓝。

另确认未溢出：打开「知识库 Wiki → 工作台」（`today.tsx`）与「知识库 → 治理」（`steward.tsx`），其「新会话」/「刷新」按钮应**保持原样**（仍是全局蓝或原有 `.ghost wikiBtn` 款）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/knowledge/Knowledge.css
git commit -m "style(ui): 治理工坊刷新按钮对齐面板内按钮尺度

四个治理面板的刷新按钮无 class，继承全局 38px 纯蓝基线，压在同屏
11px 的 PublishBar 按钮和表格行上。MetadataDashboard 的按钮在
wikiArchiveHeaderActions 内，用唯一的 wikiMetadataDashboard 限定，
避免影响同样使用该 class 的 today/steward 面板。"
```

---

### Task 4: 表格列宽约束（含测试）

**Files:**
- Modify: `frontend/src/features/knowledge/Knowledge.css`（`.wikiAdminTable` 与 `.wikiAdminTable tbody td` 两条规则，按选择器字符串定位——Task 3 已在其上方插入规则，行号已下移）
- Modify: `frontend/src/features/knowledge/atlas.tsx`（3 张表加 `<colgroup>`，按 `<table className="wikiAdminTable">` 字符串定位）
- Modify: `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`（**追加** describe 块，文件已由 Task 1 创建）

**Interfaces:**
- Consumes: Task 1 已在测试文件中定义的 `response()`、`ISO_FIXTURE`、`mockGovernanceApi()`、`renderGovernance()`、`openTab(tabName)` —— **直接复用，不要重复定义**（重复声明会导致 TS 编译报错）
- Produces: 无

**背景（实施者必读）：** `.wikiAdminTable` 只有 `width:100%`，无 `table-layout`，浏览器按内容自动分配列宽。9 列表格因此把表头「版本」竖排、「当前生效」折成两截，行高被撑到约 93px。用 `<colgroup>` 而非 CSS `nth-child`：三张表列数不同（9/6/5），`table-layout:fixed` 无显式宽度时均分，会让 `✓` 这类窄列与「更新时间」同宽。

三张表的列构成（已核验 `<th>` 数与 `colSpan` 一致）：

| 面板 | 行号 | 列数 | 表头 |
| --- | --- | --- | --- |
| `TaxonomiesGovernance` | `:1172` | 9 | 范围/类型/取值/标签/状态/版本/当前生效/更新时间/操作 |
| `StatePoliciesGovernance` | `:1262` | 6 | 业务域/版本/当前生效/状态数/更新时间/操作 |
| `DomainGovernance` | `:1343` | 5 | 业务域/版本/当前生效/更新时间/操作 |

- [ ] **Step 1: 写失败测试 — colgroup 列数必须与表头列数一致**

在 `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`（Task 1 已创建）**末尾追加**下述 describe 块。文件顶部的 import、`response()`、`ISO_FIXTURE`、`mockGovernanceApi()`、`renderGovernance()`、`openTab()` 都已存在，**不要重复定义**：

```tsx
describe("治理工坊表格列宽约束", () => {
  // table-layout:fixed 下无显式列宽会均分，9 列均分会让 ✓ 窄列与更新时间同宽。
  // 故每张表都必须有 colgroup，且列数与表头严格一致——将来加列漏改 colgroup
  // 会被此测试拦下。jsdom 无布局引擎，只能断言结构，实际列宽需目视确认。
  it.each([
    ["分类系统", 9],
    ["状态策略", 6],
    ["域配置", 5],
  ])("%s 表的 colgroup 列数与表头一致（%i 列）", async (tabName, expectedColumns) => {
    mockGovernanceApi();
    renderGovernance();
    const table = await openTab(tabName as string);

    const headerCells = table.querySelectorAll("thead th");
    expect(headerCells).toHaveLength(expectedColumns as number);

    const cols = table.querySelectorAll("colgroup col");
    expect(cols).toHaveLength(expectedColumns as number);

    // 每个 col 都必须声明宽度，否则退化为均分。
    cols.forEach((col) => {
      expect((col as HTMLTableColElement).style.width).not.toBe("");
    });
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/adminGovernance.test.tsx`

Expected: 3 个用例全 FAIL，报 `expected length 0 to be 9`（以及 6、5）——`colgroup col` 尚未存在。若报的是「面板未渲染表格」或 provider 错误，先修 fixture/包裹再继续。

- [ ] **Step 3: 给三张表加 `<colgroup>`**

打开 `frontend/src/features/knowledge/atlas.tsx`。

**3a.** `TaxonomiesGovernance`，第 1172 行 `<table className="wikiAdminTable">` 之后、`<thead>` 之前插入（9 列，合计 100%）：

```tsx
        <colgroup>
          <col style={{ width: "8%" }} />
          <col style={{ width: "12%" }} />
          <col style={{ width: "14%" }} />
          <col style={{ width: "12%" }} />
          <col style={{ width: "8%" }} />
          <col style={{ width: "6%" }} />
          <col style={{ width: "7%" }} />
          <col style={{ width: "15%" }} />
          <col style={{ width: "18%" }} />
        </colgroup>
```

**3b.** `StatePoliciesGovernance`，第 1262 行 `<table className="wikiAdminTable">` 之后插入（6 列）：

```tsx
        <colgroup>
          <col style={{ width: "20%" }} />
          <col style={{ width: "8%" }} />
          <col style={{ width: "9%" }} />
          <col style={{ width: "12%" }} />
          <col style={{ width: "23%" }} />
          <col style={{ width: "28%" }} />
        </colgroup>
```

**3c.** `DomainGovernance`，第 1343 行 `<table className="wikiAdminTable">` 之后插入（5 列）：

```tsx
        <colgroup>
          <col style={{ width: "24%" }} />
          <col style={{ width: "9%" }} />
          <col style={{ width: "10%" }} />
          <col style={{ width: "25%" }} />
          <col style={{ width: "32%" }} />
        </colgroup>
```

注意：插入后行号会下移，`3b`/`3c` 请按 `<table className="wikiAdminTable">` 字符串定位而非硬用行号。

- [ ] **Step 4: 运行测试，确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/adminGovernance.test.tsx`
Expected: 4 passed（Task 1 的 1 个兜色用例 + 本 Task 的 3 个列宽用例）。

- [ ] **Step 5: CSS 加 `table-layout: fixed` 与折行**

打开 `frontend/src/features/knowledge/Knowledge.css`。

**5a.** `.knowledgeWiki .wikiAdminTable`（原 `:3146`）当前：

```css
  .knowledgeWiki .wikiAdminTable {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
```

改为：

```css
  .knowledgeWiki .wikiAdminTable {
    width: 100%;
    /* fixed 让列宽只由 colgroup 决定：auto 布局下 9 列会按内容抢宽度，
       把「版本」「当前生效」表头竖排、行高撑到约 93px。 */
    table-layout: fixed;
    border-collapse: collapse;
    font-size: 12.5px;
  }
```

**5b.** `.knowledgeWiki .wikiAdminTable tbody td`（原 `:3163`）当前：

```css
  .knowledgeWiki .wikiAdminTable tbody td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--surface-page);
    vertical-align: middle;
  }
```

改为（固定列宽后长内容必须能折行，否则溢出单元格）：

```css
  .knowledgeWiki .wikiAdminTable tbody td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--surface-page);
    vertical-align: middle;
    word-break: break-word;
    overflow-wrap: anywhere;
  }
```

- [ ] **Step 6: 构建 + 跑全量前端测试**

Run: `cd frontend && npm run build && npx vitest run`
Expected: 构建成功；测试全绿（新增 3 个用例，既有用例无回归）。

- [ ] **Step 7: 目视确认列宽**

`npm run dev`，打开三个 tab（分类系统 / 状态策略 / 域配置），确认：表头不竖排、行高紧凑、各列对齐、「操作」列三个按钮不折成两行、长 id 在单元格内折行而非溢出。

- [ ] **Step 8: Commit**

```bash
git add frontend/src/features/knowledge/Knowledge.css frontend/src/features/knowledge/atlas.tsx frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx
git commit -m "fix(ui): 治理工坊表格改固定列宽，表头不再竖排

wikiAdminTable 无 table-layout，浏览器按内容抢分列宽，9 列表把表头
竖排、行高撑到约 93px。改 table-layout:fixed 并为三张表（9/6/5 列）
各加 colgroup 显式列宽；单元格加 word-break 承接长内容。
新增测试锁定 colgroup 列数与表头一致，防止加列时漏改。"
```

---

### Task 5: ISO 时间本地化渲染（含测试）

**Files:**
- Modify: `frontend/src/features/knowledge/atlas.tsx` — 3 处 `updatedAt` 单元格（Task 4 插入 colgroup 后行号下移，按字符串定位）
- Modify: `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`（追加 describe 块）

**Interfaces:**
- Consumes: Task 1 在 `adminGovernance.test.tsx` 建立的 `ISO_FIXTURE` 常量与 `response()` / `mockGovernanceApi()` / `renderGovernance()` / `openTab()` helper
- Produces: 无

**背景（实施者必读）：** 三处 `updatedAt` 直接渲染后端 ISO 字符串，得到 `2026-06-26T07:25:11.049Z`，列宽不足时在连字符处断成两行。三处原始形态完全相同：

```tsx
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
```

分属 `TaxonomiesGovernance`（原 `:1205`）、`StatePoliciesGovernance`（原 `:1287`）、`DomainGovernance`（原 `:1366`）。`MetadataDashboard` 不渲染 `updatedAt`，不在范围内。

跟同频道既有惯例（`steward.tsx:495`、`steward.tsx:2250`、`CampaignList.tsx:78`）：`x ? new Date(x).toLocaleString() : "—"`。不新增工具函数——`lib/format.ts` 只有数值格式化，日期已有成型写法。

- [ ] **Step 1: 写失败测试**

在 `adminGovernance.test.tsx` 末尾追加（复用 Task 1 已定义的 `ISO_FIXTURE` 与各 helper，**不要重复定义**，也不要重新 import——文件顶部的 import 已齐全）：

```tsx
describe("治理工坊更新时间渲染", () => {
  // 后端返回 ISO 串，直接渲染会在固定列宽下于连字符处断行，且不是运营可读格式。
  // 与同频道 steward.tsx 惯例一致：new Date(x).toLocaleString()。
  // 断言用 toLocaleString() 现算而非硬编码字符串：CI 与开发机的时区/locale
  // 不同，硬编码会在其中一边失败。
  it.each([
    ["分类系统"],
    ["状态策略"],
    ["域配置"],
  ])("%s 表把 ISO 时间渲染为本地化格式", async (tabName) => {
    mockGovernanceApi();
    renderGovernance();
    const table = await openTab(tabName as string);

    const expected = new Date(ISO_FIXTURE).toLocaleString();
    expect(table.textContent).toContain(expected);
    // 原始 ISO 串不应再出现
    expect(table.textContent).not.toContain(ISO_FIXTURE);
  });

  it("updatedAt 缺失时回退为破折号", async () => {
    globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input).startsWith("/api/operation-domains")) {
        return response({
          items: [{ id: "domain-a", domain: "DEFAULT", version: 3, currentVersion: true }],
        });
      }
      return response({ items: [] });
    }) as typeof fetch;

    renderGovernance();
    const table = await openTab("域配置");
    const cells = table.querySelectorAll("tbody td");
    // 域配置表第 4 列（索引 3）是更新时间
    expect(cells[3]?.textContent).toBe("—");
  });
});
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/adminGovernance.test.tsx -t 更新时间`

Expected: 4 个用例 FAIL。前 3 个报 `not.toContain(ISO_FIXTURE)` 断言失败（原始 ISO 串仍在渲染）；第 4 个报期望 `"—"` 实得 `""`。

- [ ] **Step 3: 改三处渲染**

在 `frontend/src/features/knowledge/atlas.tsx` 中，把三处

```tsx
              <td className="wikiArchiveTimelineTime">{it.updatedAt ?? ""}</td>
```

逐处改为

```tsx
              <td className="wikiArchiveTimelineTime">
                {it.updatedAt ? new Date(it.updatedAt).toLocaleString() : "—"}
              </td>
```

三处内容完全相同，可用编辑器的「全部替换」，但必须确认只命中 3 处（`grep -c` 验证见 Step 4）。

- [ ] **Step 4: 确认改了且只改了 3 处**

Run: `cd frontend && grep -c "new Date(it.updatedAt).toLocaleString()" src/features/knowledge/atlas.tsx && grep -c "{it.updatedAt ?? \"\"}" src/features/knowledge/atlas.tsx`

Expected: 第一个数字为 `3`，第二个命令因无匹配而返回 `0`（grep 退出码 1，属正常）。

- [ ] **Step 5: 运行测试，确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/knowledge/adminGovernance.test.tsx`
Expected: 8 passed（Task 1 的 1 个 + Task 4 的 3 个 + 本 Task 的 4 个）。

- [ ] **Step 6: 全量测试 + 构建 + 门禁**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 全绿、构建成功。

Run: `cd .. && bash scripts/check-no-human-takeover.sh`
Expected: 通过。该门禁扫描 `frontend/src/` 新增行中的禁用词（含「人工」）——本次新增注释用的是「目视确认」，不含禁用词。若报错，按报错行改措辞。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/knowledge/atlas.tsx frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx
git commit -m "fix(ui): 治理工坊更新时间改本地化渲染

三处 updatedAt 直接渲染后端 ISO 串（2026-06-26T07:25:11.049Z），固定
列宽下会在连字符处断行，也不是运营可读格式。改用同频道既有惯例
new Date(x).toLocaleString()，缺失回退破折号。"
```

---

## 收尾：推送与 PR

- [ ] **Step 1: 确认只提交了本计划涉及的文件**

Run: `git log --oneline 1bc50e4..HEAD --stat && git status --short`

Expected: 提交仅含 `frontend/src/styles.css`、`frontend/src/features/knowledge/Knowledge.css`、`frontend/src/features/knowledge/atlas.tsx`、`frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`。`git status` 中他人的后端改动（`src/agent/*.rs`、`tests/*.rs`、`AGENTS.md`、`CODE_REVIEW_FINDINGS.md`、`src/agent/run_audit.rs`）应仍为未提交状态。

- [ ] **Step 2: 推送并建 PR**

```bash
git push -u origin fix/wiki-admin-governance-css-baseline-20260805
gh pr create --base main --title "fix(ui): 修复治理工坊五处渲染缺陷（含白底白字的高危按钮）" --body "$(cat <<'EOF'
## 摘要

修复知识库 Wiki「治理工坊」四个面板的五处渲染缺陷。其中「发布给全部」按钮白底白字属功能缺陷——该按钮触发不可逆的全量推送，当前渲染为不可见空白框。

设计文档：`docs/superpowers/specs/2026-08-05-wiki-admin-governance-css-baseline-design.md`

## 改动

| # | 缺陷 | 根因 | 改法 |
| --- | --- | --- | --- |
| 1 | 「发布给全部」白底白字（**功能缺陷**） | `.wikiPublishBar button` 覆盖 background 未声明 color，继承全局 `#fff` | 给该按钮加 `wikiActionBtn--neutral` class，与 `--verify`/`--reject` 同级 |
| 2 | checkbox 撑满行宽，label 竖排 | 全局 `input{width:100%}` 未排除 checkbox | width 规则加 `:not([type=checkbox]):not([type=radio])` |
| 3 | 刷新按钮 38px 纯蓝 | 裸 button 继承全局基线 | 新增作用域按钮规则（含 `.wikiMetadataDashboard` 精确限定） |
| 4 | 9 列表表头竖排、行高约 93px | 无 `table-layout` | `fixed` + 三张表 colgroup + word-break |
| 5 | 更新时间显示 ISO 串 | 直接渲染后端值 | `new Date(x).toLocaleString()` |

改动 1 没有采用「在 `.wikiPublishBar button` 里补一行 `color`」这个直觉做法：该选择器特异性是 (0,2,1)（2 类 + 1 元素），高于 `.wikiActionBtn--verify` 的 (0,2,0)，那样写会把蓝色的「发布新版」和红色的「回退上版」一起刷成灰——修一个缺陷换两个回归。改用同级 class 后，PublishBar 三个按钮各自有显式 color，形态一致。

## 测试

新增 `frontend/src/__tests__/features/knowledge/adminGovernance.test.tsx`（8 个用例）：PublishBar 三个按钮各自带兜色 class（无裸 button）、三张表 colgroup 列数与表头一致（9/6/5，防止加列漏改）、三个面板时间本地化渲染、缺失值回退破折号。

**测试边界**：jsdom 无布局引擎也无 CSS 层叠，`table-layout` / `width` / 实际颜色都无法断言，仅覆盖结构、类名与文本。改动 2、3 为纯 CSS，需目视确认（清单见设计文档第四节）。

## 影响面

改动 2 是唯一的全局改动。已核验全库 38 处 checkbox：6 处已各自写 `width` 覆盖与全局对抗，0 处依赖 `width:100%`，radio 零使用。

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: 等 CI**

Run: `gh pr checks --watch`
Expected: 全绿再合并。后端门禁（Baseline / Integration）会因 `dorny/paths-filter` 显示 skipping——本 PR 只碰 `frontend/src/` 与 `docs/`，属预期。
