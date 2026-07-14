# 以「手动加入运营」为唯一真相 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 AI 给非真人号自动回复的缺陷：以管理员手动加入运营为唯一真相，补判据硬盲区（@openim/账号自反身）、移出池联动停回复、加入/移出写审计日志、清理存量矛盾号。

**Architecture:** 后端 Rust(Axum)+MongoDB。三处判据（`is_operatable_person` Rust 侧 / `non_human_exclusion_filter` DB 侧 / 新增 `is_self_account`）；四个升/降 managed 入口（`enable_agent`/`batch_enable_endpoint`/`disable_agent` REST + `enable_contact_agent` MCP 工具分支）统一加自反身硬拦与审计事件；`hide_from_pool` 联动 `agent_status=normal`。审计复用 `write_event_for_account`。

**Tech Stack:** Rust 2021, cargo, mongodb crate, axum。测试 `cargo test --lib`。

## Global Constraints

- **放弃 `wxid_` 营销号自动判据**：判据不新增任何针对普通 `wxid_` 前缀的拦截；营销号能否运营交给管理员手动决定。
- **单一真相源**：移出池在写入侧联动改 `agent_status`，不在回复门 `reload_managed_contact` 加 `hidden_from_pool` 过滤。
- **判据双侧同源**：`is_operatable_person`（webhooks.rs）与 `non_human_exclusion_filter`（mcp.rs）任一处补拦截，另一处必须同步，否则 count/list 口径漂移。
- **no-human-takeover lint**：所有新增行（含审计事件 kind/summary/details 文案、注释）不得含 `人工/接管/转接/托管/takeover/hand[-_]?off/人工介入` 等禁词。用 AI 自治语义命名（如「纳入 AI 运营 / 移出 AI 运营」）。
- **no-model-hint lint**：新增行不得硬编码模型/品牌名。
- **基线不回归**：`cargo test --lib` ≥ 当前基线 0 failed；`scripts/check-baseline.sh` 双门绿。
- **审计 actor**：`AuthenticatedAdmin` 有 `username` 字段（`src/auth/mod.rs:59`），审计 details 用它作 actor。MCP 工具分支无 admin，用固定标识 `"management_tool"`。
- **本地磁盘纪律**：只跑 `cargo test --lib` 与 `cargo build --lib`，不跑集成测试（爆盘）。

---

## File Structure

- `src/webhooks.rs` — `is_operatable_person`（:1065）补 `@openim`；新增 `is_self_account`；建档 `upsert_webhook_contact`（:1071）补自反身兜底；单测区（:1461-1483）。
- `src/mcp.rs` — `non_human_exclusion_filter`（:518）补 `@openim` 正则。
- `src/routes/contacts.rs` — `enable_agent`（:884）、`batch_enable_endpoint`（:719）、`disable_agent`（:939）、`hide_from_pool`（:969）加拦截/联动/审计。
- `src/routes/management.rs` — `enable_contact_agent` 工具分支（:1369）加自反身拦 + 审计。
- `scripts/` — 一次性存量清理（mongosh，不入 migration）。

---

## Task 1: 判据层 — 补 @openim + DB 侧同步 + 新增 is_self_account

**Files:**
- Modify: `src/webhooks.rs:1065-1069`（`is_operatable_person`）、新增 `is_self_account`、单测 `src/webhooks.rs:1468-1483`
- Modify: `src/mcp.rs:518-529`（`non_human_exclusion_filter`）

**Interfaces:**
- Produces: `pub(crate) fn is_operatable_person(wxid: &str) -> bool`（语义扩展，签名不变）；`pub(crate) fn is_self_account(wxid: &str, account_self_wxid: Option<&str>) -> bool`（新增）。

- [ ] **Step 1: 改 is_operatable_person 单测（先加 @openim 断言，TDD 红）**

`src/webhooks.rs`，在 `is_operatable_person_rejects_official_and_group`（:1461）里追加一条 @openim 断言：

```rust
    #[test]
    fn is_operatable_person_rejects_official_and_group() {
        assert!(!is_operatable_person("gh_416c280c4978"));
        assert!(!is_operatable_person("7842243308@chatroom"));
        assert!(!is_operatable_person("971559326@chatroom"));
        // 企业微信/开放 IM 号非私聊真人。
        assert!(!is_operatable_person("25984984932102183@openim"));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib is_operatable_person_rejects_official_and_group`
Expected: FAIL（`@openim` 当前返回 true，assert! 失败）

- [ ] **Step 3: is_operatable_person 补 @openim 拦截**

