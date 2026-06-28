# 活动推送功能域补全 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 campaign 活动推送域从"只能总控 AI 操作"补全为有独立可浏览/可建/可预览的运营界面：后端补 `GET /api/campaigns` 列表端点，前端 campaign 频道拆成"列表 / 建活动表单 / 结果看板"三视图，看板加 CSV 导出与翻页。

**Architecture:** 后端只加一个只读列表端点（`CampaignListItem` 投影 struct，照 `ProductView` 范式，零写链路）。前端 `features/campaign/` 单文件拆目录，单频道内靠 `campaignStore.view` 切三视图；建活动表单四维动态圈人（产品多选/stage 下拉/枚举），dispatch（真发送）不做前端按钮——只走总控 AI 恒确认门。看板复用 PR #58 现有逻辑 + CSV 纯函数 + 内存翻页。

**Tech Stack:** Rust(Axum)+MongoDB 后端；React 19 + Vite + TypeScript + Zustand 前端；Vitest 前端测试 + cargo lib 单测。

**基线：** 分支 `feat/campaign-domain-completion` ← `origin/main` c163542（含 PR #57 `/sends` 端点 + PR #58 结果看板）。spec：`docs/superpowers/specs/2026-06-29-campaign-domain-completion-design.md`（§12 全链对 origin/main 逐行核实）。

## Global Constraints

- **dispatch 红线**：前端绝不做 dispatch（真发送）按钮/控件。真实触达客户只走总控 AI `dispatch_campaign` 恒确认门。Task 4（建活动表单）必须有测试断言"无 dispatch 控件"。
- **命名红线**（CI 硬门 `check-no-human-takeover` 扫 `src/routes/` + `frontend/src/` 新增行）：禁 `人工 / 接管 / takeover / hand[-]?off / 人工介入 / 人工托管`。用 AI 中性词：活动 / 推送 / 命中 / 已送达 / 被拦 / 已请示 / 已取消 / 草稿。注释也不得引入禁词。
- **设计系统**：只用 `frontend/src/components/ui/tokens.css` 既有 CSS 变量，禁硬编码色值。蓝（`--color-scheduled`）仅主操作/可点击；紫（`--color-brand`）仅 AI 身份。CSS 用 `Campaign.module.css` + `import styles from "./Campaign.module.css"`（CSS Modules）——**绝不裸 `import "./x.css"`**（Rollup tree-shake 会删光，有前车之鉴）。
- **JSON 字段全 camelCase**（前后端契约一致）。后端投影 struct 加 `#[serde(rename_all = "camelCase")]`。
- **IDOR**：后端所有 campaigns 查询 filter 含 `workspace_id = admin.current_workspace`。
- **测试基线不回退**：前端全套保持绿（PR #58 的 346 基线只增不减）；后端 `cargo test --lib` ≥ 350/0。新增测试只增量叠加，绝不删改 PR #58 现有 campaign 测试。
- **共享 worktree**：提交时按确切文件名 `git add <path>`，绝不 `git add -A` / `git add .`（同目录有并行会话）。
- **本地编译纪律**：后端只跑 `cargo test --lib`（集成测试编译耗磁盘，留 CI）；前端跑 `cd frontend && npx vitest run <具体文件>` + `npx tsc --noEmit`。

---

## File Structure

**后端（改 2 文件）：**
- `src/routes/campaigns.rs`：加 `CampaignListItem` 投影 struct（`From<&Campaign>`）+ `list_campaigns` handler + 投影单测。
- `src/routes/mod.rs`：`use campaigns::{...}` 加 `list_campaigns`；`/campaigns` 路由加 `.get(list_campaigns)`。

**前端（campaign 目录拆分 + store + 接线）：**
- `frontend/src/stores/campaignStore.ts`（改）：扩 `view / campaigns / listLoading / listLoaded / page` 状态 + `setView / loadCampaigns / setPage` actions；`openReport` 多设 `view:"board"`+`page:0`；`clear` 重置新字段。`CampaignListItem` 类型就近 export。
- `frontend/src/features/campaign/index.tsx`（改）：从"看板单文件"变"路由壳"——按 `view` 渲 `CampaignList` / `CampaignCreate` / `CampaignBoard`。
- `frontend/src/features/campaign/CampaignBoard.tsx`（新，搬迁）：PR #58 现 `index.tsx` 的看板逻辑（`bucketTone` / `bucketLabel` / `bucketCount` / 7桶汇总 / 明细表 / 桶筛选 / `lastAttemptedId` 防循环）搬来，加 CSV 导出按钮 + 翻页。`bucketTone` / `bucketLabel` 在此 export。
- `frontend/src/features/campaign/CampaignList.tsx`（新）：列表视图。
- `frontend/src/features/campaign/CampaignCreate.tsx`（新）：建活动 + 圈人预览表单。
- `frontend/src/features/campaign/ProductMultiSelect.tsx`（新）：产品多选 picker。
- `frontend/src/features/campaign/StageSelect.tsx`（新）：客户阶段下拉 picker。
- `frontend/src/features/campaign/buckets.ts`（新）：`bucketTone`/`bucketLabel`/`bucketCount` 纯函数（看板与 csv 共用，避免循环依赖）。
- `frontend/src/features/campaign/csv.ts`（新）：CSV 生成纯函数。
- `frontend/src/features/campaign/Campaign.module.css`（改）：扩列表/表单/翻页样式。
- `frontend/src/app/channels.ts:170`（改）：campaign 频道 subtitle 文案从"board-only"更新为"列表/建活动/看板"。
- 测试：`frontend/src/__tests__/features/campaign/` 下新增 `csv.test.ts` / `campaignStatus.test.ts` / `list.test.tsx` / `create.test.tsx` / `board-paging.test.tsx`；现有 `campaign.test.tsx`（看板渲染）的 `bucketTone`/`bucketLabel` import 路径改指 `CampaignBoard`。

**复用（零改动）：** `GET /api/products?active_only=true`（`{items:[{productId,name}]}`）、`GET /api/admin/taxonomies?kind=customer_stage`（`{items:[{value:{id,label}}]}`）。

---

## Task 1: 后端 GET /api/campaigns 列表端点 + CampaignListItem 投影

**Files:**
- Modify: `src/routes/campaigns.rs`（在 `campaign_sends_report` handler 之后、`#[cfg(test)] mod tests` 之前加投影 struct + handler；测试加进现有 `mod tests`）
- Modify: `src/routes/mod.rs:263`（use 加 `list_campaigns`）、`src/routes/mod.rs:791`（`/campaigns` route 加 `.get(list_campaigns)`）

**Interfaces:**
- Consumes: `Campaign`（models.rs:553，camelCase，字段 `id: Option<ObjectId>` / `title` / `status` / `target_count: Option<i64>` / `dispatched_count: i64` / `created_by` / `created_at: DateTime`）；`crate::models::dt_to_string(dt: DateTime) -> Option<String>`（models.rs:3356，RFC3339）；`state.db.campaigns()`（db/mod.rs:386）；`AuthenticatedAdmin`（含 `current_workspace`）。
- Produces: `GET /api/campaigns` → `{ "items": [CampaignListItem] }`，前端 Task 2 消费。

- [ ] **Step 1: 写投影 struct 的失败单测**

在 `src/routes/campaigns.rs` 的 `#[cfg(test)] mod tests` 内（`super::*` 已 import）追加：

