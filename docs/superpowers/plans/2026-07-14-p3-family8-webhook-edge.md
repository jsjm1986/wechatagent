# P3 家族⑧ webhook 入口边缘加固 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A-06（last_inbound_at 统计 update 降 best-effort，防 Mongo 瞬时错误吞回复）+ A-05（无 appId 回落 default account 收敛：verify=false 且多账号时 400，防张冠李戴）+ A-03/A-04 doc 标注（生产不触发/已幂等缓解，已知边界不修）。

**Architecture:** 单文件 `src/webhooks.rs` 三处独立改动 + 台账 docs 标注。A-06 把统计 `update_one(...).await?` 改 `if let Err(e) = ... { warn }`。A-05 在 resolve_account_context 无 appId 分支包一层 `if !state.config.webhook_verify_signature { count>1→400 }`。A-03/A-04 加代码注释 + 台账状态改 WontFix。均为 DB 交互语义调整/注释，无独立纯函数 → 靠终审亲验 + 现有集成测无回归。

**Tech Stack:** Rust 2021，Axum，MongoDB。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-07-14-p3-family8-webhook-edge-design.md`（已获批 commit b03802c）。所有行号亲验于分支 `fix/p3-family8-webhook-edge`（基于含 #201 的最新 origin/main）。
- 红线：改代码前 100% 读懂相关代码；引用必亲验 file:line；不靠记忆（行号可能漂移，以 Read 到的真实代码为准）。
- **A-06 边界**：只降 `last_inbound_at/last_message_at/updated_at` 那**一个** update_one（约 :555-569）；inbound `insert_one`（:515-521）fail-close 不动、contact `find_one`/`upsert`（:523-538）的 `?` 不动。
- **A-05 收敛**：多账号 count 必须放在 `if !state.config.webhook_verify_signature { ... }` 内（verify=true 时无 appId 已被验签门 secret=None→400，不付 count 代价）。单账号（count ≤ 1）仍回落 default。不动"有 appId 查不到→400"（:997-999 已是 400）。
- **A-03/A-04 纯注释**：不改任何逻辑，只加代码注释 + 台账状态字段。
- 反过拟合红线：本族不为凑测试改逻辑；A-05/A-06 无独立纯函数，不硬造纯函数单测；若既有 webhook 集成测有低成本扩展点则加一条断言，否则终审亲验。
- check-no-human-takeover lint 扫 `src/`——新增行/注释禁词（人工接管/接管/人工/takeover/hand-off）；用中性词（账号/消息/归属/验签/统计/旁路）。
- check-no-model-hint lint 扫 `src/`——新增行禁模型/品牌名（gpt/claude/anthropic/deepseek/gemini/qwen/kimi 等）。本族注释无品牌名。
- baseline：`cargo test --lib` ≥ 350 passed / 0 failed，不触 4 PBT。
- 子任务派 subagent 一律省略 model 参数（继承主会话 opus）。**所有文件路径用 worktree 绝对路径前缀 `E:\yw\agiatme\工作项目\wechatagent\.claude\worktrees\fix-full-system-remediation\`**（主仓被并行会话占用）。
- 本地若撞 LNK1318 PDB（Windows-only 非代码错），`cargo check --lib` 足够验证编译。

## File Structure

- `src/webhooks.rs`：A-06 统计 update 降级（约 :555-569）；A-05 resolve_account_context 无 appId 分支（约 :1001-1005）；A-03 dedupe_key 回落注释（约 :486-489）；A-04 verify_webhook_signature 注释（约 :1766）。
- `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：A-03（:88 状态行）/ A-04（:100 状态行）状态更新为 WontFix。

单 Task（同文件多处直白改动 + 一处 docs，一个测试周期）：四处改动相互独立但都在 webhooks.rs，无中间可独立测的交付物拆分点。

---

## Task 1: webhook 边缘四条加固（A-06 降级 + A-05 收敛 + A-03/A-04 标注）

**Files:**
- Modify: `src/webhooks.rs`（A-06 :555-569 / A-05 :1001-1005 / A-03 :486-489 / A-04 :1766 附近）
- Modify: `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`（A-03/A-04 状态行）

**Interfaces:**
- Consumes: `state.db.contacts()`（Collection accessor）；`state.db.accounts().count_documents(doc!{}, None)`（→ `AppResult<u64>`）；`state.config.webhook_verify_signature: bool`；`AppError::BadRequest`。
- Produces: 无新公开接口。resolve_account_context 签名不变（仍 `async fn(&AppState, Option<&str>) -> AppResult<(String,String,Option<String>)>`）。

