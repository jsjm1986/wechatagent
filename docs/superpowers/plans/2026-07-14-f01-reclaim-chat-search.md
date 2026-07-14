# F-01 reclaim 分支补权威 chat_search 核对 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `process_entry` 的 reclaim / timeout 两个 post-hoc 防重发核对块抽成一个共用函数 `verify_already_sent`，让 reclaim 崩溃恢复分支的文本核对也先查权威 `chat_search_outbound`（修 F-01 不对称），并顺带翻正 6 条台账状态。

**Architecture:** 单文件 Rust 改动（`src/agent/outbox_dispatcher.rs`）——新增一个 `async fn verify_already_sent(state, entry) -> AppResult<bool>` 内含 referral/media/text 三分派（text 路 = chat_search 带超时 → 回落本地 mcp_already_succeeded），reclaim 块与 timeout 块各把原本的三分支 `if/else` 表达式替换为一行调用。测试改 1 断言 + 加 1 哨兵测。台账翻正纯 docs。

**Tech Stack:** Rust 2021 / Axum / MongoDB / wiremock（集成测试）/ tokio。

## Global Constraints

- **红线中的红线**：改任何一行前必须先 100% 读懂受影响代码，所有 `file:line` 引用当场 Read/Grep 亲验，不靠记忆。
- **反过拟合红线**：只修改「修复本身使其失效」的测试断言（本计划仅 `reclaim_gate_precedes_pacing_gate` 的 `recv.len()==0` 一处），绝不为凑绿删改其它维度断言。
- **行为等价**：timeout 分支的 text 路本就是「chat_search 带超时 → 回落 mcp_already_succeeded」，抽函数后必须逐字保留、行为不变。referral（`Ok(false)`）/ media（`media_already_succeeded`）两路两处本就一致，抽出后不变。
- **不动相邻逻辑**：不碰 `reclaim_expired_leases` 的 F-04 `reclaim_count` 逻辑（#204 已落地，正交）；不碰两处标 sent 后的 `last_error`/event 文案（reclaim 的 `last_error` 保留 `"...MCP already succeeded..."` 子串——既有测试 `outbox_integration.rs` 断言依赖它）。
- **本地只跑** `cargo check` + `cargo test --lib`（磁盘纪律）；`#[ignore]` 集成测试交 GitHub CI 的 integration job（Docker/testcontainers）。
- 无新增依赖、无 config 字段、无 DB 迁移、无 API 变更。
- 不引入 no-human-takeover / no-model-hint lint 关注的字符串。

---

### Task 1: 抽 `verify_already_sent` 共用函数并改造 reclaim/timeout 两调用点 + 注释订正

**Files:**
- Modify: `src/agent/outbox_dispatcher.rs`（新增函数 + 改 reclaim 块 `:711-737` + 改 timeout 块 `:877-919` + 订正注释 `:98`/`:710`）

**Interfaces:**
- Consumes（当前签名，已亲验）：
  - `mcp_already_succeeded(state: &AppState, account_id: &str, contact_wxid: &str, content: &str, entry_created_at: DateTime) -> AppResult<bool>`（`outbox_dispatcher.rs:587`）
  - `crate::mcp::chat_search_outbound(state: &AppState, account_id: &str, peer: &str, content: &str, since: DateTime) -> AppResult<bool>`（`src/mcp.rs:792`）
  - `super::media_send::media_already_succeeded(state, account_id, contact_wxid, asset_id: &str, created_at: DateTime) -> AppResult<bool>`
  - 常量 `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS: u64 = 15`（`outbox_dispatcher.rs:56`）
  - `OutboxEntry` 字段：`referral_card_id: Option<...>`、`media_asset_id: Option<String>`、`account_id`、`contact_wxid`、`content`、`created_at`（`models.rs`）
- Produces：`async fn verify_already_sent(state: &AppState, entry: &OutboxEntry) -> AppResult<bool>`（模块私有 `async fn`，仅 `process_entry` 内部调用）

- [ ] **Step 1: 先亲验当前两块与相关符号**

