# 外部源列表 serde 大小写修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `GET /api/knowledge/ingest-sources` 列表返回 camelCase 键，使前端 `IngestSourceItem` 能读到 `sourceId`/`scheduleMinutes` 等，修复删除/重新激活按钮失效与「间隔」列 `undefinedm`。

**Architecture:** 不给存储结构体 `IngestSource` 加 `#[serde(rename_all="camelCase")]`（会连带改坏 bson 存储键、worker 查询、两条索引、全部存量文档）。改为在 API 输出边界抽一个私有函数 `ingest_source_json(src: &IngestSource) -> Value`，逐字段手写 camelCase `json!({...})`，完全仿照同 crate 已有的 `operation_knowledge_chunk_json`（`src/routes/knowledge/mod.rs:283-317`）。`list_ingest_sources` 把 `serde_json::to_value(src)` 换成调这个函数。存储层、worker、索引、前端零改动。

**Tech Stack:** Rust 2021 / Axum / serde_json / mongodb bson。测试用 `cargo test --lib`。

## Global Constraints

以下为全项目硬约束，每个 Task 隐含遵守（值逐字取自 spec 与 CLAUDE.md）：

- **红线中的红线**：改任何代码前必须 100% 读懂相关代码路径，所有 `file:line` 引用当场用 Read/Grep 亲验，绝不猜测。本计划所有代码块的行号/字段/签名均已在写计划时亲验（见每 Task 的「已亲验事实」）。
- **禁止 struct 级 rename_all**：`IngestSource`（`src/models.rs:1875-1909`）同时是 `Collection<IngestSource>` 的 bson 存储类型，字段以 snake_case 存 Mongo。加 `rename_all` 会破坏 `ingest_worker.rs` 的 snake_case 查询/更新、两条 snake_case 索引、全部存量文档。修复只在 API JSON 输出边界做 snake→camel。
- **基线门不得回归**：`cargo test --lib` 当前 ≥350 passed / 0 failed，只加不减。`scripts/check-baseline.sh`（或 `.ps1`）双门必须绿。
- **check-no-human-takeover lint**：CI 扫 `git diff` 新增行（`src/routes/` 等目录），禁词 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。本改纯字段映射，注释与代码均不得踩这些词。
- **回复语言**：对用户的对话回复用中文。代码、标识符、commit message 遵循现有约定。
- **commit 授权**：未经用户显式许可不得 `git commit`。计划中的 commit 步骤须在用户授权后执行；未授权时把该步理解为「暂存/待授权」，不实际提交。
- **部署授权**：改完 + 单测绿后才部署到 117；部署走 paramiko 脚本，须用户在场知会。

---

## File Structure

- **`src/routes/knowledge/sources_meta.rs`**（现 1024 行）——唯一被修改的源文件。
  - 新增私有函数 `ingest_source_json(src: &IngestSource) -> Value`（放在 `list_ingest_sources` 之前，:886 上方）。
  - 修改 `list_ingest_sources` 的循环体一行（:899）。
  - 文件末尾新增 `#[cfg(test)] mod tests`（当前无测试区，grep 零命中）。
- **不改动**：`src/models.rs`（`IngestSource` 结构体）、`src/knowledge_wiki/ingest_worker.rs`、`src/db/indexes.rs`、`src/db/mod.rs`、`frontend/src/features/knowledge/steward.tsx`。

**测试路线决定（已亲验）**：本 crate 的 API 投影测试有两种范式——契约快照（`assert_contract_fixture`，读 `frontend/src/contracts/<name>.fixture.json`）和纯手写 `assert_eq!`。经查 `frontend/src/contracts/` 33 个 fixture **无 `ingest_source`**，列表接口历来不在契约快照体系内；引入 fixture 需 bless + 前端 vitest 键集对账，对一个字段映射 bug 属过度基建。**故本计划用纯手写 `assert_eq!` 断言**（对照 `mod.rs:1477` `preserve_unmodeled_chunk_fields_*` 的手写断言范式），零副作用。

---

## Task 1: 抽 `ingest_source_json` 投影函数 + 修 list handler + 单测

