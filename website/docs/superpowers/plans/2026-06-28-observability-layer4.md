# 运营透视页 · 第 4 层（请示 / 引荐 / 总控）增量实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在已上线的 `observability.html`（运营透视频道）现有三层之后、结尾 CTA 之前，插入一个浅色（`section-tint`）的「第 4 层 · 看懂 AI 的分寸」，含 3 个业务板块：决策请示收件箱（做重·仿真截面）、专属顾问名片引荐（做轻·卡片）、AI 总控指令回放（做轻·卡片）。

**Architecture:** 纯静态 HTML/CSS 增量。第 4 层是 `observability.html` 里新增的一个 `<section class="section section-tint">`，结构复用第 1 层的 `sim-console` 时间线截面 + 第 1/2 层的 `obs-grid cols-2` / `obs-card`。CSS 端尽量零新增——只为名片卡的「可选 · 默认关」徽标加一个 `.opt-pill` 类（含三条双语显隐规则），其余全部复用 `assets/observability.css` 既有组件。10 个 HTML 页面的导航 / 页脚一律不动（仍是第 10 个频道，导航 10 项不变）。

**Tech Stack:** HTML5 + 原生 CSS（依赖 `assets/shared.css` 的 design token）+ `assets/i18n.js` 既有双语机制（`data-lang-zh` / `data-lang-en` 属性，由 `shared.js` 配合 `<html lang>` 切换）。无构建步骤；验证靠 Playwright 渲染自检。

## Global Constraints

以下为本项目硬约束，逐字摘自设计文档 `website/docs/superpowers/specs/2026-06-28-observability-channel-design.md`，每个任务的要求都隐含包含本节：

- **no-human-takeover 红线（最高优先级）**：允许语义 =「请示 / 转述 / 幕后决策源 / 决策者 / 拍板 / 引荐名片 / AI 退为辅助答疑 / 运营者确认」（客户从不面对真人、对话始终是 AI 在说）；**绝不出现**字面或语义：`人工接管` `转人工` `人工介入` `人工托管` `接管` `人工` `handoff` `hand-off` `takeover` `human takeover`。
- **零技术黑话**：不出现任何 API 路径、数据库集合名、代码字段名、内部状态串。第 4 层禁现字面具体含：`principal_escalation` `toolCalls` `management.rs` 及任何 snake_case 字段 / 文件名 / 状态码。
- **诚实边界**：① 名片引荐必须标注「可选 · 默认关」，框定为账号级可选、默认关闭的辅助模式，不写成「交接真人 / 转人工」；台前顾问 ≠ 幕后决策源。② 客户情绪 / 用户反应（reaction）前端无独立读视图，**不列入**本页可观测清单。③ 总控回放落点是"可观测"（每步看得到、可演练、高风险拦得住），「演练不真发 / 高风险二次确认」可写。
- **双语 data-lang 双显规则**：任何带显式 `display:`（flex/grid/inline-flex/block/inline 等）的双语元素，必须配三条 per-element 语言规则：`.x[data-lang-en]{display:none}` / `html[lang="en"] .x[data-lang-zh]{display:none}` / `html[lang="en"] .x[data-lang-en]{display:<原值>}`。纯文本元素（无显式 display，如 `<h4>`/`<p>`/`<span>` 默认）靠 shared.css 全局规则即可，不需补。
- **复用既有 design token**：颜色一律走 `shared.css` :root 的 `--brand` `--brand-2` `--ai` `--ai-ink` `--running` `--held` `--held-ink` `--ink-1/2/3` `--card` `--card-2` `--hair` `--fill-brand` 等；**不引入新品牌色**，不加裸 hex（既有允许例外仅 `#C9C5FF`，第 4 层用不到）。
- **占位域名** `https://weagent.example.com` 不动；无 ICP 备案号；无 agent 集群 / worker 数量。
- **不提交 git**（未授权）。实施者创建 / 编辑文件但不 commit；审查者读工作树文件。

## 文件结构