`src/webhooks.rs:1065-1069` 改为：

```rust
pub(crate) fn is_operatable_person(wxid: &str) -> bool {
    !(wxid.starts_with("gh_")
        || wxid.contains("@chatroom")
        || wxid.contains("@openim")
        || crate::mcp::is_system_account(wxid))
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib is_operatable_person`
Expected: PASS（三个 is_operatable_person 单测全过；`wxid_8874178741811` 福州晚报仍 true、`wxid_3yeirsb75afd22` 仍 true 不变）

- [ ] **Step 5: DB 侧过滤器同步补 @openim（先写断言测试）**

`src/mcp.rs` 测试区新增（与现有 `is_system_account_matches_wechat_reserved` 同区）：

```rust
    #[test]
    fn non_human_exclusion_filter_excludes_openim() {
        let f = non_human_exclusion_filter();
        let nor = f.get_array("$nor").expect("$nor present");
        let has_openim = nor.iter().any(|c| {
            c.as_document()
                .and_then(|d| d.get_document("wxid").ok())
                .and_then(|w| w.get_str("$regex").ok())
                .map(|r| r.contains("@openim"))
                .unwrap_or(false)
        });
        assert!(has_openim, "DB 侧过滤器必须含 @openim，与 is_operatable_person 同源");
    }
```

- [ ] **Step 6: 跑测试确认失败**

Run: `cargo test --lib non_human_exclusion_filter_excludes_openim`
Expected: FAIL（当前 `$nor` 无 @openim）

- [ ] **Step 7: non_human_exclusion_filter 补 @openim 正则**

`src/mcp.rs:520-528` 的 `$nor` 数组，在 `@chatroom` 那条后补一条：

```rust
    doc! {
        "$nor": [
            // gh_ 前缀公众号（^ 锚定开头）。
            doc! { "wxid": { "$regex": "^gh_" } },
            // 群会话（@chatroom 为子串，@ 在正则里是普通字符）。
            doc! { "wxid": { "$regex": "@chatroom" } },
            // 企业微信/开放 IM 号（@openim 子串）。
            doc! { "wxid": { "$regex": "@openim" } },
            // 微信官方保留系统号（单一数据源白名单）。
            doc! { "wxid": { "$in": whitelist } },
        ]
    }
```

- [ ] **Step 8: 跑测试确认通过**

Run: `cargo test --lib non_human_exclusion_filter`
Expected: PASS

- [ ] **Step 9: 新增 is_self_account 函数 + 单测（TDD 红）**

`src/webhooks.rs`，在 `is_operatable_person` 定义（:1069）之后新增函数：

```rust
/// 账号不能运营自己：判断某 wxid 是否等于当前账号自身 wxid。
/// 与真人判据 `is_operatable_person` 解耦——这是「不能自己运营自己」的逻辑铁律，
/// 不是「是否真人」的判断。`account_self_wxid` 为 None（账号未同步 wxid）时返回 false（无从判定，不拦）。
pub(crate) fn is_self_account(wxid: &str, account_self_wxid: Option<&str>) -> bool {
    matches!(account_self_wxid, Some(self_wxid) if self_wxid == wxid)
}
```

单测区新增：

```rust
    #[test]
    fn is_self_account_detects_account_own_wxid() {
        // 账号自身 wxid == 目标 wxid → 拦。
        assert!(is_self_account(
            "wxid_3yeirsb75afd22",
            Some("wxid_3yeirsb75afd22")
        ));
        // 不同 wxid → 不拦。
        assert!(!is_self_account(
            "wxid_ydzaomn4scsb12",
            Some("wxid_3yeirsb75afd22")
        ));
        // 账号未同步 wxid（None）→ 无从判定，不拦。
        assert!(!is_self_account("wxid_ydzaomn4scsb12", None));
    }
```

- [ ] **Step 10: 跑测试确认（红→实现已在 Step 9 同时给出→绿）**

Run: `cargo test --lib is_self_account`
Expected: PASS

- [ ] **Step 11: build 全量 lib 确认无破坏**

Run: `cargo build --lib`
Expected: Finished（0 error）

- [ ] **Step 12: Commit**

