# 实施计划：异步 import job + 进度轮询

日期：2026-07-12
Spec：`docs/superpowers/specs/2026-07-12-async-import-job-progress-design.md`（已获批）

自底向上分 7 阶段，每阶段可独立编译+验证。红线：分块/合并/D2锚定逻辑搬进共享 `run_import_extraction` **一字不动**；apply `draft`/`needs_review` 不变；并发度 `IMPORT_EXTRACT_CONCURRENCY=2` 不动；status 闭集拒未知值；IDOR 按 workspace 收口。

---

## 阶段 0：抽出共享抽取函数（零回归重构，先做）

**目标**：把 `import.rs::import_operation_knowledge_preview`（当前 `src/routes/knowledge/import.rs:262-366`）里内联的 split→`buffered(2)`→merge document/items/chunks→`integrity_report_for_preview` 整段抽成纯函数，同步路径立即复用它验证等价。

- 在 `import.rs` 新增 `pub(super) async fn run_import_extraction(state: &AppState, content: &str, source_name: &str, account_id: Option<&str>) -> AppResult<Value>`，返回当前响应体 `{document, items, chunks, integrityReport, importReport}`。
- 把 `import_operation_knowledge_preview` 改为：校验 content 非空 → 调 `run_import_extraction(...)` → `Ok(Json(result))`。行为**字节等价**（含小文档单段路径）。

**验证**：`cargo check` 通过；`cargo test --test <import相关>` 或 `cargo test --lib knowledge::import` 现有分块单测全绿（证明搬迁无回归）。

---

## 阶段 1：数据模型 `ImportJob`

**文件**：`src/models.rs`（仿 `AgentTask` @ `models.rs:829-857`）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub source_name: String,
    pub content: String,
    pub segments_total: i32,
    #[serde(default)] pub progress_done: i32,
    #[serde(default)] pub progress_succeeded: i32,
    #[serde(default)] pub progress_failed: i32,
    pub status: String, // pending|running|completed|failed（闭集）
    #[serde(default, skip_serializing_if = "Option::is_none")] pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub claimed_at: Option<DateTime>,
    #[serde(default)] pub claim_recovery_count: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

**status 闭集**：加 `const IMPORT_JOB_STATUSES: [&str;4] = ["pending","running","completed","failed"];` + 写库处校验（worker/endpoint 写 status 前 assert 属闭集，未知值 → `AppError::Internal`，对齐 gateway status 红线）。

**验证**：`cargo check`。

---

## 阶段 2：DB accessor + index

- `src/db/mod.rs`：加 `pub fn import_jobs(&self) -> Collection<ImportJob> { self.db.collection("import_jobs") }`（仿 `db/mod.rs:82 tasks()`）。
- `src/db/indexes.rs`：在 `ensure_all` 加两条 `create_index`（仿现有 IndexModel::builder 块）：
  - `{workspace_id:1, status:1}`（前端跨会话发现）
  - `{status:1, claimed_at:1}`（worker 认领 + 孤儿重认领）

**验证**：`cargo check`；本地起服务看 `ensure_indexes` 无报错（或 testcontainers 集成测跑 index 创建）。

---

## 阶段 3：config 间隔

**文件**：`src/config.rs`（仿 `config.rs:38/439`）

- struct 加 `pub import_worker_interval_seconds: u64,` + `pub import_job_claim_timeout_seconds: u64,`
- 构造处加 `env_or("IMPORT_WORKER_INTERVAL_SECONDS","2").parse()?` + `env_or("IMPORT_JOB_CLAIM_TIMEOUT_SECONDS","600").parse()?`（600s=10min，覆盖 ~9min 墙钟 + 余量）
- `.env.example` 补两行 + 注释

**无 enable flag**（spec 决定，常开对齐 `tasks.rs`）。

**验证**：`cargo check --tests`（config 加字段须补全所有 AppConfig 构造点，否则 E0063——见 [[config-field-add-test-helpers]]）。

---

## 阶段 4：worker `import_worker.rs`

**文件**：新 `src/import_worker.rs`（仿 `tasks.rs:12 run_task_worker` + `reclaim_stale_running_tasks` + claim + 心跳）

- `pub async fn run_import_worker(state: AppState)`：loop { reclaim_stale → claim_one → 若有则跑 → sleep(interval) }
- `reclaim_stale`：`{status:"running", claimed_at:{$lt: now - timeout}}` → 重置回 `pending`（`claim_recovery_count` +1；≥3 次直接 `failed`，仿 tasks.rs:65-91）
- `claim_one`：`find_one_and_update({status:"pending"}, {$set:{status:"running", claimed_at:now}})`
- 跑 job：调**阶段0的 `run_import_extraction`**，但**分段级进度**——需在 `run_import_extraction` 里加可选进度回调，或 worker 侧自己 split 后逐段调（见下「进度粒度」）
- 心跳：跑抽取期间每 `timeout/3` 秒 bump `claimed_at`（仿 `spawn_claim_heartbeat` tasks.rs:225）
- 收尾：全成/部分成 → `{status:"completed", result}`；全失败 → `{status:"failed", error}`
- `main.rs`：`spawn_supervised(state.clone(), "import_worker", |s| async move { wechatagent::import_worker::run_import_worker(s).await; });`（仿 `main.rs:206`）
- `lib.rs`：`pub mod import_worker;`

