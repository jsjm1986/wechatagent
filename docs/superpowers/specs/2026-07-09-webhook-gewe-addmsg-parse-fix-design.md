# webhook 解析真实 GeWe AddMsg 嵌套 payload 修复设计

**日期**：2026-07-09
**分支**：`fix/webhook-gewe-addmsg-parse`（基于 origin/main `1a80887`）
**状态**：设计已获批，待写实现计划

## 背景与症状（2026-07-09 线上 117 亲验）

给账号 102（Demi）打通消息事件推送 webhook 后，真实微信好友（吴界，`wxid_ydzaomn4scsb12`）发来的消息**送达了 wechatagent，但归错了发件人、取错了内容，导致不触发 AI 回复**。

亲验落库结果（`conversation_messages`）：
- `contact_wxid: "wxid_3yeirsb75afd22"`（**账号自己 Demi 的 wxid**，错，应是吴界的 `wxid_ydzaomn4scsb12`）
- `content: "吴界 : 你好"`（**带发件人名前缀的通知串**，错，应是干净的 `"你好"`）

因为归到账号自己的 wxid（`agent_status=normal`），gateway 不触发 Agent → 真实客户消息永远得不到自动回复。**整条真实入站流因此不工作。**

## 根因（已用真实 payload + 源码亲验，非猜测）

真实 GeWe `AddMsg` 回调经 gewe-agent 转发后的 payload 结构（顶层大写驼峰 + `Data` 嵌套 + `{string}`/`{low}` 包裹）：

```json
{
  "Wxid": "wxid_3yeirsb75afd22",          // 账号自己(接收方), 顶层
  "TypeName": "AddMsg",
  "Appid": "wx_WSHYpbq5Fdp_yGcOEl9Pn",
  "Data": {
    "FromUserName": { "string": "wxid_ydzaomn4scsb12" },  // 真实发件人, 嵌套+{string}包裹
    "ToUserName":   { "string": "wxid_3yeirsb75afd22" },
    "Content":      { "string": "你好" },                  // 真实内容, 嵌套+{string}包裹
    "MsgType":      { "low": 1 },                          // 类型码, 嵌套+{low}数字包裹
    "PushContent":  "吴界 : 你好",                          // 通知串(带名字前缀)
    "NewMsgId":     { "high": 1976706754, "low": 1032436816 }
  },
  "_mcp": { "event": "wechat.message.created", "sourceMsgId": "8489890863244754000", ... }
}
```

现有解析器 `find_string`（`src/webhooks.rs:831`）的遍历规则是**先查当前层所有候选 key（按列表顺序），再递归子对象**；`value_to_string`（`src/webhooks.rs:851`）**只认 `String`/`Number`，不解包 `{string:...}`/`{low:...}`**。两处致命碰撞：

1. **发件人归错**（`src/webhooks.rs:400` from_wxid 提取）：候选 key 列表含 `Wxid`，它在**顶层**就命中账号自己的 wxid，在递归到 `Data.FromUserName` 之前就 return 了。真实发件人丢失。
2. **内容取错**（`src/webhooks.rs:417` content 提取）：`PushContent` 是纯字符串、在 `Data` 层且键在候选列表里；而干净的 `Data.Content` 是 `{string:...}` 对象，`value_to_string` 解不出。递归遍历到 `Data` 时 `PushContent` 先命中 → 取到 `"吴界 : 你好"`。
3. **类型码取不到**（`src/webhooks.rs:870` `parse_inbound_msg_type`）：`Data.MsgType` 是 `{low:1}` 对象，`value_to_string` 解不出 → 回落默认 `"text"`。文本消息**恰好蒙对**，但真实图片/语音/名片等**非文本入站会被误判成 text**。

其余字段现状可接受，不改：`message_id`（GeWe `NewMsgId/MsgId` 也是包裹数字取不到，但 `_mcp.sourceMsgId` 兜底生效，`src/webhooks.rs:468`，dedup 正常）；`nickname`（发件人昵称本就不在 AddMsg payload 里）。

## 为什么全量 biz-test（62 PASS/0 FAIL）没抓到

