# outbox 发送幂等：timeout 兜底核对源升级为 chat_search

> 日期：2026-07-10 · 分支：feat/outbox-chat-search-idempotency · 状态：设计（待写计划）

## 背景 / 问题（有实锤）

dispatcher 对整条 send 有 150s 外层 timeout（`outbox_dispatcher.rs:52` `MCP_SEND_TIMEOUT_SECONDS`）。当 MCP 已把消息发给微信、但回包慢于 150s 时，`tokio::time::timeout` 取消整个 send future（`:774`），走 `Err(_)` 分支（`:828`）。此时：

- send future 被取消，`send_outbound_message` 里 MCP `.await`（`gateway.rs:2936`）之后的代码（提取 newMsgId、写库）都没执行；
- `logged_call_for_account`（`mcp.rs:329`）里 `mcp_call_logs` 的 `insert_one`（`:352`）在 MCP `.await`（`:338`）**之后**——future 被取消时这条日志**没写成**；
- 现有 timeout 兜底 `mcp_already_succeeded`（`outbox_dispatcher.rs:543`）查的正是**本地 `mcp_call_logs`** → 查空 → 判定"没发过" → 重发 → **客户收到重复消息**。

**实锤**（2026-07-10 chat_search 拉真实客户吴界 `wxid_ydzaomn4scsb12` 对话历史）：07-09 `12:11:46` / `12:13:52` / `18:50:31` 三条**完全相同**的 outbound `"这个我帮你确认一下，稍等我给你准信。"`——重复发送真实发生过。

## MCP server 已更新（实测确认，2026-07-10 用 filehelper 零打扰探测）

单条 send 工具现返回**统一成功信封**（desc 原文：`Returns a send receipt { ok, submitted, newMsgId, delivery, sentAt, target }`；`All single-message send tools share this receipt shape`）：

- 成功：`ok:true` + `submitted:true` + 非空 `newMsgId` + `delivery:"submitted"` + `sentAt`（新字段**叠加**在旧字段 target/msgId/aesKey 之上，向后兼容）。
- `delivery` 恒 `"submitted"` = 交给微信服务器，**不代表送达/已读**（GeWe 无送达回执，desc 明示）。故本方案**不追踪送达**，只判"是否提交成功"。
- 失败（掉线/风控/无效 recipient）desc 称抛 tool error；`call_tool_with_key`（`mcp.rs:189`）已把 `result.isError=true` 转 `AppError::External`。

**chat_search 工具**（desc 官方指认做发送审计："To verify a specific outbound send, use direction=outbound + peer + content_contains"）：

实测的关键事实（全部 filehelper 探测）：
1. **同步落库**：send 返回 `ok:true` 后 **0.02s** chat_search 即可查到。无"已发但查不到"的延迟窗口。
2. **失败不写记录**：无效 recipient 的 send → chat_search `count=0`。故"查到 outbound 记录 ⟺ 真的提交成功"。
3. **content 匹配可靠**：content 未被 server 改写，原文可 `content_contains` 命中。
4. **同内容发 2 次 → 2 个不同 newMsgId，chat_search count=2**；每条 item 的 `sourceMsgId` = 对应的 newMsgId。→ 纯 content 匹配只能答"发过≥1 次"，`newMsgId`/`sourceMsgId` 才是精确单条锚点。
5. chat_search item 字段：`{id, direction, peerWxid, chatroomId, content, msgType, sourceMsgId, toolName, createdAt}`。返回 `{items, count}`。

## 设计决策（已与用户对齐）

1. **timeout 兜底核对源升级**：`Err(_)` 分支优先查 MCP `chat_search`（server 侧真实已发记录），命中即确认已发、标 sent 不重发；chat_search 调用失败/异常 → **回落**现有 `mcp_already_succeeded`（本地 `mcp_call_logs`），不倒退。
2. **核对精度（防历史误命中）**：两步——`content_contains` 做**服务端初筛**拉候选，再在**客户端精确等于**判命中。即 `chat_search(direction="outbound", peer=contact_wxid, content_contains=entry.content, since=entry.created_at, limit=N)` 拉回候选，命中判据 = items 里存在一条 **`item.content == entry.content`（精确等于，非子串）** 且 `item.createdAt >= entry.created_at`。精确等于防"你好"被历史"你好呀"误命中；`since=entry.created_at`（已有字段，无需发送前额外记基线）排除本 entry 创建前的历史同内容消息。
3. **零重发零漏发论证**（timeout 时 MCP 真实状态只有两种）：
   - 已 accepted（回包慢）→ chat_search 查到新记录（同步落库）→ 标 sent → **不重发** ✅
   - 未 accepted（网络断/未处理）→ chat_search 查不到 → 重试 → **不漏发** ✅
   - 残留风险：仅当 chat_search 调用**本身**失败 → 回落本地日志 → 都查不到则保守重试（偏"重发"而非"漏发"，重发可挽回、漏发不可）。要 chat_search + 本地日志同时失效才触发，概率极低。
