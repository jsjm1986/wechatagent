# 用户运营池「真人漏斗」重设计 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把用户运营池从「昵称全是 Demi、无头像、混入公众号/群、看不懂」修成「主动来找过你的私聊真人」的干净、可理解、好用的运营工作台。

**Architecture:** 三层改动共用一个纯函数真人判据 `is_operatable_person(wxid)`。入口层（webhook 建档）拦非真人 + 按 roster 富化昵称头像；migration 层一次性清理存量脏数据；展示层后端读时兜底富化/排序 + 前端 ContactsView 重构成漏斗工作台（分档差异化行 + 标签 + 超时提醒 + 批量启用）。

**Tech Stack:** Rust (Axum) 后端 + MongoDB migration（run_step 语义）+ React 19 + TypeScript + Zustand 前端 + Vitest。

## Global Constraints

- 真人判据（黑名单，spec 亲验）：`wxid.starts_with("gh_") || wxid.contains("@chatroom")` → 非真人。roster 全量好友 gh_=0/群=0，故「在 roster」=「真人」。
- 昵称/头像唯一正确来源 = `roster_snapshots`（webhook payload 里发件人只有 wxid，无昵称头像；`_mcp.nickName` 是账号自己昵称 "Demi"，禁止再用 `find_string` 取 nickName）。
- migration 安全红线：只碰 contacts 的 nickname/avatar_url/删非真人 normal 行；**绝不动** agent_status/operation_state/agent_profile/memory_summary/commitments；managed 一律保留（只清昵称不删）；**绝不带 APP_ENV=production 守卫**（必须无条件对所有环境存量生效）；幂等；**不删 conversation_messages**。
- normal 联系人入站不调 LLM（现有语义不变）；待启用摘要 = 原文截断，非智能摘要。
- `ApiContact::from(Contact)`（models.rs:3373）是纯 From 无 DB 访问——`last_inbound_preview` 必须在 `list_contacts` 转换后单独 N+1 查询填充，不能塞进 From。
- 前端遵守 `docs/frontend-design-system.md`：muted 灰 #64748b 副标题、teal #0f766e 仅 AI/managed 状态、AI 蓝 #2563eb 仅主操作/选中。
- 前端文案不得含 `scripts/check-no-human-takeover` 禁用词（「人工接管/接管/hand-off」等）；「待启用/Agent/运营池」安全。
- 基线门（每个后端任务后）：`cargo test --lib` ≥ 350 passed / 0 failed。全前端任务后：`cd frontend && npm run build` + `npx vitest run`。
- 不改 prompt / gateway 决策链 / principal 请示通道 / quiet-hours / batch-enable 后端端点。

---

### Task 1: 真人判据纯函数 `is_operatable_person`

**Files:**
- Modify: `src/webhooks.rs`（在 `upsert_webhook_contact` 上方新增 pub(crate) 纯函数 + `#[cfg(test)]` 单测；文件末尾已有 `mod tests`，测试加到那里或就近新 mod）
- 该函数需被 migration（Task 4）复用，故用 `pub(crate)`。

**Interfaces:**
- Produces: `pub(crate) fn is_operatable_person(wxid: &str) -> bool` —— gh_ 前缀或 @chatroom 后缀返 false，其余 true。

- [ ] **Step 1: 写失败测试**

在 `src/webhooks.rs` 的 `#[cfg(test)] mod tests`（文件已有测试模块，约 :1240 起）内新增：

```rust
#[test]
fn is_operatable_person_rejects_official_and_group() {
    assert!(!is_operatable_person("gh_416c280c4978"));
    assert!(!is_operatable_person("7842243308@chatroom"));
    assert!(!is_operatable_person("971559326@chatroom"));
}

#[test]
fn is_operatable_person_accepts_real_wxid() {
    assert!(is_operatable_person("wxid_ydzaomn4scsb12"));
    assert!(is_operatable_person("wxid_3yeirsb75afd22"));
    // 边界：gh 出现在中间不算公众号（只认前缀）。
    assert!(is_operatable_person("wxid_gh_not_prefix"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib is_operatable_person`
Expected: 编译失败 `cannot find function is_operatable_person`。

- [ ] **Step 3: 实现纯函数**

在 `src/webhooks.rs` `upsert_webhook_contact` 函数定义上方新增：

```rust
/// 真人判据（黑名单）：gh_ 公众号、@chatroom 群消息不是能运营的私聊真人。
/// roster 全量好友里 gh_=0/群=0（117 亲验），故这两类天然不在好友名册。
/// webhook 建档与 m029 存量清理共用此判据，杜绝两处漂移。
pub(crate) fn is_operatable_person(wxid: &str) -> bool {
    !(wxid.starts_with("gh_") || wxid.contains("@chatroom"))
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib is_operatable_person`
Expected: 2 passed。

- [ ] **Step 5: Commit**