用 Read/Grep 确认以下均与本计划一致（行号可能因基线而漂，以符号为准）：
- reclaim 块：`outbox_dispatcher.rs:711` `if entry.reclaimed_in_flight {`，其内 `let already = if entry.referral_card_id.is_some() { Ok(false) } else if let Some(asset_id) = entry.media_asset_id.as_deref() { media_already_succeeded(...) } else { mcp_already_succeeded(...) };`（`:712-737`），随后 `if let Ok(true) = already { …标 sent… return Ok(()) }`。
- timeout 块：`match send_result { … Err(_) => { let already = if entry.referral_card_id.is_some() { Ok(false) } else if let Some(asset_id)… { media_already_succeeded(...) } else { match tokio::time::timeout(Duration::from_secs(CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS), chat_search_outbound(...)).await { Ok(Ok(hit)) => Ok(hit), Ok(Err(_)) | Err(_) => mcp_already_succeeded(...) } }; … } }`（`:877-919`）。
- `chat_search_outbound` 与 `mcp_already_succeeded` 的签名同构（上方 Interfaces）。

- [ ] **Step 2: 写失败的 lib 单测占位（可选，纯函数不易 lib 测则跳过）**

`verify_already_sent` 依赖真实 MongoDB + MCP（`AppState`），无法纯 lib 单测。**本函数的行为验证放在 Task 2 的集成测试**（哨兵测）。此处不写 lib 测，直接进 Step 3。（说明写进 commit message，避免 reviewer 误判缺测。）

- [ ] **Step 3: 新增 `verify_already_sent` 函数**

在 `mcp_already_succeeded`（`:587-614`）之后、`check_contact_status_pure`（`:623`）之前插入：

```rust
/// post-hoc 防重发核对：判断这条 outbox entry 的内容是否**其实已经发出去过**
/// （MCP 已送达微信但本地状态未落 sent）。命中（`Ok(true)`）即调用方应标 sent 不重发。
///
/// 供 `process_entry` 的两个窗口复用——崩溃恢复（reclaim）与发送 timeout——
/// 消除历史上两分支 text 路的不对称（F-01）：
/// - `referral_card` 条目：名片无 media_id、tool 不同，text/media 版核对都不适用；
///   reclaim/timeout 是边缘场景且重复推名片危害小（客户最多多收一张名片），
///   故保守取 `Ok(false)`（视为未发过、放行重发）。
/// - `media_asset` 条目：content 为空、tool 为 message_send_*，text 版核对查不到
///   → 改用 media_id 定位该素材的成功发送记录。
/// - 纯文本条目：**先查权威 `chat_search_outbound`**（MCP server 真实已发记录，
///   同步落库、不受本地 timeout 取消 mcp_call_logs 写入的影响），带
///   `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS` 独立短超时；chat_search 出错 / 超时才
///   回落本地 `mcp_already_succeeded`（不因权威通道抖动而倒退成"必重发"）。
async fn verify_already_sent(state: &AppState, entry: &OutboxEntry) -> AppResult<bool> {
    if entry.referral_card_id.is_some() {
        Ok(false)
    } else if let Some(asset_id) = entry.media_asset_id.as_deref() {
        super::media_send::media_already_succeeded(
            state,
            &entry.account_id,
            &entry.contact_wxid,
            asset_id,
            entry.created_at,
        )
        .await
    } else {
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
            // chat_search 出错 / 超时 → 回落本地 mcp_call_logs 核对（不倒退成"必重发"）。
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
    }
}
```

- [ ] **Step 4: 改造 reclaim 块**

把 reclaim 块内的整段 `let already = if entry.referral_card_id.is_some() { … } else { mcp_already_succeeded(…) };`（`:712-737`）替换为：

```rust
        let already = verify_already_sent(state, entry).await;
```

其后的 `if let Ok(true) = already { …标 sent…（含 last_error "reclaimed after crash but MCP already succeeded — confirmed via mcp_call_logs" 与 event "outbox_sent_post_hoc"）… return Ok(()); }` **保持完全不变**。

> 注：`last_error` 文案里的 "confirmed via mcp_call_logs" 在 chat_search 命中时技术上不再全准，但**保留不动**——既有测试 `reclaim_gate_precedes_pacing_gate` 断言 `last_error.contains("MCP already succeeded")` 依赖该子串；且该文案仅用于审计可读性、不影响行为。改文案需另做（超出 F-01 范围）。

- [ ] **Step 5: 改造 timeout 块**

把 timeout 块（`match send_result` 的 `Err(_)` 臂，`:872-919`）内的整段 `let already = if entry.referral_card_id.is_some() { … } else { match tokio::time::timeout(… chat_search_outbound …) { … } };`（`:877-919`）替换为：

