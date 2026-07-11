# 好友选择器弹窗 FriendPickerModal 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把手填微信 wxid 的痛点换成"点按钮 → 弹窗头像网格 + 搜索 → 点选回填"——新建共享 `FriendPickerModal` 单选弹窗，专属顾问名片（roster 全好友源）+ products-deals ContactPicker（运营池源）两处接入。

**Architecture:** 纯前端。`FriendPickerModal` 是 UI-only 受控组件，包裹现有 `Overlay`（scrim/focus-trap/Esc/portal 已封装），数据由调用方传入统一 `FriendPickerItem[]` 形态。调用方各自把 `RosterEntry`/`Contact` map 成该形态并管 open/close。后端零改动。

**Tech Stack:** React 19 + TypeScript + Zustand + Vite + CSS Modules；vitest + @testing-library/react；Python Playwright（收尾）。

## Global Constraints

- 后端零改动：不碰 `src/routes/referral_cards.rs` / roster 端点 / `target_wxid` 校验。
- 决策链 `DeciderChainEditor` 不改（多选累加交互不同）。
- 单选：`FriendPickerModal` 无 multi 模式。
- 遵守 `docs/frontend-design-system.md`：不引入新颜色（用现有 CSS token）；卡片观感参照 RosterView。
- 复用现有 `Overlay`（`frontend/src/components/ui/Overlay/Overlay.tsx`）作弹窗底座，不手写 scrim/focus-trap。
- `cd frontend && npm run build` 成功（tsc 无错）。
- `cd frontend && npx vitest run` 全绿（含新增契约测试）。
- `scripts/check-no-human-takeover.sh` 0 violations——新增文案"从好友选择/重选/手动输入 wxid"等不含 `人工/接管/takeover/hand-off`。
- 红线：改任何一行前先 100% 读懂相关代码，引用必亲验 file:line，绝不猜测。

**已亲验关键 file:line（实现据此）：**
- `Overlay`（`frontend/src/components/ui/Overlay/Overlay.tsx`）：`{ open, onClose, labelledBy?, describedBy?, children, closeOnScrim? }`，已含 scrim/focus-trap/Esc/portal/body overflow lock。ConfirmDialog/FormDialog 都基于它。
- RosterView 过滤逻辑（`frontend/src/features/user-ops/RosterView.tsx:100-106`）：按 `remark/nickname/wxid` toLowerCase includes。
- RosterView 分页 hook（`RosterView.tsx:11-17` `usePagedList`，每页 `ROSTER_PAGE_SIZE=60`）。
- RosterView 卡片观感（`RosterView.module.css` `.grid/.card/.avatar/.avatarFallback/.name/.sub`）——参照独立写，不 import。
- `RosterEntry` 类型（`frontend/src/types/index.ts:136`）：`{ wxid, nickname?, remark?, avatarUrl?, sex?, isNonHuman?, agentStatus }`。
- `Contact` 类型（`frontend/src/types/index.ts`）：`{ id, accountId, wxid, nickname?, remark?, ... }`。
- `useUserOpsStore.loadRoster`（`frontend/src/stores/userOpsStore.ts:117,475-489`）：`(accountId, opts?) => Promise<{items: RosterEntry[]; syncing}>`；缓存 `rosterCache[accountId].items`（:77）。
- `useAccountStore.currentAccountId`（`frontend/src/stores/accountStore.ts:11,24`）：`() => string`。
- `ReferralCardDraft`（`types/index.ts`）：`{ displayName, targetWxid, sendTriggerHint, targetStages, tags }`（全 string）。
- `referralCardStore`（`frontend/src/stores/referralCardStore.ts`）：`cardDraft`（:16）、`setCardDraft(draft)`（:32）。
- referral-cards 顾问微信号裸文本框（`frontend/src/features/referral-cards/index.tsx:104-112`）；顾问名称框（:95-103）；保存 disabled（:143 `!displayName.trim() || !targetWxid.trim()`）。
- products-deals `ContactPicker`（`frontend/src/features/products-deals/index.tsx:351-414`）：props `{ selected: Contact|null, onSelect: (c: Contact|null)=>void }`；`/api/contacts` 拉取（:362-374）；内联搜索框+列表（:385-412）。父用两次（:542 deal-events、:748 holdings），`const [selected,setSelected]=useState<Contact|null>(null)`（:439），deal POST 用 `selected.id`（:529）。
- 现有测试目录：`frontend/src/__tests__/features/products-deals/SuspectedDeals.test.tsx`。
- api client：`frontend/src/lib/api.ts` `api.get`。

