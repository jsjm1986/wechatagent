# 统一收件箱 UI 重做 + 前后端全量清黑话 · 设计

日期：2026-07-04
状态：设计已获用户批准（"前后端全量清黑话" + "补全样式+适度重构"）

## 背景与动机

用户反馈两个相关问题：

1. **统一收件箱（ASK-HUMAN inbox）UI 无设计感/层次感、页面过长。**
2. **后端内部术语（黑话）直接漏给非技术运营人员看。** 截图实例：一条请示卡片显示了 `blocked_unverified_product_claim`、`high_risk_gated`、`拟答风险等级：low`、短码 `#EQERR`。

二者同根：卡片组件引用的 CSS 类名几乎全部无定义（走浏览器默认样式），且后端枚举/拼接串未经翻译就下发前端。

## 根因（全部代码亲验，带 file:line）

### UI 侧
- 卡片组件类名在 CSS 里 **0 定义**：`escalationInline*`（`EscalationInline.tsx`）、`simpleAction*`（`SimpleApproveReject.tsx`）、`chunkReview*`（`ChunkReviewCard.tsx`）、`profilePublish*`（`ProfilePublishCard.tsx`）、`lessonPromote*`（`LessonPromoteCard.tsx`）全部无对应样式。`AskHuman.css:3` 自注「精细视觉留 Task 12」，Task 12 从未落地。
- `frontend/src/features/ask-human/index.tsx:114` 的 `<h1>统一收件箱</h1>` 与 `Shell.tsx:266-270` 频道外壳标题重复。
- `.askHumanChannel`（`AskHuman.css:5-10`）无 `max-width`，卡片满宽拉伸。
- 每条永远全展开 → 106 标签候选 + 34 缺口 → 页面无限长。
- 唯一有样式的 `ProposalReleaseCard`（有 `.module.css`）因「进化发布: 0」从未出现，运营从没见过正常长相。

### 黑话侧 —— 两类暴露形态

**类型 A：结构化枚举字段（前端加字典即可翻译）**
- A1 后端 seed 已备好中文 label，只是没下发/没用：`customer_stage`(9)/`intent_level`/`objection_type`(7)/`value_tier`/`relationship_type`/`conversation_mode`/`churn_reason`/`purchase_lifecycle`，中文 `display_name` 在 `m006`/`m020`~`m028` seed。根因：`operation_view.rs:54-64` 的 active-view 端点只下发 `profile_dimensions ∪ {relationship_type, conversation_mode}`，DEFAULT profile 只含 `customer_stage`+`intent_level`，其余字典未下发 → 前端 `labelFor` 走 `no_dict` 回落显示英文 id。
- A2 纯代码枚举，无中文源，需新建映射：发送/复核终态 `run_envelope.rs:67-135`（~30 值）、风险评分 `types.rs:1135-1160`、hold 类别、gap_signal kind(10)/severity/source/status（`sources_meta.rs:363` 裸下发）、digest 卡片 kind/suggestedAction（`digest_inbox.rs:168-177`）、escalation category/verdict/status/resolvedVia。前端 `reviewLabels.ts` + `command-center/index.tsx:50-86` 已翻译 ~35 个终态+hold，但 gap_signal/digest/escalation/taxonomy 决策枚举未接，且统一收件箱未使用这些字典。

**类型 B：黑话嵌在文本/拼接串里（字典翻不了，必须改后端）**
- escalation 请示串 `escalation/mod.rs:102-105`：`format!("…触发高风险闸门（{}）…拟答风险等级：{}…", blocked_status, final_decision.risk_level)` —— **截图黑话直接根因**。
- digest 兜底文案 `knowledge_digest/mod.rs:354`：`format!("…被 {} 拦截", top_block_reason)` 直接拼英文状态码。
- inbox 文案内嵌英文字段名 `digest_inbox.rs:471,487`：`"…缺 sourceQuote…"` / `"…sourceAnchors 为空…"`。
- review.risks 明细串 `gates.rs:116-186`：`hallucination_score_8_ge_6`、`product_claim_without_verified_knowledge` 等拼接串可能透出前端。

## 设计方案

四块并行，收进一份 spec、一个实现计划。

### 一、后端：类型 B 拼接串中文化

1. **escalation 请示串**（`escalation/mod.rs:102-105`）：拼串前把 `blocked_status`、`risk_level` 映射成中文。目标文案：「该客户议题触发高风险闸门（产品说法未经核实），AI 暂不自行答复。拟答风险等级：低。请领导定夺该如何回复。」新增 `blocked_status → 中文` 与 `risk_level → 中文` 两个映射函数（放 `escalation` 或复用统一映射模块）。
2. **`blocked_unverified_product_claim` 提升为命名常量**（当前是裸字面量 `logic.rs:337`、`gateway.rs:5099`，游离于 `HOLD_CATEGORY_VALUES` 三值闭集外），纳入统一映射，避免只照闭集翻译时漏掉这个高频值。
3. **digest 兜底文案**（`knowledge_digest/mod.rs:354` 及 `:349`）：`top_block_reason` 拼串前过 `block_reason → 中文` 映射。
4. **inbox 内嵌英文字段名**（`digest_inbox.rs:471,487`）：`sourceQuote`→「原文出处」、`sourceAnchors`→「原文定位锚点」，直接改中文句子。

### 二、后端：扩 active-view 字典下发范围

`operation_view.rs:54-64` 的 `kinds` 集合补上 `objection_type`、`value_tier`、`churn_reason`、`purchase_lifecycle`（这些 seed 里已有中文 label）。补下发后前端 `labelFor` 即可翻译，无需新建映射。保持现有 `find_or_load` 缓存预热与 global scope 回落逻辑不变。

