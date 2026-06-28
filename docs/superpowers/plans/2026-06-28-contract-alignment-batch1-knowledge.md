# 前后端契约对齐机制 — 批次1（地基 + 知识域）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立"投影函数级快照 + 前端 vitest 键集对账"双门契约机制的可复用地基，并以知识域全部 5 个投影函数为首批落地，含防腐烂 lint 与前端 CI job。

**Architecture:** 后端纯 `#[cfg(test)]` 测试构造全量 model → 调投影函数 → 递归排序键 canonicalize → 写/读 `frontend/src/contracts/<name>.fixture.json`（前后端唯一真相源）。前端 vitest 导入同一份 fixture，与显式声明的 `CANONICAL_KEYS` 做双向键集对账。共享 helper `assert_contract_fixture` 下沉到 `src/routes/contract_snapshot.rs` 供五域复用。一个运行时 glob lint 扫 `src/routes/**` 强制每个投影都有契约测试。

**Tech Stack:** Rust（serde_json / mongodb::bson）、cargo test --lib、TypeScript、Vitest 4、GitHub Actions（dorny/paths-filter）。

## Global Constraints

- 测试 only：绝不为过测试改业务逻辑 / prompt / guards / 阈值（过拟合红线）。
- 新增测试只增量叠加，不删改旧维度 / 旧断言。
- `cargo test --lib` baseline ≥350/0 + 4 PBT 累计 ≥33/0 不退。
- 跑 cargo 前必须 `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target"`（磁盘纪律）。
- 子 agent 一律 `model: "opus"`。
- 仅在用户明确要求时 commit（subagent-driven 执行时由控制者按计划 commit 步骤执行）。
- 回复用中文。
- fixture 路径一律用 `env!("CARGO_MANIFEST_DIR")` 定位（仓库非 cargo workspace，frontend/ 是子目录）。
- 对账只断言**键集合**，不断言语义 / 值 / 可选性（避免误判"有意设计"为缺陷）。
- canonical 类型仅供对账，**不**改 `types/index.ts` 业务类型、**不**强塞既有组件。

---

## 文件结构

| 文件 | 职责 | 本批次动作 |
|---|---|---|
| `src/routes/contract_snapshot.rs` | 共享 `assert_contract_fixture` helper + canonicalize + 防腐烂 lint | 新建 |
| `src/routes/mod.rs` | 注册 `contract_snapshot` 子模块 | 改（加 `mod` 行）|
| `src/routes/knowledge/mod.rs` | 知识域 3 投影（document/chunk/usage）契约测试 | 改（POC chunk 测试改用共享 helper + 补 2 个）|
| `src/routes/knowledge/wiki_edit.rs` | `revision_applied_to_json` 契约测试 | 改 |
| `src/routes/knowledge/crud.rs` | 详情端点裸 struct 投影契约测试 | 改 |
| `frontend/src/contracts/*.fixture.json` | 5 个投影的 fixture（后端 bless 写）| 新建（bless 生成）|
| `frontend/src/contracts/*.contract.ts` | 5 个投影的 `CANONICAL_KEYS` 声明 | 新建 |
| `frontend/src/__tests__/contracts/*.contract.test.ts` | 5 个投影的前端键集对账 | 新建 |
| `.github/workflows/ci.yml` | 新增 `frontend-contract` job + paths-filter | 改 |

---

## Task 1: 共享 `assert_contract_fixture` helper 下沉到独立模块

把 POC 内联在 `knowledge/mod.rs` 测试模块里的 helper 抽到 `src/routes/contract_snapshot.rs`，供五域复用。沿用现有 `pub(crate) mod test_helpers`（knowledge_agent.rs:1922）的 `#[cfg(test)] pub(crate)` 先例。

**Files:**
- Create: `src/routes/contract_snapshot.rs`
- Modify: `src/routes/mod.rs`（在 `mod` 声明区加一行）

**Interfaces:**
- Produces:
  - `pub(crate) fn assert_contract_fixture(name: &str, value: serde_json::Value)` — name 是 fixture 文件名（不含扩展名），value 是投影产出。`UPDATE_SNAPSHOTS=1` 写 `frontend/src/contracts/<name>.fixture.json`，否则只读对账，不一致 panic。
  - `pub(crate) fn canonicalize(v: serde_json::Value) -> serde_json::Value` — 递归排序对象键。
  - `pub(crate) fn project_subset(value: serde_json::Value, drop_keys: &[&str]) -> serde_json::Value` — 从顶层对象剔除指定键（用于 raw Document 纯审计字段剔除，spec §4.3 第三档）。

- [ ] **Step 1: 新建模块文件**

Create `src/routes/contract_snapshot.rs`:

```rust
//! 前后端契约快照机制（共享地基）。
//!
//! 每个实体级投影函数（`xxx_json(model) -> Value`）配一个 `#[cfg(test)]` 测试：
//! 构造全量 model → 调投影 → `assert_contract_fixture` → 写/读
//! `frontend/src/contracts/<name>.fixture.json`。fixture 是前后端唯一真相源：
//! 后端测试写它、前端 vitest 导入同一份做键集对账，杜绝手抄漂移。
//!
//! 默认只读对账；`UPDATE_SNAPSHOTS=1 cargo test --lib <name>` re-bless 写文件。

#![cfg(test)]

use serde_json::Value;

