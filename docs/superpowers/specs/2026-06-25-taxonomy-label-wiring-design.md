# 取值字典接线（行业化标签 + AI 生成取值）设计

- 日期：2026-06-25
- 分支：tag-trust-clean（实现时另开干净分支，从 main 切）
- 状态：设计待复审

## 1. 背景与动机

WechatAgent 是"全 AI 自治"的微信私域运营 agent，设计目标是用 `DomainProfile` 配置适配任意行业（销售 / 情感陪伴 / 教育 / 医美咨询等），后端引擎行业无关。

2026-06-25 一次"配置通用性全审"（25-agent workflow + 独立证伪）的结论：**配置容器（DomainProfile）结构表达力够用，引擎核心真通用；真正的缺口在边缘接线层**。其中排序第一的命门是：

> **取值字典断层**：单一真相源 `system_taxonomies`（`TaxonomyValue`: canonical `id` + `display_name`）存在，但到两个消费侧无自动单源通路——prompt 侧维度取值指引留空靠 description 自由文本、前端侧 stageLabel 直出英文 canonical / relationship 下拉 / completeness 五维写死销售枚举。

同时，"行业配置 agent 化"实测的最大缺口是：`generate_domain_profile_candidate` 生成了维度声明，但**取值字典零生成**——AI 配出维度，运营还得手动一条条建取值字典，否则维度形同虚设。

本设计把这两件事合为一个完整闭环：**AI 生成字典（候选）→ 人审采纳 → prompt 用它判得准 + 前端用它显得对**。

### 1.1 核心立场（已与用户确认）

- **单一真相源**：所有取值标签翻译、prompt 取值指引、AI 生成取值，都围绕 `system_taxonomies` 这一份字典，不引副本（与项目反 drift 立场一致）。
- **诚实优先于好看**：字典查不到时绝不显示错误的销售标签（实质误导），而是诚实暴露 + 引导补全。
- **AI 永不自动 verify**：AI 生成的取值只进候选层（`taxonomy_candidate`），绝不直接进正式字典，复用已有 `approve` 人审采纳通路。

## 2. 现状实证（落笔前核对 HEAD，纠正过期结论）

