# 运营透视 Observability 频道页 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增第 10 个频道页 `observability.html`，用纯业务语言向运营管理者/老板讲透"AI 不是黑箱"——分 3 层 18 个板块呈现单次决策可回放、客户画像持续观测、系统态势一屏，并在全站 10 个页面接入导航。

**Architecture:** 纯静态 HTML + 一份专属 CSS（`assets/observability.css`），完全复用 `shared.css` 的 design token 与 `shared.js` 的零依赖交互引擎（语言切换 / 移动菜单 / reveal 动画 / 复制）。新页结构对齐既有子页（`trust.html` 为范本）：`nav` + `mobile-menu` + `page-hero section-deep` + 多个 `section` + `footer` + 两个 `<script>`。无任何可运行 JS 看板，仿真"运营台界面"全部是静态 HTML/CSS 摆出来的截面。

**Tech Stack:** HTML5、CSS3（Grid/Flex、CSS 变量）、既有 vanilla JS（不新增脚本）。本地静态预览 `python -m http.server`，渲染自检 Playwright（headless Chromium）。

## Global Constraints

每个任务的要求都隐含包含本节，逐条照抄自设计文档 `docs/superpowers/specs/2026-06-28-observability-channel-design.md`，值不可改：

- **零技术黑话**：全页不出现任何 API 路径、数据库集合名、代码字段名、内部状态字符串（禁止出现 `held_by_ai_policy` / `agent_run_logs` / `tool_trace` / `final_review_status` / `manual_tags` / `bayesian_signals` 等任何技术标识符）。这与 `trust.html` / `technology.html` 不同——那两页可以出现技术标识符，本页一律不可。
- **诚实边界（不可破）**：
  - 贝叶斯信号 / 大五人格 = 纯观测、永不驱动决策。只写"持续观测、沉淀走势、诚实标注置信、运营者可查可复盘"，并把这条边界当卖点讲透（避免人格标签偏见、证据不足如实标注、不拿没把握的猜测预设客户）。**绝不写**"用画像自动调话术 / 贝叶斯实时决定怎么回复"。不提契约测试名，用大白话讲价值。
  - 真正"驱动行为"的功劳只归：关系分化（客户 / 同行 / 朋友语气节奏不同）、每联系人口吻指令、意图轨迹。
  - **意图轨迹**：真实存在且真改变行为，但后端不对运营者暴露读视图——本页**不**把它列为"运营者能观察到"的维度；如需提及只用链接引到 `technology.html`，不在可观测清单里宣称可见。
  - **对话模式**：只写"判定规则可由运营者按行业配置"，**不**写"逐次用了哪个模式都能看到"。
  - **当日发送上限**：写"可观察到触顶 / 退避的提醒"，**不**写"实时剩余还能发几条"。
  - **用量成本**：写"调用次数、用量、缓存命中率看得到"，**不**折算成具体金额。
  - **自我批判**：前端确有展示，**可写**"AI 每次自我反省都留痕可看"。
- **设计系统**：复用 `shared.css` :root 既有 token（`--brand` #5E5CE6 / `--brand-2` #7C6BFF / `--scheduled` #0A84FF / `--ai` #0FB5A8 / `--running` #30D158 / `--held` #FF9F0A / `--blocked` #FF453A / `--ink-*` / `--on-deep-*` / `--r-md` 16px / `--mono` 等）。**不引入任何新品牌色或新组件库**。浅底文字用 `-ink` 后缀色（`--ai-ink` / `--held-ink` / `--blocked-ink` / `--scheduled-ink`）保 WCAG≥4.5。
- **双语**：所有文案 `data-lang-zh` / `data-lang-en` 成对。任何带显式 `display`（flex/inline-flex/grid 等）的双语元素，必须补三条 per-element 语言显隐规则，否则 ZH/EN 双显（per-page CSS 晚于 shared.css 加载，显式 display 会盖掉 `[data-lang-en]{display:none}`）。无显式 display 的元素（纯文本 `<p>`/`<h*>`/`<span>` 继承默认）依赖 shared.css 全局规则即可，不必补。
- **不放集群 / worker 数量**（既定红线）。
- **占位域名** `https://weagent.example.com` 不动；不加 ICP 备案。
- **不触 no-human-takeover 禁词**：用 AI 自治措辞（"AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文"），绝不写"人工接管 / takeover / hand-off / 人工介入"。
- **回复全程中文**（对用户）。**不提交 git**（未授权）。

## 文件结构

| 文件 | 责任 | 操作 |
| --- | --- | --- |
| `website/observability.html` | 新频道页全部内容：导航 + 移动菜单 + 英雄 + 3 层 18 板块 + CTA + 页脚 | 新建 |
| `website/assets/observability.css` | 本页专属样式：英雄、三层各自的版式组件、仿真截面、紧凑网格卡、响应式、语言显隐 | 新建 |
| `website/index.html` | 桌面 nav-links + 移动 mobile-menu 各插 1 项 | 改 |
| `website/solutions.html` | 同上 | 改 |
| `website/product.html` | 同上 | 改 |
| `website/agents.html` | 同上 | 改 |
| `website/technology.html` | 同上 | 改 |
| `website/engineering.html` | 同上 | 改 |
| `website/evolution.html` | 同上 | 改 |
| `website/scenarios.html` | 同上 | 改 |
| `website/trust.html` | 同上 | 改 |
| `website/404.html` | nav-links + mobile-menu 各插 1 项（绝对路径风格，同时补回缺失的 solutions） | 改 |
| 各页页脚"信任"列 | 补 1 条"运营透视"链接 | 改（并入各自页面任务） |

**导航接入统一规范**（所有页面照此插入，位置 = Scenarios 与 Trust 之间）：

内容页（相对路径，9 页）nav-links 与 mobile-menu 各插入：
```html
<a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
```
404 页（绝对路径）插入：
```html
<a href="/observability.html" data-lang-zh>运营透视</a><a href="/observability.html" data-lang-en>Observability</a>
```

**导航横向拥挤处理**：导航在 `max-width:1140px` 以下整体折叠为汉堡菜单（`shared.css:300-303`），所以 9→10 项只影响 ≥1141px 桌面。`nav-links` gap 当前 2px、链接 padding 9px 12px。Task 16 渲染自检在 1280 宽确认 10 项不溢出 / 不换行；若溢出，仅在 `observability.css` 加一条全局兜底（不改 shared.css）：`@media(min-width:1141px){.nav-links a{padding-left:10px;padding-right:10px}}`，否则不动。

---

## 实施顺序总览

- **Task 1**：建 `observability.css` 骨架（英雄 + 通用 section 辅助类 + 语言显隐基底）。
- **Task 2**：建 `observability.html` 骨架（head / nav / mobile-menu / hero / 空 main 占位 / CTA / footer / scripts）——本任务结束页面已可本地打开、双语可切、导航可用。
- **Task 3**：第 1 层"看懂这一次决策"6 板块（含仿真运营台截面）+ 对应 CSS。
- **Task 4**：第 2 层"看懂这个客户"6 板块 + 对应 CSS。
- **Task 5**：第 3 层"看懂整个系统"6 板块（紧凑网格卡）+ 对应 CSS。
- **Task 6**：结尾 CTA 文案落定（骨架在 Task 2 已搭，本任务确认双语文案与跳转无误）。
- **Task 7–15**：把"运营透视"导航项 + 移动菜单项 + 页脚链接接入 9 个内容页（每页一个任务）。
- **Task 15**：404 页单独接入（绝对路径，并补回缺失的 solutions）。
- **Task 16**：全站渲染自检（Playwright 1280 ZH/EN + 移动 390）+ 零黑话通读 + 诚实边界核对 + 双显排查。

> 注：Task 3/4/5 是本页主体，CSS 与 HTML 同任务交付（一个板块组的样式和结构强耦合，分开无法独立验收）。导航接入（7–15）彼此独立、可并行。

---

### Task 1: observability.css 骨架

**Files:**
- Create: `website/assets/observability.css`

**Interfaces:**
- Consumes: `shared.css` 的 :root token（`--brand` / `--ink-deep` / `--on-deep-*` / `--card` / `--hair` / `--r-lg` / `--mono` / `--gap` 等，已全局可用）。
- Produces: 供 Task 2 hero 用的 `.page-hero` / `.page-hero-meta`（自包含一份，避免跨文件依赖）；供 Task 3/4/5 在文件末尾追加各层组件样式的落点。

- [ ] **Step 1: 写 CSS 文件头与英雄 + 章节标题样式**

把以下内容写入 `website/assets/observability.css`（Task 3/4/5 之后在文件**末尾追加**各层组件样式）：

```css
/* ============================================================
   WeAgent 官网 — 运营透视（Observability）页专属样式
   依赖 shared.css 的 token；纯静态，无新品牌色
   ============================================================ */

/* ---------- 页内英雄（自包含，与其它子页一致） ---------- */
.page-hero { position: relative; overflow: hidden; padding: clamp(64px,9vw,116px) 0 clamp(48px,6vw,80px); }
.page-hero-bg { position: absolute; inset: 0; pointer-events: none; }
.page-hero .h-display { margin: 22px 0 20px; }
.page-hero .lead { max-width: 800px; }
.page-hero-meta { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 30px; }
.page-hero-meta .pill { background: rgba(255,255,255,.06); border: 1px solid var(--ink-hair); color: var(--on-deep-2); }
.page-hero-meta .pill[data-lang-en] { display: none; }
html[lang="en"] .page-hero-meta .pill[data-lang-zh] { display: none; }
html[lang="en"] .page-hero-meta .pill[data-lang-en] { display: inline-flex; }

/* ---------- 章节小标题（与 trust.css 同款） ---------- */
.s-head { max-width: 860px; margin-bottom: 44px; }
.s-head .h-section { margin: 16px 0 14px; }
.section-deep .s-head .eyebrow { color: var(--brand-2); }

/* ---------- 层级引导小标（每层开头的层号 + 主题） ---------- */
.layer-head { display: flex; align-items: baseline; gap: 14px; flex-wrap: wrap; margin-bottom: 8px; }
.layer-no { font-family: var(--mono); font-size: 14px; font-weight: 700; color: var(--brand); letter-spacing: .04em; }
.section-deep .layer-no { color: var(--brand-2); }

/* ===== 以下组件样式由 Task 3 / 4 / 5 追加到文件末尾 ===== */
```

