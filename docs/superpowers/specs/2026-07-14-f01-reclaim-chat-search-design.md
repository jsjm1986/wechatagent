# F-01 reclaim 崩溃恢复分支补权威 chat_search 核对 —— 设计

> 深度逻辑审查台账（`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`）P3 收官后发现的 P2 遗漏 Medium finding。修 F-01（唯一真代码缺陷）+ 顺带批量翻正 6 条已修/已处理但台账状态滞后的 finding（纯 docs）。

## 背景与问题（全部亲验 file:line）

`src/agent/outbox_dispatcher.rs::process_entry` 有两个"发送后 / 崩溃后"的 post-hoc 防重发核对块，用于在"MCP 可能已把消息送达微信、但本地状态未落 sent"的窗口里避免重发同一句给真实客户。两块的结构本应完全相同——都是按发送物类型三分派：

```
referral_card → Ok(false)         // 名片无 media_id、tool 不同，两版核对都不适用；
                                  // reclaimed/timeout 是边缘场景且重复推名片危害小
                                  // （客户最多多收一张名片），故跳过核对、保守放行重发
media_asset   → media_already_succeeded(state, account, contact, asset_id, created_at)
text          → 【核对文本是否已发过】
```

**不对称就出在 text 路：**

- **timeout 分支**（`outbox_dispatcher.rs:877-919` 亲验）text 路：先查权威 `crate::mcp::chat_search_outbound(...)`（MCP server 侧真实已发记录，同步落库、不受本地 timeout 取消 `mcp_call_logs` 写入的影响），带 `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS = 15s`（`:56`）独立短超时；chat_search 出错 / 超时才回落本地 `mcp_already_succeeded`（查本地 `mcp_call_logs`）。
- **reclaim 崩溃恢复分支**（`outbox_dispatcher.rs:711-737` 亲验）text 路：**直接** `mcp_already_succeeded`（只查本地 `mcp_call_logs`），**无 chat_search**。

`referral` 与 `media` 两路在两处逐字节一致，仅 text 路漂移。

### 危害

`mcp_already_succeeded` 查的本地 `mcp_call_logs` 写入是 best-effort（`src/mcp.rs:358` `let _`）且在 MCP 响应**之后**才写。worker 在"MCP 已把消息送达微信、但 `mcp_call_logs` 尚未落库"这一窗口崩溃 → lease 过期被 `reclaim_expired_leases` 放回 pending + 置 `reclaimed_in_flight=true` → 下一个 worker 进 reclaim 分支 → `mcp_already_succeeded` 本地查不到 → 判"没发过" → **重发同一句给真实客户**。

崩溃恰恰是本地日志最不可靠的时刻，而这个分支却唯独不查权威的 chat_search——比 timeout 分支更需要权威核对，却反而没有。

### 根因

PR#164 给 timeout 分支加 `chat_search_outbound` 权威核对时，漏了同步给 reclaim 分支。`reclaim_expired_leases` 的 doc 注释（`:98`）和 reclaim 块注释（`:710`「与 timeout 分支同一核对函数」）声称两分支同源，但实现层并不同源——注释是过期/错误的。

### 严重度：Medium

- 后果 = 重发给真实客户，顶住下限（不是绕红线、不是数据损坏）。
- 触发需「MCP 已送达微信 + `mcp_call_logs` 未落库 + 此刻恰好崩溃 + lease 过期被 reclaim」多条件叠加，且单 worker 崩溃在生产罕见——压住上限。
- 台账主控已裁定 Medium。

## 修复方案：抽整个三分支为共用函数（用户已批准）

把「按发送物类型三分派 → 得到 already-sent 布尔」这整段逻辑抽成一个共用 async 函数，reclaim 与 timeout 两个块各改为一行调用它。这样：

1. **修 F-01**：reclaim 的 text 路自动获得与 timeout 一致的「先 chat_search 权威核对 → 回落本地 mcp_already_succeeded」行为。
2. **根除未来再漂移**：referral / media / text 三路只有一份实现，任何一方改动都不会再造成两分支不对称（DRY）。

### 新函数