```rust
    #[test]
    fn campaign_list_item_projection_shape_and_no_leak() {
        use serde_json::to_value;
        let now = DateTime::from_millis(1_700_000_000_000);
        let c = Campaign {
            id: Some(ObjectId::parse_str("64a1f0c2e4b0a1b2c3d4e5f6").unwrap()),
            workspace_id: "ws_secret".to_string(),
            account_id: "acc".to_string(),
            title: "双11老客7折".to_string(),
            intent_text: "内部意图不该泄漏".to_string(),
            segment_filter: SegmentFilter::default(),
            status: "completed".to_string(),
            target_count: Some(500),
            dispatched_count: 470,
            created_by: "admin".to_string(),
            created_at: now,
            updated_at: now,
        };
        let v = to_value(CampaignListItem::from(&c)).unwrap();
        // 投影字段齐全且 camelCase
        assert_eq!(v.get("campaignId").unwrap(), "64a1f0c2e4b0a1b2c3d4e5f6");
        assert_eq!(v.get("title").unwrap(), "双11老客7折");
        assert_eq!(v.get("status").unwrap(), "completed");
        assert_eq!(v.get("targetCount").unwrap(), 500);
        assert_eq!(v.get("dispatchedCount").unwrap(), 470);
        assert_eq!(v.get("createdBy").unwrap(), "admin");
        // createdAt 是 RFC3339 字符串（非 {$date} 对象）
        assert!(v.get("createdAt").unwrap().is_string());
        assert!(v.get("createdAt").unwrap().as_str().unwrap().contains("2023"));
        // 不泄漏内部字段
        assert!(v.get("workspaceId").is_none());
        assert!(v.get("workspace_id").is_none());
        assert!(v.get("segmentFilter").is_none());
        assert!(v.get("intentText").is_none());
        assert!(v.get("accountId").is_none());
    }

    #[test]
    fn campaign_list_item_omits_target_count_when_none() {
        use serde_json::to_value;
        let now = DateTime::from_millis(1_700_000_000_000);
        let mut c = super::tests::base_campaign();
        c.target_count = None;
        let v = to_value(CampaignListItem::from(&c)).unwrap();
        // draft 没预览过 → targetCount 字段整个缺失（skip_serializing_if）
        assert!(v.get("targetCount").is_none());
        assert_eq!(v.get("dispatchedCount").unwrap(), 0);
    }
```

并在 `mod tests` 内加一个最小 `Campaign` 构造 helper（紧跟现有 `base_contact` 之后）：

```rust
    pub(super) fn base_campaign() -> Campaign {
        let now = DateTime::from_millis(1_700_000_000_000);
        Campaign {
            id: Some(ObjectId::new()),
            workspace_id: "ws".to_string(),
            account_id: "acc".to_string(),
            title: "t".to_string(),
            intent_text: "i".to_string(),
            segment_filter: SegmentFilter::default(),
            status: "draft".to_string(),
            target_count: None,
            dispatched_count: 0,
            created_by: "admin".to_string(),
            created_at: now,
            updated_at: now,
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib campaign_list_item`
Expected: 编译失败（`CampaignListItem` 未定义）。

- [ ] **Step 3: 写投影 struct + From 实现**

在 `campaigns.rs` 的 `campaign_sends_report` handler 之后、`#[cfg(test)]` 之前插入：

```rust
/// `GET /api/campaigns` 列表项投影（不裸序列化 Campaign，避免泄漏
/// workspace_id/segment_filter/intent_text，且 created_at 转 RFC3339 string
/// 而非 {$date}——照 products.rs:85 ProductView 范式）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CampaignListItem {
    campaign_id: String,
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_count: Option<i64>,
    dispatched_count: i64,
    created_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

impl From<&Campaign> for CampaignListItem {
    fn from(c: &Campaign) -> Self {
        Self {
            campaign_id: c.id.map(|i| i.to_hex()).unwrap_or_default(),
            title: c.title.clone(),
            status: c.status.clone(),
            target_count: c.target_count,
            dispatched_count: c.dispatched_count,
            created_by: c.created_by.clone(),
            created_at: crate::models::dt_to_string(c.created_at),
        }
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib campaign_list_item`
Expected: 2 passed（投影形态 + targetCount 缺失）。

- [ ] **Step 5: 写 list_campaigns handler**

在投影 struct 之后插入（`FindOptions` 已被 sends handler 区域用到；若未 import 则补 `use mongodb::options::FindOptions;`——先 grep 确认，campaigns.rs 顶部已有的话不重复）：

```rust
/// GET /api/campaigns —— 列出本 workspace 全部活动（只读，createdAt 倒序）。
/// 无分页（活动数量本身有限）。IDOR：filter 含 workspace_id。
pub async fn list_campaigns(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
) -> AppResult<Json<Value>> {
    let mut cursor = state
        .db
        .campaigns()
        .find(
            doc! { "workspaceId": &admin.current_workspace },
            mongodb::options::FindOptions::builder()
                .sort(doc! { "createdAt": -1 })
                .build(),
        )
        .await?;
    let mut items: Vec<CampaignListItem> = Vec::new();
    while let Some(c) = cursor.try_next().await? {
        items.push(CampaignListItem::from(&c));
    }
    Ok(Json(json!({ "items": items })))
}
```

> 注意：Campaign 是 `rename_all="camelCase"`，存储键是 `workspaceId` / `createdAt`（与 create_campaign 的 `$set` 写键一致，campaigns.rs:268/313 用 camelCase）。filter/sort 必须用 camelCase 键。

- [ ] **Step 6: 注册路由**

`src/routes/mod.rs:263` 的 use 改为（加 `list_campaigns`，保持字母序）：

```rust
use campaigns::{
    campaign_sends_report, create_campaign, dispatch_campaign, list_campaigns, preview_campaign,
};
```

`src/routes/mod.rs:791` 的 `/campaigns` route 改为：

```rust
        .route("/campaigns", post(create_campaign).get(list_campaigns))
```

- [ ] **Step 7: 跑全 lib 测试 + 编译**

Run: `cargo test --lib campaign`
Expected: 投影 2 测试 + 现有 campaign 测试全 passed。
Run: `cargo check`
Expected: 0 error（确认路由接线类型对）。

- [ ] **Step 8: Commit**

```bash
git add src/routes/campaigns.rs src/routes/mod.rs
git commit -m "feat(campaign): GET /api/campaigns 列表端点 + CampaignListItem 投影(不泄漏内部字段/createdAt RFC3339)"
```

---

## Task 2: campaignStore 扩展（view/列表/翻页状态）

**Files:**
- Modify: `frontend/src/stores/campaignStore.ts`（现有 PR #58 store，保留所有现字段/action，只增量加）
- Test: `frontend/src/__tests__/features/campaign/store.test.ts`（PR #58 现有文件，追加新测试，不改旧）

**Interfaces:**
- Consumes: `api.get<T>(url)`（lib/api.ts:53）；`useUiStore.getState().setError(msg)`（stores/uiStore.ts:7）；`useNavigationStore.getState().setChannel("campaign")`（现有）。
- Produces: `CampaignListItem` 类型 + `view / campaigns / listLoading / listLoaded / page` 状态 + `setView / loadCampaigns / setPage` actions，供 Task 3/5/6 消费。

- [ ] **Step 1: 写失败测试**

在 `store.test.ts` 末尾（现有 `describe("campaignStore")` 之后）追加新 describe。现有文件顶部已 mock `api`/`navigationStore`/`uiStore`，复用：

```ts
describe("campaignStore 列表/视图扩展", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCampaignStore.setState({
      selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null,
      view: "list", campaigns: [], listLoading: false, listLoaded: false, page: 0,
    });
  });

  it("setView 切换视图", () => {
    useCampaignStore.getState().setView("create");
    expect(useCampaignStore.getState().view).toBe("create");
  });

  it("loadCampaigns 成功写入 campaigns + listLoaded", async () => {
    const items = [{ campaignId: "c1", title: "T", status: "completed", dispatchedCount: 5, createdBy: "a" }];
    (api.get as any).mockResolvedValue({ items });
    await useCampaignStore.getState().loadCampaigns();
    const s = useCampaignStore.getState();
    expect(s.campaigns).toEqual(items);
    expect(s.listLoaded).toBe(true);
    expect(s.listLoading).toBe(false);
    expect(api.get).toHaveBeenCalledWith("/api/campaigns");
  });

  it("loadCampaigns 失败也置 listLoaded=true(防重试循环) + campaigns 保持空", async () => {
    (api.get as any).mockRejectedValue(new Error("boom"));
    await useCampaignStore.getState().loadCampaigns();
    const s = useCampaignStore.getState();
    expect(s.listLoaded).toBe(true);
    expect(s.listLoading).toBe(false);
    expect(s.campaigns).toEqual([]);
  });

  it("openReport 多设 view=board + page=0", () => {
    (api.get as any).mockResolvedValue({ campaignId: "c1", title: "", status: "", summary: {}, items: [] });
    useCampaignStore.setState({ page: 7 });
    useCampaignStore.getState().openReport("c1");
    const s = useCampaignStore.getState();
    expect(s.view).toBe("board");
    expect(s.page).toBe(0);
    expect(s.selectedCampaignId).toBe("c1");
  });

  it("setPage 改翻页", () => {
    useCampaignStore.getState().setPage(3);
    expect(useCampaignStore.getState().page).toBe(3);
  });

  it("clear 重置新字段", () => {
    useCampaignStore.setState({ view: "board", campaigns: [{ campaignId: "x", title: "", status: "", dispatchedCount: 0, createdBy: "" }], listLoaded: true, page: 5 });
    useCampaignStore.getState().clear();
    const s = useCampaignStore.getState();
    expect(s.view).toBe("list");
    expect(s.campaigns).toEqual([]);
    expect(s.listLoaded).toBe(false);
    expect(s.page).toBe(0);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/store.test.ts`