```bash
git add src/webhooks.rs
git commit -m "feat(user-ops): add is_operatable_person real-contact predicate (gh_/@chatroom blacklist)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 2: roster 富化 helper `roster_identity_for`

**Files:**
- Modify: `src/webhooks.rs`（新增 async helper 查 roster_snapshots 取某 wxid 的 nickname/avatar_url）
- 参考：`src/agent/...` 或 `src/mcp.rs` 已有 `read_roster_snapshot`（spec 提及）；先 Grep 确认签名再用。

**Interfaces:**
- Consumes: `RosterSnapshot` / `RosterFriend`（models.rs，字段 wxid/nickname/avatar_url 亲验存在）。
- Produces: `async fn roster_identity_for(state: &AppState, workspace_id: &str, account_id: &str, wxid: &str) -> Option<(Option<String>, Option<String>)>` —— 返回 `(nickname, avatar_url)`；roster 无该 wxid 或读快照失败 → None（交调用方留空）。

- [ ] **Step 1: 先亲验 roster 读取 API**

Run: `grep -n "read_roster_snapshot\|struct RosterSnapshot\|struct RosterFriend\|pub friends" src/mcp.rs src/models.rs`
读懂 `read_roster_snapshot` 的确切签名（参数顺序、返回 `AppResult<Option<RosterSnapshot>>` 还是别的）与 `RosterFriend` 字段名后再写。**不得凭本 plan 的字段名假设——以亲验为准。**

- [ ] **Step 2: 写失败测试（纯逻辑部分抽出）**

roster 查找是「在 friends 里按 wxid 找一条，返回其 nickname/avatar_url」。把纯查找逻辑抽成纯函数便于测：

在 webhooks.rs 测试模块新增：

```rust
#[test]
fn pick_identity_from_friends_finds_match() {
    let friends = vec![
        RosterFriend { wxid: "wxid_a".into(), nickname: Some("小明".into()), avatar_url: Some("http://img/a".into()), remark: None, sex: Some(0), is_non_human: false },
        RosterFriend { wxid: "wxid_b".into(), nickname: None, avatar_url: None, remark: None, sex: Some(0), is_non_human: false },
    ];
    assert_eq!(pick_identity_from_friends(&friends, "wxid_a"), Some((Some("小明".to_string()), Some("http://img/a".to_string()))));
    assert_eq!(pick_identity_from_friends(&friends, "wxid_b"), Some((None, None)));
    assert_eq!(pick_identity_from_friends(&friends, "wxid_missing"), None);
}
```

**注意**：`RosterFriend` 的真实字段以 Step 1 亲验为准；若字段名/可见性不符，调整构造体字面量使编译通过（红线：不猜字段）。

- [ ] **Step 3: 运行确认失败**

Run: `cargo test --lib pick_identity_from_friends`
Expected: 编译失败 `cannot find function pick_identity_from_friends`。

- [ ] **Step 4: 实现纯函数 + async 包装**

```rust
/// 从 roster friends 里按 wxid 找身份（nickname, avatar_url）。找不到返 None。
pub(crate) fn pick_identity_from_friends(
    friends: &[RosterFriend],
    wxid: &str,
) -> Option<(Option<String>, Option<String>)> {
    friends
        .iter()
        .find(|f| f.wxid == wxid)
        .map(|f| (f.nickname.clone(), f.avatar_url.clone()))
}

/// 查 roster 快照拿某 wxid 的 (nickname, avatar_url)。快照缺失/读失败/无该 wxid → None。
/// best-effort：读失败只返 None，不阻断建档。
async fn roster_identity_for(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    wxid: &str,
) -> Option<(Option<String>, Option<String>)> {
    // 以 Step 1 亲验的 read_roster_snapshot 签名为准调用。
    let snap = crate::mcp::read_roster_snapshot(state, workspace_id, account_id)
        .await
        .ok()
        .flatten()?;
    pick_identity_from_friends(&snap.friends, wxid)
}
```

- [ ] **Step 5: 运行确认通过**

Run: `cargo test --lib pick_identity_from_friends`
Expected: 1 passed。`cargo test --lib` ≥ 350 / 0。

- [ ] **Step 6: Commit**

```bash
git add src/webhooks.rs
git commit -m "feat(user-ops): add roster identity enrichment helper for webhook contact upsert

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 3: webhook 建档接线过滤 + 富化 + 删 Demi 取值

**Files:**
- Modify: `src/webhooks.rs:1030-1121`（`upsert_webhook_contact`）
- Modify: `src/webhooks.rs:533-540`（调用点，处理 `Ok(None)` 优雅跳过）

**Interfaces:**
- Consumes: `is_operatable_person`（Task 1）、`roster_identity_for`（Task 2）。
- `upsert_webhook_contact` 签名不变，仍返回 `AppResult<Option<Contact>>`；非真人时返回 `Ok(None)`。

- [ ] **Step 1: 亲验调用点当前行为**

Read `src/webhooks.rs:520-540`。当前：`contact.is_none()` 时调 `upsert_webhook_contact`，然后 `let Some(contact) = contact else { return Err(AppError::External(...)) }`（:538-540）。改造后 `upsert_webhook_contact` 对非真人返 `Ok(None)`，此时**不能再当错误**——应优雅短路返回（消息已在 :512 落库；gh_/群不可能 managed，无需触发流水线）。

- [ ] **Step 2: 改调用点处理 None（非真人短路）**

把 `src/webhooks.rs:538-540`：

```rust
    let Some(contact) = contact else {
        return Err(AppError::External("failed to create contact".to_string()));
    };
```

