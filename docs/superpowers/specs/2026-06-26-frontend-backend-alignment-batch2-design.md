# 前后端业务对齐修复 批次2(通用化前端断裂)设计

> 本 spec 把批次2 的 9 条从父 spec `2026-06-26-frontend-backend-alignment-fixes-design.md` 的"方案要点"细化为可直接 writing-plans 的详设。父 spec 是全量 76→67 路线图,本文件只覆盖批次2。

- 状态：设计稿，待用户审。
- 基线：批次1（PR#44）已合并 main（merge `9d78282`，CI 双门全绿）。批次2 实现须基于此 main，几处文件与批次1 重叠（`types/index.ts`、`ask_human_inbox.rs`、`operations/index.tsx`）。
- 条目集（用户拍板）：**9 条通用化** = A4 / A5 / C7 / C8 / D6 / D7 / D8 / E10 / E13。C9（tier 遥测）依赖 C6（run envelope 视图）作宿主，C6 在批次3 → **C9 顺延批次3**，本批不做。

---

## 一、核心原则（贯穿 9 条）

批次2 的 9 条几乎全是同一个反模式的不同切面：**前端把行业语义（标签/维度/枚举）写死成销售域**，非销售域（情感陪伴/正式咨询/数字分身朋友态）走 `default` 回落显示英文 canonical key。修复统一走一条**已建好的基础设施**：

- **取数**：`GET /api/operation/active-view`（`src/routes/operation_view.rs:27-88`）已返回 `{dimensions: ProfileDimensionView[], taxonomies: {kind: [{id, label}]}}`，值级中文标签源自 `system_taxonomies.value.display_name`（单一真相源，`m006_taxonomy_seed.rs` 种 customer_stage 9 值等）。
- **翻译**：`profileStore.labelFor(taxonomies, kind, value)`（`frontend/src/stores/profileStore.ts:23-29`）已存在，**诚实三态**：`ok`（命中字典）/ `unknown_value`（字典内无此值，灰显原始值）/ `no_dict`（该维度无字典，灰显原始值）。**绝不显示错误的销售标签**（守诚实立场，呼应 `feedback_no_overfitting` 与通用化承诺）。
- **已验证的样板**：`PlannerViewSection`（`frontend/src/features/user-ops/legacy.tsx:2029-2096`）已用 `labelFor` 翻译 `customer_stage` 单维并按三态分流渲染。批次2 多数条目是**把这套样板扩到其余维度/标签**，不是从零建。

**边界铁律**：
1. taxonomy 字典已覆盖的维度（customer_stage / intent_level / objection_type / relationship_type）→ **纯前端**接线即可。
2. 字典未覆盖的语义（conversation_modes）→ **必须先在后端补字典**，前端才能统一走 labelFor。
3. **系统级闭集枚举**（finalReviewStatus / holdCategory）不是行业 taxonomy → 用前端**常量 label map**，不走 labelFor（这些值由后端代码定义、跨行业不变）。
4. 本批所有后端改动遵守现存红线：无人工接管 lint（不得引入 `人工/接管/takeover/hand-off`）、AI 不自动验证知识、closed-set 状态枚举在 DB 写点校验。

---

## 二、条目分组（= 后续 plan 的任务簇）

| 组 | 条目 | 前后端边界 | 已核实 |
| --- | --- | --- | --- |
| 组一 纯前端字典翻译 | A4 / C7 / E13 | 纯前端，零后端 | ✅ 取数基础设施已就位 |
| 组二 纯前端 + 类型添加 | D7 / D8 | 后端字段已存在，前端补 types + 编辑 UI | ✅ models.rs 字段已确认 |
| 组三 需后端配合 | A5 / C8 / D6 / E10 | 前后端成对改 | ✅ 后端 gap 已确认 |

---

## 组一：纯前端字典翻译扩展（3 条，零后端）

