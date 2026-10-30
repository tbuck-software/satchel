//! Backup publication and cheap, conservative retention eligibility.
//!
//! Legacy manifests are descriptive, not proof that creation finished. Only
//! this protocol's completion record permits automatic deletion.

use super::{atomic_write, file_hash, sync_directory, BackupManifest, BACKUP_MANIFEST};
use crate::db::MASTER_KEY_FILES;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

pub const PUBLICATION_FAILED: &str = "backend.errors.backupPublicationFailed";

const COMPLETED: &str = "satchel-backup-complete.json";
const STAGING_PREFIX: &str = ".satchel-backup-";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    version: u32,
    name: String,
    manifest_hash: String,
    files: BTreeMap<String, u64>,
}

/// Owns only the randomly created staging directory. Dropping an unfinished
/// attempt cleans it up; process death can leave it behind, but never visible.
pub struct StagedBackup {
    directory: tempfile::TempDir,
    name: String,
}

impl StagedBackup {
    pub fn new(root: &Path, created_at: chrono::DateTime<chrono::Utc>) -> Result<Self, String> {
        let directory = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .tempdir_in(root)
            .map_err(publication_failed)?;
        Ok(Self {
            directory,
            name: format!(
                "satchel-{}-{}",
                created_at.format("%Y%m%d-%H%M%S-%9f"),
                uuid::Uuid::new_v4()
            ),
        })
    }

    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    /// Call only after snapshot inspection and successful portable key export.
    /// Sync every artifact before publishing the whole directory in one rename.
    pub fn publish(self, manifest: &BackupManifest) -> Result<std::path::PathBuf, String> {
        super::write_backup_manifest(self.path(), manifest).map_err(publication_failed)?;
        let files = inventory(self.path()).ok_or(PUBLICATION_FAILED)?;
        if !files.contains_key("satchel.db") || !files.contains_key(MASTER_KEY_FILES[0]) {
            return Err(PUBLICATION_FAILED.into());
        }
        for name in files.keys() {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.path().join(name))
                .and_then(|file| file.sync_all())
                .map_err(publication_failed)?;
        }
        let completion = Completion {
            version: 1,
            name: self.name.clone(),
            manifest_hash: file_hash(&self.path().join(BACKUP_MANIFEST))
                .map_err(publication_failed)?,
            files,
        };
        atomic_write(
            &self.path().join(COMPLETED),
            &serde_json::to_vec(&completion).map_err(publication_failed)?,
        )
        .map_err(publication_failed)?;
        let root = self
            .path()
            .parent()
            .ok_or(PUBLICATION_FAILED)?
            .to_path_buf();
        let destination = root.join(&self.name);
        rename_noreplace(self.path(), &destination).map_err(publication_failed)?;
        // The staging path no longer belongs to us after the rename.
        let _ = self.directory.keep();
        sync_directory(&root).map_err(publication_failed)?;
        Ok(destination)
    }
}

fn publication_failed(error: impl std::fmt::Display) -> String {
    tracing::warn!(%error, "backup publication failed");
    PUBLICATION_FAILED.to_string()
}

// std::fs::rename replaces an existing empty directory on Unix. The exclusive
// OS operation also protects foreign destinations if a name ever collides.
fn rename_noreplace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            from,
            rustix::fs::CWD,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )?;
        Ok(())
    }
    #[cfg(windows)]
    fs::rename(from, to)
}

/// Preserve legacy directory names, while excluding private staging paths and
/// symlinks. Completion eligibility applies the stricter publication naming rule.
pub fn is_backup_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    !name.starts_with(STAGING_PREFIX)
        && fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        && fs::symlink_metadata(path.join("satchel.db")).is_ok_and(|meta| meta.is_file())
        && regular_or_absent(&path.join(BACKUP_MANIFEST))
        && regular_or_absent(&path.join(COMPLETED))
        && regular_or_absent(&path.join(MASTER_KEY_FILES[0]))
        && regular_or_absent(&path.join(crate::utils::crypto::LOCAL_BACKUP_KEY_MARKER))
        && match fs::symlink_metadata(path.join("crypto")) {
            Ok(meta) => meta.is_dir() && regular_or_absent(&path.join("crypto/master.key")),
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        }
}