**进度粒度决策**：`run_import_extraction` 加参数 `progress: Option<impl Fn(done,succeeded,failed)>`。同步路径传 `None`；worker 传闭包，每段 `buffered` 完成时回写 job 的 `progress_*`。这样分块/合并逻辑仍不动，只在段完成点插一个回调。

**验证**：`cargo check`；单测 job 状态机流转 + reclaim。

---

## 阶段 5：端点

**文件**：`src/routes/knowledge/import.rs` + `src/routes/mod.rs`（route 注册仿 `mod.rs:651`）

- 改 `import_operation_knowledge_preview`：
  - `content.chars().count() ≤ IMPORT_SINGLE_CALL_MAX_CHARS` → 同步调 `run_import_extraction(None)` 秒回（今天行为，无 jobId）
  - 否则 → 建 `import_jobs` 文档（`segments_total = split_import_content(&content).len()`，`status="pending"`）→ 返回 `{jobId, async:true, segmentsTotal}`
- 新 `get_import_preview_job(Path(id), Extension(admin))`：查 job，**校验 `workspace_id == admin.current_workspace`**（IDOR 收口，仿现有 admin handler workspace 隔离）→ 返回 `{status, progress:{done,total,succeeded,failed}, result?, error?}`
- 新 `list_import_preview_jobs(Query, Extension(admin))`：`{workspace_id: admin.current_workspace, status:"running"}` → 列表
- `mod.rs` 加两条 route：`GET /operation-knowledge/import-preview-job/:id`、`GET /operation-knowledge/import-preview-jobs`

**验证**：`cargo check`；集成测（testcontainers）：大文档 POST 返 jobId；跨 workspace 取 job 被拒(IDOR)。

---

## 阶段 6：前端 `steward.tsx`

**文件**：`frontend/src/features/knowledge/steward.tsx`（当前无轮询，`runPreview` @ ~line 661）

- `runPreview`：POST 后 `if (data.jobId) { 进轮询态 } else { 走今天同步逻辑 setPreview + setStep(2) }`
- 轮询：`useRef` 存 interval id，每 2s GET `/import-preview-job/:id`：
  - `running` → 更新进度文案（"导入进行中 {done}/{total} 段"）
  - `completed` → `setPreview(result)` + `setStep(2)` + 清 interval
  - `failed` → 显错 + 重试 + 清 interval
- 跨会话恢复：进导入向导时 GET `/import-preview-jobs?status=running`，有则接管轮询态（**不用 localStorage**）
- 组件卸载 `useEffect` cleanup 清 interval
- UI 遵守 `docs/frontend-design-system.md`，进度态复用现有 loading 视觉

**验证**：`cd frontend && npm run build` 通过；本地 `npm run dev` 或 117 端到端。

---

## 阶段 7：端到端 + 收口

**生产 117 亲验**（脚本走 `_remote_run_direct.py`，前台跑防 SIGHUP，见 [[project-prod-llm-provider-db-not-env]]）：
1. 大文档（29KB 星零感）POST `/import-preview` → 秒回 `{jobId}`；轮询 `/import-preview-job/:id` 从 running 走到 completed，result.chunks 非 0
2. 小文档（<3000字符）POST → 同步秒回，无 jobId（零回归）
3. 模拟切走：跑到一半 GET `/import-preview-jobs?status=running` 能找回 job
4. apply 结果仍 draft/needs_review（红线）
5. 清理测试产生的 job + doc + chunks

**基线门**：`scripts/check-baseline.ps1` 不回退（新测只增量叠加）；`check-no-human-takeover` 绿（新代码措辞避禁词）。

---

## 影响文件清单

- 新增：`src/import_worker.rs`、spec 已列
- 改：`src/routes/knowledge/import.rs`（run_import_extraction + preview 分流 + 2 端点）、`src/routes/mod.rs`（2 route）、`src/models.rs`（ImportJob）、`src/db/mod.rs`（accessor）、`src/db/indexes.rs`（2 index）、`src/config.rs`+`.env.example`（2 字段）、`src/main.rs`（spawn worker）、`src/lib.rs`（mod）、`frontend/src/features/knowledge/steward.tsx`
- 无破坏性迁移；小文档路径零回归
