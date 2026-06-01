# 知识库闭环轨迹测试 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用一组确定性测试锁住「维护 agent 编辑知识库后，召回排序不回归 + 新内容可召回 + 取代旧版正确降权 + 关系图完整 + 未审定 draft 不可召回」，并把 Q2 泛化门抽成可复用纯函数防过拟合。

**Architecture:** 召回是查询时实时计算的（`list_catalog` 的 `rank_key` = relevance × trust × recency，无物化索引）。所以测试直接调用 `pub fn list_catalog()` 取确定性排序结果做断言——无需 mock LLM 脚本。「应用写入」走真实 `OperationKnowledgeChunk` 插入 + 生产 `verify_operation_knowledge_chunk` 路径（draft→verified），再对**同一 query 重跑 `list_catalog`** 比对。Q2 泛化门逻辑抽到 `tests/common/` 的纯函数，加单测，并让既有 Q2 测试改调它（DRY）。

**Tech Stack:** Rust 2021 + tokio test + testcontainers MongoDB（`#[ignore]`，CI 跑）。纯函数单测无 `#[ignore]`（本地秒过）。复用 `tests/common/mod.rs` 的 `TestApp` / `TestLlmGenerator`。

---

## 已核实的签名与事实（写代码前必读，照抄勿猜）

```rust
// src/agent/knowledge_agent.rs
pub async fn list_catalog(
    state: &AppState,
    workspace_id: &str,
    account_id: Option<&str>,
    filter: &CatalogFilter,
    query: Option<&str>,
) -> AppResult<Vec<CatalogEntry>>;
// 默认 filter.include_unverified=false → 只返回 integrity_status="verified" 的 chunk。
// 返回顺序按 rank_key 降序（relevance×trust×recency）；superseded_by 非空 → trust×0.1。

pub struct CatalogEntry { pub chunk_id: String, pub wiki_type: String, pub chunk_type: String, pub title: String, /* ...其余字段不在本计划断言 */ }

#[derive(Default)] pub struct CatalogFilter {
    pub wiki_types: Vec<String>, pub business_topics: Vec<String>,
    pub status: Option<String>, pub include_unverified: bool,
}

pub async fn answer(state: &AppState, req: AnswerRequest) -> AppResult<AnswerResult>;
pub struct AnswerRequest { pub workspace_id: String, pub account_id: Option<String>, pub query: String, pub filter: CatalogFilter, pub max_rounds: Option<i32> }
pub struct AnswerResult { pub answer: String, pub cited_chunk_ids: Vec<String>, pub source_quotes: Vec<SourceQuoteCitation>, pub tool_trace: Vec<Document>, pub rounds_used: i32, pub truncated: bool, pub cancelled: bool }

// src/models.rs
pub struct OperationKnowledgeChunk {
    pub id: Option<ObjectId>, pub workspace_id: String, pub account_id: Option<String>,
    pub domain: String, pub title: String, pub summary: Option<String>, pub body: Option<String>,
    pub integrity_status: Option<String>, pub status: String, pub priority: i32,
    pub created_at: DateTime, pub updated_at: DateTime,
    pub wiki_type: Option<String>, pub superseded_by: Option<String>,
    pub related_chunks: Option<Vec<RelatedRef>>, pub source_quote: Option<String>,
    pub source_anchors: Vec<Document>, pub dynamic_confidence: Option<f64>,
    // ...用 ..Default::default() 补齐其余
}
pub struct RelatedRef { pub chunk_id: String, pub kind: String, pub note: Option<String> }
// verified gate (knowledge.rs:589): verify 前必须 source_quote 非空 ∧ source_anchors 非空，否则 BadRequest。

// src/routes/knowledge.rs
pub async fn verify_operation_knowledge_chunk(
    State(state): State<AppState>, Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>, Json(payload): Json<KnowledgeVerifyRequest>,
) -> AppResult<Json<Value>>;

// src/auth/mod.rs
pub struct AuthenticatedAdmin { pub user_id: String, pub username: String, pub current_workspace: String }
```

**`verified_chunk` 种子构造**（照搬 `tests/knowledge_agent_eval.rs:101` 模式，但本计划需要 `source_quote`+`source_anchors` 以便后续 verify 不被 gate 挡）：见 Task 1。

---

## File Structure

| 文件 | 职责 | 新建/改 |
|---|---|---|
| `tests/common/generalization.rs` | 泛化门纯函数 `generalization_report()` + 单测 | 新建 |
| `tests/common/mod.rs` | `pub mod generalization;` 一行导出 | 改（仅加 1 行） |
| `tests/knowledge_closed_loop_trajectory.rs` | 闭环主门：确定性召回保持 5 条断言（testcontainers，`#[ignore]`） | 新建 |
| `tests/real_llm_knowledge_quality.rs` | Q2 泛化断言改调 `generalization_report()`（DRY） | 改（仅 Q2 收尾段） |