改为：

```rust
    let Some(contact) = contact else {
        // 非私聊真人（gh_ 公众号 / @chatroom 群）：消息已落库（:512），但不建运营池
        // 联系人、不触发 Agent 流水线（这类 wxid 本就不可能 managed）。
        return Ok(Json(serde_json::json!({ "ok": true, "skipped": "not_operatable_contact" })));
    };
```

- [ ] **Step 3: 改 upsert_webhook_contact 入口过滤 + 取值**

`src/webhooks.rs:1036-1037`，把开头的：

```rust
    let nickname = find_string(payload, &["nickName", "nickname", "fromNickName"]);
```

替换为（并在函数最开头加过滤）：

```rust
    // 非私聊真人（公众号/群）不进运营池——消息仍在调用点落库，只是不建 contact。
    if !is_operatable_person(wxid) {
        return Ok(None);
    }
    // 昵称/头像不再从 payload 取：真实 GeWe payload 发件人只有 wxid，
    // find_string 会递归命中 _mcp.nickName（账号自己昵称 "Demi"）。改从 roster 富化。
    let (roster_nickname, roster_avatar) = roster_identity_for(state, workspace_id, account_id, wxid)
        .await
        .unwrap_or((None, None));
```

- [ ] **Step 4: 改 $set 只在命中时写（不覆盖已有）**

`src/webhooks.rs:1092-1104` 的 update doc。把 `$set` 里的 `"nickname": &nickname` 改为条件构造——先建 `$set` 基础 doc 只含 updated_at，命中 roster 才插 nickname/avatar_url：

```rust
    let mut set_doc = doc! { "updated_at": DateTime::now() };
    if let Some(nick) = &roster_nickname {
        set_doc.insert("nickname", nick);
    }
    if let Some(av) = &roster_avatar {
        set_doc.insert("avatar_url", av);
    }
    state
        .db
        .contacts()
        .update_one(
            doc! {
                "workspace_id": workspace_id,
                "account_id": account_id,
                "wxid": wxid
            },
            doc! {
                "$set": set_doc,
                "$setOnInsert": {
                    "workspace_id": workspace_id,
                    "account_id": account_id,
                    "wxid": wxid,
                    "agent_status": "normal",
                    "created_at": DateTime::now()
                }
            },
            UpdateOptions::builder().upsert(true).build(),
        )
        .await?;
```

**注意**：`nickname` 局部变量已删（Step 3），确认函数内无其它地方引用旧 `nickname`（Grep 确认）。account_id 错配的 event 分支（:1041-1082）保持不变。

- [ ] **Step 5: 写建档行为测试**

webhook 建档依赖 DB + roster 快照，属集成范畴。lib 侧只能测纯判据（Task 1 已覆盖）。集成测试放 `tests/`（需 Docker，本地 skip，CI 跑）：

在 `tests/` 新建或复用现有 webhook 集成测试文件，加用例（若无现成 harness，标 `#[ignore]` 由 CI 跑）：
- gh_ 发件人 → contacts 不新增该 wxid（但 conversation_messages 有该条）。
- 真人 wxid + roster 命中 → contact.nickname == roster 昵称、avatar_url == roster 头像。
- 真人 wxid + roster 未命中 → contact 建成但 nickname/avatar_url 为 None。

若本任务无法在无 Docker 环境跑集成测试，实现者须在 report 注明「集成测试已写、标 #[ignore]、待 CI」，并保证 `cargo test --lib` + `cargo check --tests` 通过。

- [ ] **Step 6: 编译 + 基线**

Run: `cargo test --lib` → ≥ 350 / 0。
Run: `cargo check --tests` → 通过（集成测试编译无误）。

- [ ] **Step 7: Commit**

```bash
git add src/webhooks.rs tests/
git commit -m "fix(user-ops): webhook contact upsert filters non-person + enriches from roster (kills Demi/no-avatar)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 4: 存量清理 migration m029

**Files:**
- Create: `src/db/migrations/m029_cleanup_contact_identity.rs`
- Modify: `src/db/migrations/mod.rs`（:29-65 加 `mod m029_...;`；:77 起 MIGRATIONS 列表末尾加一条 Migration）

**Interfaces:**
- Consumes: `crate::webhooks::is_operatable_person`（Task 1，pub(crate)）、`read_roster_snapshot`。
- Produces: `pub(super) async fn run_step(db: &Database) -> AppResult<()>`。

- [ ] **Step 1: 亲验 mod.rs 注册格式 + 现有 migration id 命名**

Read `src/db/migrations/mod.rs:60-130`（看 m028 的 `mod` 行 + MIGRATIONS 里的 `Migration { id: "...", run: |db| Box::pin(m028_...::run_step(db)) }` 确切格式）。**migration id 字符串命名跟随现有惯例**（如 `2026_07_...`），以亲验的 m028 id 前缀为准。

- [ ] **Step 2: 亲验 migration 是否能拿到 workspace/account 查 roster**

`run_step(db: &Database)` 只有 db 句柄，没有 AppState/config。roster_snapshots 按 workspace_id+account_id 存。Read `roster_snapshots` 的读取方式——migration 里直接查集合：`db.roster_snapshots()`（若有 typed accessor）或 `db.collection("roster_snapshots")`。先 Grep `roster_snapshots` typed accessor 确认，再决定 migration 里怎么读（可能需遍历所有快照建 wxid→identity 映射，因为 migration 不限定单一 account）。

- [ ] **Step 3: 写 migration（含 3 步治理）**

```rust
//! m029：运营池真人化——清理存量 contacts 的身份污染。
//!
//! 修 3 个存量问题（webhook 建档 bug 遗留，2026-07-10 117 亲验）：
//! 1. 删非真人 normal 记录（gh_ 公众号 / @chatroom 群，本不该进运营池）。
//! 2. 剩余 contacts 按 roster 快照回填正确 nickname/avatar_url。
//! 3. nickname == "Demi"（账号自己昵称，find_string 递归误取）且 roster 未命中 → 置 None。
//!
//! 安全红线：只碰 nickname/avatar_url/删非真人 normal 行；绝不动 agent_status/
//! operation_state/画像/记忆；managed 一律保留（只清昵称不删）；无 APP_ENV 守卫
//! （无条件对所有环境存量生效）；幂等；不删 conversation_messages。

