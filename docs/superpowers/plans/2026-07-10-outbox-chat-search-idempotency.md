# outbox 发送幂等：timeout 兜底核对源升级为 chat_search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** dispatcher timeout(150s) 兜底核对源从"本地 mcp_call_logs"升级为"优先查 MCP chat_search(server 真实已发记录)+本地日志 fallback"，根治 timeout 取消 send future 致本地日志缺失、误重发的问题。

**Architecture:** 新增 `mcp::chat_search_outbound` 只读查询封装 + 纯函数 `chat_search_hit` 判命中(content 精确等于+since 时间窗)。outbox_dispatcher 的 timeout 分支(Err(_))先带 15s 短超时查 chat_search，失败/异常回落现有 `mcp_already_succeeded`(本地日志)。gateway `send_outbound_message` 成功判定加读新信封 `ok` 字段(向后兼容旧信封)。只做 text 主链路。

**Tech Stack:** Rust (Axum) + serde_json + mongodb::bson::DateTime + tokio

设计文档：`docs/superpowers/specs/2026-07-10-outbox-chat-search-idempotency-design.md`

## Global Constraints

- 基线门：`cargo test --lib` ≥ 350 passed / 0 failed。测试只增不删。
- 本机磁盘紧：只跑 `cargo test --lib`（按名过滤更好），**绝不**跑 `cargo test` 全量/集成测试（爆盘）。
- MCP 已上线统一成功信封(实测)：单条 send 成功返回 `{ok:true, submitted:true, newMsgId, delivery:"submitted", sentAt, ...旧字段}`；失败抛 tool error(→`AppError::External`)。
- chat_search 实测事实：同步落库(send 后 0.02s 可查)；失败发送不写 outbound 记录；item 字段 `{id,direction,peerWxid,content,msgType,sourceMsgId,toolName,createdAt}`；content 未被改写。
- 核对精度铁律：`content_contains` 服务端初筛 + 客户端 **content 精确等于**(非子串) + `createdAt >= entry.created_at`。宁重发不漏发。
- 只做 text 主链路(`entry.referral_card_id` 与 `entry.media_asset_id` 均为 None 的纯文本条目)。媒体/名片分支本轮不动。
- 无人工接管红线：新增行不得含 `人工/接管/takeover/hand-off` 等词(本功能不涉及，注意注释用词)。
- MCP 调用统一走 `logged_call_for_account`(mcp.rs:329)，不绕过。DateTime→ISO 用 `try_to_rfc3339_string()`(先例 models.rs:3436)。

---

### Task 1: mcp::chat_search_outbound 封装 + chat_search_hit 纯函数

**Files:**
- Modify: `src/mcp.rs`（新增纯函数 `chat_search_hit` + 异步封装 `chat_search_outbound`，放在 `write_roster_snapshot`(:687-714) 之后、`spawn_roster_refresh`(:721) 之前，或紧邻文件内其它 pub 查询函数）
- Test: `src/mcp.rs`（新增 `chat_search_hit_tests` 模块，文件末尾）

**Interfaces:**
- Produces:
  - `pub(crate) fn chat_search_hit(items: &serde_json::Value, content: &str, since_millis: i64) -> bool`
  - `pub async fn chat_search_outbound(state: &AppState, account_id: &str, peer: &str, content: &str, since: DateTime) -> AppResult<bool>`

- [ ] **Step 1: 写失败测试**（src/mcp.rs 文件末尾新增模块）

```rust
#[cfg(test)]
mod chat_search_hit_tests {
    use super::chat_search_hit;
    use serde_json::json;

    // since = 1_700_000_000_000 ms (2023-11-14T...)。item.createdAt 用 ISO-8601。
    const SINCE: i64 = 1_700_000_000_000;
    // SINCE 之后 1 分钟。
    const AFTER: &str = "2023-11-14T22:14:20.000Z";  // = 1_700_000_060_000ms 附近，> SINCE
    // SINCE 之前。
    const BEFORE: &str = "2023-11-14T00:00:00.000Z"; // < SINCE

    #[test]
    fn exact_content_after_since_hits() {
        let items = json!([
            { "content": "你好呀，在吗", "createdAt": AFTER }
        ]);
        assert!(chat_search_hit(&items, "你好呀，在吗", SINCE));
    }

    #[test]
    fn substring_not_exact_does_not_hit() {
        // "你好" 是历史消息 "你好呀" 的子串——精确等于判据下不得命中。
        let items = json!([
            { "content": "你好呀", "createdAt": AFTER }
        ]);
        assert!(!chat_search_hit(&items, "你好", SINCE));
    }

    #[test]
    fn before_since_does_not_hit() {
        // content 精确等于但发生在 entry 创建之前(历史同内容) → 不命中。
        let items = json!([
            { "content": "确认一下", "createdAt": BEFORE }
        ]);
        assert!(!chat_search_hit(&items, "确认一下", SINCE));
    }

    #[test]
    fn empty_items_does_not_hit() {
        assert!(!chat_search_hit(&json!([]), "任意", SINCE));
        assert!(!chat_search_hit(&json!(null), "任意", SINCE));
    }

    #[test]
    fn one_of_many_matches_hits() {
        let items = json!([
            { "content": "别的消息", "createdAt": AFTER },
            { "content": "目标内容", "createdAt": AFTER }
        ]);
        assert!(chat_search_hit(&items, "目标内容", SINCE));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib chat_search_hit`
