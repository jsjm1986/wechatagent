# 用户运营池计数与文案修正 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把"用户运营池"(ContactsView) 的三个计数改为后端真实 count、修 limit 截断、tab 文案改"已互动/待启用"、移除与通讯录重复的导入框。

**Architecture:** 后端新增 `GET /api/contacts/counts` 用 `count_documents` 返回精确 `{all,managed,normal}`（与 `list_contacts` 同源 filter）；前端 store 拉该端点存 `contactCounts`，三 tab 读它而非对已加载数组 `.filter`；`refreshContacts` 拼参加 `limit=500` 防截断；ContactsView 改文案、删导入 `<form>`，顺藤清理 `importQuery`/`importContacts`/`setImportQuery` 全链与 contactStore 死方法。

**Tech Stack:** Rust (Axum 0.7 + mongodb 2.8) 后端；React 19 + TypeScript + Zustand + Vite + Vitest 前端。

## Global Constraints

- `cargo test --lib` **≥ 350 passed / 0 failed**（新增测试只增不减，来自 spec 验证门）。
- `cargo check --tests` 必须过（复刻 CI baseline step2；删 props 可能牵连编译）。
- `cd frontend && npm run build` 必须过（TS 编译 + 死代码清理后无悬空引用）。
- `cd frontend && npx vitest run` 必须过（前端契约门）。
- 红线：改任何代码前先 100% 读懂受影响代码路径，`file:line` 引用必亲验，绝不猜测。
- 纯 UI / 计数层改动，**零业务逻辑改动**：不碰 webhook 自动建档、RosterView、operation_state / 画像 / 记忆 / 状态机。
- 后端 `search`/`import` 端点保留（仅移除运营池 UI 入口）。
- 提交需用户显式授权；本计划每个 Task 末尾的 commit 步骤在获授权后执行。
- 回复用中文；代码 / 标识符 / commit message 沿用现有约定。

## File Structure

**后端：**
- `src/routes/contacts.rs` — 新增 `count_contacts` handler + `contact_count_filters` 纯函数 + 纯函数单测（模块内 `#[cfg(test)]`）。
- `src/routes/mod.rs` — 在 `/contacts/:id`（当前 :352）**之前**挂 `/contacts/counts` 路由，并把 `count_contacts` 加入 `use` 列表（当前 :176 附近）。

**前端：**
- `frontend/src/stores/userOpsStore.ts` — 新增 `contactCounts` state + `loadContactCounts` action；`refreshContacts` 加 `limit=500`；删 `importQuery`/`setImportQuery`/`importContacts`（含接口声明 + 初值 + 实现）。
- `frontend/src/features/user-ops/index.tsx` — `managedCount`/`normalCount` 改读 `contactCounts`；`totalCount` 传 `contactCounts.all`；effect 追加 `loadContactCounts`；改 managed 的动作后追加刷新；删 `onImportContacts`/`importQuery`/`setImportQuery`/`importContacts` 接线与 ContactsView 的对应 props。
- `frontend/src/features/user-ops/legacy.tsx` — ContactsView：改 tab 文案、删导入 `<form>`、改过滤框 placeholder、删 `importQuery`/`onImport`/`onImportQuery` props（保留 `onLoadAll`/`query`/`onQuery`）。
- `frontend/src/stores/contactStore.ts` — 删死方法 `managedCount`/`normalCount`（接口声明 :11-12 + 实现 :22-23）。
- `frontend/src/__tests__/features/user-ops/userOps.test.tsx` — `createMockStore` 补 `contactCounts` + `loadContactCounts`，否则 index.tsx 读 `contactCounts.managed` 崩。
- `frontend/src/__tests__/features/user-ops/contactsView.test.tsx` —（新建）单独渲染真实 ContactsView 断言新文案 + 导入框已移除（现有 userOps.test.tsx 把 ContactsView 整体 mock 成占位 div，测不到真实文案）。

---

### Task 1: 后端 count 端点 + 纯函数单测

