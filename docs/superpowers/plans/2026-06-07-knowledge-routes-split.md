# knowledge.rs 模块化解耦 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `src/routes/knowledge.rs`（9378 行 / 216 函数）纯机械搬运为 `src/routes/knowledge/` 下 9 个子文件 + 1 个 facade `mod.rs`，行为零改动、对外路径不变、基线测试数恒定。

**Architecture:** facade re-export 方案——子模块声明为私有 `mod xxx;`，`knowledge/mod.rs` 用 `pub use xxx::*` 汇总，对外暴露面与拆分前完全一致。对外 handler 标 `pub`、跨子文件共享内部项标 `pub(super)`（兄弟模块经 `super::` 访问）、子域私有项保持私有。被 2+ 子域复用的纯 helper 上提到 `mod.rs`。

**Tech Stack:** Rust 2021 / Axum；`cargo check` + `cargo test --lib` 验证；CI 全局 `RUSTFLAGS=-Dwarnings`（warning 即 error）。

---

## 关键约束（每个任务都适用，务必先读）

1. **零业务改动**：只移动函数、调可见性、改 `use`/路径。函数体一个字符都不改。
2. **`-Dwarnings` 是硬门**：移动后任何未使用的 `use`、未使用的 `pub(super)` 项都会让 CI 红。每搬一个子文件，`use` 块按该文件实际用到的符号精确裁剪；`cargo check` 出现 warning 必须当 error 处理、当场修。
3. **基线数字恒定**：`cargo test --lib` 拆分前先记录基线数字（如 `N passed`），拆分全程每步都应保持 `N passed, 0 failed`——数字若变化即说明测试丢失/重复，是 bug。
4. **对外路径不变**：`routes::knowledge::xxx` 与 `routes::ext_knowledge::xxx` 必须保持可达，靠 `mod.rs` 的 `pub use` 兜住；`routes/mod.rs` 的导入块与 `ext_knowledge` 块**零改**。
5. **逐域跳跑**：每完成一个子文件，`cargo check` + `cargo test --lib` 全绿后再做下一个。不要一次性搬完再编译。
6. **提交节奏**：全部子域绿 + orphan 数组更新后，统一提交（遵循"未经允许不 commit"，提交前先征得同意）。

## 子文件归属总表

| 子文件 | 内容（handler / helper / 迁入的测试） |
|---|---|
| `mod.rs` | 子模块声明 + `pub use` 汇总 + **跨域共享 helper**（见下）+ 这些 helper 的单测 |
| `crud.rs` | `list/create/get/update/delete_operation_knowledge[_document/_chunk]`、`list_operation_knowledge_document_chunks`、`get_operation_knowledge_chunk_source`；3 个 Query/Request struct |
| `verify.rs` | `verify_operation_knowledge_chunk`、`reject_*`、`auto_verify_*`(+inner)、`auto_verify_budget_limits`、`decide_auto_verify_status`、`budget_document`、`doc_i64/i32_with_default`；测试：`verified_when_*`/`needs_review_when_*`/`passes_through_rejected`/`unknown_model_status_*`/`auto_verify_default_*` |
| `import.rs` | `import_operation_knowledge_preview/apply/apply_pdf/apply_image`、`import_pdf_bytes`、`ingest_chunked_text`、`extract_operation_knowledge_tags`、`VisionProvider`、`IngestOutcome`、`ImportApplyImageRequest`、`OperationKnowledgeImportRequest`、`ExtractKnowledgeTagsRequest`、`IngestOutcome` |
| `catalog.rs` | `get_operation_knowledge_catalog[_persisted]`、`build_operation_knowledge_catalog`、`load_operation_knowledge_chunks_for_query`、`get/refresh_operation_knowledge_completeness`、`build_operation_knowledge_completeness`、`get_operation_knowledge_integrity_report`、`search_operation_knowledge_tool`、`open_operation_knowledge_slices`、`test_operation_knowledge_match`、`clamp_answering_mode`、`merge_completeness_gaps`；测试：`clamp_answering_mode_*`、`merge_completeness_gaps_*` |
| `repair.rs` | `propose_chunk_repair`、`answer_chunk_repair`、`propose_pack_repair`、`record_repair_apply`、`parse_repair_response`、`write_repair_usage_log`、`record_repair_event`、`classify_extras_kind`、`format_repair_apply_summary`、`truncate_for_prompt`、`chunk_verify_gate_reason`、`fuzzy_locate_quote`、`normalize_for_anchor`；测试：`parse_repair_response_*`、`classify_extras_kind_*`、`format_repair_apply_summary_*`、`chunk_verify_gate_*`(8 个) |
| `chat.rs` | `chat_turn`、`chat_apply`、`chat_history`、`chat_discard`、`allocate_next_turn_indices`、`load_chat_history`、`write_chat_turn`、`chat_turn_to_view`、`synthesize_natural_reply_from_patch`、`run_chat_turn_pipeline`、`render_chat_history_for_prompt`、`render_operator_memory_for_prompt`、`augment_chat_system_with_tools`、`run_chat_with_tools`、`source_anchor_for_quote_ffi`、`update_operator_memory_for_chat`、`classify_intent`、`draft/update_chunk_for_chat`、`update_pack_for_chat`、`clarify_for_chat`、`dispatch_digest_action_for_chat`、`apply_create/update_chunk`、`apply_update_pack`、`chunk_request_from_chat_patch`、`default_chunk_patch_source`、`chat_task_create/get/cancel`、`chat_session_stream`；struct：`ChatAttachment`、`ChatTurnRequest`、`ChatApplyRequest` |
| `digest_inbox.rs` | `digest_today/regenerate/dismiss_card`、`knowledge_inbox`、`serialize_digest_report`、`severity_to_priority`、`digest_action_to_actions`、`digest_kind_to_inbox_kind`、`priority_rank`、`inbox_pending_review_priority`；测试：`inbox_*`(7 个) |
| `wiki_edit.rs` | `patch/archive/restore/rollback_operation_knowledge_chunk`、`list_operation_knowledge_chunk_revisions`、`split/merge_operation_knowledge_chunk`、`relate/unrelate_operation_knowledge_chunk`、`batch_verify_chunks`、`batch_archive_chunks`、`list_chunk_referrers`、`bson_from_json`、`json_object_to_document`、`revision_applied_to_json`、`parse_warning_to_json`、`default_merge_strategy`；struct：`ChunkReferrersQuery`、`ChunkBatchVerifyRequest`、`ChunkBatchArchiveRequest` |
| `sources_meta.rs` | `list/create/update/delete_ingest_source`、`list/dismiss/apply/sweep_knowledge_gap_signals`、`knowledge_aggregate_metadata`、`list_knowledge_usage`、`analyze_operation_knowledge_logs`、`ask_knowledge`、`ask_knowledge_stream`、`knowledge_metrics`、`list_operator_memory`、`split_csv`；struct：`GapSignalListQuery`、`GapSignalResolutionRequest`、`IngestSourceCreateRequest`、`IngestSourceUpdateRequest` |

