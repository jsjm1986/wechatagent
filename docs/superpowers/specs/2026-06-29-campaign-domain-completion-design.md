# 活动推送功能域补全（列表页 + 建活动表单 + 看板增强）设计 spec

> 日期：2026-06-29
> 上游：`2026-06-28-campaign-targeted-push-design.md`（定向推送后端）+ `2026-06-28-campaign-sends-report-design.md`（/sends 7桶端点，PR #57 merged）+ `2026-06-28-campaign-frontend-design.md`（结果看板前端，PR #58 merged）
> 分支基线：`feat/campaign-domain-completion` ← `origin/main` c163542（含 PR #57/#58）
> 状态：设计评审稿。形态与不变量已定；落码以 §10 核实记录为准。

## 1. 背景与缺口

活动定向推送已有：4 个后端端点（create/preview/dispatch/sends）+ 总控 AI 两工具（preview_campaign / dispatch_campaign）+ 前端结果看板（PR #58，只能从总控 AI dispatch 结果跳转进入）。

运营动线仍不闭环：

| 缺口 | 现状 |
| --- | --- |
| 看不到历史活动 | 无 `GET /api/campaigns` 列表端点；前端无法枚举"我建过哪些活动、各自什么状态"，刷新即丢 campaignId |
| 建活动只能走 AI 对话 | create/preview 只能在总控 AI（command-center）里调工具完成，无表单化界面 |
| 看板无导出/翻页 | 结果只能在线看，无法导出对账；几百行明细一屏铺开 |

**本期补全这三块，让 campaign 域从"只能总控 AI 操作"变成"有独立可浏览、可操作（建/预览）的运营界面"。**

## 2. 设计决策（已与用户敲定）

| 维度 | 决策 | 理由 |
| --- | --- | --- |
| 收口范围 | **全做**：后端列表端点 + 前端列表页 + 建活动/圈人预览表单 + 看板增强（CSV+翻页） | 把 campaign 域一次性收口完整 |
| 页面组织 | **方案 A：单频道内三视图切换**（list / create / board），不新增频道 | 导航不膨胀；三视图共享同一 campaignStore；建活动整页表单（四维圈人+预览信息量撑不进模态） |
| dispatch 红线 | **前端只做"建活动 + 圈人预览"，dispatch（真发送）不做前端按钮**，仍只走总控 AI 恒确认门 | 全 AI 自治定位：高风险动作（真实触达客户）收口在 AI 恒确认门；运营操作 ≠ 客户对话，建/预览是低风险/只读，可前端化 |
| 圈人表单粒度 | **四维全上 + 动态选项**：productIds 产品多选（拉产品库）/ customerStage 下拉（拉字典）/ aftercare·valueTier 固定枚举 | 精准圈人；数据源端点已现成 |
| 看板增强 | **CSV 导出（纯前端）+ 明细表翻页**；不做跨活动对比/时间序列 | CSV 是运营对账真实需求且零后端；翻页低成本；对比/时序需额外基建超本期 |
| 重复预览 | **复用 draft**：一次建活动会话只产生一个 draft 活动，改条件再预览复用同一 campaignId 只调 preview | 避免反复点预览产生 draft 垃圾 |

## 3. 总体架构

```
campaign 频道（单一一级频道，沿用 PR #58 注册）
  campaignStore.view: "list" | "create" | "board"   ← 频道内视图切换
  │
  ├─ "list"（默认）  GET /api/campaigns
  │     活动列表表格（标题/状态/已扇出数/命中数/创建人/时间）
  │     行点击 → openReport(id)（切 "board"）
  │     「新建活动」按钮 → setView("create")
  │
  ├─ "create"        POST /api/campaigns + POST /:id/preview
  │     整页表单：标题 + 意图 + 四维圈人条件（动态选项）
  │     「圈人预览」→ create(首次)/复用 draft + preview → 就地显示命中数+抽样
  │     提示「确认推送请在 AI 总控对话中 dispatch」+ 重新圈选提醒
  │     建成后默认跳回 "list"（动线枢纽，新活动在最上）
  │
  └─ "board"         GET /api/campaigns/:id/sends（PR #58 已有）
        7桶汇总 + 明细表（+ CSV 导出 + 翻页，本期新增）
        仍可从总控 AI dispatch_campaign 结果跳入（openReport）
```

**文件结构演化**：`features/campaign/index.tsx` 单文件 → `features/campaign/` 目录：