- [ ] **Step 2: 校验 CSS 花括号配平**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "s=open('assets/observability.css',encoding='utf-8').read(); print('braces', s.count('{'), s.count('}')); assert s.count('{')==s.count('}')"`
Expected: 左右花括号数量相等。

- [ ] **Step 3: 不提交（未授权），继续 Task 2。**

---

### Task 2: observability.html 骨架（可打开、可切语言、导航可用）

**Files:**
- Create: `website/observability.html`

**Interfaces:**
- Consumes: `assets/shared.css`、`assets/observability.css`（Task 1）、`assets/i18n.js`、`assets/shared.js`。
- Produces: 三个空 `<section>` 锚点容器 `#layer-decision` / `#layer-customer` / `#layer-system`（内含 `.wrap`），Task 3/4/5 各自替换其内部占位注释填充内容。

- [ ] **Step 1: 写完整骨架文件**

把下方完整 HTML 写入 `website/observability.html`。要点：导航 / 移动菜单含本页 `class="active"`（仅本页那一项）；英雄区文案为最终文案；三层 section 留空占位（含占位注释）；CTA 与页脚完整；脚本两行接好。导航顺序 = 首页/解决什么/产品能力/智能体编队/技术架构/工程深度/自我演化/行业场景/**运营透视**/信任与安全（运营透视在场景与信任之间）。

```html
<!DOCTYPE html>
<html lang="zh">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title data-zh="运营透视 · WeAgent" data-en="Observability · WeAgent">运营透视 · WeAgent</title>
<meta name="description" content="AI 不是黑箱：每一次回复走了哪几步、为什么发、有据可溯、评审打分、自我反省都看得到；客户画像持续观测、诚实标注置信；整盘系统态势一屏掌握。面向运营管理者与企业主。">
<link rel="canonical" href="https://weagent.example.com/observability.html">
<meta property="og:type" content="website">
<meta property="og:site_name" content="WeAgent">
<meta property="og:locale" content="zh_CN">
<meta property="og:locale:alternate" content="en_US">
<meta property="og:title" content="运营透视 · WeAgent">
<meta property="og:description" content="AI 不是黑箱：单次决策可回放、客户画像持续观测、系统态势一屏掌握。诚实标注置信，运营者全看得到。">
<meta property="og:url" content="https://weagent.example.com/observability.html">
<meta property="og:image" content="https://weagent.example.com/assets/og-cover.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="WeAgent — 全自治微信私域 AI 运营">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="运营透视 · WeAgent">
<meta name="twitter:description" content="AI 不是黑箱：单次决策可回放、客户画像持续观测、系统态势一屏掌握。">
<meta name="twitter:image" content="https://weagent.example.com/assets/og-cover.png">
<link rel="stylesheet" href="assets/shared.css">
<link rel="stylesheet" href="assets/observability.css">
<link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect width='64' height='64' rx='18' fill='%235E5CE6'/%3E%3Cpath fill='%23fff' fill-rule='evenodd' clip-rule='evenodd' d='M18.5 7 H45.5 Q57.5 7 57.5 19 V33 Q57.5 45 45.5 45 H24 L13 57 V44.4 Q6.5 43 6.5 33 V19 Q6.5 7 18.5 7 Z M14 15.5 L20.2 38 L31 23 L33 23 L43.8 38 L50 15.5 L44.3 15.5 L40.4 31 L33.4 21 L30.6 21 L23.6 31 L19.7 15.5 Z M47 6.1 C47 12.02 46 13.5 42 13.5 C46 13.5 47 14.98 47 20.9 C47 14.98 48 13.5 52 13.5 C48 13.5 47 12.02 47 6.1 Z'/%3E%3C/svg%3E">
</head>
<body>

<!-- ============ 导航 ============ -->
<nav class="nav">
  <div class="wrap nav-inner">
    <a href="index.html" class="brand">
      <span class="brand-mark"><svg viewBox="0 0 64 64" fill="none"><path fill="#fff" fill-rule="evenodd" clip-rule="evenodd" d="M18.5 7 H45.5 Q57.5 7 57.5 19 V33 Q57.5 45 45.5 45 H24 L13 57 V44.4 Q6.5 43 6.5 33 V19 Q6.5 7 18.5 7 Z M14 15.5 L20.2 38 L31 23 L33 23 L43.8 38 L50 15.5 L44.3 15.5 L40.4 31 L33.4 21 L30.6 21 L23.6 31 L19.7 15.5 Z M47 6.1 C47 12.02 46 13.5 42 13.5 C46 13.5 47 14.98 47 20.9 C47 14.98 48 13.5 52 13.5 C48 13.5 47 12.02 47 6.1 Z"/></svg></span>
      WeAgent
    </a>
    <div class="nav-links">
      <a href="index.html" data-lang-zh>首页</a><a href="index.html" data-lang-en>Home</a>
      <a href="solutions.html" data-lang-zh>解决什么</a><a href="solutions.html" data-lang-en>Solutions</a>
      <a href="product.html" data-lang-zh>产品能力</a><a href="product.html" data-lang-en>Product</a>
      <a href="agents.html" data-lang-zh>智能体编队</a><a href="agents.html" data-lang-en>Agents</a>
      <a href="technology.html" data-lang-zh>技术架构</a><a href="technology.html" data-lang-en>Technology</a>
      <a href="engineering.html" data-lang-zh>工程深度</a><a href="engineering.html" data-lang-en>Engineering</a>
      <a href="evolution.html" data-lang-zh>自我演化</a><a href="evolution.html" data-lang-en>Self-evolution</a>
      <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
      <a href="observability.html" class="active" data-lang-zh>运营透视</a><a href="observability.html" class="active" data-lang-en>Observability</a>
      <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust</a>
    </div>
    <div class="nav-right">
      <div class="lang-toggle">
        <button data-lang-btn="zh">中</button>
        <button data-lang-btn="en">EN</button>
      </div>
      <button class="nav-burger" aria-label="menu"><svg viewBox="0 0 24 24" width="22" height="22" fill="none"><path d="M4 7h16M4 12h16M4 17h16" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg></button>
    </div>
  </div>
</nav>
<div class="mobile-menu">
  <a href="index.html" data-lang-zh>首页</a><a href="index.html" data-lang-en>Home</a>
  <a href="solutions.html" data-lang-zh>解决什么</a><a href="solutions.html" data-lang-en>Solutions</a>
  <a href="product.html" data-lang-zh>产品能力</a><a href="product.html" data-lang-en>Product</a>
  <a href="agents.html" data-lang-zh>智能体编队</a><a href="agents.html" data-lang-en>Agents</a>
  <a href="technology.html" data-lang-zh>技术架构</a><a href="technology.html" data-lang-en>Technology</a>
  <a href="engineering.html" data-lang-zh>工程深度</a><a href="engineering.html" data-lang-en>Engineering</a>
  <a href="evolution.html" data-lang-zh>自我演化</a><a href="evolution.html" data-lang-en>Self-evolution</a>
  <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
  <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
  <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust</a>
</div>

<!-- ============ 页内英雄 ============ -->
<header class="page-hero section-deep">
  <div class="page-hero-bg" aria-hidden="true"><div class="hero-glow" style="position:absolute;width:560px;height:560px;border-radius:50%;filter:blur(90px);opacity:.5;top:-160px;left:-100px;background:radial-gradient(circle,rgba(94,92,230,.34),transparent 68%)"></div><div class="hero-glow" style="position:absolute;width:480px;height:480px;border-radius:50%;filter:blur(90px);opacity:.4;top:-40px;right:-80px;background:radial-gradient(circle,rgba(15,181,168,.32),transparent 70%)"></div></div>
  <div class="wrap" style="position:relative">
    <span class="eyebrow reveal" data-lang-zh>运营透视 · 看懂 AI 的每一步</span><span class="eyebrow reveal" data-lang-en>Observability · See every step the AI takes</span>
    <h1 class="h-display reveal reveal-d1" data-lang-zh>AI 不是黑箱，<span class="text-grad">它的每一步都摊开给你看</span></h1>
    <h1 class="h-display reveal reveal-d1" data-lang-en>The AI is not a black box — <span class="text-grad">every step is laid open to you</span></h1>
    <p class="lead reveal reveal-d2" data-lang-zh>它为每个客户做的每一个判断、走的每一步、为什么这么做，运营者随时看得到、复盘得了。从一次回复的来龙去脉，到一个客户的画像演化，再到整盘系统的健康态势——三层都摊开。</p>
    <p class="lead reveal reveal-d2" data-lang-en>Every judgment it makes for every customer, every step it takes, and why — visible and reviewable anytime. From the story behind a single reply, to how a customer's profile evolves, to the health of the whole system — all three layers laid open.</p>
    <div class="page-hero-meta reveal reveal-d3">
      <span class="pill" data-lang-zh>单次决策可回放</span><span class="pill" data-lang-en>Replay any decision</span>
      <span class="pill" data-lang-zh>画像持续观测</span><span class="pill" data-lang-en>Always-on profiling</span>
      <span class="pill" data-lang-zh>系统态势一屏</span><span class="pill" data-lang-en>System health at a glance</span>
    </div>
  </div>
</header>

<!-- ============ 第 1 层：看懂这一次决策（Task 3 填充） ============ -->
<section class="section" id="layer-decision">
  <div class="wrap"><!-- LAYER-1-CONTENT: Task 3 在此填充，删除本注释 --></div>
</section>

<!-- ============ 第 2 层：看懂这个客户（Task 4 填充） ============ -->
<section class="section section-tint" id="layer-customer">
  <div class="wrap"><!-- LAYER-2-CONTENT: Task 4 在此填充，删除本注释 --></div>
</section>

<!-- ============ 第 3 层：看懂整个系统（Task 5 填充） ============ -->
<section class="section section-deep" id="layer-system">
  <div class="hero-bg" aria-hidden="true"><div class="hero-glow" style="position:absolute;width:680px;height:680px;border-radius:50%;filter:blur(100px);opacity:.14;top:50%;left:50%;transform:translate(-50%,-50%);background:radial-gradient(circle,rgba(94,92,230,.4),transparent 72%)"></div></div>
  <div class="wrap" style="position:relative"><!-- LAYER-3-CONTENT: Task 5 在此填充，删除本注释 --></div>
</section>

<!-- ============ 结尾 CTA（Task 6 校验文案） ============ -->
<section class="section cta-band">
  <div class="hero-bg" aria-hidden="true"><div class="hero-glow" style="position:absolute;width:560px;height:560px;border-radius:50%;filter:blur(90px);opacity:.4;top:-160px;left:-80px;background:radial-gradient(circle,rgba(94,92,230,.5),transparent 68%)"></div></div>
  <div class="wrap cta-inner reveal" style="text-align:center;max-width:720px;margin:0 auto;position:relative">
    <h2 class="h-section" data-lang-zh>可观测之上，是<span class="text-grad">写进代码的红线与可复盘的工程</span></h2>
    <h2 class="h-section" data-lang-en>Above observability lie <span class="text-grad">red lines in code and reviewable engineering</span></h2>
    <p class="lead" data-lang-zh>看得清，是为了管得住。看看这些可观测之下，红线如何写进代码、认知方法与自我演化如何工程落地。</p>
    <p class="lead" data-lang-en>Seeing clearly is what makes it governable. See how the red lines are compiled into code, and how the cognition and self-evolution are engineered beneath what you observe.</p>
    <div class="hero-cta" style="justify-content:center;margin-top:8px">
      <a href="trust.html" class="btn btn-primary"><span data-lang-zh>看红线如何写进代码</span><span data-lang-en>Red lines in code</span><svg viewBox="0 0 24 24" fill="none"><path d="M5 12h14M13 6l6 6-6 6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg></a>
      <a href="technology.html" class="btn btn-ghost"><span data-lang-zh>看技术架构</span><span data-lang-en>See the architecture</span></a>
    </div>
  </div>
</section>

<!-- ============ 页脚 ============ -->
<footer class="footer">
  <div class="wrap">
    <div class="footer-grid">
      <div class="footer-brand">
        <a href="index.html" class="brand"><span class="brand-mark"><svg viewBox="0 0 64 64" fill="none"><path fill="#fff" fill-rule="evenodd" clip-rule="evenodd" d="M18.5 7 H45.5 Q57.5 7 57.5 19 V33 Q57.5 45 45.5 45 H24 L13 57 V44.4 Q6.5 43 6.5 33 V19 Q6.5 7 18.5 7 Z M14 15.5 L20.2 38 L31 23 L33 23 L43.8 38 L50 15.5 L44.3 15.5 L40.4 31 L33.4 21 L30.6 21 L23.6 31 L19.7 15.5 Z M47 6.1 C47 12.02 46 13.5 42 13.5 C46 13.5 47 14.98 47 20.9 C47 14.98 48 13.5 52 13.5 C48 13.5 47 12.02 47 6.1 Z"/></svg></span>WeAgent</a>
        <p data-lang-zh>全自治微信私域 AI 运营系统。客户永远只跟 AI 对话，AI 像真人一样长期经营每一段关系。</p>
        <p data-lang-en>The autonomous WeChat private-domain AI operator. Customers only ever talk to the AI, which nurtures every relationship over time.</p>
        <button class="wx-copy" data-wx="agimeme" type="button">
          <svg viewBox="0 0 24 24" fill="none"><path d="M8.5 14.5c-3 0-5.5-2-5.5-4.6C3 7 5.8 5 9 5s6 2 6 4.9c0 .5-.1 1-.3 1.5M16 19c2.2 0 4-1.4 4-3.3 0-1.9-1.8-3.4-4-3.4s-4 1.5-4 3.4c0 .6.2 1.2.6 1.7L12 19l1.6-.5c.7.3 1.5.5 2.4.5z" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
          <span data-lang-zh>顾问微信 <span class="wx-id">agimeme</span></span><span data-lang-en>WeChat <span class="wx-id">agimeme</span></span>
          <span class="wx-hint" data-lang-zh>点击复制</span><span class="wx-hint" data-lang-en>tap to copy</span>
        </button>
      </div>
      <div>
        <h5 data-lang-zh>产品</h5><h5 data-lang-en>Product</h5>
        <div class="footer-links">
          <a href="solutions.html" data-lang-zh>解决什么问题</a><a href="solutions.html" data-lang-en>What it solves</a>
          <a href="product.html" data-lang-zh>产品能力</a><a href="product.html" data-lang-en>Product</a>
          <a href="agents.html" data-lang-zh>智能体编队</a><a href="agents.html" data-lang-en>Agents</a>
          <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
        </div>
      </div>
      <div>
        <h5 data-lang-zh>技术</h5><h5 data-lang-en>Technology</h5>
        <div class="footer-links">
          <a href="technology.html" data-lang-zh>技术架构</a><a href="technology.html" data-lang-en>Architecture</a>
          <a href="engineering.html" data-lang-zh>工程深度</a><a href="engineering.html" data-lang-en>Engineering depth</a>
          <a href="evolution.html" data-lang-zh>自我演化</a><a href="evolution.html" data-lang-en>Self-evolution</a>
        </div>
      </div>
      <div>
        <h5 data-lang-zh>信任</h5><h5 data-lang-en>Trust</h5>
        <div class="footer-links">
          <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
          <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust & safety</a>
          <a href="trust.html#audit" data-lang-zh>可审计性</a><a href="trust.html#audit" data-lang-en>Auditability</a>
        </div>
      </div>
    </div>
    <div class="footer-bottom">
      <span>© 2026 WeAgent · <span data-lang-zh>私域自主运营</span><span data-lang-en>Autonomous private-domain operations</span></span>
      <span data-lang-zh>Rust · Axum · MongoDB · React · MCP</span>
      <span data-lang-en>Built with Rust · Axum · MongoDB · React · MCP</span>
    </div>
  </div>
</footer>

<script src="assets/i18n.js"></script>
<script src="assets/shared.js"></script>
</body>
</html>
```