- biz-test 的 `send_webhook`（`scripts/biz-test/_lib.py`）发的是**扁平** payload `{appId, fromWxid, content, msgId}`。
- 现有 `src/webhooks.rs` 单元测试（`webhooks.rs:1155+`）也全是扁平 `{fromWxid, content}` 形态。
- 两者都**从未使用真实 GeWe 的嵌套 `{string}`/`{low}` 包裹形态** → 全绿但真实消息挂。这是测试覆盖的**形态盲区**，不是"假绿"手法。本设计的测试节专门补这个盲区。

## 范围与非目标

**范围**：修 `src/webhooks.rs` 入站解析，对 GeWe `AddMsg` 形态显式提取三个字段（发件人 / 内容 / 类型码），优先于现有通用 `find_string`，取不到才回落（保扁平自测 / biz-test 向后兼容）。

**非目标（本设计不做）**：
- **群消息**：群聊里 `FromUserName` 是 chatroom id、真实发言人 wxid 嵌在 `Content` XML 前缀里，是另一种形态。Phase 1 是私聊运营，YAGNI，不做。
- **`value_to_string` 通用解包 `{string}`**：见方案对比——治标不治本（遮蔽顺序问题仍在），且影响面大，不采纳。
- **message_id / nickname 的 Data.\* 提取**：现状（`_mcp.sourceMsgId` 兜底、昵称不在 payload）够用，不动。
- **媒体引用提取**（`extract_inbound_media_ref`）：非文本的媒体内容理解是 F2 专题，本设计只做类型**识别归类**，不碰媒体下载。

## 设计

### 方案对比

- **方案 A（采纳）**：对 GeWe `AddMsg` 形态**显式提取 `Data.*`**，优先于 `find_string`。加两个小 helper，三个提取点各自「先试 Data.\* 路径，取不到再回落现有 `find_string`」。根治遮蔽（不再被顶层 `Wxid`/`PushContent` 抢先命中）；扁平 payload 走回落，行为一字不变。
- **方案 B（否决）**：让 `value_to_string` 通用解包 `{string:...}`/`{low:...}`。**治标不治本**：即便解包，顶层 `Wxid`（账号自己）和 `Data.PushContent`（带名字前缀）仍会因 `find_string`「先当前层、再递归」的顺序**抢先命中**，发件人仍归错、content 仍取通知串。且改 `value_to_string` 影响所有字段提取，风险面更大。

### 组件（均在 `src/webhooks.rs`）

**1. 两个纯函数 helper**（放在 `find_string` / `value_to_string` 附近）：

```rust
/// 从 GeWe AddMsg 的 `Data.<field>.string` 取字符串（真实推送里发件人/内容都是
/// `{string:...}` 包裹，且嵌在 `Data` 下——通用 find_string 会被顶层同名/近义键遮蔽，
/// 故对 GeWe 形态显式走此路径，优先于 find_string）。取不到返回 None（交调用方回落）。
fn gewe_data_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get(field))
        .and_then(|f| f.get("string"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// 从 GeWe AddMsg 的 `Data.MsgType.low` 取微信消息类型数字码（`{low:N}` 包裹）。
/// 返回数字的字符串形式（交 classify_inbound_msg_type 归一）。取不到返回 None。
fn gewe_data_msg_type_code(payload: &Value) -> Option<String> {
    payload
        .get("Data")
        .and_then(|d| d.get("MsgType"))
        .and_then(|m| m.get("low"))
        .and_then(|n| n.as_i64())
        .map(|n| n.to_string())
}
```

**2. from_wxid 提取（`src/webhooks.rs:400`）** —— GeWe 优先，回落 find_string：

```rust
let from_wxid = gewe_data_string(&payload, "FromUserName")
    .or_else(|| find_string(&payload, &[/* 现有候选列表原样保留 */]))
    .ok_or_else(|| AppError::BadRequest("webhook missing sender wxid".to_string()))?;
```

**3. content 提取（`src/webhooks.rs:417`）** —— GeWe 优先，回落 find_string：

```rust
let content = gewe_data_string(&payload, "Content")
    .or_else(|| find_string(&payload, &[/* 现有候选列表原样保留 */]))
    .unwrap_or_default();
```

**关键取舍**：`gewe_data_string(&payload,"Content")` 命中 `Data.Content` 时**即使 string 为空也算命中**吗？——不。helper 里 `as_str()` 对空串返回 `Some("")`，会短路掉回落。但空内容语义上应交给下游（现状 content 空有处理，`webhooks.rs:1213` 有空 content 测试）。真实 AddMsg 文本消息 `Data.Content.string` 必非空；对于本设计关心的文本入站，命中即用干净内容是对的。空串命中直接用（返回空 content，下游按现状处理），**不回落到 PushContent 通知串**——这正是我们要的（宁可空也不要带名字前缀的脏串）。

