# 通讯录切 contacts_fetch_full + 富化字段 + 分页 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 通讯录从 MCP `contacts_fetch_cache`（只返回 wxid 串）切到 `contacts_fetch_full`（返回昵称/头像/性别等富化字段），并给 4831 条好友的前端网格加分页 + 图片懒加载。

**Architecture:** 后端 `mcp.rs` 换工具名 + 扩 `RosterFriend`/解析器；就绪判据从「空 cache」改为读 `contacts_fetch_full` 的 `status` 字段（`refreshing:true` 是干扰项）。`sex`(int) 一路透传：MCP → roster API → 前端展示，并在批量托管时落库到 `Contact.sex`。前端 `RosterView` 加分页 + `loading="lazy"` + 性别文字。

**Tech Stack:** Rust (Axum) + serde_json / mongodb bson · React 19 + TS + Vite + Vitest

## Global Constraints

- 基线门（合并前必过）：`cargo test --lib` ≥ 350 passed / 0 failed；4 个 PBT 文件累计 ≥ 33 passed / 0 failed。
- 测试只增不删：现有 `parse_roster_items` 测试（mcp.rs:697+）和 `roster_outcome_tests`（mcp.rs:853-907）针对旧 `contacts_fetch_cache` 形态，是回归守卫，**保留不动**，只新增用例。
- `sex` 是客观事实字段（MCP 源），不进 `profile_attributes`（AI 推断空间）；忠于源：MCP `sex` int（0/1/2）原样存储，文字转换只在前端展示层。
- MCP 真实验证须串行、勿撞 429（生产端点并发受限）。
- 无人工接管红线：本改动不涉及发送/状态词表，无需碰 check-no-human-takeover lint。

---

### Task 1: 后端 `RosterFriend.sex` + `parse_roster_items` 富化（头像键 + 性别）

**Files:**
- Modify: `src/mcp.rs:411-417`（`RosterFriend` struct）
- Modify: `src/mcp.rs:522-549`（`parse_roster_items` 的 `filter_map`，两个 `RosterFriend {` 构造点）
- Test: `src/mcp.rs` 的 `#[cfg(test)] mod tests`（parse_roster_items 测试，697+ 起）

**Interfaces:**
- Produces: `RosterFriend { wxid: String, nickname: Option<String>, remark: Option<String>, avatar_url: Option<String>, sex: Option<i32> }`（供 Task 2 的 `roster_outcome_from_result` 和 Task 4 的 `roster_endpoint` 使用）
- 说明：`RosterFriend` 全项目仅在 `parse_roster_items` 内构造（2 处：字符串分支 + 对象分支），无外部字面量，加字段不触发跨文件 E0063。

- [ ] **Step 1: 写失败测试**（新增到 parse_roster_items 测试模块，紧接现有用例之后）

```rust
    #[test]
    fn parses_contacts_fetch_full_envelope_with_rich_fields() {
        // contacts_fetch_full 真实形态（2026-07-09 117 亲验）：顶层 items 数组，
        // 单条带 userName(=wxid)/nickName/bigHeadImgUrl/sex。
        let v = serde_json::json!({
            "status": "ready",
            "count": 2,
            "refreshing": true,
            "items": [
                { "userName": "wxid_full1", "nickName": "富化好友", "remark": "客户", "bigHeadImgUrl": "http://img/big", "sex": 1 },
                { "userName": "wxid_full2", "nickName": "无头像", "smallHeadImgUrl": "http://img/small", "sex": 2 }
            ]
        });
        let out = parse_roster_items(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].wxid, "wxid_full1");
        assert_eq!(out[0].nickname.as_deref(), Some("富化好友"));
        assert_eq!(out[0].avatar_url.as_deref(), Some("http://img/big"), "bigHeadImgUrl 必须命中");
        assert_eq!(out[0].sex, Some(1));
        assert_eq!(out[1].avatar_url.as_deref(), Some("http://img/small"), "smallHeadImgUrl 回退命中");
        assert_eq!(out[1].sex, Some(2));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib parses_contacts_fetch_full_envelope_with_rich_fields`
Expected: 编译失败（`RosterFriend` 无 `sex` 字段）或断言失败（`bigHeadImgUrl` 未命中、`sex` 缺失）。

- [ ] **Step 3: 给 `RosterFriend` 加 `sex` 字段**