**Files:**
- Modify: `src/routes/contacts.rs`（新增 handler + 纯函数，紧跟 `list_contacts` 之后，当前 :149 附近）
- Modify: `src/routes/mod.rs`（:176 附近 `use` 列表加 `count_contacts`；:346-352 之间加路由）
- Test: `src/routes/contacts.rs` 内 `#[cfg(test)] mod tests`（文件已有该模块，:1478 起）

**Interfaces:**
- Consumes: `ContactQuery`（`src/models.rs:3289`，字段 `account_id: Option<String>` 等，已 `#[serde(rename="accountId")]`）；`AuthenticatedAdmin`（已在 contacts.rs:17 导入，字段 `current_workspace`）；`state.config.default_account_id`。
- Produces: `pub(super) async fn count_contacts(...)`（供 mod.rs 挂路由）；`fn contact_count_filters(workspace_id: &str, account_id: &str) -> (Document, Document)`（供单测）。

- [ ] **Step 1: 写失败的纯函数单测**

在 `src/routes/contacts.rs` 的 `#[cfg(test)] mod tests`（:1478 起，`use super::*;` 已在）追加：

```rust
    #[test]
    fn contact_count_filters_isolate_workspace_and_account() {
        let (base, managed) = contact_count_filters("ws1", "acct1");
        // base：仅 workspace + account 隔离（与 list_contacts 同源）。
        assert_eq!(base.get_str("workspace_id").unwrap(), "ws1");
        assert_eq!(base.get_str("account_id").unwrap(), "acct1");
        assert!(base.get("agent_status").is_none(), "base 不得含 agent_status");
        // managed：在 base 基础上加 agent_status=managed。
        assert_eq!(managed.get_str("workspace_id").unwrap(), "ws1");
        assert_eq!(managed.get_str("account_id").unwrap(), "acct1");
        assert_eq!(managed.get_str("agent_status").unwrap(), "managed");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib contact_count_filters_isolate`
Expected: 编译失败 `cannot find function contact_count_filters in this scope`。

- [ ] **Step 3: 实现纯函数 + handler**

在 `src/routes/contacts.rs` 的 `list_contacts` 函数结束后（当前 :149 `}` 之后）插入：

```rust
/// 计数端点的 filter 构造（抽纯函数便于单测口径正确性）。
/// base 与 `list_contacts`（本文件上方）的 workspace+account filter 同源；
/// managed 在其上加 `agent_status="managed"`。AgentStatus 仅 Normal/Managed
/// 两态（models.rs），故调用方 `normal = all - managed` 精确无第三态遗漏。
fn contact_count_filters(workspace_id: &str, account_id: &str) -> (Document, Document) {
    let base = doc! { "workspace_id": workspace_id, "account_id": account_id };
    let mut managed = base.clone();
    managed.insert("agent_status", "managed");
    (base, managed)
}

/// `GET /api/contacts/counts?accountId=xxx`
///
/// 返回运营池三个 tab 的**后端真实计数** `{ all, managed, normal }`，
/// 不受 `list_contacts` 的 limit 截断影响。口径与 `list_contacts` 的
/// workspace+account filter 同源。IDOR：workspace 来自 AuthenticatedAdmin，
/// 不接受请求体 workspace。
pub(super) async fn count_contacts(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ContactQuery>,
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let (base, managed_filter) = contact_count_filters(&admin.current_workspace, &account_id);
    let all = state.db.contacts().count_documents(base, None).await?;
    let managed = state.db.contacts().count_documents(managed_filter, None).await?;
    let normal = all.saturating_sub(managed);
    Ok(Json(json!({ "all": all, "managed": managed, "normal": normal })))
}
```

注：`count_documents` 返回 `u64`，用 `saturating_sub` 防理论下溢（并发写下 all/managed 两次查询间隙的极端情形），语义上 normal 永不为负。`Document`/`doc!`/`Query`/`json!`/`Value` 均已在 contacts.rs 顶部导入（:8-13 亲验）。

- [ ] **Step 4: 挂路由 + 加 use**