## 跨域共享 helper（上提到 `mod.rs`，标 `pub(super)`）

按调用点分布，这些被 2+ 子域复用，集中放 `mod.rs` 顶部，子文件经 `super::fn` 调用。它们的单测也跟到 `mod.rs` 的 `#[cfg(test)] mod tests`：

| helper | 调用点数 | 跟随的测试 |
|---|---|---|
| `json_string_list` | 36 | `json_string_list_*`(3) |
| `normalize_operation_domain` | 15 | `normalize_operation_domain_*`(3) |
| `source_anchor_for_quote` | 11 | `source_anchor_for_quote_*`(3) |
| `normalize_knowledge_tags` | 10 | `normalize_knowledge_tags_*`(3) |
| `apply_chunk_integrity` | 10 | — |
| `stable_text_hash` | 9 | `stable_text_hash_*`(1) |
| `default_user_operations_domain` | 9 | — |
| `validate_operation_knowledge_chunk` | 8 | — |
| `operation_knowledge_chunk_from_request` | 8 | — |
| `build_line_index` | 5 | `build_line_index_*`(1) |
| `validate_operation_knowledge_document` | 4 | — |
| `string_bson_array` | 4 | — |
| `operation_knowledge_document_from_request` | 4 | — |
| `integrity_report_for_preview` | 4 | `preview_anchor_match_never_auto_verifies`、`preview_claim_without_source_is_rejected` |
| `build_section_index` | 4 | — |
| `split_lines` | 3 | `split_lines_*`(1) |
| `normalize_operation_knowledge_preview_item` | 2 | — |
| `normalize_operation_knowledge_preview_document` | — | — |
| `default_operation_knowledge_preview_document` | — | — |
| `normalize_operation_knowledge_preview_chunk` | — | — |
| `default_mixed_business_type` / `default_manual_source_type` / `default_imported_markdown_source_type` / `default_active_status` | serde default | — |

> 注：上表 helper 含 5 个 `default_*`（serde `#[serde(default=...)]` 引用）+ preview 归一族。它们与 CRUD/import 强相关但被多处 `from_request`/`preview` 共用，统一放 `mod.rs` 最稳，避免 `super::` 双向依赖。`mod.rs` 因此约 700–900 行，是合理的"共享内核"。

## 搬运通用步骤（每个子域任务都按这 6 步，后文不再重复正文）