use futures::stream::TryStreamExt;
use mongodb::bson::{doc, Document};
use std::collections::HashMap;

use crate::db::Database;
use crate::error::AppResult;
use crate::webhooks::is_operatable_person;

pub(super) async fn run_step(db: &Database) -> AppResult<()> {
    // (1) 删非真人 normal 记录。managed 保留（见下 step 2 只清昵称）。
    let mut deleted = 0u64;
    let mut cursor = db
        .contacts()
        .find(doc! { "agent_status": "normal" }, None)
        .await?;
    let mut normal_wxids: Vec<String> = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        normal_wxids.push(c.wxid);
    }
    for wxid in &normal_wxids {
        if !is_operatable_person(wxid) {
            let r = db
                .contacts()
                .delete_many(doc! { "wxid": wxid, "agent_status": "normal" }, None)
                .await?;
            deleted += r.deleted_count;
        }
    }

    // (2) 建 wxid -> (nickname, avatar_url) 映射（遍历所有 roster 快照）。
    //     以 Step 2 亲验的 roster_snapshots 读取方式为准。
    let mut identity: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let mut snap_cursor = db.roster_snapshots().find(doc! {}, None).await?;
    while let Some(snap) = snap_cursor.try_next().await? {
        for f in snap.friends {
            identity.entry(f.wxid).or_insert((f.nickname, f.avatar_url));
        }
    }

    // (3) 遍历剩余 contacts：roster 命中→回填；nickname=="Demi" 且未命中→置 None。
    let mut enriched = 0u64;
    let mut demi_cleared = 0u64;
    let mut all_cursor = db.contacts().find(doc! {}, None).await?;
    while let Some(c) = all_cursor.try_next().await? {
        let wxid = c.wxid.clone();
        let mut set = Document::new();
        let mut unset = Document::new();
        match identity.get(&wxid) {
            Some((nick, avatar)) => {
                if let Some(n) = nick {
                    set.insert("nickname", n);
                }
                if let Some(a) = avatar {
                    set.insert("avatar_url", a);
                }
            }
            None => {
                // roster 未命中 + nickname 是账号自己昵称 "Demi" → 清掉（回落 wxid 显示）。
                if c.nickname.as_deref() == Some("Demi") {
                    unset.insert("nickname", "");
                    demi_cleared += 1;
                }
            }
        }
        if set.is_empty() && unset.is_empty() {
            continue;
        }
        let mut update = Document::new();
        if !set.is_empty() {
            update.insert("$set", set);
            enriched += 1;
        }
        if !unset.is_empty() {
            update.insert("$unset", unset);
        }
        db.contacts()
            .update_one(doc! { "wxid": &wxid, "account_id": &c.account_id, "workspace_id": &c.workspace_id }, update, None)
            .await?;
    }

    tracing::info!(
        migration_id = "2026_07_029_cleanup_contact_identity",
        deleted_non_person = deleted,
        enriched_from_roster = enriched,
        demi_cleared = demi_cleared,
        "cleaned up contact identity pollution"
    );
    Ok(())
}
```

**注意**：`db.roster_snapshots()` / `db.contacts()` typed accessor 与 `RosterSnapshot.friends` 字段以 Step 2 亲验为准；若无 typed accessor 用 `db.collection::<RosterSnapshot>("roster_snapshots")`。`$unset` 值用空串是 Mongo 惯例。

- [ ] **Step 4: 注册到 mod.rs**

`src/db/migrations/mod.rs` 在 m028 mod 声明后加 `mod m029_cleanup_contact_identity;`，在 MIGRATIONS 列表末尾加（id 前缀跟随亲验的现有惯例）：

```rust
    Migration {
        id: "2026_07_029_cleanup_contact_identity",
        run: |db| Box::pin(m029_cleanup_contact_identity::run_step(db)),
    },