`src/routes/mod.rs`：`use` 列表（:176 附近，`import_contacts_endpoint, list_contact_memory_candidates, list_contacts, ...`）里加 `count_contacts,`。
在 `/contacts`（:346）与 `/contacts/search`（:347）之间加一行（必须在 `/contacts/:id`（:352）之前，否则 `counts` 被 `:id` 通配吞掉）：

```rust
        .route("/contacts/counts", get(count_contacts))
```

- [ ] **Step 5: 跑测试确认通过 + 编译门**

Run: `cargo test --lib contact_count_filters_isolate`
Expected: PASS。
Run: `cargo check --tests`
Expected: 无错误（handler 签名与其它 handler 一致，编译通过）。

- [ ] **Step 6: Commit**

```bash
git add src/routes/contacts.rs src/routes/mod.rs
git commit -m "feat(contacts): add GET /api/contacts/counts for accurate pool counts

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 2: 前端 store — contactCounts + loadContactCounts + 修 limit

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`

**Interfaces:**
- Consumes: `GET /api/contacts/counts?accountId=`（Task 1）返回 `{ all: number, managed: number, normal: number }`；`api.get`（已导入 :22）；`useUiStore`（:24）。
- Produces: state `contactCounts: { all: number; managed: number; normal: number }`；action `loadContactCounts: (accountId: string) => Promise<void>`。供 index.tsx（Task 3）消费。

- [ ] **Step 1: 接口声明加字段**

`UserOpsState`（:29-83）里，在 `rosterCache`（:78）之后加：

```ts
  // 运营池三 tab 的后端真实计数（不受 list_contacts 的 limit 截断影响）。
  contactCounts: { all: number; managed: number; normal: number };
```

`UserOpsActions`（:85-150）里，在 `loadContacts`（:113）之后加：

```ts
  loadContactCounts: (accountId: string) => Promise<void>;
```

- [ ] **Step 2: 初值**

`create<...>` 初始 state 区（:300-341），在 `rosterCache: {},`（:338）之后加：

```ts
  contactCounts: { all: 0, managed: 0, normal: 0 },
```

- [ ] **Step 3: refreshContacts 加 limit=500**

`refreshContacts`（:286-298）里，`const params = [...]`（:290）之后、`q` 处理之前或之后均可，加一行把 limit 顶满（后端 clamp 1..500，contacts.rs:139）：

```ts
    params.push("limit=500");
```

放在 `const params = [`accountId=...`];`（:290）下一行。

- [ ] **Step 4: 实现 loadContactCounts**

在 `loadContacts`（:454-456）实现之后加：

```ts
  // 拉运营池三 tab 的后端真实计数。失败回落保留旧值（不弹错、不清零），
  // 避免网络抖动把计数瞬间清 0 误导运营。
  loadContactCounts: async (accountId) => {
    if (!accountId) return;
    try {
      const data = await api.get<{ all: number; managed: number; normal: number }>(
        `/api/contacts/counts?accountId=${encodeURIComponent(accountId)}`
      );
      set({ contactCounts: { all: data.all, managed: data.managed, normal: data.normal } });
    } catch {
      // 保留旧值，静默降级。
    }
  },
```

- [ ] **Step 5: 编译门**

Run: `cd frontend && npx tsc --noEmit`
Expected: 无错误（本 Task 只加字段/action，index.tsx 消费在 Task 3；tsc 此时应仍通过，因新增项不破坏既有类型）。

注：若 tsc 因"声明了 contactCounts 但初值/实现齐全"无报错即通过；本步不跑 build（build 在 Task 5 全量验证）。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/stores/userOpsStore.ts
git commit -m "feat(user-ops): add contactCounts state + loadContactCounts action, send limit=500

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 3: 前端 index.tsx — 计数改读 contactCounts + effect 刷新 + 拆导入接线

**Files:**
- Modify: `frontend/src/features/user-ops/index.tsx`

**Interfaces:**
- Consumes: `contactCounts` + `loadContactCounts`（Task 2）；ContactsView 改后的 props（Task 4，去掉 `importQuery`/`onImport`/`onImportQuery`）。
- Produces: 无对外新接口；本 Task 把 `managedCount`/`normalCount`/`totalCount` 数据源切到 `contactCounts`，并移除导入接线。