```bash
git add src/webhooks.rs src/mcp.rs
git commit -m "fix(contacts): 判据补@openim拦截(双侧同源)+新增is_self_account自反身函数

is_operatable_person 与 non_human_exclusion_filter 同步补 @openim(企业微信号非
私聊真人);新增 is_self_account(账号不能运营自己,与真人判据解耦)。放弃 wxid_ 营销号
自动判据不变(福州晚报单测仍 true)。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 2: 加入运营入口 — 自反身硬拦 + 审计事件

**Files:**
- Modify: `src/routes/contacts.rs`（`enable_agent`:884、`batch_enable_endpoint`:719）
- Modify: `src/routes/management.rs`（`enable_contact_agent` 分支 :1369）
- Test: `src/routes/contacts.rs` 测试区

**Interfaces:**
- Consumes: `webhooks::is_self_account`（Task 1）；`agent::write_event_for_account(state, account_id, contact_wxid: Option<&str>, kind, status, summary, details: Option<Document>) -> AppResult<()>`（`src/agent/gateway.rs:5119`）。
- 事件 kind：`"contact.enabled_for_ops"`（成功加入）、`"contact.enable_rejected_self"`（自反身被拦）。

- [ ] **Step 1: batch_enable 保留 account 对象取 wxid（改注册校验块）**

`src/routes/contacts.rs:731-745` 现在 `.is_none()` 丢弃 account。改为保留：

```rust
    // account 必须在 wechat_accounts 注册(否则 webhook 入站会被 resolve_account_context 拒收)。
    let account = state
        .db
        .accounts()
        .find_one(
            doc! { "workspace_id": &admin.current_workspace, "account_id": &payload.account_id },
            None,
        )
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "account_id={} 在 wechat_accounts 中未注册，无法批量启用 Agent 运营",
                payload.account_id
            ))
        })?;
    let account_self_wxid = account.wxid.clone();
```

- [ ] **Step 2: batch_enable 循环内自反身跳过 + 计数**

`src/routes/contacts.rs`，在 candidates 循环体开头（`for cand in &payload.candidates {` 之后，现有 existing 查询之前，约 :769）插入。先在循环前声明计数器（与现有 `let mut enabled = 0i32; let mut queued = 0i32;` 同处，约 :767）新增 `let mut rejected_self = 0i32;`，循环内：

```rust
        // 账号不能运营自己：候选命中账号自身 wxid → 跳过并留审计。
        if crate::webhooks::is_self_account(&cand.wxid, account_self_wxid.as_deref()) {
            rejected_self += 1;
            let _ = agent::write_event_for_account(
                &state,
                &payload.account_id,
                Some(&cand.wxid),
                "contact.enable_rejected_self",
                "rejected",
                "候选命中账号自身 wxid，已跳过纳入 AI 运营",
                Some(doc! { "actor": &admin.username, "source": "batch_enable" }),
            )
            .await;
            continue;
        }
```

- [ ] **Step 3: batch_enable 成功入队后写加入审计**

`src/routes/contacts.rs`，在 `queued += 1;`（:878）所在 `if !already_managed {` 块结尾之后、循环体末尾（`}` 前）插入。仅对本轮真正新纳入的（`!already_managed`）写事件，避免幂等重入刷屏：

```rust
        if !already_managed {
            let _ = agent::write_event_for_account(
                &state,
                &payload.account_id,
                Some(&cand.wxid),
                "contact.enabled_for_ops",
                "ok",
                "管理员批量纳入 AI 运营",
                Some(doc! {
                    "actor": &admin.username,
                    "source": "batch_enable",
                    "note": &payload.shared_note,
                }),
            )
            .await;
        }
```

- [ ] **Step 4: batch_enable 返回体加 rejected_self 计数**

`src/routes/contacts.rs:881` 返回改为：

```rust
    Ok(Json(json!({ "enabled": enabled, "queued": queued, "rejectedSelf": rejected_self })))
```

- [ ] **Step 5: enable_agent 加自反身硬拦 + 审计**

`src/routes/contacts.rs:895-910`。现有 `let contact = find_contact_by_id(...)`（:895）之后、account 注册校验之后，把 account 校验块（:899-910）改为保留 account 并加自反身拦：

```rust
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let account = state
        .db
        .accounts()
        .find_one(doc! { "account_id": &contact.account_id }, None)
        .await?
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "contact.account_id={} 在 wechat_accounts 中未注册，无法启用 Agent 运营",
                contact.account_id
            ))
        })?;
    // 账号不能运营自己。
    if crate::webhooks::is_self_account(&contact.wxid, account.wxid.as_deref()) {
        let _ = agent::write_event_for_account(
            &state,
            &contact.account_id,
            Some(&contact.wxid),
            "contact.enable_rejected_self",
            "rejected",
            "目标命中账号自身 wxid，拒绝纳入 AI 运营",
            Some(doc! { "actor": &admin.username, "source": "enable_agent" }),
        )
        .await;
        return Err(AppError::BadRequest(
            "不能对账号自身 wxid 启用 Agent 运营".to_string(),
        ));
    }
