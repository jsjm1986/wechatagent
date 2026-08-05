# 决策人链改用通讯录选择器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把「请示通道配置」页决策人链的「+ 从联系人添加」换成从微信通讯录选择，复用共享 `FriendPickerModal`，未入库好友选中后自动导入。

**Architecture:** 数据源由 `/api/contacts`（本地已入库联系人）换成 `userOpsStore.loadRoster`（通讯录快照，带缓存）。交互由内联面板换成弹窗式 `FriendPickerModal`。选中 `agentStatus === "not_imported"` 的好友时先调 `POST /api/contacts/import` 落库（该端点写 `agent_status: "normal"`，不托管），再加入链——因为后端 `put_ask_human_policy` fail-closed 要求决策人已在 `contacts` 表。

**Tech Stack:** React 19 + TypeScript + Zustand + CSS Modules + Vitest/jsdom + @testing-library/react

**设计文档:** `docs/superpowers/specs/2026-08-06-decider-chain-roster-picker-design.md`

## Global Constraints

- 分支 `fix/decider-chain-roster-picker-20260806`，已建好，**不要切回 main**。
- **纯前端改动**：不碰任何 `.rs` 文件。`/api/contacts/import` 已存在（`src/routes/mod.rs:346`），只是前端从未使用。
- 工作树里有**他人 21 项未提交改动**（含 `frontend/src/styles.css`、`features/user-ops/**`）。提交时只 `git add` 明确列出的文件，**禁止 `git add .` / `git add -A`**。
- CI 门禁 `scripts/check-no-human-takeover.sh` 扫 `frontend/src/` 新增行的禁用词（含「**人工**」）。新增注释/测试名一律用「目视确认 / 视觉核验」。
- 门禁必须**从仓库根**执行：`cd "$(git rev-parse --show-toplevel)" && bash scripts/check-no-human-takeover.sh`。从 `frontend/` 跑会因 pathspec 相对 cwd 解析而输出 `no changed files under scan dirs; ok` 并 exit 0——**假通过**。
- 色值/圆角/间距一律走 `components/ui/tokens.css` 变量，禁硬编码。
- 行号会随改动漂移，**一律按字符串定位**，不要照抄行号。

---

### Task 1: 非真人过滤纯函数

**背景（必读）：** roster 的 `isNonHuman` 判据比后端 import 守卫**宽松**，两者不等价：

```
roster.isNonHuman = item_type=="system" || is_system_account(wxid)          // src/mcp.rs:761
后端 import 守卫  = !(gh_ 前缀 || @chatroom || @openim || is_system_account)  // src/webhooks.rs:1948
```

公众号（`gh_`）、群（`@chatroom`）、企业号（`@openim`）只有在 `item_type=="system"` 时才被 roster 标 `isNonHuman`，否则会显示为可选——但 import 会**静默拒绝**它们（返回 200 但 `items: []`）。

**关键推论（避免维护陷阱）：** `isNonHuman` 已覆盖 `is_system_account(wxid)` 这一半，所以前端**只需补三条结构性规则**（`gh_` / `@chatroom` / `@openim`），**不要**把后端那份 13 条系统号白名单（`src/mcp.rs:711` 的 `WECHAT_SYSTEM_ACCOUNTS`）复制到前端——复制会产生两份清单漂移。二者叠加后与后端守卫等价。

**Files:**
- Create: `frontend/src/features/ask-human-config/deciderCandidates.ts`
- Test: `frontend/src/__tests__/features/ask-human-config/deciderCandidates.test.ts`

**Interfaces:**
- Produces: `isPickableDecider(entry: { wxid: string; isNonHuman?: boolean }): boolean` —— 供 Task 2 过滤 roster 候选。

- [ ] **Step 1: 写失败的测试**

