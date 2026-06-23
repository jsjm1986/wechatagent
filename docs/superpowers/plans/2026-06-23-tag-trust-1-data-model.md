# 标签可信度改造 · 子计划 1：数据模型 + 三层字段 + manual_tags 录入 + 顺手修 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 `Contact` 落地"人工权威层 / AI 确信层 / 贝叶斯评估层 / 人格层"四类新字段与结构体，新增 `manual_tags` 运营录入端点，废弃裸 `tags` 字段并把下游 4 个 prompt 注入点改读新字段，同时修两个同源隐患（人工备注连带 AI 重生成、MCP 校验旁路）。

**Architecture:** 纯加法为主——新增 BSON-serde 结构体到 `models.rs`，新增字段到 `Contact`，加一条 migration 给存量文档补默认值（虽设计称无存量，仍按项目习惯补 migration 保证索引/反序列化一致）。`manual_tags` 只走 admin 端点写，AI 写路径在代码层面不引用它。下游 prompt 注入改读 `manual_tags + confirmed_tags` 并标注来源。

**Tech Stack:** Rust 2021 / Axum / MongoDB (bson + mongodb crate) / serde。后端无 workspace、单 crate。

## Global Constraints

- `cargo test --lib` ≥ 350 passed / 0 failed；四 PBT（`state_transition_pbt` / `memory_card_invariants` / `wiki_chunk_revision_pbt` / `llm_retry_jitter`）累计 ≥ 33 / 0 不回归。
- 新增字段一律 `#[serde(default)]`，保证旧 BSON 文档反序列化不破。
- 本地只跑 `cargo test --lib` 与单个 PBT 文件（`cargo test --test <name>`）；完整集成测试（`tests/` 下 `#[ignore]`）留 GitHub CI，磁盘受限禁止本地全量编译。
- 编译校验用 `cargo check --tests`（如磁盘紧张去掉 `--tests`）。
- **agent-first**：不引入关键词词表判断语义。
- **no-human-takeover**：新增字段名/注释/UI 文案避开禁用词 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`（CI lint 扫 `src/agent`/`src/routes`/`src/evolution`/`frontend/src` 新增行，tests 目录豁免）。人工标签层用 `manual_tags`（manual 不在禁用词内），注释用"运营录入/运营确认/operator-authored"，**不要**写"人工标签"。
- **既成事实纪律**：MCP/业务动作成功后 DB/审计写失败只 `tracing::warn!`，不返 Err。
- 所有新查询带 `workspace_id` scope（anti-IDOR）。
- 提交需用户显式批准；精确 `git add` 指定文件，排除并行产物（`agent_t*.txt`、`.kiro/specs/universal-test-coverage/*.json` 等）。

## 设计来源

`docs/superpowers/specs/2026-06-23-tag-trust-two-layer-design.md` —— 三层数据模型、人类权威层清单、顺手修两项。

## 现状核实（已亲读，写计划时的事实基线）

- `Contact` struct：`src/models.rs:145-219`。`tags: Vec<String>`（:159-160）、`agent_profile`（:155）、`memory_summary`（:156）、`domain_attributes: Option<Document>`（:163-164）、`profile_attributes: Document`（:185-186）、`profile_updated_at`（:187）、`outcome_events`（:209-210）、`custom_agent_instructions`（:146-147）。
- 逐轮 tags 写入：`src/agent/gateway.rs:3184-3193`，`merge_tags_union_capped`（:3113）union+cap16（`TAGS_PER_MESSAGE_CAP`:3098）。
- 人工写 tags 端点：
  - `update_operation_profile`（`src/routes/contacts.rs:649`）：`set_doc` 含 `"tags": payload.tags`（:686）整体覆盖。
  - `update_profile_note`（`contacts.rs:483`）：写 `"tags": generated.tags`（:508）——**AI 连带重生成**，顺手修目标。
  - `analyze_contact_profile`（`contacts.rs:786` 附近）：AI 重生成 tags。
- MCP 校验旁路：`management.rs:902` 附近 `update_contact_profile` 写 tags/stage/intent 未过 `validate_dimension_value`。
- prompt 注入读 tags 的 4 点：`src/agent/decision.rs:711`、`src/routes/shared.rs:900`、`src/prompts.rs:799`、`src/agent/memory.rs:216`。
- 维度校验：`src/agent/dimension_registry.rs::validate_dimension_value` + `WriteIntent::{AdminWrite,MachineWrite}`。
- migration 范本：`src/db/migrations/` 下各 `mNNN_*.rs`，用 `run_step` 注册（见 `project_digital_twin_relationship_closure` 经验：用 run_step 非 run）。

---

## Task 1：新增证据与确信标签结构体（models.rs）

**Files:**
- Modify: `src/models.rs`（在 Contact struct 定义附近，约 :219 之后的结构体区追加）
- Test: `src/models.rs`（`#[cfg(test)] mod tests` 内追加，文件内单测）

**Interfaces:**
- Produces:
  - `pub struct Evidence { pub turn: i32, pub msg_id: String }`
  - `pub struct ConfirmedTag { pub value: String, pub evidences: Vec<Evidence>, pub confirmed_at: DateTime, pub confirmed_by: String }`
  - 两者派生 `Debug, Clone, Serialize, Deserialize, PartialEq`。

- [ ] **Step 1: 写失败测试**

在 `src/models.rs` 的测试 mod 内追加（确认文件已有 `#[cfg(test)] mod tests`，没有则新建一个）：

```rust
#[test]
fn evidence_and_confirmed_tag_roundtrip_bson() {
    use bson::DateTime;
    let tag = ConfirmedTag {
        value: "价格敏感".to_string(),
        evidences: vec![Evidence { turn: 47, msg_id: "m_abc".to_string() }],
        confirmed_at: DateTime::from_millis(0),
        confirmed_by: "consolidation".to_string(),
    };
    let doc = bson::to_document(&tag).expect("serialize");
    let back: ConfirmedTag = bson::from_document(doc).expect("deserialize");
    assert_eq!(back, tag);
}

#[test]
fn confirmed_tag_missing_evidences_defaults_empty() {
    // 缺 evidences 字段的旧文档（理论上无存量，仍验证向后兼容）反序列化为空 Vec。
    let doc = bson::doc! { "value": "x", "confirmedAt": bson::DateTime::from_millis(0), "confirmedBy": "consolidation" };
    let back: ConfirmedTag = bson::from_document(doc).expect("deserialize");
    assert!(back.evidences.is_empty());
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib confirmed_tag`
Expected: 编译失败 —— `cannot find type ConfirmedTag` / `Evidence`。

- [ ] **Step 3: 写最小实现**

在 `src/models.rs` Contact struct 之后追加（serde 字段名遵循项目惯例：检查 Contact 是否用 `#[serde(rename_all = "camelCase")]` 或逐字段 rename。若项目惯例是 camelCase wire 形态，对结构体加 `#[serde(rename_all = "camelCase")]`）：

```rust
/// 标签可信度改造：单条证据，存对话引用（不拷贝原文），对齐 D2 source_anchors 哲学。
/// turn = 该 contact 会话内的轮次序号；msg_id = conversation_messages 的消息 id。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub turn: i32,
    pub msg_id: String,
}

/// AI 确信层标签：压缩归并时整体重判（replace）写回，每条必带证据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedTag {
    pub value: String,
    #[serde(default)]
    pub evidences: Vec<Evidence>,
    pub confirmed_at: DateTime,
    /// "consolidation"（压缩重判）| "strong_evidence"（强证据快通道）
    pub confirmed_by: String,
}
```

> 注意 import：`models.rs` 顶部应已有 `use bson::DateTime;` 或 `use mongodb::bson::...`。沿用文件现有 import 风格，不要新引第二套路径。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib confirmed_tag evidence`
Expected: 2 passed。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs
git commit -m "feat(tag-trust): add Evidence + ConfirmedTag structs (子计划1 Task1)"
```

---

## Task 2：新增贝叶斯与人格结构体（models.rs）

**Files:**
- Modify: `src/models.rs`（紧接 Task 1 的结构体之后）
- Test: `src/models.rs` 测试 mod

**Interfaces:**
- Produces:
  - `pub struct BayesianPoint { turn: i32, value: String, confidence: f64, value_changed: bool, confidence_changed: bool, reason: Option<String> }`
  - `pub struct BayesianSignal { dimension: String, current_value: String, current_confidence: f64, locked: bool, history: Vec<BayesianPoint> }`
  - `pub struct PersonalityFacet { score: f64, confidence: f64, evidence_refs: Vec<Evidence> }`
  - `pub struct PersonalitySnapshot { consolidated_at: DateTime, scores: Vec<f64>, confidences: Vec<f64> }`
  - `pub struct PersonalityProfile { openness, conscientiousness, extraversion, agreeableness, neuroticism: PersonalityFacet, updated_at: DateTime, snapshots: Vec<PersonalitySnapshot> }`
  - 全部派生 `Debug, Clone, Serialize, Deserialize, PartialEq`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn bayesian_signal_roundtrip_and_history_default() {
    let sig = BayesianSignal {
        dimension: "价格敏感度".to_string(),
        current_value: "高".to_string(),
        current_confidence: 0.7,
        locked: true,
        history: vec![BayesianPoint {
            turn: 3, value: "高".to_string(), confidence: 0.7,
            value_changed: false, confidence_changed: true, reason: None,
        }],
    };
    let doc = bson::to_document(&sig).expect("ser");
    let back: BayesianSignal = bson::from_document(doc).expect("de");
    assert_eq!(back, sig);
}

#[test]
fn personality_profile_roundtrip() {
    let facet = || PersonalityFacet { score: 0.5, confidence: 0.3, evidence_refs: vec![] };
    let p = PersonalityProfile {
        openness: facet(), conscientiousness: facet(), extraversion: facet(),
        agreeableness: facet(), neuroticism: facet(),
        updated_at: bson::DateTime::from_millis(0),
        snapshots: vec![],
    };
    let doc = bson::to_document(&p).expect("ser");
    let back: PersonalityProfile = bson::from_document(doc).expect("de");
    assert_eq!(back, p);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib bayesian_signal_roundtrip personality_profile_roundtrip`
Expected: 编译失败 —— 类型未定义。

- [ ] **Step 3: 写最小实现**

```rust
/// 贝叶斯评估旁路：单轮观测点（append-only ledger），供置信度走势图。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BayesianPoint {
    pub turn: i32,
    pub value: String,
    pub confidence: f64,
    pub value_changed: bool,
    pub confidence_changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 贝叶斯评估旁路：一个被追踪的维度槽（最多 6 个，见 budget 约束）。
/// locked=false 为暂定观察、未正式占槽；locked=true 才画走势线。永不驱动行为。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BayesianSignal {
    pub dimension: String,
    pub current_value: String,
    pub current_confidence: f64,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub history: Vec<BayesianPoint>,
}

/// 大五人格单维度：分值 + 证据充分度 + 支撑引用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityFacet {
    pub score: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence_refs: Vec<Evidence>,
}

/// 人格演化快照：每次压缩归并存一份（粒度=压缩周期，非逐轮）。
/// scores/confidences 顺序固定 [O, C, E, A, N]。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalitySnapshot {
    pub consolidated_at: DateTime,
    pub scores: Vec<f64>,
    pub confidences: Vec<f64>,
}

/// 大五 OCEAN 人格画像：只在压缩归并时更新（慢变量），不进逐轮、不驱动行为。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonalityProfile {
    pub openness: PersonalityFacet,
    pub conscientiousness: PersonalityFacet,
    pub extraversion: PersonalityFacet,
    pub agreeableness: PersonalityFacet,
    pub neuroticism: PersonalityFacet,
    pub updated_at: DateTime,
    #[serde(default)]
    pub snapshots: Vec<PersonalitySnapshot>,
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib bayesian_signal_roundtrip personality_profile_roundtrip`
Expected: 2 passed。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs
git commit -m "feat(tag-trust): add BayesianSignal/Point + PersonalityProfile structs (子计划1 Task2)"
```

---

## Task 3：Contact 新增四个字段

**Files:**
- Modify: `src/models.rs:159-160`（`tags` 字段附近，追加新字段；本任务**不删** `tags`，废弃在 Task 6）
- Test: `src/models.rs` 测试 mod

**Interfaces:**
- Consumes: Task 1/2 的 `ConfirmedTag` / `BayesianSignal` / `PersonalityProfile`。
- Produces: `Contact` 新增 `manual_tags: Vec<String>`、`manual_tags_updated_at: Option<DateTime>`、`manual_tags_by: Option<String>`、`confirmed_tags: Vec<ConfirmedTag>`、`bayesian_signals: Vec<BayesianSignal>`、`personality_profile: Option<PersonalityProfile>`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn contact_new_trust_fields_default_when_absent() {
    // 不含新字段的最小 Contact BSON 应反序列化成功，新字段取默认值。
    // 用已有 Contact 构造 helper 或最小 doc——若 models 有 test helper（如 sample_contact），复用之。
    let c = super::tests_support_minimal_contact(); // 见 Step 3 说明：复用现有 helper 或内联
    assert!(c.manual_tags.is_empty());
    assert!(c.confirmed_tags.is_empty());
    assert!(c.bayesian_signals.is_empty());
    assert!(c.personality_profile.is_none());
    assert!(c.manual_tags_updated_at.is_none());
}
```

> 实现者注意：先 grep `fn .*minimal.*contact|sample_contact|make_contact` 找现有 Contact 构造 helper；有则复用并补新字段，无则用 `bson::from_document` 构造最小文档测默认值。本步的断言目标是"新字段缺省安全"。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib contact_new_trust_fields_default`
Expected: 编译失败 —— Contact 无 `manual_tags` 等字段。

- [ ] **Step 3: 写最小实现**

在 `src/models.rs` `tags` 字段（:160）之后追加：

```rust
    /// 标签可信度改造 · 人工权威层：运营录入的标签，自由文本，AI 写路径不触达。
    /// 与 AI 产出的 confirmed_tags 物理分家，压缩重判永不覆盖本字段。
    #[serde(default)]
    pub manual_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_tags_updated_at: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_tags_by: Option<String>,
    /// AI 确信层：压缩归并整体重判写回，每条带证据。取代裸 tags 的 AI 部分。
    #[serde(default)]
    pub confirmed_tags: Vec<ConfirmedTag>,
    /// 贝叶斯评估旁路（最多 6 槽）：纯观测，永不驱动行为。
    #[serde(default)]
    pub bayesian_signals: Vec<BayesianSignal>,
    /// 大五 OCEAN 人格画像：压缩时更新，软提示用，不驱动行为。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_profile: Option<PersonalityProfile>,
```

> 注意：所有 `Contact { ... }` 的内联构造点（测试 fixture、`build_initial_operation_profile` 等）会因新增非 default 字段而编译失败——但本任务全部字段都带 `#[serde(default)]`，结构体字面量构造仍需显式给值。grep `Contact \{` 找全构造点，逐个补这 6 个字段的默认值（`Vec::new()` / `None`）。这是编译期强约束，漏一个就编译失败，不会静默。

- [ ] **Step 4: 运行测试确认通过 + 全库编译**

Run: `cargo test --lib contact_new_trust_fields_default`
Expected: passed。
Run: `cargo check --tests`
Expected: 0 errors（所有 Contact 构造点已补字段）。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs
git commit -m "feat(tag-trust): add manual_tags/confirmed_tags/bayesian_signals/personality_profile to Contact (子计划1 Task3)"
```

---

## Task 4：migration 给存量文档补默认字段

**Files:**
- Create: `src/db/migrations/mNNN_tag_trust_fields.rs`（NNN = 现有最大编号 +1，先 `ls src/db/migrations/` 确认）
- Modify: `src/db/migrations/mod.rs`（注册新 migration step）
- Test: migration 自带的 `#[cfg(test)]` 或 `tests/` 集成（集成留 CI）

**Interfaces:**
- Consumes: Task 3 的字段名（manual_tags / confirmed_tags / bayesian_signals）。
- Produces: migration step，给缺字段的 contacts 文档 `$set` 空默认值。

- [ ] **Step 1: 确认现有 migration 形态**

Run: `ls src/db/migrations/` 看最大编号与命名规则。
Read: `src/db/migrations/mod.rs` 看 `run_step` 注册方式（用 `run_step` 非 `run`，见经验记忆）。
Read 一个最近的纯 `$set` migration（如某个加字段的）作范本。

- [ ] **Step 2: 写 migration（幂等 $set 仅补缺字段）**

```rust
// src/db/migrations/mNNN_tag_trust_fields.rs
use mongodb::bson::doc;
use crate::db::Database;
use crate::error::AppResult;

/// 给存量 contacts 补标签可信度改造的新字段默认值（幂等：仅 $set 到缺字段的文档）。
/// 虽设计称无存量，仍按项目习惯保证反序列化与查询一致。
pub async fn run(db: &Database) -> AppResult<()> {
    db.contacts().update_many(
        doc! { "manual_tags": { "$exists": false } },
        doc! { "$set": { "manual_tags": [], "confirmed_tags": [], "bayesian_signals": [] } },
        None,
    ).await?;
    Ok(())
}
```

> personality_profile 是 `Option`（缺字段→None），无需 migration 补。confirmed_tags/bayesian_signals/manual_tags 是 `Vec`（serde default 已兜底反序列化），migration 仅为查询/索引一致性，幂等安全。
> 实际函数签名/返回类型对齐范本 migration（可能是 `run_step` 包装、可能传 `&mut` 上下文）。

- [ ] **Step 3: 注册到 mod.rs**

按范本在 `src/db/migrations/mod.rs` 的 migration 序列里加一行（用 `run_step`，对齐现有写法）。

- [ ] **Step 4: 编译校验**

Run: `cargo check`
Expected: 0 errors。

> migration 的真实运行验证依赖 MongoDB（testcontainers），留 CI 集成 job。本地只验编译。

- [ ] **Step 5: 提交**

```bash
git add src/db/migrations/mNNN_tag_trust_fields.rs src/db/migrations/mod.rs
git commit -m "feat(tag-trust): migration to backfill trust fields on contacts (子计划1 Task4)"
```

---

## Task 5：manual_tags 运营录入端点

**Files:**
- Modify: `src/routes/contacts.rs`（在 `update_operation_profile` 附近加新 handler，或扩展其 payload —— 见 Step 1 决策）
- Modify: `src/routes/mod.rs`（注册路由）
- Test: 端点逻辑的纯函数部分本地测；HTTP 集成留 CI。

**Interfaces:**
- Consumes: Task 3 的 `manual_tags` 字段。
- Produces: `PUT /api/contacts/:id/manual-tags`，body `{ tags: Vec<String> }`，写 `manual_tags` + `manual_tags_updated_at` + `manual_tags_by`（取登录 admin）。

- [ ] **Step 1: 决策——独立端点 vs 扩展 operation-profile**

Read: `src/routes/contacts.rs:649-744`（`update_operation_profile` 全貌）与 `src/routes/mod.rs` 路由注册段。
决策：**新增独立端点** `PUT /api/contacts/:id/manual-tags`（理由：manual_tags 是人工权威层，独立端点边界清晰，审计 `manual_tags_by` 单独记；不与混杂 AI 字段的 operation-profile 耦合）。

- [ ] **Step 2: 写 handler**

仿 `update_custom_agent_instructions`（`contacts.rs:603`）的结构（同样是单字段 admin 写 + 审计）：

```rust
#[derive(serde::Deserialize)]
pub(super) struct ManualTagsRequest {
    pub tags: Vec<String>,
}

/// PUT /api/contacts/:id/manual-tags
/// 运营录入标签（人工权威层）。自由文本，去空白去重，AI 永不覆盖本字段。
pub(super) async fn update_manual_tags(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ManualTagsRequest>,
) -> AppResult<Json<Value>> {
    let object_id = parse_object_id(&id)?;
    let _ = find_contact_by_id(&state, &admin.current_workspace, &id).await?; // 存在 + workspace scope 校验
    let cleaned = normalize_manual_tags(&payload.tags);
    state.db.contacts().update_one(
        doc! { "_id": object_id, "workspace_id": &admin.current_workspace },
        doc! { "$set": {
            "manual_tags": &cleaned,
            "manual_tags_updated_at": DateTime::now(),
            "manual_tags_by": &admin.username,
        }},
        None,
    ).await?;
    let contact = find_contact_by_id(&state, &admin.current_workspace, &id).await?;
    Ok(Json(json!({ "item": ApiContact::from(contact) })))
}

/// 去首尾空白、去空串、去重保序。自由文本，不查字典（设计选择）。
pub(crate) fn normalize_manual_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let s = t.trim();
        if !s.is_empty() && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    }
    out
}
```

> 确认 `ApiContact::from` 已包含或需补 manual_tags 投影（见 Task 7）。`admin.username` 字段名对齐 `AuthenticatedAdmin` 实际定义（grep 确认）。

- [ ] **Step 3: 写 normalize_manual_tags 纯函数单测**

```rust
#[test]
fn normalize_manual_tags_trims_dedups_drops_empty() {
    let input = vec!["  vip ".to_string(), "vip".to_string(), "".to_string(), "老客户".to_string()];
    assert_eq!(normalize_manual_tags(&input), vec!["vip".to_string(), "老客户".to_string()]);
}
```

- [ ] **Step 4: 注册路由 + 编译 + 测试**

在 `src/routes/mod.rs` 仿现有 `/custom-agent-instructions` 注册行加：
```rust
.route("/api/contacts/:id/manual-tags", put(contacts::update_manual_tags))
```
Run: `cargo test --lib normalize_manual_tags`
Expected: passed。
Run: `cargo check`
Expected: 0 errors。

- [ ] **Step 5: 提交**

```bash
git add src/routes/contacts.rs src/routes/mod.rs
git commit -m "feat(tag-trust): add PUT /contacts/:id/manual-tags operator endpoint (子计划1 Task5)"
```

---

## Task 6：废弃裸 tags，下游 prompt 注入改读 manual_tags + confirmed_tags

**Files:**
- Modify: `src/agent/gateway.rs:3184-3193`（删逐轮 tags 写入；该路径不再写裸 tags）
- Modify: `src/agent/decision.rs:711`、`src/routes/shared.rs:900`、`src/agent/memory.rs:216`（prompt 注入改读新字段）
- Modify: `src/models.rs`（删 `tags` 字段；或保留为 `#[deprecated]` 过渡——见 Step 1 决策）
- Test: 注入渲染纯函数单测

**Interfaces:**
- Consumes: `manual_tags` / `confirmed_tags`。
- Produces: 一个共享渲染 helper，把两层标签渲染成带来源标注的 prompt 文本。

- [ ] **Step 1: 决策——删字段 vs 保留**

设计称无存量数据，且裸 tags 已被三层取代。**直接删 `Contact.tags`**（`models.rs:159-160`）。删后所有读 `contact.tags` 的点编译失败，逐个改到新字段（编译期强约束保证不漏）。

> 风险点：grep `\.tags` 在 Contact 上的**所有**读取点（不止已知 4 处），含 admin 检索 filter（`assets.rs` / `evaluations.rs` 等若读 contact.tags）。逐个评估：prompt 注入点改读新字段；纯展示点改读 confirmed+manual；churn 探针（gateway.rs:3026-3081 读 tags 量化审计）改读 confirmed_tags 的 value 投影。

- [ ] **Step 2: 写共享渲染 helper 的失败测试**

新增 helper（位置：`src/agent/decision.rs` 或新 `src/agent/tag_render.rs`，跟随项目惯例——若 decision.rs 已大则新文件）：

```rust
#[test]
fn render_tags_for_prompt_labels_sources() {
    let manual = vec!["VIP".to_string()];
    let confirmed = vec![
        ConfirmedTag { value: "价格敏感".to_string(), evidences: vec![], confirmed_at: DateTime::from_millis(0), confirmed_by: "consolidation".to_string() },
    ];
    let out = render_tags_for_prompt(&manual, &confirmed);
    assert!(out.contains("VIP"));
    assert!(out.contains("价格敏感"));
    // 来源标注：人工层标"运营确认"，AI 层标"AI 判断"
    assert!(out.contains("运营确认"));
    assert!(out.contains("AI 判断"));
}

#[test]
fn render_tags_for_prompt_empty_yields_empty() {
    assert_eq!(render_tags_for_prompt(&[], &[]), String::new());
}
```

- [ ] **Step 3: 实现 helper + 改 4 个注入点**

```rust
/// 把人工层 + AI 确信层标签渲染成 prompt 文本，标注来源让 LLM 自行掂量分量。
/// 两层皆空 → 空串（调用点据此决定是否注入该段）。
pub(crate) fn render_tags_for_prompt(manual: &[String], confirmed: &[ConfirmedTag]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !manual.is_empty() {
        parts.push(format!("运营确认标签（权威）：{}", manual.join("、")));
    }
    if !confirmed.is_empty() {
        let vals: Vec<&str> = confirmed.iter().map(|c| c.value.as_str()).collect();
        parts.push(format!("AI 判断标签（可能调整）：{}", vals.join("、")));
    }
    parts.join("\n")
}
```

各注入点把原 `contact.tags.join(", ")` 改为 `render_tags_for_prompt(&contact.manual_tags, &contact.confirmed_tags)`：
- `decision.rs:711`
- `shared.rs:900`
- `memory.rs:216-220`（原把 tags 填进 core_facts；改为用两层 value，cap 逻辑不变）
- `prompts.rs:799`（若是模板字面量提及 tags，改文案描述；若是代码注入点同上）

- [ ] **Step 4: 删 gateway 逐轮 tags 写入 + 删字段**

删 `gateway.rs:3184-3193` 的 `if !decision.tags.is_empty() { merge_tags_union_capped... }` 整段（逐轮不再写裸 tags；AI 标签改由子计划 2/3 走 observations + 压缩重判）。
删 `models.rs:159-160` 的 `tags` 字段。
删 `merge_tags_union_capped`（gateway.rs:3113）若无其它调用方（grep 确认；churn 探针若仍引用则一并改）。

> `decision.tags`（AgentDecision 上的字段，types.rs:98）暂留——子计划 2 会把它改写为"写入 observations"的来源。本任务只断"decision.tags → contact.tags 的写回链"，不动 AgentDecision 结构。

- [ ] **Step 5: 编译 + 测试 + 提交**

Run: `cargo test --lib render_tags_for_prompt`
Expected: 2 passed。
Run: `cargo check --tests`
Expected: 0 errors（所有 contact.tags 读取点已改）。
Run: `cargo test --lib`
Expected: ≥ 350 passed / 0 failed（基线不回归）。

```bash
git add src/models.rs src/agent/gateway.rs src/agent/decision.rs src/routes/shared.rs src/agent/memory.rs src/prompts.rs
git commit -m "feat(tag-trust): retire bare tags, prompt injection reads manual+confirmed with source labels (子计划1 Task6)"
```

---

## Task 7：ApiContact 投影 + 顺手修 1（update_profile_note 不覆盖人工层）

**Files:**
- Modify: `src/models.rs` 或 `src/routes/shared.rs`（`ApiContact::from` 定义处——grep 确认）
- Modify: `src/routes/contacts.rs:483-545`（`update_profile_note`）
- Test: 投影字段存在性 + note 端点不写 manual_tags 的逻辑断言

**Interfaces:**
- Consumes: 新字段。
- Produces: `ApiContact` 含 `manual_tags` / `confirmed_tags` / `bayesian_signals` / `personality_profile` 投影；`update_profile_note` 不再触碰人工层、且只生成 AI 层。

- [ ] **Step 1: ApiContact 投影新字段**

grep `struct ApiContact` 找定义与 `From<Contact>` impl。给 ApiContact 加对应字段并在 `from` 里映射（wire 形态 camelCase）。补一个测试断言 `ApiContact::from(contact)` 携带 manual_tags / confirmed_tags。

- [ ] **Step 2: 顺手修 update_profile_note**

现状（`contacts.rs:505-512`）：admin 写 `human_profile_note` 时 `set_doc` 含 `"tags": generated.tags` —— 但 Task 6 已删 `tags` 字段，此处必然编译失败，正好借此修正。

修正语义：人工备注触发的 AI 重生成**只写 AI 层**，绝不触碰 `manual_tags`：
- 删 `"tags": generated.tags`（裸字段已不存在）。
- `generated.tags`（AI 生成的标签）此处的去向：**不直接写 confirmed_tags**（confirmed 是压缩重判产物，不该被 note 旁路直接灌入）。决策：note 重生成只更新 `agent_profile` / `profile_attributes`（画像摘要），标签留给正常的 observations→压缩链路产出。即此处**删 tags 写入，不替换**。
- 确认 `set_doc` 不含任何 `manual_tags` 键（本就不含，验证即可）。

```rust
// 修正后 set_doc（删掉 "tags" 行）：
let mut set_doc = doc! {
    "human_profile_note": payload.human_profile_note,
    "agent_profile": to_bson(&generated.agent_profile)?,
    "profile_attributes": generated.profile_attributes,
    "profile_updated_at": DateTime::now(),
    "updated_at": DateTime::now(),
};
```

- [ ] **Step 3: 测试**

加一个断言：构造 `update_profile_note` 的 set_doc 构造逻辑（若可抽纯函数则抽；否则在集成测试留 CI），本地至少验证编译 + 一个"set_doc 不含 manual_tags 键"的轻量单测（若 handler 难以单测，记 CI 集成覆盖，并在报告里说明）。

- [ ] **Step 4: 编译 + 测试**

Run: `cargo check --tests` → 0 errors。
Run: `cargo test --lib` → ≥ 350 / 0。

- [ ] **Step 5: 提交**

```bash
git add src/models.rs src/routes/contacts.rs src/routes/shared.rs
git commit -m "feat(tag-trust): ApiContact projects trust fields; profile-note regen no longer writes tags (子计划1 Task7)"
```

---

## Task 8：顺手修 2（management.rs MCP 校验旁路补维度校验）

**Files:**
- Modify: `src/routes/management.rs`（`update_contact_profile` 工具，约 :902）
- Test: 维度校验被调用的断言（纯函数边界）

**Interfaces:**
- Consumes: `dimension_registry::validate_dimension_value` + `WriteIntent`。
- Produces: MCP `update_contact_profile` 写 customer_stage / intent_level 前过维度校验（与 admin 写入端点一致）。

- [ ] **Step 1: 核实现状**

Read: `src/routes/management.rs:880-960`（`update_contact_profile` 工具实现）。确认它写 tags/stage/intent 时**未**调 `validate_dimension_value`。对照 `contacts.rs:660-728`（admin 端点如何对 customer_stage/intent_level/relationship_type 走 `validate_dimension_value(..., WriteIntent::AdminWrite)` + `apply_admin_dim_validation`）。

- [ ] **Step 2: 决策 WriteIntent**

management 工具是 AI/MCP 通道还是运营操作？Read 调用上下文确认。
- 若为 AI 经 MCP 写 → 用 `WriteIntent::MachineWrite`（越界 DropSilently，不阻断）。
- 若为运营经 management UI 写 → 用 `WriteIntent::AdminWrite`（越界 Reject 报错）。
依据 `management.rs` 该路由的鉴权与语义判定（默认倾向 MachineWrite，因 management agent 是 AI 侧工具）。

- [ ] **Step 3: 给 stage/intent 写入加校验**

仿 `contacts.rs:660-705` 的 `validate_dimension_value` + `apply_admin_dim_validation`（或 MachineWrite 对应的处理），对 customer_stage / intent_level 写入前归一校验；tags 写入改走 `normalize_manual_tags` 或保持（视该工具语义——若它写的是 AI 画像而非人工层，则 tags 部分按子计划 2 的 observations 方向处理，本任务仅堵 stage/intent 校验旁路，tags 留注释 TODO 指向子计划 2）。

> 严守范围：本任务只补 **stage/intent 维度校验旁路**，不顺势改 tags 写入语义（那属子计划 2/3）。

- [ ] **Step 4: 测试 + 编译**

加断言：越界 stage 值经该路径被 Drop/Reject（对齐选定 WriteIntent）。
Run: `cargo test --lib`（含 dimension_registry 既有测试）→ ≥ 350 / 0。
Run: `cargo check` → 0 errors。

- [ ] **Step 5: 提交**

```bash
git add src/routes/management.rs
git commit -m "fix(tag-trust): management update_contact_profile validates stage/intent dimensions (子计划1 Task8)"
```

---

## Self-Review（写计划者自检）

**Spec 覆盖：**
- 三层数据模型 → Task 1/2/3（结构体 + 字段）✓
- manual_tags 录入 → Task 5 ✓
- 裸 tags 废弃 + 下游改读 → Task 6 ✓
- 顺手修 2 项 → Task 7（note）+ Task 8（management）✓
- 证据/强弱/快通道/压缩/贝叶斯/人格的**逻辑** → 不在本子计划（子计划 2-4），本子计划只落**字段与结构体地基** ✓

**占位符扫描：** Task 4 migration 函数签名、Task 7 Step 3 的"难以单测则留 CI"是**真实的实现期判断点**（依赖未读的范本/handler 形态），非偷懒占位——已显式标注"grep/Read 确认"动作。Task 8 tags 部分明确划归子计划 2，非 TODO 占位。

**类型一致性：** `Evidence{turn,msg_id}`、`ConfirmedTag{value,evidences,confirmed_at,confirmed_by}`、`BayesianSignal{dimension,current_value,current_confidence,locked,history}`、`PersonalityProfile{5×facet,updated_at,snapshots}` 在 Task 1-3、6-7 引用一致 ✓。

**已知需实现期核实（非阻塞，已在步骤内标注 grep/Read）：** Contact 是否 camelCase wire、migration 范本签名、ApiContact 定义位置、admin.username 字段名、management 工具的 WriteIntent 归属。