> **可见性铁律（Task 2/3 执行中发现，所有域必须照做）**：子文件位于 `routes::knowledge`，比 mod.rs 深一级。原 mod.rs 里的 `pub(super)`（= `pub(in crate::routes)`）若照抄进子文件会窄一级（只到 `routes::knowledge`），导致 `routes/mod.rs` import 不到。**解法**：
> - 子文件里：原 `pub(super)` 对外 handler → `pub(in crate::routes)`；原 `pub`/`pub fn`（ext_knowledge 直调）→ **保持 `pub`**；原私有 → 保持私有。
> - mod.rs 的 re-export 二选一，**按该域是否含真正 `pub` 项决定**：
>   - 域内**全是** `pub(in crate::routes)` 项（无 ext_knowledge 直调，如 crud）→ 写 `pub(in crate::routes) use S::*;`（裸 `pub use` 会报 "glob import doesn't reexport anything"）。
>   - 域内**含真正 `pub` 项**（被 ext_knowledge 直调，如 verify/import/catalog/repair/chat/wiki_edit/sources_meta）→ 写裸 `pub use S::*;`。**不能**用 `pub(in crate::routes) use`——受限 glob 无法把私有子模块里的 `pub` 项提升到模块外，会报 E0364/E0365 "X is private, and cannot be re-exported"。此时裸 `pub use` 因 glob 非空不会触发 unused warning。
>
> **字段可见性推论（Task 4 发现）**：若某 struct 搬进子文件，但留在 mod.rs 的共享 helper 仍按 `&Struct` 读它的字段，会报 E0616（父模块看不到子模块私有字段）。解法：把该 struct 被父模块读取的字段标 `pub(super)`（恰好暴露给父模块 `routes::knowledge`，不外泄）。纯可见性调整，零逻辑改动。
>
> **`use` 起手式（Task 2 验证可行）**：子文件头部用 `use super::*;` 一次性拉入 mod.rs 的共享 struct/helper，使被搬函数体内的裸名调用无需逐个加前缀、函数体一字不改；再补 `use super::super::shared::*;`、`use super::super::AppState;`、以及 axum/mongodb/serde 等按需 use。`use super::*;` 通常已覆盖共享层，故第 3 步的"逐个改 `super::`"多数情况可省。

对每个子文件 `S`：
1. **建文件骨架**：`src/routes/knowledge/S.rs` 顶部写文件级 `//!` 注释 + `use super::*;` + `use super::super::shared::*;` + `use super::super::AppState;` + 按需的外部 crate use。
2. **移动函数/struct**：用 `git show HEAD:src/routes/knowledge.rs` 或读当前 mod.rs 对应行段，把归属表里属于 `S` 的函数整段剪切过去；**函数体一字不改**；`pub(super)` → `pub(in crate::routes)`，`pub` 保持。
3. **改路径**：有了 `use super::*;`，被搬代码调用 mod.rs 共享项一般无需改前缀；仅当出现命名冲突或 glob 未覆盖时按编译错误补 `super::`。
4. **裁剪 `use`**：删掉 `S.rs` 里该文件实际不用的 `use`（`-Dwarnings` 会抓 unused import）。
5. **迁测试**：把归属表标注的 `#[test]` 从 mod.rs 的 `mod tests` 剪到 `S.rs` 末尾的 `#[cfg(test)] mod tests { use super::*; ... }`。
6. **验证**：`cargo check 2>&1 | grep -E "warning|error"` 必须空；`cargo test --lib` 必须 `=899 passed, 0 failed`（基线，已实测）。绿了才做下一域。

---

## Task 0：建目录骨架 + 记录基线

**Files:**
- Create: `src/routes/knowledge/mod.rs`（先空骨架）
- 暂留: `src/routes/knowledge.rs`（逐域搬空后在 Task 11 删）

- [ ] **Step 1: 记录拆分前基线数字**

Run: `cargo test --lib 2>&1 | tail -3`
预期：记下形如 `test result: ok. N passed; 0 failed`。**这个 N 是后续每步的不变量**。同时跑：
`cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter 2>&1 | grep "test result"`
记下 PBT 累计数（应 ≥ 33）。

- [ ] **Step 2: 确认 routes 下无 `knowledge/` 目录冲突**

Run: `ls src/routes/knowledge* 2>&1`
预期：只有 `src/routes/knowledge.rs`，无同名目录。

> 说明：Rust 不允许 `knowledge.rs` 与 `knowledge/mod.rs` 同时存在（模块路径冲突）。本计划策略：**先把 `knowledge.rs` 改名为 `knowledge/mod.rs`**，原内容原封不动搬进去，编译应立即通过（路径等价），再从 `mod.rs` 往各子文件搬。

- [ ] **Step 3: 把 `knowledge.rs` 移动为 `knowledge/mod.rs`**

```bash
mkdir -p src/routes/knowledge
git mv src/routes/knowledge.rs src/routes/knowledge/mod.rs
```
（用 `git mv` 保留 blame 链。）

- [ ] **Step 4: 修正 `mod.rs` 内部的 `include_str!` 自引用**