创建 `frontend/src/__tests__/features/ask-human-config/deciderCandidates.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { isPickableDecider } from "../../../features/ask-human-config/deciderCandidates";

describe("isPickableDecider", () => {
  it("真人 wxid 可选", () => {
    expect(isPickableDecider({ wxid: "wxid_ydzaomn4scsb12" })).toBe(true);
    expect(isPickableDecider({ wxid: "wxid_8874178741811" })).toBe(true);
  });

  it("roster 已标 isNonHuman 的不可选（覆盖系统号那一半判据）", () => {
    expect(isPickableDecider({ wxid: "weixin", isNonHuman: true })).toBe(false);
    expect(isPickableDecider({ wxid: "fmessage", isNonHuman: true })).toBe(false);
  });

  it("公众号 gh_ 前缀不可选（roster 可能漏标，后端会静默拒绝）", () => {
    expect(isPickableDecider({ wxid: "gh_416c280c4978" })).toBe(false);
    expect(isPickableDecider({ wxid: "gh_416c280c4978", isNonHuman: false })).toBe(false);
  });

  it("群 @chatroom 不可选", () => {
    expect(isPickableDecider({ wxid: "7842243308@chatroom" })).toBe(false);
  });

  it("企业微信/开放 IM @openim 不可选", () => {
    expect(isPickableDecider({ wxid: "25984984932102183@openim" })).toBe(false);
  });

  it("gh 出现在中间不算公众号（只认前缀，与后端 starts_with 对齐）", () => {
    expect(isPickableDecider({ wxid: "wxid_gh_not_prefix" })).toBe(true);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/deciderCandidates.test.ts`
Expected: FAIL —— 报无法解析 `../../../features/ask-human-config/deciderCandidates`（文件尚不存在）。

- [ ] **Step 3: 写实现**

创建 `frontend/src/features/ask-human-config/deciderCandidates.ts`：

```ts
/**
 * 决策人候选过滤：与后端 import 守卫 `webhooks::is_operatable_person` 等价。
 *
 * 为何不是只看 isNonHuman：roster 的 isNonHuman 判据是
 * `item_type=="system" || is_system_account(wxid)`（src/mcp.rs:761），
 * 而后端 import 拒绝的是
 * `gh_ 前缀 || @chatroom || @openim || is_system_account`（src/webhooks.rs:1948）。
 * 公众号/群/企业号只在 item_type=="system" 时才被 roster 标记，否则漏网——
 * 用户能选中，但 import 会静默拒绝（返回 200 且 items 为空），表现为「点了没反应」。
 *
 * 为何不复制后端的系统号白名单：isNonHuman 已覆盖 is_system_account 那一半，
 * 此处只补三条结构性规则即与后端等价。复制 WECHAT_SYSTEM_ACCOUNTS（src/mcp.rs:711，13 条）
 * 会产生两份清单，后端增删时前端必然漂移。
 */
export function isPickableDecider(entry: { wxid: string; isNonHuman?: boolean }): boolean {
  if (entry.isNonHuman) return false;
  const wxid = entry.wxid;
  if (wxid.startsWith("gh_")) return false;
  if (wxid.includes("@chatroom")) return false;
  if (wxid.includes("@openim")) return false;
  return true;
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/deciderCandidates.test.ts`
Expected: PASS，6 个用例全绿。

- [ ] **Step 5: 门禁 + 提交**

Run（**必须从仓库根**）: `cd "$(git rev-parse --show-toplevel)" && bash scripts/check-no-human-takeover.sh`

```bash
git add frontend/src/features/ask-human-config/deciderCandidates.ts \
        frontend/src/__tests__/features/ask-human-config/deciderCandidates.test.ts
git commit -m "feat(ui): 决策人候选过滤与后端 import 守卫对齐

roster 的 isNonHuman 只覆盖 is_system_account 一半判据，公众号 gh_/群
@chatroom/企业号 @openim 会漏网被选中，而 import 静默拒绝（200 + 空 items），
用户看到「点了没反应」。此处补三条结构性规则使之与后端等价。

不复制后端 13 条系统号白名单：isNonHuman 已覆盖该半，复制会两份漂移。"
```

---

### Task 2: DeciderChainEditor 换用通讯录选择器

**Files:**
- Modify: `frontend/src/features/ask-human-config/DeciderChainEditor.tsx`（整体重写选择逻辑，122 行）
- Modify: `frontend/src/features/ask-human-config/AskHumanConfig.module.css`（删内联面板样式，加 badge/提示样式）
- Test: `frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`（**重写现有 6 个用例**）

**Interfaces:**
- Consumes: `isPickableDecider(entry)` from Task 1（`./deciderCandidates`）。
- Consumes: `FriendPickerModal` + `FriendPickerItem` from `../../components/ui/FriendPickerModal`（已有共享组件，140 行）。
- Consumes: `useUserOpsStore().loadRoster(accountId, opts?) => Promise<{ items: RosterEntry[]; syncing: boolean }>`、`useUserOpsStore().rosterCache`。
- Consumes: `useAccountStore().currentAccountId() => string`。
- Produces: 组件 props 签名**不变** —— `{ chain: DeciderRef[]; onChange: (next: DeciderRef[]) => void }`，故 `index.tsx:120` 的调用处无需改动。