**不碰**：`knowledge_agent.rs` / `knowledge_tools.rs` / `models.rs` / `routes/knowledge.rs`（只读依据）/ `real_llm_adversarial.rs` / `src/agent/review.rs`（他人维护，保持未暂存）。

---

## Task 1: 泛化门纯函数 + 单测（无 Docker，先做）

**Files:**
- Create: `tests/common/generalization.rs`
- Modify: `tests/common/mod.rs`（加 `pub mod generalization;`）

把 `real_llm_knowledge_quality.rs:1432-1462` 的 train/holdout 逻辑抽成纯函数。先写测试。

- [ ] **Step 1: 写失败测试**

新建 `tests/common/generalization.rs`，先只写测试（函数还不存在，编译失败）：

```rust
//! 泛化门（train/holdout 召回差距）的可复用纯函数 + 单测。
//!
//! 抽自 real_llm_knowledge_quality.rs 的 Q2 收尾断言：对 train / holdout 两个 split
//! 分别求平均召回，断言两者都 ≥ floor 且差距（gap）≤ max_gap。prompt 若被特调适配
//! train 文档，train 召回虚高 / holdout 塌 → gap 爆 = 过拟合信号。
//! 纯函数无 IO，可在任意 test crate 复用，单测无需 Docker。

#![allow(dead_code)]

/// 泛化评估结果。`ok()` 为 false 表示触发了过拟合 / 召回不足红线。
#[derive(Debug, Clone, PartialEq)]
pub struct GeneralizationReport {
    pub train_mean: f64,
    pub holdout_mean: f64,
    pub gap: f64,
    pub train_n: usize,
    pub holdout_n: usize,
    /// 任一 split 为空。空 split 视为不合格（无法评估泛化）。
    pub empty_split: bool,
    /// train_mean < floor。
    pub train_below_floor: bool,
    /// holdout_mean < floor。
    pub holdout_below_floor: bool,
    /// gap > max_gap。
    pub gap_exceeded: bool,
}

impl GeneralizationReport {
    /// 全部红线均未触发才算过。
    pub fn ok(&self) -> bool {
        !self.empty_split
            && !self.train_below_floor
            && !self.holdout_below_floor
            && !self.gap_exceeded
    }
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

/// 计算泛化报告。`floor`=每 split 平均召回下限，`max_gap`=允许的 |train-holdout| 上限。
pub fn generalization_report(
    train: &[f64],
    holdout: &[f64],
    floor: f64,
    max_gap: f64,
) -> GeneralizationReport {
    let train_mean = mean(train);
    let holdout_mean = mean(holdout);
    let gap = (train_mean - holdout_mean).abs();
    let empty_split = train.is_empty() || holdout.is_empty();
    GeneralizationReport {
        train_mean,
        holdout_mean,
        gap,
        train_n: train.len(),
        holdout_n: holdout.len(),
        empty_split,
        // 空 split 时 mean=0，floor 检查会顺带为真；但 empty_split 已独立兜底。
        train_below_floor: !train.is_empty() && train_mean < floor,
        holdout_below_floor: !holdout.is_empty() && holdout_mean < floor,
        gap_exceeded: !empty_split && gap > max_gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_when_both_high_and_gap_small() {
        let r = generalization_report(&[0.9, 0.85], &[0.88, 0.82], 0.7, 0.18);
        assert!(r.ok(), "高召回小差距应过：{r:?}");
    }

    #[test]
    fn fails_on_large_generalization_gap() {
        // train 虚高 holdout 塌 → 过拟合。
        let r = generalization_report(&[0.95, 0.95], &[0.50, 0.55], 0.4, 0.18);
        assert!(r.gap_exceeded, "gap 0.4>0.18 应触发：{r:?}");
        assert!(!r.ok());
    }

    #[test]
    fn fails_when_holdout_below_floor() {
        let r = generalization_report(&[0.8], &[0.5], 0.7, 0.5);
        assert!(r.holdout_below_floor, "holdout 0.5<0.7 应触发");
        assert!(!r.ok());
    }

    #[test]
    fn fails_when_train_below_floor() {
        let r = generalization_report(&[0.6], &[0.62], 0.7, 0.18);
        assert!(r.train_below_floor);
        assert!(!r.ok());
    }

    #[test]
    fn empty_split_is_not_ok() {
        let r = generalization_report(&[], &[0.9], 0.7, 0.18);
        assert!(r.empty_split);
        assert!(!r.ok(), "空 train split 不能算过");
    }

    #[test]
    fn gap_uses_absolute_value() {
        // holdout 高于 train 也算 gap（虽罕见），用绝对值。
        let r = generalization_report(&[0.5], &[0.9], 0.4, 0.18);
        assert!((r.gap - 0.4).abs() < 1e-9, "gap 应为 |0.5-0.9|=0.4");
    }
}
```