- [ ] **Step 2: 校验双语成对 + 三个 section 锚点存在**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "s=open('observability.html',encoding='utf-8').read(); zh=s.count('data-lang-zh'); en=s.count('data-lang-en'); print('zh',zh,'en',en); assert zh==en; print('ids', [x for x in ['layer-decision','layer-customer','layer-system'] if 'id=\"'+x+'\"' in s])"`
Expected: `zh` 与 `en` 相等；打印出三个 id 全部存在。

- [ ] **Step 3: 起静态服务器确认 200**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && (python -m http.server 8125 >/dev/null 2>&1 &) && sleep 1 && curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8125/observability.html && pkill -f "http.server 8125"`
Expected: `200`。

- [ ] **Step 4: 不提交（未授权），继续 Task 3。**

---

### Task 3: 第 1 层「看懂这一次决策」— 仿真截面 + 6 板块

**Files:**
- Modify: `website/observability.html`（替换 `#layer-decision` 内 `.wrap` 的占位注释 `<!-- LAYER-1-CONTENT ... -->`）
- Modify: `website/assets/observability.css`（在文件末尾 `/* ===== ... ===== */` 标记后追加第 1 层组件样式）

**Interfaces:**
- Consumes: Task 2 的 `#layer-decision` 容器；`shared.css` 的 `.s-head` / `.eyebrow` / `.h-section` / `.reveal` / token。
- Produces: 仿真截面类 `.sim-console` / `.sim-step` 与卡片网格类 `.obs-grid` / `.obs-card`（Task 4 会复用 `.obs-grid` / `.obs-card`，命名须与本任务一致）。

板块内容（业务语言，零技术标识符）：① 仿真运营台时间线截面（6 步）② 这条回复走了哪几步 ③ 为什么发 / 为什么没发 ④ 答案有据可溯 ⑤ 评审打分 + 改写 ⑥ AI 自我反省 + 这次花了多少。仿真截面承载①②，④⑤⑥用卡片。

- [ ] **Step 1: 替换 `#layer-decision` 占位注释为以下 HTML**

把 `website/observability.html` 中 `<section class="section" id="layer-decision">` 内的 `<div class="wrap"><!-- LAYER-1-CONTENT: Task 3 在此填充，删除本注释 --></div>` 整体替换为：

