# 活动推送结果看板（前端）设计 spec

> 日期：2026-06-28
> 上游后端：`docs/superpowers/specs/2026-06-28-campaign-targeted-push-design.md`（活动定向推送，PR #53）+ `2026-06-28-campaign-sends-report-design.md`（结果查询 7 桶端点，PR #57，已合并 main）
> 分支基线：`feat/campaign-frontend` ← `origin/main`（最新，含 PR #57 的 `/campaigns/:id/sends`）
> 状态：设计评审稿。形态与不变量已定；落码以 §8 核实记录为准。

## 1. 背景与缺口

活动定向推送后端已端到端齐活（main，4 端点）：

| 端点 | 作用 | 现有前端消费 |
| --- | --- | --- |
| `POST /api/campaigns`（create） | 建活动 | 走总控 AI 工具 `wechatagent.preview_campaign`（内部先 create 再 preview） |
| `POST /api/campaigns/:id/preview` | 圈人预览（命中数+抽样） | 同上 |
| `POST /api/campaigns/:id/dispatch` | 确认扇出（恒确认门） | 走总控 AI 工具 `wechatagent.dispatch_campaign` |
| `GET /api/campaigns/:id/sends` | 7 桶推送结果聚合 | **零前端消费 ← 本期缺口** |

**结论**：声明/圈人/dispatch 已能通过总控 AI（command-center 频道）两工具操作，真正零前端消费的只有 `/sends` 的 7 桶结果聚合。运营 dispatch 后看不到「圈中多少人、真送达多少、多少被频控拦/转请示」。**本期只补这个结果看板。**

**关键后端约束**：路由表只有上述 4 端点，**没有 `GET /api/campaigns` 列表端点**。所以前端无法枚举历史活动、无法自行发现 campaignId——看板必须靠外部带入 id 才能查 `/sends`。

## 2. 设计决策（已与用户敲定）

| 维度 | 决策 | 理由 |
| --- | --- | --- |
| 前端范围 | **仅结果看板**（消费 `/sends`） | create/preview/dispatch 已有总控 AI 路径，不重复造表单化 UI；零后端改动 |
| 入口方式 | **从总控 AI 结果跳转**：dispatch_campaign 工具调用成功后，在那条 tool-call 结果下加「查看推送结果」入口，带 campaignId 跳转看板 | 运营本就在总控 AI dispatch，动线最短；不需新增后端 list 端点 |
| 看板深度 | **汇总 + 明细两层**：顶部 7 桶汇总指标块（blocked/canceled/escalated 带 reason 二级细分），下方每人明细表 | 镜像 send-analytics 范式；运营能看到具体哪些人被拦/转请示 |
| 大数据 | **后端一次性返回 + 前端内存按桶筛选/截断**，不翻页 | 与后端 spec §7 一致（单活动人群受 managed 数限，通常几十到几百）；真到万级再另立专题 |

## 3. 总体架构

新建一级频道 `campaign`（沿用 `app/channels.ts` 的 `ChannelDef` 注册范式，与 `send-analytics` 同构）。

```
总控 AI（command-center 频道）
  └─ dispatch_campaign 工具调用成功 → response.campaignId
        └─ [查看推送结果] 入口 → campaignStore.openReport(campaignId)
              ├─ set({ selectedCampaignId })
              ├─ navigationStore.setChannel("campaign")   ← 跨频道跳转
              └─ loadReport(id) → GET /api/campaigns/:id/sends

活动频道（features/campaign）
  selectedCampaignId == null → EmptyState（提示从总控 AI 进入）
  loading                    → 占位 "—"（全局 busy 约定）
  report 就绪                → 看板：页头 → 7 桶汇总指标块 → 明细表（按桶内存筛选）
```

**改动面**：
- 新文件 3：`features/campaign/index.tsx`、`features/campaign/Campaign.module.css`、`stores/campaignStore.ts`。
- 单行接线 3：`types/index.ts`（Channel 联合加 `"campaign"`）、`app/channels.ts`（lazy import + 一条 entry）、`features/command-center/index.tsx`（dispatch_campaign 行加跳转入口）。
- 测试 1 目录：`__tests__/features/campaign/`。
- **零后端改动、零其它频道改动。**