`src/mcp.rs:411`，在 `avatar_url` 后加一行：

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct RosterFriend {
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub avatar_url: Option<String>,
    pub sex: Option<i32>,
}
```

- [ ] **Step 4: 改 `parse_roster_items` 两个构造点**

字符串分支（原 mcp.rs:529-534）补 `sex: None`：

```rust
                return Some(RosterFriend {
                    wxid: s.to_string(),
                    nickname: None,
                    remark: None,
                    avatar_url: None,
                    sex: None,
                });
```

对象分支（原 mcp.rs:539-547）头像键补 `bigHeadImgUrl`/`smallHeadImgUrl`（放最前）+ 提取 `sex`：

```rust
            Some(RosterFriend {
                wxid,
                nickname: first_str(obj, &["nickName", "nickname", "NickName"]),
                remark: first_str(obj, &["remark", "Remark", "conRemark"]),
                avatar_url: first_str(
                    obj,
                    &["bigHeadImgUrl", "smallHeadImgUrl", "bigHeadImg", "smallHeadImg", "headImgUrl", "avatarUrl", "headimgurl"],
                ),
                sex: obj.get("sex").and_then(|v| v.as_i64()).map(|n| n as i32),
            })
```

- [ ] **Step 5: 跑测试确认通过（含旧回归）**

Run: `cargo test --lib parse_roster`
Expected: 新用例 PASS，现有 parse_roster_items 用例全 PASS（`/items` 已在命名候选内，旧字符串数组形态不受影响）。

- [ ] **Step 6: 提交**

```bash
git add src/mcp.rs
git commit -m "feat(roster): RosterFriend 补 sex 字段 + parse 补 bigHeadImgUrl/smallHeadImgUrl 头像键"
```

---

### Task 2: 后端就绪判据（`status` 字段）+ 工具名切换

**Files:**
- Modify: `src/mcp.rs:558-562`（`roster_outcome_from_result`）
- Modify: `src/mcp.rs:564-596`（`fetch_roster_for_account` 工具名 + 注释）
- Test: `src/mcp.rs` 的 `mod roster_outcome_tests`（853+）

**Interfaces:**
- Consumes: `parse_roster_items`（Task 1）
- Produces: `roster_outcome_from_result` 就绪语义 —— `status=="ready"` 权威判就绪；旧形态无 `status` 时回落 `roster_result_is_empty_cache`。

- [ ] **Step 1: 写失败测试**（新增到 `roster_outcome_tests` 模块末尾）

```rust
    #[test]
    fn full_ready_with_items_is_ready() {
        // contacts_fetch_full：status=ready + items 非空 → 就绪。
        let v = serde_json::json!({ "status": "ready", "items": [ { "userName": "wxid_a", "sex": 1 } ] });
        let out = roster_outcome_from_result(&v);
        assert_eq!(out.friends.len(), 1);
        assert!(!out.syncing);
    }

    #[test]
    fn full_ready_zero_items_is_ready_not_syncing() {
        // status=ready + 空 items = 真 0 好友，就绪不重试。
        let v = serde_json::json!({ "status": "ready", "items": [] });
        let out = roster_outcome_from_result(&v);
        assert!(out.friends.is_empty());
        assert!(!out.syncing, "ready 且 0 好友必须 syncing=false");
    }

    #[test]
    fn full_pending_empty_is_syncing() {
        // status!=ready + 空 items → 未就绪，同步中（refreshing 是干扰项，不参与判据）。
        let v = serde_json::json!({ "status": "pending", "items": [], "refreshing": true });
        let out = roster_outcome_from_result(&v);
        assert!(out.friends.is_empty());
        assert!(out.syncing, "非 ready 空列表必须 syncing=true");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib full_pending_empty_is_syncing`
Expected: FAIL —— 现逻辑 `roster_result_is_empty_cache({status:pending,items:[]})` 因 `/items` 空数组存在而返回 false → syncing=false，与期望 true 相悖。

- [ ] **Step 3: 改 `roster_outcome_from_result`**

替换 `src/mcp.rs:558-562`：

```rust
fn roster_outcome_from_result(result: &serde_json::Value) -> RosterFetchOutcome {
    let friends = parse_roster_items(result);
    if !friends.is_empty() {
        // 铁律：解析出任何好友一定就绪（否则前端无限重拉且清空运营勾选）。
        return RosterFetchOutcome { friends, syncing: false };
    }
    // 空列表：区分「真 0 好友（就绪）」vs「未就绪（同步中）」。
    // contacts_fetch_full 有权威 status：ready → 就绪；其它 → 同步中。refreshing:true 带全量
    // 数据也算就绪，故不参与判据。旧 contacts_fetch_cache 形态无 status → 回落空 cache 判据。
    let syncing = match result.pointer("/status").and_then(|v| v.as_str()) {
        Some("ready") => false,
        Some(_) => true,
        None => roster_result_is_empty_cache(result),
    };
    RosterFetchOutcome { friends, syncing }
}
```

- [ ] **Step 4: 跑测试确认通过（含旧 5 个回归）**

Run: `cargo test --lib roster_outcome`
Expected: 3 新用例 PASS；现有 `production_string_array_is_ready` / `empty_object_is_syncing` / `real_zero_friends_is_ready_not_syncing` / `parseable_but_not_in_empty_cache_pathset_is_ready` / `content_fallback_array_is_ready` 全 PASS（旧形态无 `status` → 走 None 回落分支，行为不变）。

- [ ] **Step 5: 切换工具名 + 更新注释**

`src/mcp.rs:564-582`，把 `fetch_roster_for_account` 里的工具名与注释改为 `contacts_fetch_full`：

```rust
pub async fn fetch_roster_for_account(
    state: &AppState,
    account_id: &str,
) -> AppResult<RosterFetchOutcome> {
    // contacts_fetch_full 是全量好友工具（返回昵称/头像/性别等富化字段，无参）；
    // account_alias 由 logged_call_for_account 自动注入。就绪信号是返回体 status=="ready"
    // （亲验：ready 时带全量 items，refreshing:true 是后台刷新标志、非未就绪）。未就绪时
    // 同一请求内短重试（间隔 2s、最多 3 次），仍未就绪则 syncing=true 让前端提示「同步中」。
    const MAX_RETRIES: usize = 3;
    const RETRY_INTERVAL_SECS: u64 = 2;
    let mut last_result = serde_json::Value::Null;
    for attempt in 0..MAX_RETRIES {
        last_result = logged_call_for_account(
            state,
            account_id,
            "contacts_fetch_full",
            serde_json::json!({}),
        )
        .await?;
        let outcome = roster_outcome_from_result(&last_result);
        if !outcome.syncing {
            return Ok(outcome);
        }
        if attempt + 1 < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECS)).await;
        }
    }
    Ok(roster_outcome_from_result(&last_result))
}
```

- [ ] **Step 6: 跑 lib 全量 + 提交**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

```bash
git add src/mcp.rs
git commit -m "feat(roster): 就绪判据改读 contacts_fetch_full 的 status + 工具名切 contacts_fetch_full"
```

---

### Task 3: `Contact.sex` + `ApiContact.sex`（E0063 全站补齐）

**Files:**
- Modify: `src/models.rs:149-150`（`Contact` struct，`avatar_url` 后加 `sex`）
- Modify: `src/models.rs:3310`（`ApiContact` struct，`avatar_url` 后加 `sex`）
- Modify: `src/models.rs:3355`（`ApiContact::from` 映射）
- Modify: 约 30 处 `Contact { ... }` 字面量构造点（src + 测试；`Contact` 无 `#[derive(Default)]`，加字段必打挂全部）
- Test: `cargo check --tests` + `cargo test --lib`