Expected: 编译失败（`chat_search_hit` 未定义）。

- [ ] **Step 3: 写纯函数 + 异步封装**（src/mcp.rs，write_roster_snapshot 之后）

先亲验：`DateTime` 已在 mcp.rs:1 导入(`use mongodb::bson::{..., DateTime}`)；`try_to_rfc3339_string()` 是 bson DateTime 方法(先例 models.rs:3436)；`logged_call_for_account`(mcp.rs:329) 签名 `(state, account_id, tool_name, arguments)`。

```rust
/// chat_search 命中判据(纯函数便于单测)：items 里存在一条 **content 精确等于**
/// `content`(非子串) 且 `createdAt >= since_millis`。用于 timeout 兜底核对"这条是否
/// 已真的提交给微信"。精确等于防历史相似内容误命中；since 排除本 entry 创建前的历史同内容。
pub(crate) fn chat_search_hit(items: &serde_json::Value, content: &str, since_millis: i64) -> bool {
    let arr = match items.as_array() {
        Some(a) => a,
        None => return false,
    };
    arr.iter().any(|item| {
        let c = item.get("content").and_then(|v| v.as_str());
        if c != Some(content) {
            return false;
        }
        // createdAt 是 ISO-8601 字符串；解析成 bson DateTime 再比 millis。解析失败保守视为不命中。
        match item.get("createdAt").and_then(|v| v.as_str()) {
            Some(ts) => match DateTime::parse_rfc3339_str(ts) {
                Ok(dt) => dt.timestamp_millis() >= since_millis,
                Err(_) => false,
            },
            None => false,
        }
    })
}

/// 查 MCP chat_search 确认某条 outbound 文本是否已提交给微信(server 侧真实已发记录，
/// 同步落库、失败不写)。命中判据见 [`chat_search_hit`]。调用失败向上抛(由调用方回落本地日志)。
pub async fn chat_search_outbound(
    state: &AppState,
    account_id: &str,
    peer: &str,
    content: &str,
    since: DateTime,
) -> AppResult<bool> {
    let since_iso = since
        .try_to_rfc3339_string()
        .unwrap_or_default();
    let resp = logged_call_for_account(
        state,
        account_id,
        "chat_search",
        serde_json::json!({
            "direction": "outbound",
            "peer": peer,
            "content_contains": content,
            "since": since_iso,
            "limit": 20,
        }),
    )
    .await?;
    // 返回体形如 { items:[...], count }。call_tool_with_key 已剥壳到 structuredContent 本体。
    let items = resp.get("items").cloned().unwrap_or(serde_json::Value::Null);
    Ok(chat_search_hit(&items, content, since.timestamp_millis()))
}
```

- [ ] **Step 4: 跑测试确认通过 + lib 全量**

Run: `cargo test --lib chat_search_hit && cargo test --lib`
Expected: 5 用例 PASS；lib 全量 ≥ 350 passed / 0 failed。

- [ ] **Step 5: 提交**

```bash
git add src/mcp.rs
git commit -m "feat(outbox): mcp::chat_search_outbound 封装 + chat_search_hit 精确命中纯函数"
```

---

### Task 2: outbox_dispatcher timeout 兜底优先查 chat_search

**Files:**
- Modify: `src/agent/outbox_dispatcher.rs`（新增常量 + timeout 分支 text 核对改为先查 chat_search 回落本地日志）

**Interfaces:**
- Consumes: Task 1 的 `mcp::chat_search_outbound`；既有 `mcp_already_succeeded`(:543)。

- [ ] **Step 1: 加常量**（src/agent/outbox_dispatcher.rs，MCP_SEND_TIMEOUT_SECONDS(:52) 附近）

```rust
/// timeout 兜底里调 chat_search 核对的独立短超时——核对本身绝不能卡死 dispatcher。
/// 超时/出错即回落本地 mcp_call_logs 核对。
const CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS: u64 = 15;
```

- [ ] **Step 2: 亲验现有 timeout 分支 text 核对**

