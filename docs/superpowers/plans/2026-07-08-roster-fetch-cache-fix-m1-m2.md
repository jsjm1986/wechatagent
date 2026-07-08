# 通讯录 roster 修复 — 模块1(解析器) + 模块2(空态处理) 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让通讯录能正确解析 `contacts_fetch_cache` 的真实返回形态（`{result:{friends:[wxid字符串]}}`），并把异步空 cache 与「真的没好友」区分开，前端显示「同步中」而非误导性的「暂无好友」。

**Architecture:** 后端 `parse_roster_items` 增加嵌套路径候选 + 支持纯字符串元素；`fetch_roster_for_account` 对空 cache 短重试并返回 `(friends, syncing)`；`roster_endpoint` 响应加 `syncing` 字段；前端 `loadRoster` 透传 `syncing`，`RosterView` 据此显示「同步中」并自动重拉。

**Tech Stack:** Rust (Axum) 后端 + React19/Vite/TypeScript 前端；后端 `cargo test --lib`，前端 `npx vitest`。

## Global Constraints

- 改代码前 100% 读懂相关代码，file:line 引用亲验（CLAUDE.md 最高红线）。
- 不为迎合测试改业务逻辑（反过拟合）；真 bug 才修。
- no-human-takeover lint：`src/agent|routes|evolution` + `frontend/src` 新增行禁词 `人工/接管/takeover/hand-off`（本计划文案不涉及，仍需自查）。
- 基线门不回退：`cargo test --lib` ≥ 350 passed 0 failed；4 个 PBT 累计 ≥ 33。
- 向后兼容红线：Contact serde default 不破坏（本计划不改 Contact struct）。
- 本地磁盘紧张：后端只跑 `cargo test --lib`，不跑全量集成测试。Windows Defender 锁 worktree target → 用 `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`。
- 中文回报。仅用户明确要求时才 commit/push（本计划每任务含 commit 步，执行时按用户节奏）。

## 真实数据形态（2026-07-08 线上 117 亲验）

`contacts_fetch_cache` 就绪返回（经 call_tool_with_key 剥壳后，parse_roster_items 收到的是 structuredContent 本体）：

```json
{ "result": { "friends": ["medianote", "wxid_8874178741811", "wxid_2o93p4cc9n4x22"] } }
```

空态返回：`{}`（structuredContent 空对象，isError:None，无错误）。

## File Structure

- `src/mcp.rs`：`parse_roster_items`（447-502 区）改造支持 `/result/friends` + 字符串元素；`fetch_roster_for_account`（504-517）加短重试 + 返回 syncing；新增单测。
- `src/routes/contacts.rs`：`roster_endpoint`（367-414）响应加 `syncing` 字段。
- `frontend/src/stores/userOpsStore.ts`：`loadRoster`（484-489）返回 `{items, syncing}`。
- `frontend/src/features/user-ops/RosterView.tsx`：消费 syncing，显示「同步中」+ 自动重拉。
- `frontend/src/__tests__/features/user-ops/roster.test.tsx`：补 syncing 态测试。

---

### Task 1: parse_roster_items 支持 `/result/friends` 嵌套路径 + 纯字符串元素

**Files:**
- Modify: `src/mcp.rs:447-502`（`parse_roster_items` 函数体）
- Test: `src/mcp.rs`（`roster_parse_tests` mod，616-688 区，追加）

**Interfaces:**
- Consumes: 无（纯函数改造）
- Produces: `parse_roster_items(result: &serde_json::Value) -> Vec<RosterFriend>` 签名不变；行为扩展：能解析 `{result:{friends:[字符串]}}` 与混合数组。

- [ ] **Step 1: 写失败测试**

在 `src/mcp.rs` 的 `mod roster_parse_tests`（`}` 闭合前，约 687 行）追加：