| 文件 | 责任 | 改动 |
| --- | --- | --- |
| `website/observability.html` | 运营透视页主文档 | 在第 318 行 `<!-- 结尾 CTA -->` 注释前，插入第 4 层 `<section>`（约 90 行 HTML） |
| `website/assets/observability.css` | 该页专属样式 | 文件末尾追加第 4 层小节：仅 `.opt-pill` 徽标类 + 三条双语规则 + 必要的移动端微调（约 12 行） |

> 注：HTML 行号以**当前文件状态**为准（CTA 注释现在第 318 行 `<!-- ============ 结尾 CTA ... -->`）。实施者插入前先 Grep 定位 `结尾 CTA` 注释行，在其前插入，不要硬编码行号。

## 任务概览

- **Task 1**：第 4 层 HTML 骨架（section 容器 + layer-head + s-head 层标题/eyebrow/h2/lead）。
- **Task 2**：板块 19 — 决策请示收件箱（`sim-console` 6 步请示截面 + 截面下 2 张 `obs-card`）。
- **Task 3**：板块 20 + 21 — 名片引荐卡（含 `.opt-pill`）+ 总控回放卡（`obs-grid cols-2`）。
- **Task 4**：`observability.css` 追加 `.opt-pill` 样式 + 三条双语规则 + 移动端微调。
- **Task 5**：全页渲染自检（Playwright desktop ZH/EN + mobile）+ 诚实边界/零黑话/双显终审，清理临时文件。

每个任务结束都是一个可独立验证的交付物。Task 1-3 改 HTML，Task 4 改 CSS（Task 3 产出 `.opt-pill` 的使用、Task 4 补其定义——故 Task 4 紧随 Task 3），Task 5 整体验证。

---

### Task 1: 第 4 层 HTML 骨架

**Files:**
- Modify: `website/observability.html`（在「结尾 CTA」注释前插入新 `<section>`）

**Interfaces:**
- Consumes: 既有类 `section` `section-tint` `wrap` `s-head` `reveal` `layer-head` `layer-no` `eyebrow` `h-section` `text-grad` `lead`（均已在 shared.css / observability.css 定义）。
- Produces: 一个 `<section class="section section-tint" id="layer-boundary">`，内部含 `.layer-head`（layer-no「第 4 层 / Layer 4」）、`.s-head`（eyebrow + h2 + lead），后续 Task 2/3 在此 section 的 `.wrap` 内追加截面与卡片。section 必须**包住** Task 2/3 的内容（即先建好开/闭标签与内部 `.wrap`）。

- [ ] **Step 1: 定位插入点**

用 Grep 在 `website/observability.html` 找结尾 CTA 注释的行号（不要硬编码，文件可能已变）：

```
Grep pattern="结尾 CTA" path="website/observability.html" output_mode=content -n=true
```
Expected: 命中一行形如 `<!-- ============ 结尾 CTA（Task 6 校验文案） ============ -->`。第 4 层 section 插在**这一行之前**（即第 3 层 `</section>` 之后、CTA 注释之前）。

- [ ] **Step 2: 插入第 4 层骨架**

在结尾 CTA 注释行之前，插入以下 HTML（Task 2、Task 3 的内容稍后填进 `<!-- 板块19 -->` / `<!-- 板块20+21 -->` 占位注释处）：

```html
<!-- ============ 第 4 层：看懂 AI 的分寸（请示 / 引荐 / 总控） ============ -->
<section class="section section-tint" id="layer-boundary">
  <div class="wrap">
    <div class="s-head reveal">
      <div class="layer-head"><span class="layer-no">第 4 层 / Layer 4</span></div>
      <span class="eyebrow" data-lang-zh>看懂 AI 的分寸 · 越界的事它从不自作主张</span><span class="eyebrow" data-lang-en>Knowing its limits · it never oversteps on its own</span>
      <h2 class="h-section" data-lang-zh>遇到拿不准、超出职权的事，<span class="text-grad">它请示、不自作主张</span></h2>
      <h2 class="h-section" data-lang-en>When something is beyond its remit, <span class="text-grad">it asks — it never decides alone</span></h2>
      <p class="lead" data-lang-zh>全自治不等于没有边界。客户始终只跟 AI 对话；遇到超出 AI 职权或能力的事，它不装懂、不乱答，而是向幕后决策源请示、拿回结论后用自己的口吻转述客户。这一层，就是看懂 AI 怎么守住分寸。</p>
      <p class="lead" data-lang-en>Full autonomy is not the absence of boundaries. The customer only ever talks to the AI; when something exceeds the AI's remit or ability, it doesn't fake it or wing an answer — it consults the decision-maker behind the scenes, then relays the conclusion to the customer in its own voice. This layer is about how the AI keeps within its limits.</p>
    </div>

    <!-- 板块19：决策请示收件箱（Task 2 填充） -->

    <!-- 板块20+21：名片引荐 + 总控回放（Task 3 填充） -->

  </div>
</section>
```