/// 递归排序对象键，消除嵌套 BSON Document 的键序抖动，保证快照稳定。
pub(crate) fn canonicalize(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            let mut out = serde_json::Map::new();
            for (k, val) in entries {
                out.insert(k, canonicalize(val));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

/// 从顶层对象剔除指定键（spec §4.3 第三档：纯审计、前端不读的 raw Document 字段）。
/// 非对象原样返回。
pub(crate) fn project_subset(value: Value, drop_keys: &[&str]) -> Value {
    match value {
        Value::Object(mut map) => {
            for k in drop_keys {
                map.remove(*k);
            }
            Value::Object(map)
        }
        other => other,
    }
}

/// 契约 fixture bless/对账。`UPDATE_SNAPSHOTS=1` 写文件，否则只读对账。
pub(crate) fn assert_contract_fixture(name: &str, value: Value) {
    let canonical = canonicalize(value);
    let pretty = serde_json::to_string_pretty(&canonical).expect("serialize fixture") + "\n";

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("frontend/src/contracts")
        .join(format!("{name}.fixture.json"));

    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create contracts dir");
        std::fs::write(&path, &pretty).expect("write fixture");
        return;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "契约 fixture 缺失:{}\n请运行 UPDATE_SNAPSHOTS=1 cargo test --lib {} 生成(bless)。",
            path.display(),
            name
        )
    });
    let existing_canonical =
        canonicalize(serde_json::from_str(&existing).expect("fixture 不是合法 JSON"));
    let existing_pretty =
        serde_json::to_string_pretty(&existing_canonical).expect("re-serialize") + "\n";

    assert_eq!(
        existing_pretty, pretty,
        "\n投影 {name} 的线上形状与 fixture 不一致。\n\
         若后端投影确有变更:运行 UPDATE_SNAPSHOTS=1 cargo test --lib {name} re-bless,\n\
         再同步前端 vitest 契约测试的 CANONICAL_KEYS。\n"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonicalize_sorts_nested_keys() {
        let input = json!({"b": 1, "a": {"d": 2, "c": 3}});
        let out = canonicalize(input);
        let s = serde_json::to_string(&out).unwrap();
        assert_eq!(s, r#"{"a":{"c":3,"d":2},"b":1}"#);
    }

    #[test]
    fn project_subset_drops_top_level_keys() {
        let input = json!({"keep": 1, "dropMe": 2});
        let out = project_subset(input, &["dropMe"]);
        assert_eq!(out, json!({"keep": 1}));
    }
}
```

- [ ] **Step 2: 注册子模块**

Modify `src/routes/mod.rs` — 在 `mod` 声明区（紧邻其它 `mod` 行，按字母序在 `mod conversations;` 之后）插入：

```rust
#[cfg(test)]
mod contract_snapshot;
```

- [ ] **Step 3: 跑 helper 自测**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib contract_snapshot::tests`
Expected: PASS（2 个测试：`canonicalize_sorts_nested_keys`、`project_subset_drops_top_level_keys`）

- [ ] **Step 4: Commit**

```bash
git add src/routes/contract_snapshot.rs src/routes/mod.rs
git commit -m "feat(contract): 共享契约快照 helper assert_contract_fixture 下沉独立模块"
```

---

## Task 2: chunk 列表投影改用共享 helper（重构 POC，确立模板）

POC 在 `knowledge/mod.rs` 测试模块内联了 `assert_contract_fixture` 与 `operation_knowledge_chunk_json_matches_contract_fixture`。本任务删掉内联 helper，改调 Task 1 的共享 helper。fixture 已 bless 存在（POC 产物），内容不变，只换调用来源。

**Files:**
- Modify: `src/routes/knowledge/mod.rs`（删内联 `assert_contract_fixture`，改 `operation_knowledge_chunk_json_matches_contract_fixture` 调用 `super::super::contract_snapshot::assert_contract_fixture`）

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture`（Task 1）

- [ ] **Step 1: 删掉内联 helper**

Modify `src/routes/knowledge/mod.rs` — 删除 POC 加的内联 `#[cfg(test)] fn assert_contract_fixture(...)`（含其内部 `canonicalize` 闭包，整个函数体）。该函数位于 `mod tests` 内、`chunk_json_emits_product_tags` 测试之后。

- [ ] **Step 2: 改测试调用共享 helper**

Modify `src/routes/knowledge/mod.rs` — `operation_knowledge_chunk_json_matches_contract_fixture` 测试末尾两行改为：

```rust
        let projected = operation_knowledge_chunk_json(chunk);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "operation_knowledge_chunk",
            projected,
        );
```

- [ ] **Step 3: 只读对账跑绿（fixture 未变）**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operation_knowledge_chunk_json_matches_contract_fixture`
Expected: PASS（共享 helper 产出与 POC 内联版逐字节一致，fixture 无需 re-bless）

- [ ] **Step 4: Commit**

```bash
git add src/routes/knowledge/mod.rs
git commit -m "refactor(contract): chunk 列表投影契约测试改用共享 helper"
```

---

## Task 3: document 列表投影契约测试

`operation_knowledge_document_json`（mod.rs:229，16 键）补契约测试 + bless fixture。

**Files:**
- Modify: `src/routes/knowledge/mod.rs`（`mod tests` 内新增测试）

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture`、`crate::models::OperationKnowledgeDocument`

- [ ] **Step 1: 写测试（构造全量 document）**

Modify `src/routes/knowledge/mod.rs` — 在 `mod tests` 内 `operation_knowledge_chunk_json_matches_contract_fixture` 之后新增：

```rust
    #[test]
    fn operation_knowledge_document_json_matches_contract_fixture() {
        use crate::models::OperationKnowledgeDocument;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let document = OperationKnowledgeDocument {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899a001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: Some("acc-1".to_string()),
            domain: "user_operations".to_string(),
            source_type: "import".to_string(),
            source_name: Some("产品手册.pdf".to_string()),
            title: "企业版产品手册".to_string(),
            summary: Some("企业版能力总览".to_string()),
            catalog_summary: Some("目录摘要".to_string()),
            routing_map: vec!["产品定位".to_string()],
            risk_notes: vec!["勿夸大疗效".to_string()],
            product_tags: vec!["企业版".to_string()],
            business_topics: vec!["产品定位".to_string()],
            raw_content: Some("原文全文".to_string()),
            content_hash: Some("hash-abc".to_string()),
            line_index: vec![doc! { "line": 1i32 }],
            section_index: vec![doc! { "section": 1i32 }],
            status: "draft".to_string(),
            version: 2,
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            catalog_summary_persisted: Some("持久化目录".to_string()),
            catalog_version: Some(3),
        };
        let projected = operation_knowledge_document_json(document);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "operation_knowledge_document",
            projected,
        );
    }
```

> model 字段已核对（src/models.rs:1380-1411）：`routing_map`/`risk_notes` 是 `Vec<String>`、`line_index`/`section_index` 是 `Vec<Document>`、另有 `product_tags`/`business_topics`/`catalog_summary_persisted`/`catalog_version` 等**不被投影下发**的字段（投影 mod.rs:230-249 只下发 16 键）。fixture 只反映投影输出，故含 16 键、不含 `createdAt`/`productTags`/`catalogSummaryPersisted` 等。构造时全量赋值是为让"投影确实只挑这 16 个"被快照固定。

- [ ] **Step 2: bless fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib operation_knowledge_document_json_matches_contract_fixture`
Expected: PASS，生成 `frontend/src/contracts/operation_knowledge_document.fixture.json`

- [ ] **Step 3: 只读对账跑绿**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib operation_knowledge_document_json_matches_contract_fixture`
Expected: PASS

- [ ] **Step 4: 验证 fixture 键数**

Run: `python -c "import json;print(sorted(json.load(open('frontend/src/contracts/operation_knowledge_document.fixture.json')).keys()))"`
Expected: 打印 16 个 camelCase 键（id/workspaceId/accountId/domain/sourceType/sourceName/title/summary/catalogSummary/routingMap/riskNotes/rawContent/contentHash/lineIndex/sectionIndex/status/version/updatedAt 中投影实际下发的那些）

- [ ] **Step 5: Commit**

```bash
git add src/routes/knowledge/mod.rs frontend/src/contracts/operation_knowledge_document.fixture.json
git commit -m "feat(contract): document 列表投影契约快照"
```

---

## Task 4: knowledge_usage 投影契约测试

`knowledge_usage_json`（mod.rs:321）含 BSON Document 桥接（`route_result` / `tool_trace`，spec §4.3）。构造时给 `route_result` 一个固定 Document、`tool_trace` 一个固定单元素 Vec。

**Files:**
- Modify: `src/routes/knowledge/mod.rs`（`mod tests` 内新增测试）

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture`、`crate::models::KnowledgeUsageLog`

- [ ] **Step 1: 写测试（构造全量 usage log）**

Modify `src/routes/knowledge/mod.rs` — 在 `mod tests` 内 Task 3 测试之后新增：

```rust
    #[test]
    fn knowledge_usage_json_matches_contract_fixture() {
        use crate::models::KnowledgeUsageLog;
        use mongodb::bson::{doc, oid::ObjectId, DateTime};

        let log = KnowledgeUsageLog {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899b001").unwrap()),
            workspace_id: "ws-1".to_string(),
            account_id: Some("acc-1".to_string()),
            contact_wxid: Some("wxid_abc".to_string()),
            run_id: Some("run-1".to_string()),
            knowledge_ids: vec![ObjectId::parse_str("64a1f2c3e4b5a6978899b002").unwrap()],
            route_result: doc! { "matched": 2i32, "strategy": "catalog" },
            reply_text: Some("回复正文".to_string()),
            review_approved: Some(true),
            blocked_reason: None,
            tool_trace: vec![doc! { "tool": "search", "ms": 12i32 }],
            created_at: DateTime::from_millis(1_700_000_000_000),
        };
        let projected = knowledge_usage_json(log);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "knowledge_usage_log",
            projected,
        );
    }