**Files:**
- Modify: `src/routes/knowledge/sources_meta.rs`（新增函数于 :886 前；改 :899 一行；文件末尾加 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes:
  - `crate::models::IngestSource`（`src/models.rs:1876`）字段（全部亲验）：`id: Option<ObjectId>`(`#[serde(rename="_id")]`)、`source_id: String`、`workspace_id: String`、`kind: String`、`url: String`、`schedule_minutes: i64`、`label: Option<String>`、`last_fetched_at: Option<DateTime>`、`last_etag: Option<String>`、`last_error: Option<String>`、`status: String`、`failure_streak: i32`、`ingest_count: i64`、`created_at: DateTime`、`updated_at: DateTime`。
  - `crate::models::dt_to_string(dt: DateTime) -> Option<String>`（`src/models.rs:3608`，入参**非 Option**，返回 RFC3339 `Option<String>`）。
  - `serde_json::{json, Value}`（文件顶部 :13 已 `use`，无需新增 import）。
- Produces:
  - `fn ingest_source_json(src: &crate::models::IngestSource) -> Value`（私有，同模块内 `list_ingest_sources` 调用）。签名用**全路径** `crate::models::IngestSource`——已亲验 `mod.rs:17-19` 的 `models::{...}` re-export 只含 `KnowledgeChatTurn/KnowledgeUsageLog/OperationKnowledgeChunk/OperationKnowledgeDocument`，**不含 `IngestSource`**；且本文件现有代码（`:924` 建 row、`:359` `KnowledgeGapSignal`）一律用全路径，无裸类型名先例。

**已亲验事实（非猜测，写计划时用 Read 确认）：**
- `list_ingest_sources`（:886-902）现循环体第 :899 行 = `items.push(serde_json::to_value(src).map_err(|e| AppError::External(e.to_string()))?);` — 这是 snake_case 泄漏点。
- `operation_knowledge_chunk_json`（`mod.rs:283-317`）范式：`json!({...})` camelCase 键；ObjectId 用 `item.id.map(|id| id.to_hex())`；非 Option DateTime 用 `crate::models::dt_to_string(item.updated_at)`（:316）；Option DateTime 用 `item.valid_from.and_then(crate::models::dt_to_string)`（:308）。
- `create_ingest_source`（:947）已返回 `json!({ "sourceId": source_id })` = camelCase（反证，证明前端读 camelCase）。
- 前端 `IngestSourceItem`（`steward.tsx:1840-1854`）声明字段（camelCase）：`sourceId`/`workspaceId`/`kind`/`url`/`scheduleMinutes`/`label?`/`lastFetchedAt?`/`lastEtag?`/`lastError?`/`status`/`failureStreak`/`ingestCount`/`createdAt`/`updatedAt`——**无 `id` 字段**（故投影不输出 `id`）。

- [ ] **Step 1: 在文件末尾追加失败单测**

