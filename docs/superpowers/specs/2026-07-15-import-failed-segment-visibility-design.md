# 导入失败段可见性(M1) + 召回 haystack 补 product_tags(M4) 设计

> 日期：2026-07-15
> 状态：设计已获批，待写实现计划
> 关联审查：[[project_wiki_extraction_classification_audit_fix]]（M1=其中的 M1；M4=其中的 M4）；交叉验证见本会话（M1=CONFIRMED，M4=OVERSTATED）

## 背景与缺陷

知识库导入：管理员粘贴长文本，后端把文本分段（segment），每段独立调 LLM 抽取成结构化 chunk。

- **小文档**（≤ `IMPORT_SINGLE_CALL_MAX_CHARS`，`import.rs:274`）→ 同步秒回。`split_import_content`（`import.rs:147-148`）对 ≤MAX 恒返回**单段**，单段失败在 `import.rs:476` `values.is_empty()` 直接 `Err` → **failed>0 在同步路径不可达**。
- **大文档**（>MAX）→ 建 `import_jobs`（pending），`import_worker`（`src/import_worker.rs`）异步分段并发抽取，前端每 2s 轮询。**这是 M1 真实触发路径**。

### M1（CONFIRMED，纯前端缺陷）

后端数据链**已完全就绪**（本次修复后端零改动，均已 file:line 亲验）：

- `import_worker.rs:161-163`：drainer 实时把 `progress_done/succeeded/failed` 写进 job（`max_done` 守单调）。
- `import_worker.rs:201-220`：部分失败仍是 `completed`（只要 `values` 非空即 `Ok`），`result` 存 `run_import_extraction` 完整返回值 `result_bson`，**含 `importReport{totalSegments,succeeded,failed}`**（`import.rs:512-516`）。全失败才 `failed` 态（`import_worker.rs:242`）。
- `import.rs:368-380` `import_job_progress_json`：把 `progress.succeeded/failed` **和** `result`（含 importReport）**全都返回**给前端。

缺陷在前端 `frontend/src/features/knowledge/steward.tsx`：

- 进度渲染只用 `done/total`（`steward.tsx:877-878`），从不读 `failed`。
- `status==="completed"` 时 `setJobProgress(null)` 丢弃进度（`steward.tsx:742`），`acceptPreviewResult(data.result)` 只取 `result.chunks`（`steward.tsx:711-718`）。
- `ImportPreviewResult` interface 不声明 `importReport`（`steward.tsx:604-608`）；`ImportJobProgress.failed` 有字段（:614）但无任何组件渲染。

**后果**：导入 >MAX 字符长文本且某段 LLM 抽取失败时，进度条照显"已完成 N/N 段"、完成后只列存活 chunk，用户全程看不到"X 段失败"，误以为内容完整而实际缺失。（会话 A 第 4 段曾真实触发，见 [[project_wiki_extraction_classification_audit_fix]]。）

### M4（OVERSTATED，低概率软缺陷，补上无害）

`chunk_haystack`（`knowledge_agent.rs:1886-1906`）构造召回检索文本时拼了 `title/summary/body/business_topics/wiki_type`，**独漏 `product_tags`**——与同族字段 `business_topics` 不一致。`rank_key`（`knowledge_agent.rs:1666`）`relevance_score(query, chunk_haystack(chunk))` 只吃这个 haystack，故 product_tags 完全不参与 `list_catalog` 召回打分。

**为何 OVERSTATED**：抽取 prompt（`import.rs:584`）强制"productTags 只放正文里**确实出现的**产品名，没有就留空"——tag 词面天然取自 body，而 body 已在 haystack，客户报产品名时 body 侧已能命中打分，多数自愈。真实漏召仅限"管理员手工加的 tag / 品牌别名，词面不在正文"的窄场景。属低概率一致性遗漏，补上无害且不违反过拟合红线（纯补检索信号）。

## 用户裁决（本次设计）

1. **展示形态**：预览页（step2）顶部非阻断黄色警示条。
2. **是否阻断 apply**：不阻断，仅警示（成功段仍全是 draft，需逐条确认后 AI 才用）。
3. **数据源**：以 `result.importReport` 为唯一权威源贯穿 step2（不用轮询 `progress.failed`——那是运行中快照，完成后该由 result 接管）。
4. 范围：M1 + M4 一并修。

