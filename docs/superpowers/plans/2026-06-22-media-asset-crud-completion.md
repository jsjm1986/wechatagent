# 素材库 CRUD 补全（media asset CRUD completion）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为素材库（content-assets）补齐 edit（改元数据 + 换文件）/ delete（引用计数清理）/ disable（toggle sendable）四个端点 + 前端入口，对齐已完整的专属顾问名片功能。

**Architecture:** 不新建模块。在 `media_assets.rs` 加 4 个 handler + 2 个纯函数；`media_storage.rs` 加 `delete_bytes`；`assets.rs` 的 list 补返回 `sendable`；`mod.rs` 挂 4 路由；前端 `contentStore.ts` 加 4 个 action、`types/index.ts` 给 ContentAsset 加 sendable、`content-assets/index.tsx` 加操作 UI。

**Tech Stack:** Rust (Axum) + MongoDB + React 19 + TypeScript + Vite。设计文档：`docs/superpowers/specs/2026-06-22-media-asset-crud-completion-design.md`。

## Global Constraints

- **AI 不自我核验红线**：换文件 → 强制 `review_status="draft"` 重审（发送物变了必须人类重新核验）。纯改元数据不动 review_status。
- **media_id 一致性**：换文件必清 `media_id=None`——`ensure_media_uploaded`（`media_send.rs:82`）在 TTL 内复用 media_id，不清则 AI 发旧文件。
- **既成事实纪律**：delete / 换文件的物理删文件 fail-soft——删文件失败只 `tracing::warn!`，不回滚 DB、不返 Err。
- **引用计数保护**：物理删文件前必须确认无兄弟记录引用同 file_path（upload 不去重，同文件多记录共享物理文件）。
- **workspace_id scope 防 IDOR**：4 个端点的查询/更新/删除 filter 全带 `workspace_id`（= `admin.current_workspace`）。
- **target_stages 归一**：edit 元数据复用簇 B 的 `crate::agent::dimension_registry::normalize_target_stages`（单一事实源），越界 400。
- **no-human-takeover lint**：`src/routes/`、`frontend/src/` 新增行禁止 `human[_ -]?takeover|takeover|hand[ -]?off|人工接管|人工介入|人工托管|接管|人工`。用"编辑/停用/删除/换文件"中性词。
- **测试基线**：`cargo test --lib` ≥ 350 passed / 0 failed。本地只跑 `cargo test --lib` + 单 PBT（磁盘紧）。新增测试只 append。
- **不改发送门禁**：`validate_asset_sendable`（`media_send.rs:24`）已查 `sendable==Some(true)`，本簇只加写 sendable 的端点，不动门禁。
- **回复语言**：与用户对话用中文；代码 / 标识符 / commit 沿用既有约定。

---

### Task 1: `media_storage::delete_bytes` + `should_delete_physical_file` 纯函数（地基）

**Files:**
- Modify: `src/media_storage.rs`（加 `delete_bytes` 函数 + `should_delete_physical_file` 纯函数；测试加进既有 `mod tests`）

**Interfaces:**
- Produces:
  - `pub async fn delete_bytes(root: &std::path::Path, rel: &str) -> std::io::Result<()>`——物理删文件，文件不存在视为成功（幂等）。Task 3（换文件）、Task 5（delete）消费。
  - `pub fn should_delete_physical_file(remaining_refs: u64) -> bool`——`remaining_refs == 0` 才删。Task 3、Task 5 消费。

- [ ] **Step 1: 写失败测试**

加到 `src/media_storage.rs` 的 `mod tests` 末尾（`}` 之前）：

```rust
    #[test]
    fn should_delete_only_when_no_refs() {
        assert!(should_delete_physical_file(0));
        assert!(!should_delete_physical_file(1));
        assert!(!should_delete_physical_file(5));
    }

    #[tokio::test]
    async fn delete_bytes_removes_file_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("mediadel_{}", sha256_hex(format!("{:?}", std::time::SystemTime::now()).as_bytes())));
        let rel = "ws/ab/abcd.pdf";
        store_bytes(&dir, rel, b"hi").await.unwrap();
        assert!(dir.join(rel).exists());
        // 第一次删：成功，文件消失
        delete_bytes(&dir, rel).await.unwrap();
        assert!(!dir.join(rel).exists());
        // 第二次删（文件已不存在）：幂等，仍 Ok
        delete_bytes(&dir, rel).await.unwrap();
        // 清理
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib should_delete_only_when_no_refs delete_bytes_removes_file`
Expected: FAIL —— `should_delete_physical_file` / `delete_bytes` 未定义，编译错误。