```rust
            let already = verify_already_sent(state, entry).await;
```

其后 `if let Ok(true) = already { …标 sent… }` 保持不变。

**关键**：此替换必须行为等价——原 timeout text 路即「chat_search 带超时 → 回落 mcp_already_succeeded」，`verify_already_sent` 的 text 分支逐字实现同一逻辑。

- [ ] **Step 6: 订正两处过期注释**

- `reclaim_expired_leases` doc（`:98`）：`重发前须先跑 \`mcp_already_succeeded\` post-hoc 核对。` → `重发前须先跑 \`verify_already_sent\` post-hoc 核对（文本先查权威 chat_search、失败回落本地 mcp_call_logs）。`
- reclaim 块头注释（`:708-710`）：把「重发前先 post-hoc 核对 mcp_call_logs；命中即标 sent 不重发（与 timeout 分支同一核对函数）。」订正为「重发前先跑 `verify_already_sent` post-hoc 核对（文本先查权威 chat_search、回落本地 mcp_call_logs）；命中即标 sent 不重发。与 timeout 分支复用同一 `verify_already_sent`。」（这句"同一核对函数"抽函数后由错变对，明确指向 `verify_already_sent`。）

- [ ] **Step 7: 编译核验**

Run: `cargo check`
Expected: 0 error（可能有既有 warning，不引入新 warning）。若报 `verify_already_sent` 未用 / import 缺失，检查是否两调用点都改到、`Duration`/`chat_search_outbound` 路径是否已在文件顶部 in scope（timeout 块原本就用了它们，故应已 in scope）。

- [ ] **Step 8: 跑 lib 测确认不回归**

Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed（baseline gate 门槛）。本改动不涉及 lib 单测覆盖的路径，应全绿。

- [ ] **Step 9: Commit**

```bash
git add src/agent/outbox_dispatcher.rs
git commit -m "fix(outbox): reclaim 文本分支抽 verify_already_sent 先查权威 chat_search (F-01)

process_entry 的 reclaim 崩溃恢复分支 text 路原直接查本地 mcp_already_succeeded、
timeout 分支先查权威 chat_search——两分支不对称。worker 在 MCP 已送达微信但本地
mcp_call_logs 未落库的窗口崩溃 → reclaim 后本地查不到 → 重发同句给真实客户。
抽 verify_already_sent 三分支共用函数(text=chat_search 带超时→回落本地)让两处
复用,修 F-01 且从结构根除再漂移。timeout 分支逐字等价、行为不变。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 更新既有集成测试断言 + 新增 reclaim chat_search 哨兵测

**Files:**
- Modify: `tests/outbox_integration.rs`（改 `reclaim_gate_precedes_pacing_gate` 的 `recv.len()==0` 断言 + 加哨兵测 + 加按 tool 名分派的 mock responder）

**Interfaces:**
- Consumes：`process_entry`、`atomic_claim_pending`、`enqueue`、`make_contact`、`enqueue_request`、`count_tool_calls`（`tests/outbox_integration.rs` 既有）；`OutboxStatus`（`wechatagent::...`）
- Produces：新 responder struct + 新测 `reclaim_text_verifies_via_chat_search_before_local`

- [ ] **Step 1: 亲验 chat_search 命中判据**

Read `src/mcp.rs` 的 `chat_search_outbound`（`:792-822`）与 `chat_search_hit`（Grep 定位其定义）。确认命中判据：从返回体取 `items`（顶层或 `/structuredContent/items`），`chat_search_hit(items, content, since_millis)` 按 items 内某条的 content 匹配 + 时间 ≥ since 判 true。据其实现记下 items 的确切 JSON 形状（字段名、时间字段），供 Step 4 构造 mock envelope，避免字面漂移导致命不中而假绿。

- [ ] **Step 2: 更新 `reclaim_gate_precedes_pacing_gate` 的收尾断言**

定位该测（`:1217`）末尾（`:1359-1368`）：

```rust
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    assert_eq!(
        recv.len(),
        0,
        "2B post-hoc 确认已发过，不应再向 MCP 发送；recv={}",
        recv.len()
    );