### 三、前端：翻译层补全 + 接入收件箱

1. 扩 `frontend/src/lib/reviewLabels.ts`（或同目录新建映射模块），补：
   - gap_signal：kind(10 类 orphan/broken_link/…/recall_miss)、severity(info/warning/error/high)、source(rule/llm/recall_trace)、status(pending/auto_resolved/…)
   - digest：kind、suggestedAction、severity(info/warn/critical)
   - escalation：category(high_risk_gated/out_of_scope_decision/stuck_or_undelivered)、verdict(approved/rejected/conditional/deferred/delegated_back)、status、resolvedVia
   - 风险评分名：factRisk/PressureRisk/HumanLikeScore/EmotionalValue/ProductAccuracyScore
   - 注意注释陈旧：gap_signal 实际 10 类（`models.rs:1644` 写 8）、severity 实际含 error/high（`models.rs:1653` 写只 warning/info）——按代码实际取值建字典，别照注释。
2. 把字典接到统一收件箱各卡片（`EscalationInline` / `SimpleApproveReject` / 知识面板卡片）与 gap-signal 列表。
3. 措辞严守 AI 自主定位（避开 CI 禁词 `人工`/`接管`/`takeover`/`hand-off`），沿用已定口径「AI 策略主动暂缓 / 安全门拦截 / AI 等待更多上下文」。`reviewLabels.ts` 新增行受 `scripts/check-no-human-takeover.{sh,ps1}` 扫描。

### 四、前端：统一收件箱 UI 重做（补样式 + 适度重构）

1. 删 `index.tsx:114` 面板内重复 `<h1>`，右侧操作组保留（已裁决历史 / 刷新），`justify-content: flex-end`。
2. `.askHumanChannel` 加 `max-width: 920px`（左对齐）。
3. 补全全部裸类名的 CSS（`escalationInline*`/`simpleAction*`/`chunkReview*`/`profilePublish*`/`lessonPromote*`）到 `AskHuman.css`（plain CSS 全局导入，**绝不改 .module.css 副作用导入**——Rollup tree-shake 会删光）：
   - 元信息 `<dl>` 改对齐的 `label:value` 两列网格（复用已验证的 `.resolvedEscMeta` 模式）。
   - select/input/textarea 统一宽度、间距、描边。
   - 操作页脚：主操作蓝底、次操作描边弱化。
   - 每类来源一个色标徽标（取 tokens 的 `--fill-*`），一眼区分 8 类事项。
   - 色值/圆角/描边全走 `tokens.css` 变量，禁硬编码。
4. 每条默认折叠成一行摘要 `[来源徽标] 标题 · 摘要预览 [指标chip] ▸`，点击展开显示完整表单/详情。折叠状态封装在小组件（`InboxRow` 折叠壳）内自管，**不改 `ReviewQueue` 泛型逻辑**。
5. `ProposalReleaseCard`（已有 module.css）与已裁决历史（已有 `resolvedEsc*` 样式）不动。

## 不做（YAGNI / 单独议）

- **不改后端数据流、不改 `ReviewQueue` 泛型、不动路由/端点契约**（除第二块补 active-view kinds、第一块拼串中文化外）。
- **不做左右分栏工作台**（选项 3，已排除）。
- **情绪价值阈值不一致**（后端默认 6 `runtime.rs:593` / 前端展示 5 `userOpsDomainHelpers.ts:20` / CLAUDE.md 写 <5）：这是业务阈值口径问题、涉「改阈值」红线，**单独议**，不塞进本 spec。
- **请示 severity 恒为 high**（`ask_human_inbox.rs:75` 写死）：排序分流的独立 UX 改进，**后续单独做**。
- **short_code（EQERR）**：无语义、纯人类引用编号，运营需看到，UI 标注为「请示单号」即可，不隐藏、不翻译。
- 注释陈旧（gap_signal 8→10 类、severity 词表）顺手修，不作为独立目标。

## 涉及文件清单

后端：
- `src/agent/escalation/mod.rs`（拼串中文化）、`src/agent/escalation/logic.rs`（常量提升）
- `src/agent/gateway.rs`（`blocked_unverified_product_claim` 常量引用点）
- `src/knowledge_digest/mod.rs`（兜底文案映射）
- `src/routes/knowledge/digest_inbox.rs`（内嵌字段名中文化）
- `src/routes/operation_view.rs`（扩 kinds）
- 可能新增：统一的 `block_status/risk_level → 中文` 映射（后端侧，位置实现计划定）

前端：
- `frontend/src/lib/reviewLabels.ts`（扩字典）
- `frontend/src/features/ask-human/index.tsx`（删重复标题、接字典、折叠壳）
- `frontend/src/features/ask-human/AskHuman.css`（补全样式）
- `frontend/src/features/ask-human/inline/EscalationInline.tsx`、`SimpleApproveReject.tsx`（接字典、样式类）
- `frontend/src/components/review/*Card.tsx`（补样式类，若卡片本身缺样式）

## 验证

- 前端：`cd frontend && npm run build`（tsc）+ 相关 vitest 组件测试（收件箱卡片渲染、折叠展开、字典翻译断言）。
- 后端：`cargo check` + `cargo test --lib`（≥350/0 基线不回归）+ 拼串映射的纯函数单测。
- `bash scripts/check-no-human-takeover.sh`（前端/后端新增行 0 禁词）。
- 浏览器实测（若后端栈可跑）：收件箱各卡片视觉、折叠、翻译落地；否则如实说明用组件测试替代。

## 测试基线红线

- 新增测试只 append，不删改旧维度。
- 翻译措辞不得引入 CI 禁词。
- 不为过测试改业务逻辑/阈值。