---

### Task 1: `FriendPickerModal` 组件 + 契约测试

**Files:**
- Create: `frontend/src/components/ui/FriendPickerModal/FriendPickerModal.tsx`
- Create: `frontend/src/components/ui/FriendPickerModal/FriendPickerModal.module.css`
- Create: `frontend/src/components/ui/FriendPickerModal/index.ts`
- Test: `frontend/src/__tests__/components/FriendPickerModal.test.tsx`

**Interfaces:**
- Consumes: `Overlay`（`../Overlay/Overlay` 的 `{open,onClose,children,labelledBy?}`）。
- Produces:
  ```ts
  export type FriendPickerItem = {
    wxid: string;
    nickname?: string | null;
    remark?: string | null;
    avatarUrl?: string | null;
    sex?: number | null;
    badge?: string;
  };
  export function FriendPickerModal(props: {
    open: boolean;
    items: FriendPickerItem[];
    onSelect: (item: FriendPickerItem) => void;
    onClose: () => void;
    title?: string;
    loading?: boolean;
    error?: string | null;
    allowManualWxid?: boolean;
    onManualWxid?: (wxid: string) => void;
  }): JSX.Element | null;
  ```

- [ ] **Step 1: 写失败测试**

创建 `frontend/src/__tests__/components/FriendPickerModal.test.tsx`：

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { FriendPickerModal, type FriendPickerItem } from "../../components/ui/FriendPickerModal";

const items: FriendPickerItem[] = [
  { wxid: "wxid_a", nickname: "张三", remark: "老张" },
  { wxid: "wxid_b", nickname: "李四", avatarUrl: "http://x/y.png" },
  { wxid: "wxid_zhang_media", nickname: "某广播" },
];

const baseProps = {
  open: true,
  items,
  onSelect: vi.fn(),
  onClose: vi.fn(),
};

