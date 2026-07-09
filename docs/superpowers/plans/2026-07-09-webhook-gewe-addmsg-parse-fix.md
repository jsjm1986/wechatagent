# webhook 真实 GeWe AddMsg 嵌套 payload 解析修复 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 wechatagent 正确解析真实 GeWe `AddMsg` 嵌套 payload——从 `Data.FromUserName.string` 取真实发件人、`Data.Content.string` 取干净内容、`Data.MsgType.low` 取类型码，而非被顶层 `Wxid`/`PushContent` 遮蔽。

**Architecture:** 在 `src/webhooks.rs` 加两个纯函数 helper（`gewe_data_string` / `gewe_data_msg_type_code`），三个提取点（from_wxid / content / msg_type）改成「GeWe `Data.*` 优先 + 现有 `find_string` 回落」。扁平 payload 无 `Data` → helper 返 None → 完全走原逻辑，向后兼容。

**Tech Stack:** Rust（axum webhook handler），`serde_json::Value`，`cargo test --lib`。

设计出处：`docs/superpowers/specs/2026-07-09-webhook-gewe-addmsg-parse-fix-design.md`。

## Global Constraints

- 只改 `src/webhooks.rs`。不碰 gewe-agent（MCP server）、前端、其它后端模块。
- 私聊 only。**不做群消息**（chatroom 里发言人 wxid 嵌在 Content XML 前缀，是另一形态，Phase 1 之外）。
- **不改** `find_string`（webhooks.rs:831）/ `value_to_string`（webhooks.rs:851）的签名与实现——新 helper 独立，不碰通用递归。
- **不改**控制事件段（testMsg / TypeName=Online/Offline，webhooks.rs:323-360）——这些字段真在顶层，`find_string` 命中正确。
- 基线门不回退：`cargo test --lib` ≥ 350 passed / 0 failed。只增测试，不删改现有 20+ webhook 单测。
- no-human-takeover lint：新增行无禁词（本改动纯解析，无相关文案）。
- 分支 `fix/webhook-gewe-addmsg-parse`（worktree `.claude/worktrees/roster-debug`，基于 origin/main `1a80887`）。
- 本机磁盘紧 + Windows Defender 对默认 `target/` 有 exec 锁：跑 cargo 用 `CARGO_TARGET_DIR=E:/yw/cargo-target-roster` 覆盖；只跑 `cargo test --lib`（含子串过滤），不跑全量 `cargo test`。跑不动时退 `cargo check --lib` 证明编译过，报告注明单测留 CI，**不假绿**。
- 红线：动手前先 Read `src/webhooks.rs` 确认下述「当前代码」块与文件实际一致（行号可能漂移，按内容锚定），引用必亲验。

---

### Task 1: `gewe_data_string` helper + from_wxid / content 提取改「Data.* 优先 + 回落」

**Files:**
- Modify: `src/webhooks.rs`
  - 新增 `fn gewe_data_string`（紧跟在 `value_to_string` 之后，当前 :857）
  - 改 from_wxid 提取（当前 :400）
  - 改 content 提取（当前 :417）
  - 新增单测（`#[cfg(test)] mod tests`，当前 :1155 起）

**Interfaces:**
- Consumes: 既有 `fn find_string(value: &Value, keys: &[&str]) -> Option<String>`（webhooks.rs:831）。
- Produces: `fn gewe_data_string(payload: &Value, field: &str) -> Option<String>` —— 读 `payload["Data"][field]["string"]`；取不到返回 None。Task 2 会在其旁边加同类 helper。

- [ ] **Step 1: 写失败测试（helper + 回归留证）**

`src/webhooks.rs` 的 `#[cfg(test)] mod tests`（当前 :1155 起）内，追加一个构造真实 payload 的辅助 + 三个测试。测试模块顶部已有 `use super::*;` 与 `use serde_json::json;`（既有测试 :1220 已用 `json!` 与私有 `parse_inbound_msg_type`，可直接复用）。