- [ ] **Step 3: 验证骨架结构完整**

```
Grep pattern="id=\"layer-boundary\"" path="website/observability.html" output_mode=content -n=true
```
Expected: 命中 1 行。再确认该 section 的 `</section>` 闭合在 CTA 注释之前（Read 插入区段，目视确认 `<section ... id="layer-boundary">` … `</section>` 完整包裹两个占位注释，且 CTA 注释仍在其后）。

- [ ] **Step 4: 双语成对自检**

```
Grep pattern="data-lang-zh|data-lang-en" path="website/observability.html" output_mode=count
```
Expected: 比插入前增加 6（本任务加了 3 对：eyebrow 1 对 + h2 1 对 + lead 1 对 = 6 个 data-lang 属性）。核对：新插入段落里 `data-lang-zh` 与 `data-lang-en` 各 3 个，成对。

> 说明：`.layer-no` 是单 span 合并字串「第 4 层 / Layer 4」（与前 3 层一致），**不**拆双语对、恒显，正确。

- [ ] **Step 5: 不提交（按 Global Constraints）**

不运行 git commit。本任务交付 = 工作树中 `observability.html` 含完整第 4 层骨架 section。

---

### Task 2: 板块 19 — 决策请示收件箱（做重 · 仿真截面）

**Files:**
- Modify: `website/observability.html`（替换 Task 1 的 `<!-- 板块19：... -->` 占位注释）

**Interfaces:**
- Consumes: 既有组件 `sim-console` `sim-top` `sim-dot` `sim-title` `sim-badge`（`.ok` 态）`sim-body` `sim-step`（`.done` 态）`sim-ic` `sim-txt` `sim-foot`；卡片 `obs-grid cols-2` `obs-card` `obs-ic`。全部已在 `observability.css` 定义且自带双语规则（`sim-title` / `sim-badge` / `sim-step .sim-txt b` 已有三条 per-element 规则）。
- Produces: 一张 6 步请示截面 + 2 张 obs-card；填入 Task 1 的板块19 占位处。

**关键设计约束：**
- `sim-body` 是 `grid-template-columns: repeat(3,1fr)`（observability.css:44）。为对齐网格、零新增 CSS，请示截面做 **6 步**（与第 1 层回复截面一致，3×2 排满，无空格）。
- 所有 6 步都用 `.done` 态（请示已完成全流程）。`.done` 的图标底色是绿（observability.css:48）——语义为"该步已走完"，与第 1 层一致。
- `sim-badge` 仅 `.ok` 一种态已定义（绿色），用于「已转述客户」。

- [ ] **Step 1: 填入请示截面 + 2 卡**

把 Task 1 的 `<!-- 板块19：决策请示收件箱（Task 2 填充） -->` 整行替换为：

