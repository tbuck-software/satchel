//! Restore preparation and crash recovery, independent of the app runtime.
//!
//! Only a fully validated database is published. Secrets are rewrapped under
//! the destination profile's key before publication, so restoring one profile
//! never replaces a shared operating-system key used by another profile.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

use crate::{
    db::{MASTER_KEY_FILES, RESTORE_PENDING_SUFFIX},
    utils::crypto,
};

const READY: &str = "satchel.restore-ready.json";
const APPLYING: &str = "satchel.restore-applying.json";
const DATABASE: &str = "satchel.db";
const PROFILE_FILES: &[&str] = &[DATABASE, "satchel.db-wal", "satchel.db-shm"];
/// Written beside `satchel.db` in a backup directory. Purely descriptive: the
/// authoritative schema version is the one inside the database, so a backup
/// from before this file existed still restores.
pub const BACKUP_MANIFEST: &str = "manifest.json";

/// What a backup says about itself, for listing it without opening it.
///
/// The counts are taken once, while the backup is being written. Reading them
/// back out of the database would mean opening every listed file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core//backup/"))]
pub struct BackupManifest {
    pub schema_version: i64,
    pub app_version: String,
    pub created_at: String,
    #[serde(default)]
    pub account_count: i64,
    #[serde(default)]
    pub message_count: i64,
    /// Whether the search index matched the message rows when the backup was
    /// written. `None` in a manifest from before this field — unknown is not
    /// the same as broken.
    #[serde(default)]
    pub search_index_ok: Option<bool>,
}

#[derive(Serialize, Deserialize)]
struct RestoreManifest {
    database_hash: String,
    key_hash: Option<String>,
    previous: Option<String>,
}

