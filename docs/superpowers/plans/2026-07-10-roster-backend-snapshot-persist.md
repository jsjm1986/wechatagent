# 通讯录后端持久化快照 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 通讯录进频道读 DB 快照秒回、快照 > 24h 后台静默自刷、MCP 传输失败永远兜底旧快照，根治「每次进通讯录长时间加载/转圈」。

**Architecture:** 新增 `roster_snapshots` 集合（每 workspace+account 一条，覆盖写）。`roster_endpoint` 改快照优先：有快照秒回（stale 则 fire-and-forget 后台自刷），无快照同步拉（失败显同步中 + 后台重试）。后台任务是独立的更健壮重试循环（连 `AppError::Http` 解码失败也退避重试），区别于同步路径的既有 3 次重试。

**Tech Stack:** Rust (Axum) + serde + MongoDB · React 19 + TS + Zustand + Vitest

设计文档：`docs/superpowers/specs/2026-07-10-roster-backend-snapshot-persist-design.md`

## Global Constraints

- 基线门：`cargo test --lib` ≥ 350 passed / 0 failed；前端 `tsc --noEmit` 0 error + `vitest run` 全绿。
- 测试只增不删：`roster_parse_tests`（src/mcp.rs:696）、`roster_outcome_tests`（src/mcp.rs:854）、前端 roster.test.tsx（11 用例）是回归守卫，保留不动，只新增。
- 快照键维度 = `current_workspace + account_id`（与 `roster_endpoint` 查 contacts 同维，src/routes/contacts.rs:383）。
- 有快照时任何 MCP 失败都返回旧快照（syncing:false），绝不让页面转圈/空白。
- 后台自刷是 best-effort fire-and-forget（复用 src/agent/decision_taxonomy.rs:135 范式）：失败仅 `tracing::warn`，不加锁/去重，重复 spawn 覆盖写同一条无害。
- 无人工接管红线：本改动不涉及 agent 决策/状态词，无红线词风险；但提交前 src/ 新增行不得含 `人工/接管/takeover/hand-off` 等词（check-no-human-takeover 门）。
- 常量：`ROSTER_SNAPSHOT_STALE_HOURS = 24`、`ROSTER_REFRESH_MAX_RETRIES = 5`、退避 `3→6→12→24→48s`。

---

### Task 1: RosterSnapshot 模型 + RosterFriend 补 Deserialize

**Files:**
- Modify: `src/mcp.rs:3`（use serde 加 Deserialize）、`src/mcp.rs:421`（RosterFriend derive）
- Modify: `src/models.rs`（新增 RosterSnapshot struct，接在 McpCallLog 后，约 :960）
- Test: `src/models.rs`（新增 roster_snapshot_tests 模块，文件末尾）

**Interfaces:**
- Produces: `pub struct RosterSnapshot { id, workspace_id, account_id, friends: Vec<RosterFriend>, total: i64, fetched_at: DateTime }`（Serialize+Deserialize）；`RosterFriend` 现额外实现 `Deserialize`。

- [ ] **Step 1: RosterFriend 加 Deserialize**

`src/mcp.rs:3` 把：
```rust
use serde::Serialize;
```
改为：
```rust
use serde::{Deserialize, Serialize};
```