`knowledge/mod.rs` 内若有 `include_str!` 引用同目录文件需改相对路径；本文件无，跳过。但 `routes/mod.rs:843` 的 `include_str!("knowledge.rs")` 现在路径失效——**本步暂不改**（留到 Task 11 统一处理，此刻它会编译错）。

为让 Task 0 自身可编译验证，本步临时把 `routes/mod.rs:843` 的 `include_str!("knowledge.rs")` 改为 `include_str!("knowledge/mod.rs")`：

Modify: `src/routes/mod.rs:843`
```rust
            include_str!("knowledge/mod.rs"),
```

- [ ] **Step 5: 验证等价改名后全绿**

Run: `cargo check 2>&1 | grep -E "warning|error" | head`
预期：空（无 warning/error）。
Run: `cargo test --lib 2>&1 | tail -3`
预期：`N passed; 0 failed`（与 Step 1 完全一致）。

> 此刻：物理上已是 `knowledge/mod.rs`，逻辑零变化，基线不变。后续 Task 逐步把函数从 `mod.rs` 搬到兄弟子文件。

---

## Task 1：搬共享 helper 到 mod.rs 顶部 + 声明子模块

**Files:**
- Modify: `src/routes/knowledge/mod.rs`

- [ ] **Step 1: 在 mod.rs 顶部声明 9 个子模块并预留 re-export**

在 `mod.rs` 的 `use` 块之后、第一个函数之前插入：
```rust
mod crud;
mod verify;
mod import;
mod catalog;
mod repair;
mod chat;
mod digest_inbox;
mod wiki_edit;
mod sources_meta;

pub use crud::*;
pub use verify::*;
pub use import::*;
pub use catalog::*;
pub use repair::*;
pub use chat::*;
pub use digest_inbox::*;
pub use wiki_edit::*;
pub use sources_meta::*;
```
（此刻子文件还不存在，会编译错——所以本步先只写注释占位，真正放开 `mod`/`pub use` 是在对应子文件建好后逐行解开。**实操：每建好一个子文件 S，就解开 `mod S;` 和 `pub use S::*;` 两行。**）

- [ ] **Step 2: 确认共享 helper 留在 mod.rs**

"跨域共享 helper" 表里的 26 个项（`json_string_list`、`normalize_operation_domain`、`source_anchor_for_quote`、`normalize_knowledge_tags`、`apply_chunk_integrity`、`stable_text_hash`、`default_user_operations_domain`、`validate_operation_knowledge_chunk`、`operation_knowledge_chunk_from_request`、`build_line_index`、`validate_operation_knowledge_document`、`string_bson_array`、`operation_knowledge_document_from_request`、`integrity_report_for_preview`、`build_section_index`、`split_lines`、`normalize_operation_knowledge_preview_item/document/chunk`、`default_operation_knowledge_preview_document`、`default_mixed_business_type`、`default_manual_source_type`、`default_imported_markdown_source_type`、`default_active_status`）**保持原位不动**，可见性确认为 `pub(super)`（已是）。

- [ ] **Step 3: 共享 helper 的单测留在 mod.rs**

这些测试留在 `mod.rs` 的 `mod tests`（搬其它域测试时不要误带走）：
`json_string_list_*`(3)、`normalize_operation_domain_*`(3)、`source_anchor_for_quote_*`(3)、`normalize_knowledge_tags_*`(3)、`stable_text_hash_*`(1)、`build_line_index_*`(1)、`split_lines_*`(1)、`preview_anchor_match_never_auto_verifies`、`preview_claim_without_source_is_rejected`。

- [ ] **Step 4: 本任务无独立验证**（随 Task 2 起逐域验证）

---

## Task 2：搬 crud.rs

**Files:** Create `src/routes/knowledge/crud.rs`；Modify `src/routes/knowledge/mod.rs`

按"搬运通用步骤"6 步执行。本域内容：

- **handler（标 `pub(super)`）**：`list_operation_knowledge`、`list_operation_knowledge_documents`、`create_operation_knowledge_document`、`get_operation_knowledge_document`、`update_operation_knowledge_document`、`delete_operation_knowledge_document`、`list_operation_knowledge_chunks`、`list_operation_knowledge_document_chunks`、`create_operation_knowledge_chunk`、`update_operation_knowledge_chunk`、`delete_operation_knowledge_chunk`、`get_operation_knowledge_chunk_source`、`create_operation_knowledge`、`update_operation_knowledge`、`delete_operation_knowledge`
- **struct（保留在 mod.rs，不随 crud 迁移）**：`OperationKnowledgeQuery`、`OperationKnowledgeDocumentQuery`、`OperationKnowledgeChunkQuery`、`OperationKnowledgeDocumentRequest`、`OperationKnowledgeRequest`、`OperationKnowledgeChunkRequest` — 执行时核实：这 6 个被 import/catalog/chat 多域引用（`OperationKnowledgeChunkRequest` 15 处、`OperationKnowledgeDocumentRequest` 6 处），留在 mod.rs 作共享类型，crud.rs 经 `super::` 引用
- **对共享 helper/struct 的调用**改 `super::`：`validate_operation_knowledge_*`、`operation_knowledge_*_from_request`、`normalize_operation_domain`、`default_*`、`json_string_list`、`normalize_knowledge_tags`、以及上面 6 个 struct
- **迁移测试**：本域无独占单测（CRUD 由集成测试覆盖），`mod tests` 不迁
- **解开 mod.rs**：`mod crud;` + `pub use crud::*;`