```html
<!-- 板块19：决策请示收件箱（仿真截面 + 2 卡） -->
    <div class="sim-console reveal">
      <div class="sim-top">
        <span class="sim-dot"></span><span class="sim-dot"></span><span class="sim-dot"></span>
        <span class="sim-title" data-lang-zh>请示裁决 · 王女士想要的折扣超出 AI 权限 · 今天 15:10</span><span class="sim-title" data-lang-en>Escalation · Ms. Wang's discount ask exceeds the AI's remit · today 15:10</span>
        <span class="sim-badge ok" data-lang-zh>已转述客户</span><span class="sim-badge ok" data-lang-en>Relayed to customer</span>
      </div>
      <div class="sim-body">
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 9v4M12 17h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>识别到超出职权</b><b data-lang-en>Spots it's beyond remit</b><span data-lang-zh>客户要的折扣超过 AI 能答应的范围，它没硬扛、也没乱许诺</span><span data-lang-en>The discount asked exceeds what the AI may grant — it neither forces it nor over-promises</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 4h16v12H5.2L4 17.2zM8 9h8M8 12h5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>整理成请示</b><b data-lang-en>Frames the question</b><span data-lang-zh>把"客户是谁、想要什么、卡在哪"理清楚，送到幕后决策者面前</span><span data-lang-en>Lays out who the customer is, what they want, where it's stuck — and sends it to the decision-maker behind the scenes</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>等决策者拍板</b><b data-lang-en>Awaits the call</b><span data-lang-zh>这期间它对客户正常维持对话，不晾着、不催</span><span data-lang-en>Meanwhile it keeps the conversation going normally — never leaves the customer hanging</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 6 9 17l-5-5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>拿回结论：有条件批准</b><b data-lang-en>Gets the verdict: approved with conditions</b><span data-lang-zh>决策者给了口径（可批 X 折，需先确认意向）＋授权窗口＋约束</span><span data-lang-en>The decision-maker sets the terms (up to X% off, confirm intent first), a window, and limits</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 12h18M3 6h18M3 18h12" stroke-linecap="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>用自己的口吻转述</b><b data-lang-en>Relays in its own voice</b><span data-lang-zh>结论落地成 AI 自然的话术发出去，客户全程只跟 AI 对话</span><span data-lang-en>The verdict becomes the AI's own natural wording — the customer talks only to the AI throughout</span></div>
        </div>
        <div class="sim-step done">
          <div class="sim-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 19V5a2 2 0 0 1 2-2h9l5 5v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z"/><path d="M8 12h8M8 16h5" stroke-linecap="round"/></svg></div>
          <div class="sim-txt"><b data-lang-zh>全程留痕可回溯</b><b data-lang-en>Logged end to end</b><span data-lang-zh>谁拍的板、什么口径、什么约束，都记下可查</span><span data-lang-en>Who decided, on what terms, with what limits — all recorded and reviewable</span></div>
        </div>
      </div>
      <div class="sim-foot">
        <span data-lang-zh>客户从头到尾只跟 AI 对话，从不知道背后有人拍过板；AI 是发起方，也是转述方——这正是"全自治、但不越权"的样子。</span>
        <span data-lang-en>The customer talks only to the AI from start to finish, never knowing a person weighed in behind the scenes; the AI both raises the question and relays the answer — this is what "fully autonomous, yet never overstepping" looks like.</span>
      </div>
    </div>

    <div class="obs-grid cols-2" style="margin-top:var(--gap)">
      <div class="obs-card reveal">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M9 11l3 3 8-8" stroke-linecap="round" stroke-linejoin="round"/><path d="M21 12v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h11" stroke-linecap="round"/></svg></div>
        <h4 data-lang-zh>裁决类型一目了然</h4><h4 data-lang-en>Verdict types at a glance</h4>
        <p data-lang-zh>批准、驳回、有条件批准、退回再议——每一种都看得到决策者给的口径、授权窗口和约束条件。AI 严格按这个口径转述，不擅自加码。</p>
        <p data-lang-en>Approve, decline, approve-with-conditions, send back for more thought — for each you can see the terms, the authorization window, and the limits the decision-maker set. The AI relays strictly to that brief, never adding on its own.</p>
      </div>
      <div class="obs-card reveal reveal-d1">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M12 3a9 9 0 1 0 9 9" stroke-linecap="round"/><path d="M21 3v6h-6" stroke-linecap="round" stroke-linejoin="round"/><path d="M12 8v4l3 2" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>请示有迹，裁决留痕</h4><h4 data-lang-en>Every consult traceable</h4>
        <p data-lang-zh>哪些事项触发了请示、由谁拍板、走的什么渠道（决策者微信、后台直接裁决、决策者对话）、结论是什么，全程留痕可回溯；需要时还能改派给备选决策人。</p>
        <p data-lang-en>What triggered the consult, who decided, through which channel (the decision-maker's WeChat, a direct call in the back office, or a chat with them), and the conclusion — all traceable; you can also reassign to a backup decision-maker when needed.</p>
      </div>
    </div>
```