## 4. 数据契约（消费现有端点，已核实 §8）

### 4.1 `GET /api/campaigns/:id/sends` 响应（前端类型源）

```ts
// stores/campaignStore.ts 就近定义并 export（模块私有类型惯例）
export interface CampaignReport {
  campaignId: string;
  title: string;
  status: string;          // draft|previewed|dispatching|completed
  summary: CampaignSummary;
  items: CampaignSendItem[];
}
export interface CampaignSummary {
  targetCount: number;
  sent: number;
  pending: number;
  skipped: number;
  unknown: number;
  blocked: Record<string, number>;    // {reason: count}
  canceled: Record<string, number>;
  escalated: Record<string, number>;
}
export interface CampaignSendItem {
  contactWxid: string;
  name: string;            // 可能空串 → 渲 "—"
  status: string;          // 7 桶之一：sent|pending|blocked|canceled|escalated|skipped|unknown
  reason?: string;         // blocked/canceled/escalated/unknown 带；sent/pending/skipped 不带
}
```

### 4.2 跳转入口的 campaignId 来源

总控 AI 的 `dispatch_campaign` 工具返回 JSON `{campaignId, dispatchedCount, status}`，经 management.rs 原样放进 `CommandToolCall.response`（`Record<string,unknown>`）。前端读 `call.response.campaignId`。

**显示守卫**（防死链）：仅当 `call.toolName === "wechatagent.dispatch_campaign"` 且 `call.status ∈ {"succeeded","executed_unverified"}` 且 `typeof call.response?.campaignId === "string"` 时才渲跳转入口。dry-run（status=`dry_run`）/ 待确认（pending_confirmation，response 为 None）态不渲。

## 5. 组件与状态

### 5.1 `stores/campaignStore.ts`（Zustand，沿用 sendAnalyticsStore 范式）

```
state:
  selectedCampaignId: string | null = null
  report: CampaignReport | null = null
  loading: boolean = false

actions:
  openReport(id):                       // 跨频道跳转入口（command-center 调）
    set({ selectedCampaignId: id, report: null })
    useNavigationStore.getState().setChannel("campaign")
    void get().loadReport(id)
  loadReport(id):
    set({ loading: true })
    try   { const r = await api.get<CampaignReport>(`/api/campaigns/${id}/sends`); set({ report: r }) }
    catch (e) { useUiStore.getState().setError(e instanceof Error ? e.message : String(e)) }
    finally { set({ loading: false }) }
  clear(): set({ selectedCampaignId: null, report: null, loading: false })
```

### 5.2 `features/campaign/index.tsx`（default export，单文件模块）

三态渲染：
- `selectedCampaignId == null` → `<EmptyState title="暂无活动结果" hint="在 AI 总控 dispatch 活动后，点「查看推送结果」进入" />`。
- `loading && !report` → 汇总区数值占位 `—`（不自渲 spinner，沿用全局 busy 约定）。
- `report` 就绪 → 看板（§5.3）。

进入频道时若已有 `selectedCampaignId` 且 `report` 为空且 `!loading`，才触发一次 `loadReport`（仅覆盖「直接切到该频道、未经 openReport」的场景）。经 `openReport` 进入时它已发起加载，此处 `loading=true` 守卫避免重复请求。

### 5.3 看板布局（镜像 send-analytics 的 page→panel→metrics 结构）

```
styles.page
  页头：活动标题 {report.title} + 状态 badge {report.status}
  panel「推送结果总览」(eyebrow=Campaign Result)
    styles.metrics：7 个 metric 块
      sent / pending / skipped / unknown   → label + 大数值
      blocked / canceled / escalated       → label + 合计数值 + reason 二级细分（小字 "reason ×N" 列表）
  panel「推送明细」(eyebrow=Per-Contact)
    桶筛选：7 个可点 chip（全部 / 各桶），内存过滤
    表：固定 --row-h 行高
      列：客户名(name||"—") | wxid(截断) | 状态 StatusBadge | 原因(reason||"—")
    空 items → 表区 EmptyState
```

### 5.4 7 桶 → StatusBadge tone 映射（StatusBadge 仅 5 tone）