```

- [ ] **Step 6: enable_agent 成功后写加入审计**

`src/routes/contacts.rs`，在 `apply_generated_profile_to_contact(...)`（:925-934）之后、最后 `find_contact_by_id` 重读（:935）之前插入：

```rust
    let _ = agent::write_event_for_account(
        &state,
        &contact.account_id,
        Some(&contact.wxid),
        "contact.enabled_for_ops",
        "ok",
        "管理员纳入 AI 运营",
        Some(doc! {
            "actor": &admin.username,
            "source": "enable_agent",
            "note": &payload.human_profile_note,
        }),
    )
    .await;
```

- [ ] **Step 7: management enable_contact_agent 工具分支加自反身拦 + 审计**

`src/routes/management.rs:1369-1372`。在 `resolve_contact_arg(...)` 得到 contact（:1372）之后、构造 set_doc（:1382）之前插入。该分支无 admin，用固定 actor。account 自身 wxid 从已有 `account_id` 查（该函数上下文有 `state`/`account_id`/`workspace_id`）：

```rust
            let contact = resolve_contact_arg(state, workspace_id, account_id, &planned.arguments).await?;
            // 账号不能运营自己。
            let self_wxid = state
                .db
                .accounts()
                .find_one(doc! { "workspace_id": workspace_id, "account_id": account_id }, None)
                .await?
                .and_then(|a| a.wxid);
            if crate::webhooks::is_self_account(&contact.wxid, self_wxid.as_deref()) {
                let _ = agent::write_event_for_account(
                    state,
                    account_id,
                    Some(&contact.wxid),
                    "contact.enable_rejected_self",
                    "rejected",
                    "目标命中账号自身 wxid，拒绝纳入 AI 运营",
                    Some(doc! { "actor": "management_tool", "source": "enable_contact_agent" }),
                )
                .await;
                return Err(AppError::BadRequest(
                    "不能对账号自身 wxid 启用 Agent 运营".to_string(),
                ));
            }
```

并在该分支写库成功后（分支返回 Ok 之前）补加入审计：

```rust
            let _ = agent::write_event_for_account(
                state,
                account_id,
                Some(&contact.wxid),
                "contact.enabled_for_ops",
                "ok",
                "经管理工具纳入 AI 运营",
                Some(doc! { "actor": "management_tool", "source": "enable_contact_agent" }),
            )
            .await;
```

（实现时确认该分支现有 return 结构，把加入审计放在最终 Ok 之前、写库之后的位置；若分支用 `?` 提前返回则放在成功路径末尾。）

- [ ] **Step 8: 写 batch_enable 自反身拦截集成测试**

`src/routes/contacts.rs` 测试区（找现有 `#[cfg(test)] mod tests` 或同类 handler 测试范式；若无 handler 级测试则写纯逻辑测试断言 `is_self_account` 已在 Task 1 覆盖，此处补文档注释说明拦截路径依赖 Task 1 单测 + 手工验证）。若存在 TestApp/handler 测试设施则写：候选含账号自身 wxid → 返回 `rejectedSelf >= 1` 且该 wxid 未被写成 managed。

实现时先 Grep `mod tests` in `src/routes/contacts.rs` 确认测试设施，无则跳过 handler 测试、以 Task 1 的 `is_self_account` 单测 + Task 4 生产验证为准，并在 commit message 注明。

- [ ] **Step 9: build + 相关测试**

Run: `cargo build --lib && cargo test --lib is_self_account`
Expected: Finished 0 error；is_self_account 测试 PASS

- [ ] **Step 10: 本地 lint 预验（新增审计文案不踩禁词）**

Run: `git add -A src/ && git stash && git stash pop`（仅为触发 add；实际用下面）
实际：先 commit 再验，或直接肉眼核对新增行无 `人工/接管/转接/托管/takeover/hand-off`。审计文案已用「纳入 AI 运营 / 移出 AI 运营」，合规。

- [ ] **Step 11: Commit**