- [ ] **Step 2: 步数与网格对齐自检**

```
Grep pattern="sim-step done" path="website/observability.html" output_mode=count
```
Expected: 计数 = 第 1 层 6 步 + 本层 6 步 = 12（若第 1 层有非 done 态另算；当前第 1 层为 5 个 done + 1 个 warn，故 done 计数 = 5 + 6 = 11）。关键是本层新增 6 个 `sim-step done`，3 列网格排满 2 行无空格。

- [ ] **Step 3: no-takeover 禁词扫描（本任务新增文本）**

```
Grep pattern="人工|接管|转人工|handoff|hand-off|takeover" path="website/observability.html" output_mode=content -n=true
```
Expected: **0 命中**。本板块用「请示 / 决策者 / 拍板 / 转述 / 决策源」表达，绝无禁词。

- [ ] **Step 4: 零黑话扫描（本任务新增文本）**

```
Grep pattern="principal_escalation|escalation_|_id|status=|snake_case" path="website/observability.html" output_mode=content -n=true
```
Expected: 0 命中（注意 `id="..."` 的 HTML 属性不算；这里查的是文案正文里的字段名）。目视确认正文无英文状态串。

- [ ] **Step 5: 双语成对自检**

确认本板块每个 `data-lang-zh` 都有紧邻的 `data-lang-en`：sim-title(1) + sim-badge(1) + 6 步 ×（b 1 + span 1）=12 + sim-foot(1) + 2 卡 ×（h4 1 + p 1）=4，共 19 对。

```
Grep pattern="data-lang-zh" path="website/observability.html" output_mode=count
```
Expected: 比 Task 1 后的计数增加 19。

- [ ] **Step 6: 不提交。** 交付 = 工作树 `observability.html` 第 4 层含请示截面 + 2 卡。

---

### Task 3: 板块 20 + 21 — 名片引荐 + 总控回放（做轻 · 两张卡）

**Files:**
- Modify: `website/observability.html`（替换 Task 1 的 `<!-- 板块20+21：... -->` 占位注释）

**Interfaces:**
- Consumes: `obs-grid cols-2` `obs-card` `obs-ic`（已定义）；新徽标类 `.opt-pill`（**Task 4 定义**——本任务先用，Task 4 紧接着补样式与双语规则）。
- Produces: 一个 `obs-grid cols-2`，含名片引荐卡（带一枚 `.opt-pill`「可选 · 默认关」）+ 总控回放卡。填入 Task 1 的板块20+21 占位处。

**关键设计约束（诚实边界）：**
- 名片卡必须含「可选 · 默认关」徽标，框定为账号级可选、默认关闭的辅助模式。措辞只用「引荐名片 / AI 退为辅助答疑」；**绝不**写「交接真人 / 转人工 / 接管」。点明台前顾问 ≠ 幕后决策源。
- 总控卡落点是"可观测"：每步看得到、可演练（dry-run 不真发）、高风险二次确认。不写 `toolCalls` / `management.rs`。

- [ ] **Step 1: 填入两张卡**

把 Task 1 的 `<!-- 板块20+21：名片引荐 + 总控回放（Task 3 填充） -->` 整行替换为：