```rust
    #[test]
    fn parses_nested_result_friends_string_array() {
        // 生产真实形态（2026-07-08 线上亲验）：structuredContent.result.friends
        // 是纯 wxid 字符串数组。
        let v = serde_json::json!({
            "result": { "friends": ["medianote", "wxid_2o93p4cc9n4x22", "wxid_ax8y68dxucvm22"] }
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 3, "纯字符串数组应逐条解析为 wxid-only");
        assert_eq!(out[0].wxid, "medianote");
        assert_eq!(out[1].wxid, "wxid_2o93p4cc9n4x22");
        assert_eq!(out[0].nickname, None, "字符串元素无昵称");
        assert_eq!(out[0].avatar_url, None, "字符串元素无头像");
    }

    #[test]
    fn parses_nested_result_friends_object_array() {
        // 防御：万一 GeWe 换成 result.friends 里带对象详情，也要能解析。
        let v = serde_json::json!({
            "result": { "friends": [
                { "wxid": "wxid_a", "nickName": "小明", "bigHeadImg": "http://img/a" }
            ]}
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].wxid, "wxid_a");
        assert_eq!(out[0].nickname.as_deref(), Some("小明"));
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/a"));
    }

    #[test]
    fn empty_object_yields_empty_roster() {
        // 空 cache 返回 {} → 空列表（不 panic、不误命中）。
        let v = serde_json::json!({});
        assert_eq!(parse_roster_items(&v).len(), 0);
    }

    #[test]
    fn mixed_string_and_object_array_all_parsed() {
        // 混合数组：字符串 + 对象都应解析（不因首元素类型短路）。
        let v = serde_json::json!({
            "result": { "friends": [
                "wxid_str",
                { "userName": "wxid_obj", "nickName": "对象好友" }
            ]}
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].wxid, "wxid_str");
        assert_eq!(out[1].wxid, "wxid_obj");
        assert_eq!(out[1].nickname.as_deref(), Some("对象好友"));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo test --lib parses_nested_result_friends -- --nocapture`
Expected: FAIL — `parses_nested_result_friends_string_array` 断言 len==3 实得 0（当前候选无 /result/friends 且元素非对象）。

- [ ] **Step 3: 改造 parse_roster_items**

把 `src/mcp.rs` 的 `parse_roster_items`（447-502）整体替换为：

```rust
fn parse_roster_items(result: &serde_json::Value) -> Vec<RosterFriend> {
    // 数组路径多候选。取第一个真正 **是数组** 的候选——不能先选中"存在的键"再
    // as_array，否则某高优先候选键存在但非数组（server 回 {} 或标量）会短路掉后面
    // 真正的数组候选，导致空列表。
    //
    // 关键事实（2026-07-08 线上亲验）：contacts_fetch_cache 就绪返回
    // structuredContent = {result:{friends:[wxid字符串]}}，故真正生效的是嵌套
    // /result/friends。call_tool_with_key 已剥掉 JSON-RPC 外壳与 content[0].text，
    // 生产态本函数收到的就是 structuredContent 本体。顶层 /contacts 等 + /content
    // 兜底仅作防御（万一 server 换形态或某调用方传入完整外壳）。
    let first_array = |v: &serde_json::Value, keys: &[&str]| -> Option<Vec<serde_json::Value>> {
        for k in keys {
            if let Some(arr) = v.pointer(k).and_then(|x| x.as_array()) {
                return Some(arr.clone());
            }
        }
        None
    };
    let named = [
        // 生产态：contacts_fetch_cache 的嵌套 result.friends。
        "/result/friends",
        "/result/contacts",
        "/result/list",
        // 顶层数组（其它工具/形态）。
        "/contacts",
        "/friends",
        "/list",
        "/items",
        "/data",
        // 防御：完整外壳形态（未剥壳的调用方）。
        "/structuredContent/result/friends",
        "/structuredContent/contacts",
        "/structuredContent/friends",
        "/structuredContent/list",
    ];
    let arr = first_array(result, &named)
        .or_else(|| {
            // content[0].text 内嵌 JSON 字符串形态（防御）。
            let text = result.pointer("/content/0/text")?.as_str()?;
            let inner: serde_json::Value = serde_json::from_str(text).ok()?;
            first_array(&inner, &named).or_else(|| inner.as_object().and_then(contact_like_array))
        })
        // 末位兜底：命名候选全落空时，按内容识别顶层任一「联系人数组」。
        .or_else(|| result.as_object().and_then(contact_like_array))
        .unwrap_or_default();

    arr.iter()
        .filter_map(|item| {
            // 纯字符串元素：直接当 wxid（contacts_fetch_cache 的生产形态）。
            if let Some(s) = item.as_str() {
                if s.is_empty() {
                    return None;
                }
                return Some(RosterFriend {
                    wxid: s.to_string(),
                    nickname: None,
                    remark: None,
                    avatar_url: None,
                });
            }
            // 对象元素：从命名键提取（防御其它形态）。
            let obj = item.as_object()?;
            let wxid = first_str(obj, &["wxid", "userName", "UserName", "username"])?;
            Some(RosterFriend {
                wxid,
                nickname: first_str(obj, &["nickName", "nickname", "NickName"]),
                remark: first_str(obj, &["remark", "Remark", "conRemark"]),
                avatar_url: first_str(
                    obj,
                    &["bigHeadImg", "smallHeadImg", "headImgUrl", "avatarUrl", "headimgurl"],
                ),
            })
        })
        .collect()
}
```

