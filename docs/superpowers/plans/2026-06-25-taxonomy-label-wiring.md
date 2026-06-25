# 取值字典接线（行业化标签 + AI 生成取值）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把单一真相源 `system_taxonomies`（canonical id→display_name）接线到 prompt 侧（AI 决策取值指引）、前端侧（运营可读标签）、AI 生成侧（冷启动生成取值字典 + typed 维度 override），消除取值字典断层这个通用化命门。

**Architecture:** 三条数据流共享 `system_taxonomies`，互不直接耦合。流 A 给 TaxonomyCache 补 display_name 并在决策 prompt 注入 extra 维度合法取值；流 B 新增运营态只读聚合端点 + 前端 labelFor 三情形分流翻译；流 C 让 AI 生成 profile 时连取值字典（落候选层）和三个 prompt override（typed 维度行业化）一起生成。

**Tech Stack:** Rust 2021 / Axum / MongoDB（mongodb driver）/ React 19 + TypeScript + Vite + Zustand / vitest。

设计依据：`docs/superpowers/specs/2026-06-25-taxonomy-label-wiring-design.md`

## Global Constraints

- 单一真相源：所有取值翻译/指引/生成围绕 `system_taxonomies`，不引副本（反 drift）。
- 诚实优先：字典查不到绝不显示错误销售标签；三情形分流（命中 display_name / 野值灰化 / 缺配待配置）。
- AI 永不自动 verify：AI 生成的取值只进 `taxonomy_candidate` 候选层，复用已有 `approve` 人审；profile 仍 `is_active=false` 待人审。
- DEFAULT 销售域字节等价：override=None 时走 prompts.rs 销售兜底，现有行为不回归。
- 测试基线门（不可回归）：`cargo test --lib` ≥350 passed / 0 failed；四 PBT 累计 ≥33/0；前端 vitest 现有 168 测试不回归。
- 本地磁盘受限：只跑 `cargo test --lib` 和单个 PBT；全量集成 `cargo test --test <name> -- --ignored` 留 CI（需 Docker）。前端 `cd frontend && npx vitest run <file>` 单文件可本地跑。
- 字符串 lint 门：禁用词（人工接管/takeover/hand-off 等）。本工程不涉及，但新增文案避开。
- 提交需用户显式批准；精确 `git add` 命名具体文件，排除工作树并行产物。
- 前端遵守现有设计系统：灰化用 `--muted`/`--muted-soft` token，组件走现有模式，不自由发挥。
- camelCase wire 契约：后端 JSON 出参 camelCase，前端 TS 对齐。

---

## 文件结构

| 文件 | 责任 | 流 |
| --- | --- | --- |
| `src/agent/taxonomy.rs` | `CachedEntry` 加 `display_name` + reload 填充 + `dimension_values_with_labels` 查询函数 | A、B |
| `src/agent/domain_profile.rs` | `render_decision_dimensions_guidance` 接 cache，按有无字典渲染 extra 维度取值指引 | A |
| `src/agent/decision.rs` | `render_decision_dimensions_guidance` 调用点传 cache | A |
| `src/routes/operation_view.rs`（新建） | `GET /api/operation/active-view` 运营态聚合端点 | B |
| `src/routes/mod.rs` | 注册新端点 | B |
| `frontend/src/stores/profileStore.ts` | 数据源换运营态端点 + taxonomies/dimensions + `labelFor` | B |
| `frontend/src/features/user-ops/legacy.tsx` | stageLabel + relationship 下拉走 labelFor | B |
| `frontend/src/features/knowledge/trustTypes.ts` | completeness 维度走后端 dimensionList | B |
| `src/routes/guide_profile.rs` | schema 加 suggestedValues 落候选 + 加三个 override 生成 | C |

依赖：流 A Task 1（缓存加字段）是流 B 端点（Task 3）的前置；其余流内 Task 顺序见下。流 A/B/C 之间可并行（共享 system_taxonomies，无代码耦合），但 Task 1 必须最先。

---

## ⚠️ 可行性审查必修修正（实现前必读，覆盖下方 Task）

2026-06-25 一次 4-agent 可行性审查（技术/业务/红线/对抗，全判 FEASIBLE_WITH_FIXES）抓出 5 项必修。实现对应 Task 时，**以本节为准**覆盖下方 Task 原文里被修正的部分：

**M1 [blocker] 流 C label 通路必须先贯通（改 Task 8）。** 实测：`upsert_candidate`（taxonomy.rs:319）真实签名 `(db, scope_account_id, kind, raw_value, evidence: Option<&str>, confidence: i32)` **无 label 参数**；`TaxonomyCandidate`（models.rs:2588）**无 label 字段**；`approve`（taxonomy.rs:456）硬编码 `display_name = candidate.raw_value`（英文 id）。不修则 AI 的中文 label 全程丢失、approve 后字典仍是英文 id，**defeat 工程目标**。Task 8 扩为先贯通 label（6 处）：①`TaxonomyCandidate` 加 `suggested_display_name: Option<String>`（serde default 向后兼容）；②`upsert_candidate` 加 `suggested_display_name: Option<&str>` 参数写入候选；③`approve` 改 `display_name = candidate.suggested_display_name.unwrap_or(raw_value)`；④现有 `upsert_candidate` 调用点（decision_taxonomy.rs 等运行时落候选处）同步补 `None` 实参（行为不变）；⑤流 C 落候选调 `upsert_candidate(db,"global",kind,id,Some(label),None,<confidence>)`；⑥approve 路由可选回 suggested_display_name 供预览（本期可省）。