**注意（亲验）：** `managedCount` 有第二消费者 `TraditionalOpsTabs`（legacy.tsx:415 `${managedCount} 个运营好友`，经 index.tsx:346 传入），**不能删该变量**，只改其数据源。

- [ ] **Step 1: 从 store 解构新增项**

`index.tsx` 解构 store 的块（:71-151），在 `loadContacts,`（:124）后加 `loadContactCounts,`；在状态解构区（:100 `operationDomains,` 附近同层）加 `contactCounts,`。具体：`operationDomains,`（:101）前一行加：

```ts
    contactCounts,
```

`loadContacts,`（:124）后加：

```ts
    loadContactCounts,
```

- [ ] **Step 2: 三处派生改数据源**

`index.tsx:210-214`：

```ts
  // 计算衍生状态——计数改用后端真实 count（contactCounts），不再对已加载数组 .filter，
  // 避免 list_contacts 的 limit 截断导致计数偏小。managedCount 另有 TraditionalOpsTabs
  // 消费（下方传入），保留该派生名，仅切数据源。
  const managedCount = contactCounts.managed;
  const normalCount = contactCounts.normal;
```

删掉原 `useMemo` 版 `managedCount`（:210-213）与 `const normalCount = contacts.length - managedCount;`（:214）。`useMemo` 若不再被其它处使用，检查文件末 import（:28 `import { useMemo } from "react";`）是否还有其它 `useMemo` 调用——`filteredContacts`（:216）仍用 `useMemo`，故保留 import。

- [ ] **Step 3: totalCount 传参改 contactCounts.all**

`ContactsView` 调用处 `totalCount={contacts.length}`（:279）改为：

```tsx
            totalCount={contactCounts.all}
```

- [ ] **Step 4: 挂载/切账号 effect 追加 loadContactCounts**

`index.tsx:243-249` effect：

```ts
  useEffect(() => {
    if (effectiveAccountId) {
      setSelected(null);
      void loadContacts(effectiveAccountId);
      void loadContactCounts(effectiveAccountId);
      void loadPlaybooks(effectiveAccountId);
    }
  }, [effectiveAccountId, loadContacts, loadContactCounts, loadPlaybooks, setSelected]);
```

- [ ] **Step 5: 删导入接线**

- 删 `onImportContacts`（:227-230）整个函数。
- 删解构里的 `importQuery,`（:86）、`setImportQuery,`（:114）、`importContacts,`（:126）。
- ContactsView 调用处（:273-289）删 `importQuery={importQuery}`（:277）、`onImport={onImportContacts}`（:284）、`onImportQuery={setImportQuery}`（:285）三行。保留 `query`/`onQuery`/`onLoadAll`（过滤框仍用）。
- 保留 `FormEvent` import 若仍被其它处用；`onImportContacts` 删除后 grep `FormEvent` 在本文件是否还有引用（:1 `import type { FormEvent }`），无则删该 import。

- [ ] **Step 6: 编译门**

Run: `cd frontend && npx tsc --noEmit`
Expected: 报错仅来自 ContactsView 仍声明 `importQuery`/`onImport`/`onImportQuery` 为必填 props（Task 4 修）。若 tsc 因缺 props 报错，属预期——Task 4 完成后消失。**为保证本 Task 可独立提交**，Task 3 与 Task 4 合并为同一 commit（见 Step 7）。

- [ ] **Step 7: 与 Task 4 合并提交**

Task 3 改 index.tsx 消费端、Task 4 改 ContactsView props 契约，二者互为依赖，无法独立编译通过。**先做完 Task 4 的编辑，再一起编译 + 提交**（见 Task 4 Step 5）。本 Task 不单独 commit。

---

### Task 4: 前端 legacy.tsx ContactsView — 文案 + 删导入框 + 收窄 props

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（ContactsView，:440-544）

**Interfaces:**
- Consumes: 无（叶子展示组件）。
- Produces: ContactsView 新 props 契约——移除 `importQuery`/`onImport`/`onImportQuery`，保留 `busy`/`contactTab`/`contacts`/`managedCount`/`normalCount`/`query`/`selected`/`totalCount`/`onContactTab`/`onLoadAll`/`onOpenContact`/`onQuery`。