```html
<!-- 板块20+21：名片引荐 + 总控回放 -->
    <div class="obs-grid cols-2" style="margin-top:var(--gap)">
      <div class="obs-card reveal">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="8.5" cy="11" r="2"/><path d="M5 16c.6-1.6 2-2.4 3.5-2.4S11.4 14.4 12 16M15 9h4M15 13h3" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>到了该见真人的时候，它会引荐</h4><h4 data-lang-en>When it's time for a human, it makes the intro</h4>
        <p data-lang-zh>某些高价值时刻——客户明确要签约、想到店参观——AI 会主动把真人专属顾问的微信名片递给客户，自己退到辅助答疑。哪些客户已被引荐、它何时退的辅助、每张名片覆盖了多少客户、回应如何，你都看得到。台前顾问负责临门一脚，与幕后决策者是两回事。</p>
        <p data-lang-en>At certain high-value moments — a customer clearly ready to sign, or wanting to visit in person — the AI proactively hands over a real advisor's WeChat card and steps back into a support role. Which customers got an intro, when it stepped back, how many customers each card reached, and the responses — all visible. The front-stage advisor closes the last mile, and is separate from the behind-the-scenes decision-maker.</p>
        <span class="opt-pill" data-lang-zh>可选 · 默认关闭的辅助模式</span><span class="opt-pill" data-lang-en>Optional · assist mode, off by default</span>
      </div>
      <div class="obs-card reveal reveal-d1">
        <div class="obs-ic"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M5 7h14M5 12h14M5 17h9" stroke-linecap="round"/><path d="M18 16l2 2 3-3.5" stroke-linecap="round" stroke-linejoin="round"/></svg></div>
        <h4 data-lang-zh>用大白话指挥它，每一步都看得到</h4><h4 data-lang-en>Direct it in plain words — every step visible</h4>
        <p data-lang-zh>你用日常的话给后台 AI 下指令（比如"给最近问过价的客户发个新品提醒"），它会拆成一步步要做的事逐个执行，每一步成功没成功都摊开。可以先"演练一遍"只看它打算做什么、并不真发；高风险的计划必须你二次确认才会执行。</p>
        <p data-lang-en>Give the back-office AI an instruction in everyday words (say, "send a new-arrival nudge to customers who recently asked about price"), and it breaks the job into steps and runs them one by one, each step's success or failure laid open. You can "dry-run" first to see what it intends without anything actually sending; any high-risk plan runs only after you confirm a second time.</p>
      </div>
    </div>
```

- [ ] **Step 2: `.opt-pill` 使用自检**

```
Grep pattern="opt-pill" path="website/observability.html" output_mode=content -n=true
```
Expected: 2 行命中（中英各 1）。注意此时 `.opt-pill` 样式尚未定义（Task 4 补），渲染上暂时按默认 inline 显示——不影响本任务交付（HTML 正确即可）。

- [ ] **Step 3: no-takeover 禁词扫描**

```
Grep pattern="人工|接管|转人工|交接|handoff|hand-off|takeover" path="website/observability.html" output_mode=content -n=true
```
Expected: **0 命中**。名片卡用「引荐 / 递名片 / 退到辅助答疑」，绝无「交接真人 / 转人工 / 接管」。

- [ ] **Step 4: 双语成对自检**

本任务加 5 对：名片卡 h4 1 + p 1 + opt-pill 1 = 3 对；总控卡 h4 1 + p 1 = 2 对。

```
Grep pattern="data-lang-zh" path="website/observability.html" output_mode=count
```
Expected: 比 Task 2 后的计数增加 5。

- [ ] **Step 5: 不提交。** 交付 = 工作树 `observability.html` 第 4 层三块齐全（HTML）。

---

### Task 4: `.opt-pill` 样式 + 双语规则（CSS）

**Files:**
- Modify: `website/assets/observability.css`（文件末尾追加第 4 层小节）

**Interfaces:**
- Consumes: shared.css token `--held`（#FF9F0A）`--held-ink`（#8A5500，浅底可读）`--card-2` `--hair` `--ink-3`；既有双语显隐三段式模式。
- Produces: `.opt-pill` 类（名片卡「可选 · 默认关」徽标），含三条双语规则。

**关键设计约束：**
- `.opt-pill` 是 `display: inline-flex` 的双语元素 → **必须**配三条 per-element 双语规则，否则 ZH+EN 双显。
- 用 `--held` / `--held-ink` 系（琥珀，语义=「可选/注意，非默认」），与 `.cred-flag.norec` 的中性灰区分开，让"默认关"有恰当的提示色但不刺眼。
- 名片卡里 opt-pill 在 `<p>` 之后，需与正文有间距 → `margin-top`。

- [ ] **Step 1: 追加 `.opt-pill` 样式到文件末尾**

在 `website/assets/observability.css` 末尾（第 138 行后）追加：