| 组件 | HEAD 真实状态 | 影响 |
| --- | --- | --- |
| `system_taxonomies` 真相源 | ✅ `TaxonomyValue` 含 `id` + `display_name` + `aliases` + `status`（models.rs:2387 区域） | 翻译/指引/生成都基于它 |
| `TaxonomyCache` | ⚠️ `CachedEntry`（taxonomy.rs:79）**没缓存 `display_name`**——reload 时只取 id/aliases/status/priority_weight/is_terminal/is_reactivation_target，丢掉了 display_name | 流 A 必须给缓存加字段 |
| `kind_has_entries` | ✅ 已有（taxonomy.rs:297）判维度有无字典 | 三流共用的"有无字典"判断 |
| `dimension_value_weights` | ✅ 已有（taxonomy.rs:262）但返回 `(id, weight, is_terminal, is_reactivation)`，**不含 label** | 流 A 需新增带 label 的查询函数 |
| `check_value` 四态 | ✅ Active/Alias/Deprecated/**CandidateNew**（decision_taxonomy.rs:59）——AI 产字典外值落 `taxonomy_candidate` | 流 A 不碰它（它在决策后校验，流 A 改决策前指引） |
| `render_decision_dimensions_guidance` | ✅ 已有（domain_profile.rs:1182）但只渲染 `description` 自由文本 | 流 A 改造点 |
| `active_domain_profile` 端点 | ⚠️ **已存在**（domain_profiles.rs:129，路由 mod.rs:927）但 **admin 权限**（`Extension<AuthenticatedAdmin>`）、**只返 profile 不返 taxonomy 取值字典** | 流 B 新增运营态聚合端点（补权限 + 聚合 taxonomy） |
| `profileStore.ts` | ⚠️ **已存在**（已调 `/api/admin/domain-profiles/active` + 有降级逻辑） | 流 B **扩展**它（加 taxonomies + labelFor），非新建；数据源换运营态端点 |
| `upsert_candidate` / `approve` | ✅ 已有（taxonomy.rs:319 / :437）候选→审核→并入字典闭环 | 流 C 复用 |
| `generate_domain_profile_candidate` | ✅ 已有（guide_profile.rs:251）生成维度声明，schema 在 :199 区域，要求 ≥3 维度；**取值字典零生成** | 流 C 改造点 |

## 3. 总体架构与三条数据流

单一真相源 `system_taxonomies`。三条数据流共享它，互不直接耦合（可独立测试/交付），合起来是完整闭环。

```
【流 A · prompt 侧 — AI 决策准确】
decision.rs 渲染维度取值指引 (render_decision_dimensions_guidance)
  → 查 TaxonomyCache(加了 display_name 字段)
  → kind_has_entries 判断:
      有字典 → 注入「合法取值: canonical(中文名) / ...」+ 保留 description
      无字典 → 注入「本维度暂无受控取值,据语义判断,新值会被收集待运营确认」
  → AI 判得准、命中字典、candidate 噪声下降

【流 B · 前端侧 — 运营可读】
新增 GET 运营态只读端点 (非 admin)
  → 返回 { dimensions: [profile维度声明], taxonomies: {kind: [{id,label}]} }
  → profileStore(扩展现有) 缓存
  → labelFor(kind, value) 三情形分流
  → stageLabel / relationship 下拉 / completeness 五维 字典驱动

【流 C · AI 生成 — 冷启动有字典】
generate_domain_profile_candidate 生成维度时
  → 同时为每维度生成初始取值集(id + 中文 label, suggestedValues)
  → 落 taxonomy_candidate (复用 upsert_candidate)
  → 运营在已有审核界面逐值 approve → 并入 system_taxonomies
  → 流 A / 流 B 即刻可用
```

## 4. 流 A：prompt 侧取值指引接线

**目标**：决策时把维度的合法取值（带中文名）注入 prompt，让 AI 判得准、命中字典；无字典时明确告知"暂无受控取值"。

**范围**：流 A 只做 **extra 维度**（domainSignals 容器里、`participates_in_decision` 且非 typed 的维度），改 `render_decision_dimensions_guidance` 纯 Rust 函数。**typed 维度**（customer_stage / intent_level 等，取值指引散在 prompts.rs Soul/方法论/对话模式判定散文里）的行业化不在流 A——走 §6.5 的 profile override 生成路径（不改销售散文，复用已有 override 整段替换机制）。

### 4.1 TaxonomyCache 加 display_name（taxonomy.rs）

- `CachedEntry`（:79）增加 `display_name: String` 字段；reload 填充处（:144 区域）从 `entry.value.display_name` 填入（现在被丢弃）。
- 新增查询函数 `dimension_values_with_labels(kind, scope_account_id, cache) -> Vec<(String, String)>`：返回 `(canonical_id, display_name)` 对，只取 `status == "active"` 的条目，scope 回落（account 私有优先 global）逻辑与 `dimension_value_weights`（:270）一致。

### 4.2 渲染取值指引（domain_profile.rs:1182 `render_decision_dimensions_guidance`）

签名需要能访问 cache（当前只收 `&[ProfileDimension]`）。对每个 `participates_in_decision` 的维度：

- `kind_has_entries == true` → 渲染：`「{display_name}」合法取值：first_contact（初次接触）/ qualified（已确认意向）/ …` + 保留 `description` 作补充说明。
- `kind_has_entries == false` → 渲染：`「{display_name}」暂无受控取值，请据对话语义判断；新取值会被收集为候选待运营确认。`

**全注入**（已与用户确认）：维度的全部 active 取值都注入，不设上限——维度取值通常个位数到十几个，token 可控；截断会让 AI 误以为"只有这几个合法"。

### 4.3 边界

- 不碰 `check_value` 四态逻辑（它在决策**之后**校验 AI 输出；流 A 只改决策**之前**的指引）。
- 调用点（decision.rs 渲染 `render_decision_dimensions_guidance` 处）需传入 cache 引用——实现时确认该处能取到 `global_taxonomy_cache()`。

### 4.4 测试

- 缓存：reload 后 `dimension_values_with_labels` 返回带 label 的取值对（`taxonomy_cache_for_tests` 已有构造器，纯函数确定性单测）。
- 渲染：注入固定 cache，断言——有字典维度的指引串含中文取值；无字典维度的指引串含"暂无受控取值"话术。确定性单测锁死。
- 基线：lib ≥350/0 不回归（现有 G1 测试 domain_profile.rs:2312 区域断言空维度渲染空串，新签名需兼容）。

## 5. 流 B：前端侧翻译接线

**目标**：运营看客户画像时，维度值显示中文行业标签而非英文 canonical；下拉 / 五维卡按当前 profile 字典驱动。

### 5.1 新增运营态只读端点（`src/routes/operation_view.rs`，新文件）

- `GET /api/operation/active-view`（路径前缀实现时对齐现有运营态惯例，参照 mod.rs 现有 `/operation-knowledge`、`/operation-domains` 命名）。
- 权限：运营态 `require_session`（**不要** admin）——挂在 `/api` 普通鉴权下。区别于已有 admin-only 的 `active_domain_profile`。
- 实现：`load_active_domain_profile`（已有，30s 缓存）拿维度声明 + 取 taxonomy 取值字典（带 display_name）。**取数走 TaxonomyCache（已在 4.1 补 display_name）**，与流 A 单一取数路径一致；无 active profile 时 `dimensions: []` + `taxonomies: {}`（合法状态，运行时回落 DEFAULT）。
- 返回结构（camelCase wire）：

```json
{
  "dimensions": [
    {"kind": "customer_stage", "displayName": "客户阶段", "participatesInDecision": true}
  ],
  "taxonomies": {
    "customer_stage": [{"id": "first_contact", "label": "初次接触"}],
    "relationship_type": [{"id": "customer", "label": "客户"}]
  }
}
```

### 5.2 扩展 profileStore（`frontend/src/stores/profileStore.ts`，现有文件）

- 现状：调 `/api/admin/domain-profiles/active`，存 `activeProfile`，有降级。
- 扩展：数据源改调新运营态端点 `/api/operation/active-view`，新增 `dimensions` + `taxonomies` state；暴露 `labelFor(kind, value): LabelResult`。
- 保留现有降级语义：拿不到数据时 `labelFor` 一律回落 `no_dict`（前端照常跑，只是没行业化数据）。

### 5.3 labelFor 三情形分流（核心翻译层）

```ts
type LabelResult = { text: string; status: 'ok' | 'unknown_value' | 'no_dict' }

labelFor(kind, value):
  taxonomies[kind] 不存在/为空        → { text: value, status: 'no_dict' }       // 缺配:维度无字典
  taxonomies[kind] 有但 value 不在内  → { text: value, status: 'unknown_value' } // 野值:AI 产了字典外值/旧值残留
  命中                                → { text: <label>, status: 'ok' }          // 正常
```

渲染约定（遵守现有设计系统，记忆铁律 [[frontend_follow_design_system]]）：

- `ok` → 正常显示 label。
- `unknown_value` → 显示原始 value + 灰化（`--muted` token）+ 角标提示"未知取值"。
- `no_dict` → 显示原始 value + 灰化 + "待配置"提示。

**理由**：三情形分流区分"数据野值"与"配置缺失"两种本质不同的问题，绝不显示错误销售标签（守诚实立场），并把"哪里字典没配好"变成运营可见信号（自驱动补全）。

### 5.4 三个渲染点改造

- `stageLabel`（legacy.tsx:2011）：`labelFor('customer_stage', stage)` 取代直出 canonical。
- `relationship_type` 下拉（legacy.tsx:447 区域）：选项来自 `taxonomies.relationship_type`（取代写死枚举），label 走字典。
- `completeness` 五维（trustTypes.ts:45 区域）：维度来自 `dimensions`（取代写死销售五维）——后端 catalog.rs:606 区域已动态回 profile 维度，前端改成读 `dimensions` 即对齐。

### 5.5 测试

- `labelFor` 三情形纯函数单测（vitest）。
- 端点返回结构测（后端）。
- 三渲染点改造后 vitest 组件测不回归（现有前端 168 测试基线）。

## 6. 流 C：AI 生成取值字典

**目标**：AI 生成 profile 时连每个维度的初始取值集一起生成，落候选层，运营审核采纳后字典从无到有。

### 6.1 扩 generate prompt schema（guide_profile.rs:199 区域）

`profileDimensions[]` 每个维度附带初始取值集：

```json
"profileDimensions": [{
  "kind": "consultation_stage",
  "displayName": "咨询阶段",
  "participatesInDecision": true,
  "description": "...",
  "suggestedValues": [
    {"id": "initial_inquiry", "label": "初步咨询"},
    {"id": "plan_discussion", "label": "方案沟通"},
    {"id": "post_treatment", "label": "术后回访"}
  ]
}]
```

prompt 指引 AI：`id` 用 snake_case 英文 canonical、`label` 用中文行业术语、每维度给 3-8 个典型取值。

### 6.2 落候选层（guide_profile.rs 生成流程末尾）

profile 候选落库后，遍历各维度 `suggestedValues`，对每个值调已有 `upsert_candidate`（taxonomy.rs:319）落 `taxonomy_candidate`：scope=global、kind=维度 kind、value 带 id + label。与运行时 AI 跑出的新值进**同一张候选表、同一个 approve 通路**——一套机制。

### 6.3 红线守护

- AI 生成的取值**只进候选层，绝不直接进 `system_taxonomies`**（守"AI 永不自动 verify"红线）。
- profile 本身仍 `is_active=false` 待人审（现有红线不动）。
- 运营在已有 taxonomy 审核界面**逐值 approve**（复用现有机制，不新建批量通路——冷启动一次性动作，逐值审符合"人审每个 AI 产出"）。

### 6.4 失败软化

取值生成是 profile 生成的**附加产物**：若 AI 没给 `suggestedValues` 或格式不对，profile 照常落库（维度声明仍在），只是没初始候选，运营手配或运行时生长兜底。不因取值生成失败阻断 profile 生成（仿 guide_profile 现有 `coerce_scalar_string_fields` 软化风格）。

### 6.5 typed 维度行业化：生成 override（不改销售散文）

**问题**：customer_stage / intent_level 等 **typed 维度**的取值指引不在 `render_decision_dimensions_guidance`（那只管 domainSignals 的 extra 维度），而是散落在 `prompts.rs` 的 Soul / 方法论 / 对话模式判定 prompt 模板里，用销售词硬编码（prompts.rs:498 stage_method「陌生接触/需求探索/方案评估…」、:798 Soul 取值举例、:986「customer_stage ∈ {方案匹配,异议处理…} → consultative」）。这些是 **DEFAULT 销售域兜底**，且取值与业务规则焊死——直接改散文换词会破坏 DEFAULT、且治标不治本（类 2/类 3 取值嵌在规则里）。

**更优解（已与用户确认）**：typed 维度行业化**不改 prompts.rs 散文**，而是复用**已存在的 profile override 整段替换机制**，把它并进 AI 生成：

- 已有机制（引擎零改动）：`soul_override`（decision.rs:292 整体替换出厂人格）、`methodology_override`（decision.rs:370 整体替换运营方法论 / stage_method）、`conversation_mode_policy` override（domain_profile.rs:307 `strip_conversation_mode_section` 剥离销售判定段 + :331 `apply_conversation_mode_policy` 注入本行业规则）。
- 改造：`generate_domain_profile_candidate` 的 prompt schema 增产 `soulOverride` / `methodologyOverride` / `conversationModePolicy` 三段（现状落 serde default → 即不生成）。AI 据行业描述生成本行业的人格本体、阶段方法论、对话模式判定规则——typed 维度的取值语义 + 驱动规则自然包含其中。
- DEFAULT 销售域：override=None（现有 DEFAULT_PROFILE 行为不变）→ 走 prompts.rs 销售兜底，**字节等价不回归**。
- 非销售行业：激活自己的 profile → 三个 override 整段替换销售散文 → AI 看到本行业的阶段取值 + 判定规则。

**红线**：这三段 override 仍随 profile 走 `is_active=false` 候选 + 人审 publish/activate（现有红线）。生成是附加产物，缺失则该字段 None 回落销售兜底（软化，§6.4 同款）。

### 6.6 测试

- schema 解析：含 `suggestedValues` 的生成输出能正确解析（仿 guide_profile 现有 coerce 测试 :460 区域）。
- 候选落库：生成后 `taxonomy_candidate` 有对应条目（集成测试，标 `#[ignore]` 需 Docker）。
- 软化：缺 `suggestedValues` 时 profile 仍落库不 panic（单测）。
- override 生成：含 `soulOverride` / `methodologyOverride` / `conversationModePolicy` 的生成输出正确解析落 profile 字段；缺失时落 None（单测）。

## 7. 代码落点汇总

| 文件 | 改动 | 流 |
| --- | --- | --- |
| `src/agent/taxonomy.rs` | `CachedEntry` 加 `display_name` + reload 填充 + `dimension_values_with_labels` 查询函数 | A、B |
| `src/agent/domain_profile.rs` | `render_decision_dimensions_guidance` 接 cache，按有无字典渲染取值指引 | A |
| `src/agent/decision.rs` | 调用点传 cache 引用 | A |
| `src/routes/operation_view.rs`（新建） | `GET /api/operation/active-view` 运营态聚合端点 | B |
| `src/routes/mod.rs` | 注册新端点（`/api` 普通鉴权下） | B |
| `frontend/src/stores/profileStore.ts` | 数据源换运营态端点 + 加 taxonomies/dimensions + `labelFor` | B |
| `frontend/src/features/user-ops/legacy.tsx` | stageLabel + relationship 下拉走 labelFor | B |
| `frontend/src/features/knowledge/trustTypes.ts` | completeness 五维走 dimensions | B |
| `src/routes/guide_profile.rs` | schema 加 suggestedValues + 落候选；schema 加 soulOverride/methodologyOverride/conversationModePolicy 三段生成（typed 维度行业化，§6.5） | C |

## 8. 错误处理

- **流 A 查询失败 fail-soft**：cache 查询失败/无字典时渲染"暂无受控取值"指引，不阻断决策。
- **流 B 端点/降级**：拿不到 active-view 时 `labelFor` 一律 `no_dict`，前端照常显示原始值，不白屏。
- **流 C 生成软化**：取值生成失败不阻断 profile 生成（§6.4）。

## 9. 范围边界（YAGNI）

本设计**只做**取值字典在三个消费侧的单源接线 + AI 生成取值落候选。明确**不做**（留后续独立专题，来自同次审查的其它命门）：

- 承重 profile 字段（chunk_roles / answering_mode / reviewer_orientation 等）的 AI 生成 + 前端编辑器暴露（审查第 2 组）。
- chunk_type 录入侧通路（审查第 3 组）。
- stage 属性三件套（is_terminal / priority_weight / is_reactivation_target）的 admin 通路 + 终态回落修正（审查第 4 组）。
- 批量 approve 取值（逐值复用现有即可）。

## 10. 安全与合规

- 不改"全 AI 自治"红线：取值字典只影响标签翻译与 prompt 指引，不影响决策内容或安全门。
- 守"AI 永不自动 verify"红线：AI 生成取值只进候选层，人审采纳才生效。
- 运营态端点只读、不写，不引入越权写入面。
- 不引入 no-human-takeover 禁用词。