先 Read `src/agent/outbox_dispatcher.rs:828-901`（`Err(_)` timeout 分支），确认现结构：`:833` 起按 referral/media/text 三分支算 `already`，text 走 `:848 mcp_already_succeeded(...)`；`:857 if let Ok(true) = already` 标 sent，`:899 else` 走 schedule_retry。本任务只改 **text 分支**取 `already` 的方式。

- [ ] **Step 3: 改 text 核对为 chat_search 优先 + 本地回落**

把 timeout 分支里 text 的 `already` 计算（现 `:847-856` 的 `else { mcp_already_succeeded(...).await }` 分支）改为：先查 chat_search（带 15s 短超时），命中即 `Ok(true)`；chat_search 超时/出错则回落 `mcp_already_succeeded`。

具体：定位 timeout 分支里形如
```rust
            } else {
                mcp_already_succeeded(
                    state,
                    &entry.account_id,
                    &entry.contact_wxid,
                    &entry.content,
                    entry.created_at,
                )
                .await
            };
```
改为：
```rust
            } else {
                // 先查 MCP chat_search(server 真实已发记录，同步落库、失败不写)——不受本地
                // timeout 取消 mcp_call_logs 写入的影响。带独立短超时；超时/出错回落本地日志核对。
                match tokio::time::timeout(
                    Duration::from_secs(CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS),
                    crate::mcp::chat_search_outbound(
                        state,
                        &entry.account_id,
                        &entry.contact_wxid,
                        &entry.content,
                        entry.created_at,
                    ),
                )
                .await
                {
                    Ok(Ok(hit)) => Ok(hit),
                    // chat_search 出错 / 超时 → 回落本地 mcp_call_logs 核对(不倒退)。
                    Ok(Err(_)) | Err(_) => {
                        mcp_already_succeeded(
                            state,
                            &entry.account_id,
                            &entry.contact_wxid,
                            &entry.content,
                            entry.created_at,
                        )
                        .await
                    }
                }
            };
```

注：`Duration` 已在文件顶部导入（`:774` 已用 `Duration::from_secs`）。referral/media 分支不动。

- [ ] **Step 4: 更新标 sent 的 last_error 文案**（可选，同分支 :870 附近）

`:870` 现文案 `"send timeout (150s) but MCP already succeeded — confirmed via mcp_call_logs"` 改为 `"send timeout (150s) but MCP already succeeded — confirmed via chat_search/mcp_call_logs"`（诚实反映核对源可能是二者之一）。若嫌细枝末节可跳过——不影响行为。

- [ ] **Step 5: 编译确认**

Run: `cargo test --lib`
Expected: 编译通过；lib 全量 ≥ 350 passed / 0 failed（本任务无新单测——纯 async 接入逻辑靠编译 + 集成层；Task 1 纯函数已覆盖判据）。

- [ ] **Step 6: 提交**

```bash
git add src/agent/outbox_dispatcher.rs
git commit -m "feat(outbox): timeout 兜底优先查 chat_search 核对已发,失败回落本地日志"
```

---

### Task 3: gateway send_outbound_message 成功判定读新信封 ok 字段

**Files:**
- Modify: `src/agent/gateway.rs`（send_outbound_message :2930 起）
- Test: `src/agent/gateway.rs`（新增 `send_receipt_tests` 模块，或并入现有 gateway 测试模块）

**Interfaces:**
- Produces: `pub(crate) fn send_receipt_is_ok(response: &serde_json::Value) -> bool`

**说明：** 当前 `send_outbound_message` 靠 `logged_call_for_account(...).await?` 不抛错即视为成功，并提取 `newMsgId`(`:2946`) 存库。新信封有显式 `ok:true`。本任务加一个纯函数判定"提交成功"，优先读 `ok===true`，回落 `newMsgId` 存在(兼容旧信封)——**仅用于可观测/审计判定，不改变现有 `.await?` 成功语义与落库流程**（保守：不因新判定拦截已 await 成功的发送）。

- [ ] **Step 1: 写失败测试**（src/agent/gateway.rs，新增测试模块）

```rust
#[cfg(test)]
mod send_receipt_tests {
    use super::send_receipt_is_ok;
    use serde_json::json;

    #[test]
    fn ok_true_is_success() {
        assert!(send_receipt_is_ok(&json!({ "ok": true, "newMsgId": "123" })));
    }

    #[test]
    fn ok_false_is_not_success() {
        assert!(!send_receipt_is_ok(&json!({ "ok": false })));
    }

    #[test]
    fn legacy_envelope_without_ok_but_with_newmsgid_is_success() {
        // 旧信封无 ok 字段，但有非空 newMsgId → 兼容判成功。
        assert!(send_receipt_is_ok(&json!({ "newMsgId": "8974400044288526000" })));
    }

    #[test]
    fn neither_ok_nor_newmsgid_is_not_success() {
        assert!(!send_receipt_is_ok(&json!({ "target": {} })));
        assert!(!send_receipt_is_ok(&json!(null)));
    }

    #[test]
    fn empty_newmsgid_is_not_success() {
        assert!(!send_receipt_is_ok(&json!({ "newMsgId": "" })));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib send_receipt`