同时改 `contact_like_array`（433-445）让它也识别「纯字符串数组」——把该函数替换为：

```rust
/// 从对象里挑第一个「元素像联系人」的数组值：元素带 wxid/userName/username 键，
/// 或元素是纯字符串（contacts_fetch_cache 的 wxid 字符串数组）。
/// 命名候选之外再兜一层「按内容识别数组」——避免 server 用列表外的新 key 时整表解析成空。
fn contact_like_array(obj: &serde_json::Map<String, serde_json::Value>) -> Option<Vec<serde_json::Value>> {
    for value in obj.values() {
        if let Some(arr) = value.as_array() {
            let looks_like_contacts = arr.first().is_some_and(|first| {
                first.as_str().is_some()
                    || first.as_object().is_some_and(|o| {
                        ["wxid", "userName", "UserName", "username"].iter().any(|k| o.contains_key(*k))
                    })
            });
            if looks_like_contacts {
                return Some(arr.clone());
            }
        }
    }
    None
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo test --lib roster_parse -- --nocapture`
Expected: PASS — 新 4 个测试 + 原有 `roster_parse_tests` 全部通过（原测试 `parses_unwrapped_structured_content_top_level_contacts` 等仍绿，向后兼容）。

- [ ] **Step 5: Commit**

```bash
git add src/mcp.rs
git commit -m "fix(roster): parse_roster_items 支持 /result/friends 嵌套 + 纯 wxid 字符串元素

contacts_fetch_cache 线上真实返回 {result:{friends:[wxid字符串]}},原候选路径无
/result/friends 且要求元素 as_object → 恒解析空。加嵌套候选 + 字符串元素分支,
contact_like_array 兜底也识别字符串数组。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: fetch_roster_for_account 空 cache 短重试 + 返回 syncing

**Files:**
- Modify: `src/mcp.rs:504-517`（`fetch_roster_for_account`）
- Test: `src/mcp.rs`（`roster_parse_tests` 或新 mod，追加纯函数测试）

**Interfaces:**
- Consumes: `parse_roster_items`（Task 1）、`logged_call_for_account`（mcp.rs:319，已存在）
- Produces: `fetch_roster_for_account(state, account_id) -> AppResult<RosterFetchOutcome>`，其中
  ```rust
  pub struct RosterFetchOutcome {
      pub friends: Vec<RosterFriend>,
      pub syncing: bool, // true = cache 空 {} 且重试仍空（GeWe 异步未就绪）
  }
  ```
  判定纯函数 `roster_result_is_empty_cache(result: &Value) -> bool` 供测试。

- [ ] **Step 1: 写失败测试（纯函数判定）**

在 `src/mcp.rs` 追加一个新测试 mod（文件末尾 `}` 之后）：

```rust
#[cfg(test)]
mod roster_empty_cache_tests {
    use super::roster_result_is_empty_cache;

    #[test]
    fn empty_object_is_empty_cache() {
        // contacts_fetch_cache 未就绪返回 {} → 判定为空 cache（syncing）。
        assert!(roster_result_is_empty_cache(&serde_json::json!({})));
    }

    #[test]
    fn null_is_empty_cache() {
        // call_tool_with_key 无 structuredContent 时返回 Null → 也视为空 cache。
        assert!(roster_result_is_empty_cache(&serde_json::Value::Null));
    }

    #[test]
    fn populated_result_is_not_empty_cache() {
        // 有 friends 数据 → 不是空 cache（已就绪）。
        let v = serde_json::json!({ "result": { "friends": ["wxid_a"] } });
        assert!(!roster_result_is_empty_cache(&v));
    }