> 注：`cargo test --lib` 接受多个测试名前缀，若报参数问题改为分别跑或用 `cargo test --lib media_storage`。

- [ ] **Step 3: 实现**

加到 `src/media_storage.rs` 的 `read_bytes`（`:83`）之后、`#[cfg(test)]`（`:85`）之前：

```rust
/// 物理删除素材文件。文件不存在（已被删/从未落盘）视为成功——幂等。
/// 调用方须先确认无其它 content_asset 记录引用同 rel（见 should_delete_physical_file）。
pub async fn delete_bytes(root: &Path, rel: &str) -> std::io::Result<()> {
    match tokio::fs::remove_file(root.join(rel)).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// 纯决策：物理删文件前，给定"删本记录后同 file_path 的剩余引用数"，
/// 仅当剩余引用为 0（无兄弟记录共享该物理文件）才可物理删。
/// upload 不去重，同文件多次上传 = 多条记录共享一个 file_path，故必须查引用计数。
pub fn should_delete_physical_file(remaining_refs: u64) -> bool {
    remaining_refs == 0
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --lib media_storage`
Expected: 既有 5 个 + 新增 2 个测试全 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/media_storage.rs
git commit -m "feat(media-asset-crud): delete_bytes 幂等删文件 + should_delete_physical_file 引用计数纯函数(簇C地基)"
```

---

### Task 2: edit 元数据端点 `PUT /content-assets/:id`（JSON）

**Files:**
- Modify: `src/routes/media_assets.rs`（加 `UpdateMetaRequest` 结构 + `update_content_asset_meta` handler）

**Interfaces:**
- Consumes: `crate::agent::dimension_registry::normalize_target_stages(db, scope, &[String]) -> Result<Vec<String>, String>`（簇 B）；`ObjectId::parse_str`；`state.db.content_assets()`。
- Produces: `pub(super) async fn update_content_asset_meta(...)`。Task 6 挂路由。

**背景**：素材上传后运营无法改元数据（错别字的触发提示也得删了重传）。本端点改描述性字段，不碰文件本体、不碰审核态（描述词变不影响已核验文件，故不退 draft）。

- [ ] **Step 1: 实现请求结构 + handler**

在 `src/routes/media_assets.rs` 顶部 import 后加结构（确认 `Document` / `to_bson` 已 import；`media_assets.rs:12` 已有 `use mongodb::bson::{doc, oid::ObjectId, DateTime}`，需补 `Document`）：

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateMetaRequest {
    title: Option<String>,
    body: Option<String>,
    tags: Option<Vec<String>>,
    url: Option<String>,
    usage_scene: Option<String>,
    send_trigger_hint: Option<String>,
    expression_pref: Option<String>,
    target_stages: Option<Vec<String>>,
    requires_principal_approval: Option<bool>,
}
```

handler（加在 `review_media_asset` 之后）：

```rust
/// PUT /content-assets/:id —— 改元数据（JSON，部分更新）。
/// 只 $set 客户端提供的字段；不动 file_*/media_id/review_status/sendable。
/// target_stages 复用簇 B normalize_target_stages 归一，越界 400。
pub(super) async fn update_content_asset_meta(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateMetaRequest>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查 asset（workspace 隔离）拿 account_id 做归一 scope。
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;

    let mut set = Document::new();
    if let Some(v) = payload.title { set.insert("title", v); }
    if let Some(v) = payload.body { set.insert("body", v); }
    if let Some(v) = payload.tags { set.insert("tags", v); }
    if let Some(v) = payload.url { set.insert("url", v); }
    if let Some(v) = payload.usage_scene { set.insert("usage_scene", v); }
    if let Some(v) = payload.send_trigger_hint { set.insert("send_trigger_hint", v); }
    if let Some(v) = payload.expression_pref { set.insert("expression_pref", v); }
    if let Some(v) = payload.requires_principal_approval { set.insert("requires_principal_approval", v); }
    if let Some(stages) = payload.target_stages {
        let scope = asset.account_id.as_deref().unwrap_or("");
        let normalized = crate::agent::dimension_registry::normalize_target_stages(&state.db, scope, &stages)
            .await
            .map_err(|reason| AppError::BadRequest(format!("target_stages 校验未通过：{reason}")))?;
        set.insert("target_stages", normalized);
    }
    set.insert("updated_at", DateTime::now());

    state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": set },
            None,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}
```

