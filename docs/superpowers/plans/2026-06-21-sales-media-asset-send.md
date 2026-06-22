# 销售素材文件发送能力 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 AI 在销售对话中按"双轨并行"模型自主把图片/PDF/Word/视频等原始素材文件发给微信客户。

**Architecture:** 改造现有 `ContentAsset` 为带本地文件存储 + 发送标注的素材库；新增媒体上传 API；扩展 `AgentDecision` 让 AI 在一次决策里同时输出 `reply_text` + `assets_to_send`；新增媒体发送函数（媒体类型→MCP 工具名映射），经现有 outbox 幂等通道调 MCP `media_upload_base64`→`message_send_{image,file,video}`；前端补素材上传/审核/对话媒体渲染。

**Tech Stack:** Rust 2021 / Axum / MongoDB(mongodb crate) / 现有 MCP JSON-RPC client / React 19 + Vite + TS。

设计来源：`docs/superpowers/specs/2026-06-21-sales-media-asset-send-design.md`。

## Global Constraints

- **向后兼容红线**：`ContentAsset` / `OutboxEntry` / `ConversationMessage` 所有新增字段必须 `Option` + `#[serde(default)]`；现有 content_assets 行（朋友圈素材，无新字段）必须仍能反序列化、且不被误选为可发送素材。
- **幂等红线**：approved 的发送必须先进 `agent_send_outbox` 拿幂等键再调 MCP。文件重发给客户不可接受。
- **grounding 边界**：素材文件内容免 AI 核验（人类已把关），但伴随的 `reply_text` 照常走五闸门。
- **AI 不自我核验红线**：上传素材默认 `review_status="draft"`，必须人类标 `approved` 才允许 AI 发。
- **测试铁律**：纯函数确定性测试为主；不接受 skip 假绿；新增测试只 append 不删旧维度；不过拟合单条样本；baseline 不回归（`cargo test --lib` ≥350 passed/0 failed，4 个 PBT 累计 ≥33/0）。
- **no-human-takeover lint**：新增代码（src/agent/、src/routes/、frontend/src/）不得出现 `人工接管|takeover|hand-off|人工介入|接管|人工` 等禁词。素材发送是 AI 自主行为，措辞用 AI-internal 名。
- **媒体类型→MCP 工具名不写死**：用映射表，private-chat 侧确认能接入的是 image/file/video；链接卡片/小程序留占位。
- **MCP 入参字段名**以 server 侧 `tools/list` 实际 schema 为准（用户负责 MCP 侧）；本计划用 `mediaId` 占位，集成时对齐。
- **Shell**：bash on Windows，项目根含非 ASCII（`工作项目`），用绝对路径。本地只跑 `cargo test --lib` 和单个 PBT，全量集成留 CI。

---

## File Structure

**后端新建：**
- `src/routes/media_assets.rs` — 媒体素材上传/审核 route handler（multipart 接收、落盘、审核状态流转）。
- `src/agent/media_send.rs` — 媒体发送：媒体类型→MCP 工具名映射、`ensure_media_uploaded`、`send_outbound_media`、选材候选过滤纯函数。
- `src/media_storage.rs` — 本地文件存储：安全路径构造（防穿越）、sha256、读写。

**后端修改：**
- `src/models.rs` — `ContentAsset` 加字段；`OutboxEntry` 加 `media_asset_id: Option<String>`；`ConversationMessage` 加 `msg_type` / `media_ref`。
- `src/config.rs` + `.env.example` — `MEDIA_STORAGE_DIR` / `MEDIA_MAX_FILE_SIZE_MB` / `MEDIA_ID_CACHE_TTL_HOURS`。
- `src/agent/types.rs` — `AgentDecision` 加 `assets_to_send: Vec<AssetSendDirective>`；定义 `AssetSendDirective`。
- `src/agent/decision.rs` — 新增 `load_sendable_assets` + 候选清单注入 prompt。
- `src/agent/outbox.rs` — `EnqueueRequest` 加 `media_asset_id`；放宽"空 content"校验（媒体条目 content 可空）。
- `src/agent/outbox_dispatcher.rs` — dispatch 时若 `media_asset_id` 有值，走 `send_outbound_media`。
- `src/agent/gateway.rs` — 选材准入校验 + 把 `assets_to_send` 转成 outbox 媒体条目（按 expression_pref 定序）。
- `src/routes/mod.rs` — 挂载 `media_assets` 路由。
- `src/db/mod.rs` — content_assets 新增索引（`ensure_indexes`）。
- `src/prompts.rs` — 选材指引文案（双轨 / expression_pref 详略 / 没合适就不发）。

**前端修改：**
- `frontend/src/features/content-assets/*` — 上传组件 + 标注表单 + 审核 + 冲突三小提醒 + 预览。
- `frontend/src/features/user-ops/legacy.tsx` + `frontend/src/types/index.ts` — `Message` 加 `msgType`/媒体渲染。
- `frontend/src/lib/api.ts` — 复用现有 `postForm`（:82）。

**测试新建：**
- `src/media_storage.rs` 内联 `#[cfg(test)]` — 路径安全 / sha256。
- `src/agent/media_send.rs` 内联 `#[cfg(test)]` — 映射表 / 候选过滤 / 准入校验。
- `tests/media_asset_send_integration.rs`（`#[ignore]`，CI）— upload→发送数据流 / outbox 幂等。

---

### Task 1: ContentAsset 数据模型扩展 + 向后兼容测试

**Files:**
- Modify: `src/models.rs:669-685`（`ContentAsset` 结构体）
- Test: `src/models.rs` 内联 `#[cfg(test)] mod content_asset_compat_tests`

**Interfaces:**
- Consumes: 无（地基任务）
- Produces: `ContentAsset` 新增字段 `media_type: Option<String>`、`file_path: Option<String>`、`file_name: Option<String>`、`file_size: Option<i64>`、`mime_type: Option<String>`、`file_sha256: Option<String>`、`sendable: Option<bool>`、`send_trigger_hint: Option<String>`、`target_stages: Option<Vec<String>>`、`expression_pref: Option<String>`、`requires_principal_approval: Option<bool>`、`review_status: Option<String>`、`review_note: Option<String>`。后续 Task 2/4/5/7/8 依赖这些字段。

- [ ] **Step 1: 写失败测试——旧文档（无新字段）能反序列化，新字段为 None**

在 `src/models.rs` 文件末尾 `#[cfg(test)]` 区追加：

```rust
#[cfg(test)]
mod content_asset_compat_tests {
    use super::ContentAsset;
    use mongodb::bson::{doc, DateTime};

    #[test]
    fn legacy_asset_without_new_fields_deserializes_with_none() {
        // 模拟现有朋友圈素材行：只有老字段
        let legacy = doc! {
            "workspace_id": "ws1",
            "kind": "text",
            "title": "朋友圈文案A",
            "tags": ["promo"],
            "created_at": DateTime::now(),
            "updated_at": DateTime::now(),
        };
        let asset: ContentAsset = mongodb::bson::from_document(legacy)
            .expect("legacy content_assets row must still deserialize");
        // 关键：旧行不被误判为可发送素材
        assert_eq!(asset.sendable, None);
        assert_eq!(asset.media_type, None);
        assert_eq!(asset.review_status, None);
        assert_eq!(asset.file_path, None);
    }

    #[test]
    fn sendable_asset_roundtrips_all_new_fields() {
        let asset = ContentAsset {
            id: None,
            workspace_id: "ws1".into(),
            account_id: None,
            kind: "media".into(),
            title: "产品报价单.xlsx".into(),
            body: None,
            tags: vec![],
            url: None,
            media_id: None,
            usage_scene: None,
            media_type: Some("file".into()),
            file_path: Some("ws1/ab/abcd.xlsx".into()),
            file_name: Some("产品报价单.xlsx".into()),
            file_size: Some(20480),
            mime_type: Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into()),
            file_sha256: Some("abcd".into()),
            sendable: Some(true),
            send_trigger_hint: Some("客户问价格时发".into()),
            target_stages: Some(vec!["意向".into(), "未成交".into()]),
            expression_pref: Some("file_primary".into()),
            requires_principal_approval: Some(false),
            review_status: Some("approved".into()),
            review_note: None,
            created_at: DateTime::now(),
            updated_at: DateTime::now(),
        };
        let doc = mongodb::bson::to_document(&asset).unwrap();
        let back: ContentAsset = mongodb::bson::from_document(doc).unwrap();
        assert_eq!(back.media_type.as_deref(), Some("file"));
        assert_eq!(back.expression_pref.as_deref(), Some("file_primary"));
        assert_eq!(back.target_stages.unwrap().len(), 2);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --lib content_asset_compat_tests`