Expected: FAIL（`setView`/`loadCampaigns`/`view` 等不存在，类型报错）。

- [ ] **Step 3: 扩展 store**

`frontend/src/stores/campaignStore.ts` 现有 PR #58 内容保留。加 `CampaignListItem` 类型（紧跟现有 `CampaignReport` 等类型 export 之后）：

```ts
export interface CampaignListItem {
  campaignId: string;
  title: string;
  status: string;
  targetCount?: number;
  dispatchedCount: number;
  createdBy: string;
  createdAt?: string;
}
```

`CampaignState` interface 加字段（在现有字段之后）：

```ts
  view: "list" | "create" | "board";
  campaigns: CampaignListItem[];
  listLoading: boolean;
  listLoaded: boolean;
  page: number;
  setView: (v: "list" | "create" | "board") => void;
  loadCampaigns: () => Promise<void>;
  setPage: (n: number) => void;
```

create() 初始 state 加：

```ts
  view: "list",
  campaigns: [],
  listLoading: false,
  listLoaded: false,
  page: 0,
```

新 actions（加在 clear 之前）：

```ts
  setView: (v) => set({ view: v }),
  loadCampaigns: async () => {
    set({ listLoading: true, listLoaded: true });
    try {
      const r = await api.get<{ items: CampaignListItem[] }>("/api/campaigns");
      set({ campaigns: r.items });
    } catch (e) {
      useUiStore.getState().setError(e instanceof Error ? e.message : String(e));
    } finally {
      set({ listLoading: false });
    }
  },
  setPage: (n) => set({ page: n }),
```

`openReport` 改为多设 `view`+`page`：

```ts
  openReport: (id) => {
    set({ selectedCampaignId: id, report: null, view: "board", page: 0 });
    useNavigationStore.getState().setChannel("campaign");
    void get().loadReport(id);
  },
```

`clear` 改为重置新字段：

```ts
  clear: () => set({ selectedCampaignId: null, report: null, loading: false, lastAttemptedId: null, view: "list", campaigns: [], listLoading: false, listLoaded: false, page: 0 }),
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/store.test.ts`
Expected: 新 6 测试 + PR #58 现有 store 测试全 passed。

- [ ] **Step 5: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 6: Commit**

```bash
git add frontend/src/stores/campaignStore.ts frontend/src/__tests__/features/campaign/store.test.ts
git commit -m "feat(campaign-fe): campaignStore 扩 view/campaigns/listLoaded/page(列表+翻页+防循环)"
```

---

## Task 3: 看板搬迁到 CampaignBoard + CSV 导出 + 翻页

把 PR #58 现 `index.tsx` 的看板逻辑搬到 `CampaignBoard.tsx`，加 CSV 纯函数 + 内存翻页。搬迁后 `index.tsx` 暂时透传（`export { default } from "./CampaignBoard"`），保证 PR #58 现有 `campaign.test.tsx` 仍绿；Task 5 才把 `index.tsx` 变路由壳。

> **避免循环依赖**：`bucketTone`/`bucketLabel` 抽到独立小模块 `buckets.ts`，`CampaignBoard.tsx`（视图）和 `csv.ts`（导出）都从 `buckets.ts` import。否则 csv.ts↔CampaignBoard 互相 import 形成环（运行期虽可工作但脆弱，且步骤顺序上 csv.ts 先于 CampaignBoard 写会悬空）。`campaign.test.tsx` 的 `bucketTone`/`bucketLabel` import 改指 `buckets`，看板组件 import 改指 `CampaignBoard` 的 default。

**Files:**
- Create: `frontend/src/features/campaign/buckets.ts`（`bucketTone`/`bucketLabel`/`bucketCount` 纯函数）
- Create: `frontend/src/features/campaign/CampaignBoard.tsx`
- Create: `frontend/src/features/campaign/csv.ts`
- Modify: `frontend/src/features/campaign/index.tsx`（透传）
- Modify: `frontend/src/features/campaign/Campaign.module.css`（加 CSV 按钮 + 翻页样式）
- Test: `frontend/src/__tests__/features/campaign/csv.test.ts`（新）、`board-paging.test.tsx`（新）
- Modify: `frontend/src/__tests__/features/campaign/campaign.test.tsx`（`bucketTone`/`bucketLabel` import 改指 `buckets`；`CampaignFeature` default import 改指 `CampaignBoard`；两处 mockReturnValue 补 page/setPage）

**Interfaces:**
- Consumes: `useCampaignStore`（Task 2：`report / selectedCampaignId / loadReport / loading / lastAttemptedId / page / setPage`）；`StatusBadge`（tone: running/scheduled/held/blocked/inactive）；`EmptyState`。
- Produces: `buckets.ts` export `bucketTone(s): StatusTone` / `bucketLabel(s): string` / `bucketCount(summary, s): number`；`CampaignBoard`（default export）；`toCsv(items): string`（csv.ts）。

- [ ] **Step 1: 写 buckets.ts（纯函数，无依赖）**

`frontend/src/features/campaign/buckets.ts`：

```ts
import type { StatusTone } from "../../components/ui/StatusBadge";

export function bucketTone(bucket: string): StatusTone {
  switch (bucket) {
    case "sent": return "running";
    case "pending": return "scheduled";
    case "blocked": return "blocked";
    case "escalated": return "held";
    default: return "inactive";
  }
}

export function bucketLabel(bucket: string): string {
  switch (bucket) {
    case "sent": return "已送达";
    case "pending": return "在途";
    case "blocked": return "被拦";
    case "escalated": return "已请示";
    case "canceled": return "已取消";
    case "skipped": return "去重跳过";
    default: return "未知";
  }
}

export function bucketCount(summary: Record<string, unknown>, bucket: string): number {
  const v = summary[bucket];
  if (typeof v === "number") return v;
  if (v && typeof v === "object") {
    return Object.values(v as Record<string, number>).reduce((a, b) => a + b, 0);
  }
  return 0;
}
```

- [ ] **Step 1b: 写 csv.ts 失败测试**

`frontend/src/__tests__/features/campaign/csv.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { toCsv } from "../../../features/campaign/csv";
import type { CampaignSendItem } from "../../../stores/campaignStore";

describe("toCsv", () => {
  it("表头 + 每行 客户名/wxid/状态中文/原因", () => {
    const items: CampaignSendItem[] = [
      { contactWxid: "wx_a", name: "张三", status: "sent" },
      { contactWxid: "wx_b", name: "李四", status: "blocked", reason: "daily_limit" },
    ];
    const csv = toCsv(items);
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe("客户名,wxid,状态,原因");
    expect(lines[1]).toBe("张三,wx_a,已送达,");
    expect(lines[2]).toBe("李四,wx_b,被拦,daily_limit");
  });

  it("空 items 仅表头", () => {
    expect(toCsv([])).toBe("客户名,wxid,状态,原因");
  });

  it("含逗号/引号/换行的值用双引号转义", () => {
    const items: CampaignSendItem[] = [
      { contactWxid: "wx_c", name: 'a,b"c', status: "unknown", reason: "x\ny" },
    ];
    const csv = toCsv(items);
    const line = csv.split("\r\n")[1];
    expect(line).toContain('"a,b""c"');
    expect(line).toContain('"x\ny"');
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/csv.test.ts`
Expected: FAIL（`toCsv` 不存在）。

- [ ] **Step 3: 写 csv.ts**

`frontend/src/features/campaign/csv.ts`：