> 部分更新语义：字段 `Some → $set`、`None（JSON 缺失或 null）→ 不动`。serde 不区分缺失与 null，故不支持显式清成 null；清空走传 `""`/`[]`。

- [ ] **Step 2: 跑 lib 编译**

Run: `cargo build --lib && cargo test --lib media_assets`
Expected: 编译通过（handler 暂无调用方/路由，Task 6 才挂——会有 dead_code warning，预期，commit message 注明）；既有 media_assets 测试 PASS。

- [ ] **Step 3: 提交**

```bash
git add src/routes/media_assets.rs
git commit -m "feat(media-asset-crud): edit 元数据端点 PUT /content-assets/:id(缺口4)

部分更新,只\$set 提供字段;target_stages 复用簇B归一越界400;不动 file_*/media_id/review_status。
未挂路由前有预期 dead_code warning,Task 6 接线后消失。"
```

---

### Task 3: 换文件端点 `POST /content-assets/:id/file`（multipart）

**Files:**
- Modify: `src/routes/media_assets.rs`（加 `replace_content_asset_file` handler）

**Interfaces:**
- Consumes: `media_storage::{sha256_hex, safe_relative_path, sanitize_ext, store_bytes, delete_bytes, should_delete_physical_file}`（Task 1 加了后两个）；`state.config.media_storage_dir`、`media_max_file_size_mb`。
- Produces: `pub(super) async fn replace_content_asset_file(...)`。Task 6 挂路由。

**背景**：换素材文件。换文件 = 发送物变了，必须清 media_id（防 TTL 内发旧文件）+ 退 draft 重审（AI 不自我核验红线）+ 旧文件按引用计数清理。

- [ ] **Step 1: 实现 handler**

参照 `upload_media_asset`（`media_assets.rs:34-164`）的 multipart 解析 + 落盘逻辑（就地复用，不抽 helper——避免改动已工作的 upload）。加在 `update_content_asset_meta` 之后：

```rust
/// POST /content-assets/:id/file —— 换文件（multipart）。
/// 落新文件 → $set file_* + media_id=None（清缓存防发旧文件）+ review_status="draft"（强制重审）。
/// 旧文件无兄弟引用则物理删（fail-soft）。
pub(super) async fn replace_content_asset_file(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查 asset（workspace 隔离）拿旧 file_path。
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;
    let old_file_path = asset.file_path.clone();

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut mime = String::new();
    let mut media_type = asset.media_type.clone().unwrap_or_default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "file" => {
                file_name = field.file_name().unwrap_or_default().to_string();
                mime = field.content_type().unwrap_or_default().to_string();
                file_bytes = Some(
                    field.bytes().await
                        .map_err(|e| AppError::BadRequest(format!("read file failed: {e}")))?
                        .to_vec(),
                );
            }
            "mediaType" => media_type = field.text().await.unwrap_or_default(),
            _ => { let _ = field.bytes().await; }
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError::BadRequest("file field required".into()))?;
    let max = state.config.media_max_file_size_mb * 1024 * 1024;
    if bytes.len() as u64 > max {
        return Err(AppError::BadRequest(format!("file exceeds {} MB", state.config.media_max_file_size_mb)));
    }
    if !is_valid_media_type(&media_type) {
        return Err(AppError::BadRequest("mediaType must be image|file|video".into()));
    }
    let ext = media_storage::sanitize_ext(&file_name, &mime)
        .ok_or_else(|| AppError::BadRequest("file type not allowed".into()))?;
    let sha = media_storage::sha256_hex(&bytes);
    let rel = media_storage::safe_relative_path(&admin.current_workspace, &sha, &ext)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    media_storage::store_bytes(root, &rel, &bytes)
        .await
        .map_err(|e| AppError::External(format!("store file failed: {e}")))?;

    // 换文件副作用：清 media_id（防 TTL 内发旧文件）+ 退 draft（强制重审）。
    state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": {
                "file_path": &rel,
                "file_name": &file_name,
                "file_size": bytes.len() as i64,
                "mime_type": &mime,
                "file_sha256": &sha,
                "media_type": &media_type,
                "media_id": null,
                "review_status": "draft",
                "updated_at": DateTime::now(),
            }},
            None,
        )
        .await?;

    // 旧文件清理：仅当旧路径与新路径不同（确实换了文件）且无兄弟引用时物理删。fail-soft。
    if let Some(old) = old_file_path {
        if old != rel {
            let refs = state
                .db
                .content_assets()
                .count_documents(doc! { "workspace_id": &admin.current_workspace, "file_path": &old }, None)
                .await
                .unwrap_or(1); // 查询失败 → 视为有引用，保守不删
            if media_storage::should_delete_physical_file(refs) {
                if let Err(e) = media_storage::delete_bytes(root, &old).await {
                    tracing::warn!("换文件后旧素材文件删除失败（不影响换文件）: {e}");
                }
            }
        }
    }
    Ok(Json(json!({ "ok": true })))
}
```