纯函数 `bucketTone(bucket: string): StatusTone`：

| 桶 | tone | 色（tokens.css） | 语义 |
| --- | --- | --- | --- |
| `sent` | `running` | 绿 #30D158 | 真送达 |
| `pending` | `scheduled` | 蓝 #0A84FF | 在途/会继续 |
| `blocked` | `blocked` | 红 #FF453A | 被频控/硬约束拦下 |
| `escalated` | `held` | 橙 #FF9F0A | 已转幕后领导请示（待裁决继续） |
| `canceled` | `inactive` | 灰 #8E8E93 | 取消（无后续） |
| `skipped` | `inactive` | 灰 #8E8E93 | 去重跳过 |
| `unknown` | `inactive` | 灰 #8E8E93 | 诚实兜底 |

中文桶标签 `bucketLabel(bucket)`：sent=已送达 / pending=在途 / blocked=被拦 / escalated=已请示 / canceled=已取消 / skipped=去重跳过 / unknown=未知。

### 5.5 `features/command-center/index.tsx` 增量（仅 1 处）

在 tool-call 渲染（现 `PlanStep` 行，index.tsx:286 附近）内，对满足 §4.2 守卫的 `dispatch_campaign` 行，追加一个「查看推送结果」文字按钮，`onClick={() => useCampaignStore.getState().openReport(campaignId)}`。不改动其它 tool-call 渲染逻辑。

## 6. 设计系统纪律（严守，不自由发挥）

- **色彩**：只用 `components/ui/tokens.css` 既有 token，禁硬编码色值。蓝（`--color-scheduled`）仅主操作/可点击；紫（`--color-brand`）仅 AI 身份，不可当普通状态色。7 桶状态色全部走 StatusBadge 既有 tone class，不新造颜色。
- **层级**：四级（App shell / 频道页头 / panel / 行）。panel 内不嵌 card；汇总与明细用两个并列 panel + 内部分隔，不堆叠多余面板、不加第三级持久导航。
- **排版**：页头 28-40px / panel 标题 18px / 行与表 13px / 元数据 10.5-12px；字距 0。
- **列表**：固定 `--row-h` 行高；长 name/wxid 截断或换行不溢出；筛选选中用软蓝填充不用重边框。
- **CSS 落地**：用 `Campaign.module.css` + `import styles`（CSS Modules，避免 memory 记录的 tree-shake 副作用导入坑——绝不用裸 `import "./x.css"`）。
- 写码前对照 `docs/frontend-design-system.md` + 现有 `send-analytics/*.module.css` 范例。

## 7. 错误处理与边界

- **API 失败**（非法 id 400 / 跨 workspace 404 / 网络错）→ `catch` 调 `uiStore.setError`，全局 `GlobalErrorBanner` 显示；看板回落空态，不自渲错误块。
- **空活动**（dispatch 前 / 去重全跳）→ `/sends` 返回 summary 全 0 + items `[]`，正常渲染（汇总全 0、明细表空态），200 不报错。
- **无值字段**（name 空串 / reason 缺）→ 渲 `—`。
- **大 items**（几百行）→ 内存按桶筛选 + 截断，不翻页。
- **跨频道跳转后** `selectedCampaignId` 留在 store，重进频道仍显上次结果，直到下次 `openReport` 覆盖。
- **命名红线**（CI 硬门 check-no-human-takeover 扫 frontend/src 新增行）：禁 `人工/接管/takeover/hand-off/人工介入/人工托管`。看板用 已送达/被拦/已请示 等 AI 中性词，`escalated` 标「已请示」非「转人工」，天然安全；注释也不得引入禁词。

## 8. 范围边界（YAGNI）

**本期做**：
- 新频道 `campaign` + `features/campaign` 结果看板（7 桶汇总 + 明细表）；
- `campaignStore`（含跨频道 `openReport` 跳转）；
- command-center dispatch_campaign 行的「查看推送结果」入口；
- 纯函数（bucketTone/bucketLabel）+ 看板渲染 + 三态 + 跳转守卫的 Vitest 测试。