```html
  <div class="wrap">
    <div class="s-head reveal">
      <div class="layer-head"><span class="layer-no">第 1 层 / Layer 1</span></div>
      <span class="eyebrow" data-lang-zh>看懂这一次决策 · 为什么这么答</span><span class="eyebrow" data-lang-en>Understand one decision · why it answered this way</span>
      <h2 class="h-section" data-lang-zh>一条回复的来龙去脉，<span class="text-grad">一步步摊开</span></h2>
      <h2 class="h-section" data-lang-en>The full story behind one reply, <span class="text-grad">opened step by step</span></h2>
      <p class="lead" data-lang-zh>客户发来一条消息，AI 这一次到底想了什么、查了什么、为什么这么回——整个经过像回放一样看得清清楚楚。</p>
      <p class="lead" data-lang-en>A customer sends a message; what the AI thought, checked, and why it replied this way — the whole process plays back, crystal clear.</p>
    </div>

    <!-- 仿真运营台截面：一次回复的时间线 -->
    <div class="sim-console reveal">
      <div class="sim-top">
        <span class="sim-dot"></span><span class="sim-dot"></span><span class="sim-dot"></span>
        <span class="sim-title" data-lang-zh>回复回放 · 王女士 · 今天 14:32</span><span class="sim-title" data-lang-en>Reply replay · Ms. Wang · today 14:32</span>
        <span class="sim-badge ok" data-lang-zh>已发送</span><span class="sim-badge ok" data-lang-en>Sent</span>
      </div>
      <div class="sim-body">
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6 9 17l-5-5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>读懂上下文</b><b data-lang-en>Read the context</b><span data-lang-zh>翻看这位客户最近聊了什么、记得哪些事</span><span data-lang-en>Recalls recent chats and what's remembered about her</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6 9 17l-5-5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>查知识库找依据</b><b data-lang-en>Find grounded backing</b><span data-lang-zh>就她问的产品，翻出已核实的资料作依据</span><span data-lang-en>Pulls verified material on the product she asked about</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6 9 17l-5-5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>分层决策</b><b data-lang-en>Make the call</b><span data-lang-zh>定下这一轮怎么回、用什么语气</span><span data-lang-en>Decides what to say this turn, in what tone</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6 9 17l-5-5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>独立评审打分</b><b data-lang-en>Independent review</b><span data-lang-zh>另一个 AI 把关：靠不靠谱、像不像人、有没有压力感</span><span data-lang-en>A second AI checks: reliable, human, any pressure</span></div>
        </div>
        <div class="sim-step warn">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 20h9M3 20l9-16 4 7" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>自动改写一次</b><b data-lang-en>One auto-rewrite</b><span data-lang-zh>第一版语气略硬，自动润色了一遍</span><span data-lang-en>First draft read a bit stiff — polished once</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 12l4 4L19 6" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>确认无误后发送</b><b data-lang-en>Send after it clears</b><span data-lang-zh>全部过关，这条回复才发出去</span><span data-lang-en>Only ships once every check clears</span></div>
        </div>
      </div>
      <div class="sim-foot">
        <span data-lang-zh>每一步的状态——已完成 / 进行中 / 没通过——都一眼可见，AI 这一次"想了什么、做了什么"全程可回放。</span>
        <span data-lang-en>Every step's status — done / in progress / didn't pass — is visible at a glance; what the AI "thought and did" replays end to end.</span>
      </div>
    </div>

    <!-- 6 板块中余下 4 张卡（步骤①②已由截面承载，这里是④⑤⑥的展开 + 为什么发/没发） -->
    <div class="obs-grid cols-2" style="margin-top:var(--gap)">
      <div class="obs-card reveal">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 3v4M12 17v4M3 12h4M17 12h4" stroke-linecap="round"/><circle cx="12" cy="12" r="3.2"/></svg></div>
        <h4 data-lang-zh>为什么发 / 为什么没发</h4><h4 data-lang-en>Why it sent — or didn't</h4>
        <p data-lang-zh>这条回复最终是发出了、先压住了，还是被拦下了，理由写得明明白白。比如"涉及还没核实的产品说法，按规矩先压住、不乱讲"——你一看就懂它为什么这么处理。</p>
        <p data-lang-en>Whether the reply was sent, held back, or blocked — with the reason spelled out plainly. E.g. "touches an unverified product claim, so it's held by rule, not winged" — you instantly see why.</p>
      </div>
      <div class="obs-card reveal reveal-d1">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 19V5a2 2 0 0 1 2-2h9l5 5v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z"/><path d="M8 12h8M8 16h5" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>答案有据可溯</h4><h4 data-lang-en>Answers you can trace</h4>
        <p data-lang-zh>这条回复是基于知识库里哪几条已核实资料说出来的，都能回看；还会标出"这次缺哪块知识"，提示你去补。这是"AI 不会信口开河"最硬的证据。</p>
        <p data-lang-en>Which verified pieces of knowledge the reply stood on — all reviewable; it even flags "what knowledge was missing this time" so you can fill the gap. The hardest proof that the AI doesn't make things up.</p>
      </div>
      <div class="obs-card reveal">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 2l2.4 7.4H22l-6 4.4 2.3 7.2-6.3-4.6L5.7 21l2.3-7.2-6-4.4h7.6z" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>评审打了几分，改了什么</h4><h4 data-lang-en>The review score, and the rewrite</h4>
        <p data-lang-zh>独立评审会从"靠不靠谱、像不像真人、有没有温度、会不会给压力、产品说法准不准"几个角度打分；第一版不达标时，AI 自动改写一次，改前改后都摆给你看。</p>
        <p data-lang-en>An independent review scores it on reliability, human feel, warmth, pressure, and product accuracy; if the first draft falls short, the AI rewrites once — and shows you before and after.</p>
      </div>
      <div class="obs-card reveal reveal-d1">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 3a9 9 0 1 0 9 9" stroke-linecap="round"/><path d="M12 7v5l3 2" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>AI 的自我反省 + 这次花了多少</h4><h4 data-lang-en>Self-reflection, and what it cost</h4>
        <p data-lang-zh>每次决策 AI 都留一段"我这么做对不对、还能怎么更好"的自评，你看得到，也看得到"上次提到的问题这次有没有改进"。这一次用了几次大模型、用量多少、有多少命中缓存省下来，也一并透明（不折算成具体金额）。</p>
        <p data-lang-en>Each decision leaves a note — "did I do right, how could I do better" — and you can see whether last time's issue improved. How many model calls this took, the usage, and how much was saved by cache hits are all transparent too (no dollar figure attached).</p>
      </div>
    </div>
  </div>
```

- [ ] **Step 2: 在 `observability.css` 末尾追加第 1 层样式**

在 `website/assets/observability.css` 文件末尾（`/* ===== ... ===== */` 标记之后）追加：

```css
/* ===================== 第 1 层：决策回放 ===================== */
/* 仿真运营台截面 */
.sim-console { background: var(--card); border: 1px solid var(--hair); border-radius: var(--r-lg); box-shadow: var(--shadow-md); overflow: hidden; }
.sim-top { display: flex; align-items: center; gap: 8px; padding: 14px 20px; background: var(--card-2); border-bottom: 1px solid var(--hair); }
.sim-top .sim-dot { width: 11px; height: 11px; border-radius: 50%; background: var(--hair-strong); }
.sim-top .sim-dot:nth-child(1) { background: #FF5F57; } .sim-top .sim-dot:nth-child(2) { background: #FEBC2E; } .sim-top .sim-dot:nth-child(3) { background: #28C840; }
.sim-title { margin-left: 10px; font-family: var(--mono); font-size: 13px; color: var(--ink-2); }
.sim-title[data-lang-en] { display: none; }
html[lang="en"] .sim-title[data-lang-zh] { display: none; }
html[lang="en"] .sim-title[data-lang-en] { display: inline; }
.sim-badge { margin-left: auto; font-size: 12px; font-weight: 700; padding: 4px 12px; border-radius: 999px; }
.sim-badge.ok { color: var(--running); background: rgba(48,209,88,.14); }
.sim-badge[data-lang-en] { display: none; }
html[lang="en"] .sim-badge[data-lang-zh] { display: none; }
html[lang="en"] .sim-badge[data-lang-en] { display: inline-block; }
.sim-body { display: grid; grid-template-columns: repeat(3,1fr); gap: 14px; padding: 22px; }
.sim-step { position: relative; display: flex; gap: 12px; padding: 16px; background: var(--card-2); border: 1px solid var(--hair); border-radius: var(--r-md); }
.sim-step .sim-ic { width: 32px; height: 32px; flex-shrink: 0; border-radius: 9px; display: grid; place-items: center; }
.sim-step .sim-ic svg { width: 17px; height: 17px; }
.sim-step.done .sim-ic { background: rgba(48,209,88,.14); color: var(--running); }
.sim-step.warn .sim-ic { background: rgba(255,159,10,.14); color: var(--held); }
.sim-step .sim-txt { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
.sim-step .sim-txt b { font-size: 14.5px; font-weight: 700; color: var(--ink-1); }
.sim-step .sim-txt b[data-lang-en] { display: none; }
html[lang="en"] .sim-step .sim-txt b[data-lang-zh] { display: none; }
html[lang="en"] .sim-step .sim-txt b[data-lang-en] { display: block; }
.sim-step .sim-txt span { font-size: 12.5px; color: var(--ink-3); line-height: 1.5; }
.sim-foot { padding: 16px 22px; border-top: 1px solid var(--hair); font-size: 13.5px; color: var(--ink-2); line-height: 1.6; }
/* 通用卡片网格（第 1、2 层共用） */
.obs-grid { display: grid; gap: var(--gap); }
.obs-grid.cols-2 { grid-template-columns: 1fr 1fr; }
.obs-grid.cols-3 { grid-template-columns: repeat(3,1fr); }
.obs-card { background: var(--card); border: 1px solid var(--hair); border-radius: var(--r-lg); padding: 26px; box-shadow: var(--shadow-sm); transition: transform .2s, box-shadow .2s, border-color .2s; }
.obs-card:hover { transform: translateY(-3px); box-shadow: var(--shadow-md); border-color: rgba(94,92,230,.28); }
.obs-card .obs-ic { width: 46px; height: 46px; border-radius: 13px; display: grid; place-items: center; margin-bottom: 15px; background: var(--fill-brand); color: var(--brand); }
.obs-card .obs-ic svg { width: 23px; height: 23px; }
.obs-card h4 { font-size: 17px; font-weight: 700; line-height: 1.35; margin-bottom: 9px; }
.obs-card p { font-size: 14px; color: var(--ink-2); line-height: 1.65; }
@media (max-width: 980px) {
  .sim-body { grid-template-columns: 1fr 1fr; }
  .obs-grid.cols-2, .obs-grid.cols-3 { grid-template-columns: 1fr; }
}
@media (max-width: 600px) {
  .sim-body { grid-template-columns: 1fr; }
}
```

- [ ] **Step 3: 校验花括号配平 + 双语成对**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "c=open('assets/observability.css',encoding='utf-8').read(); assert c.count('{')==c.count('}'),'css brace'; h=open('observability.html',encoding='utf-8').read(); assert h.count('data-lang-zh')==h.count('data-lang-en'),'lang pair'; assert 'LAYER-1-CONTENT' not in h,'placeholder left'; print('ok css braces',c.count('{'),'lang pairs',h.count('data-lang-zh'))"`
Expected: 打印 `ok css braces N lang pairs M`，无 assert 失败（占位注释已删除）。

- [ ] **Step 4: 零技术黑话抽查（第 1 层新增文本）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "import re; h=open('observability.html',encoding='utf-8').read(); bad=[w for w in ['agent_run_logs','llm_call_logs','final_review_status','tool_trace','FactRisk','PressureRisk','held_by_ai_policy','_id','draft','schema','API','endpoint'] if w in h]; print('FORBIDDEN FOUND:',bad) if bad else print('clean')"`
Expected: `clean`（本页禁止技术标识符；注意"草稿"用中文不用 `draft`）。

- [ ] **Step 5: 不提交（未授权），继续 Task 4。**

---

### Task 4: 第 2 层「看懂这个客户」— 画像演化 6 板块（严守只观测不驱动）

**Files:**
- Modify: `website/observability.html`（替换 `#layer-customer` 内 `.wrap` 的占位注释 `<!-- LAYER-2-CONTENT ... -->`）
- Modify: `website/assets/observability.css`（文件末尾追加第 2 层样式）

**Interfaces:**
- Consumes: Task 3 产出的 `.obs-grid` / `.obs-card`（复用，命名一致）；`shared.css` token。
- Produces: 三层可信度条 `.cred-tiers` / `.cred-row`、健康度仪表 `.health-grid` / `.health-cell`（Task 5 不复用这两个，仅本层用）。