    #[test]
    fn empty_friends_array_is_not_empty_cache() {
        // result.friends 是空数组（真 0 好友，已就绪）→ 不是空 cache，不该无限重试。
        let v = serde_json::json!({ "result": { "friends": [] } });
        assert!(!roster_result_is_empty_cache(&v));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo test --lib roster_empty_cache -- --nocapture`
Expected: FAIL — `roster_result_is_empty_cache` 未定义（编译错误 unresolved import）。

- [ ] **Step 3: 实现判定函数 + 改造 fetch_roster_for_account**

在 `src/mcp.rs` 的 `RosterFriend` struct 定义后（约 417 行后）加：

```rust
/// roster 拉取结果：友列表 + 是否仍在同步（cache 空 {} 未就绪）。
pub struct RosterFetchOutcome {
    pub friends: Vec<RosterFriend>,
    pub syncing: bool,
}

/// 判定 contacts_fetch_cache 返回是否为「空 cache（异步未就绪）」——区别于
/// 「真 0 好友」（result.friends 是空数组）。空对象 {} / Null → 空 cache（可重试）；
/// 任何含非空数组候选的形态 → 已就绪。空数组 → 已就绪（真 0 好友，不重试）。
fn roster_result_is_empty_cache(result: &serde_json::Value) -> bool {
    match result {
        serde_json::Value::Null => true,
        serde_json::Value::Object(map) if map.is_empty() => true,
        // result.friends / result.contacts 等存在且是数组（哪怕空）→ 已就绪。
        _ => {
            // 若解析能拿到任何数组候选（含空数组），视为已就绪；完全无数组候选 → 空 cache。
            let has_any_array = ["/result/friends", "/result/contacts", "/result/list",
                "/contacts", "/friends", "/list", "/items", "/data"]
                .iter()
                .any(|k| result.pointer(k).and_then(|v| v.as_array()).is_some());
            !has_any_array
        }
    }
}
```

把 `fetch_roster_for_account`（原 504-517）替换为：

```rust
pub async fn fetch_roster_for_account(
    state: &AppState,
    account_id: &str,
) -> AppResult<RosterFetchOutcome> {
    // contacts_fetch_cache 是全量好友工具（gewe "Fetch the full remote contacts cache
    // from GeWe"，无参）；account_alias 由 logged_call_for_account 自动注入。
    // GeWe 缓存异步就绪：未就绪时返回 {}（空），就绪返回 {result:{friends:[wxid]}}。
    // 空 {} 时同一请求内短重试（间隔 2s、最多 3 次），仍空则返回 syncing=true 让
    // 前端提示「同步中」并自动重拉。重试复用同一 MCP session，间隔足够避免自撞 429。
    const MAX_RETRIES: usize = 3;
    const RETRY_INTERVAL_SECS: u64 = 2;
    let mut last_result = serde_json::Value::Null;
    for attempt in 0..MAX_RETRIES {
        last_result = logged_call_for_account(
            state,
            account_id,
            "contacts_fetch_cache",
            serde_json::json!({}),
        )
        .await?;
        if !roster_result_is_empty_cache(&last_result) {
            return Ok(RosterFetchOutcome {
                friends: parse_roster_items(&last_result),
                syncing: false,
            });
        }
        // 空 cache：还有重试机会则等待后重试（最后一次不等）。
        if attempt + 1 < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECS)).await;
        }
    }
    // 重试用尽仍空 → syncing。friends 用最后一次解析（正常为空）。
    Ok(RosterFetchOutcome {
        friends: parse_roster_items(&last_result),
        syncing: true,
    })
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo test --lib roster_empty_cache -- --nocapture`
Expected: PASS — 4 个判定测试全通过。

- [ ] **Step 5: 编译确认（fetch_roster_for_account 签名变了，调用方 roster_endpoint 会编译错，Task 3 修）**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo build --lib 2>&1 | grep -E "error|roster_endpoint" | head`
Expected: 出现 `roster_endpoint` 处的类型错误（`RosterFetchOutcome` vs `Vec<RosterFriend>`）——预期，Task 3 修复。若无其它错误即正常。

- [ ] **Step 6: Commit（与 Task 3 一起可编译，但先提交 mcp 层）**

暂不单独提交（签名变更导致 build 未过）。合并到 Task 3 提交。跳过本步，继续 Task 3。

---

### Task 3: roster_endpoint 响应加 syncing 字段

**Files:**
- Modify: `src/routes/contacts.rs:367-414`（`roster_endpoint`）