- [ ] **Step 1: 删解构与类型里的导入 props**

ContactsView 参数解构（:440-455）删 `importQuery,`（:444）、`onImport,`（:451）、`onImportQuery,`（:452）。
类型块（:456-472）删 `importQuery: string;`（:460）、`onImport: (event: FormEvent) => void;`（:467）、`onImportQuery: (value: string) => void;`（:468）。
保留 `onLoadAll`/`busy`/`query`/`onQuery` 等。

- [ ] **Step 2: 改 tab 文案**

`:482` `全部 {totalCount}` → `已互动 {totalCount}`
`:485` `Agent {managedCount}` 不变
`:488` `普通 {normalCount}` → `待启用 {normalCount}`

- [ ] **Step 3: 删导入 form + 改过滤框 placeholder**

删整块导入 form（:494-506，`<form className="searchRow" ...>...</form>`）。
保留过滤 `<label className="filter">`（:508-516），其 `placeholder="过滤已导入好友"`（:514）改为 `placeholder="过滤已互动"`。
`toolbar` 容器（:493 `<div className="toolbar">`）保留（仍含过滤框）。

- [ ] **Step 4: 检查 FormEvent import**

删 form 后，若 legacy.tsx 内 `FormEvent` 无其它引用则删对应 import。Run: `grep -n "FormEvent" frontend/src/features/user-ops/legacy.tsx`——若仅剩 import 行则删，否则保留。

- [ ] **Step 5: 编译门（Task 3 + Task 4 合并验证）**

Run: `cd frontend && npx tsc --noEmit`
Expected: PASS（index.tsx 消费端与 ContactsView props 契约现已对齐）。

- [ ] **Step 6: Commit（含 Task 3 改动）**

```bash
git add frontend/src/features/user-ops/index.tsx frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(user-ops): pool tabs read backend counts, rename 已互动/待启用, drop import form

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 5: 删 store 导入残留 + contactStore 死方法 + 全量前端门

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`（删 importQuery/setImportQuery/importContacts）
- Modify: `frontend/src/stores/contactStore.ts`（删死方法 managedCount/normalCount）

**Interfaces:**
- Consumes: 无。
- Produces: 无（纯删除死代码）。

- [ ] **Step 1: grep 确认 importContacts 链零剩余引用**

Run: `grep -rn "importContacts\|setImportQuery\|importQuery" frontend/src`
Expected: 只剩 `userOpsStore.ts` 里的声明/实现（index.tsx 与 legacy.tsx 已在 Task 3/4 清理）。若有其它文件引用，停下核对。

- [ ] **Step 2: 删 userOpsStore 导入链**

`userOpsStore.ts` 删：
- 接口 state `importQuery: string;`（:45）
- 接口 action `setImportQuery: (value: string) => void;`（:97）
- 接口 action `importContacts: () => Promise<void>;`（:114）
- 初值 `importQuery: "",`（:320）
- setter 实现 `setImportQuery: (value) => set({ importQuery: value }),`（:354）
- `importContacts` 实现整块（:460-487）

注：`importContacts` 实现里用到的 `useAccountStore`（:26 导入）在别处仍用（如 :461 之外的 saveXxx），删 importContacts 后确认 `useAccountStore` 仍被引用（grep），是则保留 import。

- [ ] **Step 3: 删 contactStore 死方法**

`frontend/src/stores/contactStore.ts`：
- 接口 `managedCount: () => number;`（:11）、`normalCount: () => number;`（:12）删。
- 实现 `managedCount: () => get().contacts.filter(...)`（:22）、`normalCount: () => ...`（:23）删。
- 删后 `create<ContactState>((set, get) => ({...}))` 里若 `get` 不再被使用（其余 setter 只用 `set`），把 `(set, get)` 改为 `(set)` 防 TS unused 警告 → 实为编译错误（`noUnusedParameters` 若开）。先 `grep -n "get()" frontend/src/stores/contactStore.ts` 确认无其它 `get()` 调用再改。