**诚实边界（本层最吃重，逐字照 Global Constraints）**：大五人格 / 判断走势 = 只观测、不驱动，且把这条当卖点讲透；成交必须运营者确认、AI 永不自宣；不出现任何技术字段名。

- [ ] **Step 1: 替换 `#layer-customer` 占位注释为以下 HTML**

```html
  <div class="wrap">
    <div class="s-head reveal">
      <div class="layer-head"><span class="layer-no">第 2 层 / Layer 2</span></div>
      <span class="eyebrow" data-lang-zh>看懂这个客户 · 画像越用越准</span><span class="eyebrow" data-lang-en>Understand a customer · profiles that sharpen over time</span>
      <h2 class="h-section" data-lang-zh>它怎么认识你的客户，<span class="text-grad">全摊开、且诚实</span></h2>
      <h2 class="h-section" data-lang-en>How it gets to know your customer — <span class="text-grad">open, and honest</span></h2>
      <p class="lead" data-lang-zh>AI 对每个客户的认知分层级、有出处、标置信。最关键的一条诚实边界：它只观测、只沉淀，绝不拿没把握的猜测去预设客户、左右话术——这既避免贴标签偏见，又让你越用越懂这个人。</p>
      <p class="lead" data-lang-en>Its read on each customer is tiered, sourced, and confidence-marked. The key honest boundary: it only observes and records — it never lets an unsure guess preset the customer or steer the wording. That avoids label bias and helps you understand the person better over time.</p>
    </div>

    <!-- 三层可信度认知 -->
    <div class="cred-tiers">
      <div class="cred-row is-human reveal">
        <div class="cred-rank"><span data-lang-zh>人定</span><span data-lang-en>Human</span></div>
        <div class="cred-main">
          <h4 data-lang-zh>运营者亲手录入的判断</h4><h4 data-lang-en>What the operator entered by hand</h4>
          <p data-lang-zh>你对这个客户下的判断，AI 永远改不动、坚信不疑、原样带进每一次决策。这是最高权威，AI 不会"自作聪明"覆盖。</p>
          <p data-lang-en>Your call on the customer: the AI can never change it, trusts it absolutely, and carries it verbatim into every decision. Top authority — never "optimized" away by the AI.</p>
          <span class="cred-flag drive" data-lang-zh>驱动决策</span><span class="cred-flag drive" data-lang-en>Drives decisions</span>
        </div>
      </div>
      <div class="cred-row is-confirmed reveal reveal-d1">
        <div class="cred-rank"><span data-lang-zh>有据</span><span data-lang-en>Grounded</span></div>
        <div class="cred-main">
          <h4 data-lang-zh>AI 有确凿证据才敢确信的判断</h4><h4 data-lang-en>What the AI is confident in — only with evidence</h4>
          <p data-lang-zh>每一条都挂着原始聊天证据，找不到出处就直接丢弃。它会进决策，但门槛是"拿得出依据"，不是 AI 随口一说。</p>
          <p data-lang-en>Each carries the original chat evidence; no source, no keep. It feeds decisions, but the bar is "can show receipts," not the AI's say-so.</p>
          <span class="cred-flag drive" data-lang-zh>有证据才驱动</span><span class="cred-flag drive" data-lang-en>Drives — with evidence</span>
        </div>
      </div>
      <div class="cred-row is-observed reveal reveal-d2">
        <div class="cred-rank"><span data-lang-zh>参考</span><span data-lang-en>Hint</span></div>
        <div class="cred-main">
          <h4 data-lang-zh>AI 暂时的观察猜测</h4><h4 data-lang-en>The AI's tentative observations</h4>
          <p data-lang-zh>证据还不够、置信还不高的猜测。只记录、供你参考，<b>永远不影响 AI 怎么回复</b>。一眼看清哪些是人定的、哪些 AI 有据、哪些只是参考。</p>
          <p data-lang-en>Guesses without enough evidence or confidence. Recorded for your reference only, and <b>never affect how the AI replies</b>. You see at a glance what's human-set, what's grounded, what's just a hint.</p>
          <span class="cred-flag norec" data-lang-zh>只记录 · 不驱动</span><span class="cred-flag norec" data-lang-en>Record only · never drives</span>
        </div>
      </div>
    </div>

    <!-- 余下板块卡片 -->
    <div class="obs-grid cols-3" style="margin-top:var(--gap)">
      <div class="obs-card reveal">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-7 8-7s8 3 8 7" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>性格画像，越用越懂</h4><h4 data-lang-en>A personality read that grows</h4>
        <p data-lang-zh>用心理学公认的"大五人格"五个维度，慢慢摸清每个客户的性格倾向，形成画像供你参考。它只观测、不替客户预设回应；证据不够时如实标注"还不确定"。</p>
        <p data-lang-en>Using the well-established Big Five dimensions, it gradually reads each customer's traits into a profile for your reference. It observes only, never presets responses; when evidence is thin it honestly marks "not sure yet."</p>
      </div>
      <div class="obs-card reveal reveal-d1">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 18l5-6 4 3 6-8" stroke-linecap="round" stroke-linejoin="round"/><path d="M4 21h16" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>判断走势看得见</h4><h4 data-lang-en>Watch the judgment trend</h4>
        <p data-lang-zh>AI 对客户的一些关键判断，会随多轮聊天累积证据、连成走势线，置信高低、是否已经站稳都标得清清楚楚。同样只供观测复盘，不驱动回复。</p>
        <p data-lang-en>Key judgments accrue evidence across turns into a trend line, with confidence and whether it's settled clearly marked. Also observation-and-review only — it doesn't drive replies.</p>
      </div>
      <div class="obs-card reveal reveal-d2">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 7h7v13H4zM13 4h7v16h-7z"/><path d="M7 11v5M16.5 9v7" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>长期记忆看得到</h4><h4 data-lang-en>Long-term memory, visible</h4>
        <p data-lang-zh>AI 给每个客户记的长期记忆卡——核心事实、近期动态、已过时信息分层——以及还在观察、尚未正式入卡的候选记忆，你都能查、能管。</p>
        <p data-lang-en>The long-term memory card the AI keeps per customer — core facts, recent moves, outdated info, tiered — plus candidate memories still under observation, all reviewable and manageable.</p>
      </div>
    </div>

    <!-- 客户健康度仪表 -->
    <div class="health-band reveal" style="margin-top:var(--gap)">
      <div class="health-head">
        <h3 data-lang-zh>客户健康度，一屏看清谁要重点跟</h3><h3 data-lang-en>Customer health — see who needs attention at a glance</h3>
        <p data-lang-zh>把 AI 对这个客户的几项关键评估打成 0–100 的健康分，红黄绿一目了然。注意：这些分只是给你看的体检表，不会反过来左右 AI 怎么说话。</p>
        <p data-lang-en>It turns several key assessments into 0–100 health scores, red-amber-green at a glance. Note: these scores are a check-up for you to read — they never loop back to steer how the AI talks.</p>
      </div>
      <div class="health-grid">
        <div class="health-cell good"><span class="hc-num">86</span><span class="hc-lbl" data-lang-zh>理解程度</span><span class="hc-lbl" data-lang-en>Understanding</span></div>
        <div class="health-cell good"><span class="hc-num">78</span><span class="hc-lbl" data-lang-zh>关系质量</span><span class="hc-lbl" data-lang-en>Relationship</span></div>
        <div class="health-cell warn"><span class="hc-num">64</span><span class="hc-lbl" data-lang-zh>产品契合</span><span class="hc-lbl" data-lang-en>Product fit</span></div>
        <div class="health-cell warn"><span class="hc-num">59</span><span class="hc-lbl" data-lang-zh>跟进节奏</span><span class="hc-lbl" data-lang-en>Follow-up pace</span></div>
        <div class="health-cell good"><span class="hc-num">92</span><span class="hc-lbl" data-lang-zh>说法有据</span><span class="hc-lbl" data-lang-en>Grounded claims</span></div>
        <div class="health-cell good"><span class="hc-num">95</span><span class="hc-lbl" data-lang-zh>不给压力</span><span class="hc-lbl" data-lang-en>Low pressure</span></div>
        <div class="health-cell good"><span class="hc-num">88</span><span class="hc-lbl" data-lang-zh>不乱说话</span><span class="hc-lbl" data-lang-en>No making-things-up</span></div>
      </div>
    </div>

    <!-- 成效与反响 -->
    <div class="obs-grid cols-1" style="margin-top:var(--gap)">
      <div class="obs-card is-wide reveal">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M3 12h4l3 8 4-16 3 8h4" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>成效与反响，连成一张账</h4><h4 data-lang-en>Outcomes and responses, in one ledger</h4>
        <p data-lang-zh>成交了没有（<b>AI 永远不会自己宣布成交，必须由运营者确认</b>）、主动发出去的内容客户有没有回应、有没有推进关系，加上回复率、对话深度这些日常成效，连成一张看得见的成效账——干得好不好，是数据说了算。</p>
        <p data-lang-en>Whether a deal closed (<b>the AI never declares a deal itself — an operator must confirm</b>), whether proactive messages drew responses, whether the relationship advanced, plus reply rate and conversation depth — woven into one visible ledger. How well it's doing is the data's call.</p>
      </div>
    </div>
  </div>
```

- [ ] **Step 2: 在 `observability.css` 末尾追加第 2 层样式**

