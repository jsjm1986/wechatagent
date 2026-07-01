# 内容资产 / 知识库 前端概念混淆消除（导航文案对齐）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用纯导航文案对齐消除「内容资产（素材库）」与「知识库 Wiki」在前端菜单层的概念混淆，且让被 subtitle 藏住的知识录入口显形。

**Architecture:** 纯前端文案改动，改动面 = `frontend/src/app/channels.ts` 两个频道对象的显示文案字段（label/caption/eyebrow/title/subtitle）。侧栏按钮文本来自 `channels.ts` 的 `label`（`Shell.tsx:235` 渲染 `{c.label}`），页头 eyebrow/title/subtitle 来自同一对象（`Shell.tsx:267-269`）。频道 `id`、分组结构、组件、store、后端全不动。

**Tech Stack:** Vite + React 19 + TypeScript（前端 admin，无路由库，频道即导航单元）。

## Global Constraints

- 只改 `frontend/src/app/channels.ts` 两个频道的**显示文案字段**；`id`（`content` / `knowledgeWiki`）、`group`、`icon`、`Component`、频道在数组中的顺序全部逐字不动。
- 不动任何页面组件、store、后端、CSS、路由。不重排导航分组、不移动「专属顾问」。
- 文案为「知识/素材/话术/录入/审核/问答」等中性词，**不得**引入 no-human-takeover 禁词集（`human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`）。
- 侧栏按钮文本 = `label`。改 `knowledgeWiki` 的 `label`（`Wiki 管理`→`知识库 Wiki`）会连带影响任何按 `"Wiki 管理"` 文本定位侧栏按钮的工具 —— 已核实 `frontend/walkthrough.py:84/87` 字面依赖该文本（本地手动静态走查工具，**不在任何 CI job**；`.github` 全目录无 python/playwright/pytest 引用）。为保持工具可用，同一 PR 内同步更新它。
- 基线：本 worktree（`prompt-evolution`）落后于 origin/main（缺 #74/#76/#77）。本改动要进 main，须基于**最新 origin/main** 开新分支 `fix/nav-content-vs-knowledge`（用临时 worktree，避免在旧分支上堆叠无关历史）。

---

### Task 1: content 频道文案去「知识」标签 + 点明与知识库的分工

**Files:**
- Modify: `frontend/src/app/channels.ts:118-128`（`content` 频道对象）

**Interfaces:**
- Consumes: 无（改的是数据字面量，`ChannelDef` 类型不变）。
- Produces: `content` 频道的 `caption` / `eyebrow` / `subtitle` 三个字符串值变更；`id`/`group`/`label`/`icon`/`title`/`Component` 不变。后续 Task 无依赖此变更。

改动逐字对照（**只改这 3 个字段的值**）：

| 字段 | 现状值 | 改为 |
| --- | --- | --- |
| caption | `素材知识` | `话术 / 素材` |
| eyebrow | `Knowledge Assets` | `Content Assets` |
| subtitle | `维护产品资料、FAQ、话术、禁用表达、品牌语气和朋友圈素材。` | `维护 AI 可直接引用发送的话术、FAQ、品牌口吻、禁用表达与文件素材。事实依据与产品口径以知识库为准。` |

`label`（`内容资产`）、`title`（`内容资产`）、`id`（`content`）、`group`（`知识`）保持逐字不动。

- [ ] **Step 1: 应用三处文案改动**

编辑 `frontend/src/app/channels.ts` 的 `content` 频道对象（当前 :118-128）：

```ts
  {
    id: "content",
    group: "知识",
    label: "内容资产",
    caption: "话术 / 素材",
    icon: FileText,
    eyebrow: "Content Assets",
    title: "内容资产",
    subtitle: "维护 AI 可直接引用发送的话术、FAQ、品牌口吻、禁用表达与文件素材。事实依据与产品口径以知识库为准。",
    Component: ContentAssetsFeature,
  },
```

- [ ] **Step 2: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error（纯字符串值改动，类型不变）。

- [ ] **Step 3: grep 确认旧文案已无残留、新文案已落地**

