//! Size ceilings for files a command pulls into memory whole.
//!
//! Every importer read the file first and decided afterwards. A 4 GB `.mbox` —
//! or `/dev/zero` — therefore took the process down before anything could
//! object, and `import_mbox` then ran `from_utf8_lossy` and `split_mbox` over
//! the same buffer again. The ceiling is checked on the metadata instead, so a
//! file that is too big is never read.

use std::path::Path;

/// Ceiling for a file a command reads in one piece.
///
/// Shared with the `.eml` viewer, which had the only such check in the tree.
pub const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;

/// Message key for a file above [`MAX_IMPORT_BYTES`].
pub const FILE_TOO_LARGE: &str = "backend.errors.importFileTooLarge";

/// Refuse a file above the ceiling before a single byte is read.
///
/// A path with no metadata — gone, or unreadable — passes; the read that
/// follows reports that better than a size check can.
pub async fn guard_import_size(path: &Path) -> Result<(), String> {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return Ok(());
    };
    if meta.len() <= MAX_IMPORT_BYTES {
        return Ok(());
    }
    Err(serde_json::json!({
        "key": FILE_TOO_LARGE,
        "params": {
            "megabytes": meta.len() / (1024 * 1024),
            "limit": MAX_IMPORT_BYTES / (1024 * 1024),
        },
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn file_of(bytes: u64) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().expect("temp file");
        // Sparse: the point is the reported length, not the disk.
        file.as_file().set_len(bytes).expect("size");
        file
    }

    #[tokio::test]
    async fn a_file_at_the_limit_is_allowed() {
        let file = file_of(MAX_IMPORT_BYTES).await;
        guard_import_size(file.path()).await.expect("at the limit");
    }

    #[tokio::test]
    async fn a_file_over_the_limit_is_refused_by_key() {
        let file = file_of(MAX_IMPORT_BYTES + 1).await;
        let error = guard_import_size(file.path())
            .await
            .expect_err("over the limit");
        let decoded: serde_json::Value = serde_json::from_str(&error).expect("encoded error");
        assert_eq!(decoded["key"], FILE_TOO_LARGE);
        assert_eq!(decoded["params"]["limit"], 64);
    }

    #[tokio::test]
    async fn a_missing_file_is_left_to_the_read() {
        guard_import_size(Path::new("/nonexistent/satchel/import.mbox"))
            .await
            .expect("no metadata, no verdict");
    }
}