Expected: 编译失败（`send_receipt_is_ok` 未定义）。

- [ ] **Step 3: 写纯函数**（src/agent/gateway.rs，send_outbound_message 函数之前或附近）

```rust
/// 判定单条 send 返回信封是否表示"提交成功"。优先读 MCP 统一成功信封的显式
/// `ok===true`；无 ok 字段(旧信封)则回落"存在非空 newMsgId"。用于可观测判定。
pub(crate) fn send_receipt_is_ok(response: &serde_json::Value) -> bool {
    if let Some(ok) = response.get("ok").and_then(|v| v.as_bool()) {
        return ok;
    }
    response
        .get("newMsgId")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}
```

- [ ] **Step 4: 在 send_outbound_message 里用它记一条可观测(不改成功语义)**

先 Read `src/agent/gateway.rs:2946-2990` 确认 `message_id` 变量(`:2946`)后续用途与函数返回。在提取 `message_id` 后加一条：若 `!send_receipt_is_ok(&response)` 则 `tracing::warn!`（send 已 await 成功但信封未含成功标志——异常信封预警，便于观测新旧信封切换期的异常）。**不 return Err、不改落库**——保守，避免误拦已成功发送。

```rust
    // 新信封成功标志观测(不改变 .await? 的成功语义)：await 成功但信封无 ok/newMsgId
    // 属异常(如空返回体)，warn 供排查，不拦截。
    if !send_receipt_is_ok(&response) {
        tracing::warn!(
            account_id = %contact.account_id,
            "send_outbound_message: MCP 返回无成功标志(ok/newMsgId 缺失),疑似异常信封"
        );
    }
```

- [ ] **Step 5: 跑测试确认通过 + lib 全量**

Run: `cargo test --lib send_receipt && cargo test --lib`
Expected: 5 用例 PASS；lib 全量 ≥ 350 passed / 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/agent/gateway.rs
git commit -m "feat(outbox): send_outbound_message 加信封成功标志观测(ok 优先,newMsgId 兼容)"
```

---

## Self-Review

**Spec coverage:**
- timeout 兜底核对源升级 chat_search + 本地回落 → Task 2 ✓
- 核对精度(content 精确等于 + since 时间窗) → Task 1 `chat_search_hit` + 单测钉死 ✓
- chat_search 单独短超时 → Task 2 CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS ✓
- 只做 text 主链路(media/referral 不动) → Task 2 只改 else(text) 分支 ✓
- gateway 成功判定读 ok(兼容旧信封) → Task 3 `send_receipt_is_ok` ✓
- 不追踪送达 → 全程只判"提交"，无 delivered/read ✓

**Placeholder scan:** 无 TBD/TODO；每步含实际代码或确切命令。Task 2 Step 4、Task 3 Step 4 要求实现者先 Read 亲验现场再改（因是嵌入既有大函数的局部修改，需对齐真实上下文）——这不是占位符，是红线要求的亲验动作。

**Type consistency:**
- `chat_search_hit(&Value, &str, i64) -> bool` Task 1 定义、`chat_search_outbound` 内部调用一致。
- `chat_search_outbound(&AppState, &str, &str, &str, DateTime) -> AppResult<bool>` Task 1 定义、Task 2 调用参数一致(account_id/contact_wxid/content/created_at)。
- `send_receipt_is_ok(&Value) -> bool` Task 3 定义、Step 4 调用一致。
- `entry.created_at` 是 `DateTime`(OutboxEntry 字段，mcp_already_succeeded:548 已用同参)——chat_search_outbound 的 `since: DateTime` 匹配。

**实现者留意：**
- Task 1 `chat_search_outbound` 依赖 chat_search 返回体已被 `call_tool_with_key` 剥壳到 structuredContent 本体(顶层直接 items/count)——与 roster 的 parse 同源假设(mcp.rs:508 注释)。若实测发现未剥壳，加 `/structuredContent/items` 防御(参照 parse_roster_items 的多候选)。
- Task 2 是嵌入既有 timeout 分支的局部替换，务必先 Read :828-901 对齐真实缩进/变量名(`already` binding、`entry`/`state` 在作用域内)。
- Task 3 保守原则：新判定只加 warn 观测，不改 `.await?` 成功语义——避免"信封判定"误拦已 await 成功的发送造成漏发。