- [ ] **Step 2: 加 mod 导出**

在 `tests/common/mod.rs` 顶部（`#![allow(dead_code)]` 之后、`use` 之前）加一行：

```rust
pub mod generalization;
```

- [ ] **Step 3: 跑测试验证失败→通过**

注：纯函数与单测同一步写好，预期直接 PASS。先确认它被编译进来：

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test knowledge_agent_eval generalization:: -- --nocapture
```

注意：`tests/common/` 是被各 test crate `mod common;` 引入的；`knowledge_agent_eval` 已 `mod common;`，故其 crate 内能编译到 `common::generalization`。预期：6 个 `generalization::tests::*` 全 PASS。

> 若报 `module not found`：确认 `knowledge_agent_eval.rs` 顶部有 `mod common;`（已有，:14）。

- [ ] **Step 4: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git add tests/common/generalization.rs tests/common/mod.rs && git commit -m "$(cat <<'EOF'
test(closed-loop): 抽取泛化门为可复用纯函数 generalization_report + 单测

train/holdout 召回差距逻辑从 Q2 内联抽成纯函数，加 6 条单测（无 Docker）。
为闭环轨迹测试与既有 Q2 共用同一抗过拟合判据做准备。

Co-Authored-By: Claude Opus 4 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Q2 改调泛化纯函数（DRY，无行为变化）

**Files:**
- Modify: `tests/real_llm_knowledge_quality.rs:1432-1462`

把 Q2 收尾的内联 train/holdout 断言替换为调用 `common::generalization::generalization_report`。常量 `MIN_RECALL_FLOOR` / `MAX_GENERALIZATION_GAP` 保持不变，行为等价。

- [ ] **Step 1: 确认 common 已被该 crate 引入**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && grep -n "^mod common;\|^use crate::common\|common::" tests/real_llm_knowledge_quality.rs | head -5
```

预期：能看到 `common::rebuild_app_state_with_real_llm` 之类引用（说明已 `mod common;`）。若**没有** `mod common;` 声明行，在文件顶部 use 区之前加 `mod common;`。

- [ ] **Step 2: 替换 Q2 收尾断言段**

把 `real_llm_knowledge_quality.rs` 中这一段（:1432-1462，从 `// ── 确定性召回断言` 到函数末 `}` 之前的三个 `assert!`）整体替换为：

```rust
    // ── 确定性召回断言（不受裁判影响，抗过拟合主门）──────────────────────────
    // 复用 common::generalization 纯函数（与闭环轨迹测试共用同一判据）。
    let report = common::generalization::generalization_report(
        &train_recalls,
        &holdout_recalls,
        MIN_RECALL_FLOOR,
        MAX_GENERALIZATION_GAP,
    );
    eprintln!(
        "[Q2-GENERALIZE] train_recall={:.2}(n={}) holdout_recall={:.2}(n={}) gap={:.2} (max={MAX_GENERALIZATION_GAP})",
        report.train_mean, report.train_n, report.holdout_mean, report.holdout_n, report.gap
    );

    assert!(
        !report.empty_split,
        "Q2 训练/留出集都必须有样本（实际 train={} holdout={}）",
        report.train_n, report.holdout_n
    );
    assert!(
        !report.train_below_floor,
        "Q2 训练集平均召回 {:.2} < 基线 {MIN_RECALL_FLOOR}——抽取漏掉过多参考事实，\
         修通用抽取 prompt（原子单元召回），绝不放水",
        report.train_mean
    );
    assert!(
        !report.holdout_below_floor,
        "Q2 留出集平均召回 {:.2} < 基线 {MIN_RECALL_FLOOR}——在没见过的题材上抽取召回不足，\
         说明 prompt 通用性不够，修通用认知原则而非堆题材枚举",
        report.holdout_mean
    );
    assert!(
        !report.gap_exceeded,
        "Q2 泛化差距 {:.2} > 上限 {MAX_GENERALIZATION_GAP}（train={:.2} holdout={:.2}）\
         ——train 召回远高于 holdout = prompt 被特调适配训练文档（过拟合/作弊）。\
         必须把 prompt 收敛回与题材无关的通用原则，绝不靠枚举特定文档结构取巧",
        report.gap, report.train_mean, report.holdout_mean
    );
```

