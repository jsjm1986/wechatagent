//! 销售素材本地文件存储：安全路径构造（防穿越）、sha256、原子发布与一致性扫描。
use futures::TryStreamExt;
use mongodb::bson::{doc, Document};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::db::Database;

const PENDING_SUFFIX: &str = ".wa-pending";
pub const RECONCILE_INTERVAL_SECS: u64 = 60 * 60;

static PATH_LOCKS: LazyLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub recovered_pending: u64,
    pub removed_pending: u64,
    pub removed_orphans: u64,
    pub removed_corrupt: u64,
    pub disabled_missing_assets: u64,
    pub disabled_invalid_assets: u64,
}

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

/// 仅允许 [A-Za-z0-9_-] 的 segment（workspace_id / sha 都应满足；含 . / 或其它即拒）。
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Validate a persisted relative path before joining it to the configured root.
/// Old/corrupt DB values must never turn cleanup or reads into path traversal.
pub fn is_safe_relative_path(rel: &str) -> bool {
    let path = Path::new(rel);
    !path.is_absolute()
        && !rel.is_empty()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
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
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
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
    ensure_safe_relative(rel)?;
    let full: PathBuf = root.join(rel);
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(full, bytes).await
}

pub async fn read_bytes(root: &Path, rel: &str) -> std::io::Result<Vec<u8>> {
    ensure_safe_relative(rel)?;
    tokio::fs::read(root.join(rel)).await
}

fn ensure_safe_relative(rel: &str) -> std::io::Result<()> {
    if is_safe_relative_path(rel) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsafe media relative path",
        ))
    }
}

fn absolute_lock_key(root: &Path, rel: &str) -> std::io::Result<String> {
    ensure_safe_relative(rel)?;
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()?.join(root)
    };
    Ok(root.join(rel).to_string_lossy().into_owned())
}

async fn path_lock(root: &Path, rel: &str) -> std::io::Result<OwnedMutexGuard<()>> {
    let key = absolute_lock_key(root, rel)?;
    let lock = {
        let mut locks = PATH_LOCKS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, weak| weak.strong_count() > 0);
        match locks.get(&key).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        }
    };
    Ok(lock.lock_owned().await)
}

/// Lock one or more content-addressed paths in stable order. All reference
/// creation/release and physical publish/delete operations use this guard,
/// closing the single-process count-then-delete race.
pub async fn lock_paths(
    root: &Path,
    paths: impl IntoIterator<Item = String>,
) -> std::io::Result<Vec<OwnedMutexGuard<()>>> {
    let mut paths: Vec<String> = paths.into_iter().collect();
    paths.sort();
    paths.dedup();
    let mut guards = Vec::with_capacity(paths.len());
    for rel in paths {
        guards.push(path_lock(root, &rel).await?);
    }
    Ok(guards)
}

pub fn pending_relative_path(rel: &str) -> std::io::Result<String> {
    ensure_safe_relative(rel)?;
    Ok(format!("{rel}{PENDING_SUFFIX}"))
}

fn final_relative_from_pending(rel: &str) -> Option<&str> {
    rel.strip_suffix(PENDING_SUFFIX)
}

/// Stage bytes beside the final object and fsync them before any Mongo write.
/// Returns false when an already-published content-addressed object exists.
pub async fn stage_bytes(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<bool> {
    ensure_safe_relative(rel)?;
    let final_path = root.join(rel);
    match managed_object_state(root, rel).await? {
        ManagedObjectState::Valid => return Ok(false),
        ManagedObjectState::Corrupt => delete_path_if_exists(&final_path).await?,
        ManagedObjectState::Missing => {}
    }
    let pending_rel = pending_relative_path(rel)?;
    let pending_path = root.join(pending_rel);
    if let Some(parent) = pending_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&pending_path)
        .await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(true)
}

/// Atomically publish a staged object. If another completed object is already
/// present, the redundant pending file is removed (content path is SHA based).
pub async fn publish_staged(root: &Path, rel: &str) -> std::io::Result<()> {
    ensure_safe_relative(rel)?;
    let final_path = root.join(rel);
    let pending_path = root.join(pending_relative_path(rel)?);
    match managed_object_state(root, rel).await? {
        ManagedObjectState::Valid => return delete_path_if_exists(&pending_path).await,
        ManagedObjectState::Corrupt => delete_path_if_exists(&final_path).await?,
        ManagedObjectState::Missing => {}
    }
    let expected_sha = sha_from_managed_path(rel).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "media path does not contain a valid sha256",
        )
    })?;
    let pending_bytes = tokio::fs::read(&pending_path).await?;
    if sha256_hex(&pending_bytes) != expected_sha {
        delete_path_if_exists(&pending_path).await?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "staged media sha256 mismatch",
        ));
    }
    tokio::fs::rename(pending_path, final_path).await
}