**M2 [blocker] 鉴权用 AuthenticatedAdmin（改 Task 3）。** 实测：本系统**只有一种鉴权角色 `AuthenticatedAdmin`**（auth/middleware.rs:51 require_session 对 cookie/JWT 都注入它），无独立"运营态非 admin"角色；所有 `/api` 运营 handler（contacts.rs:106/160/200）都用 `Extension<AuthenticatedAdmin>`。计划里的 `AuthenticatedUser` **不存在**（编译失败）。Task 3 端点改用 `Extension<AuthenticatedAdmin>` + `admin.current_workspace`，与现有运营 handler 一致；无越权问题（系统无角色区分）。

**M3 [major] 端点 taxonomies 的 kind 来源（改 Task 3）。** 端点**不能只遍历 `profile.profile_dimensions` 取 kind**——`relationship_type` 不是 profile 维度（DEFAULT profile_dimensions 只有 customer_stage/intent_level），只遍历它会导致 Task 6 的 relationship 下拉空。端点要取的 kind 集 = profile 维度 kind ∪ `["relationship_type"]`（前端要翻译的 AdminDirect 维度），逐个 `dimension_values_with_labels` 建 taxonomies。

**M4 [major] completeness 需后端先加 dimensionList（改 Task 7）。** 实测：completeness 响应（catalog.rs:698 区域 `build_operation_knowledge_completeness`）只回 `coverage` + `answeringModeLabels`，**无 dimensionList 字段**；前端类型有、后端没回。且 completeness 维度来自 `DomainProfile.coverage_dimensions`（知识覆盖维度），与 active-view 的 `profile_dimensions`（画像维度）是**两套不同维度，勿混**。Task 7 扩为后端 + 前端两侧：(a) 后端 `build_operation_knowledge_completeness` 增产 `dimensionList`（key + 中文 label，源自 `active_profile.coverage_dimensions.display_name`）；(b) 前端解析优先读 dimensionList，缺省回落写死 DIM_ORDER。

**M5 [major] profileStore 扩展而非替换（改 Task 4）。** 现有 `activeProfile` / `loadActiveProfile` 有真实调用方（Shell.tsx:136/162 频道 visibleWhen 门控、App.tsx:144），删除会断频道可见性。Task 4 **保留** `activeProfile` + `loadActiveProfile`，**新增** `dimensions`/`taxonomies`/`loadActiveView`/`labelFor`，两者并存。

**minor（实现时注意）：** ①测试 helper 真名是 `make_entry`（taxonomy.rs:636，内部硬编码 display_name=canonical_id，需加 display_name 参数）/`make_cache_with_entries`（:611），非计划写的 `mk_entry`；改 helper 签名要同步所有调用点。②流 C 三个 override（M 之外）的 key 加进 `coerce_scalar_string_fields`（guide_profile.rs:93 SCALAR_STRING_KEYS）防 LLM 给对象/数组。③流 C 逐值串行 await upsert_candidate，几十维度×8 取值可能慢——本期维度少（≤3 维×3-8 值）无碍，`let _` 软化失败不阻断。④taxonomy scope 是 account_id，端点传 workspace 概念错位但 global seed 可达（account-first→global 回落），DEFAULT 取值能翻译。

---

## 流 A：prompt 侧取值指引接线

### Task 1: TaxonomyCache 加 display_name + 带 label 查询函数

**Files:**
- Modify: `src/agent/taxonomy.rs`（`CachedEntry` :79、reload 填充 :143-150、新增查询函数）

**Interfaces:**
- Produces: `pub(crate) fn dimension_values_with_labels(kind: &str, scope_account_id: &str, cache: &TaxonomyCache) -> Vec<(String, String)>` — 返回该 kind 下 status=active 的 `(canonical_id, display_name)` 对，scope 回落（account 私有优先 global），去重。
- Produces: `CachedEntry.display_name: String` 字段。

- [ ] **Step 1: 给 CachedEntry 加字段 + 写失败测试**

Modify `src/agent/taxonomy.rs` `struct CachedEntry`（:79 区域），在 `is_reactivation_target` 后加：

```rust
    /// 取值字典的人类可读名（来自 TaxonomyValue.display_name）。流 A prompt 取值
    /// 指引 + 流 B 前端 labelFor 翻译都用它；早期只缓存 planner 排序字段时被丢弃。
    display_name: String,
```

在 `#[cfg(test)]` 测试模块（文件末尾 tests）加失败测试：

```rust
    #[test]
    fn dimension_values_with_labels_returns_id_label_pairs() {
        let cache = taxonomy_cache_for_tests(vec![
            mk_entry("global", "customer_stage", "first_contact", "初次接触", vec![], "active"),
            mk_entry("global", "customer_stage", "qualified", "已确认意向", vec![], "active"),
            mk_entry("global", "customer_stage", "old_dep", "废弃", vec![], "deprecated"),
        ]);
        let mut got = dimension_values_with_labels("customer_stage", "acct1", &cache);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("first_contact".to_string(), "初次接触".to_string()),
                ("qualified".to_string(), "已确认意向".to_string()),
            ]
        );
    }
```

> 注：`mk_entry` 是现有测试 helper（taxonomy.rs:153 区域）。确认其签名是否已含 display_name 参数——若现有 `mk_entry` 不接 display_name，本步同时扩展它加该参数（其它调用点同步补一个中文串实参）。`taxonomy_cache_for_tests` 见 taxonomy.rs:584。