**Interfaces:**
- Produces: `Contact.sex: Option<i32>`、`ApiContact.sex: Option<i32>`（供 Task 4 落库/输出、前端读取）

- [ ] **Step 1: 给 `Contact` 加字段**

`src/models.rs:150`（`avatar_url` 后）：

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sex: Option<i32>,
```

- [ ] **Step 2: 给 `ApiContact` 加字段 + From 映射**

`src/models.rs:3310`（`avatar_url` 后）：

```rust
    pub avatar_url: Option<String>,
    pub sex: Option<i32>,
```

`src/models.rs:3355`（`From<Contact>` impl，`avatar_url: contact.avatar_url,` 后）：

```rust
            avatar_url: contact.avatar_url,
            sex: contact.sex,
```

- [ ] **Step 3: 跑 `cargo check --tests` 收集所有 E0063**

Run: `cargo check --tests 2>&1 | grep -E "E0063|Contact\b" | head -50`
Expected: 一串 `missing field `sex` in initializer of ... Contact` —— 约 30 处，分布在 `src/agent/escalation/logic.rs`、`src/agent/knowledge_router.rs`、`src/agent/memory.rs`、`src/agent/mod.rs`、`src/agent/quiet_hours.rs`、`src/agent/review/gates.rs`、`src/cold_contact_worker.rs`、`src/planner/mod.rs`。

- [ ] **Step 4: 逐点补 `sex: None,`**

对 Step 3 列出的**每一个** `Contact { ... }` 构造点，在字段列表里加一行 `sex: None,`（紧跟 `avatar_url: ...,` 之后，保持字段顺序一致便于阅读）。这些都是测试模板/桩，性别与被测逻辑无关，故 `None`。

- [ ] **Step 5: 反复 check 直到零 E0063**

Run: `cargo check --tests 2>&1 | grep -c E0063`
Expected: `0`（若非 0，回 Step 3 看剩余点补齐）。

- [ ] **Step 6: 跑 lib 全量 + 提交**

Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed。

```bash
git add src/models.rs src/agent src/cold_contact_worker.rs src/planner
git commit -m "feat(contact): Contact/ApiContact 补 sex 字段 + 全站字面量补齐"
```

---

### Task 4: `BatchEnableCandidate.sex` + 批量托管落库 + roster API 输出

**Files:**
- Modify: `src/models.rs:3203-3211`（`BatchEnableCandidate`，加 `sex`）
- Modify: `src/routes/contacts.rs:635-637`（`batch_enable_endpoint` 的 `$set`，加 sex 带值才写）
- Modify: `src/routes/contacts.rs:404-410`（`roster_endpoint` json 输出，加 `sex`）
- Test: `cargo check` + `cargo test --lib`（端点集成测试需 Docker、`#[ignore]`，走 CI）

