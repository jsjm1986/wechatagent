# 通讯录性别解析修复 + 非真人标记折叠 + roster 缓存 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复通讯录性别全显「未知」的解析 bug，把微信系统账号标记为非真人并在前端默认折叠，roster 数据改 store 缓存（进频道不重拉、仅点刷新才重拉）。

**Architecture:** 后端 `mcp.rs` 修 `sex` 解析（取 int64 对象的 `.low`）+ 加 `is_non_human` 白名单判定，roster API 透出 `isNonHuman`；前端 `RosterView` 按真人/非真人拆分展示（非真人默认折叠），roster 数据源从组件 local state 提到 `userOpsStore` 的 `rosterCache`（按 accountId 键控，force 才重拉）。

**Tech Stack:** Rust (Axum) + serde_json · React 19 + TS + Zustand + Vitest

## Global Constraints

- 基线门：`cargo test --lib` ≥ 350 passed / 0 failed；前端 `tsc --noEmit` 0 error + `vitest run` 全绿。
- 测试只增不删：`parse_roster_items` 测试（mcp.rs）、`roster.test.tsx`（6 旧用例）是回归守卫，保留不动，只新增。
- 公众号无可靠字段识别，**不自动过滤**（保留当普通好友，避免误伤真人）。非真人判定仅限：`type=="system"` 或 wxid 在微信保留白名单。
- `sex` 忠于源：原始 int（0未知/1男/2女）存储，男/女文字转换只在前端。
- 缓存 session 内存活（zustand），不做 localStorage 持久化；`syncing:true` 结果不落缓存。

---

### Task 1: 后端 sex 解析修复（int64 对象取 .low）

**Files:**
- Modify: `src/mcp.rs:559`（parse_roster_items 对象分支 sex 提取）
- Test: `src/mcp.rs` roster_parse_tests 模块

**Interfaces:**
- Produces: `parse_roster_items` 对 `sex:{high,low}` 对象形态正确提取（`.low`），裸整数形态仍兼容。

- [ ] **Step 1: 写失败测试**（新增到 roster_parse_tests 模块，紧接现有用例后）

```rust
    #[test]
    fn parses_sex_int64_object_form() {
        // MCP contacts_fetch_full 真实形态：sex 是 int64 序列化对象 {high,low}，真值在 .low。
        let v = serde_json::json!({
            "status": "ready",
            "items": [
                { "userName": "wx_m", "nickName": "男", "sex": { "high": 0, "low": 1, "unsigned": false } },
                { "userName": "wx_f", "nickName": "女", "sex": { "high": 0, "low": 2, "unsigned": false } },
                { "userName": "wx_bare", "nickName": "裸整数", "sex": 1 }
            ]
        });
        let out = parse_roster_items(&v);
        assert_eq!(out[0].sex, Some(1), "对象 {{low:1}} → 男");
        assert_eq!(out[1].sex, Some(2), "对象 {{low:2}} → 女");
        assert_eq!(out[2].sex, Some(1), "裸整数 1 仍兼容");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib parses_sex_int64_object_form`
Expected: FAIL —— 现 `as_i64()` 对 `{high,low}` 对象返回 None，`out[0].sex` 是 None ≠ Some(1)。

- [ ] **Step 3: 改 sex 提取**（src/mcp.rs:559）

把：
```rust
                sex: obj.get("sex").and_then(|v| v.as_i64()).map(|n| n as i32),
```
改为：
```rust
                sex: obj
                    .get("sex")
                    .and_then(|v| v.as_i64().or_else(|| v.get("low").and_then(|l| l.as_i64())))
                    .map(|n| n as i32),
```

- [ ] **Step 4: 跑测试确认通过（含旧回归）**

Run: `cargo test --lib roster_parse`
Expected: 新用例 PASS，现有 parse 用例全 PASS（旧用例 sex 多为缺省/裸值，不受影响）。

- [ ] **Step 5: 提交**

```bash
git add src/mcp.rs
git commit -m "fix(roster): sex 解析兼容 int64 对象形态(取 .low),修性别全显未知"
```

---

### Task 2: 后端非真人判定（白名单 + is_non_human 字段）

**Files:**
- Modify: `src/mcp.rs`（加白名单常量 + `is_non_human_account` 函数 + `RosterFriend.is_non_human` 字段 + 两处构造点）
- Modify: `src/routes/contacts.rs:409`（roster API 输出 isNonHuman）
- Test: `src/mcp.rs` 新增 is_non_human 测试模块