```ts
import type { CampaignSendItem } from "../../stores/campaignStore";
import { bucketLabel } from "./buckets";

function esc(v: string): string {
  if (/[",\n]/.test(v)) return '"' + v.replace(/"/g, '""') + '"';
  return v;
}

export function toCsv(items: CampaignSendItem[]): string {
  const header = "客户名,wxid,状态,原因";
  const rows = items.map((it) =>
    [esc(it.name || ""), esc(it.contactWxid), esc(bucketLabel(it.status)), esc(it.reason || "")].join(","),
  );
  return [header, ...rows].join("\r\n");
}
```

- [ ] **Step 4: 写 CampaignBoard.tsx**

把 PR #58 `index.tsx` 看板逻辑搬来，default export 改名 `CampaignBoard`。`bucketTone`/`bucketLabel`/`bucketCount` 已移到 `buckets.ts`（Step 1），此处 import 不再重定义。增量加 CSV 按钮 + 翻页。完整文件：

```tsx
import { useEffect, useState } from "react";
import { Megaphone, Download } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge } from "../../components/ui/StatusBadge";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CampaignSendItem } from "../../stores/campaignStore";
import { bucketTone, bucketLabel, bucketCount } from "./buckets";
import { toCsv } from "./csv";
import styles from "./Campaign.module.css";

const BUCKETS = ["sent", "pending", "blocked", "escalated", "canceled", "skipped", "unknown"] as const;
const PAGE_SIZE = 50;

function downloadCsv(filename: string, csv: string) {
  const blob = new Blob(["﻿" + csv], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export default function CampaignBoard() {
  const { selectedCampaignId, report, loadReport } = useCampaignStore();
  const [filter, setFilter] = useState<string>("all");
  const loading = useCampaignStore((s) => s.loading);
  const lastAttemptedId = useCampaignStore((s) => s.lastAttemptedId);
  const page = useCampaignStore((s) => s.page);
  const setPage = useCampaignStore((s) => s.setPage);

  useEffect(() => {
    if (selectedCampaignId && !report && !loading && selectedCampaignId !== lastAttemptedId) {
      void loadReport(selectedCampaignId);
    }
  }, [selectedCampaignId, report, loading, lastAttemptedId, loadReport]);

  if (!selectedCampaignId) {
    return (
      <div className={styles.page}>
        <EmptyState
          icon={<Megaphone size={28} />}
          title="暂无活动结果"
          hint="在 AI 总控 dispatch 活动后，点「查看推送结果」进入这里查看真实触达分布。"
        />
      </div>
    );
  }

  const summary = report?.summary;
  const items: CampaignSendItem[] = report?.items ?? [];
  const shown = filter === "all" ? items : items.filter((it) => it.status === filter);
  const pageCount = Math.max(1, Math.ceil(shown.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount - 1);
  const pageRows = shown.slice(safePage * PAGE_SIZE, safePage * PAGE_SIZE + PAGE_SIZE);

  const reasonMap = (bucket: "blocked" | "canceled" | "escalated"): Record<string, number> =>
    (summary?.[bucket] as Record<string, number> | undefined) ?? {};

  const pickFilter = (b: string) => { setFilter(b); setPage(0); };

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Campaign Result</span>
            <span className={styles.title}>{report ? report.title : "—"}</span>
          </div>
          {report && <StatusBadge tone="scheduled">{report.status}</StatusBadge>}
        </div>
        <div className={styles.metrics}>
          {BUCKETS.map((b) => (
            <div key={b} className={styles.metric} data-testid={`metric-${b}`}>
              <span className={styles.metricLabel}>{bucketLabel(b)}</span>
              <span className={styles.metricValue}>{summary ? bucketCount(summary as unknown as Record<string, unknown>, b) : "—"}</span>
              {(b === "blocked" || b === "canceled" || b === "escalated") && summary && (
                <div className={styles.reasons}>
                  {Object.entries(reasonMap(b)).map(([reason, n]) => (
                    <span key={reason} className={styles.reasonItem}>{reason} ×{n}</span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Per-Contact</span>
            <span className={styles.title}>推送明细</span>
          </div>
          <button
            type="button"
            className={styles.exportBtn}
            disabled={items.length === 0}
            onClick={() => downloadCsv(`campaign-${selectedCampaignId}-sends.csv`, toCsv(items))}
          >
            <Download size={14} /> 导出 CSV
          </button>
        </div>

        <div className={styles.filters}>
          <button type="button" className={`${styles.chip} ${filter === "all" ? styles.chipActive : ""}`} onClick={() => pickFilter("all")}>
            全部 ({items.length})
          </button>
          {BUCKETS.map((b) => (
            <button key={b} type="button" className={`${styles.chip} ${filter === b ? styles.chipActive : ""}`} onClick={() => pickFilter(b)}>
              {bucketLabel(b)}
            </button>
          ))}
        </div>

        {shown.length === 0 ? (
          <EmptyState title="暂无推送明细" hint="该筛选下没有客户记录。" />
        ) : (
          <>
            <table className={styles.table}>
              <thead>
                <tr className={styles.tr}>
                  <th className={`${styles.th} ${styles.thName}`}>客户</th>
                  <th className={styles.th}>wxid</th>
                  <th className={styles.th}>状态</th>
                  <th className={styles.th}>原因</th>
                </tr>
              </thead>
              <tbody>
                {pageRows.map((it) => (
                  <tr key={it.contactWxid} className={styles.tr} data-testid="detail-row">
                    <td className={`${styles.td} ${styles.tdName}`}>{it.name || "—"}</td>
                    <td className={`${styles.td} ${styles.tdWxid}`}>{it.contactWxid}</td>
                    <td className={styles.td}><StatusBadge tone={bucketTone(it.status)}>{bucketLabel(it.status)}</StatusBadge></td>
                    <td className={styles.td}>{it.reason || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {pageCount > 1 && (
              <div className={styles.pager}>
                <button type="button" className={styles.pagerBtn} disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>上一页</button>
                <span className={styles.pagerInfo}>{safePage + 1} / {pageCount}</span>
                <button type="button" className={styles.pagerBtn} disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}>下一页</button>
              </div>
            )}
          </>
        )}
      </section>
    </div>
  );
}
```

- [ ] **Step 5: index.tsx 暂时透传**

`frontend/src/features/campaign/index.tsx` 全文替换（透传看板 default；`bucketTone`/`bucketLabel` 现在在 `buckets.ts`，无需 re-export）：

```tsx
export { default } from "./CampaignBoard";
```

- [ ] **Step 6: campaign.test.tsx import 改指 + 补 mock 字段**

`campaign.test.tsx` 第 3 行原 `import CampaignFeature, { bucketTone, bucketLabel } from "../../../features/campaign";` 拆成两行（看板 default 来自 `CampaignBoard`，纯函数来自 `buckets`）：

```ts
import CampaignFeature from "../../../features/campaign/CampaignBoard";
import { bucketTone, bucketLabel } from "../../../features/campaign/buckets";
```

两处 `(useCampaignStore as any).mockReturnValue({...})`（"有 report"那条 + "空 items"那条）各补 `page: 0, setPage: vi.fn()` 字段（CampaignBoard 新增了 page/setPage 的 selector 读取，否则取到 undefined 致 `Math.min(undefined,...)` NaN）。其余断言不动。

> 注：selector 调用 `useCampaignStore((s)=>s.page)` 在 mockReturnValue 下会忽略 selector 直接返回整个对象——所以 mock 对象必须含 `page`/`setPage` 顶层字段。

- [ ] **Step 7: 加 CSS**

`Campaign.module.css` 末尾追加：

```css
.exportBtn {
  display: inline-flex; align-items: center; gap: 6px;
  font: inherit; font-size: 12px; font-weight: 600; letter-spacing: -.1px;
  padding: 7px 13px; border-radius: var(--r-sm); cursor: pointer;
  color: var(--color-scheduled); background: var(--surface-card);
  border: 1px solid rgba(10, 132, 255, .35);
  transition: background .18s, border-color .18s;
}
.exportBtn:hover:not(:disabled) { background: rgba(10, 132, 255, .06); border-color: rgba(10, 132, 255, .5); }
.exportBtn:disabled { color: var(--ink-3); border-color: var(--hairline); cursor: not-allowed; }

.pager { display: flex; align-items: center; justify-content: center; gap: 14px; margin-top: 14px; }
.pagerBtn {
  font: inherit; font-size: 12px; font-weight: 600; padding: 6px 14px;
  border-radius: var(--r-sm); cursor: pointer;
  color: var(--ink-2); background: var(--surface-card); border: 1px solid var(--hairline);
}
.pagerBtn:hover:not(:disabled) { border-color: rgba(10, 132, 255, .3); }
.pagerBtn:disabled { color: var(--ink-3); cursor: not-allowed; opacity: .6; }
.pagerInfo { font-size: 12px; color: var(--ink-3); font-variant-numeric: tabular-nums; }
```