**Interfaces:**
- Consumes: `Contact.sex`（Task 3）、`RosterFriend.sex`（Task 1）
- Produces: roster API items 含 `"sex"`；batch-enable 接受候选 `sex` 并落库到 `Contact.sex`。
- 说明：`BatchEnableCandidate` 是 `#[derive(Deserialize)]` 请求体、无字面量构造，加 `#[serde(default)]` 字段安全。

- [ ] **Step 1: `BatchEnableCandidate` 加 `sex`**

`src/models.rs:3203`：

```rust
pub struct BatchEnableCandidate {
    pub wxid: String,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub sex: Option<i32>,
}
```

- [ ] **Step 2: 批量托管 `$set` 带值才写 sex**

`src/routes/contacts.rs:635`（`avatar_url` 的 `if let` 之后，镜像其「带值才写」保护，避免覆盖已入库数据）：

```rust
        if let Some(avatar_url) = &cand.avatar_url {
            set_doc.insert("avatar_url", avatar_url);
        }
        if let Some(sex) = cand.sex {
            set_doc.insert("sex", sex);
        }
```

- [ ] **Step 3: roster API 输出 sex**

`src/routes/contacts.rs:404-410`（json! items）：

```rust
            json!({
                "wxid": f.wxid,
                "nickname": f.nickname,
                "remark": f.remark,
                "avatarUrl": f.avatar_url,
                "sex": f.sex,
                "agentStatus": agent_status,
            })
```

- [ ] **Step 4: 编译 + lib 全量**

Run: `cargo test --lib`
Expected: 编译通过，≥ 350 passed / 0 failed。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs src/routes/contacts.rs
git commit -m "feat(roster): batch-enable 落库 sex + roster API 输出 sex"
```

---

### Task 5: 前端 `RosterEntry.sex` + `RosterView` 分页 + 懒加载 + 性别展示

**Files:**
- Modify: `frontend/src/types/index.ts:134-140`（`RosterEntry` 加 `sex`）
- Modify: `frontend/src/stores/userOpsStore.ts:112-117`（`batchEnable` payload candidate 类型加 `sex`）
- Modify: `frontend/src/features/user-ops/RosterView.tsx`（分页 hook + `loading="lazy"` + 性别文字 + 提交透传 sex）
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`

**Interfaces:**
- Consumes: roster API 的 `sex`（Task 4）
- Produces: `RosterEntry.sex?: number | null`；`RosterView` 分页展示（每页 60）。

- [ ] **Step 1: 写失败测试**（新增到 roster.test.tsx `describe` 内）