`src/mcp.rs:421` 把：
```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct RosterFriend {
```
改为：
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterFriend {
```

- [ ] **Step 2: 写失败测试**（src/models.rs 文件末尾新增模块）

```rust
#[cfg(test)]
mod roster_snapshot_tests {
    use super::RosterSnapshot;
    use crate::mcp::RosterFriend;
    use mongodb::bson::{self, DateTime};

    #[test]
    fn roster_snapshot_bson_round_trip_preserves_fields() {
        // 快照写盘再读回：sex / is_non_human / 头像等字段一致（验 RosterFriend 的 Deserialize）。
        let snap = RosterSnapshot {
            id: None,
            workspace_id: "ws1".into(),
            account_id: "acc1".into(),
            friends: vec![
                RosterFriend {
                    wxid: "wxid_a".into(),
                    nickname: Some("小明".into()),
                    remark: Some("客户A".into()),
                    avatar_url: Some("http://img/a".into()),
                    sex: Some(1),
                    is_non_human: false,
                },
                RosterFriend {
                    wxid: "fmessage".into(),
                    nickname: None,
                    remark: None,
                    avatar_url: None,
                    sex: None,
                    is_non_human: true,
                },
            ],
            total: 2,
            fetched_at: DateTime::from_millis(1_700_000_000_000),
        };
        let doc = bson::to_document(&snap).expect("序列化");
        let back: RosterSnapshot = bson::from_document(doc).expect("反序列化");
        assert_eq!(back.friends.len(), 2);
        assert_eq!(back.friends[0].sex, Some(1));
        assert_eq!(back.friends[0].avatar_url.as_deref(), Some("http://img/a"));
        assert!(!back.friends[0].is_non_human);
        assert!(back.friends[1].is_non_human);
        assert_eq!(back.total, 2);
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test --lib roster_snapshot_bson_round_trip`
Expected: 编译失败（`RosterSnapshot` 未定义）。

- [ ] **Step 4: 加 RosterSnapshot struct**（src/models.rs，接在 McpCallLog struct 后，约 :960）

```rust
/// 通讯录全量快照：每 (workspace_id, account_id) 恒一条（覆盖写）。进频道读此快照
/// 秒回，避免每次重打 MCP contacts_fetch_full（4832 条大 body、传输慢且偶发
/// `error decoding response body`）。快照龄 > 24h 由 roster_endpoint 触发后台自刷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterSnapshot {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: String,
    pub friends: Vec<crate::mcp::RosterFriend>,
    pub total: i64,
    pub fetched_at: DateTime,
}
```

（`ObjectId` / `DateTime` / `Serialize` / `Deserialize` 已在 src/models.rs:3-4 导入，无需新增 use。）

- [ ] **Step 5: 跑测试确认通过 + lib 全量**

Run: `cargo test --lib roster_snapshot && cargo test --lib`
Expected: round-trip PASS；lib 全量 ≥ 350 passed / 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/mcp.rs src/models.rs
git commit -m "feat(roster): RosterSnapshot 模型 + RosterFriend 补 Deserialize"
```

---

### Task 2: db accessor + 唯一索引

**Files:**
- Modify: `src/db/mod.rs`（新增 roster_snapshots() accessor，接在 campaign_sends 后，约 :393）
- Modify: `src/db/indexes.rs`（新增 (workspace_id, account_id) unique 索引，接在主 ensure_all 的 `Ok(())` 前，约 :1717）

**Interfaces:**
- Consumes: Task 1 的 `RosterSnapshot`。
- Produces: `db.roster_snapshots() -> Collection<RosterSnapshot>`；`roster_snapshots` 集合有 `(workspace_id, account_id)` unique 索引。

- [ ] **Step 1: 加 accessor**（src/db/mod.rs，接在 `campaign_sends` accessor 后，约 :393-394）

```rust
    /// 通讯录全量快照（每 workspace+account 一条，覆盖写）。进频道读此快照秒回。
    pub fn roster_snapshots(&self) -> Collection<crate::models::RosterSnapshot> {
        self.db.collection("roster_snapshots")
    }
```

- [ ] **Step 2: 加唯一索引**（src/db/indexes.rs，接在主 `ensure_all` 函数结尾 `Ok(())`（约 :1717）前）

```rust
    db.roster_snapshots()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "account_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            None,
        )
        .await?;
```

- [ ] **Step 3: 编译确认**

Run: `cargo check --lib`
Expected: 编译通过（accessor 返回类型 + 索引 builder 正确）。

- [ ] **Step 4: 提交**

```bash
git add src/db/mod.rs src/db/indexes.rs
git commit -m "feat(roster): roster_snapshots 集合 accessor + (workspace,account) 唯一索引"
```

---

### Task 3: 纯函数（过期判定 + 退避）+ 常量

**Files:**
- Modify: `src/mcp.rs`（新增两个纯函数 + 常量，放在 `fetch_roster_for_account` 前，约 :606）
- Test: `src/mcp.rs`（新增 roster_snapshot_policy_tests 模块，文件末尾）

**Interfaces:**
- Produces: `const ROSTER_SNAPSHOT_STALE_HOURS: i64 = 24;`、`const ROSTER_REFRESH_MAX_RETRIES: usize = 5;`、`fn snapshot_is_stale(fetched_at: DateTime, now: DateTime) -> bool`、`fn roster_refresh_backoff_secs(attempt: usize) -> u64`。

- [ ] **Step 1: 写失败测试**（src/mcp.rs 文件末尾新增模块）

```rust
#[cfg(test)]
mod roster_snapshot_policy_tests {
    use super::{roster_refresh_backoff_secs, snapshot_is_stale};
    use mongodb::bson::DateTime;

    #[test]
    fn stale_after_24h() {
        let base = 1_700_000_000_000i64; // ms
        let now = DateTime::from_millis(base);
        // 23h 前 → 未过期。
        assert!(!snapshot_is_stale(DateTime::from_millis(base - 23 * 3600_000), now));
        // 25h 前 → 过期。
        assert!(snapshot_is_stale(DateTime::from_millis(base - 25 * 3600_000), now));
    }

    #[test]
    fn backoff_is_exponential_3_to_48() {
        assert_eq!(roster_refresh_backoff_secs(0), 3);
        assert_eq!(roster_refresh_backoff_secs(1), 6);
        assert_eq!(roster_refresh_backoff_secs(2), 12);
        assert_eq!(roster_refresh_backoff_secs(3), 24);
        assert_eq!(roster_refresh_backoff_secs(4), 48);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib roster_snapshot_policy`
Expected: 编译失败（两函数未定义）。

- [ ] **Step 3: 加常量 + 纯函数**（src/mcp.rs，放在 `pub async fn fetch_roster_for_account`（:607）前）

```rust
/// 快照过期阈值：龄 > 24h 触发后台自刷（进频道仍先秒回旧快照）。
const ROSTER_SNAPSHOT_STALE_HOURS: i64 = 24;
/// 后台自刷/首次重试的最大尝试次数（连 http 解码失败也计入）。
const ROSTER_REFRESH_MAX_RETRIES: usize = 5;

/// 快照是否过期（龄 > ROSTER_SNAPSHOT_STALE_HOURS）。
fn snapshot_is_stale(fetched_at: DateTime, now: DateTime) -> bool {
    now.timestamp_millis() - fetched_at.timestamp_millis()
        > ROSTER_SNAPSHOT_STALE_HOURS * 3600_000
}

/// 后台重试退避秒数：3 * 2^attempt（3/6/12/24/48…）。
fn roster_refresh_backoff_secs(attempt: usize) -> u64 {
    3u64 * 2u64.pow(attempt as u32)
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib roster_snapshot_policy`
Expected: 两用例 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/mcp.rs
git commit -m "feat(roster): 快照过期判定 + 后台重试退避纯函数 + 常量"
```

---

### Task 4: 快照读写 helper + 后台自刷任务

**Files:**
- Modify: `src/mcp.rs`（新增 `read_roster_snapshot` / `write_roster_snapshot` / `spawn_roster_refresh`，放在 `fetch_roster_for_account` 后，约 :651）

**Interfaces:**
- Consumes: Task 1 `RosterSnapshot`、Task 2 `db.roster_snapshots()`、Task 3 常量/退避函数、既有 `fetch_roster_for_account`（:607）。
- Produces:
  - `pub async fn read_roster_snapshot(state: &AppState, workspace_id: &str, account_id: &str) -> AppResult<Option<RosterSnapshot>>`
  - `pub async fn write_roster_snapshot(state: &AppState, workspace_id: &str, account_id: &str, friends: &[RosterFriend]) -> AppResult<()>`（replace_one upsert）
  - `pub fn spawn_roster_refresh(state: AppState, workspace_id: String, account_id: String)`（fire-and-forget 后台重试直到就绪或达上限）

- [ ] **Step 1: 加读快照 helper**（src/mcp.rs，`fetch_roster_for_account` 后，约 :651）

```rust
use crate::models::RosterSnapshot;

/// 读某账号的 roster 快照（无则 None）。
pub async fn read_roster_snapshot(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
) -> AppResult<Option<RosterSnapshot>> {
    let snap = state
        .db
        .roster_snapshots()
        .find_one(
            doc! { "workspace_id": workspace_id, "account_id": account_id },
            None,
        )
        .await?;
    Ok(snap)
}
```

（注：`use crate::models::RosterSnapshot;` 若 Task 1 已在别处引入则合并，不重复导入——实现时置于文件已有 `use crate::{...}` 块或就近函数上方，编译器报重复则删其一。）

- [ ] **Step 2: 加写快照 helper**（紧接 Step 1）

```rust
/// 覆盖写某账号的 roster 快照（replace_one upsert，每账号恒一条）。
pub async fn write_roster_snapshot(
    state: &AppState,
    workspace_id: &str,
    account_id: &str,
    friends: &[RosterFriend],
) -> AppResult<()> {
    let snap = RosterSnapshot {
        id: None,
        workspace_id: workspace_id.to_string(),
        account_id: account_id.to_string(),
        friends: friends.to_vec(),
        total: friends.len() as i64,
        fetched_at: DateTime::now(),
    };
    let options = mongodb::options::ReplaceOptions::builder()
        .upsert(true)
        .build();
    state
        .db
        .roster_snapshots()
        .replace_one(
            doc! { "workspace_id": workspace_id, "account_id": account_id },
            &snap,
            options,
        )
        .await?;
    Ok(())
}
```

- [ ] **Step 3: 加后台自刷任务**（紧接 Step 2）

```rust
/// 后台静默自刷某账号的 roster 快照：fire-and-forget，不阻塞请求。最多
/// `ROSTER_REFRESH_MAX_RETRIES` 次调 `fetch_roster_for_account`，**任何错误
/// （含 AppError::Http 解码失败）都退避重试**（区别于同步路径 Err(other) 直接上抛）。
/// 拿到就绪结果即覆盖写快照；用尽仍未就绪仅 warn（下次进频道再触发）。best-effort，
/// 不加锁/去重，重复 spawn 覆盖写同一条无害。
pub fn spawn_roster_refresh(state: AppState, workspace_id: String, account_id: String) {
    tokio::spawn(async move {
        for attempt in 0..ROSTER_REFRESH_MAX_RETRIES {
            match fetch_roster_for_account(&state, &account_id).await {
                Ok(outcome) if !outcome.syncing => {
                    if let Err(err) =
                        write_roster_snapshot(&state, &workspace_id, &account_id, &outcome.friends)
                            .await
                    {
                        tracing::warn!(?err, account_id = %account_id, "roster 快照写入失败");
                    }
                    return;
                }
                // 就绪但仍 syncing（空 cache）或出错：退避后重试。
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(?err, account_id = %account_id, attempt, "roster 后台刷新单次失败,退避重试");
                }
            }
            if attempt + 1 < ROSTER_REFRESH_MAX_RETRIES {
                tokio::time::sleep(std::time::Duration::from_secs(
                    roster_refresh_backoff_secs(attempt),
                ))
                .await;
            }
        }
        tracing::warn!(account_id = %account_id, "roster 后台刷新用尽重试仍未就绪,放弃(下次进频道再触发)");
    });
}
```

- [ ] **Step 4: 编译确认**

Run: `cargo check --lib`
Expected: 编译通过（ReplaceOptions/find_one/replace_one 类型正确；spawn 内 owned state 满足 'static）。

- [ ] **Step 5: 提交**

```bash
git add src/mcp.rs
git commit -m "feat(roster): 快照读写 helper + 后台自刷任务(解码失败也退避重试)"
```

---

### Task 5: roster_endpoint 重写（快照优先 + force）

**Files:**
- Modify: `src/routes/contacts.rs:361-417`（RosterQuery 加 force + roster_endpoint 重写）

**Interfaces:**
- Consumes: Task 4 的 `read_roster_snapshot` / `write_roster_snapshot` / `spawn_roster_refresh`、Task 3 的 `snapshot_is_stale`、既有 `fetch_roster_for_account`。
- Produces: `GET /api/contacts/roster?accountId=X&force=true` — force 强制拉新覆盖写；非 force 快照优先。返回体不变 `{items, total, syncing}`。

**注：** `snapshot_is_stale` 现是 `src/mcp.rs` 私有 `fn`。本任务需在 contacts.rs 调用它 → 实现时把它改 `pub(crate) fn`（连同其单测保持不动）。read/write/spawn 已是 `pub`。

- [ ] **Step 1: RosterQuery 加 force 字段**（src/routes/contacts.rs:361-365）

```rust
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RosterQuery {
    pub account_id: String,
    #[serde(default)]
    pub force: bool,
}
```

- [ ] **Step 2: 抽 agent_status 拼装为闭包 + 重写 roster_endpoint**（src/routes/contacts.rs:367-417 整体替换）

```rust
pub(super) async fn roster_endpoint(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Query(query): Query<RosterQuery>,
) -> AppResult<Json<Value>> {
    validate_account(&state, &admin.current_workspace, &query.account_id).await?;
    let ws = &admin.current_workspace;
    let acc = &query.account_id;

    // 本地已入库联系人：wxid -> agent_status。拿到 friends（快照或实时）后统一拼装。
    let build_items = |friends: Vec<mcp::RosterFriend>,
                       status_by_wxid: &std::collections::HashMap<String, String>|
     -> Vec<Value> {
        friends
            .into_iter()
            .map(|f| {
                let agent_status = status_by_wxid
                    .get(&f.wxid)
                    .cloned()
                    .unwrap_or_else(|| "not_imported".to_string());
                json!({
                    "wxid": f.wxid,
                    "nickname": f.nickname,
                    "remark": f.remark,
                    "avatarUrl": f.avatar_url,
                    "sex": f.sex,
                    "isNonHuman": f.is_non_human,
                    "agentStatus": agent_status,
                })
            })
            .collect()
    };

    // 拉当前 workspace+account 的 contacts 状态映射。
    let load_status_map = || async {
        let mut cursor = state
            .db
            .contacts()
            .find(doc! { "workspace_id": ws, "account_id": acc }, None)
            .await?;
        let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        while let Some(c) = cursor.try_next().await? {
            let status = match c.agent_status {
                crate::models::AgentStatus::Managed => "managed",
                _ => "normal",
            };
            map.insert(c.wxid, status.to_string());
        }
        Ok::<_, AppError>(map)
    };

    // 决定本次返回的 friends + syncing。
    let (friends, syncing): (Vec<mcp::RosterFriend>, bool) = if query.force {
        // 强制刷新：同步拉；就绪则覆盖写快照并用新数据；失败则回退旧快照（有则用，syncing:false）。
        match mcp::fetch_roster_for_account(&state, acc).await {
            Ok(outcome) if !outcome.syncing => {
                mcp::write_roster_snapshot(&state, ws, acc, &outcome.friends).await?;
                (outcome.friends, false)
            }
            _ => match mcp::read_roster_snapshot(&state, ws, acc).await? {
                Some(snap) => (snap.friends, false),
                None => {
                    mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
                    (Vec::new(), true)
                }
            },
        }
    } else {
        // 非 force：快照优先。
        match mcp::read_roster_snapshot(&state, ws, acc).await? {
            Some(snap) => {
                if mcp::snapshot_is_stale(snap.fetched_at, mongodb::bson::DateTime::now()) {
                    mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
                }
                (snap.friends, false)
            }
            None => match mcp::fetch_roster_for_account(&state, acc).await {
                Ok(outcome) if !outcome.syncing => {
                    mcp::write_roster_snapshot(&state, ws, acc, &outcome.friends).await?;
                    (outcome.friends, false)
                }
                _ => {
                    mcp::spawn_roster_refresh(state.clone(), ws.clone(), acc.clone());
                    (Vec::new(), true)
                }
            },
        }
    };

    let status_by_wxid = load_status_map().await?;
    let items = build_items(friends, &status_by_wxid);
    let total = items.len();
    Ok(Json(json!({ "items": items, "total": total, "syncing": syncing })))
}
```

- [ ] **Step 3: 把 snapshot_is_stale 改 pub(crate)**（src/mcp.rs Task 3 定义处）

```rust
pub(crate) fn snapshot_is_stale(fetched_at: DateTime, now: DateTime) -> bool {
```

- [ ] **Step 4: 编译确认（含 imports）**

Run: `cargo check --lib`
Expected: 编译通过。若报 `AppError` / `mcp` / `try_next` 未导入：contacts.rs 顶部已有 `use crate::mcp;` 与 `AppError`（既有 roster_endpoint 已用 `mcp::fetch_roster_for_account` 与 `?`），沿用即可；`try_next` 来自 `futures::TryStreamExt`（文件既有 cursor.try_next 已导入）。

- [ ] **Step 5: lib 全量基线**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

- [ ] **Step 6: 提交**

```bash
git add src/mcp.rs src/routes/contacts.rs
git commit -m "feat(roster): roster_endpoint 快照优先(秒回+stale 后台自刷+force 强拉+失败兜底旧快照)"
```

---

### Task 6: 前端 force 透传后端 URL

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts:490-503`（loadRoster URL 拼 force）
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`（新增 force URL 用例）

**Interfaces:**
- Consumes: Task 5 的 `?force=true` 后端参数。
- Produces: `loadRoster(acc, {force:true})` 打的 URL 含 `&force=true`；非 force 不含。

- [ ] **Step 1: 写失败测试**（roster.test.tsx 末尾新增，紧接现有 "命中缓存" 用例后）

```tsx
  it("force 刷新时 URL 带 &force=true，非 force 不带", async () => {
    getMock.mockResolvedValue({ items: ROSTER, syncing: false });
    // 非 force：URL 不含 force。
    await useUserOpsStore.getState().loadRoster("accForce");
    const firstUrl = String(getMock.mock.calls.at(-1)?.[0] ?? "");
    expect(firstUrl).toContain("accountId=accForce");
    expect(firstUrl).not.toContain("force=true");
    // force：URL 含 &force=true。
    await useUserOpsStore.getState().loadRoster("accForce", { force: true });
    const forceUrl = String(getMock.mock.calls.at(-1)?.[0] ?? "");
    expect(forceUrl).toContain("force=true");
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx -t "force=true"`
Expected: FAIL —— 现 URL 恒不含 force。

- [ ] **Step 3: loadRoster 拼 force**（frontend/src/stores/userOpsStore.ts:495-497）

把：
```ts
    const data = await api.get<{ items: RosterEntry[]; syncing?: boolean }>(
      `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}`
    );
```
改为：
```ts
    const url = `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}${
      opts?.force ? "&force=true" : ""
    }`;
    const data = await api.get<{ items: RosterEntry[]; syncing?: boolean }>(url);
```

- [ ] **Step 4: 跑测试确认通过（含旧回归）**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: 新用例 + 现有 11 用例全 PASS。

- [ ] **Step 5: 前端契约门 + 提交**

Run: `cd frontend && npx tsc --noEmit && npx vitest run`
Expected: tsc 0 error，vitest 全绿。

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "feat(roster): 前端 loadRoster force 透传后端 URL(刷新真正强制重拉)"
```

---

## Self-Review

**Spec coverage:**
- 读快照优先秒回 → Task 5 非 force 分支 ✓
- 24h 过期后台自刷 → Task 3 snapshot_is_stale + Task 5 spawn_roster_refresh ✓
- 有快照永远兜底 → Task 5 force 失败/无 ready 回退旧快照分支 ✓
- 首次无快照失败显同步中 + 后台重试（解码也重试）→ Task 4 spawn_roster_refresh + Task 5 None 分支 ✓
- 后台重试上限 5 次退避 3→48s → Task 3 常量/退避 + Task 4 循环 ✓
- force 刷新透传后端 → Task 5 RosterQuery.force + Task 6 前端 URL ✓
- roster_snapshots 集合 + RosterFriend Deserialize → Task 1/2 ✓

**Placeholder scan:** 无 TBD/TODO；每步含实际代码或确切命令。

**Type consistency:**
- `RosterSnapshot { friends: Vec<RosterFriend>, total: i64, fetched_at: DateTime }` Task 1 定义、Task 4 写/读、Task 5 读一致。
- `snapshot_is_stale(DateTime, DateTime) -> bool` Task 3 定义（Step 5 改 pub(crate)）、Task 5 调用一致。
- `spawn_roster_refresh(AppState, String, String)` Task 4 定义、Task 5 三处调用（state.clone()/ws.clone()/acc.clone()）一致。
- `RosterQuery.force: bool`（serde default）Task 5 定义、`?force=true` wire、Task 6 前端拼 `&force=true` 一致。
- `read_roster_snapshot -> AppResult<Option<RosterSnapshot>>` Task 4 定义、Task 5 `?` + match Some/None 一致。

**实现者留意：**
- Task 4 的 `use crate::models::RosterSnapshot;` 与 Task 1 若重复，合并到 mcp.rs 顶部 `use crate::{...}` 块，勿双重导入。
- Task 5 抽了两个闭包（build_items 同步闭包 / load_status_map async 闭包）——若 async 闭包借用 `state`/`ws`/`acc` 与后续 spawn 的 move 冲突，改为普通 async fn 或内联；编译器报借用错时以「先算 friends 再算 status_map」顺序规避（spawn 用 clone、不夺所有权）。
- Task 5 force 失败回退旧快照时 syncing 必须为 false（有数据不显同步中）——已在分支写死 `(snap.friends, false)`。
- 集成测试（endpoint 秒回/写入/force 覆盖）属 #[ignore] Docker 层，本计划不含编写步骤，留后续 CI 专项；lib 层用纯函数 + serde round-trip 覆盖逻辑。