- [ ] **Step 8: 写翻页测试**

`frontend/src/__tests__/features/campaign/board-paging.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CampaignBoard from "../../../features/campaign/CampaignBoard";
import { useCampaignStore } from "../../../stores/campaignStore";
import type { CampaignReport } from "../../../stores/campaignStore";

vi.mock("../../../stores/campaignStore");

function makeReport(n: number): CampaignReport {
  return {
    campaignId: "c1", title: "T", status: "completed",
    summary: { targetCount: n, sent: n, pending: 0, skipped: 0, unknown: 0, blocked: {}, canceled: {}, escalated: {} },
    items: Array.from({ length: n }, (_, i) => ({ contactWxid: `wx_${i}`, name: `n${i}`, status: "sent" })),
  };
}

function mockStore(report: CampaignReport, page: number, setPage: () => void) {
  (useCampaignStore as any).mockImplementation((sel?: any) => {
    const state = { selectedCampaignId: "c1", report, loadReport: vi.fn(), loading: false, lastAttemptedId: "c1", page, setPage };
    return sel ? sel(state) : state;
  });
}

describe("看板翻页", () => {
  beforeEach(() => vi.clearAllMocks());

  it("items > 50 只渲一页 50 行 + 翻页器", () => {
    mockStore(makeReport(120), 0, vi.fn());
    render(<CampaignBoard />);
    expect(screen.getAllByTestId("detail-row")).toHaveLength(50);
    expect(screen.getByText("1 / 3")).toBeInTheDocument();
  });

  it("点下一页调 setPage(1)", () => {
    const setPage = vi.fn();
    mockStore(makeReport(120), 0, setPage);
    render(<CampaignBoard />);
    fireEvent.click(screen.getByText("下一页"));
    expect(setPage).toHaveBeenCalledWith(1);
  });

  it("items <= 50 不渲翻页器", () => {
    mockStore(makeReport(10), 0, vi.fn());
    render(<CampaignBoard />);
    expect(screen.queryByText(/\/ \d/)).toBeNull();
  });
});
```

- [ ] **Step 9: 跑测试 + 类型**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/`
Expected: csv 3 + board-paging 3 + 现有 campaign/store/jump/no-refetch-loop 全 passed。
Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 10: Commit**

```bash
git add frontend/src/features/campaign/CampaignBoard.tsx frontend/src/features/campaign/csv.ts frontend/src/features/campaign/index.tsx frontend/src/features/campaign/Campaign.module.css frontend/src/__tests__/features/campaign/csv.test.ts frontend/src/__tests__/features/campaign/board-paging.test.tsx frontend/src/__tests__/features/campaign/campaign.test.tsx
git commit -m "feat(campaign-fe): 看板搬迁 CampaignBoard + CSV 导出 + 明细翻页"
```

---

## Task 4: 两个 picker 原语 + CampaignCreate 建活动表单

建活动 + 圈人预览整页表单。四维动态圈人：产品多选 + stage 下拉 + aftercare/valueTier 枚举。**dispatch 红线：无 dispatch 按钮**。draft 复用：一次会话只产生一个 draft。

**Files:**
- Create: `frontend/src/features/campaign/ProductMultiSelect.tsx`
- Create: `frontend/src/features/campaign/StageSelect.tsx`
- Create: `frontend/src/features/campaign/CampaignCreate.tsx`
- Modify: `frontend/src/features/campaign/Campaign.module.css`（加表单样式）
- Test: `frontend/src/__tests__/features/campaign/create.test.tsx`（新）

**Interfaces:**
- Consumes: `api.get`（拉产品/字典）、`api.post`（create/preview）；`useCampaignStore`（`setView` / `openReport`）；`useUiStore.setError`。
- Produces: `CampaignCreate`（default export）；`ProductMultiSelect`（`{ value: string[]; onChange: (v: string[]) => void }`）；`StageSelect`（`{ value: string; onChange: (v: string) => void }`）。

- [ ] **Step 1: 写 ProductMultiSelect**

`frontend/src/features/campaign/ProductMultiSelect.tsx`：

```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import styles from "./Campaign.module.css";

interface ProductOption { productId: string; name: string; }

export function ProductMultiSelect({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const [opts, setOpts] = useState<ProductOption[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const r = await api.get<{ items: ProductOption[] }>("/api/products?active_only=true");
        if (alive) setOpts(r.items);
      } catch {
        if (alive) setFailed(true);
      }
    })();
    return () => { alive = false; };
  }, []);

  if (failed) return <div className={styles.fieldHint}>产品选项加载失败</div>;
  if (opts.length === 0) return <div className={styles.fieldHint}>暂无可选产品</div>;

  const toggle = (pid: string) => {
    onChange(value.includes(pid) ? value.filter((x) => x !== pid) : [...value, pid]);
  };

  return (
    <div className={styles.checkGroup}>
      {opts.map((o) => (
        <label key={o.productId} className={styles.checkItem}>
          <input type="checkbox" checked={value.includes(o.productId)} onChange={() => toggle(o.productId)} />
          <span>{o.name}</span>
        </label>
      ))}
    </div>
  );
}
```

- [ ] **Step 2: 写 StageSelect**

`frontend/src/features/campaign/StageSelect.tsx`：

```tsx
import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import styles from "./Campaign.module.css";

interface StageOption { value: { id: string; label: string }; }

export function StageSelect({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [opts, setOpts] = useState<{ id: string; label: string }[]>([]);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const r = await api.get<{ items: StageOption[] }>("/api/admin/taxonomies?kind=customer_stage");
        if (alive) setOpts(r.items.map((i) => ({ id: i.value.id, label: i.value.label })));
      } catch {
        if (alive) setFailed(true);
      }
    })();
    return () => { alive = false; };
  }, []);

  if (failed) return <div className={styles.fieldHint}>客户阶段选项加载失败</div>;

  return (
    <select className={styles.select} value={value} onChange={(e) => onChange(e.target.value)}>
      <option value="">不限</option>
      {opts.map((o) => (
        <option key={o.id} value={o.id}>{o.label}</option>
      ))}
    </select>
  );
}
```

- [ ] **Step 3: 写 CampaignCreate.tsx**

`frontend/src/features/campaign/CampaignCreate.tsx`。注意 draft 复用：`draftCampaignId` 状态，首次预览 create+preview，改条件再预览复用同一 id 只调 preview。

```tsx
import { useState } from "react";
import { api } from "../../lib/api";
import { useCampaignStore } from "../../stores/campaignStore";
import { useUiStore } from "../../stores/uiStore";
import { ProductMultiSelect } from "./ProductMultiSelect";
import { StageSelect } from "./StageSelect";
import styles from "./Campaign.module.css";

interface PreviewResult { campaignId: string; targetCount: number; samples: { wxid: string; name: string }[]; }