```

改为（用既有 `count_tool_calls` + 过滤 message_send_text）：

```rust
    let recv = mcp_server
        .received_requests()
        .await
        .expect("wiremock recorded requests");
    // F-01 修复后 reclaim text 路会先发一次 chat_search 的 tools/call（本 mock 返回无
    // items → 判未命中 → 回落本地 mcp_already_succeeded，seed 命中 → 仍标 sent）。故不再
    // 断言"零请求"，而是断言"零 message_send_text 真实重发"——这正是 2B 门要守护的不变量。
    let send_calls = recv
        .iter()
        .filter(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body)
                .ok()
                .and_then(|v| {
                    v.pointer("/params/name")
                        .and_then(|n| n.as_str())
                        .map(|s| s == "message_send_text")
                })
                .unwrap_or(false)
        })
        .count();
    assert_eq!(
        send_calls, 0,
        "2B post-hoc 确认已发过，不应再真实重发 message_send_text；send_calls={send_calls}"
    );
```

> 亲验 `params.name` 是否为 MCP `tools/call` 里 tool 名的正确 JSON 路径（对照 `count_tool_calls` 的 `method==tools/call` 与 `src/mcp.rs` 请求体构造；tool 名通常在 `params.name`）。若路径不同，据真实请求体订正 pointer。

- [ ] **Step 3: 跑既有测确认改后仍绿（Docker 环境）**

Run: `cargo test --test outbox_integration reclaim_gate_precedes_pacing_gate -- --ignored`
Expected: PASS（若本地无 Docker 则跳过，交 CI）。断言：status=Sent、last_error 含 marker、send_calls==0。

- [ ] **Step 4: 加按 tool 名分派的 mock responder**

在 `UniqueMsgIdResponder`（`:90-111`）附近新增一个 responder：`params.name == "chat_search"` 返回含匹配 items 的 envelope（据 Step 1 的形状，使 `chat_search_hit` 判 true）；其它（含 `message_send_text`）返回唯一 newMsgId 成功 envelope。示例骨架（items 形状以 Step 1 亲验为准，占位字段名需替换成真实字段）：

```rust
/// 按 tool 名分派：chat_search 返回命中 items（供 reclaim 权威核对命中），
/// 其它 tool 返回唯一 newMsgId 成功 envelope（供计数）。
struct ChatSearchHitResponder {
    counter: std::sync::atomic::AtomicU64,
    hit_content: String,
}

impl wiremock::Respond for ChatSearchHitResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let tool = serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|v| v.pointer("/params/name").and_then(|n| n.as_str()).map(String::from))
            .unwrap_or_default();
        if tool == "chat_search" {
            // items 形状照 src/mcp.rs::chat_search_hit 亲验后填写：
            // 一条 outbound、content 含 hit_content、时间 ≥ since。
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "structuredContent": {
                    "items": [ /* { "content": self.hit_content, "<time_field>": <now_ms/iso> , ... } */ ],
                    "count": 1
                }}
            });
            return ResponseTemplate::new(200).set_body_json(body);
        }
        let seq = self.counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "structuredContent": { "newMsgId": format!("mock_msg_id_{seq}"), "content": [] }}
        });
        ResponseTemplate::new(200).set_body_json(body)
    }
}
```

- [ ] **Step 5: 写哨兵测 `reclaim_text_verifies_via_chat_search_before_local`**

参照 `reclaim_gate_precedes_pacing_gate` 的骨架（enqueue → 置 `reclaimed_in_flight=true` → claim → process_entry），关键差异：
- mount `ChatSearchHitResponder`（`hit_content` = enqueue 的 content）而非 `UniqueMsgIdResponder`。
- **故意不 seed 本地 mcp_call_logs**（本地查不到）。
- 不开 pacing（默认关即可，本测不验 pacing 序）。

```rust
/// F-01 守门：reclaim text 路必须先查权威 chat_search——本地 mcp_call_logs 查不到时，
/// 只要 chat_search 命中就标 sent 不重发。回退（reclaim 直接查本地）→ 本地查不到 →
/// 真实重发 message_send_text → send_calls≥1 → 本测变红。
#[tokio::test]
#[ignore]
async fn reclaim_text_verifies_via_chat_search_before_local() {
    let app = common::TestApp::start().await;
    let mcp_server = MockServer::start().await;
    let contact = make_contact("user_reclaim_chat_search");
    // enqueue 后取 claimed.content 作为 chat_search 命中内容（避免字面漂移）。
    // ...（insert contact；enqueue；置 reclaimed_in_flight=true；atomic_claim_pending 拿 claimed）
    Mock::given(method("POST")).and(path("/mcp"))
        .respond_with(ChatSearchHitResponder {
            counter: std::sync::atomic::AtomicU64::new(0),
            hit_content: claimed.content.clone(),
        })
        .mount(&mcp_server).await;
    let state = common::rebuild_app_state_with_mcp_url(&app, mcp_server.uri());
    // 不 seed mcp_call_logs。
    process_entry(&state, &claimed).await.expect("process entry ok");

    let entry = collection.find_one(doc!{"_id": outbox_id}, None).await.unwrap().unwrap();
    assert_eq!(entry.status, OutboxStatus::Sent.as_str(),
        "chat_search 命中即应标 sent，status={:?}", entry.status);
    let recv = mcp_server.received_requests().await.unwrap();
    let send_calls = recv.iter().filter(|r| /* params.name == "message_send_text" */ ).count();
    assert_eq!(send_calls, 0, "chat_search 命中不应真实重发；send_calls={send_calls}");
    // 反向坐实走了 chat_search：断言收到过 chat_search 的 tools/call（≥1）。
    let search_calls = recv.iter().filter(|r| /* params.name == "chat_search" */ ).count();
    assert!(search_calls >= 1, "reclaim text 路必须先查 chat_search；search_calls={search_calls}");
}
```

> 骨架中 `// ...` 的 enqueue/claim/置位步骤照 `reclaim_gate_precedes_pacing_gate` 的 `(b)(c)(d)` 段逐字复用（那段已亲验可用）；`collection`/`outbox_id` 变量同源。`send_calls`/`search_calls` 的过滤闭包复用 Step 2 的 `params.name` 判据。