- [ ] **Step 1: 亲验四处真实现状**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && grep -n "last_inbound_at\|fn resolve_account_context\|webhook_verify_signature\|dedupe_key = effective\|fn verify_webhook_signature\|insert_one(&inbound" src/webhooks.rs`
Expected: 确认 A-06 统计 update（约 :555-569，`update_one` 写 last_inbound_at/last_message_at/updated_at）、inbound insert_one（约 :515）、resolve_account_context（约 :980，无 appId 分支约 :1001-1005）、dedupe_key 回落（约 :486-489）、verify_webhook_signature（约 :1766）。**实现者 Read 这几段全貌**后再改（行号以真实为准）。

- [ ] **Step 2: A-06 —— 统计 update 降 best-effort**

把 `src/webhooks.rs` 约 :555-569 的：

```rust
    state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! {
                "$set": {
                    "last_inbound_at": now,
                    "last_message_at": now,
                    "updated_at": now
                }
            },
            None,
        )
        .await?;
```

改为（`.await?` → `if let Err(e) = ... { warn }`）：

```rust
    // A-06：last_inbound_at/last_message_at/updated_at 是统计/信号旁路字段，落库失败不应连累
    // 本轮应答（inbound 已在上方 insert 成功、去重已保证）。降 best-effort：失败仅 warn，与紧邻的
    // collect_inbound_behavior_signals（下方）旁路纪律对齐。
    if let Err(e) = state
        .db
        .contacts()
        .update_one(
            doc! { "_id": contact.id },
            doc! {
                "$set": {
                    "last_inbound_at": now,
                    "last_message_at": now,
                    "updated_at": now
                }
            },
            None,
        )
        .await
    {
        tracing::warn!(contact_wxid = %from_wxid, error = ?e, "更新 last_inbound_at 失败（统计旁路，不影响应答）");
    }
```

（`from_wxid` 在该作用域可用——上方 insert/find 已用。实现者亲验 `contact.id` 与 `from_wxid` 的真实变量名；若 `now` 变量名不同按真实的来。inbound insert_one :515-521 与 contact find/upsert :523-538 一字不动。）

- [ ] **Step 3: A-05 —— 无 appId 分支收敛到 verify=false**

把 `src/webhooks.rs` resolve_account_context 的无 appId 分支（约 :1001-1005）：

```rust
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
        None,
    ))
```

改为（前置 `if !webhook_verify_signature` 内 count 守卫）：

```rust
    // A-05：无 appId 时的账号归属防线。验签门（handler 的 webhook_verify_signature 块）在本函数
    // 之后执行——verify=true 时无 appId → 返回 secret=None → verify_webhook_signature 必
    // SecretNotConfigured → 400，default 回退到不了副作用点，无需在此付 count 代价。仅当未开验签
    // （default 回退是唯一防线）时才校验：多账号无 appId 无法判断消息归属 → 400（防落到 default
    // account 张冠李戴）；单账号（≤1）无歧义 → 回落 default，不打断上游确实不带 appId 的单账号部署。
    if !state.config.webhook_verify_signature {
        let account_count = state.db.accounts().count_documents(doc! {}, None).await?;
        if account_count > 1 {
            return Err(AppError::BadRequest(
                "webhook 缺 appId 且存在多个账号，无法判断消息归属".into(),
            ));
        }
    }
    Ok((
        state.config.default_workspace_id.clone(),
        state.config.default_account_id.clone(),
        None,
    ))
```

（有 appId 分支 :984-999 一字不动，含 :997-999 "查不到→400"。实现者亲验 `count_documents` 返回类型是 `u64`，`> 1` 比较合法。`AppError::BadRequest` 已在文件 use 中——亲验；:997 已用同款，必在。）

- [ ] **Step 4: A-03 —— dedupe_key 回落加注释**

在 `src/webhooks.rs` dedupe_key 回落（约 :486-489）上方加注释（不改代码）：

```rust
    // A-03（已知边界，不修）：无任何 msgId（顶层 MsgId/NewMsgId + _mcp.sourceMsgId 全缺）时
    // dedupe_key 回落 payload-hash，同内容连发的第二条 hash 相同 → 命中 unique 索引被当 duplicate
    // 丢弃。生产 GeWe AddMsg 恒带 NewMsgId → effective_message_id 必有值走 message:{id} 分支，
    // 此路径仅自测 / 无 ID payload 触发。掺接收时刻/nonce 会削弱重放去重，收益不抵，故不修。
    let dedupe_key = effective_message_id
        .as_ref()
        .map(|id| format!("message:{id}"))
        .unwrap_or_else(|| format!("payload:{}", stable_payload_hash(&payload)));