pub async fn discard_staged(root: &Path, rel: &str) -> std::io::Result<()> {
    let pending = root.join(pending_relative_path(rel)?);
    delete_path_if_exists(&pending).await
}

/// Resolve a staged file after a Mongo mutation failed while the caller still
/// holds the path lock. Existing references win; no references means cleanup.
/// A query failure leaves the pending file for the periodic reconciler.
pub async fn settle_staged_after_db_failure(
    db: &Database,
    root: &Path,
    rel: &str,
) -> anyhow::Result<()> {
    ensure_safe_relative(rel)?;
    let references = db
        .raw()
        .collection::<Document>("content_assets")
        .count_documents(doc! { "kind": "media", "file_path": rel }, None)
        .await?;
    if references == 0 {
        discard_staged(root, rel).await?;
    } else {
        publish_staged(root, rel).await?;
    }
    Ok(())
}

/// Recover the narrow crash window where Mongo references the final path but
/// the same-directory atomic rename had not happened yet.
pub async fn recover_pending_file(root: &Path, rel: &str) -> std::io::Result<bool> {
    let _guards = lock_paths(root, [rel.to_string()]).await?;
    let final_path = root.join(rel);
    if tokio::fs::try_exists(&final_path).await? {
        return Ok(true);
    }
    let pending = root.join(pending_relative_path(rel)?);
    if !tokio::fs::try_exists(&pending).await? {
        return Ok(false);
    }
    publish_staged(root, rel).await?;
    Ok(true)
}

/// Read a referenced object while holding its path lock. This closes the race
/// where delete or replacement removes it after recovery but before open.
pub async fn read_bytes_recovering(root: &Path, rel: &str) -> std::io::Result<Vec<u8>> {
    let _guards = lock_paths(root, [rel.to_string()]).await?;
    let final_path = root.join(rel);
    let was_corrupt = match managed_object_state(root, rel).await? {
        ManagedObjectState::Valid => return read_bytes(root, rel).await,
        ManagedObjectState::Corrupt => {
            delete_path_if_exists(&final_path).await?;
            true
        }
        ManagedObjectState::Missing => false,
    };
    let pending = root.join(pending_relative_path(rel)?);
    if tokio::fs::try_exists(&pending).await? {
        publish_staged(root, rel).await?;
        return read_bytes(root, rel).await;
    }
    Err(std::io::Error::new(
        if was_corrupt {
            std::io::ErrorKind::InvalidData
        } else {
            std::io::ErrorKind::NotFound
        },
        if was_corrupt {
            "published media sha256 mismatch"
        } else {
            "published media object missing"
        },
    ))
}

/// 物理删除素材文件。文件不存在（已被删/从未落盘）视为成功——幂等。
/// 调用方须先确认无其它 content_asset 记录引用同 rel（见 should_delete_physical_file）。
pub async fn delete_bytes(root: &Path, rel: &str) -> std::io::Result<()> {
    ensure_safe_relative(rel)?;
    delete_path_if_exists(&root.join(rel)).await
}

async fn delete_path_if_exists(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn collect_managed_files(root: &Path) -> std::io::Result<Vec<String>> {
    fn walk(root: &Path, dir: &Path, output: &mut Vec<String>) -> std::io::Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(root, &path, output)?;
            } else if entry.file_type()?.is_file() {
                if let Ok(relative) = path.strip_prefix(root) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    if is_managed_layout(&relative) {
                        output.push(relative);
                    }
                }
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    Ok(files)
}

fn is_managed_layout(rel: &str) -> bool {
    let final_rel = final_relative_from_pending(rel).unwrap_or(rel);
    let parts: Vec<&str> = final_rel.split('/').collect();
    if parts.len() != 3 || !is_safe_segment(parts[0]) || !is_safe_segment(parts[1]) {
        return false;
    }
    let Some((sha, ext)) = parts[2].rsplit_once('.') else {
        return false;
    };
    sha.len() == 64
        && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        && parts[1] == &sha[..2]
        && is_safe_segment(ext)
}