## 架构

M1 后端零改动，纯前端消费；M4 后端一行 + 单测。两者独立无耦合。

### 组件 1 — M1 前端类型（`steward.tsx:604-608`）

`ImportPreviewResult` interface 补可选字段，对齐后端 `import_job_progress_json` 已输出的 `result` 形状：

```ts
interface ImportPreviewResult {
  document?: { title?: string; summary?: string; catalogSummary?: string } | null;
  items?: unknown[];
  chunks?: ImportPreviewChunk[];
  importReport?: { totalSegments: number; succeeded: number; failed: number };
}
```

用可选（`?`）：同步路径后端也返回 importReport（`import.rs:512`，单段 failed 恒 0），但保守用可选防未来路径缺失 / 旧 job result。

### 组件 2 — M1 警示条渲染（step2 预览页顶部）

step2 区块（`step === 2 && preview` 时）顶部读 `preview.importReport`，仅 `failed > 0` 才渲染一条警示（复用现有 `wikiAlert` 样式类，与 `steward.tsx:848` 的 error 条同族，但用 warning 语气而非 error）：

> ⚠ 共 {totalSegments} 段，{failed} 段抽取失败，下方仅为成功段内容，可能不完整。

`failed === 0` 或 `importReport` 缺失 → 不渲染（可选链兜底，零回归）。不阻断「应用」按钮。

### 组件 3 — M4 后端 haystack（`knowledge_agent.rs` `chunk_haystack` 内，business_topics 循环后）

```rust
for t in &c.business_topics { s.push(' '); s.push_str(t); }
for t in &c.product_tags { s.push(' '); s.push_str(t); }   // 新增，与 business_topics 并列
```

位置紧跟 business_topics，语义对称。

## 数据流

- **M1**：worker 写 `result`（含 importReport，`import_worker.rs:213`）→ job 端点返回 `result`（`import.rs:378`）→ 前端轮询 `completed` → `acceptPreviewResult(data.result)` 存 preview → step2 读 `preview.importReport.failed` 渲染警示条。
- **M4**：`list_catalog` → `rank_key` → `relevance_score(query, chunk_haystack)`，haystack 现含 product_tags。

## 错误处理 / 边界

- `importReport` 缺失 / `undefined` → 警示条不显（可选链兜底）。
- **全失败**已是 job `failed` 态（走 `steward.tsx:744` error 分支，不进 step2）；警示条只覆盖"部分失败仍 completed"。
- **同步单段路径** failed 恒 0 → 警示条永不显示，字节等价零回归。
- **M4** product_tags 空 Vec → haystack 不变，零回归。

## 测试

- **M1（前端）**：TS 编译通过 + `cd frontend && npm run build`。前端无单测框架；后端数据链已由契约测试 `import_job_progress_json_matches_contract_fixture`（`import.rs:1342`）钉死 `result`/`progress.failed` 形状，故前端侧靠类型 + 构建 + 渲染逻辑人工核对。
- **M4（后端）**：新增 lib 单测 `chunk_haystack_includes_product_tags`——构造 product_tags 非空的 chunk，断言 haystack 字符串含该 tag 词；再断言 `rank_key(该tag, chunk_with_tag, now)` 的 `effective_relevance_micros` > 同 chunk 但 product_tags 清空时（证明 tag 确实进了打分）。
- **基线门**：`cargo test --lib` ≥350/0、`RUSTFLAGS=-D warnings cargo check --tests`、4 PBT ≥33/0、3 lint（no-human-takeover / no-model-hint / evolution-isolation）。

## 非目标（YAGNI）

- 不改后端任何 job / worker / API（M1 数据已就绪）。
- 不做进度条实时显失败数（用户已选仅 step2 警示）。
- 不改 H3（PDF/RSS 落 raw blob，有意设计）、M2/M3/M5/H5（M5 已随 D2 修复，其余属未采纳的质量增强）。
- 不给 product_tags 加 DB filter / catalog 回传（仅补 haystack 打分信号，最小改动）。