describe("FriendPickerModal", () => {
  it("open=false 不渲染内容", () => {
    render(<FriendPickerModal {...baseProps} open={false} />);
    expect(screen.queryByText("张三")).toBeNull();
  });

  it("渲染所有好友卡片(名字取 remark||nickname||wxid)", () => {
    render(<FriendPickerModal {...baseProps} />);
    expect(screen.getByText("老张")).toBeInTheDocument(); // remark 优先
    expect(screen.getByText("李四")).toBeInTheDocument();
  });

  it("搜索框按 nickname/remark/wxid 过滤", () => {
    render(<FriendPickerModal {...baseProps} />);
    fireEvent.change(screen.getByPlaceholderText(/搜索/), { target: { value: "李四" } });
    expect(screen.getByText("李四")).toBeInTheDocument();
    expect(screen.queryByText("老张")).toBeNull();
  });

  it("点选卡片触发 onSelect 一次(对应 item)", () => {
    const onSelect = vi.fn();
    render(<FriendPickerModal {...baseProps} onSelect={onSelect} />);
    fireEvent.click(screen.getByText("李四"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect).toHaveBeenCalledWith(items[1]);
  });

  it("loading 显加载态", () => {
    render(<FriendPickerModal {...baseProps} items={[]} loading />);
    expect(screen.getByText(/加载中/)).toBeInTheDocument();
  });

  it("error 显错误态", () => {
    render(<FriendPickerModal {...baseProps} items={[]} error="拉取失败" />);
    expect(screen.getByText(/拉取失败/)).toBeInTheDocument();
  });

  it("空 items 显空态", () => {
    render(<FriendPickerModal {...baseProps} items={[]} />);
    expect(screen.getByText(/暂无好友|没有匹配/)).toBeInTheDocument();
  });

  it("allowManualWxid=true 时有手动输入入口,提交调 onManualWxid", () => {
    const onManualWxid = vi.fn();
    render(<FriendPickerModal {...baseProps} allowManualWxid onManualWxid={onManualWxid} />);
    fireEvent.click(screen.getByText(/手动输入/));
    fireEvent.change(screen.getByPlaceholderText(/输入.*wxid|微信/i), { target: { value: "wxid_manual" } });
    fireEvent.click(screen.getByText(/确认/));
    expect(onManualWxid).toHaveBeenCalledWith("wxid_manual");
  });

  it("allowManualWxid 默认 false 时无手动输入入口", () => {
    render(<FriendPickerModal {...baseProps} />);
    expect(screen.queryByText(/手动输入/)).toBeNull();
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run FriendPickerModal`
Expected: FAIL —— 模块不存在 / 组件未定义。

- [ ] **Step 3: 写 CSS module**

创建 `frontend/src/components/ui/FriendPickerModal/FriendPickerModal.module.css`（观感参照 RosterView.module.css，用现有 token，不引入新颜色）：

```css
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
.title { font-size: 15px; font-weight: 600; color: var(--ink); }
.closeBtn {
  border: 0; background: transparent; color: var(--muted);
  font-size: 18px; line-height: 1; cursor: pointer; padding: 4px;
}
.search { width: 100%; margin-bottom: 12px; }
.searchInput {
  width: 100%; box-sizing: border-box;
  border: 1px solid var(--line); border-radius: 8px;
  padding: 8px 12px; font-size: 13px;
}
.grid {
  display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 8px; max-height: 420px; overflow-y: auto;
}
.card {
  display: flex; align-items: center; gap: 10px;
  border: 1px solid var(--line); border-radius: 10px;
  padding: 8px 10px; background: var(--surface, #fff);
  cursor: pointer; text-align: left; width: 100%;
}
.card:hover { background: var(--surface-soft); }
.avatar { width: 36px; height: 36px; border-radius: 8px; object-fit: cover; flex: none; }
.avatarFallback {
  width: 36px; height: 36px; border-radius: 8px; flex: none;
  display: flex; align-items: center; justify-content: center;
  background: var(--surface-soft); color: var(--muted); font-weight: 600;
}
.cardBody { display: flex; flex-direction: column; min-width: 0; }
.name { font-size: 13px; color: var(--ink); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sub { font-size: 11px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.badge { margin-left: auto; font-size: 11px; color: var(--muted); flex: none; }
.state { padding: 32px 12px; text-align: center; color: var(--muted); font-size: 13px; }
.pager { display: flex; align-items: center; justify-content: center; gap: 12px; margin-top: 12px; }
.pagerBtn { border: 1px solid var(--line); background: transparent; border-radius: 6px; padding: 4px 10px; font-size: 12px; cursor: pointer; }
.pagerBtn:disabled { opacity: 0.5; cursor: default; }
.manual { margin-top: 12px; border-top: 1px solid var(--line); padding-top: 10px; }
.manualToggle { border: 0; background: transparent; color: var(--muted); font-size: 12px; cursor: pointer; padding: 0; }
.manualRow { display: flex; gap: 8px; margin-top: 8px; }
.manualInput { flex: 1; border: 1px solid var(--line); border-radius: 8px; padding: 6px 10px; font-size: 13px; }
.manualBtn { border: 1px solid var(--line); background: var(--surface-soft); border-radius: 8px; padding: 6px 12px; font-size: 12px; cursor: pointer; }
```

- [ ] **Step 4: 写组件实现**

创建 `frontend/src/components/ui/FriendPickerModal/FriendPickerModal.tsx`：

```tsx
import { useMemo, useState } from "react";
import { Overlay } from "../Overlay/Overlay";
import styles from "./FriendPickerModal.module.css";

export type FriendPickerItem = {
  wxid: string;
  nickname?: string | null;
  remark?: string | null;
  avatarUrl?: string | null;
  sex?: number | null;
  badge?: string;
};

const PAGE_SIZE = 60;

function label(item: FriendPickerItem): string {
  return item.remark || item.nickname || item.wxid;
}

export function FriendPickerModal({
  open,
  items,
  onSelect,
  onClose,
  title = "选择好友",
  loading = false,
  error = null,
  allowManualWxid = false,
  onManualWxid,
}: {
  open: boolean;
  items: FriendPickerItem[];
  onSelect: (item: FriendPickerItem) => void;
  onClose: () => void;
  title?: string;
  loading?: boolean;
  error?: string | null;
  allowManualWxid?: boolean;
  onManualWxid?: (wxid: string) => void;
}) {
  const [q, setQ] = useState("");
  const [page, setPage] = useState(0);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualWxid, setManualWxid] = useState("");

  const filtered = useMemo(() => {
    const query = q.trim().toLowerCase();
    if (!query) return items;
    return items.filter((it) =>
      [it.remark, it.nickname, it.wxid].some((v) => v?.toLowerCase().includes(query))
    );
  }, [items, q]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const pageRows = filtered.slice(safePage * PAGE_SIZE, safePage * PAGE_SIZE + PAGE_SIZE);

  if (!open) return null;

  return (
    <Overlay open={open} onClose={onClose} labelledBy="friendPickerTitle">
      <div className={styles.head}>
        <span className={styles.title} id="friendPickerTitle">{title}</span>
        <button type="button" className={styles.closeBtn} aria-label="关闭" onClick={onClose}>×</button>
      </div>

      <div className={styles.search}>
        <input
          className={styles.searchInput}
          placeholder="搜索好友（昵称/备注/wxid）"
          value={q}
          onChange={(e) => { setQ(e.target.value); setPage(0); }}
        />
      </div>

      {loading ? (
        <div className={styles.state}>加载中…</div>
      ) : error ? (
        <div className={styles.state} role="alert">加载失败：{error}</div>
      ) : filtered.length === 0 ? (
        <div className={styles.state}>{items.length === 0 ? "暂无好友" : "没有匹配的好友，换个关键词试试"}</div>
      ) : (
        <>
          <div className={styles.grid}>
            {pageRows.map((it) => (
              <button key={it.wxid} type="button" className={styles.card} onClick={() => onSelect(it)}>
                {it.avatarUrl ? (
                  <img className={styles.avatar} src={it.avatarUrl} alt="" loading="lazy" />
                ) : (
                  <span className={styles.avatarFallback}>{label(it).trim().charAt(0).toUpperCase()}</span>
                )}
                <span className={styles.cardBody}>
                  <span className={styles.name}>{label(it)}</span>
                  <span className={styles.sub}>{it.wxid}</span>
                </span>
                {it.badge && <span className={styles.badge}>{it.badge}</span>}
              </button>
            ))}
          </div>
          {pageCount > 1 && (
            <div className={styles.pager}>
              <button type="button" className={styles.pagerBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
              <span>{safePage + 1} / {pageCount}</span>
              <button type="button" className={styles.pagerBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
            </div>
          )}
        </>
      )}

      {allowManualWxid && (
        <div className={styles.manual}>
          {manualOpen ? (
            <div className={styles.manualRow}>
              <input
                className={styles.manualInput}
                placeholder="输入好友微信 wxid"
                value={manualWxid}
                onChange={(e) => setManualWxid(e.target.value)}
              />
              <button
                type="button"
                className={styles.manualBtn}
                disabled={!manualWxid.trim()}
                onClick={() => { onManualWxid?.(manualWxid.trim()); setManualWxid(""); setManualOpen(false); }}
              >
                确认
              </button>
            </div>
          ) : (
            <button type="button" className={styles.manualToggle} onClick={() => setManualOpen(true)}>
              找不到？手动输入 wxid
            </button>
          )}
        </div>
      )}
    </Overlay>
  );
}
```

- [ ] **Step 5: 写 index barrel**

创建 `frontend/src/components/ui/FriendPickerModal/index.ts`：

```ts
export { FriendPickerModal } from "./FriendPickerModal";
export type { FriendPickerItem } from "./FriendPickerModal";
```

- [ ] **Step 6: 运行确认通过**

Run: `cd frontend && npx vitest run FriendPickerModal`
Expected: PASS（9 tests）

- [ ] **Step 7: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 errors

- [ ] **Step 8: Commit**

```bash
git add frontend/src/components/ui/FriendPickerModal/ frontend/src/__tests__/components/FriendPickerModal.test.tsx
git commit -m "feat(ui): 好友选择器弹窗FriendPickerModal(包裹Overlay+头像网格+搜索+手填兜底)"
```

---

### Task 2: 专属顾问接入 FriendPickerModal

**Files:**
- Modify: `frontend/src/features/referral-cards/index.tsx`（顾问微信号栏 :104-112 + 导入 + roster 加载）
- Test: `frontend/src/__tests__/features/referral-cards/ReferralCards.test.tsx`（新建）

**Interfaces:**
- Consumes: `FriendPickerModal` + `FriendPickerItem`（Task 1）；`useUserOpsStore.loadRoster`；`useAccountStore.currentAccountId`；`referralCardStore.setCardDraft`。
- Produces: 无（终端接入）。

- [ ] **Step 1: 亲验 referral-cards 现状 + 写失败测试**

先 Read `frontend/src/features/referral-cards/index.tsx` 确认 :104-112 顾问微信号栏、:95-103 顾问名称栏、`useReferralCardStore` 解构。创建 `frontend/src/__tests__/features/referral-cards/ReferralCards.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import ReferralCardsFeature from "../../../features/referral-cards";
import { useReferralCardStore } from "../../../stores/referralCardStore";
import { useUserOpsStore } from "../../../stores/userOpsStore";

// roster 数据 stub：loadRoster 直接回一个含好友的结果，rosterCache 落地。
beforeEach(() => {
  useReferralCardStore.setState({
    cards: [],
    cardDraft: { displayName: "", targetWxid: "", sendTriggerHint: "", targetStages: "", tags: "" },
  } as any);
  useUserOpsStore.setState({
    rosterCache: { "": { items: [{ wxid: "wxid_adv", nickname: "王顾问", remark: null, avatarUrl: null, sex: 1, agentStatus: "not_imported" }], syncing: false, fetchedAt: Date.now() } },
    loadRoster: vi.fn().mockResolvedValue({ items: [{ wxid: "wxid_adv", nickname: "王顾问", agentStatus: "not_imported" }], syncing: false }),
  } as any);
});

describe("ReferralCards 顾问选择器", () => {
  it("点「从好友选择」打开弹窗,选好友后回填 wxid 且名称为空时联动回填", async () => {
    render(<ReferralCardsFeature />);
    fireEvent.click(screen.getByText(/从好友选择/));
    // 弹窗出现好友
    const friend = await screen.findByText("王顾问");
    fireEvent.click(friend);
    // 回填后:已选展示 wxid + 名称联动
    await waitFor(() => {
      expect(useReferralCardStore.getState().cardDraft.targetWxid).toBe("wxid_adv");
      expect(useReferralCardStore.getState().cardDraft.displayName).toBe("王顾问");
    });
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run ReferralCards`
Expected: FAIL —— 无"从好友选择"按钮（现状是裸文本框）。

- [ ] **Step 3: 改 referral-cards 顾问微信号栏**

在 `frontend/src/features/referral-cards/index.tsx` 顶部加导入：

```tsx
import { useState } from "react";
import { FriendPickerModal, type FriendPickerItem } from "../../components/ui/FriendPickerModal";
import { useUserOpsStore } from "../../stores/userOpsStore";
```

在 `ReferralCardsFeature` 组件体内（`const currentAccountId = ...` 附近）加 roster 加载 + 弹窗 state：

```tsx
  const loadRoster = useUserOpsStore((s) => s.loadRoster);
  const rosterCache = useUserOpsStore((s) => s.rosterCache);
  const [pickerOpen, setPickerOpen] = useState(false);

  useEffect(() => {
    if (currentAccountId) void loadRoster(currentAccountId);
  }, [currentAccountId, loadRoster]);

  const rosterItems: FriendPickerItem[] = (rosterCache[currentAccountId]?.items ?? []).map((r) => ({
    wxid: r.wxid,
    nickname: r.nickname,
    remark: r.remark,
    avatarUrl: r.avatarUrl,
    sex: r.sex,
  }));

  const pickFriend = (item: FriendPickerItem) => {
    setCardDraft({
      ...cardDraft,
      targetWxid: item.wxid,
      displayName: cardDraft.displayName.trim() ? cardDraft.displayName : (item.remark || item.nickname || ""),
    });
    setPickerOpen(false);
  };
```

把顾问微信号栏（:104-112 的 `<label>...顾问微信号...<input value={cardDraft.targetWxid}/></label>`）替换为：

```tsx
            <label className={styles.field}>
              <span className={styles.fieldLabel}>顾问微信号</span>
              {cardDraft.targetWxid ? (
                <div className={styles.pickedRow}>
                  <span className={styles.pickedWxid}>{cardDraft.targetWxid}</span>
                  <button type="button" className={styles.repickBtn} onClick={() => setPickerOpen(true)}>重选</button>
                </div>
              ) : (
                <button type="button" className={styles.pickBtn} onClick={() => setPickerOpen(true)}>
                  从好友选择
                </button>
              )}
            </label>
```

在组件 return 的最外层 `<div className={styles.page}>` 末尾（`</div>` 前）挂弹窗：

```tsx
      <FriendPickerModal
        open={pickerOpen}
        items={rosterItems}
        onSelect={pickFriend}
        onClose={() => setPickerOpen(false)}
        title="选择专属顾问"
        allowManualWxid
        onManualWxid={(wxid) => { setCardDraft({ ...cardDraft, targetWxid: wxid }); setPickerOpen(false); }}
      />
```

确认 `useEffect` 已在导入列表（文件首行已 `import { useEffect } from "react"`，:1）——若只 import useEffect 则补 useState。

- [ ] **Step 4: 加样式**

在 `frontend/src/features/referral-cards/ReferralCards.module.css` 末尾加：

```css
.pickBtn {
  border: 1px dashed var(--line); background: var(--surface-soft);
  border-radius: 8px; padding: 9px 12px; font-size: 13px; color: var(--ink-soft);
  cursor: pointer; text-align: left;
}
.pickBtn:hover { color: var(--ink); }
.pickedRow { display: flex; align-items: center; gap: 10px; }
.pickedWxid { font-size: 13px; color: var(--ink); font-family: monospace; }
.repickBtn {
  border: 1px solid var(--line); background: transparent; border-radius: 6px;
  padding: 3px 10px; font-size: 12px; color: var(--muted); cursor: pointer;
}
.repickBtn:hover { color: var(--ink); }
```

- [ ] **Step 5: 运行确认通过**

Run: `cd frontend && npx vitest run ReferralCards`
Expected: PASS

- [ ] **Step 6: 类型检查 + 文案门**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 errors

Run: `bash scripts/check-no-human-takeover.sh`
Expected: exit 0（"从好友选择/重选"不含禁用词）

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/referral-cards/ frontend/src/__tests__/features/referral-cards/
git commit -m "feat(referral): 专属顾问微信号改好友选择器(roster源+名称联动回填+手填兜底)"
```

---

### Task 3: products-deals ContactPicker 换壳为 FriendPickerModal

**Files:**
- Modify: `frontend/src/features/products-deals/index.tsx:351-414`（ContactPicker 内部换壳，外部契约不变）
- Test: `frontend/src/__tests__/features/products-deals/ContactPicker.test.tsx`（新建）

**Interfaces:**
- Consumes: `FriendPickerModal` + `FriendPickerItem`（Task 1）；现有 `/api/contacts` 拉取（不变）；`Contact` 类型。
- Produces: `ContactPicker` 外部契约 `{ selected: Contact|null, onSelect: (c: Contact|null)=>void }` **保持不变**（父组件 :439/542/748 不改）。

- [ ] **Step 1: 亲验 + 写失败测试**

先 Read `frontend/src/features/products-deals/index.tsx:351-414` 确认 ContactPicker 现状与 api 导入。创建 `frontend/src/__tests__/features/products-deals/ContactPicker.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn() } }));

// ContactPicker 未导出则测试父组件入口;若导出则直接测。实现时确保可测:
// 优先 export ContactPicker 供测试(named export,不改父用法)。
import { ContactPicker } from "../../../features/products-deals";

beforeEach(() => {
  (api.get as any).mockResolvedValue({ items: [
    { id: "c1", accountId: "102", wxid: "wxid_x", nickname: "客户甲" },
    { id: "c2", accountId: "102", wxid: "wxid_y", nickname: "客户乙" },
  ]});
});

describe("products-deals ContactPicker 换壳", () => {
  it("点按钮开弹窗,点选好友以正确 Contact 调 onSelect", async () => {
    const onSelect = vi.fn();
    render(<ContactPicker selected={null} onSelect={onSelect} />);
    fireEvent.click(await screen.findByText(/选择好友|选择联系人/));
    fireEvent.click(await screen.findByText("客户乙"));
    await waitFor(() => {
      expect(onSelect).toHaveBeenCalledWith(expect.objectContaining({ id: "c2", wxid: "wxid_y" }));
    });
  });
});
```

- [ ] **Step 2: 运行确认失败**

Run: `cd frontend && npx vitest run ContactPicker`
Expected: FAIL —— `ContactPicker` 未导出 / 无"选择好友"按钮。

- [ ] **Step 3: 换壳 ContactPicker**

在 `frontend/src/features/products-deals/index.tsx` 顶部导入区加：

```tsx
import { FriendPickerModal, type FriendPickerItem } from "../../components/ui/FriendPickerModal";
```

把 `ContactPicker`（:351-414）整体替换为（**导出 named** 供测试；保留 `contacts` 拉取逻辑不变；内联搜索列表换成按钮+弹窗）：

```tsx
export function ContactPicker({
  selected,
  onSelect,
}: {
  selected: Contact | null;
  onSelect: (c: Contact | null) => void;
}) {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [open, setOpen] = useState(false);
  const accountId = useAccountStore((s) => s.currentAccountId());

  useEffect(() => {
    void (async () => {
      try {
        const url = accountId
          ? `/api/contacts?limit=100&accountId=${encodeURIComponent(accountId)}`
          : "/api/contacts?limit=100";
        const res = await api.get<{ items: Contact[] }>(url);
        setContacts(res.items);
      } catch {
        setContacts([]);
      }
    })();
  }, [accountId]);

  const items: FriendPickerItem[] = contacts.map((c) => ({
    wxid: c.wxid,
    nickname: c.nickname,
    remark: c.remark,
    avatarUrl: c.avatarUrl,
  }));

  const pick = (item: FriendPickerItem) => {
    const c = contacts.find((x) => x.wxid === item.wxid) ?? null;
    onSelect(c);
    setOpen(false);
  };

  return (
    <section className={styles.pickerPanel}>
      <button type="button" className={styles.input} onClick={() => setOpen(true)} style={{ textAlign: "left", cursor: "pointer" }}>
        {selected ? (selected.nickname || selected.remark || selected.wxid) : "选择好友…"}
      </button>
      <FriendPickerModal
        open={open}
        items={items}
        onSelect={pick}
        onClose={() => setOpen(false)}
        title="选择好友"
      />
    </section>
  );
}
```

注意：`Contact` 类型有 `nickname?`/`remark?`/`avatarUrl?` 但**无 `sex` 字段**（已亲验 `types/index.ts`），故 map 时不含 sex（FriendPickerItem.sex 可选，省略即可）。确认 `useState/useEffect/useAccountStore/api/styles` 均已在文件导入（现有 ContactPicker 已用）。

- [ ] **Step 4: 运行确认通过 + 现有测试不回归**

Run: `cd frontend && npx vitest run ContactPicker SuspectedDeals`
Expected: PASS（新测试 + 现有 products-deals 测试全绿）

- [ ] **Step 5: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 errors

- [ ] **Step 6: Commit**

```bash
git add frontend/src/features/products-deals/index.tsx frontend/src/__tests__/features/products-deals/ContactPicker.test.tsx
git commit -m "refactor(products-deals): ContactPicker换壳FriendPickerModal(数据源/契约不变)"
```

---

### Task 4: 全量门 + Playwright 收尾验证

**Files:**
- 无代码改动（验证 + 可能的临时 harness，最后清理）

- [ ] **Step 1: 前端全量门**

Run: `cd frontend && npx vitest run`
Expected: 全绿（含 Task 1-3 新增测试）

Run: `cd frontend && npm run build`
Expected: 成功，无 TS 报错

- [ ] **Step 2: 文案门**

Run: `bash scripts/check-no-human-takeover.sh`
Expected: exit 0

- [ ] **Step 3: Playwright 验证专属顾问选好友（可选，需 dev server + roster mock）**

若本地能起 dev server（`scripts/with_server.py` + roster API mock），写一个 Playwright 脚本走查专属顾问表单：点"从好友选择"→ 弹窗出好友网格 → 搜索 → 点选 → 顾问微信号栏显示回填的 wxid。截图 `friend_picker_after.png`。

若本地 dev server 依赖后端（roster 需真数据）跑不通，则显式说明"无法本地端到端验证 UI，逻辑已由 vitest 契约测试覆盖"——不假装成功。

- [ ] **Step 4: 清理临时验证产物**

删除 Playwright 临时脚本/harness/截图（若创建过）；确认 `git status` 无临时物残留。

- [ ] **Step 5: 记录验证结论**

在 commit message 或对话记录三个验证门结果（vitest 数 / build / 文案门）。

---

## Self-Review

**1. Spec coverage：**
- spec 组件1 FriendPickerModal（弹窗/网格/搜索/分页/空加载错误态/手填兜底）→ Task 1 ✓（包裹现有 Overlay，比 spec "手写遮罩"更稳）
- spec 组件2 专属顾问接入（roster 源/按钮/已选展示/重选/名称联动/手填兜底）→ Task 2 ✓
- spec 组件3 products-deals 换壳（数据源+契约不变）→ Task 3 ✓
- spec 决策链不改 → 无任务触及 DeciderChainEditor ✓
- spec 测试（FriendPickerModal 搜索/点选/空加载错误/手填 + referral 回填联动 + products 换壳不回归）→ Task 1/2/3 契约测试 ✓
- spec 验证门（build/vitest/check-no-human-takeover）→ Task 4 ✓

**对 spec 的合理优化（已亲验依据）：** spec 说"遮罩+居中弹窗"手写，实现改为**包裹现有 `Overlay` 组件**（`components/ui/Overlay/Overlay.tsx` 已封装 scrim/focus-trap/Esc/portal/body-lock，ConfirmDialog/FormDialog 都基于它）。更 DRY、无障碍更好、风险更低，行为等价。

**2. Placeholder scan：** 无 TBD/TODO；每个改码 step 给完整代码 + 确切命令 + 预期。

**3. Type consistency：**
- `FriendPickerItem { wxid, nickname?, remark?, avatarUrl?, sex?, badge? }`：Task 1 定义，Task 2（rosterItems map）、Task 3（contacts map）消费一致。
- `FriendPickerModal` props：Task 1 定义，Task 2（allowManualWxid+onManualWxid）、Task 3（不传 manual）用法一致。
- `ContactPicker { selected, onSelect: (c: Contact|null)=>void }`：Task 3 保持外部契约不变（父 :439/542/748 不改）。
- referral `setCardDraft` + `cardDraft.targetWxid/displayName`：Task 2 与 referralCardStore（string 字段）一致。

**风险点标注：** Task 3 测试假设 `ContactPicker` 可被 named-export 单测——实现时确保 `export function ContactPicker`（named，不改父组件内部用法，父仍在同文件引用）。若父组件用的是局部（非 export）引用，named export 不影响它。