```rust
/// post-hoc 防重发核对：判断这条 outbox entry 的内容是否**其实已经发出去过**
/// （MCP 已送达微信但本地状态未落 sent）。命中即调用方应标 sent 不重发。
///
/// 供 `process_entry` 的两个窗口复用，消除历史上 reclaim / timeout 两分支
/// text 路的不对称（F-01）：
/// - `referral_card` 条目：名片无 media_id、tool 不同，两版核对都不适用；
///   reclaim/timeout 是边缘场景且重复推名片危害小（客户最多多收一张名片），
///   故保守取 `false`（视为未发过、放行重发）。
/// - `media_asset` 条目：content 为空、tool 为 message_send_*，text 版核对
///   查不到 → 改用 media_id 定位该素材的成功发送记录。
/// - 纯文本条目：**先查权威 `chat_search_outbound`**（MCP server 真实已发
///   记录，同步落库、不受本地 timeout 取消 mcp_call_logs 写入的影响），带
///   `CHAT_SEARCH_VERIFY_TIMEOUT_SECONDS` 独立短超时；chat_search 出错 /
///   超时才回落本地 `mcp_already_succeeded`（不因权威通道抖动而倒退成"必重发"）。
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

签名说明：`chat_search_outbound(state, account_id, peer, content, since)`（`src/mcp.rs:792` 亲验）与 `mcp_already_succeeded(state, account_id, contact_wxid, content, entry_created_at)`（`:587` 亲验）签名同构，可无缝组合。

### 两个调用点改造

- **reclaim 块**（`:711-737`）：`let already = if … {…} else {…};` 整段（三分支）替换为 `let already = verify_already_sent(state, entry).await;`。命中后标 sent 的 `if let Ok(true) = already { … }` 逻辑（`:738` 起）保持不变。
- **timeout 块**（`:872-919`）：同样把 `let already = if … {…} else {…};` 整段替换为 `let already = verify_already_sent(state, entry).await;`。命中后标 sent 逻辑不变。

**行为等价性说明**：timeout 块的 text 路本就是「chat_search 带超时 → 回落 mcp_already_succeeded」，抽函数后逐字保留，timeout 分支行为**完全不变**。reclaim 块的 text 路从「直接 mcp_already_succeeded」变为「chat_search 带超时 → 回落 mcp_already_succeeded」——这正是 F-01 的修复。referral / media 两路两块本就一致，抽出后不变。

### 连带：过期注释订正

- `reclaim_expired_leases` doc（`:98`）：「重发前须先跑 `mcp_already_succeeded` post-hoc 核对」→ 改为「重发前须先跑 `verify_already_sent` post-hoc 核对（文本先查权威 chat_search、回落本地日志）」。
- reclaim 块注释（`:710`「与 timeout 分支同一核对函数」）：抽函数后这句从"错误"变"正确"，保留并明确指向 `verify_already_sent`。

## 测试

### 反过拟合边界（亲验）

既有集成测试 `reclaim_gate_precedes_pacing_gate`（`tests/outbox_integration.rs:1217`，`#[ignore]` + testcontainers）用 `start_mcp_mock_success`（`UniqueMsgIdResponder`，对**任何** `tools/call` 都返回成功 envelope）。测试末尾断言 `recv.len() == 0`（`:1363-1368`，零 MCP 请求）——其原意是「走 reclaim 2B post-hoc 标 sent，**不真实重发** message_send_text」。

修复后 reclaim text 路会先发一次 `chat_search` 的 `tools/call`（mock 返回无 `items` → `chat_search_hit` 判 false → 回落本地 `mcp_already_succeeded`，seed 命中 → 仍标 sent）。因此：

- 测试**核心不变量仍成立**：reclaim 先于 pacing、标 sent、不真实重发 `message_send_text`。
- 但 `recv.len() == 0` 这条断言被**修复故意失效**（现在会有 chat_search 的握手 + 调用）。这是修复合法改变了行为、需要更新的断言，符合反过拟合红线（只改修复本身失效的断言，不为凑绿而改）。

**改法**：把 `recv.len() == 0` 改为用既有的 `count_tool_calls(&recv)` 过滤，断言**没有 `message_send_text` 的真实重发**（而非"零请求"）。具体判据：解析每个 `tools/call` 请求体的 `params.name`，断言其中 `message_send_text` 计数为 0（chat_search 允许存在）。保留 `entry.status == Sent`、`last_error` 含专属 marker、`worker_id`/`locked_until` 清空等原有断言。

### 新增哨兵测

在 `tests/outbox_integration.rs` 新增一个 `#[ignore]` 集成测试 `reclaim_text_verifies_via_chat_search_before_local`：

- 目的：锁死「reclaim text 路先查 chat_search 权威通道」这条 F-01 修复不变量——回退（reclaim 直接查本地）即变红。
- 构造：reclaim 分支的一条 text entry（`reclaimed_in_flight=true`）；让 chat_search **命中**（返回含匹配 `content` 的 `items`），且**故意不 seed 本地 mcp_call_logs**（本地查不到）。
- 断言：entry 标 sent、无 `message_send_text` 真实重发。若 reclaim 仍直接查本地（未修），本地查不到 → 会真实重发 → `message_send_text` 计数 ≥ 1 → 测试红。这样测试专门守护"reclaim 走了权威 chat_search 而非本地"。