```css
/* ===================== 第 2 层：画像演化 ===================== */
/* 三层可信度 */
.cred-tiers { display: flex; flex-direction: column; gap: 14px; }
.cred-row { display: grid; grid-template-columns: 92px 1fr; gap: 22px; align-items: center; background: var(--card); border: 1px solid var(--hair); border-left-width: 4px; border-radius: var(--r-lg); padding: 24px 28px; transition: transform .2s; }
.cred-row:hover { transform: translateX(4px); }
.cred-row.is-human { border-left-color: var(--blocked); }
.cred-row.is-confirmed { border-left-color: var(--ai); }
.cred-row.is-observed { border-left-color: var(--ink-3); }
.cred-rank { display: grid; place-items: center; width: 92px; height: 56px; border-radius: var(--r-md); font-size: 17px; font-weight: 800; }
.cred-row.is-human .cred-rank { background: rgba(255,69,58,.1); color: var(--blocked-ink); }
.cred-row.is-confirmed .cred-rank { background: var(--fill-ai); color: var(--ai-ink); }
.cred-row.is-observed .cred-rank { background: var(--card-2); color: var(--ink-3); }
.cred-rank span[data-lang-en] { display: none; }
html[lang="en"] .cred-rank span[data-lang-zh] { display: none; }
html[lang="en"] .cred-rank span[data-lang-en] { display: block; }
.cred-main h4 { font-size: 17px; font-weight: 700; margin-bottom: 6px; }
.cred-main p { font-size: 13.8px; color: var(--ink-2); line-height: 1.6; margin-bottom: 10px; }
.cred-main p b { color: var(--ink-1); }
.cred-flag { display: inline-flex; align-items: center; font-size: 12px; font-weight: 700; padding: 4px 12px; border-radius: 999px; }
.cred-flag.drive { color: var(--ai-ink); background: var(--fill-ai); }
.cred-flag.norec { color: var(--ink-3); background: var(--card-2); border: 1px solid var(--hair); }
.cred-flag[data-lang-en] { display: none; }
html[lang="en"] .cred-flag[data-lang-zh] { display: none; }
html[lang="en"] .cred-flag[data-lang-en] { display: inline-flex; }
.obs-grid.cols-1 { grid-template-columns: 1fr; }
.obs-card.is-wide { display: block; }
/* 健康度仪表 */
.health-band { background: var(--card-2); border: 1px solid var(--hair); border-radius: var(--r-lg); padding: clamp(24px,3vw,34px); }
.health-head { max-width: 760px; margin-bottom: 22px; }
.health-head h3 { font-size: clamp(19px,2.2vw,24px); font-weight: 800; letter-spacing: -.02em; margin-bottom: 10px; }
.health-head p { font-size: 14px; color: var(--ink-2); line-height: 1.65; }
.health-grid { display: grid; grid-template-columns: repeat(7,1fr); gap: 12px; }
.health-cell { display: flex; flex-direction: column; align-items: center; gap: 8px; background: var(--card); border: 1px solid var(--hair); border-radius: var(--r-md); padding: 18px 10px; text-align: center; }
.health-cell .hc-num { font-family: var(--mono); font-size: 26px; font-weight: 800; letter-spacing: -.02em; }
.health-cell.good .hc-num { color: var(--ai-ink); }
.health-cell.warn .hc-num { color: var(--held-ink); }
.health-cell .hc-lbl { font-size: 12px; color: var(--ink-2); line-height: 1.35; }
.health-cell .hc-lbl[data-lang-en] { display: none; }
html[lang="en"] .health-cell .hc-lbl[data-lang-zh] { display: none; }
html[lang="en"] .health-cell .hc-lbl[data-lang-en] { display: block; }
@media (max-width: 980px) {
  .cred-row { grid-template-columns: 1fr; gap: 14px; }
  .cred-rank { width: auto; justify-self: start; padding: 0 18px; }
  .health-grid { grid-template-columns: repeat(4,1fr); }
}
@media (max-width: 600px) {
  .health-grid { grid-template-columns: repeat(2,1fr); }
}
```

- [ ] **Step 3: 校验花括号 + 双语成对 + 占位注释已删**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "c=open('assets/observability.css',encoding='utf-8').read(); assert c.count('{')==c.count('}'),'css brace'; h=open('observability.html',encoding='utf-8').read(); assert h.count('data-lang-zh')==h.count('data-lang-en'),'lang pair'; assert 'LAYER-2-CONTENT' not in h,'placeholder left'; print('ok braces',c.count('{'),'pairs',h.count('data-lang-zh'))"`
Expected: 打印 `ok braces N pairs M`，无失败。

- [ ] **Step 4: 不提交（未授权），继续 Task 5。**

---

### Task 5: 第 3 层「看懂整个系统」— 态势大盘 6 紧凑网格卡（深色区）

**Files:**
- Modify: `website/observability.html`（替换 `#layer-system` 内 `.wrap` 的占位注释 `<!-- LAYER-3-CONTENT ... -->`）
- Modify: `website/assets/observability.css`（文件末尾追加第 3 层样式）

**Interfaces:**
- Consumes: Task 2 的 `#layer-system`（已是 `section-deep` 深色区，含装饰光晕）；`shared.css` token。
- Produces: 深色紧凑网格卡 `.sys-grid` / `.sys-card`（仅本层用）。

板块（每块 2–3 行紧凑）：① 自治回路健康 ② 近 24 小时态势 ③ AI 的新想法待你审 ④ 它怎么自我进化 ⑤ 知识库健不健康 ⑥ 运行基础透明。深色区文字用 `--on-deep-*`，eyebrow/标题用浅色。

- [ ] **Step 1: 替换 `#layer-system` 占位注释为以下 HTML**

注意：`#layer-system` 的 `.wrap` 已带 `style="position:relative"` 和前置的 `.hero-bg` 光晕（Task 2 骨架已放），这里只替换 `.wrap` 内的占位注释。

```html
    <div class="s-head reveal">
      <div class="layer-head"><span class="layer-no">第 3 层 / Layer 3</span></div>
      <span class="eyebrow" data-lang-zh>看懂整个系统 · 态势一屏掌握</span><span class="eyebrow" data-lang-en>Understand the whole system · posture at a glance</span>
      <h2 class="h-section" data-lang-zh>整盘跑得健不健康，<span class="text-grad">一屏看住</span></h2>
      <h2 class="h-section" data-lang-en>Whether the whole operation is healthy — <span class="text-grad">held in one screen</span></h2>
      <p class="lead" data-lang-zh>从单个客户抬起头，看整盘：AI 整体表现、这一天的态势、它琢磨出的新想法、怎么自我进化、知识库健不健康、底层有没有稳稳跑着——管理者要的全局视角，都在这里。</p>
      <p class="lead" data-lang-en>Lift your eyes from one customer to the whole board: overall AI performance, the day's posture, the new ideas it surfaced, how it self-evolves, knowledge-base health, and whether the plumbing runs steady — the manager's bird's-eye view, all here.</p>
    </div>

    <div class="sys-grid">
      <div class="sys-card reveal">
        <div class="sys-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 3a9 9 0 1 0 9 9" stroke-linecap="round"/><path d="M21 3v6h-6" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>自治回路健康</h4><h4 data-lang-en>Autonomy-loop health</h4>
        <p data-lang-zh>AI 主动暂缓的三类细分（策略暂缓 / 安全拦截 / 等更多信息）、没核实的产品说法被拦了多少、自我反省的改进率、发送成功与取消的比例——AI 整体表现一屏看住。</p>
        <p data-lang-en>The three kinds of holds (policy hold / safety block / awaiting more context), how many unverified product claims got stopped, the self-reflection improvement rate, and send-vs-cancel ratio — overall AI performance in one view.</p>
      </div>
      <div class="sys-card reveal reveal-d1">
        <div class="sys-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>近 24 小时态势</h4><h4 data-lang-en>Last 24 hours</h4>
        <p data-lang-zh>这一天 AI 跑了多少次、分别停在哪个环节、有多少正等着你拍板——积压一眼看到，及时介入。</p>
        <p data-lang-en>How many runs today, where each one paused, and how many await your call — backlog spotted at a glance.</p>
      </div>
      <div class="sys-card reveal reveal-d2">
        <div class="sys-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 3v18M5 8l7-5 7 5" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="14" r="3"/></svg></div>
        <h4 data-lang-zh>AI 的新想法待你审</h4><h4 data-lang-en>New ideas pending your review</h4>
        <p data-lang-zh>AI 自己琢磨出来的新标签、新关系类型、疑似成交线索，都进"待审"区，<b>你审了才生效</b>——它永远不会自作主张。</p>
        <p data-lang-en>New tags, relationship types, and suspected deal signals the AI surfaces all land in a review queue — <b>they take effect only after you approve</b>. It never acts on its own.</p>
      </div>
      <div class="sys-card reveal">
        <div class="sys-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 2v4M12 18v4M2 12h4M18 12h4M5 5l3 3M16 16l3 3M19 5l-3 3M8 16l-3 3"/><circle cx="12" cy="12" r="3.4"/></svg></div>
        <h4 data-lang-zh>它怎么自我进化</h4><h4 data-lang-en>How it self-evolves</h4>
        <p data-lang-zh>AI 从历史里学、提出"这样调会不会更好"的建议，先在影子环境拿历史对话重跑验证（<b>零副作用、不碰真实客户</b>）；只有确实更好、过了安全回归、再经你二次确认才真正上线，还能一键回滚。全程透明可查。</p>
        <p data-lang-en>It learns from history and proposes "would this tweak help?", first replaying past chats in a shadow run (<b>zero side effects, no real customers touched</b>); only if it's truly better, clears safety regression, and you confirm does it go live — with one-click rollback. Fully transparent.</p>
      </div>
      <div class="sys-card reveal reveal-d1">
        <div class="sys-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3z"/><path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>知识库健不健康</h4><h4 data-lang-en>Knowledge-base health</h4>
        <p data-lang-zh>每条知识改过几次、谁改的、改了什么都留痕可回溯；哪些还没核实、哪些有矛盾或过时，系统会标出来提示补。AI 写进来的知识一律先标"待核实"，<b>从不自说自话当成真</b>。</p>
        <p data-lang-en>Every edit to a knowledge item — how many times, by whom, what changed — is traceable; what's unverified, conflicting, or stale gets flagged for follow-up. Anything the AI writes in is marked "to be verified" first — <b>never self-certified as true</b>.</p>
      </div>
      <div class="sys-card reveal reveal-d2">
        <div class="sys-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="4" width="18" height="7" rx="1.5"/><rect x="3" y="13" width="18" height="7" rx="1.5"/><path d="M7 7.5h.01M7 16.5h.01" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>运行基础透明</h4><h4 data-lang-en>Runtime, transparent</h4>
        <p data-lang-zh>哪些微信号在线、跟进任务排了多少、信号采集有没有断流（断了会告警）、发送有没有触顶 / 退避的提醒——保障整盘稳稳跑着。</p>
        <p data-lang-en>Which WeChat accounts are online, how many follow-up tasks are queued, whether signal intake stalled (alerts if so), and send cap-hit / back-off alerts — keeping the whole board running steady.</p>
      </div>
    </div>
```

- [ ] **Step 2: 在 `observability.css` 末尾追加第 3 层样式**