> `media_id: null` 在 `doc! {}` 里写 `null` 即 BSON Null，反序列化为 `Option::None`。`is_valid_media_type` 已在 media_assets.rs（upload 用）。

- [ ] **Step 2: 跑 lib 编译**

Run: `cargo build --lib && cargo test --lib media_assets`
Expected: 编译通过（dead_code warning 预期，Task 6 消）；既有测试 PASS。

- [ ] **Step 3: 提交**

```bash
git add src/routes/media_assets.rs
git commit -m "feat(media-asset-crud): 换文件端点 POST /content-assets/:id/file(缺口4)

落新文件+清media_id(防TTL发旧文件)+退draft(强制重审红线)+旧文件按引用计数fail-soft清理。"
```

---

### Task 4: toggle 端点 `POST /content-assets/:id/toggle`

**Files:**
- Modify: `src/routes/media_assets.rs`（加 `ToggleSendableRequest` 结构 + `toggle_content_asset_sendable` handler）

**Interfaces:**
- Produces: `pub(super) async fn toggle_content_asset_sendable(...)`。Task 6 挂路由。

**背景**：素材启停。`validate_asset_sendable`（`media_send.rs:24`）已查 `sendable==Some(true)`，门禁现成，本端点只写 sendable。与 review_status 正交（启停不动审核态）。对称 `toggle_referral_card`。

- [ ] **Step 1: 实现结构 + handler**

加在 `replace_content_asset_file` 之后：

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ToggleSendableRequest {
    sendable: bool,
}