### A4. 行业化画像维度看板（多维渲染）`[前端]` — 通用化
- **现状**：`PlannerViewSection`（`legacy.tsx:2029-2096`）已用 `labelFor` 翻译 `customer_stage` 单维（含三态渲染，:2042/:2059-2080）；`useProfileStore.dimensions`（`ProfileDimensionView[]`，active profile 声明的维度列表）仍 **0 处消费**（死字段）。intent_level / value_tier / churn_reason / purchase_lifecycle 等维度无渲染。
- **修复**：PlannerViewSection 增加一个"画像维度"区块，遍历 `store.dimensions`，对每个维度从 `contact.domainAttributes[kind]` 取值 → `labelFor(taxonomies, kind, value)` 渲染，复用已有三态分流 UI（ok 正常 / unknown_value、no_dict 灰显 + title 提示）。维度顺序按 profile 声明顺序。空值维度跳过不渲染。customer_stage 既有的专属"运营阶段"行保留（它额外展示 stageUpdatedAt 时间），新区块只渲染**其余**维度避免重复。
- **边界**：维度的 displayName 来自 `ProfileDimensionView.displayName`（已下发），值标签来自 `taxonomies[kind]`（已下发）。零后端。
- **测试**：vitest 组件测——给 dimensions=[customer_stage, intent_level] + taxonomies 含 intent_level 字典，断言 intent_level 维度渲染中文 label；给一个 taxonomies 无字典的维度，断言走 no_dict 灰显原始值。
- **验收**：✅需浏览器。

### C7. autonomy 逐行 finalReviewStatus/holdCategory 中文化 `[前端]`
- **现状**：`autonomy/index.tsx:360-361` 逐行直接渲染 `item.finalReviewStatus` / `item.holdCategory` 裸英文闭集值（如 `blocked_unverified_product_claim`）；聚合层 HoldBar（:195-197）已有中文标签（"AI 策略主动暂缓"等），但逐行明细裸露英文。
- **修复**：在 autonomy 模块加两个**闭集常量 label map**（值域已 grep 实证）：
  - `FINAL_REVIEW_STATUS_LABELS`：覆盖 `final_review_status` **10 项枚举闭集**（gateway finalize 算出，`assert_final_review_status_valid` 校验；含 approved / held_by_ai_policy / blocked_by_safety_guard / blocked_unverified_product_claim / ai_waiting_for_more_context / revision_failed 等——writing-plans 阶段从 `src/agent/gateway.rs` + `assert_final_review_status_valid` 抄全 10 项准确值）。
  - `HOLD_CATEGORY_LABELS`：覆盖 `hold_category` **三选一闭集**（`DecisionReviewResult` 字段 `types.rs:1209`，`assert_hold_category_valid` 校验三值——writing-plans 阶段抄准）。
  - 逐行用 `LABELS[value] ?? value`（未知值回落原始值，不吞）。
- **边界**：这两个是**系统级闭集枚举**（后端代码定义，跨行业不变），用常量 map 而非 labelFor。措辞遵守无人工接管 lint（用"AI 策略主动暂缓"等 AI-internal 名）。**C7 与 C8 共用这两个 map** → 抽到共享常量模块（如 `frontend/src/lib/reviewLabels.ts`），C7 先建、C8 复用。
- **测试**：vitest 组件测——给已知状态值断言中文标签；给未知值断言回落原始值。
- **验收**：需浏览器。

### E13. reviewer 隐私边界维度 boundaryPrivacySafety 显形 `[前端]` — 通用化
- **现状**：`operations/index.tsx:42-50` `formatScores` 硬编码 5-key 白名单 `["humanLike","emotionalValue","hallucinationScore","knowledgeGroundingScore","pressureRisk"]` + undefined 过滤，新维度 `boundaryPrivacySafety`（渐进式三档加固加的隐私维度，后端 `ReviewScores` `types.rs:1135-1160` 已随 scores 下发）被静默丢弃。
- **修复**：`formatScores` 改为**动态遍历** `scores` 的所有 key，配一个 `SCORE_LABELS` 中文 map（humanLike→"拟人度"等，含 boundaryPrivacySafety→"隐私边界"）；缺映射的 key 回落显示原始 key 名（不吞）。这样未来新增评分维度自动显形。
- **边界**：纯前端。后端已下发 boundaryPrivacySafety，无需改。
- **测试**：vitest 组件测——scores 含 boundaryPrivacySafety 断言显示；scores 含一个未知 key 断言不被吞、回落原始 key。
- **验收**：需浏览器。

---

## 组二：纯前端 + 类型添加（2 条，后端字段已存在）