```css

/* ===================== 第 4 层：看懂 AI 的分寸 ===================== */
/* 名片引荐卡的「可选 · 默认关」徽标 */
.opt-pill { display: inline-flex; align-items: center; margin-top: 14px; font-size: 12px; font-weight: 700; padding: 4px 12px; border-radius: 999px; color: var(--held-ink); background: rgba(255,159,10,.12); border: 1px solid rgba(255,159,10,.28); }
.opt-pill[data-lang-en] { display: none; }
html[lang="en"] .opt-pill[data-lang-zh] { display: none; }
html[lang="en"] .opt-pill[data-lang-en] { display: inline-flex; }
```

> 第 4 层的 section 容器（`section-tint`）、s-head、layer-head、layer-no、sim-console、obs-grid、obs-card 全部复用既有样式，**无需新增**。本任务唯一新增的就是 `.opt-pill`。

- [ ] **Step 2: 双语规则完整性自检**

```
Grep pattern="opt-pill" path="website/assets/observability.css" output_mode=content -n=true
```
Expected: 4 行——基样式 1 + 三条双语规则（`[data-lang-en]{display:none}` / `html[lang="en"] ...[data-lang-zh]{display:none}` / `html[lang="en"] ...[data-lang-en]{display:inline-flex}`）。三条齐全则无双显。

- [ ] **Step 3: 无裸新色自检**

```
Grep pattern="#[0-9A-Fa-f]{3,6}" path="website/assets/observability.css" output_mode=content -n=true
```
Expected: 仅既有的 `#C9C5FF`（2 处，第 3 层深色区）命中；**本任务不得新增任何裸 hex**——`.opt-pill` 的颜色用 `var(--held-ink)` + `rgba(255,159,10,...)`（与 observability.css 既有 `.sim-badge.ok` 的 `rgba(48,209,88,.14)` 同模式，rgba 透明度叠色非新品牌色，允许）。

- [ ] **Step 4: 大括号配平自检**

```
Grep pattern="\{" path="website/assets/observability.css" output_mode=count
```
与 `}` 计数比较，应相等（追加了 4 条规则，各 1 对）。

- [ ] **Step 5: 不提交。** 交付 = 工作树 `observability.css` 含 `.opt-pill` 完整定义。

---

### Task 5: 全页渲染自检 + 终审 + 清理

**Files:**
- 临时：`website/_l4shot.py`（Playwright 脚本，自检后删）
- 临时：`website/_l4_*.png`（截图，自检后删）

**Interfaces:**
- Consumes: 完整的 `observability.html` + `observability.css`（Task 1-4 产出）。
- Produces: 自检结论；无新文件留存。

- [ ] **Step 1: 非视觉门 — no-takeover 全页扫描**

```
Grep pattern="人工接管|转人工|人工介入|人工托管|接管|handoff|hand-off|takeover|human takeover" path="website/observability.html" output_mode=content -n=true
```
Expected: **0 命中**。任一命中 → 回对应任务修文案。

- [ ] **Step 2: 非视觉门 — 零黑话全页扫描**

```
Grep pattern="principal_escalation|toolCalls|management\.rs|operation_state|customer_stage|agent_run_logs|tool_trace|status=draft" path="website/observability.html" output_mode=content -n=true
```
Expected: **0 命中**。

- [ ] **Step 3: 非视觉门 — reaction 未列入可观测**

```
Grep pattern="情绪分析|用户反应|reaction|客户情绪" path="website/observability.html" output_mode=content -n=true
```
Expected: 0 命中（设计明确 reaction 前端无读视图，不列入本页）。

- [ ] **Step 4: 非视觉门 — 全页双语成对**

```
Grep pattern="data-lang-zh" path="website/observability.html" output_mode=count
```
与 `data-lang-en` 计数比较，应**相等**（全页 ZH 属性数 = EN 属性数）。不等 → 有落单的双语元素。

- [ ] **Step 5: 起静态服务器 + 写截图脚本**

创建 `website/_l4shot.py`：