验证：`cargo check 2>&1 | grep -E "warning|error"` 空；`cargo test --lib` = 基线 N。

---

## Task 3：搬 verify.rs

**Files:** Create `src/routes/knowledge/verify.rs`；Modify `mod.rs`

- **handler（`pub`：被 ext_knowledge 直调）**：`verify_operation_knowledge_chunk`、`auto_verify_operation_knowledge_chunks`、`decide_auto_verify_status`（`pub fn`）
- **handler（`pub(super)`）**：`reject_operation_knowledge_chunk`
- **内部 fn**：`auto_verify_operation_knowledge_chunks_inner`、`auto_verify_budget_limits`、`doc_i64_with_default`、`doc_i32_with_default`
- **保留在 mod.rs（不搬）**：`budget_document` — 执行时核实跨 verify/catalog/repair 三域调用（9 处），留 mod.rs 作共享 helper，verify.rs 经 `super::` 引用
- **struct**：`KnowledgeVerifyRequest`(`pub`)、`KnowledgeAutoVerifyRequest`(`pub`)
- **迁移测试**（剪到 verify.rs 的 `mod tests`）：`verified_when_all_evidence_present_and_confident`、`needs_review_when_source_quote_missing`、`needs_review_when_source_anchor_missing`、`needs_review_when_confidence_below_threshold`、`passes_through_rejected_status`、`unknown_model_status_falls_back_to_needs_review`、`auto_verify_default_call_cap_is_not_run_max_llm_calls_six`、`auto_verify_default_token_budget_is_not_simulation_60000`
- 调用 `super::` 共享 helper：`apply_chunk_integrity`、`source_anchor_for_quote` 等
- 解开 `mod verify; pub use verify::*;`

验证同上。

---

## Task 4：搬 import.rs

**Files:** Create `src/routes/knowledge/import.rs`；Modify `mod.rs`

- **handler（`pub`：ext_knowledge 直调）**：`import_operation_knowledge_preview`、`extract_operation_knowledge_tags`、`import_pdf_bytes`、`import_operation_knowledge_apply_image`、`ingest_chunked_text`
- **handler（`pub(super)`）**：`import_operation_knowledge_apply`、`import_operation_knowledge_apply_pdf`
- **struct/enum**：`OperationKnowledgeImportRequest`(197, `pub`)、`OperationKnowledgeImportApplyRequest`(205)、`ExtractKnowledgeTagsRequest`(1384, `pub`)、`ImportApplyImageRequest`(1707, `pub`)、`VisionProvider`(1725, enum)、`IngestOutcome`(1912, `pub`)
- **迁移测试**：本域无独占单测（preview 锚点红线测试归 mod.rs 共享层，因为测的是 `integrity_report_for_preview`）
- 调用 `super::`：`normalize_operation_knowledge_preview_*`、`integrity_report_for_preview`、`apply_chunk_integrity`、`source_anchor_for_quote`、`build_line_index`、`build_section_index`、`stable_text_hash`、`string_bson_array`、`normalize_operation_domain`、`json_string_list` 等
- 解开 `mod import; pub use import::*;`

> ⚠️ import 域是共享 helper 调用最密集的域，`super::` 改写点最多。逐个核对编译错误提示，缺一个补一个。

验证同上。

---

## Task 5：搬 catalog.rs

**Files:** Create `src/routes/knowledge/catalog.rs`；Modify `mod.rs`

- **handler（`pub`：ext_knowledge 直调）**：`build_operation_knowledge_completeness`
- **handler（`pub(super)` → `pub(in crate::routes)`）**：`get_operation_knowledge_catalog`、`get_operation_knowledge_catalog_persisted`、`get_operation_knowledge_completeness`、`refresh_operation_knowledge_completeness`、`get_operation_knowledge_integrity_report`、`search_operation_knowledge_tool`、`open_operation_knowledge_slices`、`test_operation_knowledge_match`、`build_operation_knowledge_catalog`
- **保留在 mod.rs（不搬）**：`load_operation_knowledge_chunks_for_query` — 执行时核实被**已搬走的 crud.rs**（168/178）和 catalog 两域共用，必须留 mod.rs 作共享 helper，否则 crud.rs 的 `use super::*;` 够不到。catalog.rs 经 `use super::*;` 调用它。
- **内部 fn**：`clamp_answering_mode`、`merge_completeness_gaps`
- **struct**：`KnowledgeToolSearchRequest`、`KnowledgeToolOpenRequest`、`OperationKnowledgeTestRequest`（均 `pub(super)` → 留 mod.rs 还是随 catalog？执行时按引用核实——若仅 catalog 用则随迁并改 `pub(in crate::routes)`）
- **迁移测试**：`clamp_answering_mode_demotes_fully_supported_when_drafts_pending`、`clamp_answering_mode_never_upgrades_weaker_modes`、`merge_completeness_gaps_keeps_deterministic_floor_when_llm_empty`、`merge_completeness_gaps_unions_deterministic_then_llm_extra`、`merge_completeness_gaps_dedups_and_drops_empty`
- 调用 `super::`：`load_*`(域内自调保留裸名)、`apply_chunk_integrity`、`json_string_list`、`normalize_operation_domain` 等
- 解开 `mod catalog; pub use catalog::*;`