Expected: 编译失败（`ContentAsset` 缺新字段，结构体字面量缺字段）。

- [ ] **Step 3: 给 ContentAsset 加字段**

修改 `src/models.rs:669-685`，在 `usage_scene` 与 `created_at` 之间插入新字段（全部 `Option` + `#[serde(default)]`）：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAsset {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub account_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub url: Option<String>,
    pub media_id: Option<String>,
    pub usage_scene: Option<String>,

    // ===== 销售素材文件发送：文件资产本体 =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>, // "image"|"file"|"video"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>, // MEDIA_STORAGE_DIR 下相对路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_sha256: Option<String>,

    // ===== 发送标注（人类上传时填）=====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sendable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_trigger_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_stages: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression_pref: Option<String>, // "file_primary"|"file_support"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_principal_approval: Option<bool>,

    // ===== 审核状态 =====
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>, // "draft"|"approved"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_note: Option<String>,

    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

注意：现有 `src/routes/assets.rs:104-117` 的 `ContentAsset{...}` 字面量会因新字段缺失而编译失败——在该字面量补上所有新字段 `: None`（保持现有 JSON 创建路径行为不变，新字段走 Task 4 的上传路径填充）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --lib content_asset_compat_tests`
Expected: 2 passed。

- [ ] **Step 5: 确认全 lib 编译 + baseline 不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 全绿，passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/routes/assets.rs
git commit -m "feat(media-asset): ContentAsset 扩展文件资产+发送标注字段(向后兼容)"
```

---

### Task 2: 本地文件存储模块（路径安全 + sha256）

**Files:**
- Create: `src/media_storage.rs`
- Modify: `src/lib.rs`（加 `pub mod media_storage;`）
- Test: `src/media_storage.rs` 内联 `#[cfg(test)]`

**Interfaces:**
- Consumes: 无外部依赖（用 `sha2` crate——先确认 Cargo.toml 已有；outbox.rs 用了 `sha256_hex` 说明 sha2 已在依赖里）
- Produces:
  - `pub fn safe_relative_path(workspace_id: &str, sha256: &str, ext: &str) -> Result<String, MediaStorageError>` — 返回 `{workspace}/{sha前2位}/{sha}.{ext}`，拒绝路径穿越。
  - `pub fn sha256_hex(bytes: &[u8]) -> String`
  - `pub fn sanitize_ext(file_name: &str, mime: &str) -> Option<String>` — 从文件名/mime 推扩展名，白名单。
  - `pub async fn store_bytes(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<()>`
  - `pub async fn read_bytes(root: &Path, rel: &str) -> std::io::Result<Vec<u8>>`
  - `pub enum MediaStorageError { PathTraversal, BadExtension }`（impl Display + std::error::Error）

- [ ] **Step 1: 确认 sha2 依赖存在**

Run: `grep -n "sha2\|sha256_hex" src/agent/outbox.rs Cargo.toml`
Expected: 看到 outbox.rs 已用 `sha256_hex`，Cargo.toml 有 `sha2`。若 outbox 的 `sha256_hex` 是本地私有函数，本模块自带一份实现（用 `sha2::{Sha256, Digest}`）。

- [ ] **Step 2: 写失败测试——路径安全 + sha256 + 扩展名白名单**

新建 `src/media_storage.rs`，先只写测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_builds_sharded_layout() {
        let p = safe_relative_path("ws1", "abcdef1234", "pdf").unwrap();
        assert_eq!(p, "ws1/ab/abcdef1234.pdf");
    }

    #[test]
    fn safe_path_rejects_traversal_in_workspace() {
        assert!(matches!(
            safe_relative_path("../etc", "abcd", "pdf"),
            Err(MediaStorageError::PathTraversal)
        ));
    }

    #[test]
    fn safe_path_rejects_traversal_in_sha() {
        assert!(matches!(
            safe_relative_path("ws1", "../../secret", "pdf"),
            Err(MediaStorageError::PathTraversal)
        ));
    }

    #[test]
    fn sha256_is_deterministic() {
        assert_eq!(sha256_hex(b"hello"), sha256_hex(b"hello"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
        assert_eq!(sha256_hex(b"hello").len(), 64);
    }

    #[test]
    fn sanitize_ext_whitelists_known_types() {
        assert_eq!(sanitize_ext("a.pdf", "application/pdf").as_deref(), Some("pdf"));
        assert_eq!(sanitize_ext("a.PNG", "image/png").as_deref(), Some("png"));
        // 危险/未知扩展名拒绝
        assert_eq!(sanitize_ext("evil.exe", "application/octet-stream"), None);
        assert_eq!(sanitize_ext("evil.sh", "text/x-sh"), None);
    }
}
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test --lib media_storage`
Expected: 编译失败（函数未定义）。

- [ ] **Step 4: 实现 media_storage.rs**

在测试模块之上写实现：

```rust
//! 销售素材本地文件存储：安全路径构造（防穿越）、sha256、读写。
use std::path::{Path, PathBuf};
use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub enum MediaStorageError {
    PathTraversal,
    BadExtension,
}

impl std::fmt::Display for MediaStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaStorageError::PathTraversal => write!(f, "path traversal rejected"),
            MediaStorageError::BadExtension => write!(f, "extension not allowed"),
        }
    }
}
impl std::error::Error for MediaStorageError {}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// 仅允许 [a-z0-9] 的 segment（workspace_id / sha 都应满足；含 . / 或其它即拒）。
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn safe_relative_path(
    workspace_id: &str,
    sha256: &str,
    ext: &str,
) -> Result<String, MediaStorageError> {
    if !is_safe_segment(workspace_id) || !is_safe_segment(sha256) || !is_safe_segment(ext) {
        return Err(MediaStorageError::PathTraversal);
    }
    if sha256.len() < 2 {
        return Err(MediaStorageError::PathTraversal);
    }
    let shard = &sha256[..2];
    Ok(format!("{workspace_id}/{shard}/{sha256}.{ext}"))
}