- [ ] **Step 4: grep 确认 contactStore 死方法零引用**

Run: `grep -rn "\.managedCount()\|\.normalCount()" frontend/src`
Expected: 无命中（界面用的是 index.tsx 的 `contactCounts`，非 contactStore 方法）。若有命中，停下核对。

- [ ] **Step 5: 全量前端门**

Run: `cd frontend && npx tsc --noEmit`
Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/stores/contactStore.ts
git commit -m "refactor(user-ops): remove pool import wiring and dead contactStore count methods

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 6: 前端测试 — 修 mock + 新增 ContactsView 文案契约测试

**Files:**
- Modify: `frontend/src/__tests__/features/user-ops/userOps.test.tsx`（`createMockStore` 补 contactCounts + loadContactCounts）
- Create: `frontend/src/__tests__/features/user-ops/contactsView.test.tsx`

**Interfaces:**
- Consumes: 真实 `ContactsView`（`frontend/src/features/user-ops/legacy.tsx`，具名导出）。
- Produces: 无。

**注意（亲验）：** 现有 `userOps.test.tsx` 把 `ContactsView` 整体 mock 成占位 div（:27-34），断言真实文案测不到。故新增独立文件渲染真实 ContactsView。

- [ ] **Step 1: 修 userOps.test.tsx 的 mock store（防崩）**

`createMockStore`（:65-165）在 `domainDrafts: {},`（:127）之后加：

```ts
    contactCounts: { all: 1, managed: 1, normal: 0 },
```

在 `loadContacts: vi.fn().mockResolvedValue(undefined),`（:143）之后加：

```ts
    loadContactCounts: vi.fn().mockResolvedValue(undefined),
```

（index.tsx Task 3 起读 `contactCounts.managed`，mock 不补会 `Cannot read properties of undefined`。）

- [ ] **Step 2: 跑现有测试确认仍绿**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/userOps.test.tsx`
Expected: 5 个既有用例全 PASS（补 mock 后 index.tsx 不再崩）。

- [ ] **Step 3: 写 ContactsView 文案契约测试**

新建 `frontend/src/__tests__/features/user-ops/contactsView.test.tsx`：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ContactsView } from "../../../features/user-ops/legacy";

describe("ContactsView 运营池", () => {
  const baseProps = {
    busy: false,
    contactTab: "all" as const,
    contacts: [],
    managedCount: 2,
    normalCount: 61,
    query: "",
    selected: null,
    totalCount: 63,
    onContactTab: vi.fn(),
    onLoadAll: vi.fn(),
    onOpenContact: vi.fn(),
    onQuery: vi.fn(),
  };

  it("三 tab 文案为 已互动 / Agent / 待启用，计数来自 props", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.getByText("已互动 63")).toBeInTheDocument();
    expect(screen.getByText("Agent 2")).toBeInTheDocument();
    expect(screen.getByText("待启用 61")).toBeInTheDocument();
    // 旧文案不得残留。
    expect(screen.queryByText(/^全部 /)).toBeNull();
    expect(screen.queryByText(/^普通 /)).toBeNull();
  });

  it("导入框已移除，只保留过滤框", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.queryByPlaceholderText("搜索并导入好友，例如 AI应用开发")).toBeNull();
    expect(screen.getByPlaceholderText("过滤已互动")).toBeInTheDocument();
  });
});
```

注：ContactsView 用到 `Search` 图标（lucide-react）与 `FormEvent` 类型，纯渲染不需 provider。若渲染报缺 context，改用最小 props 已覆盖（组件内无 store 依赖，亲验 :440-544 纯 props 驱动）。