在 `src/routes/knowledge/sources_meta.rs` 末尾（第 1024 行之后）追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::IngestSource;
    use mongodb::bson::{oid::ObjectId, DateTime};

    fn sample_source() -> IngestSource {
        IngestSource {
            id: Some(ObjectId::parse_str("64a1f2c3e4b5a6978899d001").unwrap()),
            source_id: "ing_abc123".to_string(),
            workspace_id: "ws-1".to_string(),
            kind: "rss".to_string(),
            url: "https://example.com/feed".to_string(),
            schedule_minutes: 60,
            label: Some("行业资讯源".to_string()),
            last_fetched_at: Some(DateTime::from_millis(1_700_000_000_000)),
            last_etag: Some("\"etag-xyz\"".to_string()),
            last_error: None,
            status: "active".to_string(),
            failure_streak: 2,
            ingest_count: 7,
            created_at: DateTime::from_millis(1_699_000_000_000),
            updated_at: DateTime::from_millis(1_700_000_100_000),
        }
    }

    #[test]
    fn ingest_source_json_emits_camel_case() {
        let v = ingest_source_json(&sample_source());

        // 前端读的 camelCase 键必须存在且值正确
        assert_eq!(v.get("sourceId").and_then(|x| x.as_str()), Some("ing_abc123"));
        assert_eq!(v.get("workspaceId").and_then(|x| x.as_str()), Some("ws-1"));
        assert_eq!(v.get("kind").and_then(|x| x.as_str()), Some("rss"));
        assert_eq!(v.get("url").and_then(|x| x.as_str()), Some("https://example.com/feed"));
        assert_eq!(v.get("scheduleMinutes").and_then(|x| x.as_i64()), Some(60));
        assert_eq!(v.get("label").and_then(|x| x.as_str()), Some("行业资讯源"));
        assert_eq!(v.get("lastEtag").and_then(|x| x.as_str()), Some("\"etag-xyz\""));
        assert_eq!(v.get("status").and_then(|x| x.as_str()), Some("active"));
        assert_eq!(v.get("failureStreak").and_then(|x| x.as_i64()), Some(2));
        assert_eq!(v.get("ingestCount").and_then(|x| x.as_i64()), Some(7));
        assert!(v.get("lastFetchedAt").and_then(|x| x.as_str()).is_some(), "lastFetchedAt 应为 RFC3339 串");
        assert!(v.get("createdAt").and_then(|x| x.as_str()).is_some(), "createdAt 应为 RFC3339 串");
        assert!(v.get("updatedAt").and_then(|x| x.as_str()).is_some(), "updatedAt 应为 RFC3339 串");
        // lastError=None → null
        assert!(v.get("lastError").map(|x| x.is_null()).unwrap_or(false), "lastError 应为 null");

        // 绝不能再泄漏 snake_case 键（这些是 bug 的根源）
        assert!(v.get("source_id").is_none(), "不得含 snake_case source_id");
        assert!(v.get("schedule_minutes").is_none(), "不得含 snake_case schedule_minutes");
        assert!(v.get("last_fetched_at").is_none(), "不得含 snake_case last_fetched_at");
        assert!(v.get("failure_streak").is_none(), "不得含 snake_case failure_streak");
        assert!(v.get("ingest_count").is_none(), "不得含 snake_case ingest_count");
        assert!(v.get("created_at").is_none(), "不得含 snake_case created_at");
        // 前端接口无 id 字段，投影不输出 id
        assert!(v.get("id").is_none(), "投影不应输出 id（前端 IngestSourceItem 无此字段）");
    }

    #[test]
    fn ingest_source_json_null_datetime() {
        let mut src = sample_source();
        src.last_fetched_at = None;
        let v = ingest_source_json(&src);
        assert!(v.get("lastFetchedAt").map(|x| x.is_null()).unwrap_or(false),
            "last_fetched_at=None → lastFetchedAt 应为 null");
    }
}
```

- [ ] **Step 2: 运行测试确认失败（函数未定义）**

Run: `cargo test --lib ingest_source_json`
Expected: 编译失败，`cannot find function \`ingest_source_json\` in this scope`（因函数尚未定义）。

- [ ] **Step 3: 在 `list_ingest_sources` 上方新增投影函数**

在 `src/routes/knowledge/sources_meta.rs` 第 886 行 `pub async fn list_ingest_sources(` 之前插入：

```rust
/// 把存储结构体 `IngestSource`（snake_case bson）投影成前端 `IngestSourceItem`
/// 期望的 camelCase JSON。仿 `operation_knowledge_chunk_json`（mod.rs:283）——
/// 存储层不加 rename_all（会破坏 worker 查询/索引/存量文档），只在 API 边界映射。
fn ingest_source_json(src: &crate::models::IngestSource) -> Value {
    json!({
        "sourceId": src.source_id,
        "workspaceId": src.workspace_id,
        "kind": src.kind,
        "url": src.url,
        "scheduleMinutes": src.schedule_minutes,
        "label": src.label,
        "lastFetchedAt": src.last_fetched_at.and_then(crate::models::dt_to_string),
        "lastEtag": src.last_etag,
        "lastError": src.last_error,
        "status": src.status,
        "failureStreak": src.failure_streak,
        "ingestCount": src.ingest_count,
        "createdAt": crate::models::dt_to_string(src.created_at),
        "updatedAt": crate::models::dt_to_string(src.updated_at)
    })
}
```

- [ ] **Step 4: 运行单测确认通过**