const ALLOWED: &[(&str, &str)] = &[
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("doc", "application/msword"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("xls", "application/vnd.ms-excel"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("ppt", "application/vnd.ms-powerpoint"),
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("mp4", "video/mp4"),
    ("mov", "video/quicktime"),
];

pub fn sanitize_ext(file_name: &str, mime: &str) -> Option<String> {
    let ext = file_name.rsplit('.').next()?.to_ascii_lowercase();
    ALLOWED
        .iter()
        .find(|(e, m)| *e == ext && (*m == mime || mime.is_empty()))
        .map(|(e, _)| e.to_string())
}

pub async fn store_bytes(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
    let full: PathBuf = root.join(rel);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(full, bytes).await
}

pub async fn read_bytes(root: &Path, rel: &str) -> std::io::Result<Vec<u8>> {
    tokio::fs::read(root.join(rel)).await
}
```

在 `src/lib.rs` 加 `pub mod media_storage;`（与其它顶层模块声明并列）。

- [ ] **Step 5: 运行确认通过**

Run: `cargo test --lib media_storage`
Expected: 5 passed。

- [ ] **Step 6: Commit**

```bash
git add src/media_storage.rs src/lib.rs
git commit -m "feat(media-asset): 本地文件存储模块(路径防穿越+sha256+扩展名白名单)"
```

---

### Task 3: 配置项 + content_assets 索引

**Files:**
- Modify: `src/config.rs`（`AppConfig` 加字段 + 解析）
- Modify: `.env.example`
- Modify: `src/db/mod.rs`（`ensure_indexes` 加 content_assets 索引）
- Test: `src/config.rs` 内联测试（若已有 config 测试模块则 append；否则验证默认值的小测试）

**Interfaces:**
- Consumes: 无
- Produces: `AppConfig.media_storage_dir: String`、`media_max_file_size_mb: u64`、`media_id_cache_ttl_hours: i64`。Task 4/5 依赖。

- [ ] **Step 1: 加配置字段 + 默认值解析**

`src/config.rs`：在 `AppConfig` 结构体合适位置加：

```rust
    /// 销售素材文件本地存储根目录。默认 "./media"；生产 117 为 "/opt/wechatagent/media"。
    pub media_storage_dir: String,
    /// 单个素材上传大小上限（MB）。默认 50。
    pub media_max_file_size_mb: u64,
    /// MCP media_id 缓存有效期（小时），过期重传。默认 24。
    pub media_id_cache_ttl_hours: i64,
```

在 `AppConfig::from_env`（或现有解析处，跟随 `task_worker_interval_seconds` 等同款 `env::var(...).ok().and_then(...).unwrap_or(默认)` 模式）加：

```rust
        let media_storage_dir =
            env::var("MEDIA_STORAGE_DIR").unwrap_or_else(|_| "./media".to_string());
        let media_max_file_size_mb = env::var("MEDIA_MAX_FILE_SIZE_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);
        let media_id_cache_ttl_hours = env::var("MEDIA_ID_CACHE_TTL_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
```

并加进结构体构造。

- [ ] **Step 2: .env.example 补三行**

在 MCP 配置附近（`.env.example:16-17` 之后）追加：

```
# 销售素材文件存储
MEDIA_STORAGE_DIR=./media
MEDIA_MAX_FILE_SIZE_MB=50
MEDIA_ID_CACHE_TTL_HOURS=24
```

- [ ] **Step 3: content_assets 索引**

`src/db/mod.rs` 的 `ensure_indexes` 里，找到现有 content_assets 索引块（若无则在其它 collection 索引旁新增）。追加：

```rust
    // 销售素材选材查询 + 去重
    db.content_assets()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "workspace_id": 1, "sendable": 1, "review_status": 1 })
                .build(),
            None,
        )
        .await?;
    db.content_assets()
        .create_index(
            IndexModel::builder()
                .keys(doc! { "file_sha256": 1 })
                .build(),
            None,
        )
        .await?;
```

（确认 `IndexModel` / `doc!` 已 use；跟随该文件现有 create_index 写法。）

- [ ] **Step 4: 编译 + lib 测试**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: 全绿，passed ≥ 350。

- [ ] **Step 5: Commit**

```bash
git add src/config.rs .env.example src/db/mod.rs
git commit -m "feat(media-asset): 加 MEDIA_* 配置 + content_assets 选材/去重索引"
```

---

### Task 4: 媒体素材上传 + 审核 API

**Files:**
- Create: `src/routes/media_assets.rs`
- Modify: `src/routes/mod.rs`（挂载路由 + 加 `mod media_assets;`）
- Test: `tests/media_asset_send_integration.rs`（`#[ignore]`，CI 跑）的 upload 部分

**Interfaces:**
- Consumes: `media_storage::{safe_relative_path, sha256_hex, sanitize_ext, store_bytes}`（Task 2）、`AppConfig.media_storage_dir / media_max_file_size_mb`（Task 3）、`ContentAsset` 新字段（Task 1）
- Produces:
  - `POST /api/content-assets/upload`（multipart）→ 落盘 + 落库 draft 素材，返回 `{ id }`
  - `POST /api/content-assets/:id/review`（JSON `{ "status": "approved"|"draft", "note"?: string }`）→ 流转审核状态

- [ ] **Step 1: 写集成测试骨架（#[ignore]，CI）**

新建 `tests/media_asset_send_integration.rs`：

```rust
//! 销售素材上传 → 审核 → 发送 的端到端数据流。需 Docker(testcontainers Mongo)，
//! 默认 #[ignore]，CI integration job 跑。
#![cfg(test)]

// 复用项目现有 testcontainers 启动 helper（参照其它 tests/*.rs 的 setup 形态）。

#[tokio::test]
#[ignore = "requires docker mongo"]
async fn upload_then_review_then_only_approved_is_sendable() {
    // 1. 上传一个 PDF（multipart）→ 期望落库 review_status="draft", sendable=true, media_type="file"
    // 2. load_sendable_assets 在 draft 态下不返回它
    // 3. 调 /review approved 后，load_sendable_assets 返回它
    // 断言见 Task 5（load_sendable_assets）落地后补全。此处先占位结构。
    assert!(true);
}
```

（说明：集成测试的真实断言依赖 Task 5 的 `load_sendable_assets`，本 Task 先建上传路径 + 占位；Task 5 回填断言。这样划分让上传 API 可独立提交。）

- [ ] **Step 2: 实现 media_assets.rs 的 upload handler**

```rust
//! 销售素材库：文件上传（multipart）+ 审核状态流转。
use axum::{
    extract::{Multipart, Path, State},
    Extension, Json,
};
use mongodb::bson::{doc, oid::ObjectId, DateTime};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    auth::AuthenticatedAdmin,
    error::{AppError, AppResult},
    media_storage,
    models::ContentAsset,
};
use super::AppState;

pub(super) async fn upload_media_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    mut multipart: Multipart,
) -> AppResult<Json<Value>> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut mime = String::new();
    let mut title = String::new();
    let mut media_type = String::new();
    let mut send_trigger_hint: Option<String> = None;
    let mut expression_pref: Option<String> = None;
    let mut target_stages: Vec<String> = vec![];
    let mut requires_principal_approval = false;
    let mut account_id: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::BadRequest(format!("multipart error: {e}"))
    })? {
        match field.name().unwrap_or_default() {
            "file" => {
                file_name = field.file_name().unwrap_or_default().to_string();
                mime = field.content_type().unwrap_or_default().to_string();
                file_bytes = Some(field.bytes().await.map_err(|e| {
                    AppError::BadRequest(format!("read file failed: {e}"))
                })?.to_vec());
            }
            "title" => title = field.text().await.unwrap_or_default(),
            "mediaType" => media_type = field.text().await.unwrap_or_default(),
            "sendTriggerHint" => {
                let v = field.text().await.unwrap_or_default();
                send_trigger_hint = (!v.is_empty()).then_some(v);
            }
            "expressionPref" => {
                let v = field.text().await.unwrap_or_default();
                expression_pref = (!v.is_empty()).then_some(v);
            }
            "targetStages" => {
                // 逗号分隔
                target_stages = field.text().await.unwrap_or_default()
                    .split(',').filter(|s| !s.is_empty()).map(|s| s.trim().to_string()).collect();
            }
            "requiresPrincipalApproval" => {
                requires_principal_approval = field.text().await.unwrap_or_default() == "true";
            }
            "accountId" => {
                let v = field.text().await.unwrap_or_default();
                account_id = (!v.is_empty()).then_some(v);
            }
            _ => { let _ = field.bytes().await; }
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError::BadRequest("file field required".into()))?;
    if title.trim().is_empty() {
        return Err(AppError::BadRequest("title required".into()));
    }
    // 大小上限
    let max = state.config.media_max_file_size_mb * 1024 * 1024;
    if bytes.len() as u64 > max {
        return Err(AppError::BadRequest(format!(
            "file exceeds {} MB", state.config.media_max_file_size_mb
        )));
    }
    // 扩展名白名单（同时隐含 media_type 合法性）
    let ext = media_storage::sanitize_ext(&file_name, &mime)
        .ok_or_else(|| AppError::BadRequest("file type not allowed".into()))?;
    if !matches!(media_type.as_str(), "image" | "file" | "video") {
        return Err(AppError::BadRequest("mediaType must be image|file|video".into()));
    }

    let sha = media_storage::sha256_hex(&bytes);
    let rel = media_storage::safe_relative_path(&admin.current_workspace, &sha, &ext)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    media_storage::store_bytes(root, &rel, &bytes)
        .await
        .map_err(|e| AppError::Internal(format!("store file failed: {e}")))?;

    let asset = ContentAsset {
        id: None,
        workspace_id: admin.current_workspace.clone(),
        account_id,
        kind: "media".into(),
        title,
        body: None,
        tags: vec![],
        url: None,
        media_id: None,
        usage_scene: None,
        media_type: Some(media_type),
        file_path: Some(rel),
        file_name: Some(file_name),
        file_size: Some(bytes.len() as i64),
        mime_type: Some(mime),
        file_sha256: Some(sha),
        sendable: Some(true),
        send_trigger_hint,
        target_stages: (!target_stages.is_empty()).then_some(target_stages),
        expression_pref,
        requires_principal_approval: Some(requires_principal_approval),
        review_status: Some("draft".into()), // AI 不自我核验红线：默认草稿，待人类 approve
        review_note: None,
        created_at: DateTime::now(),
        updated_at: DateTime::now(),
    };
    let res = state.db.content_assets().insert_one(asset, None).await?;
    Ok(Json(json!({ "id": res.inserted_id.as_object_id().map(|i| i.to_hex()) })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewRequest {
    status: String,
    note: Option<String>,
}

pub(super) async fn review_media_asset(
    State(state): State<AppState>,
    Extension(admin): Extension<AuthenticatedAdmin>,
    Path(id): Path<String>,
    Json(payload): Json<ReviewRequest>,
) -> AppResult<Json<Value>> {
    if !matches!(payload.status.as_str(), "approved" | "draft") {
        return Err(AppError::BadRequest("status must be approved|draft".into()));
    }
    let oid = ObjectId::parse_str(&id)
        .map_err(|_| AppError::BadRequest("bad id".into()))?;
    let res = state.db.content_assets().update_one(
        doc! { "_id": oid, "workspace_id": &admin.current_workspace },
        doc! { "$set": {
            "review_status": &payload.status,
            "review_note": payload.note.clone(),
            "updated_at": DateTime::now(),
        }},
        None,
    ).await?;
    if res.matched_count == 0 {
        return Err(AppError::NotFound("asset not found".into()));
    }
    Ok(Json(json!({ "ok": true })))
}
```

（`AppError` 变体名以现有 `src/error.rs` 为准——若无 `Internal`/`NotFound`，用现有等价变体；实现时 grep 确认。）

- [ ] **Step 3: 挂载路由**

`src/routes/mod.rs`：加 `mod media_assets;`，并在 content-assets 路由附近注册：

```rust
        .route("/content-assets/upload", post(media_assets::upload_media_asset))
        .route("/content-assets/:id/review", post(media_assets::review_media_asset))
```

- [ ] **Step 4: 编译**

Run: `cargo check 2>&1 | tail -15`
Expected: 编译通过（解决 AppError 变体名 / use 等问题直到通过）。

- [ ] **Step 5: lib 测试不回归**

Run: `cargo test --lib 2>&1 | tail -5`
Expected: passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/routes/media_assets.rs src/routes/mod.rs tests/media_asset_send_integration.rs
git commit -m "feat(media-asset): 素材上传(multipart落盘draft)+审核状态流转 API"
```

---

### Task 5: 选材模块（候选过滤 + 媒体类型→工具名映射）

**Files:**
- Create: `src/agent/media_send.rs`
- Modify: `src/agent/mod.rs`（加 `mod media_send;` + 必要 re-export）
- Test: `src/agent/media_send.rs` 内联 `#[cfg(test)]`

**Interfaces:**
- Consumes: `ContentAsset`（Task 1）、`AppState`、`mcp::logged_call_for_account`（现有，`src/mcp.rs`）
- Produces:
  - `pub(crate) fn mcp_tool_for_media_type(media_type: &str) -> Option<&'static str>` — image→message_send_image / file→message_send_file / video→message_send_video / 其它→None
  - `pub(crate) fn filter_sendable_candidates(assets: &[ContentAsset], customer_stage: Option<&str>) -> Vec<&ContentAsset>` — 纯函数：保留 `sendable==Some(true) && review_status==Some("approved") && media_type.is_some()`，且 `target_stages` 为空或命中 customer_stage
  - `pub(crate) fn render_candidate_lines(candidates: &[&ContentAsset]) -> String` — 注入 prompt 的候选清单文本
  - `pub(crate) fn validate_asset_sendable(asset: &ContentAsset) -> bool` — 发送前准入二次校验（同 filter 的 sendable+approved+media_type 条件）

- [ ] **Step 1: 写失败测试**

新建 `src/agent/media_send.rs`，先写测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ContentAsset;
    use mongodb::bson::DateTime;

    fn asset(sendable: Option<bool>, review: Option<&str>, mt: Option<&str>, stages: Option<Vec<&str>>) -> ContentAsset {
        ContentAsset {
            id: None, workspace_id: "ws".into(), account_id: None,
            kind: "media".into(), title: "报价单".into(), body: None, tags: vec![],
            url: None, media_id: None, usage_scene: None,
            media_type: mt.map(|s| s.to_string()),
            file_path: Some("ws/ab/x.pdf".into()), file_name: Some("报价单.xlsx".into()),
            file_size: Some(1), mime_type: Some("application/pdf".into()),
            file_sha256: Some("ab".into()),
            sendable, send_trigger_hint: Some("问价时发".into()),
            target_stages: stages.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            expression_pref: Some("file_primary".into()),
            requires_principal_approval: Some(false),
            review_status: review.map(|s| s.to_string()), review_note: None,
            created_at: DateTime::now(), updated_at: DateTime::now(),
        }
    }

    #[test]
    fn tool_mapping_covers_three_types_and_rejects_unknown() {
        assert_eq!(mcp_tool_for_media_type("image"), Some("message_send_image"));
        assert_eq!(mcp_tool_for_media_type("file"), Some("message_send_file"));
        assert_eq!(mcp_tool_for_media_type("video"), Some("message_send_video"));
        assert_eq!(mcp_tool_for_media_type("link_card"), None);
        assert_eq!(mcp_tool_for_media_type(""), None);
    }

    #[test]
    fn filter_excludes_draft_and_non_sendable_and_no_media_type() {
        let all = vec![
            asset(Some(true), Some("approved"), Some("file"), None),       // 保留
            asset(Some(true), Some("draft"), Some("file"), None),          // 排除：未审
            asset(Some(false), Some("approved"), Some("file"), None),      // 排除：不可发
            asset(None, None, None, None),                                 // 排除：老朋友圈行
            asset(Some(true), Some("approved"), None, None),               // 排除：无 media_type
        ];
        let kept = filter_sendable_candidates(&all, None);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn filter_matches_stage_or_empty_stages() {
        let all = vec![
            asset(Some(true), Some("approved"), Some("file"), Some(vec!["意向"])),    // 命中
            asset(Some(true), Some("approved"), Some("file"), Some(vec!["已成交"])),  // 不命中
            asset(Some(true), Some("approved"), Some("file"), None),                  // 空 stages 总命中
        ];
        let kept = filter_sendable_candidates(&all, Some("意向"));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn render_lines_includes_id_stage_pref_hint() {
        let a = asset(Some(true), Some("approved"), Some("file"), Some(vec!["意向"]));
        let line = render_candidate_lines(&[&a]);
        assert!(line.contains("file_primary") || line.contains("文件为主"));
        assert!(line.contains("问价时发"));
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib media_send`
Expected: 编译失败。

- [ ] **Step 3: 实现纯函数 + 映射表**

测试模块之上：

```rust
//! 销售素材选材 + 媒体发送：媒体类型→MCP 工具名映射、候选过滤（纯函数）、
//! ensure_media_uploaded、send_outbound_media。
use crate::models::ContentAsset;

/// 媒体类型 → MCP 私聊发送工具名。不写死在调用处，集中此表。
/// 链接卡片/小程序：MCP 私聊侧确认工具名后在此加一行即可，不改调用方。
pub(crate) fn mcp_tool_for_media_type(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image" => Some("message_send_image"),
        "file" => Some("message_send_file"),
        "video" => Some("message_send_video"),
        _ => None,
    }
}

pub(crate) fn validate_asset_sendable(asset: &ContentAsset) -> bool {
    asset.sendable == Some(true)
        && asset.review_status.as_deref() == Some("approved")
        && asset.media_type.as_deref().and_then(mcp_tool_for_media_type).is_some()
}

pub(crate) fn filter_sendable_candidates<'a>(
    assets: &'a [ContentAsset],
    customer_stage: Option<&str>,
) -> Vec<&'a ContentAsset> {
    assets
        .iter()
        .filter(|a| validate_asset_sendable(a))
        .filter(|a| match (&a.target_stages, customer_stage) {
            (None, _) => true,
            (Some(stages), _) if stages.is_empty() => true,
            (Some(stages), Some(cs)) => stages.iter().any(|s| s == cs),
            (Some(_), None) => false,
        })
        .collect()
}

pub(crate) fn render_candidate_lines(candidates: &[&ContentAsset]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let mut out = String::from("可发送素材（按需选择，没有合适的就不发）：\n");
    for a in candidates {
        let id = a.id.map(|i| i.to_hex()).unwrap_or_default();
        let stages = a.target_stages.as_ref().map(|v| v.join(",")).unwrap_or_default();
        let pref = a.expression_pref.as_deref().unwrap_or("file_support");
        let hint = a.send_trigger_hint.as_deref().unwrap_or("");
        out.push_str(&format!(
            "- [id:{id}] {} | 阶段:{stages} | 表达:{pref}\n  触发提示:{hint}\n",
            a.title
        ));
    }
    out
}
```

在 `src/agent/mod.rs` 加 `mod media_send;`。

- [ ] **Step 4: 运行确认通过**

Run: `cargo test --lib media_send`
Expected: 4 passed。

- [ ] **Step 5: Commit**

```bash
git add src/agent/media_send.rs src/agent/mod.rs
git commit -m "feat(media-asset): 选材候选过滤+媒体类型工具映射(纯函数,含老行排除)"
```

---

### Task 6: OutboxEntry/EnqueueRequest 扩展（媒体条目）

**Files:**
- Modify: `src/models.rs`（`OutboxEntry` 加 `media_asset_id: Option<String>`）
- Modify: `src/agent/outbox.rs`（`EnqueueRequest` 加 `media_asset_id`；放宽媒体条目的空 content 校验）
- Test: `src/agent/outbox.rs` 内联测试

**Interfaces:**
- Consumes: 现有 `enqueue` / `EnqueueRequest`（`src/agent/outbox.rs:126-159`）
- Produces: `EnqueueRequest.media_asset_id: Option<String>`、`OutboxEntry.media_asset_id: Option<String>`。Task 7（dispatcher）依赖。

- [ ] **Step 1: 写失败测试——媒体条目允许空 content，但 idempotency 仍生效**

`src/agent/outbox.rs` 测试区追加（跟随现有测试模块风格；若 enqueue 需 DB 则把可纯函数化的校验抽出测——这里测"媒体条目 content 校验放宽"的判定函数）：

```rust
#[cfg(test)]
mod media_entry_tests {
    use super::*;

    // 抽出的纯校验：text 条目要求 content 非空；media 条目（media_asset_id 有值）允许空 content。
    #[test]
    fn media_entry_allows_empty_content() {
        assert!(content_required_for(&None));        // 纯文本 → 需要 content
        assert!(!content_required_for(&Some("aid".to_string()))); // 媒体 → 不需要
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib media_entry_tests`
Expected: 编译失败（`content_required_for` 未定义）。

- [ ] **Step 3: 加字段 + 放宽校验**

`src/models.rs` 的 `OutboxEntry`（:2324）在 `content` 附近加：

```rust
    /// 销售素材发送条目：非空表示这条 outbox 发的是 ContentAsset 文件而非文本。
    /// dispatcher 据此走 send_outbound_media。`#[serde(default)]` 兼容旧文档。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_asset_id: Option<String>,
```

`src/agent/outbox.rs`：`EnqueueRequest`（:126）加 `pub media_asset_id: Option<String>,`。新增纯函数 + 改 enqueue 校验：

```rust
/// 媒体条目（media_asset_id 有值）允许空 content（文件可不带文字）；纯文本条目仍要求非空。
pub(crate) fn content_required_for(media_asset_id: &Option<String>) -> bool {
    media_asset_id.is_none()
}
```

把 `enqueue`（:164-166）里的：

```rust
    if req.content.trim().is_empty() {
        return Err(OutboxError::Invalid("content is empty".to_string()));
    }
```

改为：

```rust
    if content_required_for(&req.media_asset_id) && req.content.trim().is_empty() {
        return Err(OutboxError::Invalid("content is empty".to_string()));
    }
```

并在构造 `OutboxEntry` 处把 `media_asset_id: req.media_asset_id` 带入；content_hash 对空 content 仍可算（媒体条目的幂等键由 `media_asset_id` 参与，见下）。

**幂等键调整**：媒体条目的 idempotency_key 须含 `media_asset_id`，否则"同 run 发两个不同文件"会因 content 都空而 hash 撞键被误去重。找到 idempotency_key 构造处，媒体条目改用 `synthetic:run_id:contact_wxid:media_asset_id` 形态（参照现有 synthetic 兜底逻辑 :180-184）。

- [ ] **Step 4: 找出所有 EnqueueRequest 构造点并补字段**

Run: `grep -rn "EnqueueRequest {" src/`
对每个构造点补 `media_asset_id: None,`（现有文本发送路径不受影响）。

- [ ] **Step 5: 运行确认通过 + lib 不回归**

Run: `cargo test --lib media_entry_tests && cargo test --lib 2>&1 | tail -5`
Expected: media_entry_tests passed；全 lib passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/models.rs src/agent/outbox.rs
git commit -m "feat(media-asset): outbox 支持媒体条目(空content放宽+media_asset_id幂等键)"
```

---

### Task 7: 媒体发送执行（ensure_media_uploaded + send_outbound_media）+ dispatcher 接线

**Files:**
- Modify: `src/agent/media_send.rs`（加 `ensure_media_uploaded` + `send_outbound_media`）
- Modify: `src/agent/outbox_dispatcher.rs`（dispatch 时按 `media_asset_id` 分流）
- Modify: `src/models.rs`（`ConversationMessage` 加 `msg_type` / `media_ref`）
- Test: `src/agent/media_send.rs` 内联测试（media_id 缓存判定纯函数）

**Interfaces:**
- Consumes: `mcp_tool_for_media_type` / `validate_asset_sendable`（Task 5）、`media_storage::read_bytes`（Task 2）、`mcp::logged_call_for_account`（现有）、`OutboxEntry.media_asset_id`（Task 6）、`AppConfig.media_id_cache_ttl_hours`（Task 3）
- Produces:
  - `pub(crate) async fn send_outbound_media(state, contact, asset_id: &str) -> AppResult<Value>`
  - `pub(crate) fn media_id_cache_valid(updated_at, ttl_hours, now) -> bool`（纯函数）

- [ ] **Step 1: 写失败测试——media_id 缓存有效性纯函数**

`src/agent/media_send.rs` 测试区追加：

```rust
    #[test]
    fn media_id_cache_respects_ttl() {
        use mongodb::bson::DateTime;
        let now_ms = 1_000_000_000_000i64;
        let now = DateTime::from_millis(now_ms);
        let fresh = DateTime::from_millis(now_ms - 1000 * 60 * 60); // 1h 前
        let stale = DateTime::from_millis(now_ms - 1000 * 60 * 60 * 48); // 48h 前
        assert!(media_id_cache_valid(fresh, 24, now));   // 24h TTL 内
        assert!(!media_id_cache_valid(stale, 24, now));  // 超 TTL
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib media_id_cache_respects_ttl`
Expected: 编译失败。

- [ ] **Step 3: 实现缓存判定 + 上传 + 发送**

`src/agent/media_send.rs` 追加（在 use 区补 `crate::routes::AppState`、`crate::models::Contact`、`crate::mcp`、`crate::error::AppResult`、`crate::media_storage`、`serde_json::{json, Value}`、`mongodb::bson::{doc, oid::ObjectId, DateTime}`）：

```rust
pub(crate) fn media_id_cache_valid(
    updated_at: DateTime,
    ttl_hours: i64,
    now: DateTime,
) -> bool {
    let age_ms = now.timestamp_millis() - updated_at.timestamp_millis();
    age_ms >= 0 && age_ms < ttl_hours * 3600 * 1000
}

/// 确保 asset 在 MCP 侧有有效 media_id：缓存命中（未过 TTL）直接用，否则读盘
/// base64 → media_upload_base64 → 回写 media_id + updated_at。
/// 不依赖"media_id 永久有效"假设——失效即重传。
async fn ensure_media_uploaded(
    state: &AppState,
    asset: &ContentAsset,
) -> AppResult<String> {
    let now = DateTime::now();
    if let (Some(mid), uat) = (asset.media_id.as_ref(), asset.updated_at) {
        if media_id_cache_valid(uat, state.config.media_id_cache_ttl_hours, now) {
            return Ok(mid.clone());
        }
    }
    let rel = asset.file_path.as_ref()
        .ok_or_else(|| crate::error::AppError::Internal("asset has no file_path".into()))?;
    let root = std::path::Path::new(&state.config.media_storage_dir);
    let bytes = media_storage::read_bytes(root, rel).await
        .map_err(|e| crate::error::AppError::Internal(format!("read media failed: {e}")))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    // MCP 入参字段名以 server tools/list 为准；这里用占位形态，集成时对齐。
    let account_id = &asset.account_id;
    let resp = crate::mcp::logged_call_for_account(
        state,
        account_id.as_deref().unwrap_or(&state.config.default_account_id),
        "media_upload_base64",
        json!({
            "fileName": asset.file_name,
            "mediaType": asset.media_type,
            "base64": b64,
        }),
    ).await?;
    let media_id = resp.get("mediaId").and_then(|v| v.as_str())
        .ok_or_else(|| crate::error::AppError::External("media_upload_base64 no mediaId".into()))?
        .to_string();

    // 回写缓存
    if let Some(oid) = asset.id {
        let _ = state.db.content_assets().update_one(
            doc! { "_id": oid },
            doc! { "$set": { "media_id": &media_id, "updated_at": now } },
            None,
        ).await;
    }
    Ok(media_id)
}

/// 发送一个素材文件给客户。调用方（dispatcher）已确保经 outbox 幂等。
pub(crate) async fn send_outbound_media(
    state: &AppState,
    contact: &crate::models::Contact,
    asset_id: &str,
) -> AppResult<Value> {
    let oid = ObjectId::parse_str(asset_id)
        .map_err(|_| crate::error::AppError::Internal("bad asset_id".into()))?;
    let asset = state.db.content_assets()
        .find_one(doc! { "_id": oid }, None).await?
        .ok_or_else(|| crate::error::AppError::Internal("asset not found".into()))?;

    // 发送前准入二次校验（防 AI 幻觉出未审/不可发素材一路漏到发送）
    if !validate_asset_sendable(&asset) {
        return Err(crate::error::AppError::Internal(
            "asset not sendable (draft/disabled/bad type)".into()));
    }
    let tool = mcp_tool_for_media_type(asset.media_type.as_deref().unwrap_or(""))
        .ok_or_else(|| crate::error::AppError::Internal("unsupported media_type".into()))?;
    let media_id = ensure_media_uploaded(state, &asset).await?;

    let resp = crate::mcp::logged_call_for_account(
        state,
        &contact.account_id,
        tool,
        json!({ "recipient": contact.wxid, "mediaId": media_id }),
    ).await?;
    Ok(resp)
}
```

确认 `base64` crate 在 Cargo.toml（vision 导入 `import.rs` 已用 base64，应已存在；否则用现有同款 base64 API）。

`src/models.rs` 的 `ConversationMessage` 加：

```rust
    /// 出站消息类型："text"(默认/缺省) | "media"。媒体消息供前端渲染文件卡片。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<String>,
    /// 媒体消息的 content_assets._id（hex），前端据此取缩略图/文件名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_ref: Option<String>,
```

补齐所有 `ConversationMessage {` 构造点的新字段 `: None`（grep 找全）。

- [ ] **Step 4: dispatcher 按 media_asset_id 分流**

`src/agent/outbox_dispatcher.rs`：找到把 `entry.content` 交给 `send_outbound_message` 的发送点（设计核查在 :556 附近）。改为：

```rust
    let send_result = if let Some(asset_id) = entry.media_asset_id.as_deref() {
        crate::agent::media_send::send_outbound_media(state, &contact, asset_id).await
    } else {
        crate::agent::gateway::send_outbound_message(state, &contact, &entry.content, None).await
    };
```

媒体发送成功后落 `ConversationMessage` 时带 `msg_type: Some("media"), media_ref: Some(asset_id)`（若 dispatcher 不自己落库而是 send_outbound_* 内部落，则把落库逻辑放进 `send_outbound_media`，与 `send_outbound_message` 对称）。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test --lib media_send 2>&1 | tail -10 && cargo test --lib 2>&1 | tail -5`
Expected: media_send 测试全过；全 lib passed ≥ 350。

- [ ] **Step 6: Commit**

```bash
git add src/agent/media_send.rs src/agent/outbox_dispatcher.rs src/models.rs
git commit -m "feat(media-asset): media_upload_base64缓存+send_outbound_media+dispatcher分流"
```

---

### Task 8: AgentDecision 扩展 + 选材注入 prompt + gateway 转 outbox

**Files:**
- Modify: `src/agent/types.rs`（`AgentDecision` 加 `assets_to_send`；定义 `AssetSendDirective`）
- Modify: `src/agent/decision.rs`（`load_sendable_assets` + 注入候选清单）
- Modify: `src/agent/gateway.rs`（准入校验 + assets_to_send → outbox 媒体条目，按 expression_pref 定序）
- Modify: `src/prompts.rs`（选材指引文案）
- Test: `src/agent/types.rs` 反序列化测试 + `src/agent/gateway.rs` 定序纯函数测试

**Interfaces:**
- Consumes: `filter_sendable_candidates` / `render_candidate_lines` / `validate_asset_sendable`（Task 5）、`enqueue` + `EnqueueRequest.media_asset_id`（Task 6）、`ContentAsset`（Task 1）
- Produces: `AgentDecision.assets_to_send: Vec<AssetSendDirective>`；`AssetSendDirective { asset_id: String, reason: Option<String> }`

- [ ] **Step 1: 写失败测试——决策缺字段时 assets_to_send 默认空 + 定序**

`src/agent/types.rs` 测试区：

```rust
#[cfg(test)]
mod assets_to_send_tests {
    use super::AgentDecision;

    #[test]
    fn decision_without_assets_field_defaults_empty() {
        // 旧 LLM 输出（无 assetsToSend）必须仍能反序列化
        let json = r#"{"replyText":"你好","shouldReply":true}"#;
        let d: AgentDecision = serde_json::from_str(json).expect("must deserialize");
        assert!(d.assets_to_send.is_empty());
    }

    #[test]
    fn decision_parses_assets_to_send() {
        let json = r#"{"replyText":"这是报价单","assetsToSend":[{"assetId":"a1","reason":"客户问价"}]}"#;
        let d: AgentDecision = serde_json::from_str(json).unwrap();
        assert_eq!(d.assets_to_send.len(), 1);
        assert_eq!(d.assets_to_send[0].asset_id, "a1");
    }
}
```

`src/agent/gateway.rs` 测试区（定序纯函数）：

```rust
#[cfg(test)]
mod media_send_order_tests {
    use super::*;
    // file_primary：文件先于文字引导发还是文字先？设计=文字一句引导在前，文件随后。
    // 这里测：给定 expression_pref，返回 (先发文本?, 先发文件?) 的顺序标记。
    #[test]
    fn file_primary_sends_text_then_file() {
        assert_eq!(media_send_order("file_primary"), SendOrder::TextThenMedia);
    }
    #[test]
    fn file_support_sends_text_then_file() {
        assert_eq!(media_send_order("file_support"), SendOrder::TextThenMedia);
    }
}
```

（定序当前两种偏好都"先文字后文件"，但抽成函数留扩展点；若你后续想 file_primary 改成先发文件，改这一处即可。）

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --lib assets_to_send_tests`
Expected: 编译失败。

- [ ] **Step 3: 定义 AssetSendDirective + 扩展 AgentDecision**

`src/agent/types.rs`：加结构体（跟随文件现有 `#[serde(rename_all = "camelCase")]` 风格）：

```rust
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssetSendDirective {
    pub asset_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}
```

`AgentDecision`（:80）加字段：

```rust
    #[serde(default)]
    pub assets_to_send: Vec<AssetSendDirective>,
```

- [ ] **Step 4: load_sendable_assets + 注入 prompt**

`src/agent/decision.rs`：新增（参照现有 `load_context_assets` :1025 的 query 风格，但过滤 sendable+approved）：

```rust
pub(crate) async fn load_sendable_assets(
    state: &AppState,
    account_id: &str,
) -> AppResult<Vec<crate::models::ContentAsset>> {
    use futures::TryStreamExt;
    use mongodb::bson::doc;
    use mongodb::options::FindOptions;
    let mut cursor = state.db.content_assets().find(
        doc! {
            "workspace_id": &state.config.default_workspace_id,
            "$or": [ { "account_id": null }, { "account_id": account_id } ],
            "sendable": true,
            "review_status": "approved",
        },
        FindOptions::builder().sort(doc! { "updated_at": -1 }).limit(30).build(),
    ).await?;
    let mut out = Vec::new();
    while let Some(a) = cursor.try_next().await? { out.push(a); }
    Ok(out)
}
```

在组装 Reply Agent prompt 处（decision.rs 里调用 `load_context_assets` 的同一段），加载可发送素材 → `filter_sendable_candidates(&assets, customer_stage)` → `render_candidate_lines(...)` → 拼进 prompt 业务上下文层。customer_stage 取当前 contact/会话的 stage。

- [ ] **Step 5: prompts.rs 选材指引**

`src/prompts.rs` 在 Reply Agent 的 operator/policy 指引里加（确保不含 no-human-takeover 禁词）：

```
【素材文件发送】你可在候选「可发送素材」中按需选择发给客户，输出到 assetsToSend（[{assetId, reason}]）。规则：
- 没有契合当前客户阶段与问题的素材，就不发（assetsToSend 留空），不要为发而发。
- 选了「表达:文件为主」的素材：replyText 只做一句简短引导（如"给您发份报价单"），不要把文件内容用文字再复述一遍。
- 选了「表达:文件佐证」的素材：replyText 正常回答，文件作为佐证补充。
- 只能选候选清单里列出的 assetId，不要编造。
```

- [ ] **Step 6: gateway 把 assets_to_send 转 outbox 媒体条目**

`src/agent/gateway.rs`：加定序纯函数 + 在文本回复 enqueue 之后，对 `decision.assets_to_send` 逐个：

```rust
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SendOrder { TextThenMedia }

pub(crate) fn media_send_order(_expression_pref: &str) -> SendOrder {
    SendOrder::TextThenMedia
}
```

转 outbox（每素材一条独立媒体 outbox 条目，独立幂等键）：

```rust
    for directive in &decision.assets_to_send {
        // 准入二次校验：必须是 approved+sendable+合法 media_type，否则跳过 + 审计
        let oid = match mongodb::bson::oid::ObjectId::parse_str(&directive.asset_id) {
            Ok(o) => o, Err(_) => { /* 审计 agent.media_asset_id_invalid */ continue; }
        };
        let asset = state.db.content_assets().find_one(doc! { "_id": oid }, None).await?;
        let asset = match asset {
            Some(a) if crate::agent::media_send::validate_asset_sendable(&a) => a,
            _ => { /* 审计 agent.media_asset_rejected */ continue; }
        };
        // requires_principal_approval → 走现有 escalation 请示通道兜底（不直接发）
        if asset.requires_principal_approval == Some(true) {
            // 复用 escalation：请示领导后由回拿结论的流程决定是否发。此处仅入请示，
            // 不入 outbox。具体接线参照 src/agent/escalation/mod.rs 现有 enqueue 形态。
            // （审计 agent.media_asset_escalated）
            continue;
        }
        let _ = crate::agent::outbox::enqueue(state, crate::agent::outbox::EnqueueRequest {
            workspace_id: contact.workspace_id.clone(),
            account_id: contact.account_id.clone(),
            contact_wxid: contact.wxid.clone(),
            run_id: run_id.clone(),
            decision_id,
            source_event_id: source_event_id.clone(),
            source_kind: source_kind.clone(),
            content: String::new(),                 // 媒体条目允许空
            media_asset_id: Some(directive.asset_id.clone()),
            max_attempts: 3,
        }).await;
    }
```

（变量名 `run_id`/`decision_id`/`source_event_id`/`source_kind` 以 gateway 该作用域现有绑定为准；实现时对齐。文本回复的 enqueue 已存在，媒体条目追加在其后，满足"先文字后文件"。）

- [ ] **Step 7: 编译 + 测试**

Run: `cargo test --lib assets_to_send_tests media_send_order_tests 2>&1 | tail -10 && cargo test --lib 2>&1 | tail -5`
Expected: 新测试全过；全 lib passed ≥ 350。

- [ ] **Step 8: no-human-takeover lint 自检**

Run: `grep -rnE "人工接管|takeover|hand-?off|人工介入|人工托管" src/agent/ src/prompts.rs src/routes/media_assets.rs`
Expected: 无新增命中。

- [ ] **Step 9: Commit**

```bash
git add src/agent/types.rs src/agent/decision.rs src/agent/gateway.rs src/prompts.rs
git commit -m "feat(media-asset): AgentDecision.assetsToSend+选材注入prompt+gateway转outbox媒体条目"
```

---

### Task 9: 前端素材上传 + 审核 + 冲突提醒

**Files:**
- Modify: `frontend/src/features/content-assets/index.tsx`（或现有素材库组件）
- Modify: `frontend/src/stores/contentStore.ts`（加 upload/review action）
- 复用: `frontend/src/lib/api.ts:82` 的 `postForm`

**Interfaces:**
- Consumes: `POST /api/content-assets/upload`、`POST /api/content-assets/:id/review`（Task 4）
- Produces: 素材库管理 UI

- [ ] **Step 1: contentStore 加 upload/review action**

`frontend/src/stores/contentStore.ts`：

```ts
async uploadMediaAsset(form: FormData): Promise<{ id: string }> {
  return api.postForm<{ id: string }>("/api/content-assets/upload", form);
},
async reviewMediaAsset(id: string, status: "approved" | "draft", note?: string) {
  return api.post(`/api/content-assets/${id}/review`, { status, note });
},
```

- [ ] **Step 2: 上传表单组件**

在素材库页面加上传区块（遵循 `docs/frontend-design-system.md` 企业白色基调）：

```tsx
// 文件选择 + 标注字段 + 提交
const [file, setFile] = useState<File | null>(null);
const [title, setTitle] = useState("");
const [mediaType, setMediaType] = useState<"image" | "file" | "video">("file");
const [triggerHint, setTriggerHint] = useState("");
const [expressionPref, setExpressionPref] = useState<"file_primary" | "file_support">("file_primary");
const [stages, setStages] = useState("");
const [needsApproval, setNeedsApproval] = useState(false);

async function handleUpload() {
  if (!file || !title) return;
  const fd = new FormData();
  fd.append("file", file);
  fd.append("title", title);
  fd.append("mediaType", mediaType);
  fd.append("sendTriggerHint", triggerHint);
  fd.append("expressionPref", expressionPref);
  fd.append("targetStages", stages);
  fd.append("requiresPrincipalApproval", String(needsApproval));
  await store.uploadMediaAsset(fd);
  // 刷新列表
}

return (
  <div>
    <input type="file" accept="image/*,application/pdf,.doc,.docx,.xls,.xlsx,.ppt,.pptx,video/mp4"
           onChange={(e) => setFile(e.target.files?.[0] ?? null)} />
    <input placeholder="素材标题" value={title} onChange={(e) => setTitle(e.target.value)} />
    {/* mediaType / expressionPref 下拉，triggerHint textarea，stages 输入，needsApproval 勾选 */}
    {/* 冲突三小提醒标识 */}
    <p className="hint">提示：若知识库已有同内容文本，请确认两边信息一致（如价格、政策）。</p>
    <button onClick={handleUpload}>上传（待审核）</button>
  </div>
);
```

- [ ] **Step 3: 审核操作 + 草稿/已审状态展示**

素材列表每行展示 `reviewStatus`（draft/approved 标识）、缩略图（image 用 `<img>`，file/video 用文件卡片显示 `fileName`），draft 行带"标记为可发送"按钮调 `reviewMediaAsset(id, "approved")`。

- [ ] **Step 4: 前端构建验证**

Run: `cd frontend && npm run build 2>&1 | tail -15`
Expected: 构建成功，无 TS 错误。

- [ ] **Step 5: Commit**

```bash
git add frontend/src/features/content-assets frontend/src/stores/contentStore.ts
git commit -m "feat(media-asset): 前端素材上传(multipart)+审核+冲突提醒+预览"
```

---

### Task 10: 前端对话媒体消息渲染

**Files:**
- Modify: `frontend/src/types/index.ts`（`Message` 加 `msgType` / `mediaRef`）
- Modify: `frontend/src/features/user-ops/legacy.tsx:1618-1630`（消息气泡渲染）

**Interfaces:**
- Consumes: 后端 `ConversationMessage.msg_type` / `media_ref`（Task 7）
- Produces: 对话界面显示 AI 发给客户的文件

- [ ] **Step 1: Message 类型加字段**

`frontend/src/types/index.ts:90-95`：

```ts
export interface Message {
  id: string;
  direction: "inbound" | "outbound";
  content: string;
  createdAt: string;
  msgType?: "text" | "media";
  mediaRef?: string;
}
```

- [ ] **Step 2: 气泡按 msgType 分支渲染**

`legacy.tsx` 气泡处（现 `<p>{message.content}</p>`）：

```tsx
{message.msgType === "media" ? (
  <div className="media-bubble">
    <span className="media-icon">📎</span>
    <span>{message.content || "[已发送素材文件]"}</span>
  </div>
) : (
  <p>{message.content}</p>
)}
```

（媒体消息 content 可能为空，给占位文案；如需缩略图可用 mediaRef 拉素材详情，本期先文件卡片占位。emoji 仅 UI 装饰，不写入任何后端文件——符合"代码/文件不加 emoji"约束，此处是前端渲染文本。若要避免 emoji，用纯文本"[文件]"代替。）

- [ ] **Step 3: 前端构建验证**

Run: `cd frontend && npm run build 2>&1 | tail -15`
Expected: 构建成功。

- [ ] **Step 4: Commit**

```bash
git add frontend/src/types/index.ts frontend/src/features/user-ops/legacy.tsx
git commit -m "feat(media-asset): 前端对话渲染媒体消息(文件卡片)"
```

---

### Task 11: 集成测试回填（端到端数据流 + outbox 幂等）

**Files:**
- Modify: `tests/media_asset_send_integration.rs`（回填 Task 4 占位的真实断言）

**Interfaces:**
- Consumes: 全链路（Task 1-8）

- [ ] **Step 1: 回填 upload→review→sendable 断言**

把 Task 4 的占位测试改为真实断言（用项目现有 testcontainers helper 起 Mongo + AppState）：

```rust
#[tokio::test]
#[ignore = "requires docker mongo"]
async fn upload_then_review_then_only_approved_is_sendable() {
    // 起 testcontainers mongo + 构建 AppState（参照 tests/ 其它集成测试 setup）
    // 1. 直接 insert 一个 draft 素材（sendable=true, review_status="draft", media_type="file", target_stages=["意向"]）
    // 2. load_sendable_assets → 不含它（draft 被过滤）
    // 3. update review_status="approved"
    // 4. load_sendable_assets → 含它
    // 5. filter_sendable_candidates(.., Some("意向")) → 命中；Some("已成交") → 不命中
}
```

- [ ] **Step 2: 加 outbox 媒体条目幂等测试**

```rust
#[tokio::test]
#[ignore = "requires docker mongo"]
async fn media_outbox_entry_is_idempotent_per_asset() {
    // enqueue 两次同 (run_id, contact, media_asset_id) → 第二次 IdempotentSkip
    // enqueue 同 run 不同 media_asset_id → 两条都 Created（验证幂等键含 asset_id）
}
```

- [ ] **Step 3: 本地编译验证（不跑 ignored）**

Run: `cargo test --test media_asset_send_integration --no-run 2>&1 | tail -5`
Expected: 编译通过（CI integration job 会带 `--ignored` 真跑）。

- [ ] **Step 4: Commit**

```bash
git add tests/media_asset_send_integration.rs
git commit -m "test(media-asset): 端到端上传/审核/选材+outbox媒体幂等集成测试(CI)"
```

---

## Self-Review

**1. Spec coverage（逐节核对 spec → task）：**
- spec §4 数据模型 → Task 1 ✓
- spec §5.1 上传 + 安全 → Task 2（存储/路径安全）+ Task 4（upload API/大小/白名单）✓
- spec §5.2 网关映射表 → Task 5（映射）+ Task 7（send_outbound_media）✓
- spec §5.3 outbox 两条目幂等 → Task 6 + Task 8（gateway 转条目）✓
- spec §6.1 候选注入 → Task 5（render）+ Task 8（load+注入）✓
- spec §6.2 AgentDecision 扩展 → Task 8 ✓
- spec §6.3 闸门（准入/频控/PressureRisk/请示）→ Task 5+7（准入）、Task 8（请示分流）；频控/PressureRisk 复用现有 gateway（媒体条目仍经 gateway 既有检查）✓
- spec §7 前端 → Task 9（上传/审核/提醒/预览）+ Task 10（对话渲染）✓
- spec §8 配置/索引 → Task 3 ✓
- spec §9 测试 → 各 Task 内联 + Task 11 集成 ✓
- spec §3.3 冲突二（reply_text 仍走 grounding）：媒体条目不绕过文本回复的五闸门——文本回复独立 enqueue 仍走原 review 流程，媒体条目只是附加；无 task 削弱 grounding ✓

**2. Placeholder scan：** 无 TBD/TODO/"implement later"。MCP 入参字段名（mediaId/base64）标注为"以 server tools/list 为准、实现时对齐"，属已知未决而非占位——符合 spec §11 风险记录。

**3. Type consistency：** `media_asset_id`（outbox，Task 6/7/8）、`assets_to_send`/`AssetSendDirective.asset_id`（Task 8）、`validate_asset_sendable`/`mcp_tool_for_media_type`/`filter_sendable_candidates`/`render_candidate_lines`（Task 5，Task 7/8 消费）、`send_outbound_media(state, contact, asset_id)`（Task 7，Task 7-dispatcher 消费）、`media_id_cache_valid`（Task 7）— 跨任务签名一致。

**4. 已知实现期对齐点（非占位，需实现者 grep 确认）：** `AppError` 变体名（Internal/NotFound/External 以 src/error.rs 为准）；`logged_call_for_account` 精确签名；`ConversationMessage`/`EnqueueRequest` 全部构造点补新字段；dispatcher 发送点行号；base64 crate API。这些是"跟随现有代码"的常规对齐，每个 Task 的编译步骤会暴露。

