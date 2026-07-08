# 通讯录 roster 修复设计（contacts_fetch_cache 形态对齐 + 昵称头像懒加载 + 空态处理）

**日期**：2026-07-08
**分支**：`fix/roster-fetch-cache-shape`（基于 origin/main）
**状态**：设计已获批，待写实现计划

## 背景与症状

前端「用户运营 → 通讯录」页恒显「暂无好友」空态，即便 t-1 账号真实有几百个微信好友。

## 根因（2026-07-08 线上 117 直连 MCP server 亲验，非猜测）

链路：`前端 RosterView → GET /api/contacts/roster → roster_endpoint(contacts.rs:367) → mcp::fetch_roster_for_account(mcp.rs:504) → parse_roster_items(mcp.rs:447)`

**根因 A（形态不匹配，必然失败，核心）**：`contacts_fetch_cache` 就绪时真实返回：

```json
{ "result": { "friends": ["medianote", "wxid_8874178741811", "wxid_2o93p4cc9n4x22", ...] } }
```

即**纯 wxid 字符串数组，嵌套在 `result` 键下**。而 `parse_roster_items`：

- 数组候选 pointer 路径是 `/contacts /friends /list /items /data /structuredContent/*`，**没有 `/result/friends`**（friends 在 result 下一层，顶层无 friends）→ 找不到数组；
- 即便命中，元素解析要求 `item.as_object()?`（mcp.rs:489）再取 wxid/nickName 键，但真实元素是**纯字符串**，`.as_object()` 恒 None → 全被 filter 掉 → 空 vec；
- `contact_like_array` 兜底（mcp.rs:436）也要 object 且含 wxid 键，字符串数组同样不命中。

→ 解析恒空，与 cache 是否有数据无关。mcp.rs 注释也自认「数组 key 未线上核实（测试账号缓存为空）」——**这个解析器从未跑通过真实数据**。

**根因 B（cache 异步空态，偶发）**：`contacts_fetch_cache` 多数调用返回 `structuredContent:{}`（content[0].text 也是 "{}"），isError:None，无错误。GeWe 缓存异步就绪：偶发返回全量，大多空。空对象被前端当成「真的没好友」显示成「暂无好友」。

**限流约束**：连续高频探测会撞 **HTTP 429 Too Many Requests**。同一 MCP server 也承载线上 `message_send_text` 发送，任何补详情逻辑必须自带节流、不能高频猛打。

**排除项（全部亲验非根因）**：凭证正确（t-1=account_id 102，mcp_base_url=http://117.72.54.28:3001 与线上 .env 一致，gwa_ workspace key，online:true）；权限正确（account_get_status.allowed_tools 含 contacts_fetch_cache/contacts_search/contact_get_detail）；工具选择正确（136 工具里唯一「fetch full contacts」就是 contacts_fetch_cache，无独立 sync/pull 工具）；二进制最新（线上 61c51ae 含 roster）。

**friends 只有 wxid，无昵称/头像/备注**——那些需靠 `contact_get_detail`（单条工具）另取。`contacts_search` 实测 TimeoutError（GeWe 侧慢，不宜做主力）。

## 设计（四模块）

### 模块 1：解析器修复（必做核心）

改造 `src/mcp.rs::parse_roster_items`：

- 数组候选路径**新增嵌套**：`/result/friends`、`/result/contacts`、`/result/list`（真实形态在 result 下）；保留原顶层候选向后兼容。
- 元素解析**支持两种形态**：
  - 纯字符串 → 当作 wxid（nickname/remark/avatar_url 留 None）；
  - 对象 → 沿用现有 wxid/nickName/bigHeadImg 提取。
- `contact_like_array` 内容识别兜底新增「纯字符串数组」分支：数组首元素是 string 即视为 wxid 列表。
- 新增单测：`{result:{friends:["wxid_a","wxid_b"]}}` → 解析出 2 条 wxid-only；混合对象/字符串数组也覆盖。

**产出**：cache 有数据时 roster 至少能出 wxid 列表（配前端首字母头像兜底即可用）。

### 模块 2：空 cache 处理（后端短重试 + 前端「同步中」提示）

后端 `fetch_roster_for_account`（mcp.rs:504）：