Run: `cargo test --lib ingest_source_json`
Expected: `test result: ok. 2 passed; 0 failed`（`ingest_source_json_emits_camel_case` + `ingest_source_json_null_datetime` 两条绿）。（投影函数签名已用全路径 `crate::models::IngestSource`，`mod.rs` 的 `models::{...}` re-export 不含 `IngestSource`，无裸类型名可用；测试模块内已显式 `use crate::models::IngestSource;`。）

- [ ] **Step 5: 把 list handler 的泄漏行换成调投影函数**

在 `src/routes/knowledge/sources_meta.rs` 把第 899 行：

```rust
        items.push(serde_json::to_value(src).map_err(|e| AppError::External(e.to_string()))?);
```

改为：

```rust
        items.push(ingest_source_json(&src));
```

（`ingest_source_json` 不返回 Result，去掉 `?` 与 `map_err`。循环变量 `src` 是 `IngestSource`，传 `&src`。）

- [ ] **Step 6: `cargo check` 确认整体编译 + 无 unused import 警告**

Run: `cargo check --lib`
Expected: `Finished`，无 error。注意：若 `serde_json::to_value` 是本文件唯一使用点、改后 `use serde_json::{json, Value};` 里的 `json`/`Value` 仍被投影函数使用（`json!`/`Value`），不会 unused；`AppError` 仍被 handler 其它处使用。若出现 unused 警告，按提示清理。

- [ ] **Step 7: 跑全量 lib 测试确认基线不回归**

Run: `cargo test --lib`
Expected: `test result: ok. N passed; 0 failed`，N ≥ 350（比修改前多 2 条，即新增的两个单测）。

- [ ] **Step 8: 暂存改动（commit 待用户授权）**

改动仅 `src/routes/knowledge/sources_meta.rs` 一个文件。**未经用户显式许可不 commit**。用户授权后执行：

```bash
git add src/routes/knowledge/sources_meta.rs
git commit -m "$(cat <<'EOF'
fix(ingest-sources): 列表 API 输出 camelCase 修复删除/激活/间隔显示失效

IngestSource 是 bson 存储结构体(snake_case),列表 handler 直接
serde_json::to_value 泄漏 snake_case 键,前端读 camelCase sourceId 恒
undefined→删除/重新激活打不中真实 source_id、间隔列 undefinedm。抽
ingest_source_json 投影函数在 API 边界映射 camelCase(仿
operation_knowledge_chunk_json),存储层/worker/索引零改动。

Co-Authored-By: Claude <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: 部署到 117 + 真实 UI 回归验证

**Files:** 无源码改动（部署 + 浏览器验证）。

**Interfaces:**
- Consumes: Task 1 产出的 `ingest_source_json`（已合入 `sources_meta.rs`）。
- Produces: 生产 117 上 `GET /api/knowledge/ingest-sources` 返回 camelCase 的实证。

**已亲验事实：**
- 部署脚本 `scripts/_push_bundle_direct.py` / `scripts/_remote_run_direct.py`（memory 记录），须 `MSYS_NO_PATHCONV=1 PYTHONUTF8=1 DEPLOY_BIND_IP=192.168.5.9`，凭据在 `~/.wa_deploy_env`，server 117.72.54.28，app 端口 3003，密码绝不回显。
- 纯后端改动——前端**无需 rebuild**（`frontend/dist` 不变）。
- 常驻 CDP 浏览器（port 9222）真实点击，`p.stop()` 不关窗（不破坏连贯性）。

- [ ] **Step 1: 本地二次确认门（部署前）**

Run: `cargo test --lib && cargo check --lib`
Expected: 全绿。这是部署前最后一道本地门。

- [ ] **Step 2: 征得用户部署授权**

向用户确认：「Task 1 单测绿，准备部署到 117。纯后端改动，前端无需 rebuild。是否现在部署？」等用户明确同意再执行 Step 3。

- [ ] **Step 3: 推送 bundle 到 117 并重启**

授权后执行（密码绝不回显；命令读 `~/.wa_deploy_env`）：

```bash
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 DEPLOY_BIND_IP=192.168.5.9 python scripts/_push_bundle_direct.py
MSYS_NO_PATHCONV=1 PYTHONUTF8=1 DEPLOY_BIND_IP=192.168.5.9 python scripts/_remote_run_direct.py "systemctl restart wechatagent && sleep 3 && systemctl is-active wechatagent"
```

Expected: 构建成功、`systemctl is-active` 输出 `active`。

- [ ] **Step 4: API 直核 camelCase（curl/浏览器 fetch）**

在常驻 CDP 浏览器已登录会话里执行 fetch（或用 e2e 脚本 `page.evaluate`）：

```js
async () => { const r = await fetch('/api/knowledge/ingest-sources'); const j = await r.json(); return j.items?.[0] ?? j; }
```

Expected: 若列表非空，首行含 `sourceId`（非 `undefined`）、`scheduleMinutes` 为数字、`createdAt` 为 RFC3339 串；不含 `source_id`/`schedule_minutes` 等 snake_case 键。若列表为空，先建一个 `[E2E验证]` 源再核。

- [ ] **Step 5: 真实 UI 回归——新建 → 间隔列 → 删除生效**

写一个 e2e 脚本（复用 `scripts/e2e/wiki_verify_common.py` 的 `attach()`/`ResponseCapture`，仿 `wiki_verify_T2T3_data.py` 的外部源段），常驻 CDP 真实点击：

1. 控制台 → 外部源，新建 `[E2E验证]` 源（RSS，间隔 60，URL `https://example.com/e2e-verify-camel`）。
2. 断言列表新行「间隔」列显示 `60m`（**非 `undefinedm`**）——这是 bug 的直接症状，修复后必须消失。
3. 点该行「删除」→ 断言弹出「删除外部源？」danger 弹窗 → 确认删除。
4. 断言删除后重新 `GET /api/knowledge/ingest-sources`，该 `sourceId` 不再出现（**证明 DELETE 打中了真实 source_id 而非 `/undefined`**）。
5. 截图存证 `scripts/e2e/camel_fix_*.png`。