### D7. profile 5 个高级字段编辑 UI `[前端]` — 通用化
- **核实**：5 字段在后端 `DomainProfile`（`src/models.rs`）**全部已存在**：`transaction_facts_enabled`(:1792) / `reviewer_orientation`(:1859) / `mode_gate_policy_override`(:1866) / `trajectory_dimensions`(:1830) / `debounce_window_ms_override`(:1834)。前端 `types/index.ts` DomainProfile(:607-641) 与 DomainProfileDraft(:643-665) **均未声明**这 5 字段（grep 0 命中）；`strategyStore.saveDomainProfile`（:341-352）PUT 整体透传 `profileDraft`，无字段白名单 → **零后端改动**。
- **修复（前端）**：
  1. `types/index.ts`：DomainProfile + DomainProfileDraft 加这 5 字段（类型对齐后端：transaction_facts_enabled:boolean / reviewer_orientation:string|null（枚举）/ mode_gate_policy_override:string|null / trajectory_dimensions:数组 / debounce_window_ms_override:number|null）。
  2. `strategyStore.editDomainProfile`（:303-330 拷贝列表）加这 5 字段，否则编辑时不回填。
  3. ProfileEditor 折叠面板加编辑入口。`reviewer_orientation` / `mode_gate_policy_override` / `transaction_facts_enabled` 是 **publish 危险字段** → 编辑面加说明文案；保存仍走既有 publish→riskyFields 确认流（`publishDomainProfile`/`confirmRiskyActivation` 已有此机制）。
- **边界**：`trajectory_dimensions` 是结构化数组（TrajectoryDimension），编辑器可先做"只读展示 + 整体 JSON 编辑"或最小字段编辑，writing-plans 阶段定粒度（避免过度设计）。
- **测试**：vitest 组件测——编辑 transaction_facts_enabled 复选；测 editDomainProfile 回填 5 字段。
- **验收**：需浏览器。

### D8. per_relationship_operation_mode map 编辑 `[前端]` — 通用化（数字分身）
- **核实**：后端 `per_relationship_operation_mode: Option<BTreeMap<String, OperationMode>>` 已存在（`models.rs:1763`），三级回落链 `contact.operation_mode_override ?? per_relationship_operation_mode[rt] ?? operation_mode`（models.rs:1758 注释）后端已接 → **零后端改动**。前端只编辑 profile 级单个 operation_mode（`system-strategy/index.tsx:1891-1947`），无按 relationship_type 键分别配置的 map 编辑入口（grep `per_relationship` 0 命中）。
- **修复（前端）**：
  1. `types/index.ts`：DomainProfile + Draft 加 `per_relationship_operation_mode?: Record<string, OperationMode>`。
  2. strategyStore editDomainProfile 拷贝列表加该字段。
  3. ProfileEditor 加 map 编辑：按 relationship_type **canonical 键**（customer / peer / friend，与 m024 字典 + seed profile 同源）增删键，每键复用已有单 operation_mode 编辑器样板（:1891-1947 的 funnel/silence/commitment 三 toggle 模式）。
- **边界**：relationship_type 候选键应取自 active profile / taxonomy 的 relationship_type 字典（已下发），不写死 customer/peer/friend——但 DEFAULT 域 per_relationship 永远 None（护栏，见 `project_digital_twin_relationship_closure`）。
- **测试**：vitest 组件测——增一个 relationship_type 键、配 OperationMode、删键；断言 draft.per_relationship_operation_mode 结构正确。
- **验收**：需浏览器。

---

## 组三：需后端配合（4 条）

### A5. conversation_modes 中文标签（后端补字典）`[前后端]` — 通用化
- **核实**：后端 `DomainProfile.conversation_modes: Vec<String>`（`models.rs:1746`）是裸 canonical key，**无 label 字典**；DEFAULT seed 四裸串（`domain_profile.rs:793-798`：casual_relationship / value_exchange / consultative / boundary_protection）。active-view（`operation_view.rs:54-61` kinds 集）**不下发** conversation_modes，且无 `kind="conversation_mode"` taxonomy。前端 `conversationModeLabel`（`legacy.tsx:2178-2191`）写死 4 销售 case，default 回落英文 key；调用点 :2056。
- **修复（后端）**：把 conversation_modes 纳入字典机制——
  - 方案：seed 一个 `kind="conversation_mode"` 的 system_taxonomy（4 销售域值 + 中文 label，值级 label 走 `display_name`，与其余维度同一字典机制）。情感/陪伴域的 intimate_companion 等由各 profile 自己的字典补。
  - `operation_view.rs` 的 kinds 集（:54-61）加入 `conversation_mode`，使 active-view 下发该字典。
- **修复（前端）**：`conversationModeLabel` 删 switch，改读 `taxonomies` → `labelFor(taxonomies, "conversation_mode", mode)`，三态渲染（与 customer_stage 一致）。
- **边界**：seed 走 migration 还是 prompts.rs `ensure_prompt_pack_v2`？**按 `project_config_seed_in_prompts_not_migrations` 教训**——taxonomy 种子归 m006 一类的 taxonomy migration（system_taxonomies 是 taxonomy 域），writing-plans 阶段核实 m006 的 seed 模式后对齐，helper 用 upsert 防 E11000。
- **测试**：后端集成测 active-view 返回含 conversation_mode 字典；前端 vitest 测非销售模式中文化 + 字典缺失走 no_dict。
- **验收**：✅需浏览器。