**Mock 前提（亲验，实现关键）**：现有 `UniqueMsgIdResponder`（`tests/outbox_integration.rs:90-111`）对**任何**请求都返回统一的 `{result:{structuredContent:{newMsgId, content:[]}}}` envelope，其中**没有 `items` 字段** → `chat_search_outbound`（`src/mcp.rs:816` 取 `items`）得 Null → `chat_search_hit` 判 false。因此它无法让 chat_search 命中。哨兵测需要一个**按 JSON-RPC `params.name`（tool 名）区分响应**的新 wiremock responder：
- `params.name == "chat_search"` → 返回含匹配 `items` 的 envelope（items 内一条 `content` 含目标文本、时间 ≥ since，使 `chat_search_hit` 判 true）；
- 其它 tool（如 `message_send_text`）→ 沿用 `UniqueMsgIdResponder` 那样的唯一 newMsgId 成功 envelope（供计数）。

`chat_search_hit` 的具体命中判据（`content` 包含匹配 + 时间窗）在 `src/mcp.rs` 亲验后照其实现构造 items 形状，避免字面漂移导致 mock 命不中而假绿。此 responder 是本测试独有的实现细节，写计划时给出完整代码。

> 注：baseline gate 的 `cargo test --lib` 不含集成测试；这些 `#[ignore]` 集成测试由 GitHub CI 的 integration job 跑（Docker/testcontainers）。本地只跑 `cargo check` + `cargo test --lib` 确认不回归。

## 台账状态翻正（纯 docs，同 PR）

逐条用 file:line + git log 亲验了「代码真实状态」后翻正（非盲翻）。`docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：

**F-01 本条**：`状态: Open` → `Fixed（本 PR —— reclaim 文本分支抽 verify_already_sent 共用函数，先 chat_search 权威核对再回落本地 mcp_already_succeeded，消除与 timeout 分支不对称）`。

**5 条真·代码已修 → `Open` → `Fixed`（附验证 PR + file:line）**：

| Finding | 判据（当前代码亲验） | 验证 PR |
| --- | --- | --- |
| B-02 | `webhooks.rs:189-200` 步骤(d) `record_user_reaction` 的 `if let Err` 块内无 return/continue，`:209` 步骤(e) 网关在同层无条件执行——两步已解耦（方案一），reaction 失败不再吞本轮回复 | #180 |
| C-01 | `gateway.rs:4481` `operation_state_transition_rejected` + `:4500` `operation_state_transitioned` 两审计写已改 `let _ = …await`（fail-soft） | #180 |
| H-01 | `gateway.rs:4382`(g1_correction 守卫下审计写) / `:4444` `profile_churn_observed` / `:4532` `follow_up_run_at_degraded` 三处已改 `let _ = …await` | #180 |
| D-01 | `types.rs:461-464` `customer_stage`/`intent_level` 已加 `#[serde(alias = …)]`；`:2054` 回归测试点名 D-01 | #194 |
| F-02 | `outbox_dispatcher.rs:341-344` 抽出 `effective_max_attempts`，与 enqueue 侧（`outbox.rs:244` `<=0→3`）口径对齐 | #193 |

**1 条特殊 → `Open` → `WontFix（doc 标注推迟）`**：

| Finding | 判据 | 处理 PR |
| --- | --- | --- |
| H-02 | `run_envelope.rs` 三生命周期函数（`:373/:555/:724` 仅定义）仍**零生产调用**、main.rs 无 panic hook 安装、gateway 仍走 `write_agent_run_log_with_finalize`——代码层死代码**未接线**；但 `run_envelope.rs:5-38` 模块头 doc 已按修复建议第二选项标注「R0 未接线/推迟」。故非 Fixed，标 WontFix（doc 已标注推迟，接线留将来专项） | #193 |

**保持 Open（非本次范围，逐条亲验确认性质）**：

- A-01 / A-02：产品意图裁决项（同 wxid 跨 account 错配重路由 / 领导-客户双身份语义），台账修复建议明写"待用户拍板"，属桶A。
- E-01：review 软闸 0 值双义，涉 R11 反序列化基线，"留用户裁决勿擅改"，观测项非 bug。
- F-03：send 成功 update 忽略 modified_count 的 cancel 竞态审计不一致，**不致重发**，修复建议"可选"，Low audit-only。

## 影响面 / 非目标

- **只改** `src/agent/outbox_dispatcher.rs`（抽函数 + 两调用点 + 注释）、`tests/outbox_integration.rs`（改 1 断言 + 加 1 哨兵测）、台账 docs。
- **不动** referral / media 两路的语义（本就一致）；**不动** timeout 分支的实际行为（逐字等价）；**不动** `reclaim_expired_leases` 的 F-04 reclaim_count 逻辑（#204 已落地，与本改正交）。
- 无新增依赖、无 config 字段变更、无 DB 迁移、无 API 变更。
- 不碰 no-human-takeover / no-model-hint lint 关注的字符串。