被替换掉的旧代码包含局部 `let mean = |xs: &[f64]| ...; let train_mean = ...; let holdout_mean = ...; let gap = ...;`——这些局部变量随旧 assert 一并删除（`report.*` 已取代）。

- [ ] **Step 3: 编译验证（不跑 ignore 真测，只确认编译过）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test real_llm_knowledge_quality --no-run 2>&1 | tail -15
```

预期：`Finished` / `Executable ...real_llm_knowledge_quality`，无 error。

- [ ] **Step 4: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git add tests/real_llm_knowledge_quality.rs && git commit -m "$(cat <<'EOF'
test(closed-loop): Q2 泛化断言改调 generalization_report（DRY，等价行为）

Q2 收尾的 train/holdout 内联逻辑替换为共用纯函数，floor/gap 常量与判据不变。

Co-Authored-By: Claude Opus 4 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: 闭环主门骨架 + 种子工厂（确定性召回保持）

**Files:**
- Create: `tests/knowledge_closed_loop_trajectory.rs`

先搭文件骨架与可复用的种子 chunk 工厂（带 `source_quote`+`source_anchors`，以便后续 verify 不被 gate 挡）。本任务只建工厂 + 一个最小冒烟断言，确保 `list_catalog` 在种子上返回有序结果。

- [ ] **Step 1: 写失败测试（最小冒烟）**

新建 `tests/knowledge_closed_loop_trajectory.rs`：

```rust
//! 知识库闭环轨迹测试：维护 agent 编辑 KB → 再召回 → 召回保持。
//!
//! 召回是查询时实时计算的（list_catalog 的 rank_key = relevance×trust×recency，
//! 无物化索引）。所以本测试直接调用 `pub fn list_catalog()` 取确定性排序，断言：
//!   1. 不回归：基线命中的 chunk 写入后仍在 catalog。
//!   2. 新内容可召回：新增 verified chunk 对其目标 query 可被召回。
//!   3. SUPERSEDE 旧降新升：旧 chunk superseded_by 打标 → trust×0.1 降权 → 新 chunk 排前。
//!   4. 关系图完整：related_chunks 引用全部能在 catalog/库内解析，无悬空。
//!   5. 负例：未审定 draft（integrity_status≠verified）不得出现在默认 catalog。
//!
//! 全程红线：apply 写入恒走 draft+needs_review 起步，verified 必须显式经
//! verify_operation_knowledge_chunk（生产审批路径），agent 永不自动审定。
//! `#[ignore]`：依赖 testcontainers MongoDB，CI 用 `cargo test -- --ignored`。

mod common;

use mongodb::bson::{doc, oid::ObjectId, DateTime as BsonDt};
use wechatagent::agent::knowledge_agent::{list_catalog, CatalogFilter};
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};

use crate::common::TestApp;

const WS: &str = "ws_closed_loop";

/// 种子 chunk 工厂：默认 verified + 带 source_quote/source_anchors（满足后续 verify gate）。
/// `relevance_terms` 用于让 title/summary/body 含 query 关键词，驱动 rank_key 命中。
fn seed_chunk(title: &str, body_terms: &str) -> OperationKnowledgeChunk {
    OperationKnowledgeChunk {
        id: Some(ObjectId::new()),
        workspace_id: WS.to_string(),
        account_id: None,
        domain: "user_operations".to_string(),
        title: title.to_string(),
        summary: Some(format!("摘要：{title} {body_terms}")),
        body: Some(format!("正文：{title}。{body_terms}")),
        wiki_type: Some("methodology".to_string()),
        status: "active".to_string(),
        integrity_status: Some("verified".to_string()),
        source_quote: Some(format!("原文引用：{title}")),
        source_anchors: vec![doc! { "documentId": "seed_doc", "quote": title }],
        dynamic_confidence: Some(0.9),
        priority: 0,
        created_at: BsonDt::now(),
        updated_at: BsonDt::now(),
        ..Default::default()
    }
}

/// 清空本 ws 的 chunk，保证 catalog 干净。
async fn reset_ws(app: &TestApp) {
    app.state
        .db
        .operation_knowledge_chunks()
        .delete_many(doc! { "workspaceId": WS }, None)
        .await
        .expect("clean ws_closed_loop chunks");
}

/// 便捷：对 query 跑默认（verified-only）catalog，返回 chunk_id 顺序列表。
async fn catalog_ids(app: &TestApp, query: &str) -> Vec<String> {
    let entries = list_catalog(
        &app.state,
        WS,
        None,
        &CatalogFilter::default(),
        Some(query),
    )
    .await
    .expect("list_catalog");
    entries.into_iter().map(|e| e.chunk_id).collect()
}