```tsx
  it("展示性别文字（男/女），并透传 sex 到 batch-enable", async () => {
    getMock.mockResolvedValue({
      items: [
        { wxid: "wx_m", nickname: "男好友", remark: null, avatarUrl: null, sex: 1, agentStatus: "not_imported" },
        { wxid: "wx_f", nickname: "女好友", remark: null, avatarUrl: null, sex: 2, agentStatus: "not_imported" },
      ],
      syncing: false,
    });
    const user = userEvent.setup();
    render(<ToastProvider><RosterView /></ToastProvider>);
    expect(await screen.findByText("男好友")).toBeInTheDocument();
    expect(screen.getByText("男")).toBeInTheDocument();
    expect(screen.getByText("女")).toBeInTheDocument();

    await user.click(screen.getByText("男好友").closest("button") as HTMLButtonElement);
    await user.type(screen.getByPlaceholderText(/本批运营备注/), "测试备注");
    await user.click(screen.getByText("加入 Agent 运营").closest("button") as HTMLButtonElement);
    await waitFor(() => {
      const call = postMock.mock.calls.find((c) => String(c[0]).includes("/contacts/batch-enable"));
      const body = call![1] as { candidates: { wxid: string; sex?: number | null }[] };
      expect(body.candidates[0].sex).toBe(1);
    });
  });

  it("超过一页时分页，切页显示下一批", async () => {
    const many = Array.from({ length: 75 }, (_, i) => ({
      wxid: `wx_${i}`, nickname: `好友${i}`, remark: null, avatarUrl: null, sex: 0, agentStatus: "not_imported",
    }));
    getMock.mockResolvedValue({ items: many, syncing: false });
    const user = userEvent.setup();
    render(<ToastProvider><RosterView /></ToastProvider>);
    // 首页 60 条：好友0 在，好友60 不在。
    expect(await screen.findByText("好友0")).toBeInTheDocument();
    expect(screen.queryByText("好友60")).not.toBeInTheDocument();
    // 翻到下一页：好友60 出现。
    await user.click(screen.getByRole("button", { name: /下一页/ }));
    expect(await screen.findByText("好友60")).toBeInTheDocument();
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx -t "性别"`
Expected: FAIL（性别文字未渲染 / `sex` 未透传 / 无「下一页」按钮）。

- [ ] **Step 3: `RosterEntry` 类型加 `sex`**

`frontend/src/types/index.ts:134`：

```ts
export type RosterEntry = {
  wxid: string;
  nickname?: string | null;
  remark?: string | null;
  avatarUrl?: string | null;
  sex?: number | null;
  agentStatus: "managed" | "normal" | "not_imported";
};
```

- [ ] **Step 4: `batchEnable` payload candidate 类型加 `sex`**

`frontend/src/stores/userOpsStore.ts:114`：

```ts
    candidates: { wxid: string; nickname?: string | null; remark?: string | null; avatarUrl?: string | null; sex?: number | null }[];
```

- [ ] **Step 5: `RosterView` 加分页 hook + 性别标签 helper**

`frontend/src/features/user-ops/RosterView.tsx`，在 `RosterView` 组件函数外（文件顶部 import 之后）加分页 hook 与常量（本地 6 行泛型 hook，避免改动正在工作的 system-strategy 文件；卡片网格每页 60）：

```tsx
const ROSTER_PAGE_SIZE = 60;
function usePagedList<T>(items: T[]) {
  const [page, setPage] = useState(0);
  const pageCount = Math.max(1, Math.ceil(items.length / ROSTER_PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const pageRows = items.slice(safePage * ROSTER_PAGE_SIZE, safePage * ROSTER_PAGE_SIZE + ROSTER_PAGE_SIZE);
  return { pageRows, pageCount, safePage, setPage };
}

const sexLabel = (sex?: number | null): string | null => {
  if (sex === 1) return "男";
  if (sex === 2) return "女";
  if (sex === 0) return "未知";
  return null; // 缺失（旧形态/无数据）不展示
};
```

- [ ] **Step 6: 在组件里接入分页（`filtered` 之后）**

`RosterView.tsx`，`filtered` useMemo 之后加：

```tsx
  const { pageRows, pageCount, safePage, setPage } = usePagedList(filtered);
```

把渲染网格的 `filtered.map(...)`（RosterView.tsx:198）改为 `pageRows.map(...)`。

- [ ] **Step 7: 卡片加懒加载头像 + 性别文字**

`RosterView.tsx:210-214` 头像 `<img>` 加 `loading="lazy"`：