**4. msg_type 提取（`parse_inbound_msg_type`，`src/webhooks.rs:870`）** —— GeWe 数字码优先，回落 find_string：

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

### 不动的部分

- **控制事件段**（testMsg / TypeName=Online/Offline，`src/webhooks.rs:323-360`）：这些字段真在顶层，`find_string` 命中正确，不改。
- **`_mcp.sourceMsgId` 去重兜底**（`src/webhooks.rs:468`）：现状正确，不改。
- **principal（领导回复）分流**（`src/webhooks.rs:436`）：依赖 from_wxid——修好 from_wxid 后它拿到的是**真实发件人**，分流判断反而更准，无需改逻辑。
- **`value_to_string` / `find_string`**：签名与实现一字不动（新 helper 独立，不碰通用递归）。

## 数据流（修复后）

```
真实微信消息
  → GeWe 回调 → gewe-agent 按 appId 匹配 slot 102 → 转发到 /webhooks/wechat
  → wechatagent 解析：
      from_wxid = Data.FromUserName.string = "wxid_ydzaomn4scsb12"(吴界, 对)
      content   = Data.Content.string      = "你好"(干净, 对)
      msg_type  = classify(Data.MsgType.low=1) = "text"(对)
  → 落 conversation_messages(contact_wxid=吴界)
  → 吴界 若 managed → gateway 决策→审查→MCP message_send_text 真回复
```

## 错误处理 / 边界

- helper 全程 `Option` 链，任一层缺失返回 `None` → 回落 `find_string` → 再不行才走原有 `BadRequest`（from_wxid）或 `unwrap_or_default`（content）。**不新增 panic 路径。**
- 扁平 payload（自测 / biz-test / 手工 runbook）无 `Data` → helper 全返 `None` → 完全走原逻辑 → 现有行为不变。

## 测试

新增单元测试（`src/webhooks.rs` 测试模块），**用真实嵌套形态**，补形态盲区：

1. `gewe_addmsg_extracts_real_sender_not_account_self`：喂本文档"根因"节的真实 payload 结构，断言 from_wxid == `"wxid_ydzaomn4scsb12"`（吴界），**不是** `"wxid_3yeirsb75afd22"`（账号自己）。
2. `gewe_addmsg_extracts_clean_content_not_pushcontent`：断言 content == `"你好"`，**不是** `"吴界 : 你好"`。
3. `gewe_addmsg_extracts_msg_type_from_data_low`：`Data.MsgType.low=3` → `parse_inbound_msg_type` == `"image"`。
4. `flat_payload_still_parses_via_fallback`（回归守卫）：喂扁平 `{fromWxid, content}`，断言仍走 find_string 回落、行为不变。
5. 现有 20+ 单测保持全绿（回归）。

**基线门不回退**：`cargo test --lib` ≥ 350 passed / 0 failed；4 个 PBT 文件累计 ≥ 33 / 0。只增测试。
**no-human-takeover lint**：新增行无禁词（本改动纯解析，无相关文案）。

## 影响面

- **只改 `src/webhooks.rs`**：新增 2 个 helper + 3 个提取点改「Data.\* 优先 + 回落」+ 新增单测。
- **向后兼容**：扁平 payload 全走回落，现有 biz-test（扁平）+ 20+ 单测行为不变。
- **不碰**：gewe-agent（MCP server）、前端、其它后端模块。

## 部署与联调

修复合并部署到 117 后，真实链路应端到端跑通：吴界（managed）发消息 → 归到吴界 wxid + 干净内容 → 触发 Agent 决策审查 → MCP 真回复投递回吴界。

**联调前提提醒**：117 现处于 `WEBHOOK_VERIFY_SIGNATURE=false` 临时联调态（见 [[project_webhook_sig_disabled_liaodiao]]），gewe-agent 转发不带签名。本修复与签名开关正交——修复解决"归错发件人/取错内容"，签名开关解决"转发能否进站"。两者都到位后真实闭环才通。签名的生产加固是另一独立工作。