Run: `cd frontend && grep -n "素材知识\|Knowledge Assets" src/app/channels.ts`
Expected: 无输出（旧值已全部替换；注意 `Knowledge Wiki` 是 knowledgeWiki 的 eyebrow，Task 2 保留，本 grep 用完整词 `Knowledge Assets` 不误伤）。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/app/channels.ts
git commit -m "fix(fe): 内容资产频道去「知识」标签,文案点明与知识库分工"
```

---

### Task 2: knowledgeWiki 频道改名「知识库 Wiki」+ subtitle 如实暴露录入/审核/问答职责

**Files:**
- Modify: `frontend/src/app/channels.ts:185-194`（`knowledgeWiki` 频道对象）
- Modify: `frontend/walkthrough.py:84,87`（本地手动走查工具，按 `label` 文本定位侧栏按钮 —— label 改名的必然连带）

**Interfaces:**
- Consumes: 无。
- Produces: `knowledgeWiki` 频道的 `label` / `caption` / `title` / `subtitle` 四字段值变更；`id`/`group`/`icon`/`eyebrow`/`Component` 不变。侧栏按钮文本随 `label` 变。

改动逐字对照（channels.ts，**只改这 4 个字段的值**）：

| 字段 | 现状值 | 改为 |
| --- | --- | --- |
| label | `Wiki 管理` | `知识库 Wiki` |
| caption | `schema / 信号 / 历史` | `录入 / 审核 / 问答` |
| title | `Wiki 管理` | `知识库 Wiki` |
| subtitle | `管理知识库领域 schema、缺口信号与切片修订历史。` | `录入与审核 AI 的已验证知识内容（导入、问答、待评审），并管理领域 schema、缺口信号与修订历史。` |

`eyebrow`（`Knowledge Wiki`）、`id`（`knowledgeWiki`）、`group`（`知识`）、`icon`（`FileBox`）保持逐字不动。

> **为何 subtitle 这样写**：探索证实该频道实际是知识内容录入（ImportWizard）+ 审核（ReviewView）+ 问答 + schema 的全功能工作站（`knowledge/index.tsx`）。旧 subtitle 只写「schema/信号/历史」运维向，会让想录知识的管理员扑空。新 subtitle 把「录入/审核/问答」提到最前，schema/信号/历史降为次要，如实反映功能重心。

- [ ] **Step 1: 应用 channels.ts 四处文案改动**

编辑 `frontend/src/app/channels.ts` 的 `knowledgeWiki` 频道对象（当前 :185-194）：

```ts
  {
    id: "knowledgeWiki",
    group: "知识",
    label: "知识库 Wiki",
    caption: "录入 / 审核 / 问答",
    icon: FileBox,
    eyebrow: "Knowledge Wiki",
    title: "知识库 Wiki",
    subtitle: "录入与审核 AI 的已验证知识内容（导入、问答、待评审），并管理领域 schema、缺口信号与修订历史。",
    Component: KnowledgeFeature,
  },
```

- [ ] **Step 2: 同步 walkthrough.py 的两处 label 文本依赖**

`walkthrough.py:84` 断言侧栏含该频道、`:87` 按文本点击进入。侧栏按钮渲染 `label`（`Shell.tsx:235`），label 改名后这两处字面量必须同步，否则本地走查工具会找不到按钮而失败。

编辑 `frontend/walkthrough.py:84`：

```python
    check("知识库 Wiki" in body, "侧栏含「知识库 Wiki」频道")
```

编辑 `frontend/walkthrough.py:87`：

```python
    page.locator("aside button", has_text="知识库 Wiki").first.click(timeout=5000)
```

（`:86` 的注释 `# ========== 2. 进 Wiki 管理频道 → 切「治理」模式 ==========` 可选改为「进 知识库 Wiki 频道」，不影响执行，为一致性建议一并改。）

- [ ] **Step 3: 类型检查**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 4: grep 确认 channels.ts 内旧 label/caption/title 已无残留**

Run: `cd frontend && grep -rn "Wiki 管理\|schema / 信号 / 历史" src/`
Expected: 无输出（`src/` 下 `Wiki 管理` 与旧 caption 全部替换完毕）。