**背景（必读，三个坑）：**

**坑 1 —— HTTP 200 不代表导入成功。** `import_contacts_endpoint`（`src/routes/contacts.rs:471`）是 `if let Some(contact) = upsert(...)`，upsert 返回 `None` 时**静默跳过**，接口仍回 200，只是 `items` 为空数组。故**必须检查 `items.length > 0`**，不能只看是否 throw。

**坑 2 —— syncing 态必须处理。** roster 首次无快照时后端返回 `items: []` + `syncing: true`，后台单飞异步拉取。`referral-cards/index.tsx:37` 只有一句 `void loadRoster(...)`，**没处理这个态**——选择器会是空的，用户以为没有好友。本任务抄 `RosterView.tsx:88-99` 的 10s 轮询自动重拉。

**坑 3 —— `allowManualWxid` 必须关闭。** `FriendPickerModal` 支持手输 wxid，但后端 `put_ask_human_policy`（`src/routes/domains.rs:237`）fail-closed 要求 `(workspace, accountId, wxid)` 存在于 contacts 表。手输一个不在通讯录的 wxid 会前端放行、保存被拒，正是要避免的体验。**不传该 prop**（默认 false）。

- [ ] **Step 1: 写失败的测试（重写整个测试文件）**

**完整覆盖** `frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`（旧的 6 个用例全部 mock `/api/contacts` 形状并断言「从联系人添加」文案，数据源与交互都变了，必须重写）：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { DeciderChainEditor } from "../../../features/ask-human-config/DeciderChainEditor";
import { api } from "../../../lib/api";
import { useUserOpsStore } from "../../../stores/userOpsStore";
import { useAccountStore } from "../../../stores/accountStore";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn(), post: vi.fn() } }));

const post = api.post as unknown as ReturnType<typeof vi.fn>;

/** 通讯录条目。agentStatus 决定选中后是否需要先导入。 */
function entry(
  wxid: string,
  nickname: string,
  agentStatus: "managed" | "normal" | "not_imported" = "normal",
  extra: Record<string, unknown> = {},
) {
  return { wxid, nickname, remark: null, avatarUrl: null, sex: 1, isNonHuman: false, agentStatus, ...extra };
}

/** 铺好 store：账号 + roster 缓存（syncing:false 表示快照已就绪）。 */
function seedRoster(items: ReturnType<typeof entry>[], syncing = false) {
  useAccountStore.setState({ accounts: [{ accountId: "acc1", wxid: "self_wx", online: true } as never], selectedAccountId: "acc1" });
  useUserOpsStore.setState({
    rosterCache: { acc1: { items: items as never, syncing, fetchedAt: Date.now() } },
  });
}

beforeEach(() => {
  post.mockReset();
  vi.spyOn(useUserOpsStore.getState(), "loadRoster").mockResolvedValue({ items: [], syncing: false } as never);
});