/// POST /content-assets/:id/toggle —— 启停（写 sendable）。
/// 与 review_status 正交：停用不动审核态，重启不必重审。workspace 隔离。
pub(super) async fn toggle_content_asset_sendable(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ToggleSendableRequest>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    let res = state
        .db
        .content_assets()
        .update_one(
            doc! { "_id": oid, "workspace_id": &admin.current_workspace },
            doc! { "$set": { "sendable": payload.sendable, "updated_at": DateTime::now() } },
            None,
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }
    Ok(Json(json!({ "ok": true, "sendable": payload.sendable })))
}
```

- [ ] **Step 2: 跑 lib 编译**

Run: `cargo build --lib && cargo test --lib media_assets`
Expected: 编译通过（dead_code warning 预期，Task 6 消）；既有测试 PASS。

- [ ] **Step 3: 提交**

```bash
git add src/routes/media_assets.rs
git commit -m "feat(media-asset-crud): toggle 端点 POST /content-assets/:id/toggle 写sendable(缺口4)"
```

---

### Task 5: delete 端点 `DELETE /content-assets/:id`（引用计数清理）

**Files:**
- Modify: `src/routes/media_assets.rs`（加 `delete_content_asset` handler）

**Interfaces:**
- Consumes: `media_storage::{delete_bytes, should_delete_physical_file}`（Task 1）。
- Produces: `pub(super) async fn delete_content_asset(...)`。Task 6 挂路由。

**背景**：删素材。先删 DB 记录，再查同 file_path 剩余引用，无引用才物理删文件（防误删共享文件）。物理删 fail-soft。

- [ ] **Step 1: 实现 handler**

加在 `toggle_content_asset_sendable` 之后：

```rust
/// DELETE /content-assets/:id —— 删除。
/// 先删 DB 记录,再查同 file_path 剩余引用,无引用才物理删文件(防误删兄弟共享文件)。
/// 物理删 fail-soft(DB 已删=既成事实,残留文件无害)。workspace 隔离。
pub(super) async fn delete_content_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let oid = ObjectId::parse_str(&id).map_err(|_| AppError::BadRequest("bad id".into()))?;
    // 回查拿 file_path（workspace 隔离）。
    let asset = state
        .db
        .content_assets()
        .find_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?
        .ok_or_else(|| AppError::NotFound("asset not found".into()))?;

    let res = state
        .db
        .content_assets()
        .delete_one(doc! { "_id": oid, "workspace_id": &admin.current_workspace }, None)
        .await?;
    if res.deleted_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }

    // 引用计数清理：本记录已删,count 同 file_path 剩余引用,为 0 才物理删。
    if let Some(rel) = asset.file_path {
        let refs = state
            .db
            .content_assets()
            .count_documents(doc! { "workspace_id": &admin.current_workspace, "file_path": &rel }, None)
            .await
            .unwrap_or(1); // 查询失败 → 视为有引用,保守不删
        if media_storage::should_delete_physical_file(refs) {
            let root = std::path::Path::new(&state.config.media_storage_dir);
            if let Err(e) = media_storage::delete_bytes(root, &rel).await {
                tracing::warn!("删除素材后物理文件删除失败（不影响删除）: {e}");
            }
        }
    }
    Ok(Json(json!({ "ok": true })))
}
```

- [ ] **Step 2: 跑 lib 编译**

Run: `cargo build --lib && cargo test --lib media_assets`
Expected: 编译通过（dead_code warning 预期，Task 6 消）；既有测试 PASS。

- [ ] **Step 3: 提交**

```bash
git add src/routes/media_assets.rs
git commit -m "feat(media-asset-crud): delete 端点 DELETE /content-assets/:id 引用计数清理(缺口4)

先删DB再查同file_path剩余引用,为0才物理删(防误删共享文件),物理删fail-soft。"
```

---

### Task 6: list 补返回 sendable + 挂载 4 路由

**Files:**
- Modify: `src/routes/assets.rs:76-100`（`list_content_assets` 的 json! 加 `sendable` 字段）
- Modify: `src/routes/mod.rs`（挂 4 个新路由）

**Interfaces:**
- Consumes: Task 2-5 的 4 个 handler（`update_content_asset_meta` / `replace_content_asset_file` / `toggle_content_asset_sendable` / `delete_content_asset`）。
- Produces: 4 个路由 + list 输出含 sendable。本 task 让 Task 2-5 的 dead_code warning 全消失。

- [ ] **Step 1: list 补 sendable 字段**

`src/routes/assets.rs` 的 `list_content_assets` json!（`:77-100`）里，在 `"reviewStatus": asset.review_status,`（`:97`）旁加一行：

```rust
            "sendable": asset.sendable,
```

- [ ] **Step 2: 挂载 4 路由**

`src/routes/mod.rs` 在 `/content-assets/:id/review` 路由（`:385-388`）之后加：

```rust
        .route(
            "/content-assets/:id",
            axum::routing::put(media_assets::update_content_asset_meta)
                .delete(media_assets::delete_content_asset),
        )
        .route(
            "/content-assets/:id/file",
            post(media_assets::replace_content_asset_file).layer(
                axum::extract::DefaultBodyLimit::max(
                    state.config.media_max_file_size_mb as usize * 1024 * 1024,
                ),
            ),
        )
        .route(
            "/content-assets/:id/toggle",
            post(media_assets::toggle_content_asset_sendable),
        )