```

> 实现者先读 `src/models.rs` 的 `KnowledgeUsageLog` 定义核对字段名/类型（尤其 `route_result: Document` 与 `tool_trace: Vec<Document>`、各 Option 字段），不符则以 model 为准调整。

- [ ] **Step 2: bless fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib knowledge_usage_json_matches_contract_fixture`
Expected: PASS，生成 `frontend/src/contracts/knowledge_usage_log.fixture.json`

- [ ] **Step 3: 验证 BSON 桥接无 `$` 包装泄漏**

Run: `grep -c '"\$' frontend/src/contracts/knowledge_usage_log.fixture.json || echo "0 (clean)"`
Expected: `0 (clean)`——`into_relaxed_extjson()` 已把 `route_result`/`tool_trace` 的 BSON 包装桥接成纯 JSON（`{"$numberInt"}` 等不应出现）

- [ ] **Step 4: 只读对账跑绿**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib knowledge_usage_json_matches_contract_fixture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/routes/knowledge/mod.rs frontend/src/contracts/knowledge_usage_log.fixture.json
git commit -m "feat(contract): knowledge_usage 投影契约快照(含 BSON 桥接)"
```

---

## Task 5: revision_applied 投影契约测试

`revision_applied_to_json`（wiki_edit.rs:78，7 键）是 chunk patch/revision 的响应形状。函数当前是 `fn`（私有），测试需在同文件 `mod tests` 内调用，无需改可见性。

**Files:**
- Modify: `src/routes/knowledge/wiki_edit.rs`（文件末尾新增或扩展 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture`、`crate::knowledge_wiki::chunk_revisions::RevisionApplied`