Expected: 间隔列显示 `60m`；删除后行消失、API 确认不残留。（对比修复前：间隔 `undefinedm`、删除点了无效。）

- [ ] **Step 6: 清理测试数据 + 回报**

确认 Step 5 已删除测试源（列表回干净态，无 `[E2E验证]` 残留）。向用户回报：API camelCase 实证 + UI 删除/间隔列修复实证 + 截图路径。

---

## Self-Review

对照 spec（`docs/superpowers/specs/2026-07-13-ingest-source-serde-camelcase-fix-design.md`）逐条核查：

**1. Spec coverage：**
- spec「修法：抽 `ingest_source_json` 逐字段 camelCase，改 list handler 一行」→ Task 1 Step 3/6 ✓
- spec「字段映射表 15 字段」→ Task 1 Step 3 投影函数逐字段覆盖（`id` 按 spec 省略）✓
- spec「不给 struct 加 rename_all，存储层零改动」→ Global Constraints + File Structure「不改动」清单 ✓
- spec「2 个单测：`ingest_source_json_emits_camel_case` + `ingest_source_json_null_datetime`」→ Task 1 Step 1 ✓
- spec「基线门 cargo test --lib 不回归 + check-no-human-takeover」→ Task 1 Step 8 + Global Constraints ✓
- spec「验证+部署：单测绿→无需 rebuild→paramiko 推+重启→真实 UI 回归（删除/间隔列/sourceId）」→ Task 2 全部 ✓
- spec「commit/PR 待用户显式许可」→ Task 1 Step 9 + Global Constraints ✓
- spec「全仓扫查结论：IngestSource 是唯一真漏点」→ 无需额外 Task（spec 已亲验，本计划不重复扫）✓

**2. Placeholder scan：** 无 TBD/TODO；所有代码块为完整可抄内容；测试代码含真实断言；部署命令为真实命令。Task 1 Step 4 是条件性 fallback（类型可见性实测），非占位符——给出了明确的判定条件与两种写法。

**3. Type consistency：** 函数名 `ingest_source_json` 在 Step 1（测试调用）、Step 3（定义）、Step 6（handler 调用）、Task 2 一致；签名 `(src: &IngestSource) -> Value` 一致（Step 4 fallback 用全路径 `&crate::models::IngestSource` 是同一类型的不同写法，不影响调用方）；`dt_to_string(dt: DateTime) -> Option<String>` 用法与 `mod.rs:308/316` 一致（Option 字段 `.and_then`，非 Option 字段直接调）。

无 gap，无需补 Task。