**Interfaces:**
- Consumes: Task 1 的 sex 解析（同函数）。
- Produces: `RosterFriend { ..., sex, is_non_human: bool }`；`fn is_non_human_account(user_name: &str, item_type: Option<&str>) -> bool`；roster API items 含 `"isNonHuman"`。

- [ ] **Step 1: 写失败测试**（新增独立测试模块到 mcp.rs 末尾）

```rust
#[cfg(test)]
mod is_non_human_tests {
    use super::is_non_human_account;

    #[test]
    fn system_type_is_non_human() {
        assert!(is_non_human_account("weixin", Some("system")));
    }

    #[test]
    fn whitelisted_wxid_is_non_human() {
        assert!(is_non_human_account("fmessage", Some("friend")));
        assert!(is_non_human_account("qqmail", None));
        assert!(is_non_human_account("mphelper", Some("friend")));
    }

    #[test]
    fn real_person_is_not_non_human() {
        // 真人：新号 wxid_ / 老号自定义短 id —— 都不是非真人。
        assert!(!is_non_human_account("wxid_42jvcxc49rbf12", Some("friend")));
        assert!(!is_non_human_account("songboyu1993", Some("friend")));
    }

    #[test]
    fn public_account_not_misjudged() {
        // 公众号(福州晚报 wxid_8874178741811)无可靠字段识别 → 不误判为非真人。
        assert!(!is_non_human_account("wxid_8874178741811", Some("friend")));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib is_non_human`
Expected: 编译失败（`is_non_human_account` 未定义）。

- [ ] **Step 3: 加白名单常量 + 判定函数**（src/mcp.rs，放在 `parse_roster_items` 函数前）

```rust
/// 微信官方保留系统账号 wxid（业界通用白名单）——这些不是真人好友，
/// 通讯录里标记为非真人（前端默认折叠）。公众号无可靠字段识别，不在此列。
const WECHAT_SYSTEM_ACCOUNTS: &[&str] = &[
    "fmessage", "qqmail", "weixin", "mphelper", "medianote",
    "qmessage", "floatbottle", "tmessage", "qqsync", "newsapp",
    "filehelper", "weibo", "brandsessionholder",
];

/// 判定是否非真人账号：type=="system" 或 wxid 命中微信保留白名单。
fn is_non_human_account(user_name: &str, item_type: Option<&str>) -> bool {
    item_type == Some("system") || WECHAT_SYSTEM_ACCOUNTS.contains(&user_name)
}
```

- [ ] **Step 4: RosterFriend 加字段**（src/mcp.rs:422）

```rust
pub struct RosterFriend {
    pub wxid: String,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub avatar_url: Option<String>,
    pub sex: Option<i32>,
    pub is_non_human: bool,
}
```

- [ ] **Step 5: 两处构造点补 is_non_human**

字符串分支（src/mcp.rs:540-546，字符串形态无 type，传 None）：
```rust
                return Some(RosterFriend {
                    wxid: s.to_string(),
                    nickname: None,
                    remark: None,
                    avatar_url: None,
                    sex: None,
                    is_non_human: is_non_human_account(s, None),
                });
```

对象分支（src/mcp.rs:551 起，末尾加）：
```rust
            Some(RosterFriend {
                wxid: wxid.clone(),
                nickname: first_str(obj, &["nickName", "nickname", "NickName"]),
                remark: first_str(obj, &["remark", "Remark", "conRemark"]),
                avatar_url: first_str(
                    obj,
                    &["bigHeadImgUrl", "smallHeadImgUrl", "bigHeadImg", "smallHeadImg", "headImgUrl", "avatarUrl", "headimgurl"],
                ),
                sex: obj
                    .get("sex")
                    .and_then(|v| v.as_i64().or_else(|| v.get("low").and_then(|l| l.as_i64())))
                    .map(|n| n as i32),
                is_non_human: is_non_human_account(&wxid, obj.get("type").and_then(|v| v.as_str())),
            })
```
注：`wxid` 原是 move 进 struct，现要先给 `is_non_human_account(&wxid, ...)` 用再 move，故 struct 里改 `wxid: wxid.clone()`（或调整顺序把 is_non_human 计算提到 struct 前用局部变量）。用 `wxid.clone()` 最简单。

- [ ] **Step 6: roster API 输出 isNonHuman**（src/routes/contacts.rs:404-410 json! 块）

在 `"sex": f.sex,` 后加：
```rust
                "sex": f.sex,
                "isNonHuman": f.is_non_human,
```

- [ ] **Step 7: 跑测试确认通过 + lib 全量**