```

- [ ] **Step 5: 写幂等 + 安全集成测试**

在 `tests/` 加 migration 测试（需 Docker，标 `#[ignore]` 由 CI 跑）：构造含 gh_/群 normal + 真人 + nickname=Demi 的 contacts + 一个 roster snapshot，跑 m029 后断言：
- gh_/群 normal 已删；conversation_messages 不受影响（如测试有插消息）。
- 真人 roster 命中 → nickname/avatar 回填正确。
- Demi 未命中 → nickname 变 None（$unset）。
- managed 记录（哪怕 gh_）未删。
- operation_state/agent_status 等字段零改动。
- 跑两次结果一致（幂等）。

- [ ] **Step 6: 编译 + 基线**

Run: `cargo test --lib` → ≥ 350 / 0。
Run: `cargo check --tests` → 通过。

- [ ] **Step 7: Commit**

```bash
git add src/db/migrations/m029_cleanup_contact_identity.rs src/db/migrations/mod.rs tests/
git commit -m "feat(user-ops): m029 migration cleans up stale contact identity (delete non-person, backfill roster, clear Demi)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 5: list_contacts 排序改最近来消息 + 读时兜底过滤

**Files:**
- Modify: `src/routes/contacts.rs:102-160`（`list_contacts`）

**Interfaces:**
- Consumes: `is_operatable_person`（Task 1）。
- 排序 doc 与过滤逻辑改动，不改端点签名/返回结构。

- [ ] **Step 1: 亲验当前 sort + filter**

Read `src/routes/contacts.rs:107-145`。当前 sort `doc! { "updated_at": -1 }`（:138）。filter 已含 workspace/account/status/q。

- [ ] **Step 2: 改排序为 last_inbound_at 优先**

`src/routes/contacts.rs:138`：

```rust
                .sort(doc! { "updated_at": -1 })
```

改为：

```rust
                // 最近主动来消息的人排最前（热线索优先）；last_inbound_at 为空的老记录
                // 用 updated_at 兜底。Mongo 多键 sort 按顺序。
                .sort(doc! { "last_inbound_at": -1, "updated_at": -1 })
```

- [ ] **Step 3: 读时过滤 gh_/群（双保险）**

在 `list_contacts` 拿到结果 Vec 后、映射成 ApiContact 前，用 `is_operatable_person` 过滤（防 migration 遗漏/历史残留）。找到构造返回 Vec 的位置（cursor 收集处），加：

```rust
    // 双保险：即使 migration 已清，读时再过滤一次非真人（历史残留/新 bug 兜底）。
    let contacts: Vec<Contact> = contacts
        .into_iter()
        .filter(|c| crate::webhooks::is_operatable_person(&c.wxid))
        .collect();
```

**注意**：变量名以亲验的 list_contacts 实际收集变量为准。

- [ ] **Step 4: 运行现有 contacts 测试 + 基线**

Run: `cargo test --lib contacts` （若有）+ `cargo test --lib` → ≥ 350 / 0。
Run: `cargo check --tests`。

- [ ] **Step 5: Commit**

```bash
git add src/routes/contacts.rs
git commit -m "feat(user-ops): list_contacts sorts by last_inbound_at + filters non-person (double safety)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 6: ApiContact 加 last_inbound_preview + list_contacts 填充

**Files:**
- Modify: `src/models.rs:3327-3371`（ApiContact struct 加字段）
- Modify: `src/models.rs:3373-3432`（From impl —— 填 None，因 From 无 DB）
- Modify: `src/routes/contacts.rs`（list_contacts 转换后 N+1 查最近 inbound 填充）

**Interfaces:**
- Produces: `ApiContact.last_inbound_preview: Option<String>`（camelCase 序列化 `lastInboundPreview`）。

- [ ] **Step 1: 亲验 ConversationMessage 查询方式 + messages() accessor**

Read `src/webhooks.rs:497-511`（ConversationMessage 字段：content/direction/contact_wxid/created_at）+ Grep `messages()` typed accessor。确认按 contact_wxid + direction=Inbound + sort created_at:-1 limit 1 能取最近入站。

- [ ] **Step 2: ApiContact 加字段**

`src/models.rs` ApiContact struct（:3370 `updated_at` 前）加：

```rust
    /// 最近一条入站消息原文截断（待启用档展示，帮运营判断是否开 Agent）。
    /// 非 LLM 摘要——normal 联系人不调 LLM，仅取 conversation_messages 最近 inbound content。
    /// From<Contact> 无 DB 访问故填 None，由 list_contacts 单独查询填充。
    pub last_inbound_preview: Option<String>,
```

- [ ] **Step 3: From impl 填 None**

`src/models.rs:3430`（`updated_at:` 行前）加：

```rust
            last_inbound_preview: None,
```

- [ ] **Step 4: 编译确认 From 完整**

Run: `cargo check --lib`
Expected: 通过（ApiContact 所有字段已填，无 E0063 缺字段）。

- [ ] **Step 5: list_contacts 填充 preview（N+1）**

在 `list_contacts` 把 `Vec<Contact>` 映射成 `Vec<ApiContact>` 处，改成 async 逐个查最近 inbound 填充。定义常量 + 截断纯函数（可单测）：

```rust
const INBOUND_PREVIEW_MAX_CHARS: usize = 30;

/// 按字符（非字节）截断，避免中文截半。超长加省略号。
pub(crate) fn truncate_preview(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(max_chars).collect();
    format!("{head}…")
}
```

填充逻辑（映射处）：