**Interfaces:**
- Consumes: `fetch_roster_for_account` → `RosterFetchOutcome{friends, syncing}`（Task 2）
- Produces: `GET /api/contacts/roster` 响应体 `{items, total, syncing}`（新增 syncing 布尔）

- [ ] **Step 1: 改造 roster_endpoint 消费 RosterFetchOutcome**

把 `src/routes/contacts.rs` 的 `roster_endpoint`（375 行的 fetch 调用 + 396-413 的 items 构建）改为：

将第 375 行：
```rust
    let friends = mcp::fetch_roster_for_account(&state, &query.account_id).await?;
```
替换为：
```rust
    let outcome = mcp::fetch_roster_for_account(&state, &query.account_id).await?;
    let friends = outcome.friends;
```

将末尾 items 构建后的返回（396-413）中的最终 `Ok(Json(...))` 从：
```rust
    let total = items.len();
    Ok(Json(json!({ "items": items, "total": total })))
```
替换为：
```rust
    let total = items.len();
    Ok(Json(json!({ "items": items, "total": total, "syncing": outcome.syncing })))
```

- [ ] **Step 2: 编译确认通过**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo build --lib 2>&1 | tail -5`
Expected: `Finished` 无 error。

- [ ] **Step 3: 跑 lib 测试确认无回退**

Run: `CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok`，passed 数 ≥ 350，0 failed（含新增 8 个 roster 测试）。

- [ ] **Step 4: Commit（mcp + routes 一起，可编译）**

```bash
git add src/mcp.rs src/routes/contacts.rs
git commit -m "fix(roster): 空 cache 短重试 + roster 响应加 syncing 字段

contacts_fetch_cache 异步空 {} 时同一请求短重试(2s×3),仍空则 syncing=true;
roster_endpoint 响应加 syncing 让前端区分「同步中」与「真 0 好友」。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 前端 loadRoster 透传 syncing + RosterView 显示「同步中」并自动重拉

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts:111`（loadRoster 签名）、`:484-489`（实现）
- Modify: `frontend/src/features/user-ops/RosterView.tsx`（消费 syncing）
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`（追加 syncing 态测试）

**Interfaces:**
- Consumes: `GET /api/contacts/roster` 响应 `{items, total, syncing}`（Task 3）
- Produces: `loadRoster(accountId) -> Promise<{items: RosterEntry[], syncing: boolean}>`

- [ ] **Step 1: 写失败测试（前端 syncing 态渲染「同步中」）**

在 `frontend/src/__tests__/features/user-ops/roster.test.tsx` 的 `describe` 内追加（注意现有 `getMock.mockResolvedValue({ items: ROSTER })` 在 beforeEach，本测试单独覆盖返回）：

```tsx
  it("cache 同步中(syncing:true,空列表)显示同步中提示而非「暂无好友」", async () => {
    // 后端 cache 未就绪：items 空 + syncing:true。
    getMock.mockResolvedValue({ items: [], syncing: true });
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    // 显示「同步中」文案，不显示「暂无好友」。
    expect(await screen.findByText(/正在从微信同步好友/)).toBeInTheDocument();
    expect(screen.queryByText("暂无好友")).not.toBeInTheDocument();
  });

  it("syncing:false 且空列表才显示「暂无好友」", async () => {
    getMock.mockResolvedValue({ items: [], syncing: false });
    render(
      <ToastProvider>
        <RosterView />
      </ToastProvider>
    );
    expect(await screen.findByText("暂无好友")).toBeInTheDocument();
    expect(screen.queryByText(/正在从微信同步好友/)).not.toBeInTheDocument();
  });
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: FAIL — 新测试找不到「正在从微信同步好友」文案（当前无 syncing 逻辑，空列表恒显「暂无好友」）。

- [ ] **Step 3: 改 loadRoster 返回 {items, syncing}**

`frontend/src/stores/userOpsStore.ts` 第 111 行签名：
```ts
  loadRoster: (accountId: string) => Promise<RosterEntry[]>;
```
改为：
```ts
  loadRoster: (accountId: string) => Promise<{ items: RosterEntry[]; syncing: boolean }>;
```

第 484-489 行实现：
```ts
  loadRoster: async (accountId) => {
    const data = await api.get<{ items: RosterEntry[] }>(
      `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}`
    );
    return data.items;
  },
