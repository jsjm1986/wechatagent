# 决策人链改用通讯录选择器 设计

> 频道：请示通道配置（`features/ask-human-config`）。纯前端改动，不动后端。

## 1. 问题

「+ 从联系人添加」名不副实：它拉的是 `/api/contacts`（本地 `contacts` 表已入库的联系人），不是微信通讯录。用户在通讯录里有的好友，这里可能选不到，且交互是挤在表单里的纯文本列表，无头像、无分页（`limit=100` 一次铺开）。

参考对象：「专属顾问名片库」的「从好友选择」，用共享组件 `components/ui/FriendPickerModal`（头像卡片网格、三字段搜索、`PAGE_SIZE=60` 分页）。

## 2. 决定实现方式的后端约束（均已读实现核实）

### 2.1 决策人必须已在本地通讯录（硬约束）

`put_ask_human_policy`（`src/routes/domains.rs:237`）对链中每人做两道 fail-closed 校验：

1. `accountId` 必须属于当前 workspace（查 `accounts` 集合）
2. `(workspace_id, account_id, wxid)` 必须存在于 `contacts` 集合，否则报「决策人 X 不在账号 Y 的通讯录中」

前端绕不过。这是本设计所有复杂度的来源。

### 2.2 两个数据源语义不同

| 端点 | 返回 |
| --- | --- |
| `/api/contacts`（现用） | 本地 `contacts` 表已入库联系人，按 `last_inbound_at` 排序，过滤 `hidden_from_pool` |
| `/api/contacts/roster`（名片库用） | 微信通讯录**全量快照**，每条带 `agentStatus: "managed" \| "normal" \| "not_imported"` |

roster 中 `not_imported` 的好友**不在** `contacts` 表 → 直接换数据源会导致「从通讯录选了人，保存时后端说不在通讯录中」。

### 2.3 「只导入不托管」的路径存在，故无需改后端

`POST /api/contacts/import`（`src/routes/mod.rs:346`）→ `upsert_contact_from_value`（`src/routes/shared.rs:264`）：

```rust
"$setOnInsert": { ..., "agent_status": "normal", ... }   // 不是 managed
```

只补身份字段（nickname/remark/alias）+ upsert，不建任务、不落 `operation_state`、不进运营池。传 `candidates` 时不触发 MCP 调用，纯数据库写入。自带 `is_operatable_person` 守卫。

**该端点前端目前零使用**（全库 grep 确认），但已存在且可用。

对比：`POST /api/contacts/batch-enable`（`src/routes/contacts.rs:1522`）的 `set_doc` 无条件写 `"agent_status": "managed"`，还落 `operation_state` 初始态、建 enrollment intent——把内部决策者当客户交给 AI 运营，语义不对，**不可用于本场景**。

字段映射核实：roster 的 `{wxid, nickname, remark}` 对上 `contact_identity_patch` 读的键（`nickName`/`nickname`、`remark`、`alias`）。`avatarUrl` 不在该 patch 内，导入时头像不落库——影响小，别处读时兜底富化会补（`list_contacts` 的 roster_identity 逻辑）。

## 3. 两个会导致「看起来成功但没生效」的坑

### 3.1 HTTP 200 不代表导入成功

`import_contacts_endpoint:471` 是 `if let Some(contact) = upsert(...)`——`upsert` 返回 `None` 时**静默跳过**，接口仍回 200，`items` 为空数组。

**故必须检查 `items.length > 0`，不能只看状态码。**

### 3.2 roster 的非真人判据比后端宽松

```
roster.isNonHuman = item_type=="system" || is_system_account(wxid)       // src/mcp.rs:761
后端 import 守卫  = !(gh_ 前缀 || @chatroom || @openim || is_system_account)  // src/webhooks.rs:1948
```

公众号（`gh_`）、群（`@chatroom`）、企业号（`@openim`）只在 `item_type=="system"` 时才被 roster 标 `isNonHuman`，否则显示为可选——但 import 会静默拒绝。

**故前端过滤须双重：`isNonHuman` + 后端同款 wxid 规则。**

注：`wxid_*` 开头的媒体号（如「福州晚报」`wxid_8874178741811`）两边都放行，靠人工排除，不在此拦——与既有语义一致，不改。

## 4. 设计

### 4.1 数据源与状态

