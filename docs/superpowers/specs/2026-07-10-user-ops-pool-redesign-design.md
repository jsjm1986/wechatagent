# 用户运营池（ContactsView）计数与文案修正设计

**日期**：2026-07-10
**状态**：设计已获批，待写实现计划
**范围**：纯 UI / 计数层修正，零业务逻辑改动

## 背景与问题

"用户运营页"的 `smart`（智能模式）左栏是 `ContactsView`（`frontend/src/features/user-ops/legacy.tsx:440-544`），标题"用户运营池 / 好友池"，含三个 tab：`全部 63 / Agent 2 / 普通 61`。用户反馈这三个数字"完全不对"，并指出它与"通讯录"（`roster` 模式 `RosterView`）功能重复。

经 100% 读码亲验，结论如下：

### 1. 用户运营池 ≠ 通讯录，是两个数据源

| | 用户运营池（ContactsView） | 通讯录（RosterView） |
| --- | --- | --- |
| 后端 | `GET /api/contacts` → `contacts` 集合 | `GET /api/contacts/roster` → MCP 全量好友 |
| 数量 | 63（运营子集） | 4832（全部微信好友） |
| 语义 | 进入运营视野的联系人 | 微信通讯录全量 |

### 2. "全部 63 / 普通 61" 不是数据错乱，是文案误导

`contacts` 集合这 63 条的来源（`src/webhooks.rs:533-536` → `upsert_webhook_contact` `src/webhooks.rs:1083-1107` 的 `$setOnInsert: agent_status:"normal"`，亲验）：

> 任何微信好友给该账号**发过消息**、且 `contacts` 里查不到时，webhook 自动建一条 `agent_status:"normal"` 记录。

所以 **63 = "来过消息的好友" + "主动导入/托管的好友" 的并集**，天然只是 4832 的一个子集。计数本身没错，错在"全部"这个 tab 文案让人误以为它该等于全部好友。

- `全部 63` = `contacts.length`（`frontend/src/features/user-ops/index.tsx:279` 传入的 `totalCount`）
- `Agent 2` = `agentStatus==="managed"` 数量（`index.tsx:210-213`）
- `普通 61` = `63 - 2`（`index.tsx:214`）

### 3. 两个真实缺陷（bug 级）

1. **limit 截断**：`list_contacts` 默认 `limit=100`（`src/routes/contacts.rs:139` `query.limit.unwrap_or(100).clamp(1, 500)`），前端 `refreshContacts` 只传 `accountId`+`q`（`frontend/src/stores/userOpsStore.ts:290-293`）不传 limit。一旦运营池联系人超过 100，"全部"永远停在 100，"普通"跟着算错。现在 63 < 100 未暴露。
2. **计数是前端对已加载数组 `.filter`**（`index.tsx:210-220`）非后端真实 count。即计数 = 被 limit 截断后的数组长度，不是真实总数。

## 已定决策（用户拍板）

1. **方向**：保留三 tab 结构，修正语义 + 修 limit（不合并进通讯录、不做纯运营工作台）。
2. **计数口径**：后端真实 count（`count_documents`），不受列表加载量影响。
3. **文案**：`全部→已互动`、`Agent 不变`、`普通→待启用`。
4. **导入框**：移除运营池顶部"搜索并导入好友"框，导入/托管统一走通讯录。

## 架构与数据流

`contacts` 集合仍是"进入运营视野的联系人"落脚点，语义不变。只改**怎么数**（后端 count）和**怎么叫**（tab 文案），并移除重复的导入入口。

三处改动，边界清晰：

### 改动 1：后端新增真实 count 端点

`src/routes/contacts.rs` 新增 handler + `src/routes/mod.rs` 挂路由：

```rust
// mod.rs：放在 "/contacts/:id" 之前，避免 counts 被 :id 通配吞掉
.route("/contacts/counts", get(count_contacts))
```

```rust
// contacts.rs
pub(super) async fn count_contacts(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<ContactQuery>,   // 复用现有结构体，只取 account_id
) -> AppResult<Json<Value>> {
    let account_id = query
        .account_id
        .unwrap_or_else(|| state.config.default_account_id.clone());
    let (base, managed_filter) = contact_count_filters(&admin.current_workspace, &account_id);
    let all = state.db.contacts().count_documents(base, None).await?;
    let managed = state.db.contacts().count_documents(managed_filter, None).await?;
    Ok(Json(json!({ "all": all, "managed": managed, "normal": all - managed })))
}

/// 抽纯函数便于单测 filter 构造正确性（workspace/account 隔离 + managed 条件）。
fn contact_count_filters(workspace_id: &str, account_id: &str) -> (Document, Document) {
    let base = doc! { "workspace_id": workspace_id, "account_id": account_id };
    let mut managed = base.clone();
    managed.insert("agent_status", "managed");
    (base, managed)
}
```

- filter 与 `list_contacts`（`contacts.rs:107-112`）的 workspace+account filter 逐字节同源，保证 count 和列表口径一致。
- `AgentStatus` 只有 `Normal`/`Managed` 两态（`src/models.rs:8-11` 亲验），`normal = all - managed` 精确无第三态遗漏。

### 改动 2：前端计数改用后端 count + 修 limit

`frontend/src/stores/userOpsStore.ts`：