Run: `cd frontend && grep -rn "Wiki 管理" walkthrough.py`
Expected: 无输出（走查工具两处 label 断言/定位已同步；`:86` 注释若已改则一并无输出，若保留注释里的「Wiki 管理」词则允许命中该注释行 —— 注释不影响 Playwright 执行，可接受）。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/app/channels.ts frontend/walkthrough.py
git commit -m "fix(fe): Wiki管理→知识库Wiki,subtitle如实暴露录入/审核/问答职责"
```

---

### Task 3: 整体验证（前端构建 + 组件测试 + 红线 lint）

**Files:**
- 无改动（纯验证 Task 1/2 的合并结果）。

**Interfaces:**
- Consumes: Task 1 + Task 2 的全部 channels.ts / walkthrough.py 改动。
- Produces: 无。

- [ ] **Step 1: 类型检查（合并结果）**

Run: `cd frontend && npx tsc --noEmit`
Expected: 0 error。

- [ ] **Step 2: 前端构建**

Run: `cd frontend && npm run build`
Expected: 构建成功，无报错（无组件/CSS 改动，仅数据字面量变更，dist 正常产出）。

- [ ] **Step 3: 组件测试（vitest，CI frontend-contract job 的等价物）**

Run: `cd frontend && npx vitest run --pool=forks --maxWorkers=2`
Expected: 全绿。特别确认 `src/__tests__/features/content-assets/contentAssets.test.tsx` 不回归 —— 它断言页面内标题「内容资产库」（在 `content-assets/index.tsx:133`，**不是** channels.ts，本计划不动），以及「素材 URL」「朋友圈素材」页面内文案（同样不在本计划改动面），故应保持通过。

> 注：本 worktree 的 vitest 需 `--pool=forks --maxWorkers=2`，默认 threads/全量 forks 会超时（见 wiki-audit 经验）。CI 上 `npx vitest run` 无此约束。

- [ ] **Step 4: no-human-takeover 红线 lint**

Run: `bash scripts/check-no-human-takeover.sh <BASE> HEAD`（`<BASE>` = 分支起点 commit 或 `origin/main`）
Expected: 0 violations。新增文案「话术/素材/录入/审核/问答/知识库/事实依据/产品口径」均为中性词，不含禁词集（`human_takeover|takeover|hand-off|人工接管|人工介入|人工托管|接管|人工`）。

> 说明：纯前端 PR（只碰 `frontend/**`）在 CI 上**不触发** backend baseline job（no-human-takeover lint 在其中），但本地跑一次做诚实自检。frontend-contract job（tsc + vitest）才是本 PR 的 CI 合并门。

- [ ] **Step 5: 人工目视（可选，若 dev server 起得来）**

Run: `cd frontend && npm run dev`，浏览器开 admin。
Expected: 左侧「知识」组下 —— 内容资产 caption 显「话术 / 素材」、知识库 Wiki caption 显「录入 / 审核 / 问答」；点进各频道，页头 eyebrow/title/subtitle 与新文案一致。

- [ ] **Step 6: 无独立提交**（本 Task 仅验证，Task 1/2 已各自提交）

---

## Self-Review

**1. Spec coverage：**
- spec §3.A content 频道 3 字段（caption/eyebrow/subtitle）→ Task 1 ✅
- spec §3.B knowledgeWiki 频道 4 字段（label/caption/title/subtitle）→ Task 2 ✅
- spec §5 验证（tsc / build / no-human-takeover / grep 无残留）→ Task 3 ✅
- spec §5 第 5 点「grep 确认无测试硬编码引用」的假设**被证伪**：`walkthrough.py:84/87` 字面依赖 `Wiki 管理`。计划已在 Task 2 显式纳入 walkthrough.py 同步（spec 未覆盖的连带项，实现时补上）。
- spec §6 不做项（不重排分组/不动 id/不动组件/不动页面内文案）→ Global Constraints + 各 Task 逐字不动清单 ✅

**2. Placeholder scan：** 无 TBD/TODO；每个 code step 都给出完整的目标对象字面量与精确命令。

**3. Type consistency：** 全程只改字符串值，`ChannelDef` 类型与字段名不变；无跨 Task 的函数/类型签名依赖。

## 执行与落地

- 改动极小（channels.ts 7 个字符串字段 + walkthrough.py 2 行文本）。可主会话直接改，也可 Subagent-Driven（Task 1/2 独立可并行审）。给定极小体量，**推荐主会话 inline 执行**。
- **基线**：本 worktree 落后于 origin/main。执行须基于**最新 origin/main** 开新临时 worktree + 新分支 `fix/nav-content-vs-knowledge`，避免在 wiki-audit 旧分支上堆叠无关历史。
- 推送后开 PR → 挂 cron 监看 CI（frontend-contract job）全绿自动合并；CI 失败不自动重试、不自行改代码，报告等用户指示。