### C8. DecisionReview 拦截原因四分支（后端补 emit）`[前后端]`
- **核实**：后端 `decision_review_json`（`src/routes/shared.rs:1053-1083`）**不 emit** `finalReviewStatus` / `holdCategory`（`hold_category` 全 src 零命中；`final_review_status` 仅在 `AgentRunLog` `models.rs:2642` 与 evolution/digest 路径）。前端 `types/index.ts` DecisionReview(:285-299) 缺这两字段，`operations/index.tsx:186` / `legacy.tsx:563` 仅 `approved?通过:拦截` 二元，无法区分 safety_guard / unverified_product / required_field / budget 四分支。
- **数据源核实（self-review 补，已 grep 实证）**：`decision_review_json` 的输入是 `AgentDecisionReview`（`models.rs:2493-2541`），该结构 **本身无** `final_review_status` / `hold_category` 字段。但两值都已存在于 **`AgentRunLog`**（`models.rs:2543-`，同一次 run、`run_id` 关联）：
  - `final_review_status` 是 AgentRunLog 顶层字段（10 项枚举闭集，gateway finalize 阶段算出，`gateway.rs:1845/1942/2225`，`assert_final_review_status_valid` 校验）。
  - `hold_category` 是 reviewer 输出 `DecisionReviewResult` 的字段（`types.rs:1209`，**三选一闭集**：held_by_ai_policy / blocked_by_safety_guard / …，`assert_hold_category_valid` 校验，`gateway.rs:844`），序列化进 `AgentRunLog.review` doc（`models.rs:2562`）。
  - **两值都是现成闭集字段，无需派生**（修正：前稿误判"hold_category 零命中需派生"——实为 agent 子目录 grep 截断未细看，已纠正）。
- **修复（后端）**：`decision_review_json` 按 `run_id` 关联取同 run 的 `AgentRunLog`，emit `finalReviewStatus`（取顶层 final_review_status）+ `holdCategory`（取 `review` doc 内 hold_category）。纯投影 + 一次关联查询，零新增持久化字段。**否决**在 AgentDecisionReview 冗余写字段（易与 AgentRunLog 漂移，破坏单一真相源）。
- **修复（前端）**：types DecisionReview 加 `finalReviewStatus?` / `holdCategory?`；展示**复用 C7 的 label map**（FINAL_REVIEW_STATUS_LABELS / HOLD_CATEGORY_LABELS）。C7 与 C8 共用 label map → 抽共享常量模块。
- **边界**：autonomy（C7）逐行 finalReviewStatus/holdCategory 来自 digest/run-envelope 路径（已 emit，autonomy/index.tsx:360-361 已渲染裸值）；operations 的 DecisionReview（C8）经 decision_review_json，**需后端关联 AgentRunLog 补 emit**。两者值域同一闭集、共用 label map，数据源不同。
- **测试**：后端集成测 decision_review_json 含两字段；前端 vitest 测四分支中文显示。
- **验收**：需浏览器。

### D6. 字典 is_reactivation_target / is_terminal 配置入口 `[前后端]` — 通用化
- **核实**：后端 create handler 硬编码 `is_terminal:false` / `is_reactivation_target:false`（`admin_taxonomies.rs:152-153`）；`patch_taxonomy` set_doc 白名单（:197-220）只含 label/aliases/description/deprecated，不含两 flag。前端 `TaxonomyDraft`（`system-strategy/index.tsx:50-57`）无这两字段，create/edit 表单无录入。"改字典即通用"在 UI 走不通，非销售域无法启用再激活/终态语义。
- **修复（后端）**：
  - create handler（`CreateTaxonomyRequest` 的 value）接收两 flag（可选，默认 false 保持向后兼容），写入 TaxonomyValue。
  - `patch_taxonomy` set_doc 白名单加两 flag。**注意 set_doc 键用 camelCase**：`value.isTerminal` / `value.isReactivationTarget`（对齐既有 `value.displayName` 的 camelCase 写法，:202）。
- **修复（前端）**：TaxonomyDraft + EditDraft 加 `isReactivationTarget` / `isTerminal`；create/edit 表单加两复选；提交 body 带上。
- **边界**：两 flag 改后需 `invalidate_global_taxonomy_cache()`（create 已调 :169，patch 路径同样需要——核实 patch 是否已调）。
- **测试**：后端集成测 create 落两 flag、patch 改两 flag；前端 vitest 测表单提交含两字段。
- **验收**：需浏览器。