`DeciderChainEditor` 改用 `useUserOpsStore` 的 `loadRoster(accountId)` + `rosterCache`（带缓存、跨挂载存活），替代自持 `useState<Contact[]>` + 直接 `api.get`。

`accountId` 改从 `useAccountStore(s => s.currentAccountId())` 取，不再从 `Contact.accountId` 摸——roster 本身按账号拉取，两者天然一致。

### 4.2 syncing 态必须处理（抄 RosterView，不抄 referral-cards）

roster 首次无快照时返回 `items: []` + `syncing: true`，后台单飞异步拉取。

- `referral-cards/index.tsx:37` 只有 `void loadRoster(...)`，**未处理 syncing** → 选择器空白，用户以为没好友。**不抄这里。**
- `RosterView.tsx:88-99` 有完整方案：`syncing` 时每 10s 自动重拉（不带 `force`，只读快照）+ 请求序号守卫（`reqSeqRef`，防快速切账号时旧响应覆盖新列表）。**抄这里。**

选择器在 syncing 时显示「正在同步通讯录…」，不显示空态。

### 4.3 交互：复用 FriendPickerModal

`components/ui/FriendPickerModal`（140 行，已是共享组件，`products-deals` 与 `referral-cards` 在用）：头像/首字母兜底、`remark || nickname || wxid` 三字段搜索、`PAGE_SIZE=60` 分页、`badge` 字段。

- `badge`：未入库的好友标「未导入」（入库的不标）
- `allowManualWxid`：**关闭**。手输不在通讯录的 wxid 会前端放行、后端拒绝，正是要消除的体验
- 已在链中的 wxid 从候选排除（保留现有行为）

### 4.4 选中未入库好友时的导入流程

```
选中 → agentStatus === "not_imported" ?
        ├─ 否 → 直接加入链
        └─ 是 → POST /api/contacts/import
                  { accountId, candidates: [{ wxid, nickname, remark }] }
                ├─ items.length > 0 → 加入链
                └─ items 空 / 抛错 → toast 明确失败原因，不加入链
```

导入成功后**不**强制刷新 roster：`agentStatus` 变化不影响已选中项的正确性，且刷新会打断用户连续添加多人的操作。下次自然刷新时 badge 自会消失。

导入是 upsert，重复点击幂等，无需额外去重。

### 4.5 错误呈现

现有 `loadError` 内联错误态保留（`AskHumanConfig.module.css` 的 `.loadError`）。导入失败用 `useToast`——该频道已有 `ToastProvider`（`index.tsx:205`），但 `DeciderChainEditor` 当前未用 toast，需引入 `useToast`。

## 5. 测试

`__tests__/features/ask-human-config/DeciderChainEditor.test.tsx` 现有 6 个用例全部 mock `api.get` 返回 `/api/contacts` 形状并断言「从联系人添加」文案，**需重写**。

新覆盖：

1. 打开选择器渲染 roster 好友（非 `/api/contacts`）
2. 已入库好友（`agentStatus: "normal"`）选中后直接入链，不调 import
3. 未入库好友（`not_imported`）选中后先调 import，再入链
4. import 返回空 `items` → 不入链 + 报错（坑 3.1）
5. 公众号 `gh_*` / 群 `@chatroom` 不出现在候选（坑 3.2）
6. `isNonHuman` 好友不出现在候选
7. 已在链中的 wxid 从候选排除（保留旧行为）
8. 删除 / 上移下移仍工作（保留旧行为）
9. syncing 时显示同步中文案而非空态

jsdom 无布局引擎、不跑 CSS 层叠——弹窗视觉、头像网格、分页观感需目视确认。

## 6. 不做

- 不动后端任何文件
- 不改 `FriendPickerModal` 组件本身（`badge` / `allowManualWxid` 都是既有 props）
- 不改 `put_ask_human_policy` 的校验语义
- 不动决策人链的排序/删除逻辑与 `policyForm.ts` 校验
- 不修 `referral-cards` 缺失的 syncing 处理（同类缺陷，但不在本次范围；如需修另开）

## 7. 需目视核验

- 弹窗形态、头像网格、搜索与分页可用
- 「未导入」badge 显示正确
- 选中未入库好友后能成功入链，链中显示正常
- 保存后不再出现「决策人不在通讯录中」错误
- 通讯录首次同步时显示同步中文案