```rust
    fn real_gewe_addmsg() -> serde_json::Value {
        // 2026-07-09 线上 117 亲验的真实 GeWe AddMsg 形态(经 gewe-agent 转发):
        // 顶层大写驼峰 + Data 嵌套 + {string}/{low} 包裹 + _mcp envelope。
        json!({
            "Wxid": "wxid_3yeirsb75afd22",
            "TypeName": "AddMsg",
            "Appid": "wx_WSHYpbq5Fdp_yGcOEl9Pn",
            "Data": {
                "FromUserName": { "string": "wxid_ydzaomn4scsb12" },
                "ToUserName": { "string": "wxid_3yeirsb75afd22" },
                "Content": { "string": "你好" },
                "MsgType": { "low": 1 },
                "PushContent": "吴界 : 你好",
                "NewMsgId": { "high": 1976706754, "low": 1032436816 }
            },
            "_mcp": { "event": "wechat.message.created", "sourceMsgId": "8489890863244754000" }
        })
    }

    #[test]
    fn gewe_addmsg_extracts_real_sender_not_account_self() {
        let payload = real_gewe_addmsg();
        // 修复:显式走 Data.FromUserName.string 拿真实发件人(吴界)。
        assert_eq!(
            gewe_data_string(&payload, "FromUserName").as_deref(),
            Some("wxid_ydzaomn4scsb12")
        );
        // 回归留证:通用 find_string 会被顶层 Wxid 遮蔽 → 归错成账号自己。
        // 这正是本次修复的 bug,保留断言防止有人把提取改回纯 find_string。
        assert_eq!(
            find_string(&payload, &["fromWxid", "FromUserName", "FromWxid", "Wxid"]).as_deref(),
            Some("wxid_3yeirsb75afd22")
        );
    }

    #[test]
    fn gewe_addmsg_extracts_clean_content_not_pushcontent() {
        let payload = real_gewe_addmsg();
        // 修复:Data.Content.string 拿干净正文。
        assert_eq!(gewe_data_string(&payload, "Content").as_deref(), Some("你好"));
        // 回归留证:find_string 会先命中 Data.PushContent 通知串(带发件人名前缀)。
        assert_eq!(
            find_string(&payload, &["content", "Content", "PushContent"]).as_deref(),
            Some("吴界 : 你好")
        );
    }

    #[test]
    fn flat_payload_still_parses_via_fallback() {
        // 扁平自测/biz-test payload 无 Data → helper 返 None → 走 find_string 回落,行为不变。
        let payload = json!({ "fromWxid": "wx_flat", "content": "hello flat" });
        assert_eq!(gewe_data_string(&payload, "FromUserName"), None);
        assert_eq!(gewe_data_string(&payload, "Content"), None);
        assert_eq!(find_string(&payload, &["fromWxid"]).as_deref(), Some("wx_flat"));
        assert_eq!(find_string(&payload, &["content"]).as_deref(), Some("hello flat"));
    }
```

