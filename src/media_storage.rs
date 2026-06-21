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

/// 仅允许 [A-Za-z0-9_-] 的 segment（workspace_id / sha 都应满足；含 . / 或其它即拒）。
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
}