```rust
    let mut api_contacts: Vec<ApiContact> = Vec::with_capacity(contacts.len());
    for c in contacts {
        let wxid = c.wxid.clone();
        let mut api = ApiContact::from(c);
        // 最近一条入站原文（原文截断，非 LLM 摘要）。
        if let Ok(Some(msg)) = state
            .db
            .messages()
            .find_one(
                doc! { "workspace_id": &admin.current_workspace, "contact_wxid": &wxid, "direction": "inbound" },
                mongodb::options::FindOneOptions::builder().sort(doc! { "created_at": -1 }).build(),
            )
            .await
        {
            api.last_inbound_preview = Some(truncate_preview(&msg.content, INBOUND_PREVIEW_MAX_CHARS));
        }
        api_contacts.push(api);
    }
```

**注意**：`direction` 在 DB 里的存储值以亲验为准（ConversationMessage 的 MessageDirection serde —— 确认是 "inbound" 小写还是别的）。字段名 `messages()` / `content` 以亲验为准。

- [ ] **Step 6: 写截断纯函数测试**

```rust
#[test]
fn truncate_preview_keeps_short_and_cuts_long() {
    assert_eq!(truncate_preview("你好", 30), "你好");
    assert_eq!(truncate_preview("  空白裁剪  ", 30), "空白裁剪");
    let long = "一二三四五六七八九十".repeat(5); // 50 chars
    let out = truncate_preview(&long, 30);
    assert_eq!(out.chars().count(), 31); // 30 + 省略号
    assert!(out.ends_with('…'));
}
```

- [ ] **Step 7: 运行 + 基线**

Run: `cargo test --lib truncate_preview` → passed。
Run: `cargo test --lib` → ≥ 350 / 0；`cargo check --tests`。

- [ ] **Step 8: Commit**

```bash
git add src/models.rs src/routes/contacts.rs
git commit -m "feat(user-ops): add last_inbound_preview to ApiContact (raw truncated, no LLM)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 7: 前端 —— Contact 类型加 lastInboundPreview + 超时派生纯函数

**Files:**
- Modify: `frontend/src/types/index.ts:86-130`（Contact type 加字段）
- Create: `frontend/src/features/user-ops/poolHelpers.ts`（超时未跟进派生纯函数）
- Create: `frontend/src/__tests__/features/user-ops/poolHelpers.test.ts`

**Interfaces:**
- Produces: `Contact.lastInboundPreview?: string`；`overdueHours(contact, now): number | null`（来了没回且超时→小时数，否则 null）。

- [ ] **Step 1: Contact 类型加字段**

`frontend/src/types/index.ts:129`（`lastMessageAt` 后）加：

```ts
  /** 最近一条入站消息原文截断（待启用档展示，后端 list_contacts 填充）。 */
  lastInboundPreview?: string | null;
```

- [ ] **Step 2: 写超时派生测试**

`frontend/src/__tests__/features/user-ops/poolHelpers.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { overdueHours } from "../../../features/user-ops/poolHelpers";

const now = new Date("2026-07-11T12:00:00Z").getTime();

describe("overdueHours", () => {
  it("来了消息之后没回、且超阈值 → 返回小时数", () => {
    const c = { lastInboundAt: "2026-07-10T12:00:00Z", lastOutboundAt: "2026-07-09T00:00:00Z" } as any;
    expect(overdueHours(c, now)).toBe(24);
  });
  it("回过了（outbound 晚于 inbound）→ null", () => {
    const c = { lastInboundAt: "2026-07-10T12:00:00Z", lastOutboundAt: "2026-07-11T00:00:00Z" } as any;
    expect(overdueHours(c, now)).toBeNull();
  });
  it("来了消息但未超阈值 → null", () => {
    const c = { lastInboundAt: "2026-07-11T11:00:00Z", lastOutboundAt: null } as any;
    expect(overdueHours(c, now)).toBeNull(); // 仅 1h < 24h
  });
  it("从没来过消息 → null", () => {
    const c = { lastInboundAt: undefined, lastOutboundAt: undefined } as any;
    expect(overdueHours(c, now)).toBeNull();
  });
});
```

- [ ] **Step 3: 运行确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/poolHelpers.test.ts`
Expected: FAIL（模块不存在）。

- [ ] **Step 4: 实现纯函数**

`frontend/src/features/user-ops/poolHelpers.ts`：

```ts
import type { Contact } from "../../types";

const OVERDUE_THRESHOLD_HOURS = 24;

/**
 * 超时未跟进派生（纯函数，不需已读系统）：客户来了消息（lastInboundAt）之后
 * 没有出站回复（lastOutboundAt 早于 lastInboundAt 或缺失）、且距今超过阈值 →
 * 返回已过小时数（向下取整）；否则 null。
 */
export function overdueHours(contact: Contact, nowMs: number): number | null {
  if (!contact.lastInboundAt) return null;
  const inbound = new Date(contact.lastInboundAt).getTime();
  if (Number.isNaN(inbound)) return null;
  const outbound = contact.lastOutboundAt ? new Date(contact.lastOutboundAt).getTime() : 0;
  // 已回复（出站不早于入站）→ 不算超时。
  if (outbound >= inbound) return null;
  const hours = Math.floor((nowMs - inbound) / 3_600_000);
  return hours >= OVERDUE_THRESHOLD_HOURS ? hours : null;
}
```