- [ ] **Step 2: 运行测试确认失败（编译错）**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks::tests::gewe_addmsg 2>&1 | tail -20`
Expected: 编译失败 —— `cannot find function `gewe_data_string` in this scope`（helper 未定义）。

- [ ] **Step 3: 实现 `gewe_data_string` helper**

`src/webhooks.rs`，紧跟在 `value_to_string`（当前 :851-857）之后新增：

```rust
/// 从 GeWe AddMsg 的 `Data.<field>.string` 取字符串。真实推送里发件人/内容都是
/// `{string:...}` 包裹且嵌在 `Data` 下——通用 find_string 会被顶层同名/近义键
/// (`Wxid` / `PushContent`)遮蔽,故对 GeWe 形态显式走此路径,优先于 find_string。
/// 取不到返回 None(交调用方回落 find_string)。命中空串返回 Some("")——刻意直接
/// 用空内容,不回落到带发件人名前缀的 PushContent 通知串。
fn gewe_data_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get(field))
        .and_then(|f| f.get("string"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}
```

- [ ] **Step 4: 改 from_wxid 提取(GeWe 优先 + 回落)**

`src/webhooks.rs` from_wxid 提取（当前 :400）当前：

```rust
    let from_wxid = find_string(
        &payload,
        &[
            // 小写驼峰（手工 / 自测 / 部分推送）
            "fromWxid",
            "from_wxid",
            "fromUserName",
            "from_user_name",
            "fromusername",
            "from",
            // GeWe 大写驼峰（MCP 透传的真实推送主字段）
            "FromUserName",
            "FromWxid",
            "Wxid",
        ],
    )
    .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
```
改为（`find_string(...)` 整块原样保留，只在外层套一个 `gewe_data_string(...).or_else(|| ...)`）：
```rust
    let from_wxid = gewe_data_string(&payload, "FromUserName")
        .or_else(|| {
            find_string(
                &payload,
                &[
                    // 小写驼峰（手工 / 自测 / 部分推送）
                    "fromWxid",
                    "from_wxid",
                    "fromUserName",
                    "from_user_name",
                    "fromusername",
                    "from",
                    // GeWe 大写驼峰（MCP 透传的真实推送主字段）
                    "FromUserName",
                    "FromWxid",
                    "Wxid",
                ],
            )
        })
        .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
```

- [ ] **Step 5: 改 content 提取(GeWe 优先 + 回落)**

`src/webhooks.rs` content 提取（当前 :417）当前：

```rust
    let content = find_string(
        &payload,
        &[
            // 小写驼峰
            "content",
            "text",
            "msgContent",
            "msg_content",
            "message",
            "messageContent",
            // GeWe 大写驼峰
            "Content",
            "PushContent",
        ],
    )
    .unwrap_or_default();
```
改为：
```rust
    let content = gewe_data_string(&payload, "Content")
        .or_else(|| {
            find_string(
                &payload,
                &[
                    // 小写驼峰
                    "content",
                    "text",
                    "msgContent",
                    "msg_content",
                    "message",
                    "messageContent",
                    // GeWe 大写驼峰
                    "Content",
                    "PushContent",
                ],
            )
        })
        .unwrap_or_default();
```

- [ ] **Step 6: 运行测试确认通过**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks::tests::gewe_addmsg 2>&1 | tail -20` 再跑 `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks::tests::flat_payload 2>&1 | tail -10`
Expected: `gewe_addmsg_extracts_real_sender_not_account_self`、`gewe_addmsg_extracts_clean_content_not_pushcontent`、`flat_payload_still_parses_via_fallback` 全 PASS。
（本机 target 锁/爆盘跑不动则 `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo check --lib` 证编译过，报告注明单测留 CI，不假绿。）

- [ ] **Step 7: 跑 webhook 现有单测不回退**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks:: 2>&1 | tail -6`
Expected: `test result: ok. N passed; 0 failed`（含既有 20+ 与新增 3 个；扁平形态既有测试不受影响）。

- [ ] **Step 8: Commit**

```bash
git add src/webhooks.rs
git commit -m "fix(webhook): GeWe AddMsg 从 Data.FromUserName/Content.string 取真实发件人与内容

真实 GeWe 回调 payload 嵌套(Data.FromUserName.string / Data.Content.string,
{string} 包裹),被通用 find_string 顶层 Wxid/PushContent 遮蔽 → 发件人归错成
账号自己(不触发回复)、内容取到带名字前缀的通知串。加 gewe_data_string 显式提取,
优先于 find_string,扁平 payload 走回落保持向后兼容。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: `gewe_data_msg_type_code` helper + `parse_inbound_msg_type` 取 `Data.MsgType.low`

**Files:**
- Modify: `src/webhooks.rs`
  - 新增 `fn gewe_data_msg_type_code`（紧跟在 Task 1 的 `gewe_data_string` 之后）
  - 改 `parse_inbound_msg_type`（当前 :870）
  - 新增单测（`#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: 既有 `fn classify_inbound_msg_type(raw: &str) -> &'static str`（webhooks.rs，把数字码/字符串别名归一，未知 → `"unknown"`）；既有 `fn find_string`。相邻放置 Task 1 的 `gewe_data_string`。
- Produces: `fn gewe_data_msg_type_code(payload: &Value) -> Option<String>` —— 读 `payload["Data"]["MsgType"]["low"]` 数字码，返回其字符串形式；取不到返回 None。

- [ ] **Step 1: 写失败测试**

`src/webhooks.rs` 的 `#[cfg(test)] mod tests` 内追加：

```rust
    #[test]
    fn gewe_addmsg_extracts_msg_type_from_data_low() {
        // MsgType.low=3 → image。
        let payload = json!({
            "TypeName": "AddMsg",
            "Data": {
                "FromUserName": { "string": "wxid_x" },
                "Content": { "string": "x" },
                "MsgType": { "low": 3 }
            }
        });
        assert_eq!(gewe_data_msg_type_code(&payload).as_deref(), Some("3"));
        assert_eq!(parse_inbound_msg_type(&payload), "image");
        // MsgType.low=1 → text(真实文本入站)。
        let text_payload = json!({ "Data": { "MsgType": { "low": 1 } } });
        assert_eq!(parse_inbound_msg_type(&text_payload), "text");
    }
```

- [ ] **Step 2: 运行测试确认失败(编译错)**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks::tests::gewe_addmsg_extracts_msg_type 2>&1 | tail -20`
Expected: 编译失败 —— `cannot find function `gewe_data_msg_type_code` in this scope`。

- [ ] **Step 3: 实现 `gewe_data_msg_type_code` helper**

`src/webhooks.rs`，紧跟在 Task 1 新增的 `gewe_data_string` 之后：

```rust
/// 从 GeWe AddMsg 的 `Data.MsgType.low` 取微信消息类型数字码(`{low:N}` 包裹)。
/// 返回数字的字符串形式(交 classify_inbound_msg_type 归一)。取不到返回 None。
fn gewe_data_msg_type_code(payload: &Value) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get("MsgType"))
        .and_then(|m| m.get("low"))
        .and_then(|n| n.as_i64())
        .map(|n| n.to_string())
}
```

- [ ] **Step 4: 改 `parse_inbound_msg_type`(GeWe 优先 + 回落)**

`src/webhooks.rs` `parse_inbound_msg_type`（当前 :870）当前：

```rust
fn parse_inbound_msg_type(payload: &Value) -> &'static str {
    let raw_msg_type = find_string(payload, &["msgType", "msg_type", "MsgType"]);
    match raw_msg_type.as_deref() {
        Some(raw_type) => classify_inbound_msg_type(raw_type),
        None => "text",
    }
}
```
改为：
```rust
fn parse_inbound_msg_type(payload: &Value) -> &'static str {
    let raw_msg_type = gewe_data_msg_type_code(payload)
        .or_else(|| find_string(payload, &["msgType", "msg_type", "MsgType"]));
    match raw_msg_type.as_deref() {
        Some(raw_type) => classify_inbound_msg_type(raw_type),
        None => "text",
    }
}
```

- [ ] **Step 5: 运行测试确认通过**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks::tests::gewe_addmsg_extracts_msg_type 2>&1 | tail -10`
Expected: `gewe_addmsg_extracts_msg_type_from_data_low` PASS。
（跑不动则 `cargo check --lib`，报告注明留 CI，不假绿。）