验证同上。

---

## Task 6：搬 repair.rs

**Files:** Create `src/routes/knowledge/repair.rs`；Modify `mod.rs`

- **handler（`pub`：ext_knowledge 直调）**：`propose_chunk_repair`
- **handler（`pub(super)` → `pub(in crate::routes)`）**：`answer_chunk_repair`、`propose_pack_repair`、`record_repair_apply`
- **内部 fn（随 repair 迁）**：`parse_repair_response`、`write_repair_usage_log`、`record_repair_event`、`classify_extras_kind`、`format_repair_apply_summary`
- **保留在 mod.rs（不搬，执行时核实跨域）**：
  - `chunk_verify_gate_reason` — 被 verify.rs(74)、chat 域(5507) 调用，留 mod.rs
  - `truncate_for_prompt` — 被 chat/digest 多域(1471/1638/2862/2887/3191/3368) 调用，留 mod.rs
  - `fuzzy_locate_quote`、`normalize_for_anchor` — 被 mod.rs 共享 `source_anchor_for_quote` 调用，留 mod.rs
- **struct**：`ChunkRepairAnswerBody`、`ChunkRepairAnswer`、`RepairApplyBody`（均 `pub(super)` → `pub(in crate::routes)`，执行时核实仅 repair 用）
- **迁移测试（8 个，仅 repair 自有）**：`parse_repair_response_normalizes_string_missing_fields`、`parse_repair_response_passes_through_object_missing_fields`、`parse_repair_response_caps_followup_questions_to_three`、`parse_repair_response_clamps_confidence_to_0_100`、`parse_repair_response_handles_garbage_input`、`classify_extras_kind_handles_all_shapes`、`format_repair_apply_summary_contains_target_and_counts`、`format_repair_apply_summary_has_no_forbidden_words`
- **留在 mod.rs 的测试（8 个 `chunk_verify_gate*`）**：因 `chunk_verify_gate_reason` 留 mod.rs，其 8 个测试（`chunk_verify_gate_passes/blocks_*` 4 个 + `chunk_verify_gate_reason_*` 4 个）也留 mod.rs，不迁 repair
- 解开 `mod repair;` + 裸 `pub use repair::*;`（含 pub 项 propose_chunk_repair）

验证同上。

---

## Task 7：搬 chat.rs（最大域）

**Files:** Create `src/routes/knowledge/chat.rs`；Modify `mod.rs`

- **handler（`pub`：ext_knowledge 直调）**：`chat_turn`、`chat_apply`
- **handler（`pub(super)`）**：`chat_history`、`chat_discard`、`chat_task_create`、`chat_task_get`、`chat_task_cancel`、`chat_session_stream`、`allocate_next_turn_indices`
- **内部 fn**：`load_chat_history`、`write_chat_turn`、`chat_turn_to_view`、`synthesize_natural_reply_from_patch`、`run_chat_turn_pipeline`、`render_chat_history_for_prompt`、`render_operator_memory_for_prompt`、`augment_chat_system_with_tools`、`run_chat_with_tools`、`source_anchor_for_quote_ffi`、`update_operator_memory_for_chat`、`classify_intent`、`draft_chunk_for_chat`、`update_chunk_for_chat`、`update_pack_for_chat`、`clarify_for_chat`、`dispatch_digest_action_for_chat`、`apply_create_chunk`、`apply_update_chunk`、`apply_update_pack`、`chunk_request_from_chat_patch`、`default_chunk_patch_source`
- **struct**：`ChatAttachment`(4215, `pub`)、`ChatTurnRequest`(4222, `pub`)、`ChatApplyRequest`(4556, `pub`)、`ChatTaskCreateRequest`(6205)
- **迁移测试**：本域无独占单测（chat 由 e2e/集成测试覆盖）
- 调用 `super::`：`source_anchor_for_quote`、`apply_chunk_integrity`、`operation_knowledge_chunk_from_request`、`json_string_list` 等
- 解开 `mod chat; pub use chat::*;`