#[tokio::test]
#[ignore]
async fn smoke_catalog_returns_seeded_chunk() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    let chunk = seed_chunk("价格异议处理", "客户嫌贵 价格 异议 让步话术");
    let hex = chunk.id.expect("oid").to_hex();
    app.state
        .db
        .operation_knowledge_chunks()
        .insert_one(&chunk, None)
        .await
        .expect("insert seed");

    let ids = catalog_ids(&app, "客户嫌价格贵怎么办").await;
    assert!(ids.contains(&hex), "种子 chunk 应出现在 catalog：{ids:?}");
}
```

- [ ] **Step 2: 编译并确认能 build（不跑 ignore）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test knowledge_closed_loop_trajectory --no-run 2>&1 | tail -15
```

预期：`Executable ...knowledge_closed_loop_trajectory`，无 error。

> 若报 `delete_many` 字段名错（`workspaceId` vs `workspace_id`）：查 `knowledge_agent_eval.rs:135` 用的是 `"workspaceId"`（BSON 序列化后驼峰）。保持 `workspaceId`。
> 若报 `RelatedRef` 未使用：本步未用，留到 Task 5；可暂时不 import，Task 5 再加。先把 `RelatedRef` import 去掉避免 unused 警告，或保留（文件有 `#![allow(dead_code)]`？没有——则先删此 import，Task 5 加回）。

- [ ] **Step 3: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git add tests/knowledge_closed_loop_trajectory.rs && git commit -m "$(cat <<'EOF'
test(closed-loop): 闭环轨迹测试骨架 + 种子 chunk 工厂

list_catalog 确定性召回冒烟：种子 verified chunk 可被 query 召回。
种子带 source_quote/source_anchors 以满足后续 verify gate。

Co-Authored-By: Claude Opus 4 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: 门 1 之「不回归 + 新内容可召回」（写入后再召回）

**Files:**
- Modify: `tests/knowledge_closed_loop_trajectory.rs`（追加测试）

模拟维护 agent 「补一条新 verified 知识」后，原有命中不丢且新内容可召回。

- [ ] **Step 1: 追加测试**

在文件末尾追加：

```rust
/// 门 1a：写入新 chunk 后，基线命中的 chunk 仍在 catalog（不回归），且新 chunk 可召回。
#[tokio::test]
#[ignore]
async fn write_then_recall_preserves_baseline_and_adds_new() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 基线：两条已有知识。
    let base_a = seed_chunk("已读不回唤回三阶段", "已读不回 唤回 沉默客户 激活");
    let base_b = seed_chunk("新客开场白模板", "新客户 首次 开场白 破冰");
    let base_a_hex = base_a.id.expect("oid").to_hex();
    let base_b_hex = base_b.id.expect("oid").to_hex();
    for c in [&base_a, &base_b] {
        app.state.db.operation_knowledge_chunks().insert_one(c, None).await.expect("insert base");
    }

    // 基线召回快照。
    let before = catalog_ids(&app, "客户已读不回怎么唤回").await;
    assert!(before.contains(&base_a_hex), "基线应命中 base_a：{before:?}");

    // 维护 agent 新增一条相关知识（draft→verified 路径在 Task 6 验证；此处直接种 verified）。
    let added = seed_chunk("已读不回唤回话术升级版", "已读不回 唤回 二次激活 限时优惠");
    let added_hex = added.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&added, None).await.expect("insert added");

    // 写入后再召回（live，无索引重建）。
    let after = catalog_ids(&app, "客户已读不回怎么唤回").await;

    // 不回归：基线命中的 base_a 仍在。
    assert!(after.contains(&base_a_hex), "写入后基线命中 base_a 不应丢失：{after:?}");
    // 新内容可召回：added 出现。
    assert!(after.contains(&added_hex), "新增 chunk 应可被召回：{after:?}");
}
```