- [ ] **Step 1: 确认是否已有 test 模块**

Run: `grep -n "mod tests" src/routes/knowledge/wiki_edit.rs || echo "no test mod"`
Expected: 若有则在其内追加测试；若无则新建。

- [ ] **Step 2: 写测试**

Modify `src/routes/knowledge/wiki_edit.rs` — 在文件末尾新增（若已有 `mod tests` 则把 `#[test] fn` 放进去）：

```rust
#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::knowledge_wiki::chunk_revisions::RevisionApplied;

    #[test]
    fn revision_applied_to_json_matches_contract_fixture() {
        let applied = RevisionApplied {
            revision_id: "rev-1".to_string(),
            chunk_id: "chunk-1".to_string(),
            op: "patch".to_string(),
            before_hash: Some("hash-before".to_string()),
            after_hash: Some("hash-after".to_string()),
            unchanged: false,
        };
        let projected = revision_applied_to_json(&applied);
        crate::routes::contract_snapshot::assert_contract_fixture(
            "revision_applied",
            projected,
        );
    }
}
```

> 实现者先读 `src/knowledge_wiki/chunk_revisions.rs:111` 的 `RevisionApplied` 定义核对字段名/类型（`op` / `before_hash` / `after_hash` / `unchanged` 的确切类型，Option 与否），不符则以 struct 为准调整。投影输出 7 键：`ok`(常量 true)/`revisionId`/`chunkId`/`op`/`beforeHash`/`afterHash`/`unchanged`。

- [ ] **Step 3: bless + 只读对账**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib revision_applied_to_json_matches_contract_fixture && cargo test --lib revision_applied_to_json_matches_contract_fixture`
Expected: 两次都 PASS，生成 `frontend/src/contracts/revision_applied.fixture.json`（7 键）

- [ ] **Step 4: Commit**

```bash
git add src/routes/knowledge/wiki_edit.rs frontend/src/contracts/revision_applied.fixture.json
git commit -m "feat(contract): revision_applied 投影契约快照"
```

---

## Task 6: chunk 详情端点裸 struct 投影契约测试（暴露列表/详情形状冲突）

详情端点 `crud.rs:357` 是 `json!({"item": item})`——裸 serde struct（snake_case + `{$oid}`），与列表投影（camelCase）形状冲突（spec §1.4 / §9）。本任务**快照它、暴露冲突**，不强制统一（产品决策）。

**Files:**
- Modify: `src/routes/knowledge/crud.rs`（`mod tests` 内新增测试）

**Interfaces:**
- Consumes: `crate::routes::contract_snapshot::assert_contract_fixture`、`crate::models::OperationKnowledgeChunk`

- [ ] **Step 1: 写测试（裸 struct 序列化）**

Modify `src/routes/knowledge/crud.rs` — 在文件末尾新增：

```rust
#[cfg(test)]
mod contract_tests {
    use crate::models::OperationKnowledgeChunk;
    use mongodb::bson::{oid::ObjectId, DateTime};