fn sha_from_managed_path(rel: &str) -> Option<&str> {
    if !is_managed_layout(rel) {
        return None;
    }
    Path::new(rel).file_stem()?.to_str()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedObjectState {
    Missing,
    Valid,
    Corrupt,
}

async fn managed_object_state(root: &Path, rel: &str) -> std::io::Result<ManagedObjectState> {
    ensure_safe_relative(rel)?;
    let expected_sha = sha_from_managed_path(rel).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "media path does not contain a valid sha256",
        )
    })?;
    match tokio::fs::read(root.join(rel)).await {
        Ok(bytes) if sha256_hex(&bytes) == expected_sha => Ok(ManagedObjectState::Valid),
        Ok(_) => Ok(ManagedObjectState::Corrupt),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ManagedObjectState::Missing)
        }
        Err(error) => Err(error),
    }
}

async fn fail_close_assets(
    collection: &mongodb::Collection<Document>,
    rel: &str,
    reason: &str,
) -> mongodb::error::Result<u64> {
    let result = collection
        .update_many(
            doc! {
                "kind": "media",
                "file_path": rel,
                "$or": [
                    { "sendable": { "$ne": false } },
                    { "review_status": { "$ne": "draft" } },
                    { "review_note": { "$ne": reason } },
                    { "media_id": { "$exists": true } },
                ],
            },
            doc! {
                "$set": {
                    "sendable": false,
                    "review_status": "draft",
                    "review_note": reason,
                    "updated_at": mongodb::bson::DateTime::now(),
                },
                "$unset": { "media_id": "" },
            },
            None,
        )
        .await?;
    Ok(result.modified_count)
}

/// Reconcile local files against Mongo references. Pending files repair the
/// DB-commit-before-rename crash window; unreferenced files are removed; DB
/// rows with no final or pending object are made non-sendable and draft.
pub async fn reconcile_once(db: &Database, root: &Path) -> anyhow::Result<ReconcileReport> {
    let mut report = ReconcileReport::default();
    let collection = db.raw().collection::<Document>("content_assets");
    let mut refs = HashSet::new();
    let mut invalid_paths = HashSet::new();
    let mut cursor = collection
        .find(
            doc! { "kind": "media", "file_path": { "$type": "string" } },
            None,
        )
        .await?;
    while let Some(asset) = cursor.try_next().await? {
        if let Ok(rel) = asset.get_str("file_path") {
            if is_safe_relative_path(rel) && is_managed_layout(rel) {
                refs.insert(rel.to_string());
            } else {
                invalid_paths.insert(rel.to_string());
            }
        }
    }

    for rel in invalid_paths {
        report.disabled_invalid_assets +=
            fail_close_assets(&collection, &rel, "storage_path_invalid").await?;
    }

    let files = tokio::task::spawn_blocking({
        let root = root.to_path_buf();
        move || collect_managed_files(&root)
    })
    .await??;

    for rel in &refs {
        let _guards = lock_paths(root, [rel.clone()]).await?;
        let current_references = collection
            .count_documents(doc! { "kind": "media", "file_path": rel }, None)
            .await?;
        if current_references == 0 {
            let pending = root.join(pending_relative_path(rel)?);
            if tokio::fs::try_exists(&pending).await? {
                delete_path_if_exists(&pending).await?;
                report.removed_pending += 1;
            }
            continue;
        }
        match managed_object_state(root, rel).await? {
            ManagedObjectState::Valid => {
                let pending = root.join(pending_relative_path(rel)?);
                if tokio::fs::try_exists(&pending).await? {
                    delete_path_if_exists(&pending).await?;
                    report.removed_pending += 1;
                }
                continue;
            }
            ManagedObjectState::Corrupt => {
                delete_bytes(root, rel).await?;
                report.removed_corrupt += 1;
            }
            ManagedObjectState::Missing => {}
        }
        let pending = root.join(pending_relative_path(rel)?);
        if tokio::fs::try_exists(&pending).await? {
            match publish_staged(root, rel).await {
                Ok(()) => {
                    report.recovered_pending += 1;
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                    tracing::warn!(path = %rel, "discarded corrupt staged media object");
                    report.removed_pending += 1;
                }
                Err(error) => return Err(error.into()),
            }
        }
        report.disabled_missing_assets +=
            fail_close_assets(&collection, rel, "storage_object_missing").await?;
    }

    for file in files {
        let (final_rel, pending) = match final_relative_from_pending(&file) {
            Some(final_rel) => (final_rel.to_string(), true),
            None => (file.clone(), false),
        };
        if refs.contains(&final_rel) {
            continue;
        }
        let _guards = lock_paths(root, [final_rel.clone()]).await?;
        let still_referenced = collection
            .count_documents(doc! { "kind": "media", "file_path": &final_rel }, None)
            .await?
            > 0;
        if still_referenced {
            continue;
        }
        if pending {
            delete_path_if_exists(&root.join(&file)).await?;
            report.removed_pending += 1;
        } else {
            delete_bytes(root, &file).await?;
            report.removed_orphans += 1;
        }
    }
    Ok(report)
}