fn key_hash(key: &[u8; 32]) -> String {
    format!("{:x}", Sha256::digest(key))
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("missing restore directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temporary
        .write_all(bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    sync_directory(parent)
}

fn write_manifest(path: &Path, manifest: &RestoreManifest) -> Result<(), String> {
    atomic_write(
        path,
        &serde_json::to_vec(manifest).map_err(|e| e.to_string())?,
    )
}

fn read_manifest(path: &Path) -> Result<RestoreManifest, String> {
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn write_backup_manifest(directory: &Path, manifest: &BackupManifest) -> Result<(), String> {
    atomic_write(
        &directory.join(BACKUP_MANIFEST),
        &serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?,
    )
}

/// A backup written before manifests existed simply has none.
pub fn read_backup_manifest(directory: &Path) -> Option<BackupManifest> {
    let bytes = fs::read(directory.join(BACKUP_MANIFEST)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Highest migration version recorded in a database file.
///
/// A file without `_sqlx_migrations` reports 0 rather than failing: an empty or
/// foreign database is the integrity check's business, not this one's.
pub async fn schema_version(pool: &SqlitePool) -> Result<i64, String> {
    let table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if table == 0 {
        return Ok(0);
    }
    sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

fn schema_too_new(backup: i64, app: i64) -> String {
    serde_json::json!({
        "key": "backend.errors.restoreSchemaTooNew",
        "params": { "backupSchema": backup, "appSchema": app },
    })
    .to_string()
}

pub fn restore_pending(profile: &Path) -> bool {
    profile.join(READY).is_file() || profile.join(APPLYING).is_file()
}

/// An incomplete swap must be recovered before any live connections open.
pub fn recovery_in_progress(profile: &Path) -> bool {
    profile.join(APPLYING).exists()
}

pub fn cancel_restore(profile: &Path) -> Result<(), String> {
    if profile.join(APPLYING).exists() {
        return Err("backend.errors.restoreRecoveryInProgress".into());
    }
    for name in [
        READY.to_string(),
        format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"),
    ]
    .into_iter()
    .chain(
        MASTER_KEY_FILES
            .iter()
            .map(|p| format!("{p}{RESTORE_PENDING_SUFFIX}")),
    ) {
        match fs::remove_file(profile.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    sync_directory(profile)
}

/// Convert every protected column. Missing columns are allowed for older
/// backups; malformed ciphertext or missing keys abort the whole transaction.
async fn rewrap_secrets(
    pool: &SqlitePool,
    source_keys: &[[u8; 32]],
    target: Option<&[u8; 32]>,
) -> Result<bool, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let mut protected = false;
    fn context<'a>(column: &str, row_id: &'a str) -> crypto::SecretContext<'a> {
        match column {
            "config" => crypto::SecretContext::account_config(row_id),
            "armored_key" => crypto::SecretContext::pgp_armored_key(),
            "passphrase" => crypto::SecretContext::pgp_passphrase(),
            "pem_private_key_encrypted" => crypto::SecretContext::smime_private_key(),
            _ => crypto::SecretContext::oauth_code_verifier(row_id),
        }
    }
    for (table, id_column, column) in [
        ("accounts", "id", "config"),
        ("pgp_keys", "id", "armored_key"),
        ("pgp_keys", "id", "passphrase"),
        ("smime_certificates", "id", "pem_private_key_encrypted"),
        ("oauth_flow_sessions", "state", "encrypted_code_verifier"),
    ] {
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
                .bind(table)
                .bind(column)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        if exists == 0 {
            continue;
        }
        let rows = sqlx::query_as::<_, (i64, String, String)>(&format!(
            "SELECT rowid, CAST({id_column} AS TEXT), {column} FROM {table}
             WHERE {column} LIKE 'enc:v1:%' OR {column} LIKE 'enc:v2:%'"
        ))
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        for (rowid, row_id, value) in rows {
            protected = true;
            let target = target.ok_or("backend.errors.restoreKeyUnavailable")?;
            let plain = std::iter::once(target)
                .chain(source_keys.iter())
                .find_map(|key| {
                    crypto::decrypt_with_key(&value, key, context(column, &row_id)).ok()
                })
                .ok_or("backend.errors.restoreKeyMismatch")?;
            let encrypted =
                crypto::encrypt_aead_with_key(&plain, target, context(column, &row_id))?;
            sqlx::query(&format!("UPDATE {table} SET {column} = ? WHERE rowid = ?"))
                .bind(encrypted)
                .bind(rowid)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(protected)
}

pub async fn stage_restore(
    profile: &Path,
    source: &Path,
    source_keys: &[[u8; 32]],
    target_key: Option<&[u8; 32]>,
) -> Result<(), String> {
    fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    if profile.join(APPLYING).exists() {
        return Err("backend.errors.restoreRecoveryInProgress".into());
    }
    let temporary = tempfile::NamedTempFile::new_in(profile).map_err(|e| e.to_string())?;
    // VACUUM INTO also makes imports from an open SQLite file WAL-consistent.
    // It is allowed to renumber the rowids of `messages` (no explicit INTEGER
    // PRIMARY KEY), which is what `messages_fts` and
    // `message_search_projection` are keyed on — hence the index check below.
    let source_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(source).read_only(true))
        .await
        .map_err(|e| e.to_string())?;
    let snapshot = sqlx::query("VACUUM INTO ?")
        .bind(temporary.path().to_string_lossy().as_ref())
        .execute(&source_pool)
        .await
        .map_err(|e| e.to_string());
    source_pool.close().await;
    snapshot?;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(temporary.path())
                .journal_mode(SqliteJournalMode::Delete),
        )
        .await
        .map_err(|e| e.to_string())?;
    let result = async {
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await
            .map_err(|e| e.to_string())?;
        if integrity != "ok" {
            return Err("backend.errors.restoreDatabaseInvalid".into());
        }
        // Downgrades are the dangerous direction: `set_ignore_missing(true)`
        // accepts unknown `_sqlx_migrations` rows, so a newer schema would run
        // under older code and only fail at the first trigger or constraint it
        // does not know about.
        let backup_schema = schema_version(&pool).await?;
        let app_schema = crate::db::latest_migration_version();
        if backup_schema > app_schema {
            return Err(schema_too_new(backup_schema, app_schema));
        }
        // `PRAGMA integrity_check` says nothing about an external-content FTS5
        // table. A search index that no longer matches its messages would be
        // restored as-is and answer wrongly for as long as it took someone to
        // notice. The snapshot is writable here, so ask FTS5 itself.
        if !crate::db::message_index_intact(&pool).await? {
            return Err("backend.errors.restoreSearchIndexInvalid".into());
        }
        rewrap_secrets(&pool, source_keys, target_key).await
    }
    .await;
    pool.close().await;
    let protected = result?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    let manifest = RestoreManifest {
        database_hash: file_hash(temporary.path())?,
        key_hash: if protected {
            target_key.map(key_hash)
        } else {
            None
        },
        previous: None,
    };
    temporary
        .persist(profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}")))
        .map_err(|e| e.to_string())?;
    sync_directory(profile)?;
    write_manifest(&profile.join(READY), &manifest)
}

fn copy_synced(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map_err(|e| e.to_string())?;
    File::open(to)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}

/// Recover the previous profile if interruption happened before the atomic DB
/// replacement. Its WAL travels with it, so committed pages are never dropped.
fn recover_interrupted_apply(profile: &Path) -> Result<(), String> {
    let path = profile.join(APPLYING);
    if !path.exists() {
        return Ok(());
    }
    let manifest = read_manifest(&path)?;
    // The atomic rename consumes staging. Hash equality alone cannot tell a
    // completed swap from an identical base database with newer WAL pages.
    let installed = !profile
        .join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"))
        .exists()
        && file_hash(&profile.join(DATABASE)).ok().as_ref() == Some(&manifest.database_hash);
    if !installed {
        let previous = manifest
            .previous
            .as_deref()
            .ok_or("missing restore recovery path")?;
        if !previous.starts_with("satchel.db.replaced-") || previous.contains(['/', '\\']) {
            return Err("invalid restore recovery path".into());
        }
        for name in PROFILE_FILES {
            let source = profile.join(previous).join(name);
            let live = profile.join(name);
            if source.is_file() {
                let temporary =
                    tempfile::NamedTempFile::new_in(profile).map_err(|e| e.to_string())?;
                copy_synced(&source, temporary.path())?;
                temporary.persist(&live).map_err(|e| e.to_string())?;
            } else if live.exists() {
                fs::remove_file(live).map_err(|e| e.to_string())?;
            }
        }
    } else {
        for name in &PROFILE_FILES[1..] {
            let live = profile.join(name);
            if live.exists() {
                fs::remove_file(live).map_err(|e| e.to_string())?;
            }
        }
        if profile.join(READY).exists() {
            fs::remove_file(profile.join(READY)).map_err(|e| e.to_string())?;
        }
    }
    sync_directory(profile)?;
    fs::remove_file(path).map_err(|e| e.to_string())?;
    sync_directory(profile)
}

/// Called before opening the live database; staged credentials use the active key.
pub fn apply_pending_restore(
    profile: &Path,
    current_key: Option<&[u8; 32]>,
) -> Result<bool, String> {
    if !restore_pending(profile) {
        return Ok(false);
    }
    // Another process (Android's WorkManager) still writes to the live file;
    // replacing it would orphan those commits. The next start tries again.
    let Some(_exclusive) = crate::db::profile_lock::try_exclusive(profile)? else {
        tracing::warn!("staged restore deferred: the database is open in another process");
        return Ok(false);
    };
    recover_interrupted_apply(profile)?;
    let ready = profile.join(READY);
    if !ready.exists() {
        return Ok(false);
    }
    let mut manifest = read_manifest(&ready)?;
    let pending = profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"));
    if file_hash(&pending)? != manifest.database_hash {
        return Err("backend.errors.restoreStagingChanged".into());
    }
    if manifest.key_hash.is_some() && current_key.map(key_hash) != manifest.key_hash {
        return Err("backend.errors.restoreActiveKeyChanged".into());
    }
    let previous = format!("satchel.db.replaced-{}", uuid::Uuid::new_v4());
    let backup = profile.join(&previous);
    fs::create_dir(&backup).map_err(|e| e.to_string())?;
    for name in PROFILE_FILES {
        let live = profile.join(name);
        if live.exists() {
            copy_synced(&live, &backup.join(name))?;
        }
    }
    sync_directory(&backup)?;
    manifest.previous = Some(previous);
    write_manifest(&profile.join(APPLYING), &manifest)?;
    for name in &PROFILE_FILES[1..] {
        let live = profile.join(name);
        if live.exists() {
            fs::remove_file(live).map_err(|e| e.to_string())?;
        }
    }
    fs::rename(&pending, profile.join(DATABASE)).map_err(|e| e.to_string())?;
    sync_directory(profile)?;
    recover_interrupted_apply(profile)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixture row is `row-1`.
    fn test_context(column: &str) -> crypto::SecretContext<'static> {
        match column {
            "config" => crypto::SecretContext::account_config("row-1"),
            "armored_key" => crypto::SecretContext::pgp_armored_key(),
            "passphrase" => crypto::SecretContext::pgp_passphrase(),
            "pem_private_key_encrypted" => crypto::SecretContext::smime_private_key(),
            _ => crypto::SecretContext::oauth_code_verifier("row-1"),
        }
    }

    async fn backup_file(path: &Path, key: &[u8; 32]) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        // Include every protected column, using deliberately different secrets.
        for (table, id_column, columns) in [
            ("accounts", "id", vec!["config"]),
            ("pgp_keys", "id", vec!["armored_key", "passphrase"]),
            (
                "smime_certificates",
                "id",
                vec!["pem_private_key_encrypted"],
            ),
            (
                "oauth_flow_sessions",
                "state",
                vec!["encrypted_code_verifier"],
            ),
        ] {
            sqlx::query(&format!(
                "CREATE TABLE {table} ({id_column} TEXT, {})",
                columns
                    .iter()
                    .map(|c| format!("{c} TEXT"))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(&format!(
                "INSERT INTO {table} ({id_column}) VALUES ('row-1')"
            ))
            .execute(&pool)
            .await
            .unwrap();
            for column in columns {
                sqlx::query(&format!("UPDATE {table} SET {column} = ?"))
                    .bind(crypto::encrypt_aead_with_key(column, key, test_context(column)).unwrap())
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn apply_pending_restore_defers_while_the_database_is_open_elsewhere() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        let key = [7; 32];
        backup_file(&source, &key).await;
        stage_restore(&profile, &source, &[key], Some(&key))
            .await
            .unwrap();
        let live = profile.join(DATABASE);
        fs::write(&live, b"live database").unwrap();

        // What the background process holds while its pool is open.
        let open = crate::db::profile_lock::hold_open(&profile).await.unwrap();
        assert!(!apply_pending_restore(&profile, Some(&key)).unwrap());
        assert!(restore_pending(&profile));
        assert_eq!(fs::read(&live).unwrap(), b"live database");

        drop(open);
        assert!(apply_pending_restore(&profile, Some(&key)).unwrap());
        assert!(!restore_pending(&profile));
    }

    #[tokio::test]
    async fn restore_rewraps_all_secrets_for_existing_destination_key() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        let source_key = [7; 32];
        let active_key = [9; 32];
        backup_file(&source, &source_key).await;
        stage_restore(&profile, &source, &[source_key], Some(&active_key))
            .await
            .unwrap();
        // Keychain changes between staging and restart must never activate an
        // unreadable profile.
        assert!(apply_pending_restore(&profile, Some(&source_key)).is_err());
        assert!(!profile.join(DATABASE).exists());
        assert!(apply_pending_restore(&profile, Some(&active_key)).unwrap());
        assert!(!restore_pending(&profile));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(profile.join(DATABASE)))
            .await
            .unwrap();
        for (table, column) in [
            ("accounts", "config"),
            ("pgp_keys", "armored_key"),
            ("pgp_keys", "passphrase"),
            ("smime_certificates", "pem_private_key_encrypted"),
            ("oauth_flow_sessions", "encrypted_code_verifier"),
        ] {
            let value: String = sqlx::query_scalar(&format!("SELECT {column} FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(
                crypto::decrypt_with_key(&value, &active_key, test_context(column)).unwrap(),
                column
            );
            assert!(crypto::decrypt_with_key(&value, &source_key, test_context(column)).is_err());
        }
        pool.close().await;
    }

    /// Record a schema version in a backup file the way sqlx would.
    async fn set_schema_version(path: &Path, version: i64) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(path))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (
                version BIGINT PRIMARY KEY, description TEXT, installed_on TEXT,
                success BOOLEAN, checksum BLOB, execution_time BIGINT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations
             (version, description, installed_on, success, checksum, execution_time)
             VALUES (?, 'test', '2026-01-01', 1, X'00', 0)",
        )
        .bind(version)
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn restore_rejects_a_backup_from_a_newer_schema() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        backup_file(&source, &[7; 32]).await;
        let newer = crate::db::latest_migration_version() + 1;
        set_schema_version(&source, newer).await;
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join(DATABASE), b"live database").unwrap();

        let error = stage_restore(&profile, &source, &[[7; 32]], Some(&[9; 32]))
            .await
            .expect_err("a newer schema must not be staged");
        let decoded: serde_json::Value = serde_json::from_str(&error).expect("encoded error");
        assert_eq!(decoded["key"], "backend.errors.restoreSchemaTooNew");
        assert_eq!(decoded["params"]["backupSchema"], newer);

        assert!(!restore_pending(&profile));
        assert_eq!(fs::read(profile.join(DATABASE)).unwrap(), b"live database");
    }

    #[tokio::test]
    async fn restore_accepts_a_backup_without_a_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        backup_file(&source, &[7; 32]).await;
        set_schema_version(&source, crate::db::latest_migration_version()).await;
        assert!(read_backup_manifest(directory.path()).is_none());

        stage_restore(&profile, &source, &[[7; 32]], Some(&[9; 32]))
            .await
            .expect("a backup predating manifests still restores");
        assert!(restore_pending(&profile));
    }

    #[test]
    fn a_backup_manifest_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        write_backup_manifest(
            directory.path(),
            &BackupManifest {
                schema_version: 42,
                app_version: "0.1.0".into(),
                created_at: "2026-09-10T00:00:00Z".into(),
                account_count: 2,
                message_count: 1500,
                search_index_ok: Some(true),
            },
        )
        .unwrap();
        let manifest = read_backup_manifest(directory.path()).expect("manifest");
        assert_eq!(manifest.schema_version, 42);
        assert_eq!(manifest.app_version, "0.1.0");
        assert_eq!(manifest.message_count, 1500);
        assert_eq!(manifest.search_index_ok, Some(true));
    }

    /// A manifest written before the counts existed still reads.
    #[test]
    fn an_older_manifest_reads_without_the_counts() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join(BACKUP_MANIFEST),
            br#"{"schema_version":7,"app_version":"0.1.0","created_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        let manifest = read_backup_manifest(directory.path()).expect("manifest");
        assert_eq!(manifest.schema_version, 7);
        assert_eq!(manifest.message_count, 0);
        assert_eq!(manifest.search_index_ok, None);
    }

    #[tokio::test]
    async fn wrong_source_key_never_publishes_a_restore() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("backup.db");
        let profile = directory.path().join("profile");
        backup_file(&source, &[7; 32]).await;
        assert!(stage_restore(&profile, &source, &[[1; 32]], Some(&[9; 32]))
            .await
            .is_err());
        assert!(!restore_pending(&profile));
        assert!(!profile
            .join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"))
            .exists());
    }

    #[tokio::test]
    async fn changed_staging_never_replaces_existing_database() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("backup.db");
        let profile = directory.path().join("profile");
        backup_file(&source, &[7; 32]).await;
        stage_restore(&profile, &source, &[[7; 32]], Some(&[9; 32]))
            .await
            .unwrap();
        fs::write(profile.join(DATABASE), b"existing database").unwrap();
        fs::write(
            profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}")),
            b"partial",
        )
        .unwrap();
        assert!(apply_pending_restore(&profile, Some(&[9; 32])).is_err());
        assert_eq!(
            fs::read(profile.join(DATABASE)).unwrap(),
            b"existing database"
        );
        cancel_restore(&profile).unwrap();
        assert!(!restore_pending(&profile));
    }

    #[test]
    fn interruption_before_swap_restores_wal_even_when_database_hash_matches() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        let previous = "satchel.db.replaced-test";
        fs::create_dir(profile.join(previous)).unwrap();
        fs::write(profile.join(previous).join(DATABASE), b"same base").unwrap();
        fs::write(
            profile.join(previous).join("satchel.db-wal"),
            b"committed newer pages",
        )
        .unwrap();
        fs::write(profile.join(DATABASE), b"same base").unwrap();
        fs::write(
            profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}")),
            b"same base",
        )
        .unwrap();
        let manifest = RestoreManifest {
            database_hash: file_hash(&profile.join(DATABASE)).unwrap(),
            key_hash: None,
            previous: Some(previous.into()),
        };
        write_manifest(&profile.join(APPLYING), &manifest).unwrap();
        recover_interrupted_apply(profile).unwrap();
        assert_eq!(
            fs::read(profile.join("satchel.db-wal")).unwrap(),
            b"committed newer pages"
        );
        assert!(!profile.join(APPLYING).exists());
    }

    #[test]
    fn interruption_after_swap_finishes_without_reverting_new_database() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        fs::write(profile.join(DATABASE), b"installed").unwrap();
        fs::write(profile.join("satchel.db-wal"), b"stale pages").unwrap();
        let manifest = RestoreManifest {
            database_hash: file_hash(&profile.join(DATABASE)).unwrap(),
            key_hash: None,
            previous: Some("satchel.db.replaced-test".into()),
        };
        write_manifest(&profile.join(READY), &manifest).unwrap();
        write_manifest(&profile.join(APPLYING), &manifest).unwrap();
        recover_interrupted_apply(profile).unwrap();
        assert_eq!(fs::read(profile.join(DATABASE)).unwrap(), b"installed");
        assert!(!profile.join("satchel.db-wal").exists());
        assert!(!restore_pending(profile));
    }
}