```python
from playwright.sync_api import sync_playwright

URL = "http://localhost:8127/observability.html#layer-boundary"

with sync_playwright() as p:
    b = p.chromium.launch()
    for name, w, h, lang in [("l4-zh", 1280, 1400, "zh"), ("l4-en", 1280, 1400, "en"), ("l4-mob", 390, 1400, "zh")]:
        ctx = b.new_context(viewport={"width": w, "height": h},
                            device_scale_factor=2, reduced_motion="reduce")
        pg = ctx.new_page()
        pg.goto(URL, wait_until="networkidle")
        if lang == "en":
            pg.evaluate("window.WeAgentLang && window.WeAgentLang.set('en')")
            pg.wait_for_timeout(300)
        # 滚到第 4 层
        pg.evaluate("document.getElementById('layer-boundary').scrollIntoView()")
        pg.wait_for_timeout(400)
        pg.screenshot(path="_l4_%s.png" % name, full_page=False)
        print("shot", name, "done")
        ctx.close()
    b.close()
```

运行：

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/website" && python -m http.server 8127 >/dev/null 2>&1 &
sleep 1.5
python _l4shot.py
```
Expected: 打印 `shot l4-zh done` / `shot l4-en done` / `shot l4-mob done`，生成 3 个 PNG。

- [ ] **Step 6: 目视审 3 张截图**

用 Read 看 `_l4_zh.png` / `_l4_en.png` / `_l4_mob.png`，逐项确认：
1. 第 4 层在浅色（section-tint）区，与上方第 3 层深色衔接自然。
2. 请示截面 6 步 3×2 排满、无空格；6 步图标均为绿色 done 态；右上「已转述客户」徽标显示。
3. 截面下 2 卡（裁决类型 / 留痕）并排；名片+总控 2 卡并排。
4. 名片卡有「可选 · 默认关闭的辅助模式」琥珀徽标。
5. **中英不双显**：ZH 截图无英文、EN 截图无中文（尤其 sim-title / opt-pill / sim-badge）。
6. 移动端：所有 grid 塌成单列，无横向滚动。

- [ ] **Step 7: 关服务器 + 清理临时文件**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent/website" && rm -f _l4shot.py _l4_*.png
for pid in $(netstat -ano 2>/dev/null | grep ':8127' | grep LISTENING | awk '{print $NF}' | sort -u); do taskkill //F //PID $pid >/dev/null 2>&1; done
```
Expected: 临时文件清空，服务器停止。

- [ ] **Step 8: 不提交。** 交付 = 自检结论汇报；工作树含完整第 4 层、无临时残留。

---

## Self-Review

**1. Spec coverage（对照设计文档第 4 层增量）：**
- 板块 19 决策请示收件箱（重·截面）→ Task 2 ✓
- 板块 20 名片引荐（轻·卡+「可选默认关」）→ Task 3 + Task 4（pill 样式）✓
- 板块 21 总控回放（轻·卡）→ Task 3 ✓
- 第 4 层 layer-head/eyebrow/h2/lead → Task 1 ✓
- section-tint 落点（第 3 层后 CTA 前）→ Task 1 ✓
- 诚实边界（no-takeover/零黑话/reaction 不列/名片可选默认关）→ 每任务内嵌扫描步 + Task 5 全页门 ✓
- 双语双显规则 → Task 4（.opt-pill 三条）+ 各任务成对自检 + Task 5 全页配平 ✓
- 复用 token 不新增色 → Task 4 Step 3 ✓
- 导航 10 页不动 → 计划范围明确不含其它 HTML ✓
- 不提交 git → 每任务末步 ✓

**2. Placeholder scan：** 无 TBD/TODO；每个改 HTML/CSS 的 step 都有完整代码块；每个 Grep/bash 步都有 Expected。✓

**3. Type/类名一致性：** `.opt-pill`（Task 3 用、Task 4 定义，名称一致）；`section-tint` `sim-console` `sim-step done` `obs-grid cols-2` `obs-card` `obs-ic` `sim-badge ok` 均与 observability.css 现有类名逐字一致（已对照文件确认）；`id="layer-boundary"` 在 Task 1 建、Task 5 脚本引用，一致。✓

## Execution Handoff

计划已存 `website/docs/superpowers/plans/2026-06-28-observability-layer4.md`。