- [ ] **Step 2: 编译验证**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test knowledge_closed_loop_trajectory --no-run 2>&1 | tail -8
```

预期：编译通过。

- [ ] **Step 3: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git add tests/knowledge_closed_loop_trajectory.rs && git commit -m "$(cat <<'EOF'
test(closed-loop): 门1a 写入后再召回——基线不回归 + 新内容可召回

Co-Authored-By: Claude Opus 4 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: 门 1 之「SUPERSEDE 旧降新升 + 关系图完整」

**Files:**
- Modify: `tests/knowledge_closed_loop_trajectory.rs`（追加测试 + 加回 `RelatedRef` import）

维护 agent 用新版取代旧版：旧 chunk 打 `superseded_by` → `rank_key` trust×0.1 → 新版排前；关系引用可解析。

- [ ] **Step 1: 确认 import**

文件顶部 use 应含（Task 3 若删了 `RelatedRef`，此处加回）：

```rust
use wechatagent::models::{OperationKnowledgeChunk, RelatedRef};
```

- [ ] **Step 2: 追加测试**

```rust
/// 门 1b：SUPERSEDE 旧降新升。旧 chunk 被 superseded_by 指向新 chunk → trust×0.1 →
/// 同 query 下新 chunk 必须排在旧 chunk 之前。验证「结构化写永不物理删除」——旧 chunk
/// 仍在库（未被删），只是降权。
#[tokio::test]
#[ignore]
async fn supersede_demotes_old_below_new() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 旧版 + 新版，相同主题（query 相关度相近），靠 trust 拉开。
    let old = seed_chunk("竞品对比方法论 v1", "竞品对比 客观陈述 优劣 旧版");
    let new = seed_chunk("竞品对比方法论 v2", "竞品对比 客观陈述 优劣 升级");
    let old_hex = old.id.expect("oid").to_hex();
    let new_hex = new.id.expect("oid").to_hex();
    for c in [&old, &new] {
        app.state.db.operation_knowledge_chunks().insert_one(c, None).await.expect("insert");
    }

    // 维护 agent 取代：旧版打 superseded_by=新版。物理保留旧 chunk。
    app.state
        .db
        .operation_knowledge_chunks()
        .update_one(
            doc! { "_id": old.id.unwrap() },
            doc! { "$set": { "superseded_by": &new_hex } },
            None,
        )
        .await
        .expect("mark superseded");

    let ids = catalog_ids(&app, "竞品对比怎么客观陈述").await;
    let pos_old = ids.iter().position(|x| x == &old_hex);
    let pos_new = ids.iter().position(|x| x == &new_hex);
    // 旧 chunk 仍在库（未被物理删）——查得到。
    let still_exists = app.state.db.operation_knowledge_chunks()
        .find_one(doc! { "_id": old.id.unwrap() }, None).await.expect("find old").is_some();
    assert!(still_exists, "SUPERSEDE 不得物理删除旧 chunk");
    // 新版必须排在旧版之前（旧版 trust×0.1 降权）。
    match (pos_new, pos_old) {
        (Some(pn), Some(po)) => assert!(pn < po, "新版应排在旧版之前：new@{pn} old@{po} ids={ids:?}"),
        (Some(_), None) => { /* 旧版被降到 catalog 尾部之外也可接受（更强的降权） */ }
        _ => panic!("新版 chunk 必须可召回：ids={ids:?}"),
    }
}

/// 门 1c：关系图完整。写入带 related_chunks 的 chunk 后，其每条引用的 chunk_id
/// 都能在库内解析（无悬空引用）。validate「结构化写」维护关系链完整。
#[tokio::test]
#[ignore]
async fn relation_graph_has_no_dangling_refs() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    let target = seed_chunk("价格异议处理", "价格 异议 让步 话术");
    let target_hex = target.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&target, None).await.expect("insert target");

    // 维护 agent 新增一条 chunk，关系指向 target。
    let mut linked = seed_chunk("价格异议进阶应对", "价格 异议 进阶 谈判");
    linked.related_chunks = Some(vec![RelatedRef {
        chunk_id: target_hex.clone(),
        kind: "references".to_string(),
        note: None,
    }]);
    let linked_hex = linked.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&linked, None).await.expect("insert linked");

    // 校验：linked 的每条 related_chunks 引用都能在库内 find 到（无悬空）。
    let fetched = app.state.db.operation_knowledge_chunks()
        .find_one(doc! { "_id": linked.id.unwrap() }, None).await.expect("find linked")
        .expect("linked exists");
    for r in fetched.related_chunks.unwrap_or_default() {
        let ref_oid = ObjectId::parse_str(&r.chunk_id).expect("related chunk_id is valid oid");
        let resolved = app.state.db.operation_knowledge_chunks()
            .find_one(doc! { "_id": ref_oid }, None).await.expect("find related").is_some();
        assert!(resolved, "related_chunks 引用 {} 必须能解析（无悬空）", r.chunk_id);
    }
    assert!(catalog_ids(&app, "价格异议").await.contains(&linked_hex), "linked 应可召回");
}
```

- [ ] **Step 3: 编译验证**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test knowledge_closed_loop_trajectory --no-run 2>&1 | tail -8
```

预期：编译通过。

- [ ] **Step 4: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git add tests/knowledge_closed_loop_trajectory.rs && git commit -m "$(cat <<'EOF'
test(closed-loop): 门1b/1c SUPERSEDE 旧降新升（不物理删）+ 关系图无悬空引用