- [ ] **Step 6: 编译测试目标**

Run: `cargo test --test outbox_integration --no-run`
Expected: 编译通过（0 error）。修 responder 字段/items 形状直到编译过。

- [ ] **Step 7: 跑哨兵测 + 既有测（Docker 环境；本地无 Docker 交 CI）**

Run: `cargo test --test outbox_integration -- --ignored reclaim_`
Expected: `reclaim_gate_precedes_pacing_gate` 与 `reclaim_text_verifies_via_chat_search_before_local` 均 PASS。若本地无 Docker，标注"交 CI 验证"，不假绿。

- [ ] **Step 8: Commit**

```bash
git add tests/outbox_integration.rs
git commit -m "test(outbox): F-01 哨兵测——reclaim text 路先查权威 chat_search + 既有断言随修复更新

reclaim_gate_precedes_pacing_gate 的 recv.len()==0 改为断言零 message_send_text
真实重发（修复后 reclaim 会先发一次 chat_search 的 tools/call）。新增哨兵测
reclaim_text_verifies_via_chat_search_before_local：chat_search 命中 + 本地无
mcp_call_logs → 标 sent 不重发；回退直接查本地即变红。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 翻正台账 6 条状态（纯 docs）

**Files:**
- Modify: `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`

**Interfaces:** 无代码接口，纯文档状态行改写。

- [ ] **Step 1: F-01 本条翻正**

定位 F-01 的 `- 状态: Open`（`:274` 附近，以 `### [F-01]` 段内为准），改为：

```
- 状态: Fixed（本 PR —— reclaim 文本分支抽 verify_already_sent 共用函数，先 chat_search 权威核对再回落本地 mcp_already_succeeded，消除与 timeout 分支不对称；`outbox_dispatcher.rs` verify_already_sent + reclaim/timeout 两调用点复用）
```

- [ ] **Step 2: 5 条真·代码已修翻 Fixed**

逐条定位各 `### [X]` 段内的 `- 状态: Open`，改为 Fixed 并附亲验判据 + 验证 PR（判据已在设计文档核实）：

