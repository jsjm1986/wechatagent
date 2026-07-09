# 通讯录切 contacts_fetch_full + 富化字段 + 分页

> 日期：2026-07-09 · 分支：feat/roster-fetch-full · 状态：设计（待写计划）

## 背景与问题

通讯录（`RosterView`）当前经 `mcp::fetch_roster_for_account` 调 MCP 工具 **`contacts_fetch_cache`** 拉全量好友。该工具就绪返回 `{result:{friends:[wxid字符串]}}` —— **只有一串 wxid，没有昵称/头像/备注**，导致通讯录卡片只能显示 wxid，头像恒缺。

MCP server 另有具名工具 **`contacts_fetch_full`**（117 亲验存在），返回全量富化字段。切到它可一次拿齐昵称、头像、备注、性别等。

## 已亲验事实

### 117 单发探测（账号 t-1，2026-07-09）— `contacts_fetch_full` 真实返回

- **信封**：`{status:"ready", items:[...], count:4831, fetchedAt, refreshing:true, lastError:null}`
  - ⚠️ **`refreshing:true` 却带着全量 4831 条数据** —— `refreshing` 不能当「未就绪/空态」判据；就绪信号是 `status=="ready"` 或 `items` 非空。
- **单条 item 字段**：`userName`(=wxid)、`nickName`、`remark`、`alias`、`bigHeadImgUrl`/`smallHeadImgUrl`(头像)、`sex`(整数 0/1/2)、还有 signature/city/labelList 等。

### 当前 main 代码（亲验）

- `RosterFriend`（src/mcp.rs:411）：`wxid / nickname / remark / avatar_url` —— **无 sex**。
- `parse_roster_items`（src/mcp.rs:476）头像键候选 = `["bigHeadImg","smallHeadImg","headImgUrl","avatarUrl","headimgurl"]` —— **缺 `bigHeadImgUrl`/`smallHeadImgUrl`**，即便不切工具，头像也会漏（必修）。
- `fetch_roster_for_account`（src/mcp.rs:564）：调 `contacts_fetch_cache`，`{}` 无参，3×2s 短重试；`roster_outcome_from_result`(mcp.rs:558) 用 `roster_result_is_empty_cache`（`{}`/Null/无数组候选 → 空 cache→syncing）判就绪。
- `roster_endpoint`（src/routes/contacts.rs:367）输出 `{wxid,nickname,remark,avatarUrl,agentStatus}`。
- `batch_enable_endpoint`（src/routes/contacts.rs:547）：候选带值才 `$set` nickname/remark/avatar_url（避免覆盖已入库数据）。
- `BatchEnableCandidate`（src/models.rs:3203）：`wxid/nickname/remark/avatar_url` —— 无 sex。
- `Contact`（src/models.rs:140）：有 avatar_url，**全模型无 sex/性别/gender**。
- 前端 `RosterEntry`（types/index.ts:134）：`wxid/nickname/remark/avatarUrl/agentStatus` —— 无 sex。
- `RosterView`（features/user-ops/RosterView.tsx）：grid 平铺全部好友，**无分页**、`<img>`(:211) **无 `loading="lazy"`**、无性别展示。
- 分页范式：`PANEL_PAGE_SIZE=20` + `usePagedList<T>`（features/system-strategy/index.tsx:148-156），复用 CampaignBoard 范式。

## 设计决策（已与用户对齐）

1. **全切 `contacts_fetch_full`**（不保留 cache 双路）。
2. **保留 3×2s 短重试**（就绪判据改为 `status`/`items`，不再依赖 `{}`）。
3. **性别透传 + 存进联系人画像**：MCP `sex`(int) 一路透传到前端与 batch-enable 落库。
4. **性别显示为文字**：0→未知、1→男、2→女（前端转换，存储保留原始 int 忠于源）。
5. **同时做分页**：4831 好友无分页会渲染 4831 个卡片按钮 + 4831 个头像请求，切 full 后立刻暴露。复用 `usePagedList`/`PANEL_PAGE_SIZE`。

## 实现范围

### 后端 `src/mcp.rs`
- `RosterFriend` 加 `sex: Option<i32>`。
- `fetch_roster_for_account`：工具名 `contacts_fetch_cache` → `contacts_fetch_full`（`{}` 无参，account_alias 由 `logged_call_for_account` 自动注入）；保留 3×2s 重试。
- `parse_roster_items`：
  - 头像键候选补 `bigHeadImgUrl`/`smallHeadImgUrl`（放前列，其余保留兜底）。
  - 提取 `sex`（`obj.get("sex").and_then(as_i64) as i32`）。
  - `/items` 已在命名数组候选内，`contacts_fetch_full` 顶层 items 直接命中，无需新增路径。
- **就绪判据改造**：`contacts_fetch_full` 空态是 `{status:!="ready" 或 items:[]}`，不再是 `{}`。改 `roster_result_is_empty_cache`（或 `roster_outcome_from_result`）：`syncing = items 为空 && status != "ready"`。保留铁律「解析出任何好友一定 syncing=false」。

### 数据模型 / 落库
- `Contact` 加 `sex: Option<i32>`（dedicated 字段，与 avatar_url 同级；非 AI 推断，不进 profile_attributes）。
- `BatchEnableCandidate` 加 `sex: Option<i32>`。
- `batch_enable_endpoint`：候选带 sex 才 `$set`（镜像 avatar_url 的「带值才写」保护）。

### API 层
- `roster_endpoint` items 输出补 `sex`。

### 前端
- `RosterEntry` 类型加 `sex?: number | null`。
- `RosterView`：
  - 接入 `usePagedList`（每页 20，filter 后分页）+ 页码控件。
  - `<img>` 加 `loading="lazy"`。
  - 卡片展示性别文字（男/女/未知）。
  - `onSubmit` 的 candidates 透传 `sex`。

## 风险 / 注意

- **就绪判据是最易错处**：`refreshing:true` 是干扰项，务必用 `status=="ready"`/`items` 判，否则前端会无限 8s 重拉且每次清空运营勾选。
- MCP 测试须串行、勿撞 429（生产 LLM/MCP 端点并发受限）。
- 现有 `parse_roster_items` 单测（mcp.rs:697+）大量针对 `contacts_fetch_cache` 字符串数组形态 —— **只增不删**（保留旧形态回归守卫，新增 `contacts_fetch_full` 富化形态用例）。

## 测试计划

- `parse_roster_items` 新增用例：`contacts_fetch_full` 信封（items + bigHeadImgUrl + sex）解析出富化字段；`status:"ready"`+空 items → 真 0 好友（不重试）；非 ready + 空 items → syncing。
- 头像键 `bigHeadImgUrl`/`smallHeadImgUrl` 命中回归。
- 前端 roster.test 补分页 + 性别文字断言。
- 基线门：`cargo test --lib` ≥350、4 PBT ≥33。