fn regular_or_absent(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) => meta.is_file(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

fn is_publication_name(name: &str) -> bool {
    let Some(timestamp) = name.strip_prefix("satchel-") else {
        return false;
    };
    if !timestamp.is_ascii() {
        return false;
    }
    timestamp.len() == 62
        && chrono::NaiveDateTime::parse_from_str(&timestamp[..25], "%Y%m%d-%H%M%S-%9f").is_ok()
        && timestamp.as_bytes()[25] == b'-'
        && uuid::Uuid::parse_str(&timestamp[26..]).is_ok()
}

// No recursive traversal: publications contain just these regular files. Extra
// files, subdirectories or symlinks mean the directory is left alone.
fn inventory(path: &Path) -> Option<BTreeMap<String, u64>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(path).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().into_string().ok()?;
        let metadata = fs::symlink_metadata(entry.path()).ok()?;
        if !metadata.is_file() {
            return None;
        }
        if name == COMPLETED {
            continue;
        }
        if !["satchel.db", BACKUP_MANIFEST, MASTER_KEY_FILES[0]].contains(&name.as_str()) {
            return None;
        }
        files.insert(name, metadata.len());
    }
    Some(files)
}

fn small_regular_file(path: &Path) -> Option<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return None;
    }
    fs::read(path).ok()
}

/// Metadata only: no database reads, integrity scans or keychain access. Old,
/// incomplete and altered backups are preserved without consuming a slot.
pub fn retention_eligible(path: &Path) -> bool {
    let check = || -> Option<()> {
        if !is_backup_directory(path) || !is_publication_name(path.file_name()?.to_str()?) {
            return None;
        }
        let completion: Completion =
            serde_json::from_slice(&small_regular_file(&path.join(COMPLETED))?).ok()?;
        if completion.version != 1 || path.file_name()?.to_str()? != completion.name {
            return None;
        }
        let manifest_bytes = small_regular_file(&path.join(BACKUP_MANIFEST))?;
        let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes).ok()?;
        if chrono::DateTime::parse_from_rfc3339(&manifest.created_at).is_err()
            || {
                use sha2::Digest;
                format!("{:x}", sha2::Sha256::digest(&manifest_bytes)) != completion.manifest_hash
            }
            || !completion.files.contains_key("satchel.db")
            || !completion.files.contains_key(MASTER_KEY_FILES[0])
            || inventory(path)? != completion.files
        {
            return None;
        }
        Some(())
    };
    check().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_publication_never_replaces_an_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let staging = StagedBackup::new(root.path(), chrono::Utc::now()).unwrap();
        let existing = root.path().join(&staging.name);
        // Even an empty foreign directory must survive a collision.
        fs::create_dir(&existing).unwrap();
        fs::write(staging.path().join("satchel.db"), b"snapshot").unwrap();
        fs::write(staging.path().join(MASTER_KEY_FILES[0]), b"synthetic key").unwrap();
        assert!(staging.publish(&BackupManifest::default()).is_err());
        assert_eq!(fs::read_dir(&existing).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_malformed_or_changed_completion_metadata_never_consumes_a_slot() {
        let root = tempfile::tempdir().unwrap();
        let time = chrono::Utc::now();
        let staging = StagedBackup::new(root.path(), time).unwrap();
        fs::write(staging.path().join("satchel.db"), b"synthetic snapshot").unwrap();
        fs::write(staging.path().join(MASTER_KEY_FILES[0]), b"synthetic key").unwrap();
        let path = staging
            .publish(&BackupManifest {
                created_at: time.to_rfc3339(),
                ..BackupManifest::default()
            })
            .unwrap();
        assert!(retention_eligible(&path));
        let completion = fs::read(path.join(COMPLETED)).unwrap();
        fs::remove_file(path.join(COMPLETED)).unwrap();
        assert!(!retention_eligible(&path));
        fs::write(path.join(COMPLETED), b"{}").unwrap();
        assert!(!retention_eligible(&path));
        fs::write(path.join(COMPLETED), &completion).unwrap();
        fs::write(path.join(BACKUP_MANIFEST), b"{}").unwrap();
        assert!(!retention_eligible(&path));
    }

    #[test]
    fn cancelled_staging_is_cleaned_without_touching_foreign_files() {
        let root = tempfile::tempdir().unwrap();
        let staging = StagedBackup::new(root.path(), chrono::Utc::now()).unwrap();
        fs::write(staging.path().join("satchel.db"), b"partial").unwrap();
        fs::write(root.path().join("keep"), b"foreign").unwrap();
        drop(staging);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        assert_eq!(fs::read(root.path().join("keep")).unwrap(), b"foreign");
    }
}