```

> `put` / `post` 已在 mod.rs 导入。换文件路由要单独抬高 body limit（同 upload，`:379` 同款 layer），否则 >2MB 文件 413。

- [ ] **Step 3: 跑全 lib 编译 + 测试**

Run: `cargo build --lib && cargo test --lib`
Expected: 编译通过、**无 dead_code warning**（4 handler 现在都被路由引用）；lib ≥ 350 passed / 0 failed。

- [ ] **Step 4: no-human-takeover 自查**

Run: `cargo build --lib`（确认编译）后人工核对：`git diff` 本 task 改动的新增行无禁词。

- [ ] **Step 5: 提交**

```bash
git add src/routes/assets.rs src/routes/mod.rs
git commit -m "feat(media-asset-crud): 挂载 edit/file/toggle/delete 4路由 + list返回sendable(缺口4)

content-assets/:id PUT改元数据+DELETE删除;/file换文件(抬body limit);/toggle启停。list补sendable供前端回显。"
```

---

### Task 7: 前端素材操作 UI（编辑/换文件/停用/删除）

**Files:**
- Modify: `frontend/src/types/index.ts:165-184`（ContentAsset 类型加 `sendable?: boolean`）
- Modify: `frontend/src/stores/contentStore.ts`（加 4 个 action）
- Modify: `frontend/src/features/content-assets/index.tsx`（`MediaAssetRow` 加操作入口 + 透传）

**Interfaces:**
- Consumes: Task 2-6 的 4 个端点 + list 返回的 `sendable`。`api.{put, postForm, post, delete}`（`lib/api.ts` 全有）。
- Produces: 无（纯前端 UI）。

**背景**：素材页现仅上传 + 审核（`MediaAssetRow` 只有"标记为可发送"按钮）。补编辑/换文件/停用/删除入口。照现有 `reviewMediaAsset` 的 store 链路 + `MediaAssetRow` 的 props 模式扩展。

- [ ] **Step 1: 类型加 sendable**

`frontend/src/types/index.ts` 的 `ContentAsset`（`:165-184`），在 `reviewStatus?: ...`（`:182`）旁加：

```ts
  sendable?: boolean;
```

- [ ] **Step 2: store 加 4 个 action**

`frontend/src/stores/contentStore.ts`：在 `ContentActions` interface（`:18-36`）加 4 个签名，在 store 实现（`reviewMediaAsset` 之后，`:131`）加实现。全部照 `reviewMediaAsset` 链路（setBusy/setError/finally + `loadAssets` 刷新）：

```ts
  // interface 部分：
  editAssetMeta: (id: string, fields: Record<string, unknown>, accountId?: string) => Promise<void>;
  replaceAssetFile: (id: string, form: FormData, accountId?: string) => Promise<boolean>;
  toggleAssetSendable: (id: string, sendable: boolean, accountId?: string) => Promise<void>;
  deleteAsset: (id: string, accountId?: string) => Promise<void>;