4. **不追踪送达**：`delivery` 恒 submitted，方案只判"提交成功"，不做 delivered/read（GeWe 做不到）。
5. **本轮范围**：只做 text 主链路（`entry.referral_card_id`/`media_asset_id` 均无的纯文本条目）。媒体走 `media_already_succeeded`（查 media_id）、名片现放行重发——本轮不改（YAGNI，text 覆盖绝大多数发送；媒体/名片留后续专项）。
6. **chat_search 单独短超时**：timeout 兜底里调 chat_search 用独立短 timeout（`CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS`，默认 15s），失败即回落本地日志，不让核对本身卡死 dispatcher。

## 实现范围

### `src/mcp.rs`
- 新增只读查询封装 `pub async fn chat_search_outbound(state, account_id, peer, content, since) -> AppResult<bool>`：
  - 调 `logged_call_for_account(state, account_id, "chat_search", json!({...}))`（复用统一封装，account_alias 自动注入）。
  - 参数：`{"direction":"outbound","peer":peer,"content_contains":content,"since":since_iso,"limit":20}`。`since` 用 `entry.created_at` 转 ISO-8601 字符串。
  - 解析返回体 `items[]`：存在一条 `item.content == content` 且 `item.createdAt >= since` → 返回 `Ok(true)`；否则 `Ok(false)`。
  - 调用失败（`Err`）向上抛（由调用方决定回落，不在此吞）。

### `src/agent/outbox_dispatcher.rs`
- 新增常量 `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS: u64 = 15`。
- timeout 分支（`:828` `Err(_)`）的 text 核对（`:848` 现调 `mcp_already_succeeded` 处）改为：
  1. 先 `tokio::time::timeout(CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS, mcp::chat_search_outbound(...))`。
  2. `Ok(Ok(true))` → 已发，标 sent（复用现有 `:857` 起的标 sent 逻辑，last_error 文案改为 "confirmed via chat_search"）。
  3. `Ok(Ok(false))` → 确认没发 → schedule_retry_or_terminal 重试。
  4. `Ok(Err(_))`（chat_search 出错）/ `Err(_)`（chat_search 超时）→ **回落** `mcp_already_succeeded`（本地 mcp_call_logs），沿用其结果决定标 sent 或重试。
- 媒体/名片分支不变（本轮不动）。

### `src/agent/gateway.rs`（可选小加固，本轮一并做）
- `send_outbound_message`（`:2946`）成功判断：现靠"提取 newMsgId 是否存在"，改为显式读 `response.get("ok") == Some(true)` 优先，`newMsgId` 存在作兼容 fallback（新旧信封都能判"提交成功"）。不改变返回类型与既有落库逻辑。

## 测试计划

- `src/mcp.rs` 纯函数化解析：抽 `chat_search_hit(items: &Value, content: &str, since_millis: i64) -> bool` 便于单测——覆盖：content 精确等于命中、content 子串不等于不命中（"你好"不被"你好呀"误命中）、createdAt < since 不命中（历史同内容排除）、空 items 不命中。
- `src/agent/gateway.rs`：`send_outbound_message` 成功判定纯函数化 `send_receipt_is_ok(response: &Value) -> bool`——覆盖：`{ok:true}` 命中、`{ok:false}` 不命中、无 ok 但有 newMsgId（旧信封兼容）命中、都无不命中。
- 基线门：`cargo test --lib` ≥ 350 passed / 0 failed。
- 集成层（`#[ignore]` Docker）：timeout 兜底 chat_search 命中标 sent / 未命中重试 / chat_search 失败回落本地日志——用 mock MCP 返回体驱动。

## YAGNI（明确不做）
- 媒体/名片的 chat_search 核对（本轮只 text；媒体有 media_id 核对、名片重复危害小）。
- 送达/已读追踪（GeWe 无回执）。
- 发送前置游标记录（用 entry.created_at 当基线已足够排除历史误命中，不需每次发送多一次查询）。
- 发送前置幂等预检（每次发送前查 chat_search）——本轮只在 timeout 兜底查，正常路径不加额外 MCP 调用。

## 部署提醒
纯后端改动（Rust）。MCP server 侧统一信封已由用户上线（实测确认生效）。部署走 `cargo build --release` + `systemctl restart`（见 [[deploy-server-117]]）。无前端改动、无新集合/索引。