describe("DeciderChainEditor 通讯录选择器", () => {
  it("按钮文案是「从通讯录添加」而非「从联系人添加」", () => {
    seedRoster([entry("wxid_a", "阿伟")]);
    render(<DeciderChainEditor chain={[]} onChange={vi.fn()} />);
    expect(screen.getByText(/从通讯录添加/)).toBeTruthy();
    expect(screen.queryByText(/从联系人添加/)).toBeNull();
  });

  it("已入库好友：选中直接入链，不调 import", async () => {
    seedRoster([entry("wxid_a", "阿伟", "normal")]);
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[]} onChange={onChange} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("阿伟"));

    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith([
        { wxid: "wxid_a", displayName: "阿伟", accountId: "acc1" },
      ]),
    );
    // 已入库 → 不该有 import 写操作。
    expect(post).not.toHaveBeenCalled();
  });

  it("未入库好友：先 import 落库再入链", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    post.mockResolvedValue({ items: [{ wxid: "wxid_new" }] });
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[]} onChange={onChange} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    await waitFor(() => expect(post).toHaveBeenCalled());
    // 端点与载荷：/api/contacts/import，candidates 带 wxid/nickname/remark。
    expect(post.mock.calls[0][0]).toBe("/api/contacts/import");
    const body = post.mock.calls[0][1] as { accountId: string; candidates: { wxid: string }[] };
    expect(body.accountId).toBe("acc1");
    expect(body.candidates[0].wxid).toBe("wxid_new");
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith([
        { wxid: "wxid_new", displayName: "新朋友", accountId: "acc1" },
      ]),
    );
  });

  it("import 返回空 items（后端静默拒绝）→ 不入链并报错", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    // 坑 1：接口回 200 但 items 为空 = upsert 返回 None，导入没成功。
    post.mockResolvedValue({ items: [] });
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[]} onChange={onChange} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("import 抛错 → 不入链并显示错误", async () => {
    seedRoster([entry("wxid_new", "新朋友", "not_imported")]);
    post.mockRejectedValue(new Error("network down"));
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[]} onChange={onChange} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    fireEvent.click(await screen.findByText("新朋友"));

    expect(await screen.findByText(/network down/)).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("非真人不出现在候选：公众号 gh_ / 群 @chatroom / isNonHuman", async () => {
    seedRoster([
      entry("wxid_ok", "真人甲"),
      entry("gh_416c280c4978", "某公众号"),
      entry("7842243308@chatroom", "某群"),
      entry("weixin", "微信团队", "normal", { isNonHuman: true }),
    ]);
    render(<DeciderChainEditor chain={[]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    await screen.findByText("真人甲");
    expect(screen.queryByText("某公众号")).toBeNull();
    expect(screen.queryByText("某群")).toBeNull();
    expect(screen.queryByText("微信团队")).toBeNull();
  });

  it("已在链中的 wxid 从候选排除", async () => {
    seedRoster([entry("wxid_a", "阿伟"), entry("wxid_b", "李总")]);
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a", displayName: "链中甲" }]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    await screen.findByText("李总");
    expect(screen.queryByText("阿伟")).toBeNull();
  });

  it("不提供手动输入 wxid 入口（后端 fail-closed 会拒绝不在通讯录的 wxid）", async () => {
    seedRoster([entry("wxid_a", "阿伟")]);
    render(<DeciderChainEditor chain={[]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    await screen.findByText("阿伟");
    expect(screen.queryByText(/手动输入 wxid/)).toBeNull();
  });

  it("删除 → onChange 收到去掉该项的链", () => {
    seedRoster([]);
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a" }, { wxid: "wxid_b" }]} onChange={onChange} />);
    fireEvent.click(screen.getAllByLabelText("删除")[0]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }]);
  });

  it("上移第二项 → onChange 收到顺序交换的链", () => {
    seedRoster([]);
    const onChange = vi.fn();
    render(<DeciderChainEditor chain={[{ wxid: "wxid_a" }, { wxid: "wxid_b" }]} onChange={onChange} />);
    fireEvent.click(screen.getAllByLabelText("上移")[1]);
    expect(onChange).toHaveBeenCalledWith([{ wxid: "wxid_b" }, { wxid: "wxid_a" }]);
  });

  it("通讯录同步中显示同步态，而非空列表", async () => {
    // 坑 2：首次无快照时后端返回 items:[] + syncing:true。
    seedRoster([], true);
    vi.spyOn(useUserOpsStore.getState(), "loadRoster").mockResolvedValue({ items: [], syncing: true } as never);
    render(<DeciderChainEditor chain={[]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByText(/从通讯录添加/));
    expect(await screen.findByText(/同步中/)).toBeTruthy();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`
Expected: 多数用例 FAIL —— 「从通讯录添加」文案不存在（当前是「从联系人添加」）、`api.post` 未被调用、无 `role="alert"` 等。

- [ ] **Step 3: 重写 DeciderChainEditor.tsx**

**完整覆盖**该文件（当前 122 行，`api.get('/api/contacts')` + 内联 `pickerPanel` 全部替换）：

```tsx
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../lib/api";
import { FriendPickerModal, type FriendPickerItem } from "../../components/ui/FriendPickerModal";
import { useAccountStore } from "../../stores/accountStore";
import { useUserOpsStore } from "../../stores/userOpsStore";
import type { DeciderRef, RosterEntry } from "../../types";
import { isPickableDecider } from "./deciderCandidates";
import styles from "./AskHumanConfig.module.css";

function rosterLabel(entry: RosterEntry): string {
  return entry.remark || entry.nickname || entry.wxid;
}

export function DeciderChainEditor({
  chain,
  onChange,
}: {
  chain: DeciderRef[];
  onChange: (next: DeciderRef[]) => void;
}) {
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [loading, setLoading] = useState(false);

  const accountId = useAccountStore((s) => s.currentAccountId());
  const loadRoster = useUserOpsStore((s) => s.loadRoster);
  const rosterCache = useUserOpsStore((s) => s.rosterCache);
  const roster = rosterCache[accountId]?.items ?? [];

  // 请求序号守卫：快速切账号时并发多次 loadRoster，只有最新一次允许落地，
  // 否则先发的（账号 A）若晚于后发的（账号 B）返回，会用 A 的好友覆盖 B 的列表。
  // 抄 RosterView.tsx 的 reqSeqRef 做法。
  const reqSeqRef = useRef(0);

  const refresh = useCallback(
    async (id: string) => {
      if (!id) return;
      const seq = ++reqSeqRef.current;
      const isStale = () => seq !== reqSeqRef.current;
      setLoading(true);
      setError(null);
      try {
        const { syncing: isSyncing } = await loadRoster(id);
        if (isStale()) return;
        setSyncing(isSyncing);
      } catch (e) {
        if (isStale()) return;
        setError(e instanceof Error ? e.message : "加载通讯录失败");
      } finally {
        if (!isStale()) setLoading(false);
      }
    },
    [loadRoster],
  );

  // 仅在打开选择器时拉取，避免页面加载就打通讯录接口。
  useEffect(() => {
    if (!picking) return;
    void refresh(accountId);
  }, [picking, accountId, refresh]);

  // 快照同步中时每 10s 自动重拉（不带 force，只读快照）；后台单飞任务写好快照后
  // 普通读自然读到、syncing 变 false、轮询自停。抄 RosterView.tsx:94-99。
  useEffect(() => {
    if (!picking || !syncing || !accountId) return;
    const timer = setInterval(() => {
      void refresh(accountId);
    }, 10000);
    return () => clearInterval(timer);
  }, [picking, syncing, accountId, refresh]);

  const inChain = useMemo(() => new Set(chain.map((d) => d.wxid)), [chain]);

  // 双重过滤：isPickableDecider 与后端 import 守卫等价（见 deciderCandidates.ts），
  // 再排除已在链中的。未入库的保留在候选里并打 badge——选中时自动导入。
  const candidates: FriendPickerItem[] = useMemo(
    () =>
      roster
        .filter((entry) => isPickableDecider(entry))
        .filter((entry) => !inChain.has(entry.wxid))
        .map((entry) => ({
          wxid: entry.wxid,
          nickname: entry.nickname,
          remark: entry.remark,
          avatarUrl: entry.avatarUrl,
          sex: entry.sex,
          ...(entry.agentStatus === "not_imported" ? { badge: "未导入" } : {}),
        })),
    [roster, inChain],
  );

  async function pick(item: FriendPickerItem) {
    const entry = roster.find((r) => r.wxid === item.wxid);
    const displayName = entry ? rosterLabel(entry) : item.wxid;
    if (!accountId) {
      setError("未选择账号，无法添加决策人");
      return;
    }
    setError(null);

    // 后端 put_ask_human_policy fail-closed 要求决策人已在 contacts 表
    // （src/routes/domains.rs），故未入库的好友必须先落库再入链。
    // 用 /api/contacts/import（写 agent_status: "normal"，不托管），
    // 不用 /contacts/batch-enable——后者无条件写 "managed" 并建 enrollment intent，
    // 会把内部决策者当客户交给 AI 运营，语义不对。
    if (entry?.agentStatus === "not_imported") {
      setImporting(true);
      try {
        const res = await api.post<{ items: unknown[] }>("/api/contacts/import", {
          accountId,
          candidates: [
            {
              wxid: entry.wxid,
              ...(entry.nickname ? { nickname: entry.nickname } : {}),
              ...(entry.remark ? { remark: entry.remark } : {}),
            },
          ],
        });
        // 坑：接口回 200 不代表导入成功——upsert 返回 None 时 handler 静默跳过
        // （src/routes/contacts.rs 的 `if let Some(contact)`），items 为空。
        // 只看是否 throw 会把静默失败当成功，随后保存时才被后端拒绝。
        if (!Array.isArray(res.items) || res.items.length === 0) {
          setError(`「${displayName}」未能导入通讯录（可能被识别为非真人账号），请换一位或先到「账号管理」同步通讯录`);
          return;
        }
        // 导入成功：刷新快照，让该好友的 agentStatus 从 not_imported 变为 normal。
        void loadRoster(accountId, { force: true });
      } catch (e) {
        setError(e instanceof Error ? e.message : "导入通讯录失败");
        return;
      } finally {
        setImporting(false);
      }
    }

    onChange([...chain, { wxid: item.wxid, displayName, accountId }]);
    setPicking(false);
  }

  function remove(idx: number) {
    onChange(chain.filter((_, i) => i !== idx));
  }
  function move(idx: number, dir: -1 | 1) {
    const j = idx + dir;
    if (j < 0 || j >= chain.length) return;
    const next = [...chain];
    [next[idx], next[j]] = [next[j], next[idx]];
    onChange(next);
  }

  return (
    <div className={styles.chainEditor}>
      {chain.length === 0 && <div className={styles.chainEmpty}>尚未配置决策人</div>}
      {chain.map((d, idx) => (
        <div key={d.wxid} className={styles.chainRow}>
          <span className={styles.chainName} title={d.wxid}>
            {d.displayName ?? d.wxid}
            <span className={styles.chainWxid}>{d.accountId ? `账号 ${d.accountId}` : "未绑定账号"}</span>
          </span>
          <div className={styles.chainActions}>
            <button type="button" aria-label="上移" disabled={idx === 0} onClick={() => move(idx, -1)}>↑</button>
            <button type="button" aria-label="下移" disabled={idx === chain.length - 1} onClick={() => move(idx, 1)}>↓</button>
            <button type="button" aria-label="删除" onClick={() => remove(idx)}>✕</button>
          </div>
        </div>
      ))}
      <div className={styles.chainHint}>超时未响应时，按此顺序转交链中下一位</div>

      {error && <div className={styles.loadError} role="alert">{error}</div>}

      <button
        type="button"
        className={styles.linkBtn}
        disabled={importing}
        onClick={() => setPicking(true)}
      >
        {importing ? "导入中…" : "+ 从通讯录添加"}
      </button>

      <FriendPickerModal
        open={picking}
        items={candidates}
        onSelect={(item) => void pick(item)}
        onClose={() => setPicking(false)}
        title="选择决策人"
        loading={loading && roster.length === 0 && !syncing}
        emptyText={
          syncing
            ? "通讯录同步中，稍等几秒会自动出现…"
            : "该账号通讯录为空。请先到「账号管理」同步通讯录。"
        }
      />
    </div>
  );
}
```

注意四点：

1. **不传 `allowManualWxid`**（默认 false）——后端 fail-closed 会拒绝不在通讯录的 wxid，给入口等于给用户挖坑。
2. `syncing` 态经 `emptyText` 表达，因为 `FriendPickerModal` 在 `items` 为空时渲染 `emptyText`（`FriendPickerModal.tsx:83`）。
3. 组件 props 签名未变，`index.tsx:120` 的调用处不用改。
4. `accountId` 来自 `accountStore.currentAccountId()`，不再从 `Contact.accountId` 摸——roster 本就按账号拉取，两者天然一致。

- [ ] **Step 4: 加 CSS（badge 与同步态提示）**

打开 `frontend/src/features/ask-human-config/AskHumanConfig.module.css`。

**4a.** 删掉不再使用的内联面板样式（按字符串定位，`.pickerPanel` / `.pickerList` / `.pickerItem` 三条规则及其 `:hover`）：

```css
.pickerPanel { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid var(--hairline); border-radius: var(--r-md); background: var(--surface-card); }
.pickerList { display: flex; flex-direction: column; gap: 4px; max-height: 220px; overflow-y: auto; }
.pickerItem {
  display: flex; align-items: center; justify-content: space-between; gap: 8px;
  padding: 8px 10px; border: 1px solid var(--hairline); border-radius: var(--r-sm);
  background: var(--surface-card); color: var(--ink-1); cursor: pointer; font-size: 13px; text-align: left;
}
.pickerItem:hover { background: var(--fill-scheduled); }
```

**4b.** `.input` 规则也随之无用（内联面板的搜索框，弹窗自带搜索），一并删掉：

```css
.input {
  width: 100%; height: var(--control-h, 38px); padding: 0 12px;
  border: 1px solid var(--hairline); border-radius: var(--r-sm); background: var(--surface-card);
  color: var(--ink-1); font-size: 13px;
}
.input:focus { outline: none; box-shadow: var(--focus-ring); }
```

**4c.** 给 `.linkBtn` 补 disabled 态（导入中会禁用，当前无该样式，禁用后视觉无变化会让人以为没反应）。在 `.linkBtn` 规则**之后**追加：

```css
.linkBtn:disabled { opacity: .5; cursor: not-allowed; }
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx`
Expected: PASS，11 个用例全绿。

- [ ] **Step 6: 确认删掉的 CSS 类无人引用**

Run: `cd frontend && grep -rn "pickerPanel\|pickerList\|pickerItem\|styles.input" src/features/ask-human-config/ ; echo "exit=$?"`
Expected: 无输出（`exit=1`）。若有命中说明还有引用未清，需一并处理。

- [ ] **Step 7: 全量测试 + 构建**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 全绿、`✓ built`、tsc 无报错。

> 若 `EvolutionCenterTab.test.tsx` 偶发失败，单独重跑该文件确认——这是预先存在的 flake，与本改动无关。

- [ ] **Step 8: 门禁 + 提交**

Run（**必须从仓库根**）: `cd "$(git rev-parse --show-toplevel)" && bash scripts/check-no-human-takeover.sh`
Expected: `ok: N violations` 中 N=0，且**不是** `no changed files under scan dirs`（那是 cwd 错的假通过）。

```bash
git add frontend/src/features/ask-human-config/DeciderChainEditor.tsx \
        frontend/src/features/ask-human-config/AskHumanConfig.module.css \
        frontend/src/__tests__/features/ask-human-config/DeciderChainEditor.test.tsx
git commit -m "feat(ui): 决策人链改从通讯录选择，复用 FriendPickerModal

原「从联系人添加」拉的是 /api/contacts（本地已入库联系人），名不副实：
用户想加的人若未入库就根本搜不到。改用通讯录快照（loadRoster），并复用
名片库同款 FriendPickerModal（头像网格 / 三字段搜索 / 60 条分页）。

未入库好友选中后先经 /api/contacts/import 落库再入链——后端
put_ask_human_policy fail-closed 要求决策人已在 contacts 表。用 import
而非 batch-enable：后者无条件写 agent_status:managed 并建 enrollment
intent，会把内部决策者当客户交给 AI 运营。

三处易踩的坑已处理：
- import 回 200 不代表成功（upsert 返 None 时静默跳过），故校验 items 非空
- roster 首次无快照返回 syncing:true，补 10s 轮询自动重拉（referral-cards 缺这层）
- 不提供手动输 wxid 入口，否则前端放行、保存被后端拒

重写测试 11 个用例（旧 6 个 mock 的是 /api/contacts 形状与旧文案）。"
```

---

## 收尾

- [ ] **Step 1: 推送并建 PR**

```bash
git push -u origin fix/decider-chain-roster-picker-20260806
gh pr create --base main --title "fix(ui): 决策人链改从通讯录选择" --body "$(cat <<'EOF'
## 摘要

「请示通道配置」页决策人链的「+ 从联系人添加」名不副实——它拉的是 `/api/contacts`（本地**已入库**联系人），不是微信通讯录。用户想加的决策人若尚未入库就根本搜不到。本 PR 改为从通讯录选择，并复用「专属顾问名片库」同款 `FriendPickerModal`。

设计文档：`docs/superpowers/specs/2026-08-06-decider-chain-roster-picker-design.md`

## 改动

| 项 | 改动前 | 改动后 |
| --- | --- | --- |
| 数据源 | `/api/contacts?limit=100`（已入库联系人） | `loadRoster` 通讯录快照（带缓存，跨挂载存活） |
| 交互 | 内联面板 + 纯文本列表 | 弹窗 `FriendPickerModal`：头像网格、三字段搜索、60 条分页 |
| 未入库好友 | 搜不到 | 显示并打「未导入」badge，选中后自动导入 |
| `accountId` | 从 `Contact.accountId` 摸，缺失则报错 | `accountStore.currentAccountId()` |
| 按钮文案 | 从联系人添加 | 从通讯录添加 |

## 为何用 `/api/contacts/import` 而非 `batch-enable`

后端 `put_ask_human_policy` 是 fail-closed 的：链中每人的 `accountId` 必须属于当前 workspace，且 `(workspace, accountId, wxid)` 必须存在于 `contacts` 表。所以未入库的好友必须先落库。

`/api/contacts/import` → `upsert_contact_from_value` 的 `$setOnInsert` 写 `agent_status: "normal"`，只补身份字段，不托管、不建任务、不进运营池。而 `/contacts/batch-enable` 无条件写 `agent_status: "managed"` 并建 enrollment intent——会把内部决策者当客户交给 AI 运营，语义不对。

该端点已存在但前端从未使用，**故本 PR 是纯前端改动，未碰任何 `.rs` 文件**。

## 三处易踩的坑

**HTTP 200 不代表导入成功。** `import_contacts_endpoint` 是 `if let Some(contact) = upsert(...)`，upsert 返回 `None` 时静默跳过，接口仍回 200 但 `items` 为空。只看是否 throw 会把静默失败当成功，用户直到点保存才被后端拒绝。故校验 `items.length > 0`。

**roster 的非真人判据比后端宽松。** roster 的 `isNonHuman` 是 `item_type=="system" || is_system_account(wxid)`，而后端 import 拒绝 `gh_ 前缀 || @chatroom || @openim || is_system_account`。公众号/群/企业号会漏标、显示为可选，但 import 静默拒绝，表现为「点了没反应」。新增 `isPickableDecider` 补三条结构性规则使之等价——**不复制**后端 13 条系统号白名单，因为 `isNonHuman` 已覆盖那一半，复制会两份漂移。

**syncing 态。** 通讯录首次无快照时后端返回 `items: []` + `syncing: true`，后台异步拉取。抄 `RosterView` 的 10s 轮询自动重拉与请求序号守卫（防快速切账号时旧响应覆盖新列表）——`referral-cards` 缺这层，选择器会显示为空。

## 测试

`deciderCandidates.test.ts` 6 个用例（过滤边界，含「gh 在中间不算公众号」与后端 `starts_with` 对齐）。

`DeciderChainEditor.test.tsx` 重写为 11 个用例：文案、已入库直接入链且不调 import、未入库先 import 再入链并校验载荷、import 返空 items 不入链并报错、import 抛错不入链、三类非真人不出现在候选、已在链中的排除、无手动输入入口、删除、上移、同步中态。

旧的 6 个用例全部 mock `/api/contacts` 形状并断言旧文案，数据源与交互都变了，故重写而非增补。

**测试边界**：jsdom 无布局引擎也不跑 CSS 层叠，弹窗视觉、头像网格、分页观感均**无法断言**，需目视确认。

## 影响面

改动限于 `features/ask-human-config/` 目录内（新增 1 个纯函数文件 + 重写 1 个组件 + 删 4 条无用 CSS 规则）。`DeciderChainEditor` 的 props 签名未变，`index.tsx` 调用处无需改动。`FriendPickerModal` 与 `userOpsStore` 均只读复用，未修改。

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 2: 等 CI**

Run: `gh pr checks --watch`
Expected: 「前端契约对账 (tsc + vitest)」通过。后端门禁会因 `dorny/paths-filter` 显示 skipping——本 PR 只碰 `frontend/src/` 与 `docs/`，属预期。

- [ ] **Step 3: 目视核验清单（jsdom 断不了）**

`cd frontend && npm run dev` → 「请示通道配置」频道：

| 项 | 期望 |
| --- | --- |
| 按钮文案 | 「+ 从通讯录添加」 |
| 弹窗 | 点击后弹出，头像 + 昵称 + wxid 卡片网格，与「专属顾问名片库 → 从好友选择」观感一致 |
| 搜索 | 输昵称/备注/wxid 均能过滤 |
| 分页 | 通讯录超 60 人时出现上一页/下一页 |
| 未导入 badge | 未入库好友卡片上有「未导入」标记 |
| 自动导入 | 选中未导入的好友 → 按钮短暂显示「导入中…」→ 成功入链 |
| 非真人 | 公众号、群不出现在候选 |
| 无手动入口 | 弹窗底部**没有**「找不到？手动输入 wxid」 |
| 保存 | 加完决策人点「保存」成功，刷新页面后配置仍在 |
| 同步中 | 首次打开若通讯录未同步，显示「通讯录同步中，稍等几秒会自动出现…」并在几秒后自动出现好友 |