```

（实现者按真实代码在正确位置插注释，保持原三行逻辑不变。）

- [ ] **Step 5: A-04 —— verify_webhook_signature 加注释**

在 `src/webhooks.rs` `fn verify_webhook_signature`（约 :1766）的定义上方（或函数内 skew 校验 :1787 附近）加注释（不改逻辑）：

```rust
/// A-04（已知边界，不修）：仅校验 secret 存在 + 时间戳 ±skew 窗口 + HMAC-SHA256，无 nonce /
/// 一次性签名记录。攻击者截获一条合法签名请求可在 skew（默认 300s）内原样重放。但重放无重复副作用：
/// AddMsg 重放命中 message-id dedupe 幂等短路、Offline/Online 重放幂等 $set、领导回复经
/// resolve_escalation 幂等 → 不产生重复发送。加 nonce 需状态存储，收益不抵成本，故不修。
fn verify_webhook_signature(
```

（若 :1766 上方已有 doc 注释块，实现者把 A-04 说明并入或紧邻追加，不破坏既有 doc。函数体一字不动。）

- [ ] **Step 6: A-03/A-04 —— 台账状态更新 WontFix**

在 `docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md`：
- A-03 的"状态: Open"（约 :88）→ `状态: WontFix（已知边界，doc 标注 —— 生产 GeWe 恒带 NewMsgId 走 message-id 分支，payload-hash 仅自测触发；家族⑧ #待补）`
- A-04 的"状态: Open"（约 :100）→ `状态: WontFix（已知边界，doc 标注 —— dedupe/幂等已缓解重放无重复副作用，加 nonce 收益不抵成本；家族⑧ #待补）`

（实现者 Read :78-100 确认 A-03/A-04 各自的"状态:"行真实文字后精确替换。只改这两行，不动 finding 其它内容。）

- [ ] **Step 7: 编译 + 全 lib 测**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && cargo check --lib 2>&1 | tail -8 && cargo test --lib 2>&1 | tail -5`
Expected: `Finished` + `test result: ok.` ≥ 350 passed / 0 failed（本族不新增/删 lib 测；A-06 降级 + A-05 分支 + 注释不影响 lib 测计数）。若 LNK1318（Windows-only）→ `cargo check --lib` 通过即可 + 人工核对。本地只跑 `cargo test --lib`，绝不跑集成测。

- [ ] **Step 8: Commit + lint 预验（commit 后跑）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/.claude/worktrees/fix-full-system-remediation" && git add src/webhooks.rs docs/superpowers/specs/2026-07-11-deep-logic-audit-findings.md && git commit -m "fix(webhook): last_inbound_at统计写降best-effort + 无appId多账号收敛400 + A-03/04已知边界标注 (A-05/06 P3家族⑧)"
bash scripts/check-no-human-takeover.sh origin/main HEAD
bash scripts/check-no-model-hint.sh origin/main HEAD
```

两个 lint 都必须 `ok: 0 violations`。只 add 这两个文件，绝不 `git add -A`。若 lint 报违规，修正后补一个修正 commit。

---

## Self-Review 结论

- **Spec coverage**：A-06 降级（Step 2）+ A-05 收敛（Step 3）+ A-03 注释（Step 4）+ A-04 注释（Step 5）+ 台账状态（Step 6）→ 覆盖设计全部四条 finding + 台账更新。设计非目标（不加 nonce/不改 payload-hash/不动 fail-close insert）通过"只改指定处"落实。
- **Placeholder scan**：无 TBD/TODO。所有代码块是完整可粘贴的真实替换。台账 Step 6 的 "#待补" 指 PR 号占位——实现者不填 PR 号（PR 建好后由主控补），但状态字段本身完整改为 WontFix，非 placeholder。
- **Type consistency**：resolve_account_context 签名不变（返回 triple）；A-06 update 降级后无返回值变化（本就是语句）；`count_documents` → u64、`> 1` 合法；`AppError::BadRequest` 既有。
- **反过拟合**：本族无独立纯函数，不硬造单测；A-05/A-06 靠终审代码级亲验 + 现有 webhook 集成测无回归（改动直白：一个 `?`→best-effort、一个 if 分支、两处注释、两行 docs）。
- **红线合规**：inbound insert fail-close / contact 查询 `?` / 验签门 / payload-hash 逻辑 / 有 appId 分支全不动；新增注释中性词无禁词、无模型名；baseline 不回退；worktree 绝对路径。