- [ ] **Step 5: 运行确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/poolHelpers.test.ts`
Expected: 4 passed。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/types/index.ts frontend/src/features/user-ops/poolHelpers.ts frontend/src/__tests__/features/user-ops/poolHelpers.test.ts
git commit -m "feat(user-ops): add lastInboundPreview type + overdueHours derivation (time-based, no read-state)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 8: 前端 ContactsView 重构成漏斗工作台

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx:440-521`（ContactsView 组件）
- Modify: `frontend/src/features/user-ops/index.tsx`（传 taxonomies 给 ContactsView 用于标签/阶段 labelFor；传 batch 相关）
- Modify: `frontend/src/styles.css`（contact 行相关 class：头像 28→40px，新增阶段徽章/标签 chip/超时标记/启用按钮样式）

**Interfaces:**
- Consumes: `overdueHours`（Task 7）、`labelFor(taxonomies, kind, value)`（profileStore.ts:23）、`Contact.lastInboundPreview/manualTags/confirmedTags/operationState/lastInboundAt/lastOutboundAt`、`batchEnable`（userOpsStore.ts:494）+ RosterView 多选范式（RosterView.tsx:114-143）。

- [ ] **Step 1: 亲验现有 ContactsView props + index.tsx 传参 + taxonomies 来源**

Read `frontend/src/features/user-ops/legacy.tsx:440-521`（当前 props）+ `index.tsx:264-278`（ContactsView 调用点）+ Grep `taxonomies` 在 index.tsx / userOpsStore 里怎么拿（profileStore？）。确认能拿到 taxonomies 传进 ContactsView。相对时间格式化函数是否已有（Grep `formatRelative\|相对时间\|ago`）——有则复用，无则本任务加一个纯函数（并补单测）。

- [ ] **Step 2: 顶部定位说明 + tab 副标题**

`legacy.tsx:466-483` 的 `panelHead`。标题「运营池」下方加定位副标题 + 区别通讯录小字（muted 灰）；`segmented`（:472-482）下方加当前 tab 的人话副标题（待启用/Agent 不同文案）。文案：
- 定位：`主动来找过你的人 → 挑价值高的交 AI 接管`
- 区别：`区别于通讯录（全部好友）：这里只收主动来消息的私聊真人`
- 待启用 tab 副标题：`待你评估是否开 AI 自动回复`
- Agent tab 副标题：`AI 正在自动运营 · 显示当前运营阶段`

- [ ] **Step 3: 分档差异化行 + 标签 + 超时 + 启用按钮**

重写 `legacy.tsx:497-519` 的 `contactList` 渲染。按 `contact.agentStatus` 分档：
- 待启用（normal）：40px 头像 + `remark||nickname||wxid` + `lastInboundPreview`（灰摘要）+ 相对时间 + 标签 chips（`labelFor` 转中文）+ `overdueHours` 超时标记（暖色）+ 行尾「启用 Agent」按钮（调下 Step 5 的单人启用或选中）。
- Agent（managed）：40px 头像 + 名 + `operationState` 阶段徽章（青色，`labelFor(taxonomies, "customer_stage", operationState)` 转中文，unknown 回落原值）+ 标签 chips + 相对时间。

标签 chip 渲染：合并 `manualTags` + `confirmedTags.map(t=>t.value)`，每个经 `labelFor` 转中文；无标签不显示该区。

- [ ] **Step 4: 批量选择 + 批量启用（复用 RosterView 范式）**

在 ContactsView 加 `selectedWxids: Set<string>` state（仅待启用档启用勾选）。参考 `RosterView.tsx:114-143`：勾选框 toggle、顶部出现「批量启用 Agent（N）」按钮、调 `batchEnable({accountId, candidates, sharedNote, playbookId})`。candidates 从选中 contacts 映射 `{wxid, nickname, remark, avatarUrl, sex}`。启用后清空选中 + 刷新列表/count（复用 index.tsx 现有 refreshContacts + loadContactCounts）。

**注意**：batchEnable 需 sharedNote（RosterView 有输入框）。运营池批量启用可用默认 note（如「从运营池批量启用」）或加个简单输入——实现时按最简：默认 note，避免过度设计。

- [ ] **Step 5: CSS（styles.css）**

改 `.contactAvatar, .contactAvatarFallback`（:1618-1635）宽高 28→40px、字号相应调大。新增：
- `.stageBadge`：青色 pill（bg rgba(15,118,110,.09) color #0f766e），阶段徽章。
- `.tagChip`：中性小 chip（灰底），标签。
- `.overdueTag`：暖色（如 #b45309），超时标记，非 danger 红。
- `.enableBtn`：AI 蓝描边小按钮。
- 调 `.contact` 行结构容纳多段（保持 row-h 或适度加高，flex 布局）。
遵守设计系统色板（蓝仅主操作/青仅 AI 状态）。

- [ ] **Step 6: 前端 build + 现有测试**

