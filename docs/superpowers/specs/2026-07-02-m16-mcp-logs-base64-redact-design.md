# M16 mcp_logs 落库 base64 大 payload 修复设计

> 日期：2026-07-02
> 分支：`fix/m16-mcp-logs-base64-redact`（从 origin/main e71d0c0 切,含 M1#90/M4#91/M10#94/M12#95）
> 来源：终极审判审计 M16（UPHELD Medium）

## 1. 漏洞描述（对最新代码 100% 亲验）

`mcp::logged_call`（mcp.rs:113）与 `logged_call_for_account`（mcp.rs:144）把**完整** MCP 请求参数序列化成 `request_doc`（mcp.rs:118 / 152）后，原样写进 `McpCallLog.request`（mcp.rs:133 / 175）落库 `mcp_logs`。

`media_upload_base64`（media_send.rs:148-158）经 `logged_call_for_account` 调用，其 `arguments` 携带整份文件的 base64：

```rust
json!({ "fileName": ..., "mediaType": ..., "base64": b64 })  // media_send.rs:152-156
```

`b64` 是整份素材文件读盘后的 base64（media_send.rs:137-141）。文件大小上限 `MEDIA_MAX_FILE_SIZE_MB=50`（config.rs:704），base64 膨胀 ~4/3 → **单条 mcp_logs.request 可达 ~67MB**。

### 后果（两种,都坏）

1. **超 16MB BSON 文档上限**：MongoDB 单文档硬上限 16MB。>12MB 的原文件（base64 后 >16MB）→ `insert_one` 直接失败。而两处写入都是 `let _ = ...insert_one(...).await`（mcp.rs:124 / 166）——**错误被静默吞掉**,该次 MCP 调用无任何审计留痕（审计目的落空,且无人察觉）。
2. **未超限但严重膨胀**：小于 12MB 的素材,每次上传都往高写入量的 `mcp_logs`（`ensure_media_uploaded` TTL 过期还会重传,反复写）灌几 MB base64。`mcp_logs` 是崩溃恢复热路径集合（`mcp_already_succeeded` 按 `account_id+tool_name+created_at` count,indexes.rs:511）——base64 撑大集合拖慢扫描、爆磁盘。

### base64 对审计/恢复零价值（已核实）

- 崩溃恢复 `mcp_already_succeeded`（outbox_dispatcher.rs:539）/ `media_already_succeeded`（media_send.rs:306-307）只读 `request.recipient` / `request.mediaId` / `request.content`——**从不读 `request.base64`**。
- 审计只需知道"上传了一个多大的文件",不需要原始字节。

## 2. 根因

MCP 日志层把请求参数无差别落库,未对超大二进制字段（base64）做脱敏/截断。设计时没有携带巨型 payload 的 MCP 工具,`media_upload_base64`（销售素材发送特性）加入后未同步给日志层加护栏。

## 3. 方案

### 方案 A（选定）：日志写入前按 key 脱敏 `base64` 字段

加一个纯函数 `redact_request_for_log(request: &Document) -> Document`：clone 请求 doc,把顶层 `base64` 字段的字符串值替换成占位符 `"<redacted base64: N chars>"`（保留字节数供审计"传了多大"）。两处 `logged_call` / `logged_call_for_account` 在构造 `McpCallLog { request: ... }` 时改用脱敏后的 doc。

**只脱敏 `base64` 这一个 key**（不用通用长度阈值截断）,原因（关键,已核实）：
- 崩溃恢复对 **text 发送**按 `request.content` **精确 count**（outbox_dispatcher.rs:540）。若用"长度超阈值就截断"的通用规则,一条超长 AI 回复文本的 `content` 会被截断 → `mcp_already_succeeded` 的 `request.content == content` 匹配失败 → **误判没发过 → 重发,客户收到重复消息**。这正是 outbox 幂等要防的事。
- 按 key 精确脱敏 `base64`（崩溃恢复从不读的字段）→ `content`/`recipient`/`mediaId` **一字不动**,恢复匹配零风险。
- `base64` 是当前唯一携带巨型 payload 的 MCP 字段（全仓 grep 确认,image vision 的 `imageBase64` 走 LLM 不走 MCP,不入 mcp_logs）。

### 逐场景验证

| 场景 | 当前 | 方案 A |
|---|---|---|
| media_upload_base64（含 base64） | 落 ~67MB / 超限静默失败 | request.base64 → 占位符,其它字段原样 ✓ |
| message_send_text（含 content） | 落 content | content 一字不动（无 base64 key）✓ 恢复匹配不变 |
| message_send_image（含 mediaId） | 落 mediaId | mediaId 一字不动 ✓ 恢复匹配不变 |
| 任意无 base64 的工具 | 原样落库 | 原样落库 ✓ 字节等价 |

### 否决方案 B（通用长度阈值截断）
见上：会截断超长 `content` → 破坏崩溃恢复的精确 content 匹配 → 重复发送风险。否决。

### 否决方案 C（mcp_logs 加 TTL 索引）
TTL 只延后膨胀、不解决单条超 16MB 静默失败,且不减小单次写入体积。正交问题,不在本项范围。否决。

## 4. 核心改动

落点：`src/mcp.rs`。

1. 新增纯函数 `fn redact_request_for_log(request: &Document) -> Document`（clone + 替换顶层 `base64` 值为占位符;无该 key 则等价 clone）。
2. `logged_call`（mcp.rs:127-138）与 `logged_call_for_account`（mcp.rs:169-181）构造 `McpCallLog` 时,`request: redact_request_for_log(&request_doc)`。

**不动**：真实 MCP 调用仍用未脱敏的 `arguments`/`arguments_value`（media_upload 需要真 base64,脱敏只作用于**落库副本**）;response 不含 base64 不动;crash-recovery 查询不动;model 不动。

## 5. 测试设计

新增 lib 纯函数单测到 `src/mcp.rs` 的 `#[cfg(test)] mod tests`（无需 DB/网络）：

- `redact_removes_base64_keeps_other_fields`：构造 `{fileName, mediaType, base64: "<长串>"}`,断言脱敏后 `base64` 变占位符、`fileName`/`mediaType` 原样。
- `redact_preserves_content_and_recipient`：构造 `{recipient, content: "<长文本>", }`（无 base64）,断言脱敏后 `content`/`recipient` **一字不动**（锁死"不误伤崩溃恢复精确匹配字段"这条红线）。
- `redact_noop_without_base64`：无 base64 key 的 doc 脱敏后与原 doc 相等。

### 验证
- `cargo build --lib` 无 error。
- `cargo test --lib` ≥ 350 passed / 0 failed（基线守住）+ 3 个新单测。
- 禁词 lint 通过。

## 6. 范围边界

- **只增不减**：只脱敏 base64 落库副本,真实调用与所有其它字段不变;无 base64 的调用字节等价。
- **过拟合红线**：不改崩溃恢复逻辑/幂等/阈值;修的是给日志层加超大字段护栏,精确到 base64 这个零审计价值字段,不碰恢复精确匹配的 content/mediaId/recipient。
- **YAGNI**：只脱敏当前唯一的 base64 字段,不预造通用截断框架（且通用截断有 content 误伤风险,见方案 B）。