- [ ] **Step 2: 运行测试验证失败**

Run: `cd /e/yw/agiatme/工作项目/wt-taxonomy-wiring && cargo test --lib dimension_values_with_labels`
Expected: 编译失败（`display_name` 未在 CachedEntry 构造处填充 / 函数不存在）

- [ ] **Step 3: reload 填充 display_name**

Modify reload_from_db（:143-150）的 `CachedEntry { ... }` 构造，加一行：

```rust
                    display_name: entry.value.display_name,
```

> 确认 `TaxonomyValue` 有 `display_name: String` 字段（models.rs:2387 区域）。注意字段移动顺序：`entry.value.id`/`aliases`/`status` 已被前面字段消费 `entry.value` 的部分字段，`display_name` 单独取一次即可（struct 字段是 move，但各取各的字段不冲突）。

- [ ] **Step 4: 实现 dimension_values_with_labels**

在 `dimension_value_weights`（:262）之后加：

```rust
/// 查某 `kind` 下所有 status=active 的 `(canonical_id, display_name)` 对。
/// scope 回落：account 私有 scope 优先，再补 global；按 canonical_id 去重。
/// 流 A prompt 取值指引 + 流 B 前端字典翻译共用。
pub(crate) fn dimension_values_with_labels(
    kind: &str,
    scope_account_id: &str,
    cache: &TaxonomyCache,
) -> Vec<(String, String)> {
    let inner = cache.inner.lock();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for scope in [scope_account_id, "global"] {
        let key = (scope.to_string(), kind.to_string());
        if let Some(entries) = inner.entries.get(&key) {
            for e in entries {
                if e.status == "active" && seen.insert(e.canonical_id.clone()) {
                    out.push((e.canonical_id.clone(), e.display_name.clone()));
                }
            }
        }
    }
    out
}
```

- [ ] **Step 5: 运行测试验证通过 + 基线**

Run: `cargo test --lib taxonomy`
Expected: 新测试 PASS，taxonomy 模块现有测试不回归
Run: `cargo test --lib`
Expected: ≥350 passed / 0 failed

- [ ] **Step 6: 提交**

```bash
git add src/agent/taxonomy.rs
git commit -m "feat(taxonomy): CachedEntry 缓存 display_name + dimension_values_with_labels 查询"
```

---

### Task 2: extra 维度 prompt 取值指引注入

**Files:**
- Modify: `src/agent/domain_profile.rs`（`render_decision_dimensions_guidance` :1182）
- Modify: `src/agent/decision.rs`（调用点）

**Interfaces:**
- Consumes: `dimension_values_with_labels`（Task 1）、`kind_has_entries`（taxonomy.rs:297）、`global_taxonomy_cache`（taxonomy.rs:562）。
- Produces: `render_decision_dimensions_guidance` 新签名 `(dimensions: &[ProfileDimension], scope_account_id: &str, cache: &TaxonomyCache) -> String`。

- [ ] **Step 1: 写失败测试（有字典注入取值 / 无字典提示）**

Modify `src/agent/domain_profile.rs` 测试模块（:2312 区域 G1 测试附近）加：

```rust
    #[test]
    fn dimensions_guidance_injects_dict_values_when_present() {
        use crate::agent::taxonomy::taxonomy_cache_for_tests;
        let cache = taxonomy_cache_for_tests(vec![
            // mk_entry 构造 emotion_state 维度两个 active 取值
            mk_tax_entry("global", "emotion_state", "anxious", "焦虑"),
            mk_tax_entry("global", "emotion_state", "calm", "平静"),
        ]);
        let dims = vec![ProfileDimension {
            kind: "emotion_state".to_string(),
            display_name: "情绪状态".to_string(),
            participates_in_decision: true,
            description: "客户当前情绪".to_string(),
        }];
        let out = render_decision_dimensions_guidance(&dims, "acct1", &cache);
        assert!(out.contains("anxious（焦虑）"), "应注入字典取值: {out}");
        assert!(out.contains("calm（平静）"), "应注入字典取值: {out}");
    }

    #[test]
    fn dimensions_guidance_marks_no_dict_when_empty() {
        use crate::agent::taxonomy::taxonomy_cache_for_tests;
        let cache = taxonomy_cache_for_tests(vec![]);
        let dims = vec![ProfileDimension {
            kind: "vibe".to_string(),
            display_name: "氛围".to_string(),
            participates_in_decision: true,
            description: String::new(),
        }];
        let out = render_decision_dimensions_guidance(&dims, "acct1", &cache);
        assert!(out.contains("暂无受控取值"), "无字典应提示: {out}");
    }
```