```ts
// state 新增
contactCounts: { all: number; managed: number; normal: number };
// action 新增
loadContactCounts: (accountId: string) => Promise<void>;
```

- `contactCounts` 初值 `{ all: 0, managed: 0, normal: 0 }`。
- `loadContactCounts` 调 `GET /api/contacts/counts?accountId=`，失败回落保留旧值（不弹错、不清零）。
- `refreshContacts`（`userOpsStore.ts:286-298`）拼参数时加 `limit=500`（后端 clamp 1..500）——已互动列表在 500 内不被默认 100 截断。真实总数由 count 端点保证，与列表行数脱钩。
- `index.tsx` 挂载/切账号 effect（`index.tsx:243-249`）里 `loadContacts` 之后并发 `loadContactCounts(effectiveAccountId)`；`batch-enable`/`enable-agent`/`disable-agent` 等改变 managed 数的动作之后也需刷新 count（这些走 `refreshContacts`，在其后追加一次 `loadContactCounts`）。

`frontend/src/features/user-ops/index.tsx`：

```ts
// 210-214 三处派生改为读后端 count
const managedCount = contactCounts.managed;
const normalCount = contactCounts.normal;
// totalCount 传参（:279）改为 contactCounts.all
```

- **`managedCount` 有第二消费者**：`TraditionalOpsTabs`（`legacy.tsx:403-437`，meta `${managedCount} 个运营好友`）也用它。改为读后端 count 后它自然继续可用，**不能删** `managedCount` 派生。
- `filteredContacts`（`index.tsx:216-220`）保持不变——它只决定"当前 tab 显示哪些已加载行"，与计数解耦。

### 改动 3：前端文案 + 移除导入框

`frontend/src/features/user-ops/legacy.tsx` ContactsView：

- `legacy.tsx:482` `全部 {totalCount}` → `已互动 {totalCount}`
- `legacy.tsx:485` `Agent {managedCount}` 不变
- `legacy.tsx:488` `普通 {normalCount}` → `待启用 {normalCount}`
- `legacy.tsx:493-506` 整块"搜索并导入好友"`<form>` 删除，只留"过滤已互动"`<label className="filter">`（:508-516，placeholder 由"过滤已导入好友"改为"过滤已互动"）。
- 连带清理：删除后 `onImport`/`onImportQuery`/`importQuery`/`busy`（若 ContactsView 内仅导入框用）等 props。顺藤到 `index.tsx` 的 `importContacts`/`setImportQuery`/`importQuery`/`onImportContacts`（`index.tsx:226-230`），若无其他引用则删，避免死代码。**注意**：`importContacts` action（`userOpsStore.ts:460-487`）及其依赖的 `/contacts/search`+`/contacts/import` 后端端点是否保留由实现时确认——本设计只移除运营池 UI 入口，后端端点保留（通讯录批量托管走 `batch-enable`，与 search/import 独立；search/import 端点无害保留）。

### contactStore 方法（原计划误判为死方法，实为活代码——保留）

**更正（实现期 grep 全库亲验）**：`frontend/src/stores/contactStore.ts` 的 `managedCount()`/`normalCount()` **不是死方法**，被 `frontend/src/features/command-center/index.tsx:121,170` 与 `frontend/src/features/overview/index.tsx:31,33,39,54-76` 消费（驾驶舱/概览页的"运营好友 N 位"、"deliveryRate"等）。本设计初稿只在 user-ops 范围 grep，误断为死代码。**结论：保留 contactStore 这两个方法，不删。** 本次仅删真正无消费者的 `userOpsStore` 导入链（importQuery/setImportQuery/importContacts）与 ContactsView 删表单后失去使用者的 `busy` prop。

## 测试

### 后端

- **纯函数单测**（进 lib，本地可跑）：`contact_count_filters` 断言 base filter 含 `workspace_id`/`account_id`，managed filter 额外含 `agent_status:"managed"`；`normal = all - managed` 算术。计数口径正确性不依赖 DB。
- 不新增集成测试（`count_documents` 是 driver 保证，无自研逻辑）。

### 前端

`frontend/src/__tests__/features/user-ops/userOps.test.tsx`（已存在）跟进：
- 三 tab 渲染新文案（已互动/Agent/待启用），计数来自 store `contactCounts` 而非数组长度。
- 导入框已移除（`queryByPlaceholderText("搜索并导入好友")` 为 null）。
- 现有测试若断言旧文案/导入框，一并更新（契约随设计变更，非过拟合）。

### 验证门（CLAUDE.md 基线，硬性）

1. `cargo test --lib` ≥ 350 passed / 0 failed（新增纯函数单测只增不减）。
2. `cargo check --tests`（删 props 可能牵连编译，复刻 CI baseline step2）。
3. `cd frontend && npm run build`（TS 编译 + 死代码清理后无悬空引用）。
4. `cd frontend && npx vitest run`（前端契约门）。

## 不做的事（YAGNI）

- 不改 webhook 自动建档逻辑（normal 沉淀是对的）。
- 不改通讯录 RosterView（已正确）。
- 不加分页（运营池 63 条，过度设计）。
- 不改 `operation_state` / 画像 / 记忆 / 状态机等任何业务链路。
- 保留 search/import 后端端点（仅移除运营池 UI 入口）。