```css
/* ===================== 第 3 层：系统态势（深色区） ===================== */
.sys-grid { display: grid; grid-template-columns: repeat(3,1fr); gap: var(--gap); }
.sys-card { background: linear-gradient(180deg, rgba(255,255,255,.05), rgba(255,255,255,.02)); border: 1px solid var(--ink-hair); border-radius: var(--r-lg); padding: 26px; transition: transform .25s, border-color .25s, background .25s; }
.sys-card:hover { transform: translateY(-4px); border-color: rgba(124,107,255,.4); background: linear-gradient(180deg, rgba(124,107,255,.1), rgba(255,255,255,.02)); }
.sys-card .sys-ic { width: 46px; height: 46px; border-radius: 13px; display: grid; place-items: center; margin-bottom: 15px; background: linear-gradient(135deg, rgba(94,92,230,.25), rgba(10,132,255,.18)); border: 1px solid rgba(124,107,255,.35); color: #C9C5FF; }
.sys-card .sys-ic svg { width: 23px; height: 23px; }
.sys-card h4 { font-size: 17px; font-weight: 700; color: var(--on-deep-1); margin-bottom: 9px; }
.sys-card p { font-size: 13.8px; color: var(--on-deep-2); line-height: 1.62; }
.sys-card p b { color: #C9C5FF; font-weight: 700; }
@media (max-width: 980px) {
  .sys-grid { grid-template-columns: 1fr 1fr; }
}
@media (max-width: 600px) {
  .sys-grid { grid-template-columns: 1fr; }
}
```

- [ ] **Step 3: 校验花括号 + 双语成对 + 占位注释已删 + 三层全部填充**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "c=open('assets/observability.css',encoding='utf-8').read(); assert c.count('{')==c.count('}'),'css brace'; h=open('observability.html',encoding='utf-8').read(); assert h.count('data-lang-zh')==h.count('data-lang-en'),'lang pair'; assert not any(p in h for p in ['LAYER-1-CONTENT','LAYER-2-CONTENT','LAYER-3-CONTENT']),'placeholder left'; print('ok braces',c.count('{'),'pairs',h.count('data-lang-zh'))"`
Expected: 打印 `ok braces N pairs M`，三个占位注释全部已删。

- [ ] **Step 4: 不提交（未授权），继续 Task 6。**

---

### Task 6: 结尾 CTA 文案确认（骨架已在 Task 2 落地）

**Files:**
- 无新增改动（CTA 已在 Task 2 骨架写入：跳 `trust.html` + `technology.html`，文案"可观测之上，是写进代码的红线与可复盘的工程"）。

本任务是一个**核对点**，不写新代码：确认 Task 2 写入的 CTA 区与设计文档一致。

- [ ] **Step 1: 核对 CTA 文案与跳转**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "h=open('observability.html',encoding='utf-8').read(); assert 'cta-band' in h; assert 'trust.html' in h.split('cta-band')[1]; assert 'technology.html' in h.split('cta-band')[1]; print('CTA links to trust + technology: ok')"`
Expected: 打印 `CTA links to trust + technology: ok`。

- [ ] **Step 2: 不提交（未授权），继续导航接入（Task 7+）。**

---

### Task 7–14: 把「运营透视」接入 9 个内容页（每页一个任务，模式相同）

> **执行者注意**：Task 7–14 共覆盖 8 个内容页（第 9 个 `trust.html` 因结构特殊单列为 Task 14.5 下方说明，实际同模式）。每页**三处**改动完全同构，下面给出通用三处编辑；对每个页面重复一次。这些任务彼此独立、可并行执行。

**适用页面（9 个，相对路径）**：`index.html`、`solutions.html`、`product.html`、`agents.html`、`technology.html`、`engineering.html`、`evolution.html`、`scenarios.html`、`trust.html`。

**每页三处编辑（统一模式）：**

**编辑 A — 桌面 `nav-links`**：在 `<a href="scenarios.html" ...>行业场景</a>...` 那一行与 `<a href="trust.html" ...>信任与安全</a>...` 那一行之间，插入一行：
```html
      <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
```

**编辑 B — 移动 `mobile-menu`**：在 mobile-menu 内的 scenarios 行与 trust 行之间，插入同样一行（缩进按该区为 2 空格）：
```html
  <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
```

**编辑 C — 页脚「信任」列**：在页脚 `<h5>信任/Trust</h5>` 下的 `.footer-links` 内，**最前面**插入一条（让"运营透视"排在"信任与安全"之前）：
```html
          <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
```
（注：`agents.html` 页脚"信任"列只有 2 条链接、缩进同为 10 空格，插入位置规则一致——插在该列第一条之前。）

**当前页特殊处理**：被接入的页面若是 `observability.html` 自己——不适用（本页不在 7–14 列表）。`trust.html` 接入后，其 nav 里 `trust.html` 仍带 `class="active"`，`observability.html` 不带 active（正确，因为当前页是 trust）。

#### Task 7: index.html 接入

**Files:** Modify `website/index.html`

- [ ] **Step 1: 编辑 A — 定位并插入桌面导航项**

先读取确认行内容，再用精确替换。`index.html` 桌面导航中 scenarios 与 trust 相邻两行形如：
```html
      <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
      <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust</a>
```
将其替换为（中间插入 observability 行）：
```html
      <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
      <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
      <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust</a>
```

- [ ] **Step 2: 编辑 B — mobile-menu 插入**

mobile-menu 区（2 空格缩进）scenarios 与 trust 相邻两行：
```html
  <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
  <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust</a>
```
替换为：
```html
  <a href="scenarios.html" data-lang-zh>行业场景</a><a href="scenarios.html" data-lang-en>Scenarios</a>
  <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
  <a href="trust.html" data-lang-zh>信任与安全</a><a href="trust.html" data-lang-en>Trust</a>
```

- [ ] **Step 3: 编辑 C — 页脚信任列插入**

页脚"信任"列 `.footer-links` 内第一条（通常是 `信任与安全 / Trust & safety`）之前插入运营透视行。读取页脚确认该列实际首行后，在其前插入：
```html
          <a href="observability.html" data-lang-zh>运营透视</a><a href="observability.html" data-lang-en>Observability</a>
```
（若 index.html 页脚结构与 trust.html 不同，以"信任/Trust 这一 `<h5>` 之后的第一个 `.footer-links` 的首链接前"为准。）

- [ ] **Step 4: 校验三处各加一项、双语成对**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "h=open('index.html',encoding='utf-8').read(); n=h.count('observability.html'); print('observability links:',n); assert n>=6,'expect >=6 (nav2+menu2+footer2)'; assert h.count('data-lang-zh')==h.count('data-lang-en'),'lang pair'; print('lang ok')"`
Expected: `observability links: 6`（桌面 2 + 移动 2 + 页脚 2）；`lang ok`。

- [ ] **Step 5: 不提交（未授权）。**

#### Task 8–14: 其余 8 个内容页接入（solutions / product / agents / technology / engineering / evolution / scenarios / trust）

对以下每个页面，重复 Task 7 的 Step 1–5（编辑 A/B/C + 校验 + 不提交），仅把文件名替换为对应页面：

- [ ] **Task 8: `solutions.html`** — 三处插入 + 校验 `observability links: 6`
- [ ] **Task 9: `product.html`** — 同上
- [ ] **Task 10: `agents.html`** — 同上（页脚"信任"列仅 2 条，规则不变：插在首条前）
- [ ] **Task 11: `technology.html`** — 同上
- [ ] **Task 12: `engineering.html`** — 同上
- [ ] **Task 13: `evolution.html`** — 同上
- [ ] **Task 14: `scenarios.html`** — 同上
- [ ] **Task 14b: `trust.html`** — 同上。trust 页 nav 中 trust 项保留 `class="active"`，observability 不带 active。校验命令把 `index.html` 换成 `trust.html`。

> 每页校验命令模板（把 `PAGE` 换成文件名）：
> `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "h=open('PAGE',encoding='utf-8').read(); n=h.count('observability.html'); print('PAGE',n); assert n>=6; assert h.count('data-lang-zh')==h.count('data-lang-en'); print('ok')"`

---

### Task 15: 404.html 接入（绝对路径 + 补回缺失的 solutions）

**Files:** Modify `website/404.html`

**背景**：404 页用**绝对路径** `/xxx.html`，且当前导航缺 `solutions.html`（既有不一致）。本任务接入"运营透视"并顺手补回 solutions，使其与其它页一致。404 页**无页脚**，只改 nav-links 与 mobile-menu 两处。

- [ ] **Step 1: 桌面 nav-links — 补 solutions + 插 observability**

404 页桌面导航现状（缺 solutions）：
```html
      <a href="/index.html" data-lang-zh>首页</a><a href="/index.html" data-lang-en>Home</a>
      <a href="/product.html" data-lang-zh>产品能力</a><a href="/product.html" data-lang-en>Product</a>
```
将 `<a href="/index.html"...>首页...` 行与其后的 `<a href="/product.html"...>产品能力...` 行替换为（补 solutions）：
```html
      <a href="/index.html" data-lang-zh>首页</a><a href="/index.html" data-lang-en>Home</a>
      <a href="/solutions.html" data-lang-zh>解决什么</a><a href="/solutions.html" data-lang-en>Solutions</a>
      <a href="/product.html" data-lang-zh>产品能力</a><a href="/product.html" data-lang-en>Product</a>
```
再在桌面导航的 scenarios 行与 trust 行之间插入 observability：
```html
      <a href="/scenarios.html" data-lang-zh>行业场景</a><a href="/scenarios.html" data-lang-en>Scenarios</a>
      <a href="/observability.html" data-lang-zh>运营透视</a><a href="/observability.html" data-lang-en>Observability</a>
      <a href="/trust.html" data-lang-zh>信任与安全</a><a href="/trust.html" data-lang-en>Trust</a>
```

- [ ] **Step 2: mobile-menu — 同样补 solutions + 插 observability**

mobile-menu 区（2 空格缩进）首行后补 solutions：
```html
  <a href="/index.html" data-lang-zh>首页</a><a href="/index.html" data-lang-en>Home</a>
  <a href="/solutions.html" data-lang-zh>解决什么</a><a href="/solutions.html" data-lang-en>Solutions</a>
  <a href="/product.html" data-lang-zh>产品能力</a><a href="/product.html" data-lang-en>Product</a>
```
scenarios 与 trust 之间插 observability：
```html
  <a href="/scenarios.html" data-lang-zh>行业场景</a><a href="/scenarios.html" data-lang-en>Scenarios</a>
  <a href="/observability.html" data-lang-zh>运营透视</a><a href="/observability.html" data-lang-en>Observability</a>
  <a href="/trust.html" data-lang-zh>信任与安全</a><a href="/trust.html" data-lang-en>Trust</a>
