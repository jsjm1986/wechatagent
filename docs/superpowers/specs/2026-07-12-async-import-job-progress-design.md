# 异步 import job + 进度轮询设计（改善长文档导入等待体验）

日期：2026-07-12
状态：已获用户口头批准，待实现
前置：`2026-07-12-long-doc-chunked-import-design.md`（后端自动分块已上线，功能正确但 preview 整篇 29KB 墙钟 ~9 分钟）

## 问题

长文档 `POST /api/operation-knowledge/import-preview` 已能正确分块抽取（不截断、chunks 非 0），但整篇 29KB 文档需 5 段抽取、每段输出 ~6k tokens、`buffered(2)` 下跑 3 波串行，受限端点 ~25-30 tok/s → **总墙钟 ~9 分钟**。前端当前是同步 `fetch` 死等，真实用户盯 9 分钟 spinner，且切走/刷新页面就丢失整个进度、只能重来。

## 目标与非目标

**目标（用户已选「改等待体验」路线）**：
- preview 不再让前端同步死等；提交后立即返回，前端轮询进度（"导入进行中 3/5 段"）。
- job 可恢复：存 DB，用户切走/刷新后重开向导能找回进行中的 job 继续看；后端进程重启也不丢 job（worker 接管）。

**非目标（明确 out of scope）**：
- **不提速**。并发 `IMPORT_EXTRACT_CONCURRENCY=2` 一字不动，端点不换。"提速"是用户没选的另一条路。
- 小文档（≤ `IMPORT_SINGLE_CALL_MAX_CHARS`=3000 字符，单段，当前同步 ~15-25s 秒回）**保留同步路径**，不引入轮询往返。

## 方案 A：新 `import_jobs` 集合 + 专用 worker

选定方案 A（vs B「提交即 spawn + 孤儿清扫」/ C「复用 agent_tasks」）。理由：断线恢复是硬需求，A 用现有 worker 模式（`tasks.rs` / `outbox_dispatcher` / `account_scheduler`）一比一对齐，语义最干净，不引入 B 的「spawn + 清扫」双路径复杂度；C 会把知识导入 job 塞进用户运营域的 `agent_tasks`，踩 Phase-1 域隔离红线。

### 1. 数据模型 —— 新集合 `import_jobs`

新增 BSON struct（`models.rs`）+ 新 typed accessor `db.import_jobs()`（`db/mod.rs`）+ index 条目（`db/indexes` 或 ensure_indexes）。

字段：
- `_id`
- `workspace_id`、`account_id?`
- `source_name`、`content`（原文，供 worker 抽取 + D2 锚定）
- `segments_total`、`progress_done`、`progress_succeeded`、`progress_failed`
- `status`: `pending | running | completed | failed`（**闭集**，写库处拒未知值，对齐现有 gateway/finalReview status 红线）
- `result?`（完成时嵌入 `{document, items, chunks, integrityReport, importReport}`，即当前同步 preview 的响应体）
- `error?`（失败时）
- `claimed_at?`（断线重认领用，作 worker 心跳时间戳）
- `created_at`、`updated_at`

索引：
- `{workspace_id, status}` —— 前端跨会话发现本 workspace 进行中 job
- `{status, claimed_at}` —— worker 认领 + 孤儿重认领

清扫：`completed` / `failed` job 保留 24h 后删（`result` 可能较大，不长留）。

### 2. Worker —— 新 `import_worker.rs`

- `main.rs` 里 `tokio::spawn` interval 循环（仿 `tasks.rs` follow-up worker），间隔取小值（如 1-2s，导入低频但求启动延迟低）。
- **认领**：`findOneAndUpdate` 原子抢一个 `status=pending`（或 `status=running` 且 `claimed_at` 已超心跳阈值的孤儿）→ 置 `running` + 刷新 `claimed_at`。单 worker、一次一 job（admin 导入低频，够用）。
- **抽取**：调**共享抽取函数** `run_import_extraction(state, content, source_name, account_id) -> AppResult<Value>`。这是把当前 `import.rs::import_operation_knowledge_preview` 内联的 split→`buffered(2)`→merge document/items/chunks→`integrity_report_for_preview`（对完整原文 D2 锚定）整段抽出来的纯逻辑函数。**同步小文档路径也复用它**（单段时行为与今天字节等价）。
  - 每段抽取完成后更新 `progress_done`（及 succeeded/failed 计数）并刷新 `claimed_at` 心跳。
- **收尾**：全成 / 部分成 → `completed` + `result`；全失败 → `failed` + `error`（对齐分块设计「全部段失败才报错」）。
- **断线恢复**：进程重启后，孤儿 `running` job 的 `claimed_at` 已过心跳阈值 → 下一轮 interval 自动重认领重跑（认领粒度是整个 job，不做段级断点续传——简单、够用）。