```

```ts
  // 实现部分（reviewMediaAsset 之后）：
  editAssetMeta: async (id, fields, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.put(`/api/content-assets/${id}`, fields);
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  replaceAssetFile: async (id, form, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.postForm(`/api/content-assets/${id}/file`, form);
      await get().loadAssets(accountId);
      return true;
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
      return false;
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  toggleAssetSendable: async (id, sendable, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.post(`/api/content-assets/${id}/toggle`, { sendable });
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },

  deleteAsset: async (id, accountId) => {
    useUiStore.getState().setBusy(true);
    useUiStore.getState().setError("");
    try {
      await api.delete(`/api/content-assets/${id}`);
      await get().loadAssets(accountId);
    } catch (error) {
      useUiStore.getState().setError(error instanceof Error ? error.message : String(error));
    } finally {
      useUiStore.getState().setBusy(false);
    }
  },
```

- [ ] **Step 3: MediaAssetRow 加操作入口**

`frontend/src/features/content-assets/index.tsx` 的 `MediaAssetRow`（`:307-346`）：

1）扩展 props，加 `onToggleSendable: (sendable: boolean) => void`、`onDelete: () => void`、`onEditMeta: (fields: Record<string, unknown>) => void`、`onReplaceFile: (form: FormData) => void`（照现有 `onApprove` 模式）。父组件（渲染 MediaAssetRow 处，grep `<MediaAssetRow`）把 store 的 4 个 action 绑定传入（带 accountId）。

2）在 `MediaAssetRow` 的操作区（`onApprove` 按钮旁，`:337-341`）加：
- **停用/启用开关**：读 `asset.sendable`（缺省视为 true），按钮文案"停用"/"启用"，onClick 调 `onToggleSendable(!current)`。
- **删除按钮**：onClick 弹 `window.confirm("确认删除该素材？此操作不可撤销。")`，确认才调 `onDelete()`。
- **编辑入口**：一个"编辑"按钮切出表单（本地 useState 控制展开），表单含 title / sendTriggerHint / targetStages（逗号分隔输入）等元数据字段，保存调 `onEditMeta({ title, sendTriggerHint, targetStages: [...] })`；表单里另有"换文件"文件选择 + 按钮，选文件后构造 FormData（`file` + `mediaType`）调 `onReplaceFile(form)`。

设计语言遵现有 `ContentAssets.module.css` 既有类名（`styles.reviewBtn` / `styles.row` / `styles.metaLine` 等），**不新造样式、不硬编码颜色**（对齐项目"前端遵守现有设计系统"立场）。

- [ ] **Step 4: 构建验证**

Run: `cd frontend && npm run build`
Expected: 构建通过，无 TS 错误。

> 文案"编辑/停用/启用/删除/换文件"守 no-human-takeover 禁词（均中性词，安全）。

- [ ] **Step 5: 提交**

```bash
git add frontend/src/types/index.ts frontend/src/stores/contentStore.ts frontend/src/features/content-assets/index.tsx
git commit -m "feat(media-asset-crud): 素材操作UI 编辑/换文件/停用/删除(缺口4前端)

contentStore加4 action照reviewMediaAsset链路;MediaAssetRow加停用开关+删除(二次确认)+编辑表单+换文件;ContentAsset类型加sendable。"
```

---

### Task 8: 集成测试（4 端点端到端，`#[ignore]` / CI）

**Files:**
- Create: `tests/media_asset_crud_integration.rs`

**Interfaces:**
- Consumes: 既有测试设施（`tests/common/mod.rs` 的 `TestApp`、`tests/media_asset_send_integration.rs` 的 fixture）。Task 2-6 的端点行为。
- Produces: 无（测试）。

**背景**：edit/换文件/toggle/delete 都是 DB + 文件副作用，lib 无法纯测（纯函数部分 Task 1 已测）。本 task 用 testcontainers 钉端到端。全部 `#[ignore]`（需 Docker，交 CI）。

- [ ] **Step 1: 读测试设施**

读 `tests/common/mod.rs` 确认 `TestApp` 构造 + admin 上下文构造方式。读 `tests/media_asset_send_integration.rs` 看有没有可复用的 seed content_asset fixture（Task 8 簇B 用过直调 handler 惯例——本仓集成测试直调 route handler 真函数，不走 HTTP）。**确认惯例后照搬**。

> 直调 handler 惯例：handler 参数是 axum extractor（State/Extension/Path/Json），构造好 `.await`，错误用 `Err(AppError::*)` 变体断言。换文件端点取 `Multipart`，**tests crate 无法构造 Multipart**（同簇B Task8 发现）——故换文件的端到端断言改为：直接构造一个带 file_path 的 asset 入库 → 直调 `delete_content_asset` 验引用计数清理；换文件的"清 media_id + 退 draft"副作用由 `update_one $set` 是确定性的、可在能构造的路径上验，或退而由代码审查保证（在报告里说明取舍，参考簇B Task8 对 Multipart 限制的处理）。

- [ ] **Step 2: 写测试（实义断言，全部 #[ignore]）**

按以下行为各写一个测试（helper 名对齐 common/mod.rs 实际）：

```rust
//! 簇 C 素材库 CRUD 补全集成测试：edit 元数据 / toggle / delete 引用计数。
//! 全部 #[ignore]，需 Docker testcontainers。直调 handler 真函数（本仓既有惯例）。
mod common;
use common::*;

// 缺口4：edit 元数据 —— 改 title/send_trigger_hint 落库，review_status 不变。
#[tokio::test]
#[ignore]
async fn edit_meta_updates_fields_keeps_review_status() {
    // seed 一个 approved media asset → 直调 update_content_asset_meta 改 title →
    // 回查 title 已变、review_status 仍 approved（改元数据不退审）。
}

// 缺口4：edit 元数据 —— target_stages 越界 400。
#[tokio::test]
#[ignore]
async fn edit_meta_out_of_dict_stage_rejected() {
    // 种 ≥1 个 customer_stage 字典条目 → 改 target_stages 为字典外值 → Err(BadRequest)。
}

// 缺口4：toggle —— sendable=false 落库。
#[tokio::test]
#[ignore]
async fn toggle_sets_sendable() {
    // seed asset(sendable=true) → 直调 toggle(false) → 回查 sendable==false。
}

// 缺口4：toggle —— 跨 workspace 404（IDOR）。
#[tokio::test]
#[ignore]
async fn toggle_cross_workspace_404() {
    // asset 在 other_ws、admin 在 default → Err(NotFound)。
}

// 缺口4：delete —— 无兄弟引用，DB 记录删除 + 物理文件删除。
#[tokio::test]
#[ignore]
async fn delete_removes_db_and_file_when_no_siblings() {
    // store_bytes 落一个文件 + seed 唯一引用它的 asset → 直调 delete →
    // DB 记录没了、物理文件也没了。
}

// 缺口4：delete —— 有兄弟引用，DB 记录删除但物理文件保留。
#[tokio::test]
#[ignore]
async fn delete_keeps_file_when_sibling_references_it() {
    // store_bytes 落一个文件 + seed 两条 asset 共享同一 file_path → 删其中一条 →
    // 被删记录没了、另一条还在、物理文件仍在（引用计数保护）。
}

// 缺口4：delete —— 跨 workspace 404（IDOR）。
#[tokio::test]
#[ignore]
async fn delete_cross_workspace_404() {
    // asset 在 other_ws、admin 在 default → Err(NotFound)，且 asset 未被删。
}
```

实现者填充测试体：用 common 的 TestApp + 直调 handler（`update_content_asset_meta` / `toggle_content_asset_sendable` / `delete_content_asset`），seed asset 用 `state.db.content_assets().insert_one(...)`，文件用 `media_storage::store_bytes` 落到 `state.config.media_storage_dir`。断言用回查 DB（`find_one`）+ 文件存在性（`Path::exists`）+ `Err(AppError::NotFound)` 变体。

> **delete 兄弟引用测试是缺口4 的核心保护点，必须扎实**：两条 asset 共享 file_path，删一条后文件必须还在。这条是引用计数防误删的回归守卫。

- [ ] **Step 3: 编译验证（不跑 ignored）**

Run: `cargo test --test media_asset_crud_integration --no-run`
Expected: 编译通过（`#[ignore]` 测试不执行，仅确认编译）。本地无 Docker + 磁盘紧，**不**跑 `-- --ignored`。CI integration job 跑。

- [ ] **Step 4: 提交**

```bash
git add tests/media_asset_crud_integration.rs
git commit -m "test(media-asset-crud): 4端点端到端集成测试(#[ignore]/CI)

edit元数据(改字段不退审/越界400);toggle(写sendable/跨ws 404);delete(无兄弟删文件/有兄弟保留文件引用计数保护/跨ws 404)。"
```

---

## 执行顺序与依赖

- **Task 1**（media_storage 地基：delete_bytes + should_delete_physical_file）→ Task 3、Task 5 依赖它。
- **Task 2**（edit 元数据）依赖簇 B 的 normalize_target_stages（已合入）。
- **Task 3**（换文件）依赖 Task 1。
- **Task 4**（toggle）独立。
- **Task 5**（delete）依赖 Task 1。
- **Task 6**（list + 路由挂载）依赖 Task 2-5 全部 handler 存在——它让前面所有 dead_code warning 消失。
- **Task 7**（前端）依赖 Task 6（端点可用 + list 返回 sendable）。
- **Task 8**（集成测试）依赖 Task 2-6 端点行为。

顺序：1 → 2 → 3 → 4 → 5 → 6 → 7 → 8。Task 2-5 都改 `media_assets.rs` 同文件、各加一个 handler，串行执行（subagent-driven 本就串行，无并行冲突）。Task 1-6 后端 `cargo test --lib` 验；Task 7 前端 `npm run build` 验；Task 8 仅 `--no-run` 编译验（CI 跑 ignored）。