- **B-02**（`:164` 附近）→ `- 状态: Fixed（#180 —— webhooks.rs:189-200 步骤(d) record_user_reaction 的 if let Err 块内无 return/continue，:209 步骤(e) 网关同层无条件执行，两步已解耦；reaction 失败不再吞本轮回复）`
- **C-01**（`:200` 附近）→ `- 状态: Fixed（#180 —— gateway.rs:4481 operation_state_transition_rejected + :4500 operation_state_transitioned 两审计写已改 let _ = …await fail-soft）`
- **D-01**（`:225` 附近）→ `- 状态: Fixed（#194 —— types.rs:461-464 customer_stage/intent_level 加 #[serde(alias=…)] 双形容错；:2054 回归测试点名 D-01）`
- **F-02**（`:281` 附近）→ `- 状态: Fixed（#193 —— outbox_dispatcher.rs:341 抽 effective_max_attempts，与 enqueue 侧 outbox.rs:244 <=0→3 口径对齐）`
- **H-01**（`:339` 附近）→ `- 状态: Fixed（#180 —— gateway.rs:4382/4444/4532 三审计写(g1_correction 守卫/profile_churn_observed/follow_up_run_at_degraded)已改 let _ = …await fail-soft，与 C-01 同批对齐）`

- [ ] **Step 3: H-02 翻 WontFix（doc 标注）**

定位 H-02 的 `- 状态: Open`（`:351` 附近），改为：

```
- 状态: WontFix（#193 doc 标注推迟 —— run_envelope.rs 三生命周期函数(:373/:555/:724 仅定义)仍零生产调用、main.rs 无 panic hook 安装、gateway 仍走 write_agent_run_log_with_finalize，代码层未接线；但 run_envelope.rs:5-38 模块头 doc 已按修复建议第二选项标注 R0 未接线/推迟。接线留将来专项）
```

- [ ] **Step 4: 复核未翻正的 Open 条目仍应保持 Open**

Grep `- 状态: Open` 确认剩余 Open 仅 A-01 / A-02（产品裁决·桶A）/ E-01（R11 观测裁决）/ F-03（Low audit-only·可选）四条——这些非本次范围，**保持 Open**，不动。

Run: `grep -n "^- 状态: Open" docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`
Expected: 仅剩 A-01 / A-02 / E-01 / F-03 四行。

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md
git commit -m "docs(audit): 台账翻正 F-01 Fixed + B-02/C-01/D-01/F-02/H-01 Fixed + H-02 WontFix

逐条 file:line + git log 亲验代码真实状态后翻正(非盲翻)：
- F-01 本 PR 修复；B-02/C-01/H-01(#180) D-01(#194) F-02(#193) 代码已修→Fixed
- H-02(#193) 代码层未接线但 doc 已标注推迟→WontFix
- A-01/A-02/E-01/F-03 属产品裁决/观测/Low audit-only，保持 Open

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage：**
- 设计「抽 verify_already_sent 三分支」→ Task 1 Step 3-5。✓
- 设计「注释订正」→ Task 1 Step 6。✓
- 设计「既有测断言改 count_tool_calls」→ Task 2 Step 2。✓
- 设计「新哨兵测 + 按 tool 分派 mock」→ Task 2 Step 4-5。✓
- 设计「台账 6 条翻正 + 保持 Open 4 条」→ Task 3。✓

**2. Placeholder scan：** Task 2 的 mock items 形状与过滤闭包标注「以 Step 1 亲验为准」——这是**有意的亲验指令**而非占位符（items 真实字段名必须读 `src/mcp.rs::chat_search_hit` 才能确定，计划不能瞎编字段名，否则违「引用必亲验」）。实施者按 Step 1 亲验后填入。哨兵测骨架的 `// ...` 明确指向「照 reclaim_gate_precedes_pacing_gate 的 (b)(c)(d) 段复用」。

**3. Type consistency：** `verify_already_sent(state: &AppState, entry: &OutboxEntry) -> AppResult<bool>` 全计划一致；两调用点均 `let already = verify_already_sent(state, entry).await;`（返回 `AppResult<bool>`，与原 `let already = …` 的类型一致，故 `if let Ok(true) = already` 不变）。`chat_search_outbound`/`mcp_already_succeeded`/`media_already_succeeded` 签名与 Task 1 Interfaces 一致。

## 备注

- Task 1（代码）可 lib 编译 + lib 测本地验；Task 2（集成测试）本地若无 Docker 交 CI 的 integration job，不假绿；Task 3 纯 docs。
- 三个 commit 同一分支 `fix/f01-reclaim-chat-search`（基于含 #204/#205 的最新 origin/main）。
- 双 lint（no-human-takeover / no-model-hint）：本改动无相关字符串，Task 完成后 push 前跑 `bash scripts/check-no-human-takeover.sh <base> HEAD` 与 `bash scripts/check-no-model-hint.sh <base> HEAD` 预验。