### E10. relationship_type 识别建议富投影（审核反盲批）`[前后端]` — 通用化（数字分身）
- **核实**：后端 `collect_relationship_suggestions`（`ask_human_inbox.rs:191-212`）只投影 `suggested_value`（title/summary 都是它），`rich_params=None`，evidence / confidence / contact_wxid / occurrences 全丢。前端 `SimpleApproveReject.tsx`（55 行）仅渲染 title/summary → 决策人**盲批**改写 `contact.relationship_type`。InboxItem（`inboxApi.ts:3-18`）批次1 已有 contactWxid，但无 evidence/confidence/occurrences。
- **修复（后端）**：`collect_relationship_suggestions` 用 `rich_params` doc 承载 evidence / confidence / contactWxid / occurrences（与批次1 B2 InboxItem 富字段同思路，不破其余 InboxItem 消费方）。**数据源核实（self-review 补）**：`RelationshipTypeSuggestion`（`models.rs:2823-2845`）已持久化全部所需证据字段——`evidence: Option<String>`(:2832) / `confidence: i32`(:2834) / `occurrences: i32`(:2838) / `contact_id: String`(:2828)。后端是**纯投影，零新增模型字段**。
- **修复（前端）**：SimpleApproveReject（或一个 relationship 专用 rich 组件）富展示 AI 判断依据 / 置信度 / 客户身份 / 出现次数。承载方式与批次1 B2 对齐——B2 已在 InboxItem 直接加 category/questionForPrincipal/contactWxid/principalWxid 顶层字段，**E10 同样在 InboxItem 加 `evidence?` / `confidence?` / `occurrences?` 顶层可选字段**（contactWxid 批次1 已加，复用），前端 SimpleApproveReject 直接读，不走 richParams 分发（保持与 B2 一致的简单路径）。
- **测试**：后端集成测投影含证据字段；前端 vitest 测富展示（依据/置信度/身份）。
- **验收**：需浏览器。

---

## 三、依赖与顺序

- **组一（A4/C7/E13）+ 组二（D7/D8）** = 5 条纯前端，互相独立，可任意序。
- **组三 4 条**各自前后端成对，独立。
- **C7↔C8 共享** FINAL_REVIEW_STATUS_LABELS / HOLD_CATEGORY_LABELS → 抽共享常量模块，C7 先建、C8 复用（或同 task 内建）。
- 文件重叠（批次1 已改、批次2 再动）：`types/index.ts`（DecisionReview/DomainProfile）、`ask_human_inbox.rs`（InboxItem 投影）、`operations/index.tsx`（formatScores）。批次1 已合并 main，无冲突风险，但 plan 任务须基于最新 main 的真实行号。

## 四、不在批次2 范围

- C9（tier 遥测展示）：依赖 C6（run envelope 视图），随 C6 入批次3。
- C6 本身（run envelope 视图）：批次3。
- 其余 MEDIUM/LOW（批次3/4，见父 spec）。

## 五、全局约束（实现期绑定，writing-plans 须逐条带入 plan 的 Global Constraints）

- 子 agent 一律 `model:"opus"`；回复中文。
- 无人工接管 CI lint：`src/agent|routes|evolution` + `frontend/src` 新增行禁 `人工/接管/takeover/hand-off/人工接管/转人工`（测试目录除外）。本批 C7/C8 标签措辞用 AI-internal 名。
- 测试基线不回退：`cargo test --lib ≥350/0`、4 PBT ≥33/0、`RUSTFLAGS=-Dwarnings cargo check --tests` 0/0。本地只跑 `cargo test --lib` + 单 PBT，集成测留 CI。
- 测试只增量叠加，不删改旧维度/旧弧/旧金标。
- AI 永不自动验证知识（draft + needs_review 红线）——本批不碰知识 ingest，但 D6 taxonomy seed 须遵循既有 taxonomy 域规范。
- 前端遵守现有设计系统：tokens.css 变量、`.module.css`、4 级层级、蓝=主操作专属、紫=AI 身份专属（见 `docs/frontend-design-system.md`）。
- closed-set 枚举在 DB 写点校验（C8 finalReviewStatus/holdCategory、D6 flag）。
- git：仅在用户要求时提交；只 `git add` 具名文件，绝不 `git add -A`；commit message 末尾 `Co-Authored-By: Claude <noreply@anthropic.com>`；破坏性 gitops 须显式授权。