> chat.rs 预估 ~1900 行，函数最多。建议分两批 Read/移动（4215–5500、5500–6460）以免一次处理过大。

验证同上。

---

## Task 8：搬 digest_inbox.rs

**Files:** Create `src/routes/knowledge/digest_inbox.rs`；Modify `mod.rs`

- **handler（`pub(super)`）**：`digest_today`、`digest_regenerate`、`digest_dismiss_card`、`knowledge_inbox`
- **内部 fn**：`serialize_digest_report`、`severity_to_priority`、`digest_action_to_actions`、`digest_kind_to_inbox_kind`、`priority_rank`、`inbox_pending_review_priority`
- **struct**：`DigestTodayQuery`(6052)、`DigestRegenerateRequest`(6100)、`InboxQuery`(6507)、`InboxCardView`(6515)、`InboxStats`(6530)、`InboxResponse`(6539)
- **迁移测试**：`inbox_severity_to_priority_three_buckets`、`inbox_pending_review_priority_lifts_negative_example`、`inbox_pending_review_priority_keeps_other_chunk_types_mid`、`inbox_digest_kind_mapping_is_total_for_known_kinds`、`inbox_action_mapping_always_offers_dismiss`、`inbox_priority_rank_orders_high_first`、`inbox_sort_places_high_priority_first`、`inbox_static_strings_have_no_forbidden_words`
- 解开 `mod digest_inbox; pub use digest_inbox::*;`

验证同上。

---

## Task 9：搬 wiki_edit.rs

**Files:** Create `src/routes/knowledge/wiki_edit.rs`；Modify `mod.rs`

- **handler（`pub`：ext_knowledge 直调）**：`batch_verify_chunks`、`batch_archive_chunks`、`list_chunk_referrers`
- **handler（`pub(super)`）**：`patch_operation_knowledge_chunk`、`archive_operation_knowledge_chunk`、`restore_operation_knowledge_chunk`、`rollback_operation_knowledge_chunk`、`list_operation_knowledge_chunk_revisions`、`split_operation_knowledge_chunk`、`merge_operation_knowledge_chunk`、`relate_operation_knowledge_chunk`、`unrelate_operation_knowledge_chunk`
- **内部 fn**：`bson_from_json`、`json_object_to_document`、`revision_applied_to_json`、`parse_warning_to_json`、`default_merge_strategy`
- **struct**：`ChunkPatchRequest`(6837)、`ChunkArchiveRequest`(6853)、`ChunkRollbackRequest`(6860)、`ChunkRevisionsQuery`(7115)、`ChunkSplitRequest`(7182)、`ChunkMergeRequest`(7320)、`ChunkRelateRequest`(7499)、`ChunkReferrersQuery`(7679, `pub`)、`ChunkBatchVerifyRequest`(7739, `pub`)、`ChunkBatchArchiveRequest`(7830, `pub`)
- **迁移测试**：本域无独占单测
- 调用 `super::`：`source_anchor_for_quote`、`build_line_index`、`stable_text_hash` 等
- 解开 `mod wiki_edit; pub use wiki_edit::*;`

验证同上。

---

## Task 10：搬 sources_meta.rs

**Files:** Create `src/routes/knowledge/sources_meta.rs`；Modify `mod.rs`

- **handler（`pub`：ext_knowledge 可能直调，保守标 `pub`）**：`list_ingest_sources`、`create_ingest_source`、`update_ingest_source`、`delete_ingest_source`
- **handler（`pub(super)`）**：`list_knowledge_gap_signals`、`dismiss_knowledge_gap_signal`、`apply_knowledge_gap_signal`、`sweep_knowledge_gap_signals`、`knowledge_aggregate_metadata`、`list_knowledge_usage`、`analyze_operation_knowledge_logs`、`ask_knowledge`、`ask_knowledge_stream`、`knowledge_metrics`、`list_operator_memory`
- **内部 fn**：`split_csv`
- **struct**：`AnalyzeLogsQuery`(2140)、`GapSignalListQuery`(8117, `pub`)、`GapSignalResolutionRequest`(8124, `pub`)、`KnowledgeAskRequest`(8228)、`KnowledgeAskFilter`(8242)、`KnowledgeAskStreamQuery`(8320)、`OperatorMemoryQuery`(8505)、`IngestSourceCreateRequest`(8597, `pub`)、`IngestSourceUpdateRequest`(8607, `pub`)
- **迁移测试**：本域无独占单测
- 调用 `super::`：`json_string_list`、`normalize_operation_domain`、`load_operation_knowledge_chunks_for_query`(在 catalog，需 `super::` 不可达——见下注) 等
- 解开 `mod sources_meta; pub use sources_meta::*;`

> ⚠️ 若 `ask_knowledge` 调用了 catalog 域的 `load_operation_knowledge_chunks_for_query`（兄弟模块、非父级），`super::` 不可达。两种解法择一：(a) 把该 fn 上提到 mod.rs；(b) 用 `super::catalog::load_operation_knowledge_chunks_for_query`（因 mod.rs 内 `mod catalog;` 对兄弟可见）。**推荐 (b)**，零额外移动。执行时按编译错误提示确定到底有没有跨兄弟调用。