**无 enable 开关，常开**：这个 worker 是异步导入的必需件（关了功能就废），不像 `INGEST_WORKER_ENABLED`（自动抓 RSS/HTML 是可选部署行为、故 gate）。它 inert 时只是空轮询，与 `tasks.rs` follow-up worker 同级——后者也无 on/off gate、`main.rs` 里无条件 spawn。只加可配间隔 `IMPORT_WORKER_INTERVAL_SECONDS`（`config.rs`，默认 1-2s；写入 `.env.example`），不加 enable flag。

### 3. 端点（`routes/knowledge/`）

- `POST /import-preview`（**现有，契约扩展**）：
  - 小文档（`content.chars().count() ≤ IMPORT_SINGLE_CALL_MAX_CHARS`）→ **原样同步秒回**当前响应体（`{document, items, chunks, integrityReport, importReport}`，无 `jobId`）。
  - 大文档 → 建 `import_jobs` 文档（`status=pending`，`segments_total` 由 `split_import_content` 预先算出）→ 立即返回 `{jobId, async:true, segmentsTotal}`。
- `GET /import-preview-job/:id` → `{status, progress:{done,total,succeeded,failed}, result?, error?}`。前端每 ~2s 轮询。校验 job 属当前 workspace（IDOR 收口，仿现有 admin handler workspace 隔离模式）。
- `GET /import-preview-jobs?status=running` → 本 workspace 进行中的 job 列表（跨会话/跨设备发现用，不依赖 localStorage）。

### 4. 前端（`frontend/src/features/knowledge/steward.tsx`，当前无轮询，从零加）

- `runPreview`：POST `/import-preview` 后，若响应带 `jobId` → 进**轮询态**；否则（小文档）走今天的同步逻辑（响应即 preview，直接进 step2）。
- **轮询**：`setInterval` 每 ~2s GET `/import-preview-job/:id`：
  - `running` → 更新进度文案（"导入进行中 3/5 段"）。
  - `completed` → 用 `result` 填 preview，进 step2；清 interval。
  - `failed` → 显错 + 重试入口；清 interval。
- **跨会话恢复**：进入导入向导时（或粘贴前）调 `GET /import-preview-jobs?status=running`，若有进行中 job → 直接进轮询态接管显示。**按 workspace 查，不用 localStorage 存 jobId**（换浏览器/设备也能找回，更稳——用户已确认此判断）。
- 组件卸载 / 离开向导时清 interval，防泄漏。
- UI 遵守现有设计系统（`docs/frontend-design-system.md`），进度态复用现有 loading/进度视觉，不自创。

### 5. 红线保持 / 明确不做

- 分块 `split_import_content` / 合并 `merge_preview_documents` / D2 锚定 `integrity_report_for_preview` 逻辑**一字不动**，只是被搬进共享的 `run_import_extraction`。
- apply 路径 `status=draft` + `integrity_status=needs_review` 强制不变（"AI 永不自动 verify"）。
- 并发度 `IMPORT_EXTRACT_CONCURRENCY=2` 不动（提速非本 spec）。
- 新 status 枚举闭集，写库处拒未知值。
- CI lint `check-no-human-takeover`：新代码在 `src/routes/` / `frontend/src/` 下，措辞避开禁词，用 AI-internal 状态名。

### 6. 测试

- **单测**：job 状态机流转（pending→running→completed/failed）；`run_import_extraction` 抽取逻辑（复用现有分块单测，验证搬迁后行为等价）。
- **集成测**（testcontainers Mongo，`#[ignore]` 默认，CI 跑）：worker 认领 pending job → 进度更新；孤儿 `running`（`claimed_at` 过期）被重认领；IDOR——跨 workspace 取 job 被拒。
- **端到端**（生产 117）：大文档 POST 秒回 jobId → 轮询到 `completed` 拿 result；小文档仍同步秒回（无 jobId）；模拟切走后 `GET /import-preview-jobs?status=running` 能找回 job 继续看。
- 基线门（`check-baseline`）不回退：新测只增量叠加。

## 影响面

- 后端：新 `import_worker.rs`；`import.rs` 抽出 `run_import_extraction` + 改 `import_operation_knowledge_preview` 分流 + 2 个新 job 端点；`models.rs` 加 `ImportJob` struct；`db/mod.rs` 加 accessor + index；`config.rs` + `.env.example` 加 `IMPORT_WORKER_ENABLED`；`main.rs` 起 worker。
- 前端：`steward.tsx` 加轮询态 + 跨会话恢复。
- 无破坏性 schema 迁移（新集合，不动老数据）。
- 小文档路径行为与今天字节等价（零回归）。