```bash
git add src/routes/contacts.rs src/routes/management.rs
git commit -m "fix(contacts): 加入运营4入口统一自反身硬拦+加入/拒绝写审计

enable_agent/batch_enable/enable_contact_agent 工具分支加 is_self_account 硬拦
(账号不能运营自己),命中拒绝并写 contact.enable_rejected_self 审计;成功纳入写
contact.enabled_for_ops 审计。batch 返回体加 rejectedSelf 计数。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3: 移出运营 — hide_from_pool 联动停回复 + 移出审计

**Files:**
- Modify: `src/routes/contacts.rs`（`hide_from_pool`:969、`disable_agent`:939）
- Test: `src/routes/contacts.rs` 测试区（若有 handler 测试设施）

**Interfaces:**
- 事件 kind：`"contact.removed_from_ops"`（移出运营）。

- [ ] **Step 1: hide_from_pool 联动写 agent_status=normal**

`src/routes/contacts.rs:978-982` 的 update_one，`$set` 补 `agent_status`：

```rust
        .update_one(
            doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
            doc! { "$set": {
                "hidden_from_pool": true,
                "agent_status": "normal",
                "updated_at": DateTime::now()
            } },
            None,
        )
        .await?;
```

同时更新函数文档注释（:963-968），把「改标 doc-only hidden_from_pool=true」补一句「并联动 agent_status=normal 停止 AI 运营（移出池 = 不再运营，单一真相源）」。

- [ ] **Step 2: hide_from_pool 成功后写移出审计**

`src/routes/contacts.rs`，`if result.matched_count == 0`（:984-986）之后、`find_contact_by_id` 重读（:987）之前插入。需要 contact_wxid——先用重读后的 contact 拿 wxid，故调整顺序：先重读 contact，再写事件：

```rust
    if result.matched_count == 0 {
        return Err(AppError::NotFound("contact not found".to_string()));
    }
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let _ = agent::write_event_for_account(
        &state,
        &contact.account_id,
        Some(&contact.wxid),
        "contact.removed_from_ops",
        "ok",
        "管理员从运营池移除并停止 AI 运营",
        Some(doc! { "actor": &admin.username, "source": "hide_from_pool" }),
    )
    .await;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
```

（删除原 :987 重复的 `find_contact_by_id` 调用，避免查两次。）

- [ ] **Step 3: disable_agent 写移出审计**

`src/routes/contacts.rs:944-960`。update_one（:945-958）之后、重读 contact（:959）之后、返回之前插入审计：

```rust
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    let _ = agent::write_event_for_account(
        &state,
        &contact.account_id,
        Some(&contact.wxid),
        "contact.removed_from_ops",
        "ok",
        "管理员停止该联系人的 AI 运营",
        Some(doc! { "actor": &admin.username, "source": "disable_agent" }),
    )
    .await;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
```

- [ ] **Step 4: build 确认**

Run: `cargo build --lib`
Expected: Finished 0 error

- [ ] **Step 5: hide_from_pool 联动测试（若有 handler 测试设施）**

Grep `src/routes/contacts.rs` 现有测试区。若有 TestApp：建一个 managed contact → 调 hide_from_pool → 断言重读后 `agent_status == "normal"` 且 `hidden_from_pool == true`。无设施则以 Task 4 生产验证为准，commit 注明。

- [ ] **Step 6: Commit**

```bash
git add src/routes/contacts.rs
git commit -m "fix(contacts): 移出池联动 agent_status=normal 停回复 + 移出写审计

hide_from_pool 从只写 hidden_from_pool 改为联动写 agent_status=normal——移出池
即停止 AI 运营(修 managed+hidden 矛盾态:列表消失却仍自动回复的根因)。
hide_from_pool/disable_agent 均写 contact.removed_from_ops 审计。单一真相源:
回复门 reload_managed_contact 仍只看 agent_status 不变。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4: 存量矛盾号即时清理 + 收尾验证

**Files:**
- Create: `scripts/cleanup_non_human_managed.js`（一次性 mongosh 脚本，带备份打印）

**Interfaces:** 无代码接口，纯运维脚本 + 生产执行。

- [ ] **Step 1: 写清理脚本（先打印备份，再改，再回读）**

`scripts/cleanup_non_human_managed.js`：