Run: `cargo test --lib is_non_human && cargo test --lib`
Expected: is_non_human 4 用例 PASS；lib 全量 ≥350 passed / 0 failed。

- [ ] **Step 8: 提交**

```bash
git add src/mcp.rs src/routes/contacts.rs
git commit -m "feat(roster): 非真人账号(系统账号白名单/type=system)标记 is_non_human + API 输出"
```

---

### Task 3: 前端 roster 缓存（store 化，进频道不重拉）

**Files:**
- Modify: `frontend/src/stores/userOpsStore.ts`（加 rosterCache 状态 + loadRoster 签名加 force）
- Modify: `frontend/src/types/index.ts`（RosterEntry 加 isNonHuman）
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`

**Interfaces:**
- Consumes: Task 2 的 API `isNonHuman`。
- Produces: store `rosterCache: Record<string, {items, syncing, fetchedAt}>`；`loadRoster(accountId, opts?: {force?: boolean})` 返回 `{items, syncing}`（有缓存且非 force 走缓存不打 API）；`RosterEntry.isNonHuman?: boolean`。

- [ ] **Step 1: RosterEntry 类型加 isNonHuman**（frontend/src/types/index.ts）

```ts
export type RosterEntry = {
  wxid: string;
  nickname?: string | null;
  remark?: string | null;
  avatarUrl?: string | null;
  sex?: number | null;
  isNonHuman?: boolean;
  agentStatus: "managed" | "normal" | "not_imported";
};
```

- [ ] **Step 2: 写失败测试**（roster.test.tsx 新增：缓存命中不重复打 API）

```tsx
  it("二次 loadRoster 命中缓存不重复请求，force 才重拉", async () => {
    const { useUserOpsStore } = await import("../../../stores/userOpsStore");
    getMock.mockResolvedValue({ items: ROSTER, syncing: false });
    // 首次拉：打 API。
    await useUserOpsStore.getState().loadRoster("accCache");
    const after1 = getMock.mock.calls.length;
    expect(after1).toBeGreaterThan(0);
    // 二次非 force：走缓存，不再打 API。
    const r2 = await useUserOpsStore.getState().loadRoster("accCache");
    expect(getMock.mock.calls.length).toBe(after1);
    expect(r2.items.length).toBe(ROSTER.length);
    // force：强制重拉。
    await useUserOpsStore.getState().loadRoster("accCache", { force: true });
    expect(getMock.mock.calls.length).toBe(after1 + 1);
  });
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx -t "命中缓存"`
Expected: FAIL —— 现 loadRoster 每次都打 API，二次调用后 calls 会 +1（不等 after1）；且 loadRoster 不接受 force 参数。

- [ ] **Step 4: store 加 rosterCache + 改 loadRoster**

`userOpsStore.ts` 状态定义处（interface UserOpsState 或对应位置）加：
```ts
  rosterCache: Record<string, { items: RosterEntry[]; syncing: boolean; fetchedAt: number }>;
```
初始值（store 初始 state 处）：`rosterCache: {},`

action 签名（interface UserOpsActions，约 111 行）改：
```ts
  loadRoster: (accountId: string, opts?: { force?: boolean }) => Promise<{ items: RosterEntry[]; syncing: boolean }>;
```

实现（约 484 行）改为：
```ts
  loadRoster: async (accountId, opts) => {
    const cached = get().rosterCache[accountId];
    if (!opts?.force && cached) {
      return { items: cached.items, syncing: cached.syncing };
    }
    const data = await api.get<{ items: RosterEntry[]; syncing?: boolean }>(
      `/api/contacts/roster?accountId=${encodeURIComponent(accountId)}`
    );
    const result = { items: data.items, syncing: data.syncing ?? false };
    // 仅就绪结果落缓存；同步中(syncing)不缓存，避免卡在同步中态、允许自动重拉覆盖。
    if (!result.syncing) {
      set((s) => ({
        rosterCache: { ...s.rosterCache, [accountId]: { ...result, fetchedAt: Date.now() } },
      }));
    }
    return result;
  },
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx -t "命中缓存"`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add frontend/src/stores/userOpsStore.ts frontend/src/types/index.ts frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "feat(roster): store 化 rosterCache 缓存(按 accountId, force 才重拉) + RosterEntry.isNonHuman"
```

---

### Task 4: 前端 RosterView 接缓存 + 非真人折叠 + 性别

**Files:**
- Modify: `frontend/src/features/user-ops/RosterView.tsx`
- Modify: `frontend/src/features/user-ops/RosterView.module.css`
- Test: `frontend/src/__tests__/features/user-ops/roster.test.tsx`