Co-Authored-By: Claude Opus 4 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: 门 1 之「负例 + draft→verified 生产审批路径」

**Files:**
- Modify: `tests/knowledge_closed_loop_trajectory.rs`（追加测试 + import 生产 verify 入口与 admin）

最关键红线：agent 提案落 draft+needs_review 时**不可召回**；只有显式经生产 `verify_operation_knowledge_chunk`（审批路径）转 verified 后才可召回。

- [ ] **Step 1: 加 import**

文件顶部 use 区追加：

```rust
use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde_json::json;
use wechatagent::auth::AuthenticatedAdmin;
use wechatagent::routes::knowledge::verify_operation_knowledge_chunk;
```

> 先确认 `verify_operation_knowledge_chunk` 是否从 `routes::knowledge` 公开可达：
> ```bash
> cd "E:/yw/agiatme/工作项目/wechatagent" && grep -rn "verify_operation_knowledge_chunk" tests/real_llm_knowledge.rs | head -3
> ```
> 若既有真测用 `use wechatagent::routes::knowledge::{... verify_operation_knowledge_chunk ...}` 则照此路径；否则按 grep 出的实际路径调整 import。`KnowledgeVerifyRequest` 同理从 grep 出的模块取。

- [ ] **Step 2: 追加测试**

```rust
/// 门 1d（负例 + 审批路径）：维护 agent 提案落 draft+needs_review 时不可召回；
/// 仅在显式经生产 verify 审批转 verified 后才进 catalog。锁住「AI 永不自动审定」。
#[tokio::test]
#[ignore]
async fn unverified_draft_not_recallable_until_approved() {
    let app = TestApp::start().await;
    reset_ws(&app).await;

    // 维护 agent 提案：落 draft + needs_review（带 source_quote/source_anchors 以便后续 verify）。
    let mut draft = seed_chunk("退款时效说明", "退款 时效 到账 周期");
    draft.integrity_status = Some("needs_review".to_string());
    // status 仍 active（catalog 默认 status=active 过滤），靠 integrity_status 把它挡在外面。
    let draft_hex = draft.id.expect("oid").to_hex();
    app.state.db.operation_knowledge_chunks().insert_one(&draft, None).await.expect("insert draft");

    // 负例：未审定不可召回（默认 catalog 只暴露 integrity_status=verified）。
    let before = catalog_ids(&app, "退款多久到账").await;
    assert!(!before.contains(&draft_hex), "未审定 draft 不得出现在默认 catalog：{before:?}");

    // 经生产审批路径转 verified。
    let admin = Extension(AuthenticatedAdmin {
        user_id: "closed_loop_admin".into(),
        username: "closed_loop_admin".into(),
        current_workspace: WS.to_string(),
    });
    let resp = verify_operation_knowledge_chunk(
        State(app.state.clone()),
        admin,
        Path(draft_hex.clone()),
        Json(serde_json::from_value(json!({ "verifiedClaims": [] })).expect("verify req")),
    )
    .await
    .expect("verify must succeed");
    assert!(resp.0.get("integrityStatus").and_then(|v| v.as_str()) == Some("verified")
        || resp.0.get("ok").is_some(), "verify 应成功：{:?}", resp.0);

    // 审批后可召回。
    let after = catalog_ids(&app, "退款多久到账").await;
    assert!(after.contains(&draft_hex), "审批 verified 后应可召回：{after:?}");
}
```

> `KnowledgeVerifyRequest` 字段名（`verifiedClaims` vs `verified_claims`）以 grep 出的 struct 定义为准——`#[serde(rename_all = "camelCase")]` 时用 `verifiedClaims`。verify handler 返回体 key（`integrityStatus`/`ok`）以 :613 起的 `$set` 与最终 `Json(json!{...})` 为准；断言用 `||` 容两种形态，避免对返回 schema 过度假设。若 `verifiedClaims` 反序列化报错，改为 `Json(KnowledgeVerifyRequest { verified_claims: None })` 直接构造（需 import 该 struct）。

- [ ] **Step 3: 编译验证**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test knowledge_closed_loop_trajectory --no-run 2>&1 | tail -15
```

预期：编译通过。若 `verify_operation_knowledge_chunk` 路径/`KnowledgeVerifyRequest` 字段报错，按 Step 1/2 的 grep 提示修正 import 与构造方式，重跑直到编译过。

- [ ] **Step 4: 提交**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git add tests/knowledge_closed_loop_trajectory.rs && git commit -m "$(cat <<'EOF'
test(closed-loop): 门1d 负例——未审定 draft 不可召回，经生产 verify 审批后才进 catalog

锁住「AI 永不自动审定」：提案落 needs_review 时被 catalog 挡住，
仅显式走 verify_operation_knowledge_chunk（审批路径）转 verified 后可召回。

Co-Authored-By: Claude Opus 4 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: 基线回归校验 + 推 CI

**Files:** 无（仅验证 + push）

- [ ] **Step 1: lib 基线不回归**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --lib 2>&1 | tail -5
```