Run: `cd frontend && npm run build` → 无 TS 错误/无悬空引用。
Run: `cd frontend && npx vitest run` → 现有全绿（下 Task 9 补新契约测试）。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/user-ops/legacy.tsx frontend/src/features/user-ops/index.tsx frontend/src/styles.css
git commit -m "feat(user-ops): ContactsView funnel workbench (positioning copy, per-tier rows, tags, overdue, batch enable)

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 9: 前端 ContactsView 契约测试

**Files:**
- Create: `frontend/src/__tests__/features/user-ops/contactsViewFunnel.test.tsx`
- Modify: 若 `frontend/src/__tests__/features/user-ops/userOps.test.tsx` 的 mock store 缺新字段（如 contactCounts、taxonomies），补齐防崩。

**Interfaces:**
- Consumes: 真实 `ContactsView`（具名导出，legacy.tsx）。

- [ ] **Step 1: 亲验 ContactsView 当前 props 类型 + 现有测试 mock**

Read `legacy.tsx` 里 Task 8 改后的 ContactsView props 签名（以实际实现为准）+ `frontend/src/__tests__/features/user-ops/userOps.test.tsx` 的 createMockStore。若 index.tsx mock 缺 taxonomies/新字段导致挂载崩，先补 mock。

- [ ] **Step 2: 写契约测试**

`frontend/src/__tests__/features/user-ops/contactsViewFunnel.test.tsx`（props 以 Step 1 亲验为准，下面是骨架）：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ContactsView } from "../../../features/user-ops/legacy";

const baseProps = {
  contactTab: "normal" as const,
  contacts: [],
  managedCount: 3,
  normalCount: 12,
  totalCount: 15,
  query: "",
  selected: null,
  onContactTab: vi.fn(),
  onLoadAll: vi.fn(),
  onOpenContact: vi.fn(),
  onQuery: vi.fn(),
  // Task 8 新增的 props（taxonomies / batch 相关）以实际实现补齐
};

describe("ContactsView 漏斗工作台", () => {
  it("顶部有定位说明 + 区别通讯录小字", () => {
    render(<ContactsView {...baseProps} />);
    expect(screen.getByText(/主动来找过你的人/)).toBeInTheDocument();
    expect(screen.getByText(/区别于通讯录/)).toBeInTheDocument();
  });

  it("待启用档行显示消息摘要 + 启用按钮", () => {
    const contacts = [{ id: "1", wxid: "wxid_a", nickname: "小明", agentStatus: "normal", lastInboundPreview: "想问下课程怎么收费", tags: [], operationPolicy: {}, profileAttributes: {}, updatedAt: "2026-07-11T00:00:00Z" }] as any;
    render(<ContactsView {...baseProps} contacts={contacts} />);
    expect(screen.getByText(/想问下课程怎么收费/)).toBeInTheDocument();
    expect(screen.getByText(/启用 Agent/)).toBeInTheDocument();
  });

  it("Agent 档行显示运营阶段徽章", () => {
    const contacts = [{ id: "2", wxid: "wxid_b", nickname: "张总", agentStatus: "managed", operationState: "new_contact", tags: [], operationPolicy: {}, profileAttributes: {}, updatedAt: "2026-07-11T00:00:00Z" }] as any;
    render(<ContactsView {...baseProps} contactTab="managed" contacts={contacts} />);
    // 阶段值经 labelFor 转中文；无字典时回落原值 new_contact。
    expect(screen.getByText(/new_contact|初次接触|新联系人/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: 运行新测试**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/contactsViewFunnel.test.tsx`
Expected: 3 passed（若文案匹配不到，按实际渲染微调断言，非改业务）。

- [ ] **Step 4: 全量前端门**

Run: `cd frontend && npx vitest run` → 全绿。
Run: `cd frontend && npm run build` → 成功。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/__tests__/features/user-ops/
git commit -m "test(user-ops): ContactsView funnel workbench contract tests

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

### Task 10: 全量验证门 + 文案门

**Files:** 无（仅跑门）

- [ ] **Step 1: 后端基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 2: CI 编译门**

Run: `cargo check --tests`
Expected: 通过（集成测试全编译）。

- [ ] **Step 3: 前端门**

Run: `cd frontend && npm run build && npx vitest run`
Expected: build 成功 + 全绿。

- [ ] **Step 4: 无人工接管文案门**

Run: `scripts/check-no-human-takeover.sh`（或 `.ps1`）
Expected: exit 0（新增文案「待启用/Agent/运营池/启用 Agent」均不含禁用词）。

- [ ] **Step 5: 最终 commit（若门修了任何东西）**

若前 4 步无改动则跳过。否则：

```bash
git add -A
git commit -m "chore(user-ops): pass all baseline/frontend/copy gates

$(printf '\360\237\244\226 Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>')"
```

---

## 任务依赖

- Task 1（is_operatable_person）→ 被 3/4/5 复用，最先做。
- Task 2（roster helper）→ 被 3 用。
- Task 3（webhook 接线）依赖 1+2。
- Task 4（migration）依赖 1。
- Task 5/6（list_contacts）依赖 1。
- Task 7（前端类型+纯函数）独立，可与后端并行。
- Task 8（ContactsView）依赖 6（lastInboundPreview 字段）+ 7（overdueHours）。
- Task 9（前端测试）依赖 8。
- Task 10（全量门）最后。

顺序执行：1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10。