**Interfaces:**
- Consumes: Task 3 的 `loadRoster(accountId, {force})` + store `rosterCache` + `RosterEntry.isNonHuman`。

- [ ] **Step 1: 写失败测试**（roster.test.tsx 新增：非真人默认折叠、真人正常显示）

```tsx
  it("非真人账号默认折叠，真人正常显示，展开后可见", async () => {
    getMock.mockResolvedValue({
      items: [
        { wxid: "wx_real", nickname: "张三", remark: null, avatarUrl: null, sex: 1, isNonHuman: false, agentStatus: "not_imported" },
        { wxid: "fmessage", nickname: "朋友推荐消息", remark: null, avatarUrl: null, sex: 0, isNonHuman: true, agentStatus: "not_imported" },
      ],
      syncing: false,
    });
    const user = userEvent.setup();
    render(<ToastProvider><RosterView /></ToastProvider>);
    // 真人直接可见。
    expect(await screen.findByText("张三")).toBeInTheDocument();
    // 非真人默认折叠：不直接可见，但有折叠入口(含 1 个)。
    expect(screen.queryByText("朋友推荐消息")).not.toBeInTheDocument();
    expect(screen.getByText(/系统账号/)).toBeInTheDocument();
    // 展开后可见。
    await user.click(screen.getByText(/系统账号/).closest("button") as HTMLButtonElement);
    expect(await screen.findByText("朋友推荐消息")).toBeInTheDocument();
  });
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx -t "默认折叠"`
Expected: FAIL —— 现 RosterView 不区分非真人，fmessage 会直接渲染，「系统账号」折叠入口不存在。

- [ ] **Step 3: RosterView 数据源改读 store 缓存**

删掉 `const [roster, setRoster] = useState<RosterEntry[]>([]);` 和 `const [syncing, setSyncing] = useState(false);`（约 41、42 行）。改为从 store 读：
```tsx
  const rosterCache = useUserOpsStore((s) => s.rosterCache);
  const cached = rosterCache[effectiveAccountId];
  const roster = cached?.items ?? [];
  const syncing = cached?.syncing ?? false;
```
`refresh` 里删掉 `setRoster(items)` / `setSyncing(isSyncing)`（store 已在 loadRoster 内写缓存，此处不再本地 setState）。`refresh` 仍保留 loading/error 的 local state 与请求序号守卫。

- [ ] **Step 4: refresh 走缓存 / 刷新按钮 force**

`refresh` 的 useCallback 签名加 force 透传：
```tsx
  const refresh = useCallback(
    async (accountId: string, opts?: { force?: boolean }) => {
      if (!accountId) return;
      const seq = ++reqSeqRef.current;
      const isStale = () => seq !== reqSeqRef.current;
      setLoading(true);
      setError(null);
      setSelectedWxids(new Set());
      setSharedNote("");
      setPlaybookId("");
      try {
        const { syncing: isSyncing } = await loadRoster(accountId, opts);
        if (isStale()) return;
        // syncing 仍需驱动自动重拉的 effect；roster/syncing 现从 store 缓存派生，无需 setState。
        void isSyncing;
      } catch (e) {
        if (isStale()) return;
        setError(e instanceof Error ? e.message : "加载好友列表失败");
      } finally {
        if (!isStale()) setLoading(false);
      }
    },
    [loadRoster]
  );
```
挂载 effect（约 82 行）不变（`void refresh(effectiveAccountId)` —— 无 force，命中缓存则 store 不打 API）。
「刷新」按钮 onClick（约 180 行）改：`onClick={() => void refresh(effectiveAccountId, { force: true })}`。
提交后重拉（约 138 行）改：`await refresh(effectiveAccountId, { force: true });`（批量托管改了 agentStatus，需强制重拉刷新标注）。
自动重拉 effect（syncing 时每 8s，约 90 行）：`void refresh(effectiveAccountId, { force: true })`（同步中必须强制打 API 才能拿到就绪数据）。

- [ ] **Step 5: 拆分真人/非真人 + 折叠 UI**

`filtered` useMemo 后加拆分：
```tsx
  const humanRows = useMemo(() => filtered.filter((r) => !r.isNonHuman), [filtered]);
  const nonHumanRows = useMemo(() => filtered.filter((r) => r.isNonHuman), [filtered]);
  const [showNonHuman, setShowNonHuman] = useState(false);
```
分页只对真人：`const { pageRows, pageCount, safePage, setPage } = usePagedList(humanRows);`（原来传 `filtered`，改成 `humanRows`）。