**本期不做**：
- ❌ 活动列表页（后端无 `GET /campaigns` 列表端点，需先扩后端，超本期）；
- ❌ 建活动表单 / 圈人预览 / 确认 dispatch 的表单化 UI（已走总控 AI 两工具）；
- ❌ 翻页 / CSV 导出 / 跨活动对比 / 时间序列（分析专题）；
- ❌ 任何后端改动。

## 9. 测试

Vitest，放 `frontend/src/__tests__/features/campaign/`，沿用现有前端测试范式（render + assert，零真实网络靠 mock store / mock api）：

1. **bucketTone / bucketLabel 纯函数**：7 桶各自映射到正确 StatusBadge tone + 中文标签（确定性，零渲染）。未知桶兜底 inactive/原值。
2. **看板渲染**：给定 mock `CampaignReport`，断言 7 桶指标块数值正确 + blocked/canceled/escalated 的 reason 二级细分渲染 + 明细表行数 = items 长度。
3. **三态**：`selectedCampaignId=null` 渲 EmptyState；有 report 渲看板；空 items（summary 全 0）渲空明细表态。
4. **桶筛选**：点某桶 chip 后明细表只剩该桶行（内存过滤）。
5. **command-center 跳转守卫**：`dispatch_campaign` 行 status=succeeded 且 response.campaignId 存在 → 渲「查看推送结果」；dry_run / pending_confirmation / 无 campaignId → 不渲（防死链）。

## 10. 深度代码核实记录（2026-06-28，基于 origin/main 最新含 PR #57）

| 断言 | 结论 | 真实代码证据（origin/main） |
| --- | --- | --- |
| 后端仅 4 campaign 端点，无 GET 列表 | CONFIRMED | routes/mod.rs:791-794：POST /campaigns、POST :id/preview、POST :id/dispatch、GET :id/sends |
| `/sends` 响应 = {campaignId,title,status,summary,items} | CONFIRMED | campaigns.rs:589-594 |
| summary 含 blocked/canceled/escalated 三 reason 二级 map + sent/pending/skipped/unknown 标量 | CONFIRMED | campaigns.rs build_sends_summary（PR #57） |
| dispatch_campaign 返回含 campaignId | CONFIRMED | campaigns.rs:376 `json!({ "campaignId": id, "dispatchedCount": dispatched, "status": "completed" })` |
| 工具 response 原样进 CommandToolCall.response | CONFIRMED | management.rs:169 `"response": response`；类型 types/index.ts:244-251 `response?: Record<string,unknown>` |
| dispatch 工具名 namespaced | CONFIRMED | management.rs 工具目录 `wechatagent.dispatch_campaign`；恒确认门 tool_always_requires_confirmation |
| 频道注册 = channels.ts CHANNELS 数组 + types Channel 联合 | CONFIRMED | app/channels.ts ChannelDef 数组、Shell.tsx 分组渲染、types/index.ts Channel 联合 |
| 导航跳转 = navigationStore.setChannel | CONFIRMED | stores/navigationStore.ts（activeChannel/setChannel，无 payload → 用 campaignStore 承载 id） |
| http = lib/api.ts 的 api.get<T> | CONFIRMED | lib/api.ts:52-57 |
| 错误走 uiStore.setError + 全局 banner | CONFIRMED | sendAnalyticsStore catch 范式 + uiStore |
| StatusBadge tone 闭集 = running/scheduled/held/blocked/inactive | CONFIRMED | components/ui/StatusBadge/StatusBadge.tsx：`StatusTone` 5 值 |
| EmptyState props = icon?/title/hint?/action? | CONFIRMED | components/ui/EmptyState/EmptyState.tsx |
| 模块范式 = 单文件 index.tsx default export + Module.css + store 抽到 stores/ | CONFIRMED | features/send-analytics（index.tsx + SendAnalytics.module.css；store 在 stores/sendAnalyticsStore.ts） |
| 前端零 campaign 引用 | CONFIRMED | `git grep -in "campaign\|活动" origin/main -- frontend/src` 零命中 |
| tokens.css 蓝=scheduled/紫=brand | CONFIRMED | components/ui/tokens.css：--color-scheduled #0A84FF、--color-brand #5E5CE6 |

**结论**：所有前端依赖的端点契约、UI 原语、导航/store/api 范式已对 origin/main 最新代码闭环。无残留猜测。