```
替换为：
```ts
  loadRoster: async (accountId) => {
    const data = await api.get<{ items: RosterEntry[]; syncing?: boolean }>(
      `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}`
    );
    return { items: data.items, syncing: data.syncing ?? false };
  },
```

- [ ] **Step 4: 改 RosterView 消费 syncing + 自动重拉**

`frontend/src/features/user-ops/RosterView.tsx`：

(a) 新增 syncing state（在 `const [roster, setRoster] = useState<RosterEntry[]>([]);` 后加，约第 24 行后）：
```tsx
  const [syncing, setSyncing] = useState(false);
```

(b) refresh 里消费返回结构。当前（约 48-58 行）：
```tsx
      try {
        const items = await loadRoster(accountId);
        if (isStale()) return; // 已有更新的请求发出，丢弃本次过时结果。
        setRoster(items);
      } catch (e) {
```
替换为：
```tsx
      try {
        const { items, syncing: isSyncing } = await loadRoster(accountId);
        if (isStale()) return; // 已有更新的请求发出，丢弃本次过时结果。
        setRoster(items);
        setSyncing(isSyncing);
      } catch (e) {
```

(c) 空态渲染分支（当前约 170-175 行 `filtered.length === 0` 分支）区分 syncing。当前：
```tsx
      ) : filtered.length === 0 ? (
        <div className={styles.empty}>
          <Users size={22} />
          <strong>暂无好友</strong>
          <p>该账号还没有拉取到好友，或过滤条件无匹配。点「刷新」重新从 MCP 拉取。</p>
        </div>
      ) : (
```
替换为：
```tsx
      ) : filtered.length === 0 ? (
        <div className={styles.empty}>
          <Users size={22} />
          {syncing ? (
            <>
              <strong>正在从微信同步好友…</strong>
              <p>GeWe 正在准备该账号的好友列表，稍候会自动刷新。也可点「刷新」重试。</p>
            </>
          ) : (
            <>
              <strong>暂无好友</strong>
              <p>该账号还没有拉取到好友，或过滤条件无匹配。点「刷新」重新从 MCP 拉取。</p>
            </>
          )}
        </div>
      ) : (
```

(d) syncing 时自动重拉（在现有 `useEffect(() => { void refresh(effectiveAccountId); }, [effectiveAccountId, refresh]);` 后新增一个 effect）：
```tsx
  // cache 同步中时每 8s 自动重拉，直到就绪（syncing 变 false）或账号切换。
  useEffect(() => {
    if (!syncing || !effectiveAccountId) return;
    const timer = setInterval(() => {
      void refresh(effectiveAccountId);
    }, 8000);
    return () => clearInterval(timer);
  }, [syncing, effectiveAccountId, refresh]);
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: PASS — 全部通过（原 3 个 + 新 2 个 syncing 测试）。

- [ ] **Step 6: tsc 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 无输出（EXIT 0）。

- [ ] **Step 7: 全量前端 vitest 无回退**

Run: `cd frontend && npx vitest run`
Expected: 全绿（既有文件数 + roster 新测试），0 failed。

- [ ] **Step 8: Commit**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/RosterView.tsx frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "fix(roster): 前端区分 cache 同步中与真空态,同步中自动重拉

loadRoster 透传后端 syncing;RosterView 在 syncing 时显示「正在从微信同步好友」
而非误导的「暂无好友」,并每 8s 自动重拉直到就绪;复用切账号守卫作废过时重拉。

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 验证清单（模块1+2 完成后）

- [ ] `cargo test --lib` ≥ 350 passed 0 failed（含 8 个新 roster 测试）
- [ ] 前端 vitest 全绿（含 2 个新 syncing 测试）
- [ ] `tsc --noEmit` 干净
- [ ] no-human-takeover lint 新增行无禁词（`git diff origin/main | grep -iE "人工|接管|takeover|hand-off"` 应无命中）
- [ ] 部署到 117 后线上验证：cache 就绪时列表出 wxid（首字母头像）；空 cache 时显示「同步中」并自动刷新

## 模块 3+4 待续（本计划不含）

昵称头像可视区懒加载 + detail 落库缓存 + 429 停批兜底——待 MCP 限流冷却后串行验到 `contact_get_detail` 精确返回字段（昵称/头像 key 名），再补独立计划。设计见 `docs/superpowers/specs/2026-07-08-roster-fetch-cache-shape-design.md` 模块 3、4。