```

- [ ] **Step 3: 校验**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "h=open('404.html',encoding='utf-8').read(); o=h.count('/observability.html'); s=h.count('/solutions.html'); print('obs',o,'sol',s); assert o==2,'obs nav+menu'; assert s==2,'sol nav+menu'; assert h.count('data-lang-zh')==h.count('data-lang-en'); print('ok')"`
Expected: `obs 2 sol 2`；`ok`。

- [ ] **Step 4: 不提交（未授权）。**

---

### Task 16: 全站渲染自检 + 零黑话 / 诚实边界 / 双显终审

**Files:**
- 临时：`website/_shot.py`（自检脚本，自检后删除）
- 不改产物文件（除非自检发现问题回到对应任务修）

**Interfaces:**
- Consumes: 全部前序任务产物。
- Produces: 截图证据 + 通过/失败结论。

- [ ] **Step 1: 写渲染自检脚本 `_shot.py`**

把以下写入 `website/_shot.py`（headless Chromium，device_scale_factor=2，reduced_motion 让 reveal 直接显形）：

```python
import asyncio
from playwright.async_api import async_playwright

VIEWS = [("desk-zh", 1280, 900, "zh"), ("desk-en", 1280, 900, "en"), ("mob-zh", 390, 844, "zh")]
URL = "http://localhost:8125/observability.html"

async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch()
        for name, w, h, lang in VIEWS:
            ctx = await b.new_context(viewport={"width": w, "height": h},
                                      device_scale_factor=2, reduced_motion="reduce")
            pg = await ctx.new_page()
            await pg.goto(URL, wait_until="networkidle")
            if lang == "en":
                await pg.evaluate("window.WeAgentLang && window.WeAgentLang.set('en')")
                await pg.wait_for_timeout(300)
            await pg.screenshot(path=f"_shot_{name}.png", full_page=True)
            print("shot", name, "done")
            await ctx.close()
        await b.close()

asyncio.run(main())
```

- [ ] **Step 2: 起服务器并跑截图**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && (python -m http.server 8125 >/dev/null 2>&1 &) && sleep 1 && PYTHONUTF8=1 PYTHONIOENCODING=utf-8 python _shot.py 2>/dev/null; pkill -f "http.server 8125"`
Expected: 打印 `shot desk-zh done` / `shot desk-en done` / `shot mob-zh done`，生成三张 `_shot_*.png`。
（若缺 playwright：`pip install playwright && python -m playwright install chromium`。）

- [ ] **Step 3: 人工看图核对（用 Read 看三张 PNG）**

逐张确认：
1. **导航 10 项**：1280 宽 desk-zh / desk-en 顶栏 10 个频道一行不溢出、不换行；"运营透视/Observability"在"行业场景"与"信任与安全"之间。若溢出 → 回 Task 1 在 observability.css 加 `@media(min-width:1141px){.nav-links a{padding-left:10px;padding-right:10px}}`，重跑。
2. **中英不双显**：每个区块只显一种语言（重点看英雄三 pill、仿真截面步骤标题、三层可信度徽章、健康度仪表标签、各 flag/badge）。任何同时出现中英文的元素 → 该元素缺三条 per-element 语言规则，回对应任务补。
3. **仿真截面**：6 步时间线在 desk 三列、mobile 单列；状态色（绿=done、琥珀=warn）正确；窗口标题栏三色点显示。
4. **三层版式**：第 1 层截面+4 卡、第 2 层三层可信度条+3 卡+健康度 7 格+成效宽卡、第 3 层深色 6 卡均不塌、不溢出；深色区文字（第 3 层）在深底上清晰（用 `--on-deep-*`）。
5. **移动端 390**：所有网格塌成单列或合理列数，无横向滚动条。

- [ ] **Step 4: 零技术黑话全文扫描**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "h=open('observability.html',encoding='utf-8').read(); bad=[w for w in ['agent_run_logs','llm_call_logs','agent_decision_reviews','final_review_status','operation_state','tool_trace','held_by_ai_policy','blocked_by_safety_guard','ai_waiting_for_more_context','manual_tags','confirmed_tags','bayesian_signals','personality_profile','workspace_id','account_id','idempotency','FactRisk','PressureRisk','HumanLikeScore','EmotionalValue','ProductAccuracy','operation_knowledge_chunks','integrity_status','needs_review','MongoDB','Axum','MCP','/api','endpoint','schema'] if w in h.replace('Rust · Axum · MongoDB · React · MCP','').replace('Built with Rust · Axum · MongoDB · React · MCP','')]; print('FORBIDDEN:',bad) if bad else print('zero-jargon: clean')"`
Expected: `zero-jargon: clean`。（页脚那行技术栈署名是站点统一样式，已在判定里排除；正文区严禁任何技术标识符。）

- [ ] **Step 5: 诚实边界关键措辞核对（人工 + grep）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "h=open('observability.html',encoding='utf-8').read(); import re; bad=[]; \
 [bad.append('意图轨迹被列为可观测') for _ in [1] if '意图轨迹' in h]; \
 [bad.append('出现剩余配额数字') for _ in [1] if re.search(r'剩余\s*\d+\s*条', h)]; \
 [bad.append('用量折算金额') for _ in [1] if re.search(r'(花了|省了|成本).{0,6}(元|¥|\$|块钱)', h)]; \
 print('VIOLATION:',bad) if bad else print('honesty boundary: clean')"`
Expected: `honesty boundary: clean`。同时人工确认：① 大五人格 / 判断走势处都写明"只观测 / 不驱动 / 供参考"；② 无"用画像自动调话术""贝叶斯决定怎么回复"类表述；③ 成效板块写明"AI 永远不会自己宣布成交，必须运营者确认"；④ 当日发送只写"触顶 / 退避提醒"；⑤ 用量只写"调用次数 / 用量 / 缓存命中"不折金额。

- [ ] **Step 6: no-human-takeover 禁词扫描（本页 + 所有改动页）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "import glob; pat=['人工接管','人工介入','人工托管','转人工','takeover','hand-off','handoff']; hit=[]; [hit.append((f,w)) for f in ['observability.html'] for w in pat if w in open(f,encoding='utf-8').read()]; print('REDLINE HIT:',hit) if hit else print('no-takeover: clean')"`
Expected: `no-takeover: clean`。

- [ ] **Step 7: 全站链接一致性（10 页都能到达 observability）**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -c "import glob; pages=[f for f in glob.glob('*.html') if f!='observability.html']; miss=[f for f in pages if 'observability.html' not in open(f,encoding='utf-8').read()]; print('MISSING nav:',miss) if miss else print('all 9 pages link to observability: ok')"`
Expected: `all 9 pages link to observability: ok`（9 个非本页 HTML 均含链接）。

- [ ] **Step 8: 清理临时文件**

Run: `cd "E:/yw/agiatme/工作项目/wechatagent/website" && rm -f _shot.py _shot_*.png && echo cleaned`
Expected: `cleaned`。

- [ ] **Step 9: 不提交（未授权）。完成后向用户汇报：新页已建、10 页导航接入、自检结论（含截图所见），等待用户决定是否提交 git。**

---

## Self-Review（计划自检）

**1. Spec coverage（设计文档每节 → 任务映射）：**
- 背景与目标 → 全计划；受众/语言口径（零黑话）→ Global Constraints + Task 3 Step4 + Task 16 Step4。
- 诚实边界 6 条 → Global Constraints 逐条 + Task 4（大五/走势/成效）+ Task 16 Step5。意图轨迹不列可观测 → Task 16 Step5 grep `意图轨迹` 不出现 + 全页未写该维度（CTA 链接到 technology 承接）。
- 页面身份与导航接入（第 10 频道、位置、10 页 nav+menu+页脚、head 同款、占位域名）→ Task 2（head/nav/footer）+ Task 7–15。
- 视觉形态（截面+卡片、第 3 层紧凑网格、复用 token 不引新色）→ Task 1/3/4/5 的 CSS（仅用 shared.css token）。
- 英雄区（eyebrow/标题/lead/3 pill）→ Task 2 hero。
- 第 1 层 6 板块 → Task 3（截面承载步骤+为什么发/没发；卡承载有据可溯/评审改写/自我反省+花费）。
- 第 2 层 6 板块 → Task 4（三层可信度/大五/判断走势/健康度仪表/长期记忆/成效反响）。
- 第 3 层 6 板块 → Task 5（自治回路/24h/新想法待审/自我进化/知识库健康/运行基础）。
- 结尾 CTA → Task 2 骨架 + Task 6 核对。
- 验收清单（Playwright 1280 ZH/EN+移动390、零黑话、诚实边界、双显三规则、不提交）→ Task 16。
- YAGNI（不接后端/不写 JS 看板/不复述 trust+tech/只增导航+页脚/不加新色/无技术标识符）→ 全计划遵守；新页无 `<script>` 除既有两行。

**2. Placeholder scan：** 计划内所有代码块为完整 HTML/CSS/命令，无 TBD/TODO；HTML 里的 `LAYER-N-CONTENT` 是 Task 2 故意留的占位锚点，Task 3/4/5 各自删除并在 Step3 断言其消失——非计划缺口。

**3. Type/命名一致性：** `.obs-grid`/`.obs-card` 在 Task 3 定义、Task 4 复用，命名一致；`.sim-*`(Task3)/`.cred-*`+`.health-*`(Task4)/`.sys-*`(Task5) 各层独立无碰撞；三个 section id `layer-decision/customer/system` 在 Task 2 定义、Task 3/4/5 各自引用一致；CSS 全部追加到 Task 1 建立的同一文件，花括号配平在每个 CSS 任务校验。

**4. 修正项（自检中发现并已在上文改正）：** Header 里"Task 14 渲染自检"统一改为"Task 16"；实施总览补充 Task 15=404、Task 16=自检。

---

## 执行交接

计划完成，已保存到 `docs/superpowers/plans/2026-06-28-observability-channel.md`。两种执行方式：

1. **Subagent 驱动（推荐）** — 每个任务派一个全新 subagent 实现，任务之间我来审查，迭代快、上下文干净。
2. **行内执行** — 在本会话内按 executing-plans 批量执行，带检查点供审查。

选哪种？