在网格渲染块（真人分页网格）后、`actionBar` 前，加非真人折叠区：
```tsx
      {nonHumanRows.length > 0 && (
        <div className={styles.nonHumanSection}>
          <button type="button" className={styles.nonHumanToggle} onClick={() => setShowNonHuman((v) => !v)}>
            含 {nonHumanRows.length} 个系统账号（{showNonHuman ? "收起" : "展开"}）
          </button>
          {showNonHuman && (
            <div className={styles.grid}>
              {nonHumanRows.map((entry) => {
                const checked = selectedWxids.has(entry.wxid);
                const managed = entry.agentStatus === "managed";
                return (
                  <button
                    key={entry.wxid}
                    type="button"
                    className={`${styles.card} ${checked ? styles.cardChecked : ""} ${managed ? styles.cardManaged : ""}`}
                    onClick={() => toggle(entry)}
                    disabled={managed}
                  >
                    <div className={styles.checkbox}>{checked && <Check size={13} />}</div>
                    {entry.avatarUrl ? (
                      <img className={styles.avatar} src={entry.avatarUrl} alt="" loading="lazy" />
                    ) : (
                      <span className={styles.avatarFallback}>{initial(entry)}</span>
                    )}
                    <div className={styles.cardBody}>
                      <strong className={styles.name}>{entry.remark || entry.nickname || entry.wxid}</strong>
                      <small className={styles.sub}>{entry.wxid}</small>
                      <small className={styles.sysTag}>系统账号</small>
                    </div>
                    {statusBadge(entry.agentStatus)}
                  </button>
                );
              })}
            </div>
          )}
        </div>
      )}
```

空态判断（`filtered.length === 0`）改为 `humanRows.length === 0 && nonHumanRows.length === 0`，避免只剩系统账号时误显「暂无好友」。

- [ ] **Step 6: CSS**（RosterView.module.css 末尾）

```css
.nonHumanSection { margin-top: 16px; }
.nonHumanToggle {
  width: 100%; padding: 10px; border: 1px dashed var(--line, #e5e5ea);
  border-radius: var(--r-md, 8px); background: transparent;
  color: var(--ink-3); font-size: 13px; cursor: pointer;
}
.nonHumanToggle:hover { background: var(--fill-2, #f5f5f7); }
.sysTag { font-size: 11px; color: var(--ink-4, #b0b0b5); }
```

- [ ] **Step 7: 跑 roster 测试确认通过（含旧回归）**

Run: `cd frontend && npx vitest run src/__tests__/features/user-ops/roster.test.tsx`
Expected: 新用例 + 现有用例全 PASS。

- [ ] **Step 8: 前端契约门 + 提交**

Run: `cd frontend && npx tsc --noEmit && npx vitest run`
Expected: tsc 0 error，vitest 全绿。

```bash
git add frontend/src/features/user-ops/RosterView.tsx frontend/src/features/user-ops/RosterView.module.css frontend/src/__tests__/features/user-ops/roster.test.tsx
git commit -m "feat(roster): RosterView 接 store 缓存(进频道不重拉)+非真人默认折叠"
```

---

## Self-Review

**Spec coverage:**
- 性别解析取 .low → Task 1 ✓
- 非真人判定放后端(白名单+type) → Task 2 ✓
- 公众号不自动识别 → Task 2 测试 `public_account_not_misjudged` 钉死 ✓
- API 输出 isNonHuman → Task 2 Step 6 ✓
- 前端 RosterEntry.isNonHuman → Task 3 Step 1 ✓
- 前端非真人默认折叠 → Task 4 Step 5 ✓
- roster 缓存(force 才重拉/syncing 不缓存/session 存活) → Task 3 ✓
- 数据源从 useState 提到 store → Task 4 Step 3 ✓

**Placeholder scan:** 无 TBD/TODO；每步含实际代码或确切命令。

**Type consistency:** `is_non_human`(Rust bool) ↔ `isNonHuman`(TS bool/wire)；`sex: Option<i32>` ↔ `number|null`；`loadRoster(accountId, opts?:{force?})` 在 Task 3 定义、Task 4 消费一致；`rosterCache: Record<string,{items,syncing,fetchedAt}>` Task 3 定义、Task 4 读 `.items/.syncing` 一致。

**注意点（实现者留意）：** Task 2 对象分支 `wxid` 从 move 改 `wxid.clone()`（因 is_non_human_account 先借用）。Task 4 删组件 roster/syncing 的 useState 后，所有原读 `roster`/`syncing` 的地方（filter、空态、自动重拉 effect 依赖）改从 store 派生值读——编译器会报未使用/未定义，逐个改到编译过。