export default function CampaignCreate() {
  const setView = useCampaignStore((s) => s.setView);
  const openReport = useCampaignStore((s) => s.openReport);
  const setError = useUiStore((s) => s.setError);

  const [title, setTitle] = useState("");
  const [intentText, setIntentText] = useState("");
  const [productIds, setProductIds] = useState<string[]>([]);
  const [customerStage, setCustomerStage] = useState("");
  const [aftercare, setAftercare] = useState("");
  const [valueTier, setValueTier] = useState("");
  const [draftCampaignId, setDraftCampaignId] = useState<string | null>(null);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [busy, setBusy] = useState(false);

  const canPreview = title.trim() !== "" && intentText.trim() !== "" && !busy;

  const segmentFilter = () => {
    const f: Record<string, unknown> = {};
    if (productIds.length) f.productIds = productIds;
    if (aftercare) f.aftercare = aftercare;
    if (valueTier) f.valueTier = valueTier;
    if (customerStage) f.customerStage = customerStage;
    return f;
  };

  const handlePreview = async () => {
    if (!canPreview) return;
    setBusy(true);
    try {
      let id = draftCampaignId;
      if (!id) {
        const created = await api.post<{ id: string }>("/api/campaigns", {
          title: title.trim(), intentText: intentText.trim(), segmentFilter: segmentFilter(),
        });
        id = created.id;
        setDraftCampaignId(id);
      }
      const r = await api.post<PreviewResult>(`/api/campaigns/${id}/preview`, {});
      setPreview(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  // 改任一条件 → 作废旧 preview（但保留 draftCampaignId 复用），下次预览用同一 draft 重新圈人
  const onCondChange = <T,>(setter: (v: T) => void) => (v: T) => { setter(v); setPreview(null); };

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>New Campaign</span>
            <span className={styles.title}>新建活动</span>
          </div>
          <button type="button" className={styles.pagerBtn} onClick={() => setView("list")}>返回列表</button>
        </div>

        <div className={styles.form}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>活动标题</span>
            <input className={styles.input} value={title} onChange={(e) => setTitle(e.target.value)} placeholder="如：双11老客续费7折" />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>活动意图</span>
            <textarea className={styles.textarea} value={intentText} onChange={(e) => setIntentText(e.target.value)} placeholder="活动要点，将作为给客户推送的语境，由 AI 据各自画像生成个性化话术" />
          </label>

          <div className={styles.fieldLabel}>圈人条件（各项可选，留空即不限）</div>
          <label className={styles.field}>
            <span className={styles.fieldSub}>买过的产品</span>
            <ProductMultiSelect value={productIds} onChange={onCondChange(setProductIds)} />
          </label>
          <div className={styles.fieldRow}>
            <label className={styles.field}>
              <span className={styles.fieldSub}>客户阶段</span>
              <StageSelect value={customerStage} onChange={onCondChange(setCustomerStage)} />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldSub}>售后状态</span>
              <select className={styles.select} value={aftercare} onChange={(e) => onCondChange(setAftercare)(e.target.value)}>
                <option value="">不限</option>
                <option value="in_aftercare">售后中</option>
                <option value="expired">已到期</option>
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.fieldSub}>价值分层</span>
              <select className={styles.select} value={valueTier} onChange={(e) => onCondChange(setValueTier)(e.target.value)}>
                <option value="">不限</option>
                <option value="high">高</option>
                <option value="mid">中</option>
                <option value="low">低</option>
              </select>
            </label>
          </div>

          <button type="button" className={styles.primaryBtn} disabled={!canPreview} onClick={handlePreview}>
            {busy ? "圈人中…" : "圈人预览"}
          </button>
        </div>
      </section>

      {preview && (
        <section className={styles.panel}>
          <div className={styles.head}>
            <div className={styles.headL}>
              <span className={styles.eyebrow}>Preview</span>
              <span className={styles.title}>圈人预览：命中 {preview.targetCount} 人</span>
            </div>
          </div>
          <p className={styles.previewNote}>实际推送时会重新圈选，人数可能微调。</p>
          {preview.samples.length > 0 && (
            <div className={styles.samples}>
              {preview.samples.map((s) => (
                <span key={s.wxid} className={styles.sampleChip}>{s.name || s.wxid}</span>
              ))}
            </div>
          )}
          {preview.targetCount === 0 && <p className={styles.fieldHint}>命中 0 人，调整条件再试。</p>}
          <div className={styles.previewActions}>
            <p className={styles.dispatchHint}>确认推送请在 AI 总控对话中对该活动 dispatch（高风险动作由 AI 恒确认门把关）。</p>
            <div className={styles.previewBtns}>
              <button type="button" className={styles.pagerBtn} onClick={() => setView("list")}>返回列表</button>
              {draftCampaignId && (
                <button type="button" className={styles.exportBtn} onClick={() => openReport(draftCampaignId)}>查看结果看板</button>
              )}
            </div>
          </div>
        </section>
      )}
    </div>
  );
}
```

> **dispatch 红线**：本组件无任何 `dispatch` 调用或按钮——预览区只提示"去 AI 总控 dispatch"。Step 6 测试断言守住。

- [ ] **Step 4: 加表单 CSS**

`Campaign.module.css` 末尾追加：

```css
.form { display: grid; gap: 16px; }
.field { display: grid; gap: 6px; }
.fieldRow { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; }
@media (max-width: 720px) { .fieldRow { grid-template-columns: 1fr; } }
.fieldLabel { font-size: 13px; font-weight: 600; color: var(--ink-1); }
.fieldSub { font-size: 12px; font-weight: 600; color: var(--ink-2); }
.fieldHint { font-size: 12px; color: var(--ink-3); }
.input, .textarea, .select {
  font: inherit; font-size: 13px; color: var(--ink-1);
  padding: 9px 12px; border-radius: var(--r-sm);
  border: 1px solid var(--hairline); background: var(--surface-card);
}
.textarea { min-height: 72px; resize: vertical; }
.input:focus, .textarea:focus, .select:focus { outline: none; border-color: var(--color-scheduled); }
.checkGroup { display: flex; flex-wrap: wrap; gap: 8px 16px; }
.checkItem { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; color: var(--ink-1); }
.primaryBtn {
  justify-self: start; font: inherit; font-size: 13px; font-weight: 600;
  padding: 9px 20px; border-radius: var(--r-sm); cursor: pointer;
  color: #fff; background: var(--color-scheduled); border: 1px solid rgba(10, 132, 255, .4);
}
.primaryBtn:hover:not(:disabled) { background: #0a78ec; }
.primaryBtn:disabled { background: var(--fill-inactive); color: var(--ink-3); cursor: not-allowed; border-color: var(--hairline); }
.previewNote { font-size: 12px; color: var(--ink-3); margin: 0 0 12px; }
.samples { display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 14px; }
.sampleChip { font-size: 12px; padding: 5px 12px; border-radius: var(--r-sm); background: var(--fill-inactive); color: var(--ink-2); border: 1px solid var(--hairline); }
.previewActions { display: grid; gap: 10px; }
.dispatchHint { font-size: 12.5px; color: var(--ink-2); margin: 0; padding: 10px 14px; border-radius: var(--r-sm); background: rgba(10, 132, 255, .05); border: 1px solid rgba(10, 132, 255, .2); }
.previewBtns { display: flex; gap: 10px; }
```

- [ ] **Step 5: 写 create.test.tsx 失败测试**

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import CampaignCreate from "../../../features/campaign/CampaignCreate";
import { api } from "../../../lib/api";

vi.mock("../../../lib/api", () => ({ api: { get: vi.fn(), post: vi.fn() } }));
const setView = vi.fn();
const openReport = vi.fn();
vi.mock("../../../stores/campaignStore", () => ({
  useCampaignStore: (sel: any) => sel({ setView, openReport }),
}));
vi.mock("../../../stores/uiStore", () => ({
  useUiStore: (sel: any) => sel({ setError: vi.fn() }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  (api.get as any).mockResolvedValue({ items: [] });
});

describe("CampaignCreate 建活动表单", () => {
  it("标题/意图空时圈人预览按钮 disabled", () => {
    render(<CampaignCreate />);
    const btn = screen.getByText("圈人预览") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("填表点预览 → 调 create + preview，显示命中数", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c_new" })
      .mockResolvedValueOnce({ campaignId: "c_new", targetCount: 42, samples: [{ wxid: "wx1", name: "张三" }] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "活动A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "7折" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 42 人/)).toBeInTheDocument());
    expect(api.post).toHaveBeenNthCalledWith(1, "/api/campaigns", expect.objectContaining({ title: "活动A", intentText: "7折" }));
    expect(api.post).toHaveBeenNthCalledWith(2, "/api/campaigns/c_new/preview", {});
  });

  it("改条件再预览 → 复用同一 draft 只调 preview，不再 create", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c_new" })
      .mockResolvedValueOnce({ campaignId: "c_new", targetCount: 42, samples: [] })
      .mockResolvedValueOnce({ campaignId: "c_new", targetCount: 8, samples: [] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "活动A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "7折" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 42 人/)).toBeInTheDocument());
    // 改售后条件（作废 preview），再预览
    fireEvent.change(screen.getByDisplayValue("不限"), { target: { value: "in_aftercare" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 8 人/)).toBeInTheDocument());
    // create 仅 1 次（第1次），preview 2 次 → 总 post 3 次
    const createCalls = (api.post as any).mock.calls.filter((c: any[]) => c[0] === "/api/campaigns");
    expect(createCalls).toHaveLength(1);
  });

  it("命中 0 人显示提示", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c0" })
      .mockResolvedValueOnce({ campaignId: "c0", targetCount: 0, samples: [] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "x" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 0 人，调整条件/)).toBeInTheDocument());
  });

  it("红线：无 dispatch 按钮/控件", async () => {
    (api.post as any)
      .mockResolvedValueOnce({ id: "c1" })
      .mockResolvedValueOnce({ campaignId: "c1", targetCount: 5, samples: [] });
    render(<CampaignCreate />);
    fireEvent.change(screen.getByPlaceholderText(/双11/), { target: { value: "A" } });
    fireEvent.change(screen.getByPlaceholderText(/活动要点/), { target: { value: "x" } });
    fireEvent.click(screen.getByText("圈人预览"));
    await waitFor(() => expect(screen.getByText(/命中 5 人/)).toBeInTheDocument());
    // 整个组件不得出现 dispatch/确认推送/推送 触发按钮（只允许"请在 AI 总控对话中 dispatch"提示文字）
    expect(screen.queryByText(/^确认推送$/)).toBeNull();
    expect(screen.queryByText(/^立即推送$/)).toBeNull();
    expect(screen.queryByText(/^dispatch$/i)).toBeNull();
  });
});
```

- [ ] **Step 6: 跑测试确认失败 → 实现 → 通过**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/create.test.tsx`
Expected first: FAIL（组件未建）。实现 Step 1-4 后重跑：5 测试全 passed。

- [ ] **Step 7: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 8: Commit**

```bash
git add frontend/src/features/campaign/ProductMultiSelect.tsx frontend/src/features/campaign/StageSelect.tsx frontend/src/features/campaign/CampaignCreate.tsx frontend/src/features/campaign/Campaign.module.css frontend/src/__tests__/features/campaign/create.test.tsx
git commit -m "feat(campaign-fe): 建活动表单(四维动态圈人+draft复用+预览)+产品多选/stage下拉 picker(无dispatch红线)"
```

---

## Task 5: 列表视图 + index 路由壳 + 频道接线

把 `index.tsx` 从透传变路由壳（按 `view` 渲三视图），建 `CampaignList`，更新 channels.ts 文案。

**Files:**
- Create: `frontend/src/features/campaign/CampaignList.tsx`
- Modify: `frontend/src/features/campaign/index.tsx`（透传 → 路由壳）
- Modify: `frontend/src/features/campaign/Campaign.module.css`（列表样式）
- Modify: `frontend/src/app/channels.ts:170`（subtitle 文案）
- Test: `frontend/src/__tests__/features/campaign/list.test.tsx`（新）

**Interfaces:**
- Consumes: `useCampaignStore`（`view / campaigns / listLoaded / loadCampaigns / setView / openReport`）；`StatusBadge`；`EmptyState`；`campaignStatusTone`/`campaignStatusLabel`（Task 6 提供——本任务先内联定义，Task 6 抽出测试。**为避免顺序耦合，本任务直接在 CampaignList.tsx 内定义并 export 这两个纯函数**，Task 6 只加测试不重定义）。
- Produces: `CampaignFeature`（index default export，路由壳）；`CampaignList`；`campaignStatusTone`/`campaignStatusLabel`（named export）。

- [ ] **Step 1: 写 CampaignList.tsx（含状态映射纯函数）**

```tsx
import { useEffect } from "react";
import { Megaphone, Plus } from "lucide-react";
import { EmptyState } from "../../components/ui/EmptyState";
import { StatusBadge, type StatusTone } from "../../components/ui/StatusBadge";
import { useCampaignStore } from "../../stores/campaignStore";
import type { CampaignListItem } from "../../stores/campaignStore";
import styles from "./Campaign.module.css";

export function campaignStatusTone(status: string): StatusTone {
  switch (status) {
    case "dispatching":
    case "completed": return "running";
    case "previewed":
    case "confirmed": return "scheduled";
    case "canceled": return "blocked";
    default: return "inactive"; // draft / 未知
  }
}

export function campaignStatusLabel(status: string): string {
  switch (status) {
    case "draft": return "草稿";
    case "previewed": return "已预览";
    case "confirmed": return "已确认";
    case "dispatching": return "推送中";
    case "completed": return "已完成";
    case "canceled": return "已取消";
    default: return status;
  }
}

export default function CampaignList() {
  const campaigns = useCampaignStore((s) => s.campaigns);
  const listLoaded = useCampaignStore((s) => s.listLoaded);
  const loadCampaigns = useCampaignStore((s) => s.loadCampaigns);
  const setView = useCampaignStore((s) => s.setView);
  const openReport = useCampaignStore((s) => s.openReport);

  useEffect(() => {
    if (!listLoaded) void loadCampaigns();
  }, [listLoaded, loadCampaigns]);

  return (
    <div className={styles.page}>
      <section className={styles.panel}>
        <div className={styles.head}>
          <div className={styles.headL}>
            <span className={styles.eyebrow}>Campaigns</span>
            <span className={styles.title}>活动列表</span>
          </div>
          <button type="button" className={styles.primaryBtn} onClick={() => setView("create")}>
            <Plus size={14} /> 新建活动
          </button>
        </div>

        {campaigns.length === 0 ? (
          <EmptyState icon={<Megaphone size={28} />} title="还没有活动" hint="点「新建活动」按条件圈人并预览，确认推送在 AI 总控对话中完成。" />
        ) : (
          <table className={styles.table}>
            <thead>
              <tr className={styles.tr}>
                <th className={`${styles.th} ${styles.thName}`}>活动标题</th>
                <th className={styles.th}>状态</th>
                <th className={styles.th} title="已扇出的跟进任务数，非真实送达数">已扇出</th>
                <th className={styles.th} title="圈人命中数，真实送达见结果看板">命中数</th>
                <th className={styles.th}>创建人</th>
                <th className={styles.th}>创建时间</th>
              </tr>
            </thead>
            <tbody>
              {campaigns.map((c: CampaignListItem) => (
                <tr key={c.campaignId} className={`${styles.tr} ${styles.rowClickable}`} data-testid="campaign-row" onClick={() => openReport(c.campaignId)}>
                  <td className={`${styles.td} ${styles.tdName}`}>{c.title}</td>
                  <td className={styles.td}><StatusBadge tone={campaignStatusTone(c.status)}>{campaignStatusLabel(c.status)}</StatusBadge></td>
                  <td className={styles.td}>{c.dispatchedCount}</td>
                  <td className={styles.td}>{c.targetCount ?? "—"}</td>
                  <td className={styles.td}>{c.createdBy}</td>
                  <td className={styles.td}>{c.createdAt ? new Date(c.createdAt).toLocaleString() : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
```

- [ ] **Step 2: index.tsx 变路由壳**

`frontend/src/features/campaign/index.tsx` 全文替换：

```tsx
import { useCampaignStore } from "../../stores/campaignStore";
import CampaignList from "./CampaignList";
import CampaignCreate from "./CampaignCreate";
import CampaignBoard from "./CampaignBoard";

export default function CampaignFeature() {
  const view = useCampaignStore((s) => s.view);
  if (view === "create") return <CampaignCreate />;
  if (view === "board") return <CampaignBoard />;
  return <CampaignList />;
}
```

> 注意：PR #58 现有 `campaign.test.tsx` 已在 Task 3 改为 import `CampaignBoard`，不再依赖 index 的 named export，所以 index 不再需要 re-export `bucketTone`/`bucketLabel`。`csv.ts` import 的 `bucketLabel` 来自 `./CampaignBoard`，不受影响。

- [ ] **Step 3: 加列表行 CSS**

`Campaign.module.css` 末尾追加：

```css
.rowClickable { cursor: pointer; }
.rowClickable:hover { background: rgba(10, 132, 255, .03); }
```

- [ ] **Step 4: 更新 channels.ts 文案**

`frontend/src/app/channels.ts` campaign entry（约 :169-170）的 `title`/`subtitle` 改为反映三视图：

```ts
    title: "活动推送",
    subtitle: "建活动、按购买产品/价值分层圈人预览，查看真实触达分布（已送达/在途/被拦/已请示）。确认推送在 AI 总控对话中完成。",
```

> 命名红线检查：此文案无禁词（活动/推送/圈人/已送达/被拦/已请示 全 AI 中性）。

- [ ] **Step 5: 写 list.test.tsx**

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import CampaignList from "../../../features/campaign/CampaignList";
import { useCampaignStore } from "../../../stores/campaignStore";

const openReport = vi.fn();
const setView = vi.fn();
const loadCampaigns = vi.fn();

function mockStore(over: Record<string, unknown>) {
  (useCampaignStore as any).mockImplementation((sel: any) =>
    sel({ campaigns: [], listLoaded: true, loadCampaigns, setView, openReport, ...over }),
  );
}
vi.mock("../../../stores/campaignStore");

beforeEach(() => vi.clearAllMocks());

describe("CampaignList 列表", () => {
  it("空 campaigns 渲空态", () => {
    mockStore({ campaigns: [] });
    render(<CampaignList />);
    expect(screen.getByText("还没有活动")).toBeInTheDocument();
  });

  it("有 campaigns 渲行数 = 长度 + 列头含「已扇出」文案", () => {
    mockStore({ campaigns: [
      { campaignId: "c1", title: "活动一", status: "completed", targetCount: 100, dispatchedCount: 90, createdBy: "admin", createdAt: "2026-06-28T10:00:00Z" },
      { campaignId: "c2", title: "活动二", status: "draft", dispatchedCount: 0, createdBy: "admin" },
    ] });
    render(<CampaignList />);
    expect(screen.getAllByTestId("campaign-row")).toHaveLength(2);
    expect(screen.getByText("已扇出")).toBeInTheDocument();   // A: 文案区分
    expect(screen.getByText("活动一")).toBeInTheDocument();
    // draft 无 targetCount → 渲 —
    expect(screen.getByText("—")).toBeInTheDocument();
  });

  it("点行触发 openReport(切看板)", () => {
    mockStore({ campaigns: [{ campaignId: "c1", title: "活动一", status: "completed", targetCount: 100, dispatchedCount: 90, createdBy: "a", createdAt: "2026-06-28T10:00:00Z" }] });
    render(<CampaignList />);
    fireEvent.click(screen.getAllByTestId("campaign-row")[0]);
    expect(openReport).toHaveBeenCalledWith("c1");
  });

  it("点新建活动切 create 视图", () => {
    mockStore({ campaigns: [] });
    render(<CampaignList />);
    fireEvent.click(screen.getByText("新建活动"));
    expect(setView).toHaveBeenCalledWith("create");
  });

  it("未加载时触发 loadCampaigns（且失败后不循环：listLoaded=true 即不再调）", () => {
    mockStore({ campaigns: [], listLoaded: false });
    render(<CampaignList />);
    expect(loadCampaigns).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 6: 跑全 campaign 前端测试 + 类型**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/`
Expected: list 5 + create 5 + csv 3 + board-paging 3 + 现有 store/campaign/jump/no-refetch-loop 全 passed。
Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/features/campaign/CampaignList.tsx frontend/src/features/campaign/index.tsx frontend/src/features/campaign/Campaign.module.css frontend/src/app/channels.ts frontend/src/__tests__/features/campaign/list.test.tsx
git commit -m "feat(campaign-fe): 活动列表视图 + index 路由壳(三视图切换) + 频道文案更新"
```

---

## Task 6: 状态映射纯函数测试 + 全套回归

补 `campaignStatusTone`/`campaignStatusLabel` 的独立纯函数测试（6 状态 + 兜底），并跑前端全套 + 后端 lib 回归，确认基线不退。

**Files:**
- Test: `frontend/src/__tests__/features/campaign/campaignStatus.test.ts`（新）

**Interfaces:**
- Consumes: `campaignStatusTone`/`campaignStatusLabel`（Task 5 在 `CampaignList.tsx` export）。

- [ ] **Step 1: 写纯函数测试**

```ts
import { describe, it, expect } from "vitest";
import { campaignStatusTone, campaignStatusLabel } from "../../../features/campaign/CampaignList";

describe("campaignStatusTone / campaignStatusLabel", () => {
  it("6 状态 tone 映射", () => {
    expect(campaignStatusTone("draft")).toBe("inactive");
    expect(campaignStatusTone("previewed")).toBe("scheduled");
    expect(campaignStatusTone("confirmed")).toBe("scheduled");
    expect(campaignStatusTone("dispatching")).toBe("running");
    expect(campaignStatusTone("completed")).toBe("running");
    expect(campaignStatusTone("canceled")).toBe("blocked");
    expect(campaignStatusTone("天外飞仙")).toBe("inactive"); // 兜底
  });
  it("6 状态中文标签 + 未知值返回原值", () => {
    expect(campaignStatusLabel("draft")).toBe("草稿");
    expect(campaignStatusLabel("previewed")).toBe("已预览");
    expect(campaignStatusLabel("confirmed")).toBe("已确认");
    expect(campaignStatusLabel("dispatching")).toBe("推送中");
    expect(campaignStatusLabel("completed")).toBe("已完成");
    expect(campaignStatusLabel("canceled")).toBe("已取消");
    expect(campaignStatusLabel("xyz")).toBe("xyz"); // 诚实兜底
  });
});
```

- [ ] **Step 2: 跑该测试**

Run: `cd frontend && npx vitest run src/__tests__/features/campaign/campaignStatus.test.ts`
Expected: 2 passed。

- [ ] **Step 3: 前端全套回归（确认 346 基线只增不减）**

Run: `cd frontend && npx vitest run --no-file-parallelism`
Expected: 全 passed（PR #58 的 346 + 本期新增 campaign 测试），0 failed。

> 若机器资源紧导致 worker-pool 超时（非测试失败），用 `--no-file-parallelism` 串行跑（已在命令里）。只认 "Tests N passed" 行，worker 启动超时不算失败。

- [ ] **Step 4: 前端构建 + CSS tree-shake 核验**

Run: `cd frontend && npm run build`
Expected: built 成功。
Run: `cd frontend && grep -rl "exportBtn\|pagerBtn\|checkItem\|sampleChip" dist/assets/*.css`
Expected: 至少一个 dist CSS 文件命中（确认新增 CSS module 类名未被 tree-shake 删）。

- [ ] **Step 5: 后端 lib 回归**

Run: `cargo test --lib campaign`
Expected: 投影 2 测试 + 现有 campaign 测试全 passed。
Run: `cargo test --lib`
Expected: ≥ 350 passed, 0 failed（基线不退）。

- [ ] **Step 6: 命名红线自查**

Run: `git diff origin/main...HEAD -- src/routes/ frontend/src/ | grep -nE "^\+" | grep -iE "人工|接管|takeover|hand[ -]?off|人工介入|人工托管"`
Expected: 无输出（新增行无禁词）。

- [ ] **Step 7: Commit**

```bash
git add frontend/src/__tests__/features/campaign/campaignStatus.test.ts
git commit -m "test(campaign-fe): campaignStatusTone/Label 纯函数 6 状态 + 兜底断言"
```

---

## 完成标准

- 后端 `GET /api/campaigns` 列表端点上线（投影不泄漏内部字段，createdAt RFC3339），`cargo test --lib` ≥ 350/0。
- 前端 campaign 频道三视图：列表（枚举历史活动 + 状态 + 点进看板）/ 建活动表单（四维动态圈人 + draft 复用 + 预览，**无 dispatch 按钮**）/ 看板（PR #58 + CSV 导出 + 翻页）。
- 前端全套绿（346 基线 + 新增），`tsc` 0 error，`npm run build` 成功，CSS module 存活。
- 命名红线无禁词，dispatch 红线由 create.test.tsx 守住。
- A/B/C/D 落地：已扇出≠已送达文案（列表列头 title）、建成跳列表（返回列表按钮 + openReport 切看板）、重新圈选提醒（previewNote）、draft 复用（draftCampaignId）。