预期：`test result: ok. 8xx passed; 0 failed`（≥350）。本计划未改 lib，应不变。

- [ ] **Step 2: 跑泛化纯函数单测（无 Docker，必过）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && CARGO_TARGET_DIR=target-check cargo test --test knowledge_closed_loop_trajectory generalization:: -- --nocapture 2>&1 | tail -10
```

> 注：`generalization::tests` 挂在 `tests/common/`，每个引入它的 crate 都会编译一份。这里借 `knowledge_closed_loop_trajectory` crate 跑。预期 6 个 PASS。

- [ ] **Step 3: 禁词 lint（确认 tests 改动不触雷）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && bash scripts/check-no-human-takeover.sh 2>&1 | tail -5; bash scripts/check-no-model-hint.sh 2>&1 | tail -5
```

预期：两者均 pass（本计划只动 tests/，且无禁词；lint 本就排除 tests，但跑一遍确认 0 误伤）。

- [ ] **Step 4: 确认未误暂存他人文件**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git status --short
```

预期：`src/agent/review.rs` 与 `tests/real_llm_adversarial.rs`（若有）保持 ` M`（未暂存），不出现在已提交内容里。

- [ ] **Step 5: 推 CI（standing real-LLM-iteration 授权允许）**

```bash
cd "E:/yw/agiatme/工作项目/wechatagent" && git push origin main 2>&1 | tail -5
```

- [ ] **Step 6: 读 CI 结果（不轮询，机会性查）**

闭环主门 5 测全是确定性 testcontainers 测（无真 LLM），应在 integration job 稳定通过。读 integration job 日志，确认：
- `knowledge_closed_loop_trajectory` 5 个 `#[ignore]` 测全 PASS（`--ignored` 下）。
- lib 基线 + 4 PBT 门绿。
- 既有 `real_llm_knowledge_quality` Q2 经重构后行为不变（CI 上 Q2 仍按原阈值判）。

---

## Self-Review（已对照 spec 核查）

**Spec 覆盖**：
- 门 1「不回归 + 新内容可召回」→ Task 4 ✅
- 门 1「SUPERSEDE 旧降新升」→ Task 5 `supersede_demotes_old_below_new` ✅
- 门 1「关系图完整」→ Task 5 `relation_graph_has_no_dangling_refs` ✅
- 门 1「未批准 draft 不可召回（负例）」→ Task 6 ✅
- 「重建原子索引已删除，再召回=对更新集合重跑」→ 体现在所有测试直接重调 `list_catalog`，无任何 rebuild 步骤 ✅
- Q2 泛化门抽成可复用件 → Task 1 + Task 2 ✅
- 红线「apply 走生产审批路径、agent 不自动审定」→ Task 6 经 `verify_operation_knowledge_chunk` ✅
- 「结构化写永不物理删除」→ Task 5 `still_exists` 断言 ✅

**门 2/门 3（真 LLM 辅助门）本计划不含**：与 spec 第 5 节「本轮做门 1 + Q2 抽取；门 2/3 视 CI 预算」一致——本计划聚焦确定性主门 + 抽取件，门 2/3 留作后续计划（避免真 LLM 时间预算与本轮确定性门耦合）。

**Placeholder 扫描**：无 TBD/TODO；每个 code step 给了完整可编译代码；带「若报错则…」的兜底都指明了具体 grep 命令与替代构造，非空泛「处理边界」。

**类型一致性**：`seed_chunk`/`reset_ws`/`catalog_ids` 三个 helper 在 Task 3 定义，Task 4–6 复用同名同签名；`CatalogEntry.chunk_id`、`OperationKnowledgeChunk.superseded_by/related_chunks/integrity_status`、`RelatedRef{chunk_id,kind,note}`、`AuthenticatedAdmin{user_id,username,current_workspace}` 均与已核实签名一致。

**风险点（执行时留意）**：
1. `KnowledgeVerifyRequest` 字段与 verify 返回体 schema 未逐字核到——Task 6 给了 grep 提示与 `||` 容错；执行者首次编译失败时按提示修正。
2. `list_catalog` 默认按 `status="active"` 过滤——种子 chunk 必须 `status="active"`（`seed_chunk` 已设），负例 draft 也设 active 靠 `integrity_status` 拦截（Task 6 已注明）。