- [ ] **Step 4: 跑新测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/contactsView.test.tsx`
Expected: 2 个用例 PASS。若 `已互动 63` 匹配不到（文案含空格/换行），用 `screen.getByText((_, el) => el?.textContent === "已互动 63")` 调整——但优先按上方直配，JSX `已互动 {totalCount}` 渲染为 `已互动 63`。

- [ ] **Step 5: 全量前端契约门**

Run: `cd frontend && npx vitest run`
Expected: 全绿（含既有 469+ 用例 + 新 2 例）。
Run: `cd frontend && npm run build`
Expected: build 成功，无 TS 错误、无悬空引用。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/__tests__/features/user-ops/userOps.test.tsx frontend/src/__tests__/features/user-ops/contactsView.test.tsx
git commit -m "test(user-ops): fix mock store for contactCounts, add ContactsView copy contract test

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 7: 全量基线门 + 收尾

**Files:** 无（仅验证）。

- [ ] **Step 1: 后端基线门**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed（新增 1 个纯函数单测，只增不减）。

- [ ] **Step 2: 后端编译门（复刻 CI step2）**

Run: `cargo check --tests`
Expected: 无错误。

- [ ] **Step 3: 前端全量门复核**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 全绿 + build 成功。

- [ ] **Step 4: 人工核对（golden path）**

按 CLAUDE.md「UI 改动需在浏览器验证」：`cargo run` 起后端 + `cd frontend && npm run dev`，打开用户运营 → 智能模式，确认：
- 三 tab 显示"已互动 N / Agent N / 待启用 N"，N 来自后端 count（切账号数字随之变）。
- 顶部无"搜索并导入好友"框，只有"过滤已互动"框。
- 通讯录 tab 不受影响（仍 4832 全量）。
- 若本地无法起 UI，明确声明"未做浏览器验证"，不假称通过。

---

## Self-Review

**1. Spec coverage**（逐条对 spec）：
- 改动1 后端 count 端点 → Task 1 ✓
- 改动2 前端读 count + limit=500 + effect 刷新 → Task 2（store）+ Task 3（index）✓
- 改动3 文案 + 删导入框 → Task 4 ✓
- 删 contactStore 死方法 → Task 5 ✓
- 测试（后端纯函数 + 前端契约）→ Task 1 Step 1 + Task 6 ✓
- 验证门（lib≥350 / check --tests / build / vitest）→ Task 7 ✓
- YAGNI 边界（不碰 webhook/RosterView/业务链路，保留 search/import 端点）→ Global Constraints + 各 Task 明确 ✓
- managed 数变动后刷新 count（batch/enable/disable）→ **补充见下**

**Gap 修补：** spec 提到"batch-enable/enable-agent/disable-agent 后追加刷新 count"。这些动作在 CockpitPanel/RosterView 里经 `refreshContacts` 更新列表，但计数是 index.tsx 层的 `contactCounts`。**最简做法**：Task 3 的挂载 effect 已在切账号时刷新；enable/disable 单个联系人后 managed 数 ±1，需即时反映。**决策（写入 Task 3 Step 4 的扩展）**：在 `openContact` 之外，给 `enableAgent`/`disableAgent`/`batchEnable` 成功回调后调 `loadContactCounts(effectiveAccountId)`。但这些 action 在 store 内（userOpsStore.ts），不易访问 index 的 effectiveAccountId。**采用更简方案**：保持 Task 3 现状（切账号刷新），并在 index.tsx 的"选中联系人变化"effect（:260-265）无关；**接受当前小限制**——enable/disable 后计数不即时更新，下次切账号/刷新页面才准。**若要即时**：Task 3 追加一个 effect 监听 `contacts` 变化时重拉 count。**结论**：为避免过度设计，Task 3 Step 4 只保证切账号刷新；即时刷新作为可选增强，实现者若发现 enable/disable 后计数不同步影响体验，追加 `useEffect(() => { if (effectiveAccountId) void loadContactCounts(effectiveAccountId); }, [contacts.length])`。此为已知取舍，非缺陷。

**2. Placeholder scan：** 无 TBD/TODO/"handle edge cases"。每个改代码步骤都有完整代码块。✓

**3. Type consistency：** `contactCounts: { all, managed, normal }` 在 store 声明（Task 2）、index 消费（Task 3）、mock（Task 6）三处字段名一致；`loadContactCounts` 签名 `(accountId: string) => Promise<void>` 三处一致；`count_contacts`/`contact_count_filters` 后端命名前后一致。✓
