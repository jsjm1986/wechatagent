# 设计：`src/routes/knowledge.rs` 模块化解耦

- 日期：2026-06-07
- 状态：已批准设计，待写实施计划
- 范围：**纯机械搬运**的结构性重构，零业务逻辑改动

## 背景与动机

`src/routes/knowledge.rs` 已膨胀到 **9378 行 / 216 个函数**，是第二大路由文件
（`management.rs` 1236 行）的 **7.6 倍**，是整个 routes 层的异常值。它是分阶段叠加
（代码内有 `Phase C` / `Phase G` 分隔注释）形成的，子域边界天然清晰，并非一团乱麻。

动机是**可维护性**，不是性能：在 Rust 中拆单文件为模块树几乎不改变全量/增量编译速度
（crate 才是编译单元）。真正收益在人侧——超出单次 review/单人脑容量、rust-analyzer 在巨型
文件里跳转迟钝、多人改不同子域时冲突集中同一文件。

## 不做什么（范围边界）

- **不改任何业务逻辑**：搬运后行为完全不变。
- **不合并重复 helper、不删 dead code、不统一命名**：那会让 diff 混入逻辑改动、审查变难。
- **不新增/删改任何测试**：基线门测试数必须恒定。
- **不改对外 API 路径**：`routes::knowledge::xxx` 与 `routes::ext_knowledge::xxx` 完全不变。

## 目标结构

单文件拆为 `src/routes/knowledge/` 目录，`mod.rs` 作为 facade。中粒度 9 个子文件：

```
src/routes/knowledge/
├── mod.rs          facade：子模块声明 + pub use 汇总 + 跨域共享 pub(super) helper
├── crud.rs         knowledge/documents/chunks 基础 CRUD + get_chunk_source
├── verify.rs       verify/reject/auto_verify + decide_auto_verify_status + budget/doc helper
├── import.rs       import preview/apply/pdf/image + ingest_chunked_text + extract_tags + VisionProvider
├── catalog.rs      catalog 构建 + completeness 审计 + integrity_report + search/open_slice/test_match
│                   + load_chunks_for_query + clamp_answering_mode + merge_completeness_gaps
├── repair.rs       chunk/pack repair 全流程 + anchor/quote/truncate helper + parse_repair_response
├── chat.rs         chat_turn/apply/history/discard + pipeline + tools + apply_create/update_chunk
│                   + chat_task_create/get/cancel + chat_session_stream + operator-memory 渲染
├── digest_inbox.rs digest today/regenerate/dismiss + knowledge_inbox + 优先级 helper
├── wiki_edit.rs    patch/archive/restore/rollback/revisions/split/merge/relate/unrelate
│                   + batch_verify/archive + list_chunk_referrers + bson_from_json
└── sources_meta.rs gap_signals(list/dismiss/apply/sweep) + ingest_sources(CRUD)
                    + knowledge_aggregate_metadata + list_knowledge_usage + analyze_logs
                    + ask_knowledge/stream + knowledge_metrics + list_operator_memory
```

预估行数：`chat.rs` 最大（约 1900 行，单一内聚子域，可接受），其余落 300–1200 行，与
其余 30 个路由文件量级一致。原 `knowledge.rs` 删除。

### 函数归属原则

按上面函数地图归位；散落的 handler 按业务子域就近归并（如 `ask_*`/`metrics`/`usage`/
`analyze_logs`/`operator-memory` 这类“知识检索/可观测”归 `sources_meta.rs`）。归属以
“调用内聚度”为准，不以行号相邻为准。

## 可见性策略（核心技术决策）

采用 **facade re-export** 方案：

| 项类型 | 可见性 | 说明 |
|---|---|---|
| 对外项（axum handler + 测试 `pub use` 直调的） | 子文件标 `pub` | 靠 `knowledge/mod.rs` 的 `pub use module::*` 暴露，外部路径不变 |
| 跨子文件共享的内部项（`build_catalog`、`load_chunks_for_query` 等） | `pub(super)` | 兄弟模块用 `super::module::fn` 调用；`pub(super)` **不会**被 orphan 测试扫到 |
| 子域私有项 | 私有 `fn` | 不暴露 |
| 跨 2+ 子文件的小 helper（`truncate_for_prompt`/`doc_i64_with_default` 等） | 集中到 `mod.rs` 顶部标 `pub(super)` | 子文件 `super::fn` 调用 |