- 调 `contacts_fetch_cache`，若解析列表为空**且**返回体为空 `{}`（区别于「真 0 好友」），**同一请求内**短重试：间隔 ~2s、最多 3 次（总 ~6s，控制在 HTTP 可接受时长内）；复用同一 MCP session，间隔足够避免自撞 429。
- 返回结构带状态标志。`roster_endpoint` 响应体加字段：`{items, total, syncing}`——`syncing:true` 表示 cache 未就绪（空 `{}` 且重试仍空）；`syncing:false` 表示已就绪（含真 0 好友）。

前端 `RosterView` + 类型 `RosterResponse` 加 `syncing?: boolean`：

- `syncing:true` 且 items 空 → 显示「正在从微信同步好友，稍候自动刷新…」，并自动定时重拉（每 ~8s，最多数次）。
- `syncing:false` 且空 → 才显示「暂无好友」。

### 模块 3：昵称头像懒加载 + 持久缓存

**数据落点**：复用 `contacts` 集合（现有 `upsert_contact_from_value` 已存 normal 好友的 nickname/remark，agent_status=normal 不污染 managed 语义）。扩展存 `avatar_url` + 新增 `detail_fetched_at` 时间戳（标记已补详情，避免重复拉）。

**新增后端端点** `POST /api/contacts/roster/detail`：

- 入参 `{accountId, wxids: [...]}`（前端控制批量大小，如每批 10）。
- 每个 wxid：先查本地 contacts 有无缓存详情（有且未过期→直接返回）；无→调 `contact_get_detail`，解析昵称/头像/备注，upsert 落库（agent_status 保持既有，不动 managed）。
- **自带节流**：批内串行 + 每次调用间隔（~300ms），并发=1；单个失败不中断整批（降级 wxid-only）。
- 返回 `{items:[{wxid, nickname, remark, avatarUrl}...]}`。

**前端 RosterView**：

- 先出 wxid 列表（模块1），首字母头像兜底。
- **可视区按需拉**：IntersectionObserver 观察已渲染但无详情的卡片，分批（每批 ~10）调 detail 端点，拿到就地更新卡片昵称头像。
- 已缓存的（本地 contacts 有 nickname）不再拉。

**降级保证**：detail 端点整体失败/超时，列表仍是可用 wxid 列表，不阻塞。

**实现阶段待验**（限流恢复后串行单发）：`contact_get_detail` 精确返回字段（昵称/头像 key 名）。设计上先按现有 parse_roster_items 的 key 候选（nickName/bigHeadImg/smallHeadImg 等）。

### 模块 4：缓存策略 + 限流兜底

- **详情缓存 TTL**：`detail_fetched_at` 控制，缓存长期有效（~7 天）；刷新按钮强制重拉当前可视区。
- **429 兜底**：detail 端点某次调用收到 429 → 停止本批后续调用，已拿到的正常返回，未补的保持首字母兜底（下次滚动/刷新再试）；不做无限重试。
- **列表与详情分离**：detail 挂了不影响主列表可看。
- **账号切换**：复用现有请求序号守卫（reqSeqRef），切账号时进行中的 detail 批次作废。

## 测试

- 模块1：`parse_roster_items` 新增单测——`{result:{friends:[字符串]}}` 形态、混合数组、空 `{}`。
- 模块2：`fetch_roster_for_account` / `roster_endpoint` 后端测（mock MCP 返回空 `{}` → syncing:true；返回全量 → items 非空）。
- 模块3：detail 端点测（mock contact_get_detail；缓存命中不重复调；单个失败整批不崩）。
- 前端 vitest：syncing 态渲染「同步中」；detail 补全就地更新卡片；复用切账号守卫回归。
- 基线门不回退（cargo test --lib ≥350 + 4 PBT ≥33）；no-human-takeover lint 新增行无禁词。

## 诊断产物

`scripts/diag/probe_mcp_*.py`（tools_list / call / detail / retry / ready / detail_single）——只读探测 MCP server，保留供后续复用。

## 影响面

- `src/mcp.rs`：parse_roster_items 改造 + fetch_roster_for_account 短重试。
- `src/routes/contacts.rs`：roster_endpoint 加 syncing 字段 + 新增 roster detail 端点。
- `src/routes/mod.rs`：挂新路由。
- `src/routes/shared.rs`：upsert 扩展 avatar_url + detail_fetched_at（或新 helper）。
- `src/models.rs`：Contact 加 detail_fetched_at（若需）。
- 前端 `RosterView.tsx` / `userOpsStore.ts` / `types/index.ts`：syncing 态 + 可视区懒加载。