| 文件 | 职责 | 状态 |
| --- | --- | --- |
| `index.tsx` | 路由壳：按 `view` 渲三视图之一 | 改（从看板单文件变路由壳） |
| `CampaignList.tsx` | 列表视图 | 新 |
| `CampaignCreate.tsx` | 建活动 + 圈人预览表单 | 新 |
| `CampaignBoard.tsx` | 结果看板（搬现有 + CSV/翻页）；**现 index.tsx 导出的 `bucketTone`/`bucketLabel`/`CampaignFeature` 看板逻辑搬来此处** | 改（搬+增强） |

**拆分迁移注意**：PR #58 现有 `features/campaign/index.tsx` 导出 `bucketTone`/`bucketLabel`（被 `__tests__/features/campaign/campaign.test.tsx` import）。拆分后这两个纯函数随看板逻辑搬到 `CampaignBoard.tsx`。两种兼容选择（计划阶段定）：(a) 测试 import 路径改指 `CampaignBoard`；(b) `index.tsx` 路由壳 re-export `bucketTone`/`bucketLabel` 保旧 import 不断。本期采 **(a)**——测试跟随源码搬迁，不留 re-export 兜底（避免 barrel 残留）。现有 `campaign.test.tsx` 的看板渲染断言（mock store 返回 report→看板）随之指向新的 `CampaignBoard` 或经 `index.tsx` 路由壳（view="board"）渲染，二选一在计划锁定。
| `ProductMultiSelect.tsx` | 产品多选原语（拉 /api/products） | 新 |
| `StageSelect.tsx` | 客户阶段下拉原语（拉字典） | 新 |
| `Campaign.module.css` | 共享样式（扩充） | 改 |
| `csv.ts` | CSV 生成纯函数 | 新 |

**改动面**：
- 后端：`src/routes/campaigns.rs`（+1 list handler + `CampaignListItem` 投影 struct + 单测）、`src/routes/mod.rs`（+1 路由）。**零写链路、零 gateway/worker/model 改动。**
- 前端：campaign 目录拆分 + 3 子视图 + 2 picker 原语 + csv 纯函数 + `campaignStore` 扩状态 + 测试。
- 复用（零改动）：`GET /api/products?active_only=true`、`GET /api/admin/taxonomies?kind=customer_stage`。

## 4. 后端：`GET /api/campaigns` 列表端点

```
GET /api/campaigns
  鉴权：AuthenticatedAdmin（沿用现有 admin 会话中间件）
  IDOR：campaigns.find({ workspace_id: admin.current_workspace })← 只返本 workspace
  排序：FindOptions sort { createdAt: -1 }（最新在前）
  无分页（活动数量本身有限；真到千级再另立专题）
```

### 4.1 `CampaignListItem` 投影 struct（关键：不裸序列化 Campaign）

**为何投影**：裸 `Campaign` 序列化会泄漏 `workspace_id`/`segment_filter`/`intent_text`，且 `created_at: DateTime` 经 serde_json 变 `{"$date":...}` 破坏前端 string 契约（models.rs:260-261 警告）。范式照 `list_products`（products.rs:138，返回 `ProductView::from(&p)` 投影而非裸 model）。

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CampaignListItem {
    campaign_id: String,                      // c.id.to_hex()
    title: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_count: Option<i64>,                // draft 没预览过 → None → 字段缺失
    dispatched_count: i64,
    created_by: String,
    created_at: Option<String>,               // dt_to_string(c.created_at) RFC3339
}
```

投影是纯函数 `From<&Campaign>`（可单测）：`campaign_id` 取 `id.map(|i| i.to_hex())`、`created_at` 用 `crate::models::dt_to_string`（models.rs:3356，RFC3339 `Option<String>`）。

### 4.2 响应形态

```json
{ "items": [
  { "campaignId":"...", "title":"双11老客7折", "status":"completed",
    "targetCount":500, "dispatchedCount":470, "createdBy":"admin",
    "createdAt":"2026-06-28T..." }
] }
```

- `targetCount` 是"圈人命中数"（预览/dispatch 时算）；`dispatchedCount` 是"已扇出 follow_up 任务数"。**两者都不是"真送达数"**——真送达要点进看板看 sent 桶（§6.A 文案区分）。
- 空结果 → `{items:[]}`，200。

## 5. 前端：campaignStore 扩充

现有（PR #58）：`selectedCampaignId / report / loading / lastAttemptedId` + `openReport / loadReport / clear`，**全部保留不动**。新增：

```
state（新增）:
  view: "list" | "create" | "board" = "list"
  campaigns: CampaignListItem[] = []
  listLoading: boolean = false
  listLoaded: boolean = false               // 防重复加载 + 防失败重试循环
  page: number = 0                          // 看板明细翻页(0-based)