```tsx
                {entry.avatarUrl ? (
                  <img className={styles.avatar} src={entry.avatarUrl} alt="" loading="lazy" />
                ) : (
                  <span className={styles.avatarFallback}>{initial(entry)}</span>
                )}
```

`cardBody`（RosterView.tsx:215-220）内 wxid 之后加性别文字（有值才渲染）：

```tsx
                <div className={styles.cardBody}>
                  <strong className={styles.name}>
                    {entry.remark || entry.nickname || entry.wxid}
                  </strong>
                  <small className={styles.sub}>{entry.wxid}</small>
                  {sexLabel(entry.sex) && <small className={styles.sub}>{sexLabel(entry.sex)}</small>}
                </div>
```

- [ ] **Step 8: 网格下方加翻页控件**

`RosterView.tsx`，把渲染网格的 `<div className={styles.grid}>...</div>` 用 fragment 包起来并在其后加分页条（仅多页时显示）：

```tsx
        <>
          <div className={styles.grid}>
            {pageRows.map((entry) => {
              // ...（原卡片渲染不变）
            })}
          </div>
          {pageCount > 1 && (
            <div className={styles.pager}>
              <button type="button" className={styles.ghostBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
              <span className={styles.pagerInfo}>{safePage + 1} / {pageCount}</span>
              <button type="button" className={styles.ghostBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
            </div>
          )}
        </>
```

- [ ] **Step 9: 加分页条样式**

`frontend/src/features/user-ops/RosterView.module.css` 末尾加（复用现有 `.ghostBtn`，只补容器）：

```css
.pager {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 12px;
  margin-top: 16px;
}
.pagerInfo {
  font-size: 13px;
  color: var(--text-secondary, #6b7280);
}
```

- [ ] **Step 10: 提交透传 sex**

`RosterView.tsx:101-108` 的 candidates map 加 `sex`：

```tsx
      const candidates = roster
        .filter((r) => selectedWxids.has(r.wxid))
        .map((r) => ({
          wxid: r.wxid,
          nickname: r.nickname,
          remark: r.remark,
          avatarUrl: r.avatarUrl,
          sex: r.sex,
        }));
```

- [ ] **Step 11: 跑测试确认通过（含旧回归）**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: 新用例 + 现有 6 个用例全 PASS。

- [ ] **Step 12: 前端契约门 + 提交**

Run: `cd frontend && npx tsc --noEmit && npx vitest run`
Expected: tsc 0 error，vitest 全绿。

```bash
git add frontend/src/types/index.ts frontend/src/stores/userOpsStore.ts frontend/src/features/user-ops/RosterView.tsx frontend/src/features/user-ops/RosterView.module.css frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "feat(roster): 前端 RosterView 分页 + 头像懒加载 + 性别文字展示"
```

---

## Self-Review

**Spec coverage:**
- 全切 `contacts_fetch_full` → Task 2 Step 5 ✓
- 保留 3×2s 短重试 → Task 2 Step 5（MAX_RETRIES/RETRY_INTERVAL_SECS 不变）✓
- 性别透传 + 落库 → Task 1（parse）/ Task 3（Contact.sex）/ Task 4（batch-enable $set + API 输出）/ Task 5（前端透传）✓
- 性别显示文字 男/女/未知 → Task 5 Step 5（sexLabel）✓
- 分页 → Task 5 Step 5-8 ✓
- 头像键补 `bigHeadImgUrl`/`smallHeadImgUrl` → Task 1 Step 4 ✓
- `<img loading="lazy">` → Task 5 Step 7 ✓
- 就绪判据（status 权威、refreshing 干扰项）→ Task 2 Step 3 ✓
- 旧测试保留 → Task 1/2 均只新增用例，旧用例验证通过 ✓

**Placeholder scan:** 无 TBD/TODO；每步含实际代码或确切命令。Task 3 的 30 站点补齐是机械重复（每处加同一行 `sex: None,`），用 `cargo check --tests` 逐个定位，非占位符。

**Type consistency:** `sex: Option<i32>`（Rust `RosterFriend`/`Contact`/`ApiContact`/`BatchEnableCandidate`）↔ `sex?: number | null`（TS `RosterEntry` + payload）↔ API wire `"sex"`。`roster_outcome_from_result` / `parse_roster_items` / `fetch_roster_for_account` 签名不变，仅内部逻辑与字段扩展。一致。