pub async fn reconciler_loop(db: Database, root: PathBuf) {
    loop {
        match reconcile_once(&db, &root).await {
            Ok(report) if report != ReconcileReport::default() => {
                tracing::info!(?report, "media storage reconciliation completed");
            }
            Ok(_) => {}
            Err(error) => tracing::error!(?error, "media storage reconciliation failed"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS)).await;
    }
}

/// 纯决策：物理删文件前，给定"删本记录后同 file_path 的剩余引用数"，
/// 仅当剩余引用为 0（无兄弟记录共享该物理文件）才可物理删。
/// upload 不去重，同文件多次上传 = 多条记录共享一个 file_path，故必须查引用计数。
pub fn should_delete_physical_file(remaining_refs: u64) -> bool {
    remaining_refs == 0
}

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
    fn persisted_relative_path_rejects_escape() {
        assert!(is_safe_relative_path("ws/ab/file.pdf"));
        assert!(!is_safe_relative_path("../outside.pdf"));
        assert!(!is_safe_relative_path("/absolute/file.pdf"));
    }

    #[test]
    fn sha256_is_deterministic() {
        assert_eq!(sha256_hex(b"hello"), sha256_hex(b"hello"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
        assert_eq!(sha256_hex(b"hello").len(), 64);
    }

    #[test]
    fn sanitize_ext_whitelists_known_types() {
        assert_eq!(
            sanitize_ext("a.pdf", "application/pdf").as_deref(),
            Some("pdf")
        );
        assert_eq!(sanitize_ext("a.PNG", "image/png").as_deref(), Some("png"));
        // 危险/未知扩展名拒绝
        assert_eq!(sanitize_ext("evil.exe", "application/octet-stream"), None);
        assert_eq!(sanitize_ext("evil.sh", "text/x-sh"), None);
    }

    #[test]
    fn should_delete_only_when_no_refs() {
        assert!(should_delete_physical_file(0));
        assert!(!should_delete_physical_file(1));
        assert!(!should_delete_physical_file(5));
    }

    #[tokio::test]
    async fn delete_bytes_removes_file_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "mediadel_{}",
            sha256_hex(format!("{:?}", std::time::SystemTime::now()).as_bytes())
        ));
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

    #[tokio::test]
    async fn staged_bytes_publish_atomically_and_recover() {
        let dir = std::env::temp_dir().join(format!("media_stage_{}", uuid::Uuid::new_v4()));
        let rel = safe_relative_path("ws", &sha256_hex(b"payload"), "pdf").unwrap();
        let _guards = lock_paths(&dir, [rel.clone()]).await.unwrap();
        assert!(stage_bytes(&dir, &rel, b"payload").await.unwrap());
        assert!(!dir.join(&rel).exists());
        assert!(dir.join(pending_relative_path(&rel).unwrap()).exists());
        drop(_guards);
        assert!(recover_pending_file(&dir, &rel).await.unwrap());
        assert_eq!(read_bytes(&dir, &rel).await.unwrap(), b"payload");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn recovering_read_rejects_corrupt_published_object() {
        let dir = std::env::temp_dir().join(format!("media_corrupt_{}", uuid::Uuid::new_v4()));
        let rel = safe_relative_path("ws", &sha256_hex(b"expected"), "pdf").unwrap();
        store_bytes(&dir, &rel, b"corrupt").await.unwrap();

        let error = read_bytes_recovering(&dir, &rel)
            .await
            .expect_err("corrupt content-addressed object must not be returned");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(!dir.join(&rel).exists());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