为什么选 facade 而非“扁平改路径”：facade 让 `routes/mod.rs` 的导入块与 `ext_knowledge`
re-export 块**零改**，15 个依赖测试文件**零改**。改路径方案要逐一改 `mod.rs` 导入 +
15 个测试文件 import，面更大且无收益。

跨子文件共享 helper 的调用分布（已核实）：`budget_document`(9)、`truncate_for_prompt`(7)、
`json_object_to_document`(4)、`doc_i32_with_default`(4)、`serialize_digest_report`(4)、
`doc_i64_with_default`(3)、`split_csv`(3)、`bson_from_json`(2)、`normalize_for_anchor`(2)、
`fuzzy_locate_quote`(2)、`chunk_request_from_chat_patch`(2)。多数只在单一子域内使用，应随
子域走；真正跨域的（如 `truncate_for_prompt`、`doc_i*_with_default`）上提到 `mod.rs`。

## 风险点与处置

1. **`pub(super)` 可见性断裂**：拆到子目录后层级多一级，原 `pub(super)` 语义改变。
   处置：对外项升 `pub`（facade 暴露）、内部共享项保 `pub(super)`（兄弟模块 `super::` 可达）。

2. **`ext_knowledge` 是 15 个测试文件的生命线**：`real_llm_knowledge.rs`、`import_pdf_smoke.rs`、
   `chunk_batch_ops.rs` 等通过 `routes::ext_knowledge::xxx` / `routes::knowledge::xxx` 直调内部
   函数。处置：facade 保证这些名字全部可达；`routes/mod.rs` 的 `pub use` 与 `ext_knowledge`
   块原封不动。

3. **`mod.rs:843` 的 `include_str!("knowledge.rs")` orphan 测试**：`no_orphan_pub_async_route_handlers`
   把每个路由文件当源码字符串扫 `pub async fn`，检查是否都挂载到 router。处置：把 9 个新子文件
   全部加进 `route_files` 数组，删除 `include_str!("knowledge.rs")` 那一行（文件已不存在）。
   **绝不能漏**，否则该测试漏检子文件 = 假绿。`KNOWN_NON_ROUTE_HANDLERS` 内的 3 个名字
   （`ingest_chunked_text`/`import_pdf_bytes`/`build_operation_knowledge_completeness`）不变。

4. **文件尾部 `#[cfg(test)] mod tests`（8759–9378，54 个 `#[test]`）必须分发**：这是单一测试
   模块、`use super::*`，引用多个子域的私有函数（`clamp_answering_mode`、`merge_completeness_gaps`、
   `decide_auto_verify_status`、`parse_repair_response`、`inbox_*`、`source_anchor_for_quote`、
   `normalize_knowledge_tags` 等）。拆分后每个 `#[test]` 必须随它所测的函数迁到对应子文件的
   本地 `#[cfg(test)] mod tests`，否则要么编译失败（调不到私有函数），要么测试数变化破基线门。
   这是最易出错、最需要逐个核对的风险点。

## 测试策略

纯搬运 = 行为不变，因此**不新增、不删改任何测试**。验证靠两条基线门跟跑：

- `cargo test --lib`：**≥ 350 passed, 0 failed**（含本文件迁出的 54 个单测）
- 4 个 PBT 文件累计：**≥ 33 passed, 0 failed**

搬运正确则 lib 测试数恒定。搬运后两门数字应与搬运前**完全一致**（不仅是过阈值）——
数字若变化即说明有测试丢失或重复，是 bug 信号。

## 执行顺序（增量、可回滚）

1. 建 `src/routes/knowledge/mod.rs` 骨架：声明 9 个子模块 + 预置跨域 `pub(super)` helper 占位。
2. 逐个子域搬运。每搬完一个子文件：`cargo check` + `cargo test --lib` 跳跑，绿了再搬下一个。
   出错能立即定位到当前子域，回滚面最小。
3. 9 个子域全绿后，更新 `mod.rs:843` orphan 测试的 `route_files` 数组（加 9 个子文件、删旧行）。
4. 最终 `cargo test --lib` 全量 + 4 个 PBT 验证，确认两门数字与重构前一致。

提交节奏：分子域逐步搬、最后统一提交（遵循“未经允许不 commit”，提交前会先征得同意）。