验证同上。

---

## Task 11：删原引用 + 更新 orphan 测试数组

**Files:** Modify `src/routes/mod.rs`

此刻 `knowledge/mod.rs` 应只剩共享 helper + `mod`/`pub use` + 共享层 `mod tests`，9 个子文件已建好。

- [ ] **Step 1: 确认 mod.rs 已搬空业务 handler**

Run: `grep -nE "^[[:space:]]*pub(\(super\))?[[:space:]]+async[[:space:]]+fn" src/routes/knowledge/mod.rs`
预期：空或仅剩极少数（理想为空——所有 async handler 都已进子文件）。

- [ ] **Step 2: 更新 `routes/mod.rs:843` 的 orphan 测试 `route_files` 数组**

把 Task 0 Step 4 临时改的 `include_str!("knowledge/mod.rs")` 一行，替换为 9 个子文件 + mod.rs：
```rust
            include_str!("knowledge/mod.rs"),
            include_str!("knowledge/crud.rs"),
            include_str!("knowledge/verify.rs"),
            include_str!("knowledge/import.rs"),
            include_str!("knowledge/catalog.rs"),
            include_str!("knowledge/repair.rs"),
            include_str!("knowledge/chat.rs"),
            include_str!("knowledge/digest_inbox.rs"),
            include_str!("knowledge/wiki_edit.rs"),
            include_str!("knowledge/sources_meta.rs"),
```
（保持数组其它行不变；`KNOWN_NON_ROUTE_HANDLERS` 内 3 个名字 `ingest_chunked_text`/`import_pdf_bytes`/`build_operation_knowledge_completeness` 保持不变。）

- [ ] **Step 3: 跑 orphan 测试**

Run: `cargo test --lib no_orphan_pub_async_route_handlers -- --nocapture`
预期：PASS。若报某 handler 未挂载，核对它是否真在 `api_router` 里 `.route` 或该进 `KNOWN_NON_ROUTE_HANDLERS`。

---

## Task 12：全量验证 + 自检

- [ ] **Step 1: 全量编译（warning 即 error）**

Run: `cargo check 2>&1 | grep -E "warning|error" | head -30`
预期：空。任何 unused import / unused `pub(super)` 必须清掉。

- [ ] **Step 2: lib 测试数与基线一致**

Run: `cargo test --lib 2>&1 | tail -3`
预期：`N passed; 0 failed`，N 与 Task 0 Step 1 记录的**完全一致**。不一致 → 有测试漏迁或重复，回查。

- [ ] **Step 3: PBT 四件套**

Run: `cargo test --test state_transition_pbt --test memory_card_invariants --test wiki_chunk_revision_pbt --test llm_retry_jitter 2>&1 | grep "test result"`
预期：累计 ≥ 33 passed, 0 failed。

- [ ] **Step 4: 基线门脚本**

Run: `bash scripts/check-baseline.sh`
预期：exit 0（lib ≥ 350、PBT ≥ 33 双门绿）。

- [ ] **Step 5: 文件行数体检**

Run: `for f in src/routes/knowledge/*.rs; do echo "$(wc -l < "$f") $f"; done | sort -rn`
预期：最大（chat.rs）≤ ~2000，其余 ≤ ~1200，无 9000 行巨файл。

- [ ] **Step 6: 确认对外路径未变**

Run: `cargo test --test real_llm_knowledge --test import_pdf_smoke --test chunk_batch_ops 2>&1 | grep -E "error\[|cannot find|unresolved" | head`
预期：空（这些测试通过 `routes::ext_knowledge::xxx` / `routes::knowledge::xxx` 直调，编译通过即证明 facade 路径完好）。注：这些是 `#[ignore]` 集成测试，本地可能因 Docker/磁盘跳过运行，但**编译必须过**。

---

## Self-Review 记录

- **Spec 覆盖**：spec 4 个风险点 →（1）可见性断裂=各 Task 的 `pub`/`pub(super)` 标注；（2）ext_knowledge 生命线=Task 12 Step 6 验证；（3）orphan 数组=Task 11；（4）54 单测分发=Task 3/5/6/8 + mod.rs 共享层，已逐个列名。
- **占位扫描**：无 TBD/TODO；每个函数/struct 给了精确名与行号。
- **类型一致性**：子模块名（crud/verify/import/catalog/repair/chat/digest_inbox/wiki_edit/sources_meta）全文一致；`pub use S::*` 与 `mod S` 一一对应。
- **测试计数自检**：迁出测试 verify(8)+catalog(5)+repair(16)+digest_inbox(8)=37，留 mod.rs 共享层 17，合计 54，与原 `mod tests` 总数一致。