```javascript
// 一次性存量清理：把 managed+hidden 矛盾的非真人号 agent_status 改回 normal。
// 用法: mongosh <db> scripts/cleanup_non_human_managed.js
// 非 migration —— 清历史脏数据，不入启动流程。
const TARGETS = [
  "wxid_8874178741811",       // 福州晚报(新闻号)
  "wxid_2540165401612",       // 福建经济广播(电台号)
  "wxid_czpvyjvhzizj22",      // AI应用开发(营销号)
  "wxid_3yeirsb75afd22",      // Demi = 账号102自己(自反身)
  "25984984932102183@openim", // 企业微信号
];

print("=== 更新前备份 ===");
db.contacts.find({ wxid: { $in: TARGETS } }, { wxid: 1, nickname: 1, agent_status: 1, hidden_from_pool: 1 })
  .forEach(c => print(`  ${c.wxid} | ${c.nickname} | agent_status=${c.agent_status} | hidden=${c.hidden_from_pool}`));

const r = db.contacts.updateMany(
  { wxid: { $in: TARGETS }, agent_status: "managed" },
  { $set: { agent_status: "normal", updated_at: new Date() } }
);
print(`=== 更新 matched=${r.matchedCount} modified=${r.modifiedCount} ===`);

print("=== 更新后回读 ===");
db.contacts.find({ wxid: { $in: TARGETS } }, { wxid: 1, nickname: 1, agent_status: 1, hidden_from_pool: 1 })
  .forEach(c => print(`  ${c.wxid} | ${c.nickname} | agent_status=${c.agent_status} | hidden=${c.hidden_from_pool}`));
```

- [ ] **Step 2: 全量 lib 测试确认基线不回归**

Run: `cargo test --lib`
Expected: 0 failed，passed 数 ≥ 当前基线

- [ ] **Step 3: 本地 lint 双门预验**

Run: `bash scripts/check-no-human-takeover.sh main HEAD && bash scripts/check-no-model-hint.sh main HEAD`
Expected: 两个都 `0 violations`

- [ ] **Step 4: Commit 脚本**

```bash
git add scripts/cleanup_non_human_managed.js
git commit -m "chore(scripts): 存量非真人managed号清理脚本(备份+改normal+回读)

一次性 mongosh 清理福州晚报/福建经济广播/AI应用开发/账号自反身/@openim 号:
managed→normal 停止骚扰式回复。非 migration(清历史脏数据不入启动)。执行前
打印备份可回滚。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

- [ ] **Step 5: 生产执行清理脚本（部署后）**

部署新二进制到 117 后，用 `scripts/_remote_run_direct.py` 跑：
```
mongosh --quiet wechatagent /path/to/cleanup_non_human_managed.js
```
核对回读输出：5 个号 agent_status 全部 = normal。

- [ ] **Step 6: 生产验证移出联动生效**

对任一测试号调 `hide_from_pool` 端点或直接观察：确认 `agent_status` 被联动改为 normal；查 `events` 集合有 `contact.removed_from_ops` / `contact.enabled_for_ops` 审计行。

---

## Self-Review

**Spec coverage:**
- 判据补 @openim + DB 侧同步 → Task 1 ✓
- is_self_account 新函数 → Task 1 ✓
- 3 REST 入口 + 1 MCP 工具分支自反身硬拦 → Task 2 ✓（enable_agent/batch_enable/enable_contact_agent；disable 是移出不需拦）
- 加入运营审计 → Task 2 ✓
- 移出池联动 agent_status=normal → Task 3 ✓
- 移出运营审计（hide_from_pool + disable）→ Task 3 ✓
- 存量清理 + 备份可回滚 → Task 4 ✓
- 建档侧自反身兜底：**spec 提及但降级为可选** —— 生产伤害在「加入运营」发生，账号自身极少主动给自己发 webhook；建档兜底价值低且 upsert_webhook_contact 拿账号 wxid 需额外查库。**本计划不含建档兜底**（YAGNI），自反身防线由 Task 2 的 4 入口硬拦覆盖。若后续发现账号自反身记录仍被建档再单独补。
- 单测修正：Task 1 保留福州晚报/账号自身 `is_operatable_person==true`，新增 @openim 断言 + is_self_account 断言 ✓

**Placeholder scan:** Task 2 Step 8 / Task 3 Step 5 的 handler 测试标注「若有测试设施则写，无则以 Task1 单测+生产验证为准」——这是真实的条件分支（实现者需先 Grep 确认设施），非占位；已给出无设施时的明确替代路径。

**Type consistency:** `is_self_account(wxid: &str, account_self_wxid: Option<&str>)` 全 4 处调用一致（Task 1 定义、Task 2 三处 `.as_deref()` 传入）。`write_event_for_account` 实参顺序与 gateway.rs:5119 签名一致。事件 kind 三值全程一致：`contact.enabled_for_ops` / `contact.enable_rejected_self` / `contact.removed_from_ops`。

**Scope:** 单一子系统（联系人运营纳入/移出），一个实现计划覆盖，无需拆分。