    /// 详情端点 `get_operation_knowledge_chunk`(crud.rs:357) 直接 `json!({"item": item})`
    /// 裸序列化 model——snake_case + `{$oid}`，与列表投影 camelCase **形状冲突**。
    /// 本快照刻意暴露该冲突(spec §9):快照它,让"统一与否"成为可见的产品决策,而非静默漂移。
    #[test]
    fn chunk_detail_raw_struct_matches_contract_fixture() {
        let chunk = OperationKnowledgeChunk {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899aabb").unwrap()),
            workspace_id: "ws-1".to_string(),
            title: "7x24 自动应答".to_string(),
            domain: "user_operations".to_string(),
            status: "draft".to_string(),
            created_at: DateTime::from_millis(1_700_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
            ..Default::default()
        };
        let projected = serde_json::json!({ "item": chunk });
        crate::routes::contract_snapshot::assert_contract_fixture(
            "operation_knowledge_chunk_detail",
            projected,
        );
    }
}
```

> 这里 `..Default::default()` 而非全量赋值——裸 struct 受 `#[serde(skip_serializing_if)]` 影响，None 字段会被跳过，全量赋值会让 fixture 含大量 snake_case 键。最小构造已足以暴露"snake_case + `{$oid}` + 嵌套 item"这一形状特征，这是本任务唯一目的。

- [ ] **Step 2: bless fixture**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && UPDATE_SNAPSHOTS=1 cargo test --lib chunk_detail_raw_struct_matches_contract_fixture`
Expected: PASS，生成 `frontend/src/contracts/operation_knowledge_chunk_detail.fixture.json`

- [ ] **Step 3: 验证形状冲突确实暴露**

Run: `grep -E '"_id"|"\$oid"|"workspace_id"' frontend/src/contracts/operation_knowledge_chunk_detail.fixture.json`
Expected: 命中 `_id`/`$oid`/`workspace_id`——证明详情端点确实下发 snake_case + BSON 包装，与列表投影 camelCase 冲突（此冲突现已被快照固定，后续任何"统一"改动都会触发 re-bless 提醒）

- [ ] **Step 4: 只读对账跑绿 + Commit**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib chunk_detail_raw_struct_matches_contract_fixture`
Expected: PASS

```bash
git add src/routes/knowledge/crud.rs frontend/src/contracts/operation_knowledge_chunk_detail.fixture.json
git commit -m "feat(contract): chunk 详情裸 struct 投影契约快照(暴露列表/详情形状冲突)"
```

---

## Task 7: 前端契约对账 harness（chunk 模板已存在，扩展到 5 投影）

POC 已建 chunk 的 `CANONICAL_KEYS` + 对账测试。本任务为另外 4 个 fixture（document/usage/revision/detail）建对应 `*.contract.ts` 声明 + `*.contract.test.ts` 对账。

**Files:**
- Create: `frontend/src/contracts/operationKnowledgeDocument.contract.ts`
- Create: `frontend/src/contracts/knowledgeUsageLog.contract.ts`
- Create: `frontend/src/contracts/revisionApplied.contract.ts`
- Create: `frontend/src/contracts/operationKnowledgeChunkDetail.contract.ts`
- Create: `frontend/src/__tests__/contracts/knowledgeDomain.contract.test.ts`（一个文件覆盖 4 个新投影，复用 POC 已建的 chunk 测试不动）

**Interfaces:**
- Consumes: Task 3-6 bless 出的 4 个 fixture JSON

- [ ] **Step 1: 读 4 个 fixture 的实际键集**

Run:
```bash
for f in operation_knowledge_document knowledge_usage_log revision_applied operation_knowledge_chunk_detail; do
  echo "=== $f ==="
  python -c "import json;print(sorted(json.load(open('frontend/src/contracts/$f.fixture.json')).keys()))"
done
```
Expected: 打印 4 组键集，作为下面 `CANONICAL_KEYS` 的逐字来源。

- [ ] **Step 2: 写 document 声明**

Create `frontend/src/contracts/operationKnowledgeDocument.contract.ts`（键数组逐字填 Step 1 打印的 document 键集）：

```ts
// 契约对账声明 —— GET document 列表投影 operation_knowledge_document_json 的线上键集。
// 仅服务契约对账测试,不是业务类型。后端改投影→re-bless→此处对账测红强制同步。
export const CANONICAL_KEYS = [
  // ← 逐字填入 Step 1 打印的 operation_knowledge_document 键集，每行一个字符串
] as const;
export type CanonicalKey = (typeof CANONICAL_KEYS)[number];
```

- [ ] **Step 3: 写 usage / revision / detail 三个声明**

同 Step 2 模式，分别创建 `knowledgeUsageLog.contract.ts`、`revisionApplied.contract.ts`、`operationKnowledgeChunkDetail.contract.ts`，键数组各自逐字填 Step 1 对应键集。注释里写明各自对应的后端投影函数名。

- [ ] **Step 4: 写统一对账测试**

Create `frontend/src/__tests__/contracts/knowledgeDomain.contract.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import documentFixture from "../../contracts/operation_knowledge_document.fixture.json";
import usageFixture from "../../contracts/knowledge_usage_log.fixture.json";
import revisionFixture from "../../contracts/revision_applied.fixture.json";
import detailFixture from "../../contracts/operation_knowledge_chunk_detail.fixture.json";
import { CANONICAL_KEYS as DOCUMENT_KEYS } from "../../contracts/operationKnowledgeDocument.contract";
import { CANONICAL_KEYS as USAGE_KEYS } from "../../contracts/knowledgeUsageLog.contract";
import { CANONICAL_KEYS as REVISION_KEYS } from "../../contracts/revisionApplied.contract";
import { CANONICAL_KEYS as DETAIL_KEYS } from "../../contracts/operationKnowledgeChunkDetail.contract";

// 后端投影写出的 fixture(线上真相源)与前端 CANONICAL_KEYS 声明双向键集对账。
// 任一侧漂移即测红:missingInFrontend=后端发了前端没声明;deadInFrontend=前端声明了后端没发。
function assertKeysMatch(
  label: string,
  fixture: Record<string, unknown>,
  declared: readonly string[],
) {
  const actual = Object.keys(fixture).sort();
  const decl = [...declared].sort();
  const missingInFrontend = actual.filter((k) => !decl.includes(k));
  const deadInFrontend = decl.filter((k) => !actual.includes(k));
  expect(
    { missingInFrontend, deadInFrontend },
    `${label}: 后端新增字段→前端须在 CANONICAL_KEYS 登记;后端删字段→前端须清理死键`,
  ).toEqual({ missingInFrontend: [], deadInFrontend: [] });
  expect(actual.length, `${label}: fixture 非空`).toBeGreaterThan(0);
}

describe("契约: 知识域投影键集对账", () => {
  it("operation_knowledge_document 列表投影", () =>
    assertKeysMatch("document", documentFixture, DOCUMENT_KEYS));
  it("knowledge_usage_log 投影", () =>
    assertKeysMatch("usage", usageFixture, USAGE_KEYS));
  it("revision_applied 投影", () =>
    assertKeysMatch("revision", revisionFixture, REVISION_KEYS));
  it("operation_knowledge_chunk_detail 详情裸 struct 投影", () =>
    assertKeysMatch("detail", detailFixture, DETAIL_KEYS));
});
```

- [ ] **Step 5: 跑前端对账（含 POC 已建的 chunk 测试）**

Run: `cd frontend && npx vitest run src/__tests__/contracts/`
Expected: 全 PASS（POC 的 `operationKnowledgeChunk.contract.test.ts` 2 测试 + 本任务 4 测试）

- [ ] **Step 6: Commit**

```bash
git add frontend/src/contracts/operationKnowledgeDocument.contract.ts frontend/src/contracts/knowledgeUsageLog.contract.ts frontend/src/contracts/revisionApplied.contract.ts frontend/src/contracts/operationKnowledgeChunkDetail.contract.ts frontend/src/__tests__/contracts/knowledgeDomain.contract.test.ts
git commit -m "feat(contract): 知识域 4 投影前端键集对账 harness"
```

---

## Task 8: 防腐烂 lint — 强制每个投影都有契约测试

运行时 glob 扫 `src/routes/**`，正则识别 `fn \w+_json(...) -> Value` 投影，断言每个非豁免投影都有对应 fixture + 契约测试。新增投影忘配测试 → 红（spec §6）。

**Files:**
- Modify: `src/routes/contract_snapshot.rs`（在 `mod tests` 内加 lint 测试 + 豁免清单）

**Interfaces:**
- Consumes: 仅 `std::fs`（纯递归遍历 + 字符串匹配，**不新增依赖**——已确认 `regex` 非本项目依赖，`grep -rln 'regex::Regex' src/` 零命中）

- [ ] **Step 1: 确认无 regex 依赖，用纯 std 实现**

Run: `grep -c 'regex' Cargo.toml || echo "0 (无 regex,用纯 str 匹配)"`
Expected: `0 (无 regex,用纯 str 匹配)`——故下面用 `str::find` 手扫 `fn `+`_json`+同行 `-> Value`，不引入 regex。

- [ ] **Step 2: 写 lint 测试**

Modify `src/routes/contract_snapshot.rs` — 在 `mod tests` 内新增：

```rust
    /// 防腐烂:扫 src/routes/** 找所有投影函数(`fn <name>_json(...) -> Value`),
    /// 断言每个非豁免投影都有契约测试(测试源里出现该投影名)。新增投影忘配测试 → 红。
    /// 现有 no_orphan_pub_async_route_handlers(mod.rs:1052)手维护清单已腐烂,故用运行时扫描。
    /// 纯 std 实现(本项目无 regex 依赖):逐行找 `fn ` 且行内含 `_json` 且含 `-> Value`。
    #[test]
    fn every_projection_has_contract_test() {
        use std::fs;
        use std::path::{Path, PathBuf};

        // 非实体投影豁免清单(helper / 非 model→Value / 异步生成器),逐条注明理由。
        const ALLOWLIST: &[&str] = &[
            "bson_from_json",          // helper:JSON→BSON Document,非投影
            "bson_doc_to_json",        // helper:Document→Value 通用桥
            "parse_warning_to_json",   // 解析告警,非实体投影
            "vision_generate_json",    // async LLM 调用,非 model→Value
            "lesson_doc_to_json",      // 入参是裸 Document 非 model(批次2 评估纳入)
            "experiment_summary_json", // 批次4 进化域(本批不覆盖)
            "experiment_envelope_json",
            "proposal_summary_json",
            "proposal_detail_json",
            "cohort_run_ids_json",
            "shadow_replay_json",
            "threshold_override_json",
            "threshold_override_audit_json",
            "runtime_flag_json",
            // 批次2/3/5 域投影:本批次只覆盖知识域,其余域在后续批次纳入。
            // 实现者:批次铺开时从本清单移除对应项,使 lint 真正强制。
            "operation_state_policy_json",
            "taxonomy_candidate_json",
            "suspected_deal_json",
            "outbox_entry_json",
            "relationship_suggestion_json",
            "taxonomy_entry_json",
            "behavior_signal_metric_json",
            "operation_domain_json",
            "evaluation_scenario_json",
            "playbook_json",
            "prompt_template_json",
            "operation_health_json",
            "guide_preview_json",
            "operating_memory_json",
            "memory_candidate_json",
            "llm_call_log_json",
            "decision_review_json",
            "agent_run_json",
            "outcome_metric_json",
        ];

        fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    collect_rs(&p, out);
                } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(p);
                }
            }
        }

        // 从一行形如 `... fn operation_knowledge_chunk_json(item: ...` 抽出投影名。
        // 返回 Some(name) 仅当该行含 `fn `、名字以 `_json` 结尾、且(同行或紧邻)有 `-> Value`。
        fn extract_projection_name(line: &str) -> Option<String> {
            let after_fn = line.split("fn ").nth(1)?;
            // 名字到第一个 `(` 或 `<` 为止。
            let name_end = after_fn.find(|c| c == '(' || c == '<')?;
            let name = after_fn[..name_end].trim();
            if name.ends_with("_json") && !name.is_empty() {
                Some(name.to_string())
            } else {
                None
            }
        }

        let routes_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes");
        let mut files = Vec::new();
        collect_rs(&routes_dir, &mut files);

        let all_src: Vec<String> = files
            .iter()
            .map(|f| fs::read_to_string(f).unwrap_or_default())
            .collect();

        // 覆盖集:一个投影"被契约测试覆盖"当且仅当它出现在某个**契约测试块**里。
        // 契约测试块 = 含 `assert_contract_fixture` 调用的代码区。production handler 里
        // 调用投影(document=4/chunk=14 次)不算覆盖——必须是测试块里调用,才挡得住
        // "有 production 调用方但零测试"的投影。
        // 纯 std 近似:把源按 `assert_contract_fixture` 出现位置切窗,取每次出现前后 ~600 字符
        // 窗口(覆盖一个测试函数体),窗口里出现的 `_json` 名即记为已覆盖。
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        for src in &all_src {
            let bytes = src.as_bytes();
            let mut from = 0usize;
            while let Some(rel) = src[from..].find("assert_contract_fixture") {
                let pos = from + rel;
                let start = pos.saturating_sub(600);
                let end = (pos + 200).min(bytes.len());
                // 安全切到 char 边界。
                let mut s = start;
                while s < src.len() && !src.is_char_boundary(s) { s += 1; }
                let mut e = end;
                while e < src.len() && !src.is_char_boundary(e) { e += 1; }
                let window = &src[s..e];
                for line in window.lines() {
                    if let Some(name) = extract_projection_name(line) {
                        covered.insert(name);
                    }
                    // 也捕捉 `let projected = foo_json(...)` 这类调用行(无 `fn `)。
                    for tok in line.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
                        if tok.ends_with("_json") && tok.len() > 5 {
                            covered.insert(tok.to_string());
                        }
                    }
                }
                from = pos + "assert_contract_fixture".len();
            }
        }

        // 收集所有投影定义,逐个比对覆盖集。
        let mut orphans = Vec::new();
        for src in &all_src {
            for line in src.lines() {
                if !line.contains("fn ") || !line.contains("_json") || !line.contains("-> Value") {
                    continue;
                }
                if let Some(name) = extract_projection_name(line) {
                    if ALLOWLIST.contains(&name.as_str()) {
                        continue;
                    }
                    if !covered.contains(&name) {
                        orphans.push(name);
                    }
                }
            }
        }

        assert!(
            orphans.is_empty(),
            "以下投影函数缺契约测试(加测试或加入 ALLOWLIST 并注明理由):\n{}",
            orphans.join("\n")
        );
    }
```

> 实现者注意：①覆盖判定靠"投影名出现在 `assert_contract_fixture` 调用的 ~600 字符窗口内"——production handler 调用投影**不在**这种窗口里，故挡得住"有调用方但零测试"。②600/200 字符窗口是经验值，覆盖一个典型契约测试函数体；若某测试特别长导致 `<name>(...)` 落在窗口外，把窗口调大或在测试里把 `assert_contract_fixture` 紧跟投影调用（计划里的模板都是紧跟，无此问题）。③`extract_projection_name` 只认 `fn ` 定义行；窗口里的调用行靠 `split + ends_with("_json")` token 扫描捕捉。④ALLOWLIST 含本批未覆盖域，批次铺开时逐项移除使 lint 渐紧。

- [ ] **Step 3: 跑 lint，确认知识域 3 个 mod.rs 投影 + 2 个其它文件投影全部放行**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib every_projection_has_contract_test`
Expected: PASS（知识域 5 投影都有测试，其余域在 ALLOWLIST）

- [ ] **Step 4: 反向验证 lint 真咬人（临时删一个测试名引用）**

Run: 临时把 `knowledge/mod.rs` 里 `operation_knowledge_document_json_matches_contract_fixture` 测试名改成别的（让 `all_test_src` 不再含 `operation_knowledge_document_json`），跑 lint：
`export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib every_projection_has_contract_test`
Expected: FAIL，orphans 列出 `operation_knowledge_document_json`。**确认后改回原测试名**，重跑 Step 3 绿。

- [ ] **Step 5: Commit**

```bash
git add src/routes/contract_snapshot.rs
git commit -m "feat(contract): 防腐烂 lint 强制每个投影配契约测试"
```

---

## Task 9: 前端 CI job — 让前端契约对账进入合并门

现状 `ci.yml` 的 `paths-ignore` 含 `frontend/**`，前端改动不触发任何 job。改造：用 `dorny/paths-filter` 在 job 内判定，新增 `frontend-contract` job 跑 `npm ci → tsc --noEmit → vitest run`，同时保持后端 job 对纯前端改动仍跳过（不烧真模型配额，spec §7）。

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: 前端 `package.json` 的 `test` 脚本（`vitest run`，已存在）

- [ ] **Step 1: 读现有 ci.yml 结构确定插入点**

Run: `grep -n 'jobs:\|runs-on:\|paths-ignore:\|name:' .github/workflows/ci.yml | head -40`
Expected: 定位 `jobs:` 起始行与第一个 job，确定 `frontend-contract` job 插入位置。

- [ ] **Step 2: 新增 frontend-contract job**

Modify `.github/workflows/ci.yml` — 在 `jobs:` 下新增（与其它 job 同级缩进）：

```yaml
  frontend-contract:
    name: 前端契约对账 (tsc + vitest)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          cache: 'npm'
          cache-dependency-path: frontend/package-lock.json
      - name: 安装依赖
        working-directory: frontend
        run: npm ci
      - name: 类型检查
        working-directory: frontend
        run: npx tsc --noEmit
      - name: 契约对账 + 组件测试
        working-directory: frontend
        run: npx vitest run
```

> 此 job 不在 `paths-ignore` 控制范围内的问题：`paths-ignore` 是 workflow 级，会让整个 workflow 在纯前端改动时不触发，从而**前端 job 也不跑**。这与目标相反。Step 3 解决。

- [ ] **Step 3: 改 paths 策略让前后端各自正确触发**

Modify `.github/workflows/ci.yml` — 把顶层 `on.pull_request` 与 `on.push` 的 `paths-ignore`（含 `frontend/**`）**移除 `frontend/**` 一项**，保留 `docs/**` 与 `**/*.md`。这样前端改动会触发 workflow。然后在**每个后端 job**（baseline / integration / real-llm 等）加 paths-filter 守卫，使其在纯前端改动时跳过：

在 `jobs:` 顶部新增一个 `changes` job：

```yaml
  changes:
    runs-on: ubuntu-latest
    outputs:
      backend: ${{ steps.filter.outputs.backend }}
      frontend: ${{ steps.filter.outputs.frontend }}
    steps:
      - uses: actions/checkout@v4
      - uses: dorny/paths-filter@v3
        id: filter
        with:
          filters: |
            backend:
              - 'src/**'
              - 'tests/**'
              - 'Cargo.toml'
              - 'Cargo.lock'
              - 'scripts/**'
              - '.github/workflows/ci.yml'
              - 'frontend/src/contracts/**'
            frontend:
              - 'frontend/**'
```

> 注意 `backend` filter 含 `frontend/src/contracts/**`——契约 fixture 由后端 bless，改了 fixture 必须跑后端 baseline（投影测试在那里）。`frontend-contract` job 加 `needs: changes` + `if: ${{ needs.changes.outputs.frontend == 'true' }}`；各后端 job 加 `needs: changes` + `if: ${{ needs.changes.outputs.backend == 'true' }}`。

- [ ] **Step 4: 本地校验 YAML 合法**

Run: `python -c "import yaml;yaml.safe_load(open('.github/workflows/ci.yml'));print('YAML OK')"`
Expected: `YAML OK`

- [ ] **Step 5: 校验 baseline gate 仍含契约测试**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib contract 2>&1 | tail -5`
Expected: 契约相关测试全 PASS（确认它们随 `cargo test --lib` baseline 一起跑，无需独立 CI step）

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(contract): 新增前端契约对账 job + paths-filter 前后端分流"
```

---

## Task 10: 批次收口 — baseline 不退 + 前端三连绿

**Files:** 无（验证任务）

- [ ] **Step 1: 后端 baseline 全量**

Run: `export CARGO_TARGET_DIR="E:/yw/agiatme/工作项目/wechatagent/target" && cargo test --lib 2>&1 | tail -5`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350 + 本批新增（POC 1 + Task1 2 + Task3/4/5/6 各1 + Task8 1 = 原基线 +8 左右）。0 failed。

- [ ] **Step 2: 前端 tsc + vitest 全量**

Run: `cd frontend && npx tsc --noEmit && npx vitest run 2>&1 | tail -8`
Expected: tsc 无错误；vitest 全 PASS（含 contracts/ 6 测试 + 既有组件测试无回归）。

- [ ] **Step 3: 确认无残留临时漂移**

Run: `grep -rn 'pocTempField\|pocPhantomKey\|POC_RED_TEST' src/ frontend/src/ || echo "clean"`
Expected: `clean`（Task 8 Step 4 的临时改动已还原；POC 验证用的临时字段无残留）

- [ ] **Step 4: 确认 5 个 fixture 齐备**

Run: `ls frontend/src/contracts/*.fixture.json`
Expected: 5 个文件——operation_knowledge_chunk / operation_knowledge_document / knowledge_usage_log / revision_applied / operation_knowledge_chunk_detail。

---

## Self-Review（写完计划后自查，已修）

**Spec 覆盖：** §3.3 后端门→Task1-6；§3.4 前端门→Task7；§4.2 聚合响应→本批不含（ALLOWLIST 豁免，批次2-5 纳入，spec §5 分批）；§4.3 raw Document→Task4(usage BSON 桥接) + `project_subset` helper(Task1，供后续审计字段剔除)；§5 五域分批→本计划是批次1，明确声明；§6 防腐烂 lint→Task8；§7 CI→Task9。**盲区诚实声明**：§4.2 inline 聚合 + 详情统一决策不在本批，已在 ALLOWLIST 注明。

**占位符扫描：** 无 TBD/TODO。唯一"逐字填入"在 Task7 Step2/3（`CANONICAL_KEYS` 键数组来自 Task3-6 bless 出的真实 fixture，Step1 命令打印来源）——这是 bless 驱动的必然顺序，非占位符（键集是后端测试产物，计划写死会与实际投影漂移，故指令为"读 fixture 填"）。

**类型一致：** `assert_contract_fixture(name, value)` 签名 Task1 定义、Task2-6 一致调用；`canonicalize`/`project_subset` Task1 定义；fixture 命名 `<snake_name>.fixture.json` 与 `CANONICAL_KEYS` 文件 `<camelName>.contract.ts` 全程一致。model 字段名标注"实现者以 src/models.rs 为准核对"——因为计划无法100%保证 model 字段名（实现者必读 model），已在 Task3/4/5 显式要求核对。