> `mk_tax_entry` 若不存在则在本测试模块加一个最小构造 helper（仿 taxonomy.rs 的 `mk_entry`，产 `TaxonomyEntry`）。空维度渲染空串的现有 G1 测试（:2319）签名要同步改成新的三参签名。

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib dimensions_guidance`
Expected: 编译失败（签名不匹配 / 函数还是旧版）

- [ ] **Step 3: 改 render_decision_dimensions_guidance**

Modify `src/agent/domain_profile.rs:1182`，新签名 + 每个 extra 维度按有无字典渲染：

```rust
pub fn render_decision_dimensions_guidance(
    dimensions: &[ProfileDimension],
    scope_account_id: &str,
    cache: &crate::agent::taxonomy::TaxonomyCache,
) -> String {
    let extra: Vec<&ProfileDimension> = dimensions
        .iter()
        .filter(|d| {
            d.participates_in_decision
                && !crate::agent::dimension_registry::typed_dimension_kinds()
                    .contains(&d.kind.as_str())
        })
        .collect();
    if extra.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::with_capacity(extra.len());
    for d in &extra {
        let values = crate::agent::taxonomy::dimension_values_with_labels(
            &d.kind,
            scope_account_id,
            cache,
        );
        let head = if d.description.trim().is_empty() {
            format!("- {}（{}）", d.kind, d.display_name)
        } else {
            format!("- {}（{}）：{}", d.kind, d.display_name, d.description.trim())
        };
        if values.is_empty() {
            lines.push(format!(
                "{head}\n  合法取值：暂无受控取值，请据对话语义判断；新取值会被收集为候选待运营确认。"
            ));
        } else {
            let listed = values
                .iter()
                .map(|(id, label)| format!("{id}（{label}）"))
                .collect::<Vec<_>>()
                .join(" / ");
            lines.push(format!("{head}\n  合法取值：{listed}"));
        }
    }
    format!(
        "\n\n# 本行业参与决策的画像维度（写进 domainSignals 容器）\n\
         除上面 schema 里的字段外，本行业还要在 JSON 顶层输出一个 \"domainSignals\" 对象，\
         为下列每个维度给出当前取值（优先用「合法取值」里的 id，无合适项才新造短词）：\n{}\n\
         示例：\"domainSignals\": {{ {} }}。维度取值无法判断时该键留空或省略，不要臆测。",
        lines.join("\n"),
        extra
            .iter()
            .map(|d| format!("\"{}\": \"...\"", d.kind))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
```

- [ ] **Step 4: 改调用点传 cache**

Modify `src/agent/decision.rs` 调 `render_decision_dimensions_guidance` 处（grep 定位），传入 scope 与 cache：

```rust
    let dimensions_guidance = super::domain_profile::render_decision_dimensions_guidance(
        &active_profile.profile_dimensions,
        &contact.account_id,
        crate::agent::taxonomy::global_taxonomy_cache().as_ref(),
    );
```

> 实现时确认调用点处 `contact.account_id` 字段名（可能是 `account_id`）与 `global_taxonomy_cache()` 返回 `Arc<TaxonomyCache>`（取 `.as_ref()`）。若调用点已有 dimensions_guidance 变量，替换其右值。

- [ ] **Step 5: 运行测试验证通过 + 基线**

Run: `cargo test --lib dimensions_guidance && cargo test --lib`
Expected: 新测试 PASS；lib ≥350/0

- [ ] **Step 6: 提交**

```bash
git add src/agent/domain_profile.rs src/agent/decision.rs
git commit -m "feat(prompt): extra 维度注入字典合法取值指引(无字典明示)"
```

---

## 流 B：前端侧翻译接线

### Task 3: 运营态聚合端点 active-view

**Files:**
- Create: `src/routes/operation_view.rs`
- Modify: `src/routes/mod.rs`（注册路由）

**Interfaces:**
- Consumes: `load_active_domain_profile`（agent::domain_profile，已有）、`global_taxonomy_cache` + `dimension_values_with_labels`（Task 1）。
- Produces: `GET /api/operation/active-view` → `{ dimensions: [{kind, displayName, participatesInDecision}], taxonomies: {kind: [{id, label}]} }`。

- [ ] **Step 1: 建端点文件**

Create `src/routes/operation_view.rs`：

```rust
//! 运营态只读：当前激活 profile 的维度声明 + 各维度 taxonomy 取值字典(id→label)。
//! 区别于 admin-only 的 active_domain_profile（domain_profiles.rs）：本端点走普通
//! require_session 鉴权（运营视图用，不要 admin），且聚合 taxonomy 取值供前端 labelFor。

use axum::{extract::State, Extension, Json};
use serde_json::{json, Value};

use crate::auth::middleware::AuthenticatedUser;
use crate::error::AppResult;
use crate::state::AppState;

pub(super) async fn active_view(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> AppResult<Json<Value>> {
    let profile =
        crate::agent::domain_profile::load_active_domain_profile(&state.db, &user.current_workspace)
            .await;

    let cache = crate::agent::taxonomy::global_taxonomy_cache();
    cache.find_or_load(&state.db).await;

    let dimensions: Vec<Value> = profile
        .profile_dimensions
        .iter()
        .map(|d| {
            json!({
                "kind": d.kind,
                "displayName": d.display_name,
                "participatesInDecision": d.participates_in_decision,
            })
        })
        .collect();

    let mut taxonomies = serde_json::Map::new();
    for d in &profile.profile_dimensions {
        let values = crate::agent::taxonomy::dimension_values_with_labels(
            &d.kind,
            &user.current_workspace,
            cache.as_ref(),
        );
        let arr: Vec<Value> = values
            .into_iter()
            .map(|(id, label)| json!({ "id": id, "label": label }))
            .collect();
        taxonomies.insert(d.kind.clone(), Value::Array(arr));
    }

    Ok(Json(json!({
        "dimensions": dimensions,
        "taxonomies": Value::Object(taxonomies),
    })))
}
```

> 实现时核对：(1) 运营态鉴权 Extension 的真实类型名（`AuthenticatedUser` 还是别的——grep `src/routes/contacts.rs` 现有运营态 handler 的 Extension 类型）；(2) `current_workspace` 字段名；(3) `dimension_values_with_labels` 第二参用 workspace 还是 account scope——与 Task 1 scope 语义对齐（taxonomy scope 是 account_id；workspace 级可能需传 DEFAULT account 或调整。实现时确认 taxonomy 的 scope 维度，必要时端点按 workspace 的默认 account 取 scope）。

- [ ] **Step 2: 注册路由 + 模块**

Modify `src/routes/mod.rs`：模块声明区加 `mod operation_view;`；在 `/api` 普通鉴权路由组（参照 `/operation-domains` mod.rs:712 的注册位置，**非 admin 组**）加：

```rust
        .route("/operation/active-view", get(operation_view::active_view))
```

- [ ] **Step 3: 编译验证**

Run: `cargo check --lib`
Expected: 0 error（鉴权类型 / 字段名核对无误）

- [ ] **Step 4: 写集成测试**

Modify（或新建）`tests/operation_view_integration.rs`，参照现有 routes 集成测试构造方式，标 `#[ignore]`（需 Docker）：

```rust
/// active-view 端点：种一个 active profile（含 2 维度）+ taxonomy 取值，
/// 拉端点，断言 dimensions 含两维度、taxonomies[kind] 含 (id,label) 对。
#[tokio::test]
#[ignore = "requires docker (testcontainers mongo)"]
async fn active_view_returns_dimensions_and_taxonomies() {
    // Arrange: 起 Mongo，种 active DomainProfile（profile_dimensions=[customer_stage, emotion_state]）
    //          + system_taxonomies（customer_stage:first_contact→初次接触）。
    // Act: GET /api/operation/active-view（带 session）。
    // Assert: body.dimensions 长度 2；body.taxonomies.customer_stage[0] == {id:"first_contact", label:"初次接触"}。
}
```

> Arrange/Act/Assert 注释由实现者按 tests/ 现有 helper（起 Mongo、构造 AppState、种 profile/taxonomy、带 session 发请求）补成真实代码。

- [ ] **Step 5: 编译测试（本地只编译）**

Run: `cargo test --test operation_view_integration --no-run`
Expected: 编译成功（`#[ignore]` 本地不实跑，CI 跑 `-- --ignored`）

- [ ] **Step 6: 提交**

```bash
git add src/routes/operation_view.rs src/routes/mod.rs tests/operation_view_integration.rs
git commit -m "feat(routes): 运营态 active-view 端点(profile维度+taxonomy取值字典)"
```

---

### Task 4: 前端 profileStore 扩展 + labelFor 三情形分流

**Files:**
- Modify: `frontend/src/stores/profileStore.ts`
- Test: `frontend/src/stores/__tests__/profileStore.test.ts`（新建）

**Interfaces:**
- Consumes: `GET /api/operation/active-view`（Task 3）。
- Produces: `useProfileStore` 增 `dimensions` / `taxonomies` state；导出纯函数 `labelFor(taxonomies, kind, value): LabelResult`，`LabelResult = { text: string; status: 'ok' | 'unknown_value' | 'no_dict' }`。

- [ ] **Step 1: 写 labelFor 纯函数失败测试**

Create `frontend/src/stores/__tests__/profileStore.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { labelFor, type TaxonomyMap } from "../profileStore";

const tax: TaxonomyMap = {
  customer_stage: [
    { id: "first_contact", label: "初次接触" },
    { id: "qualified", label: "已确认意向" },
  ],
};

describe("labelFor 三情形分流", () => {
  it("命中字典 → display_name, status ok", () => {
    expect(labelFor(tax, "customer_stage", "first_contact")).toEqual({
      text: "初次接触",
      status: "ok",
    });
  });
  it("有字典但值不在内 → 原值, status unknown_value", () => {
    expect(labelFor(tax, "customer_stage", "weird_value")).toEqual({
      text: "weird_value",
      status: "unknown_value",
    });
  });
  it("维度无字典 → 原值, status no_dict", () => {
    expect(labelFor(tax, "emotion_state", "anxious")).toEqual({
      text: "anxious",
      status: "no_dict",
    });
  });
});
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cd frontend && npx vitest run src/stores/__tests__/profileStore.test.ts`
Expected: FAIL（`labelFor` / `TaxonomyMap` 未导出）

- [ ] **Step 3: 实现 labelFor + 扩展 store**

Modify `frontend/src/stores/profileStore.ts`：

```ts
import { create } from "zustand";
import { api } from "../lib/api";
import type { DomainProfile } from "../types";

export interface TaxonomyValueLite {
  id: string;
  label: string;
}
export type TaxonomyMap = Record<string, TaxonomyValueLite[]>;

export interface ProfileDimensionView {
  kind: string;
  displayName: string;
  participatesInDecision: boolean;
}

export type LabelStatus = "ok" | "unknown_value" | "no_dict";
export interface LabelResult {
  text: string;
  status: LabelStatus;
}

/// 三情形分流：命中→display_name；有字典无此值→原值+unknown_value；维度无字典→原值+no_dict。
export function labelFor(
  taxonomies: TaxonomyMap,
  kind: string,
  value: string
): LabelResult {
  const entries = taxonomies[kind];
  if (!entries || entries.length === 0) {
    return { text: value, status: "no_dict" };
  }
  const hit = entries.find((e) => e.id === value);
  if (!hit) {
    return { text: value, status: "unknown_value" };
  }
  return { text: hit.label, status: "ok" };
}

interface ProfileState {
  activeProfile: DomainProfile | null;
  dimensions: ProfileDimensionView[];
  taxonomies: TaxonomyMap;
  loading: boolean;
  error: string | null;
  loadActiveView: () => Promise<void>;
}

export const useProfileStore = create<ProfileState>((set) => ({
  activeProfile: null,
  dimensions: [],
  taxonomies: {},
  loading: false,
  error: null,
  loadActiveView: async () => {
    set({ loading: true, error: null });
    try {
      const data = await api.get<{
        dimensions: ProfileDimensionView[];
        taxonomies: TaxonomyMap;
      }>("/api/operation/active-view");
      set({
        dimensions: data.dimensions ?? [],
        taxonomies: data.taxonomies ?? {},
        loading: false,
      });
    } catch (err) {
      // 降级：拿不到 active-view 时前端照常跑，labelFor 一律回落 no_dict（显示原值）。
      set({
        dimensions: [],
        taxonomies: {},
        loading: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  },
}));
```

> 注：旧 `loadActiveProfile` 调 `/api/admin/domain-profiles/active`。检查它的现有调用方（grep `loadActiveProfile`）——若有调用方，保留旧方法或迁移到 `loadActiveView`。本步若移除旧方法，须同步改调用点。

- [ ] **Step 4: 运行测试验证通过**

Run: `cd frontend && npx vitest run src/stores/__tests__/profileStore.test.ts`
Expected: 3 passed

- [ ] **Step 5: 提交**

```bash
git add frontend/src/stores/profileStore.ts frontend/src/stores/__tests__/profileStore.test.ts
git commit -m "feat(fe): profileStore active-view + labelFor 三情形分流"
```

---

### Task 5: stageLabel 走字典翻译

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（stageLabel :1997-2021）

**Interfaces:**
- Consumes: `useProfileStore` / `labelFor`（Task 4）。

- [ ] **Step 1: 接入 store + labelFor**

Modify `frontend/src/features/user-ops/legacy.tsx`，在 `PlannerViewSection`（:2006 区域）组件体内取 taxonomies，并改 stageLabel：

```tsx
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const stageLabelResult = (() => {
    const attrs = contact.domainAttributes;
    if (!attrs || typeof attrs !== "object") return null;
    const stage = (attrs as Record<string, unknown>).stage;
    if (typeof stage !== "string" || !stage) return null;
    return labelFor(taxonomies, "customer_stage", stage);
  })();
```

渲染处（:2021 `运营阶段 <strong>{stageLabel || "未分层"}</strong>`）改为：

```tsx
          运营阶段{" "}
          {stageLabelResult ? (
            <strong
              className={stageLabelResult.status === "ok" ? undefined : styles.mutedLabel}
              title={
                stageLabelResult.status === "no_dict"
                  ? "该维度暂无取值字典，显示原始值（待配置）"
                  : stageLabelResult.status === "unknown_value"
                    ? "未知取值（不在当前字典内）"
                    : undefined
              }
            >
              {stageLabelResult.text}
            </strong>
          ) : (
            <strong>未分层</strong>
          )}
```

> `styles.mutedLabel` 用现有 CSS module 的灰化类（grep legacy 对应 module.css 是否有 muted 类，没有则加一个用 `var(--muted)` 的类，遵守现有设计系统）。若该文件用全局 className 非 module，按现有惯例加类。

- [ ] **Step 2: 确保挂载时加载 active-view**

确认 user-ops 视图挂载时调了 `loadActiveView`（在 user-ops 顶层组件的 useEffect 加 `useProfileStore.getState().loadActiveView()`，若尚无）。grep `loadActiveView` 调用方确认只加载一次。

- [ ] **Step 3: 前端构建 + 现有测试不回归**

Run: `cd frontend && npx vitest run` 然后 `npm run build`
Expected: vitest 不回归（168 基线）；build 成功

- [ ] **Step 4: 提交**

```bash
git add frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(fe): stageLabel 走字典翻译(三情形灰化)"
```

---

### Task 6: relationship 下拉走字典

**Files:**
- Modify: `frontend/src/features/user-ops/legacy.tsx`（relationship 下拉 :430 区域）

**Interfaces:**
- Consumes: `useProfileStore` taxonomies（Task 4）。

- [ ] **Step 1: 下拉选项来源改字典**

Modify `legacy.tsx:430` 区域的 relationship_type 下拉（`value={relationshipType}`）。先读出该 `<select>` 现有写死的 `<option>` 列表，改为从 `taxonomies.relationship_type` 渲染：

```tsx
  const taxonomies = useProfileStore((s) => s.taxonomies);
  const relationshipOptions = taxonomies.relationship_type ?? [];
```

`<select>` 内：

```tsx
              {relationshipOptions.length > 0 ? (
                relationshipOptions.map((opt) => (
                  <option key={opt.id} value={opt.id}>
                    {opt.label}
                  </option>
                ))
              ) : (
                // 字典未配时回落：至少给空选项，避免下拉空白不可选
                <option value="">（未配置关系类型字典）</option>
              )}
```

> 读出现有写死 option（customer/peer/friend）确认 value 用 canonical id。若该组件是受控且 relationshipType 来自上层 props，保持 value 绑定不变，只改 option 来源。

- [ ] **Step 2: 前端构建 + 测试不回归**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 不回归；build 成功

- [ ] **Step 3: 提交**

```bash
git add frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(fe): relationship 下拉选项走 active profile 字典"
```

---

### Task 7: completeness 维度走后端 dimensionList

**Files:**
- Modify: `frontend/src/features/knowledge/trustTypes.ts`（DimKey/DIM_ORDER/coverage :29-51）

**Interfaces:**
- Consumes: 后端 `CompletenessView.dimensionList: CoverageDimension[]`（trustTypes.ts:42，后端已动态回）。

- [ ] **Step 1: 写失败测试（解析层用 dimensionList）**

Read trustTypes.ts 的解析函数（`:71` 区域 `coverage` 解析），确认现状用写死 `DIM_ORDER` 把 coverage Record 映射成列表。新建/扩展测试 `frontend/src/features/knowledge/__tests__/trustTypes.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { parseCompleteness } from "../trustTypes";

describe("completeness 维度动态化", () => {
  it("dimensionList 来自后端而非写死销售五维", () => {
    const raw = {
      dimensionList: [
        { key: "rapport", label: "亲密度", verifiedFact: true, methodologyOnly: false, pendingDraft: false, state: "verified" },
      ],
      coverage: {},
      answeringMode: "relationship_only",
    };
    const view = parseCompleteness(raw);
    expect(view.dimensionList.map((d) => d.label)).toEqual(["亲密度"]);
  });
});
```

> 确认解析函数真实导出名（可能不是 `parseCompleteness`，grep trustTypes.ts 的 `export function`）。

- [ ] **Step 2: 运行验证失败**

Run: `cd frontend && npx vitest run src/features/knowledge/__tests__/trustTypes.test.ts`
Expected: FAIL（现状用写死 DIM_ORDER，非销售维度拿不到）

- [ ] **Step 3: 解析改用 dimensionList**

Modify trustTypes.ts 解析逻辑：优先用后端 `dimensionList`，仅当后端没回时回落写死 `DIM_ORDER`（销售域兜底）。保留 `DEFAULT_ANSWERING_MODE_LABELS` 回落范式（trustTypes.ts:11）。具体改 `:71` 区域的 coverage→列表映射，改为 `raw.dimensionList ?? (DIM_ORDER 回落构造)`。

- [ ] **Step 4: 验证通过 + 全量不回归**

Run: `cd frontend && npx vitest run && npm run build`
Expected: 新测试 PASS；168 基线不回归；build 成功

- [ ] **Step 5: 提交**

```bash
git add frontend/src/features/knowledge/trustTypes.ts frontend/src/features/knowledge/__tests__/trustTypes.test.ts
git commit -m "feat(fe): completeness 维度走后端 dimensionList(去销售五维写死)"
```

---

## 流 C：AI 生成取值字典 + override

### Task 8: generate schema 加 suggestedValues + 落候选

**Files:**
- Modify: `src/routes/guide_profile.rs`（prompt schema :195-243、生成流程末尾落候选、解析结构）

**Interfaces:**
- Consumes: `upsert_candidate`（taxonomy.rs:319）。
- Produces: 生成的每维度 `suggestedValues: [{id, label}]` 落 `taxonomy_candidate`。

- [ ] **Step 1: prompt schema 加 suggestedValues**

Modify `src/routes/guide_profile.rs:199` 的 `profileDimensions` schema，每个维度对象加：

```
      "suggestedValues": [
        {{"id": "取值英文id(snake_case)", "label": "中文取值名"}}
      ]
```

并在 :237 区域「重要提醒」加一条：

```
- 每个维度尽量给 3-8 个该行业典型取值（suggestedValues）；`id` 用 snake_case 英文、`label` 用中文行业术语。这些取值是「建议候选」，运营审核后才生效，不必追求穷尽。
```

- [ ] **Step 2: 解析结构加字段**

定位 guide_profile.rs 里解析 AI 输出的 profile_dimensions 结构（normalize 后的 doc → ProfileDimension）。`suggestedValues` 不是 ProfileDimension 的字段（ProfileDimension 只有四字段），需在解析时**单独提取** suggestedValues（在 normalize_json_keys 之前或从 raw JSON 取 `profileDimensions[].suggestedValues`），收集成 `Vec<(kind, Vec<(id, label)>)>` 供 Step 3 落候选。

> 实现细节：参照 stateMachine 的处理（guide_profile.rs:300 区域在 normalize 前 `remove("stateMachine")` 保留 camelCase）——suggestedValues 同理在 normalize 前从每个 dim 提取出来，避免污染 ProfileDimension 反序列化（ProfileDimension 无此字段，serde 默认会忽略未知字段，但显式提取更稳）。

- [ ] **Step 3: 落候选**

在 profile 候选落库后（`generate_domain_profile_candidate` 末尾、return 前），遍历提取的 suggestedValues 调 `upsert_candidate`：

```rust
    // AI 建议的取值落候选层（绝不直接进 system_taxonomies）——复用运行时同一候选→approve 通路。
    for (kind, values) in &suggested_values {
        for (id, label) in values {
            let _ = crate::agent::taxonomy::upsert_candidate(
                &state.db,
                "global",        // scope：实现时与 taxonomy scope 语义对齐
                kind,
                id,
                label,
            )
            .await; // 失败软化：候选落库失败不阻断 profile 生成
        }
    }
```

> 核对 `upsert_candidate` 真实签名（taxonomy.rs:319）——参数顺序/类型以实际为准，本片段按 (db, scope, kind, id, label) 假定，实现时对齐。失败用 `let _ =` 软化（§6.4）。

- [ ] **Step 4: 编译验证**

Run: `cargo check --lib`
Expected: 0 error

- [ ] **Step 5: 软化单测**

加单测：生成输出缺 suggestedValues 时，提取得空 vec、profile 仍正常落库（仿 guide_profile 现有解析测试 :460 区域，纯解析层断言不 panic、profile_dimensions 仍在）。

Run: `cargo test --lib guide_profile`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add src/routes/guide_profile.rs
git commit -m "feat(guide): AI 生成维度取值 suggestedValues 落候选层(复用 approve)"
```

---

### Task 9: generate schema 加三个 override（typed 维度行业化）

**Files:**
- Modify: `src/routes/guide_profile.rs`（prompt schema + 解析）

**Interfaces:**
- Produces: 生成输出含 `soulOverride` / `methodologyOverride` / `conversationModePolicy` → 落 DomainProfile 同名字段（已有 override 整段替换机制消费）。

- [ ] **Step 1: prompt schema 加三段 override**

Modify guide_profile.rs prompt schema（:195 区域），在 `promptFragment` 附近加：

```
  "soulOverride": "本行业的 AI 人格本体——它是谁、面对客户的根本姿态。会整体替换销售域默认人格。非销售行业必填，纯销售可留空。",
  "methodologyOverride": "本行业的运营方法论——客户会经历哪些阶段、每阶段怎么推进。会整体替换销售域默认方法论。",
  "conversationModePolicy": "本行业的对话模式判定规则——什么情况进入哪种对话模式（对应 conversationModes）。会整体替换销售域默认判定段。",
```

并在「重要提醒」加：

```
- soulOverride / methodologyOverride / conversationModePolicy 是把本行业世界观「整段」写清楚——客户阶段的取值语义、推进规则都写在这里（不要用销售词如「成交/逼单/续费」，除非你就是销售行业）。这三段决定了 customer_stage 等维度对本行业的真实含义。留空则回落销售域默认。
```

- [ ] **Step 2: 解析落字段**

确认 DomainProfile 反序列化已认这三个字段（models.rs：`soul_override` / `methodology_override` / `conversation_mode_policy` 都是 `Option<String>`，camelCase wire 经 normalize_json_keys → snake_case）。这三个是标量 String，normalize_json_keys 的 camelCase→snake_case 直接覆盖，无需特殊提取（不同于 suggestedValues/stateMachine 的嵌套结构）。确认 `coerce_scalar_string_fields`（guide_profile.rs:93）已把它们纳入「LLM 偶发给对象/数组→压平」的保护（若未纳入，把这三个 key 加进该函数的标量字段清单）。

- [ ] **Step 3: 解析单测**

加单测：含三个 override 的生成输出 → 落 profile 对应字段为 Some；缺失 → None（仿 :460 解析测试）。

```rust
    #[test]
    fn generate_parses_overrides_when_present() {
        // 构造含 soulOverride/methodologyOverride/conversationModePolicy 的 raw JSON,
        // 走 normalize + from_document, 断言三字段为 Some 且内容正确。
    }

    #[test]
    fn generate_overrides_absent_fall_back_to_none() {
        // 不含三字段的 raw JSON, 断言三字段为 None(回落销售兜底)。
    }
```

- [ ] **Step 4: 验证 + 基线**

Run: `cargo test --lib guide_profile && cargo test --lib`
Expected: PASS；lib ≥350/0

- [ ] **Step 5: 提交**

```bash
git add src/routes/guide_profile.rs
git commit -m "feat(guide): AI 生成 soul/methodology/conversationMode override(typed维度行业化)"
```

---

## Self-Review

**Spec coverage（逐节对照设计文档）：**
- §4 流 A extra 维度注入 → Task 1（缓存+查询）+ Task 2（渲染+调用点）✅
- §4.1 缓存加 display_name → Task 1 ✅
- §5.1 运营态聚合端点 → Task 3 ✅
- §5.2 profileStore 扩展 → Task 4 ✅
- §5.3 labelFor 三情形 → Task 4 ✅
- §5.4 三渲染点 → Task 5（stage）+ Task 6（relationship）+ Task 7（completeness）✅
- §6.1-6.4 suggestedValues 落候选 → Task 8 ✅
- §6.5 typed 维度 override 生成 → Task 9 ✅
- §6.6 测试 → 各 Task 测试步骤 ✅

**Placeholder scan：** 集成测试体（Task 3 Step 4）是 Arrange/Act/Assert 骨架注释（依赖 testcontainers 上下文的合理留白，已标注实现者按现有 helper 补）；其余生产代码均完整可抄。Task 5/6/7 含「实现时核对/grep 确认」的指引——这些是真实代码坐标在 main 基线可能微移的核对点，非占位（代码主体已给）。

**Type consistency：**
- `dimension_values_with_labels(kind, scope_account_id, cache) -> Vec<(String, String)>`：Task 1 定义、Task 2/Task 3 消费一致。
- `render_decision_dimensions_guidance(dimensions, scope_account_id, cache)`：Task 2 定义、调用点一致。
- `labelFor(taxonomies, kind, value) -> LabelResult`：Task 4 定义、Task 5/6 消费一致；`LabelResult.status` 三态 `ok|unknown_value|no_dict` 一致。
- `TaxonomyMap = Record<string, TaxonomyValueLite[]>`：Task 4 定义、Task 5/6 消费一致。
- 端点返回 `{dimensions, taxonomies}`：Task 3 produces、Task 4 consumes 形状一致。

**待实现者核对的真实坐标（main 基线可能微移）：** 运营态鉴权 Extension 类型名、`upsert_candidate` 签名、taxonomy scope 语义（account vs workspace）、`coerce_scalar_string_fields` 是否已含三 override key、前端 CSS module 灰化类。这些都在对应 Task 标注。

---

## Execution Handoff

计划保存于 `docs/superpowers/plans/2026-06-25-taxonomy-label-wiring.md`。