actions（新增）:
  setView(v): set({ view: v })
  loadCampaigns():
    set({ listLoading: true, listLoaded: true })   // 进入即标记(失败也不重试,延续 PR#58 lastAttemptedId 教训)
    try { const r = await api.get<{items:CampaignListItem[]}>("/api/campaigns"); set({ campaigns: r.items }) }
    catch (e) { useUiStore.getState().setError(...) }
    finally { set({ listLoading: false }) }
  setPage(n): set({ page: n })

actions（改）:
  openReport(id): set({ ..., view: "board", page: 0 })  // 多设 view+page,从总控AI跳入直接落看板
  clear(): set({ ..., view: "list", campaigns: [], listLoaded: false, page: 0 })
```

前端类型（campaignStore.ts 就近 export）：

```ts
export interface CampaignListItem {
  campaignId: string;
  title: string;
  status: string;                 // draft|previewed|confirmed|dispatching|completed|canceled
  targetCount?: number;           // None 时 wire 缺失 → 渲 "—"
  dispatchedCount: number;
  createdBy: string;
  createdAt?: string;             // RFC3339 → new Date(s).toLocaleString()
}
```

## 6. 前端：三视图

### 6.1 CampaignList.tsx（列表，默认视图）

- 进入 list 视图且 `!listLoaded` → `loadCampaigns()`（effect 守卫 `listLoaded`，仿 PR #58 防循环：失败后 `listLoaded=true` 不再自动重取）。
- 表格列：标题 / 状态 StatusBadge(§7) / **已扇出**(dispatchedCount) / 命中数(targetCount ?? "—") / 创建人 / 创建时间。
  - **A. 文案区分**：列头明确写"已扇出"而非"已送达"，避免与看板 sent 混淆；命中数列头加 title 提示"圈人命中数，真实送达见结果看板"。
- 行点击 → `openReport(campaignId)`（切 board）。
- 顶部「新建活动」按钮 → `setView("create")`。
- 空 → EmptyState「还没有活动，点新建活动开始」。

### 6.2 CampaignCreate.tsx（建活动 + 圈人预览，整页表单）

字段：
- 标题（text，必填）
- 活动意图（textarea，必填，注入 follow_up content 喂 Reply Agent）
- 四维圈人条件：
  - productIds：`ProductMultiSelect`（拉 `/api/products?active_only=true`，存 productId 数组）
  - customerStage：`StageSelect`（拉 `/api/admin/taxonomies?kind=customer_stage`，存 value.id）
  - aftercare：固定下拉（不限 / in_aftercare / expired）
  - valueTier：固定下拉（不限 / high / mid / low）

动线：
- 标题或意图空 → 「圈人预览」按钮 disabled（前端先校验，后端 campaigns.rs:199 有 400 兜底）。
- 点「圈人预览」：
  - **D. 复用 draft**：首次 → `POST /api/campaigns`(create) 拿 `draftCampaignId` → `POST /:id/preview`；改条件再点 → 复用 `draftCampaignId` 只调 preview（preview 会更新 targetCount）。一次建活动会话只产生一个 draft。
  - 预览结果就地显示：命中数 + 抽样（≤5 个名/wxid）。
  - **C. 重新圈选提醒**：命中数旁注"实际推送时会重新圈选，人数可能微调"（dispatch 时重新圈人，campaigns.rs:296）。
  - 命中 0 人 → 提示"命中 0 人，调整条件再试"（非错误）。
- 预览后区域：提示「确认推送请在 AI 总控对话中对该活动 dispatch」+「查看结果看板」入口（openReport→board）+「返回列表」。
- **B. 建成默认跳列表**：明确建活动后默认 `setView("list")`（新活动在最上，动线枢纽）。
- **红线：无 dispatch 按钮**（§9 测试断言守住）。

### 6.3 CampaignBoard.tsx（看板，搬 PR #58 + 增强）

- 现有保留：7桶汇总 + 明细表 + 桶筛选 + `lastAttemptedId` 防循环 + `bucketTone`/`bucketLabel`。
- **CSV 导出**：把当前 `report.items` 经 `csv.ts` 纯函数转 CSV（表头 `客户名,wxid,状态,原因`，status 用 `bucketLabel` 中文桶名、reason 原值），`Blob`+`URL.createObjectURL` 下载，文件名 `campaign-{campaignId}-sends.csv`。items 空时按钮 disabled。
- **翻页**：明细表按 `page` + 固定 `PAGE_SIZE=50` 内存切片；桶筛选与翻页协同——切桶时 `setPage(0)`；`page` 超界自动钳制。

## 7. 状态/桶 → StatusBadge tone 映射

### 7.1 活动 status（列表用，6 状态 → 5 tone）

纯函数 `campaignStatusTone` / `campaignStatusLabel`（与看板 bucketTone 同范式）：

| status | tone | 色 | label |
| --- | --- | --- | --- |
| draft | inactive | 灰 | 草稿 |
| previewed | scheduled | 蓝 | 已预览 |
| confirmed | scheduled | 蓝 | 已确认 |
| dispatching | running | 绿 | 推送中 |
| completed | running | 绿 | 已完成 |
| canceled | blocked | 红 | 已取消 |
| 其它 | inactive | 灰 | 原值（诚实兜底） |

### 7.2 看板 7 桶（PR #58 已有，不动）

sent→running / pending→scheduled / blocked→blocked / escalated→held / canceled·skipped·unknown→inactive。

## 8. 两个自建 picker 原语

放 `features/campaign/` 内（非全局 `components/ui/`，目前只 campaign 用，YAGNI；将来别处要用再上提）。

- `ProductMultiSelect`：mount 拉 `/api/products?active_only=true` → `{items:[{productId,name}]}`；checkbox 多选，受控（`value: string[]` / `onChange`）；存 `productId`。加载失败显示"选项加载失败"不阻断其它字段。
- `StageSelect`：mount 拉 `/api/admin/taxonomies?kind=customer_stage`（**只带 kind，不带 scope**——kind 已足够窄，返回所有 scope 下 active 的 customer_stage 值；服务端默认只返 value.status=active）→ `{items:[{value:{id,label}}]}`；单选下拉，受控；存 `value.id`、显 `value.label`。同上失败降级。

## 9. 设计系统纪律 + 红线

- **色彩**：只用 `components/ui/tokens.css` token；蓝（--color-scheduled）仅主操作/可点击；紫（--color-brand）仅 AI 身份。状态色走 StatusBadge 既有 tone，不新造。
- **CSS**：`Campaign.module.css` + `import styles`（CSS Modules，避 tree-shake 副作用导入坑——绝不裸 `import "./x.css"`）。
- **层级**：四级（App shell / 频道页头 / panel / 行）；表单与列表用 panel，不堆叠多余面板。
- **命名红线**（CI 硬门 check-no-human-takeover 扫 frontend/src + src/routes 新增行）：禁 `人工/接管/takeover/hand-off/人工介入/人工托管`。用 AI 中性词（活动/推送/命中/已送达/被拦/已请示/草稿）。建活动"确认推送请在 AI 总控对话中 dispatch"是 AI 中性表述，天然安全。
- **dispatch 红线**：前端绝不做 dispatch 按钮，真发送只走总控 AI 恒确认门。

## 10. 测试

### 10.1 后端（lib 单测，零 DB）

- `CampaignListItem::from(&Campaign)` 投影纯函数：
  - `created_at` → RFC3339 string（dt_to_string）；
  - `target_count` None 时序列化字段缺失（serde_json::to_value 断言无 `targetCount` key）；
  - 不泄漏 `workspaceId`/`segmentFilter`/`intentText`（投影结构体根本无这些字段，序列化断言无对应 key）；
  - `campaign_id` = id.to_hex()。
- list handler 走 DB → 归 CI integration（与 /sends 同策略，本地不强求）。

### 10.2 前端（Vitest，mock api/store）

1. `campaignStatusTone`/`campaignStatusLabel`：6 状态各映射正确 + 未知值兜底 inactive/原值。
2. `csv.ts` 生成纯函数：items → 正确 CSV（表头、中文桶名、reason、含逗号/引号/换行值的双引号转义）。空 items → 仅表头。
3. 列表视图：mock campaigns → 行数 = items 长度；空 → 空态；行点击触发 openReport（view→board）；列头含"已扇出"文案（A）。
4. 建活动视图：填表→点预览→断言调 create+preview；改条件再预览→复用同一 campaignId 只调 preview（D）；命中 0 人渲提示（C）；**断言无 dispatch 按钮/控件**（红线）。
5. 看板翻页：items > PAGE_SIZE → 只渲一页；翻页换行；切桶 setPage(0)。
6. 防循环延续：列表 loadCampaigns 失败后不重试循环（仿 PR #58 no-refetch-loop：失败后 listLoaded=true，effect 不再触发）。

### 10.3 测试基线纪律

新增测试只增量叠加，不动现有 campaign 测试（PR #58 的 store/board/jump/no-refetch-loop 全保留）；前端全套保持绿（346 基线只增不减）；后端 lib 基线 ≥350/0 不回退。

## 11. 范围边界（YAGNI）

**本期做**：GET /api/campaigns 列表端点 + 列表页 + 建活动/圈人预览表单（四维动态）+ 看板 CSV/翻页 + draft 复用 + A/B/C 文案动线补充。

**本期不做**：
- ❌ 活动编辑/删除（后端无 update/delete 端点，需扩后端，超收口范畴）；
- ❌ 活动取消（canceled 状态存在但无端点触发，同上）；
- ❌ 前端 dispatch 按钮（红线：真发送只走总控 AI 恒确认门）；
- ❌ 列表搜索/筛选/分页（活动数量有限）；
- ❌ 跨活动对比/时间序列趋势（需额外基建，分析专题）；
- ❌ 任何 gateway/worker/发送链路改动。

## 12. 深度代码核实记录（2026-06-29，基于 origin/main c163542，主 agent 亲自逐行核实）

| 断言 | 结论 | 真实代码证据（origin/main） |
| --- | --- | --- |
| Campaign model camelCase，status 闭集 6 值 | CONFIRMED | models.rs:552 `rename_all="camelCase"`、:613 `ALLOWED_CAMPAIGN_STATUS = [draft,previewed,confirmed,dispatching,completed,canceled]` |
| Campaign.target_count Option 带 skip_serializing_if=None | CONFIRMED | models.rs:566 `#[serde(default, skip_serializing_if="Option::is_none")] target_count: Option<i64>`——None 时字段缺失非 null |
| 裸 DateTime 序列化成 {$date} 破坏前端 string 契约 | CONFIRMED | models.rs:260-261 注释明确警告；投影须转换 |
| dt_to_string 返 RFC3339 Option<String> | CONFIRMED | models.rs:3356 `pub fn dt_to_string(dt)->Option<String> { dt.try_to_rfc3339_string().ok() }` |
| list 端点须投影非裸序列化（范式） | CONFIRMED | products.rs:138 list_products 返回 `json!({"items": items})`，items=`ProductView::from(&p)` 投影；ProductView 字段 productId/name |
| 产品端点 GET /api/products?active_only 返 {items:[{productId,name}]} | CONFIRMED | mod.rs:787 `/products` get(list_products)；products.rs:44 active_only query、:85 ProductView{product_id,name,...}、:163 `{items}` |
| 字典端点 GET /api/admin/taxonomies?kind 返 {items:[{value:{id,label}}]} | CONFIRMED | mod.rs:818 `/admin/taxonomies` get(list_taxonomies)；admin_taxonomies.rs:38 kind query、:123 `{items}`、:281 taxonomy_entry_json value:{id,label,displayName,status} |
| 字典端点只读无 admin 鉴权、taxonomy 全局 | CONFIRMED | admin_taxonomies.rs:89 list_taxonomies 签名无 Extension<AuthenticatedAdmin>；默认只返 value.status=active |
| 前端字典消费先例 | CONFIRMED | frontend system-strategy/index.tsx:642 `api.get<{items:TaxonomyEntry[]}>("/api/admin/taxonomies?...")`、:622 kind="customer_stage" |
| db accessor campaigns/products/campaign_sends/system_taxonomies 全在 | CONFIRMED | db/mod.rs:386 campaigns()、:381 products()、:391 campaign_sends()、:247 collection_system_taxonomies() |
| CreateCampaignRequest = {title,intentText,segmentFilter} | CONFIRMED | campaigns.rs:111-118 camelCase；SegmentFilter models.rs:577 {productIds[],aftercare?,valueTier?,customerStage?} |
| dispatch 重新圈人（预览数≠真推数依据） | CONFIRMED | campaigns.rs:296 注释"重新跑圈人（防预览后数据漂移）"、:297 resolve_segment_contacts |
| dispatch 命中 0 人 400 | CONFIRMED | campaigns.rs:304-306 `if hits.is_empty() return BadRequest` |
| campaign 频道前端现状=单文件看板（PR#58） | CONFIRMED | frontend/src/features/campaign/ 仅 index.tsx + Campaign.module.css；campaignStore 仅 loadReport/openReport |
| /sends 端点已在 HEAD | CONFIRMED | campaigns.rs:484 campaign_sends_report、mod.rs:794 route |

**结论**：所有后端契约、UI 原语、导航/store/api 范式、数据源端点已对 origin/main c163542 逐行核实闭环。唯一后端新增=GET /api/campaigns 列表端点（+投影 struct）；其余全前端。无残留猜测。