- [ ] **Step 6: 跑 msg_type 现有单测不回退(关键回归)**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib webhooks::tests::parse_inbound_msg_type 2>&1 | tail -10`
Expected: 既有 `parse_inbound_msg_type_*` 全 PASS。**核对**：扁平 payload（`{"MsgType":"3"}` 顶层字符串、`{"msgType":"voice"}`、无类型字段默认 text、嵌套无关 `{type:"event"}` 不误命中）——这些无 `Data.MsgType.low`，`gewe_data_msg_type_code` 返 None → 回落 find_string，行为与改动前一致。

- [ ] **Step 7: 跑 lib 基线不回退**

Run: `CARGO_TARGET_DIR=E:/yw/cargo-target-roster cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350。
（本机跑不动则 `cargo check --lib` + 报告注明基线留 CI，不假绿。）

- [ ] **Step 8: Commit**

```bash
git add src/webhooks.rs
git commit -m "fix(webhook): GeWe AddMsg 从 Data.MsgType.low 取消息类型码

Data.MsgType 是 {low:N} 包裹,通用 find_string 取不到 → 非文本入站(图片/语音/
名片等)被误判成 text。加 gewe_data_msg_type_code 显式提取数字码,优先于 find_string,
扁平/字符串形态走回落保持向后兼容。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 部署（实现全绿后，单独动作）

wechatagent 单端部署到 117（前端零改动，同 PR #156 流程）：
1. PR + CI 双门 → 合并 main。
2. 117：git bundle 绕过（117→GitHub 不可达，见 [[deploy_117_bundle_when_github_unreachable]]）→ ff-only 合并 → `cargo build --release` → `systemctl restart wechatagent.service` → health 200。
   - **重启前核验**：`stat target/release/wechatagent` mtime 新于服务启动时间；若 build 完但 restart 没生效需手动补 restart（长连接尾部易被 SSH 断开掐掉）。
3. 真实闭环验证：让 **吴界（managed，wxid_ydzaomn4scsb12，账号 102）**发一条微信 → 查 `conversation_messages`：`contact_wxid == wxid_ydzaomn4scsb12`（不是账号自己）、`content` 干净（不带"吴界 : "前缀）、`msg_type` 正确 → 触发 Agent 决策审查 → MCP 真回复投递回吴界。

**联调前提**：117 现处 `WEBHOOK_VERIFY_SIGNATURE=false` 临时联调态（见 [[project_webhook_sig_disabled_liaodiao]]），本修复与签名开关正交（修复解决"归错发件人/取错内容"，签名解决"转发能否进站"）。验证完真实闭环后，签名生产加固是另一独立工作。

## 后续（不在本计划）

- 群消息形态（chatroom + 发言人 wxid 嵌 Content XML 前缀）：私聊验证通过后另立 spec。
- 非文本媒体内容理解（`extract_inbound_media_ref` → 媒体下载）：F2 专题，本计划只做类型识别归类。
