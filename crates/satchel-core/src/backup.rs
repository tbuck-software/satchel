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
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Executor, SqlitePool,
};

use crate::{
    db::{MASTER_KEY_FILES, RESTORE_PENDING_SUFFIX},
    utils::crypto,
};

pub mod publication;

const READY: &str = "satchel.restore-ready.json";
const APPLYING: &str = "satchel.restore-applying.json";
const DATABASE: &str = "satchel.db";
const APPLY_INCOMING_CUTOFF: &str = include_str!("backup/apply_incoming_cutoff.sql");
const PROFILE_FILES: &[&str] = &[
    DATABASE,
    "satchel.db-wal",
    "satchel.db-shm",
    "satchel.db-journal",
];
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
    #[serde(default)]
    replay_protection: ReplayProtection,
    #[serde(default)]
    activation_cutoff: Option<String>,
}

#[derive(Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReplayProtection {
    #[default]
    Legacy,
    Prepared,
    // The original hash was verified and the old sidecars were removed.
    // SQLite may have changed the hash or left a hot journal since then.
    RecoveringInstalled,
}

fn key_hash(key: &[u8; 32]) -> String {
    crate::utils::hex::lower_hex(&Sha256::digest(key))
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
    Ok(crate::utils::hex::lower_hex(&digest.finalize()))
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
            "encrypted_api_key" => crypto::SecretContext::translation_api_key(row_id),
            "credential" => crypto::SecretContext::push_link(row_id),
            _ => crypto::SecretContext::oauth_code_verifier(row_id),
        }
    }
    for (table, id_column, column) in [
        ("accounts", "id", "config"),
        ("pgp_keys", "id", "armored_key"),
        ("pgp_keys", "id", "passphrase"),
        ("smime_certificates", "id", "pem_private_key_encrypted"),
        ("oauth_flow_sessions", "state", "encrypted_code_verifier"),
        ("translation_engine_keys", "engine", "encrypted_api_key"),
        ("desktop_push_links", "id", "credential"),
    ] {
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?")
                .bind(table)
                .bind(column)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        if exists == 0 {
            if table == "accounts" {
                return Err("backend.errors.restoreDatabaseInvalid".into());
            }
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
        if backup_schema == 0 {
            return Err("backend.errors.restoreDatabaseInvalid".into());
        }
        if backup_schema > app_schema {
            return Err(schema_too_new(backup_schema, app_schema));
        }
        // Inspect the source schema before migrating it. In particular, a
        // claimed send predating submission_started_at has an unknown outcome.
        suppress_copied_work(&pool, None).await.map_err(|error| {
            tracing::warn!(%error, "restore work could not be held");
            "backend.errors.restoreDatabaseInvalid".to_string()
        })?;
        // Use the startup migrator, including its checksum and dirty-version
        // checks. This private copy needs neither a migration lock nor WAL.
        if let Err(error) = crate::db::run_migrations(&pool).await {
            tracing::warn!(%error, "restore database could not be migrated");
            return Err("backend.errors.restoreDatabaseInvalid".into());
        }
        for statement in [
            "SELECT id, provider, is_active FROM accounts LIMIT 0",
            "SELECT key, value, updated_at FROM app_settings LIMIT 0",
            "SELECT id, account_id, uidvalidity FROM folders LIMIT 0",
            "SELECT id, account_id, workflow_state FROM threads LIMIT 0",
            "SELECT id, account_id, folder_id, thread_id, original_source FROM messages LIMIT 0",
            "SELECT rowid FROM messages_fts LIMIT 0",
        ] {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                tracing::warn!(%error, "restore database schema is incomplete");
                return Err("backend.errors.restoreDatabaseInvalid".into());
            }
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
        replay_protection: ReplayProtection::Prepared,
        activation_cutoff: None,
    };
    temporary
        .persist(profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}")))
        .map_err(|e| e.to_string())?;
    sync_directory(profile)?;
    write_manifest(&profile.join(READY), &manifest)
}

/// Updates only the copied queue, schedules, and active intents. Also used
/// before opening a legacy installed restore, where the schema may be older.
async fn suppress_copied_work(
    pool: &SqlitePool,
    activation_cutoff: Option<&str>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let (submission_marker, draft_schedules, mail_merge, ingest, notifications, has_cutoff):
        (bool, bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT
            EXISTS (SELECT 1 FROM pragma_table_info('sync_queue') WHERE name = 'submission_started_at'),
            EXISTS (SELECT 1 FROM pragma_table_info('messages') WHERE name = 'scheduled_for'),
            EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'mail_merge_jobs'),
            EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'ingest_effects'),
            EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'notification_outbox'),
            EXISTS (SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'incoming_effects_cutoff')",
    )
    .fetch_one(&mut *tx)
    .await?;
    let hold_sends = include_str!("backup/hold_sends.sql");
    let legacy_hold_sends;
    let hold_sends = if submission_marker {
        hold_sends
    } else {
        legacy_hold_sends =
            hold_sends.replace("submission_started_at IS NOT NULL", "status = 'sending'");
        &legacy_hold_sends
    };
    for (present, statement) in [
        (
            true,
            include_str!("db/migrations/0198_restore_queue_failures.sql"),
        ),
        (true, include_str!("backup/preserve_queue_failures.sql")),
        (true, hold_sends),
        (true, include_str!("backup/hold_sent_copies.sql")),
        (
            draft_schedules,
            include_str!("backup/clear_draft_schedules.sql"),
        ),
        (mail_merge, include_str!("backup/pause_mail_merge.sql")),
        (ingest, include_str!("backup/skip_ingest_rules.sql")),
        (
            notifications,
            include_str!("backup/clear_notification_intents.sql"),
        ),
    ] {
        if present {
            tx.execute(sqlx::raw_sql(statement)).await?;
        }
    }
    if let Some(cutoff) = activation_cutoff {
        sqlx::query(include_str!("backup/record_incoming_cutoff.sql"))
            .bind(cutoff)
            .execute(&mut *tx)
            .await?;
        if has_cutoff {
            tx.execute(sqlx::raw_sql(APPLY_INCOMING_CUTOFF)).await?;
        }
    }
    tx.commit().await
}

/// Apply a legacy restore cutoff after migration. An imported Daily profile
/// can sit unused while Stable processes mail, so advance it on its first open.
pub(crate) async fn finish_imported_cutoff(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM app_settings WHERE key IN
          ('restore.incoming_effects_cutoff', 'daily_import.awaiting_first_open'))",
    )
    .fetch_one(pool)
    .await?;
    if pending {
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let first_open =
            sqlx::query("DELETE FROM app_settings WHERE key = 'daily_import.awaiting_first_open'")
                .execute(&mut *tx)
                .await?
                .rows_affected()
                > 0;
        if first_open {
            sqlx::query(include_str!("backup/record_incoming_cutoff.sql"))
                .bind(activation_time())
                .execute(&mut *tx)
                .await?;
        }
        tx.execute(sqlx::raw_sql(APPLY_INCOMING_CUTOFF)).await?;
        tx.commit().await?;
    }
    Ok(())
}

fn activation_time() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn copy_synced(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map_err(|e| e.to_string())?;
    File::open(to)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}

/// Finish an installed restore or recover the previous profile if the atomic
/// replacement did not happen. Rollback restores the previous WAL with its DB.
async fn recover_interrupted_apply(profile: &Path) -> Result<bool, String> {
    let path = profile.join(APPLYING);
    if !path.exists() {
        return Ok(false);
    }
    let mut manifest = read_manifest(&path)?;
    // The atomic rename consumes staging. Hash equality alone cannot tell a
    // completed swap from an identical base database with newer WAL pages.
    let installed = !profile
        .join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"))
        .try_exists()
        .map_err(|e| e.to_string())?;
    if !installed {
        if manifest.replay_protection == ReplayProtection::RecoveringInstalled {
            tracing::error!("unexpected staging file during installed restore recovery");
            return Err("backend.errors.restoreDatabaseInvalid".into());
        }
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
        let live = profile.join(DATABASE);
        if manifest.replay_protection != ReplayProtection::RecoveringInstalled {
            if file_hash(&live)? != manifest.database_hash {
                tracing::error!("installed restore does not match the recovery manifest");
                return Err("backend.errors.restoreDatabaseInvalid".into());
            }
            for name in &PROFILE_FILES[1..] {
                let sidecar = profile.join(name);
                if sidecar.exists() {
                    fs::remove_file(sidecar).map_err(|e| e.to_string())?;
                }
            }
        }
        if manifest.replay_protection != ReplayProtection::RecoveringInstalled
            || manifest.activation_cutoff.is_none()
        {
            // A retry must keep the activation time even if SQLite changed the hash.
            manifest.replay_protection = ReplayProtection::RecoveringInstalled;
            manifest
                .activation_cutoff
                .get_or_insert_with(activation_time);
            write_manifest(&path, &manifest)?;
        }
        let result = {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&live)
                        .journal_mode(SqliteJournalMode::Delete)
                        .synchronous(SqliteSynchronous::Extra),
                )
                .await
                .map_err(|e| e.to_string())?;
            let result = suppress_copied_work(&pool, manifest.activation_cutoff.as_deref()).await;
            pool.close().await;
            result
        };
        result.map_err(|e| e.to_string())?;
        if profile.join(READY).exists() {
            fs::remove_file(profile.join(READY)).map_err(|e| e.to_string())?;
        }
    }
    sync_directory(profile)?;
    fs::remove_file(path).map_err(|e| e.to_string())?;
    sync_directory(profile)?;
    Ok(installed)
}

/// Called before opening the live database; staged credentials use the active key.
pub async fn apply_pending_restore(
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
    if recover_interrupted_apply(profile).await? {
        return Ok(true);
    }
    let ready = profile.join(READY);
    if !ready.exists() {
        return Ok(false);
    }
    let mut manifest = read_manifest(&ready)?;
    let pending = profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"));
    if manifest.replay_protection != ReplayProtection::Prepared
        || file_hash(&pending)? != manifest.database_hash
    {
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
    manifest.activation_cutoff = Some(activation_time());
    write_manifest(&profile.join(APPLYING), &manifest)?;
    for name in &PROFILE_FILES[1..] {
        let live = profile.join(name);
        if live.exists() {
            fs::remove_file(live).map_err(|e| e.to_string())?;
        }
    }
    fs::rename(&pending, profile.join(DATABASE)).map_err(|e| e.to_string())?;
    sync_directory(profile)?;
    recover_interrupted_apply(profile).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn daily_first_open_applies_its_cutoff_once() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT, updated_at TEXT);
             INSERT INTO app_settings VALUES ('daily_import.awaiting_first_open', '1', '');
             CREATE TABLE accounts (id TEXT, incoming_effects_cutoff TEXT, last_sync_at TEXT, created_at TEXT);
             INSERT INTO accounts VALUES ('account', '2001-01-01', '2000-01-01', '1999-01-01');"
        ).execute(&pool).await.unwrap();
        let before = activation_time();
        finish_imported_cutoff(&pool).await.unwrap();
        let cutoff: String = sqlx::query_scalar("SELECT incoming_effects_cutoff FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(cutoff >= before && cutoff <= activation_time());
        sqlx::query("UPDATE accounts SET incoming_effects_cutoff = '2002-01-01'")
            .execute(&pool)
            .await
            .unwrap();
        finish_imported_cutoff(&pool).await.unwrap();
        let next: String = sqlx::query_scalar("SELECT incoming_effects_cutoff FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(next, "2002-01-01");
    }

    /// Backup manifests record this hash next to the key they carry.
    #[test]
    fn key_hash_is_pinned() {
        assert_eq!(
            key_hash(&[0; 32]),
            "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"
        );
    }

    /// Every fixture row is `row-1`.
    fn test_context(column: &str) -> crypto::SecretContext<'static> {
        match column {
            "config" => crypto::SecretContext::account_config("row-1"),
            "armored_key" => crypto::SecretContext::pgp_armored_key(),
            "passphrase" => crypto::SecretContext::pgp_passphrase(),
            "pem_private_key_encrypted" => crypto::SecretContext::smime_private_key(),
            "encrypted_api_key" => crypto::SecretContext::translation_api_key("row-1"),
            "credential" => crypto::SecretContext::push_link("row-1"),
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
        crate::db::run_migrations(&pool).await.unwrap();
        // Include every protected column, using deliberately different secrets.
        for statement in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('row-1', 'Backup', 'backup@example.test', 'imap', '', '', '', '')",
            "INSERT INTO pgp_keys (id, label, key_type, fingerprint, armored_key, created_at, updated_at)
             VALUES ('row-1', 'Backup', 'private', 'test', '', '', '')",
            "INSERT INTO smime_certificates (id, label, cert_type, fingerprint, pem_certificate, created_at, updated_at)
             VALUES ('row-1', 'Backup', 'private', 'test', '', '', '')",
            "INSERT INTO oauth_flow_sessions (state, account_id, provider, encrypted_code_verifier, created_at, expires_at)
             VALUES ('row-1', 'row-1', 'gmail', '', '', '')",
            "INSERT INTO translation_engine_keys (engine, encrypted_api_key, updated_at)
             VALUES ('row-1', '', '')",
            "INSERT INTO desktop_push_links (id, name, credential) VALUES ('row-1', 'Phone', '')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        for (table, column) in [
            ("accounts", "config"),
            ("pgp_keys", "armored_key"),
            ("pgp_keys", "passphrase"),
            ("smime_certificates", "pem_private_key_encrypted"),
            ("oauth_flow_sessions", "encrypted_code_verifier"),
            ("translation_engine_keys", "encrypted_api_key"),
            ("desktop_push_links", "credential"),
        ] {
            sqlx::query(&format!("UPDATE {table} SET {column} = ?"))
                .bind(crypto::encrypt_aead_with_key(column, key, test_context(column)).unwrap())
                .execute(&pool)
                .await
                .unwrap();
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
        assert!(!apply_pending_restore(&profile, Some(&key)).await.unwrap());
        assert!(restore_pending(&profile));
        assert_eq!(fs::read(&live).unwrap(), b"live database");

        drop(open);
        assert!(apply_pending_restore(&profile, Some(&key)).await.unwrap());
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
        assert!(apply_pending_restore(&profile, Some(&source_key))
            .await
            .is_err());
        assert!(!profile.join(DATABASE).exists());
        assert!(apply_pending_restore(&profile, Some(&active_key))
            .await
            .unwrap());
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
            ("translation_engine_keys", "encrypted_api_key"),
            ("desktop_push_links", "credential"),
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

    #[tokio::test]
    async fn restore_holds_send_work_until_explicitly_reopened_or_resumed() {
        use crate::{
            drafts::{
                outgoing::{OutgoingAttachmentInput, OutgoingMessageInput},
                send_queue::build_send_action,
            },
            mail_merge::job,
            sync::queue::{QueuedAction, SyncQueue, ACTION_RULE_FORWARD},
        };

        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        let key = [7; 32];
        backup_file(&source, &key).await;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&source))
            .await
            .unwrap();
        let queue = SyncQueue::new();
        let now = chrono::Utc::now();
        let scheduled = build_send_action(
            "row-1".into(),
            OutgoingMessageInput {
                to: vec!["recipient@example.test".into()],
                draft_id: Some("saved-draft".into()),
                subject: "Preserved draft".into(),
                body: "Unsent body".into(),
                attachments: vec![OutgoingAttachmentInput {
                    filename: "offer.pdf".into(),
                    mime_type: "application/pdf".into(),
                    content_base64: "YXR0YWNobWVudA==".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            Some(now + chrono::Duration::days(1)),
        )
        .unwrap();
        crate::drafts::enqueue_send_with_draft(&pool, &queue, &scheduled)
            .await
            .unwrap();

        let make_action = |id: &str, status: &str, action_type, last_error: Option<&str>| {
            let mut action = QueuedAction::new("row-1".into(), action_type, serde_json::json!({}));
            action.id = id.into();
            action.status = status.into();
            action.last_error = last_error.map(str::to_owned);
            action
        };
        let mut submitted = make_action("submitted", "sending", "send_message", None);
        submitted.submission_started_at = Some(now.to_rfc3339());
        for action in [
            make_action("pending", "pending", "send_message", None),
            submitted,
            make_action(
                "old-failure",
                "failed",
                "send_message",
                Some("network offline"),
            ),
            make_action("forward", "pending", ACTION_RULE_FORWARD, None),
            make_action("merge-copy", "pending", "send_message", None),
        ] {
            queue.enqueue(&pool, action).await.unwrap();
        }
        let mut sent_copy = make_action("sent-copy", "sending", "remote_mutation", None);
        sent_copy.payload = serde_json::json!({
            "kind": "append_sent",
            "raw_base64": "bWFpbA==",
            "folder_hint": "Sent"
        });
        sent_copy.last_error = Some("network offline".into());
        queue.enqueue(&pool, sent_copy.clone()).await.unwrap();
        let mut discard_copy = sent_copy.clone();
        discard_copy.id = "discard-copy".into();
        queue.enqueue(&pool, discard_copy).await.unwrap();

        for statement in [
            "INSERT INTO threads (id, account_id, last_message_date, created_at, updated_at)
             VALUES ('incoming-thread', 'row-1', '', '', '')",
            "INSERT INTO messages (id, account_id, folder_id, thread_id, from_address,
                  to_addresses, date, created_at, updated_at)
             SELECT 'incoming', account_id, folder_id, 'incoming-thread',
                    'sender@example.test', '[]', '', '', ''
             FROM messages WHERE id = 'saved-draft'",
            "INSERT INTO ingest_effects (message_id, thread_id, account_id, run_rules, created_at)
             VALUES ('incoming', 'incoming-thread', 'row-1', 1, '')",
            "INSERT OR REPLACE INTO notification_outbox (message_id, state, created_at)
             VALUES ('incoming', 'pending', '')",
            "INSERT INTO mail_merge_jobs (id, account_id, status, subject, template,
                  delay_seconds, created_at, updated_at)
             VALUES ('merge', 'row-1', 'running', 'Merge', '{}', 10, '', '')",
            "INSERT INTO mail_merge_recipients (id, job_id, position, email, fields, status,
                  queue_id, message_id_header, updated_at)
             VALUES ('recipient', 'merge', 1, 'recipient@example.test', '{}', 'queued',
                     'merge-copy', '<merge@example.test>', '')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool.close().await;

        stage_restore(&profile, &source, &[key], Some(&key))
            .await
            .unwrap();
        assert!(apply_pending_restore(&profile, Some(&key)).await.unwrap());
        let restored = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(profile.join(DATABASE)))
            .await
            .unwrap();
        assert_eq!(queue.reset_sending(&restored).await.unwrap(), 0);
        assert_eq!(queue.requeue_failed_transient(&restored).await.unwrap(), 0);
        assert!(queue.claim_next(&restored).await.unwrap().is_none());
        let pending_notifications: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notification_outbox WHERE state IN ('pending', 'dispatching')",
        )
        .fetch_one(&restored)
        .await
        .unwrap();
        assert_eq!(pending_notifications, 0);
        let run_rules: i64 = sqlx::query_scalar("SELECT run_rules FROM ingest_effects")
            .fetch_one(&restored)
            .await
            .unwrap();
        assert_eq!(run_rules, 0);
        assert!(queue.requeue_mail(&restored, "submitted").await.is_err());
        assert_eq!(
            queue
                .requeue_failed_remote_mutations(&restored)
                .await
                .unwrap(),
            0
        );
        let failed = queue.list_failed_sync_actions(&restored).await.unwrap();
        assert!(failed.iter().any(|action| action.id == "forward"));
        let held_copy = failed
            .iter()
            .find(|action| action.id == "sent-copy")
            .unwrap();
        assert_eq!(held_copy.payload, sent_copy.payload);
        assert_eq!(
            held_copy.pre_restore_error.as_deref(),
            Some("network offline")
        );
        let mail = queue.list_mail_outbox(&restored).await.unwrap();
        assert_eq!(
            mail.iter()
                .find(|item| item.id == "old-failure")
                .unwrap()
                .pre_restore_error
                .as_deref(),
            Some("network offline")
        );
        assert!(
            crate::mail_actions::discard_failed_sync_action(&restored, "discard-copy")
                .await
                .unwrap()
        );
        assert!(
            !crate::mail_actions::discard_failed_sync_action(&restored, "discard-copy")
                .await
                .unwrap()
        );
        assert!(!queue
            .list_failed_sync_actions(&restored)
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == "discard-copy"));
        job::tick(&restored, &queue, now).await.unwrap();
        let merge: (String, String) = sqlx::query_as(
            "SELECT j.status, r.status FROM mail_merge_jobs j
             JOIN mail_merge_recipients r ON r.job_id = j.id",
        )
        .fetch_one(&restored)
        .await
        .unwrap();
        assert_eq!(merge, ("paused".into(), "failed".into()));
        let schedule: Option<String> =
            sqlx::query_scalar("SELECT scheduled_for FROM messages WHERE is_draft = 1")
                .fetch_one(&restored)
                .await
                .unwrap();
        assert!(schedule.is_none());
        let draft = crate::drafts::restore_queued_draft(&restored, &queue, &scheduled.id)
            .await
            .unwrap();
        assert_eq!(draft.subject, "Preserved draft");
        assert_eq!(draft.body, "Unsent body");
        assert_eq!(draft.to, ["recipient@example.test"]);
        assert!(draft.scheduled_for.is_none());
        assert_eq!(draft.attachments[0].content_base64, "YXR0YWNobWVudA==");

        queue.requeue_mail(&restored, "pending").await.unwrap();
        assert_eq!(
            queue.claim_next(&restored).await.unwrap().unwrap().id,
            "pending"
        );
        assert!(queue
            .requeue_failed_sync_action(&restored, "forward")
            .await
            .unwrap());
        assert_eq!(
            queue.claim_next(&restored).await.unwrap().unwrap().id,
            "forward"
        );
        assert!(queue
            .requeue_failed_sync_action(&restored, "sent-copy")
            .await
            .unwrap());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM restored_queue_failures WHERE queue_id IN ('discard-copy', 'sent-copy')")
                .fetch_one(&restored).await.unwrap(),
            0
        );
        assert_eq!(
            queue.claim_next(&restored).await.unwrap().unwrap().id,
            "sent-copy"
        );
    }

    #[tokio::test]
    async fn a_restore_staged_before_send_holding_cannot_replace_the_live_database() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        let pending = profile.join(format!("{DATABASE}{RESTORE_PENDING_SUFFIX}"));
        fs::write(&pending, b"old staged database").unwrap();
        fs::write(profile.join(DATABASE), b"live database").unwrap();
        atomic_write(
            &profile.join(READY),
            &serde_json::to_vec(&serde_json::json!({
                "database_hash": file_hash(&pending).unwrap(),
                "key_hash": null,
                "previous": null
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            apply_pending_restore(profile, None).await.unwrap_err(),
            "backend.errors.restoreStagingChanged"
        );
        assert_eq!(fs::read(profile.join(DATABASE)).unwrap(), b"live database");
    }

    /// Record a schema version in a backup file the way sqlx would.
    async fn set_schema_version(path: &Path, version: i64) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(path))
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
    async fn restore_rejects_a_changed_migration_checksum_before_publication() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        backup_file(&source, &[7; 32]).await;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&source))
            .await
            .unwrap();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'00' WHERE version = 1")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let error = stage_restore(&profile, &source, &[[7; 32]], Some(&[9; 32]))
            .await
            .expect_err("a changed applied migration must not be staged");
        assert_eq!(error, "backend.errors.restoreDatabaseInvalid");
        assert!(!restore_pending(&profile));
    }

    #[tokio::test]
    async fn restore_accepts_a_backup_without_a_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("backup.db");
        backup_file(&source, &[7; 32]).await;
        assert!(read_backup_manifest(directory.path()).is_none());

        stage_restore(&profile, &source, &[[7; 32]], Some(&[9; 32]))
            .await
            .expect("a backup predating manifests still restores");
        assert!(restore_pending(&profile));
    }

    #[tokio::test]
    async fn restore_migrates_the_staging_copy_without_changing_the_backup() {
        use sqlx::migrate::Migrate;
        use sqlx::Connection;

        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        let source = directory.path().join("old-backup.db");
        let mut old = sqlx::SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&source)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        let migrations = sqlx::migrate!("./src/db/migrations");
        old.ensure_migrations_table().await.unwrap();
        old.apply(migrations.iter().next().unwrap()).await.unwrap();
        old.close().await.unwrap();
        let source_hash = file_hash(&source).unwrap();

        stage_restore(&profile, &source, &[], None).await.unwrap();
        assert!(apply_pending_restore(&profile, None).await.unwrap());
        let restored = SqlitePoolOptions::new()
            .connect_with(SqliteConnectOptions::new().filename(profile.join(DATABASE)))
            .await
            .unwrap();
        assert_eq!(
            schema_version(&restored).await.unwrap(),
            crate::db::latest_migration_version(),
        );
        assert_eq!(file_hash(&source).unwrap(), source_hash);
        restored.close().await;
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
        assert!(apply_pending_restore(&profile, Some(&[9; 32]))
            .await
            .is_err());
        assert_eq!(
            fs::read(profile.join(DATABASE)).unwrap(),
            b"existing database"
        );
        cancel_restore(&profile).unwrap();
        assert!(!restore_pending(&profile));
    }

    #[tokio::test]
    async fn interruption_before_swap_restores_wal_even_when_database_hash_matches() {
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
            replay_protection: ReplayProtection::Legacy,
            activation_cutoff: None,
        };
        write_manifest(&profile.join(APPLYING), &manifest).unwrap();
        recover_interrupted_apply(profile).await.unwrap();
        assert_eq!(
            fs::read(profile.join("satchel.db-wal")).unwrap(),
            b"committed newer pages"
        );
        assert!(!profile.join(APPLYING).exists());
    }

    #[tokio::test]
    async fn interruption_after_swap_finishes_without_reverting_new_database() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        older_backup_file(&profile.join(DATABASE)).await;
        fs::write(profile.join("satchel.db-wal"), b"stale pages").unwrap();
        let manifest = RestoreManifest {
            database_hash: file_hash(&profile.join(DATABASE)).unwrap(),
            key_hash: None,
            previous: Some("satchel.db.replaced-test".into()),
            replay_protection: ReplayProtection::Prepared,
            activation_cutoff: Some("2026-01-01T01:00:00Z".into()),
        };
        write_manifest(&profile.join(READY), &manifest).unwrap();
        write_manifest(&profile.join(APPLYING), &manifest).unwrap();
        recover_interrupted_apply(profile).await.unwrap();
        assert_eq!(
            assert_older_work_held(&profile.join(DATABASE)).await,
            "2026-01-01T01:00:00Z"
        );
        assert!(!profile.join("satchel.db-wal").exists());
        assert!(!restore_pending(profile));
    }

    /// Migration 57 has notification leases but predates submission markers,
    /// ingest effects and mail merge.
    async fn older_backup_file(path: &Path) {
        use sqlx::migrate::Migrate;
        use sqlx::Connection;

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let mut conn = sqlx::SqliteConnection::connect_with(&options)
            .await
            .unwrap();
        conn.ensure_migrations_table().await.unwrap();
        for migration in sqlx::migrate!("./src/db/migrations")
            .iter()
            .take_while(|m| m.version <= 57)
        {
            conn.apply(migration).await.unwrap();
        }
        for statement in [
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('a', 'Backup', 'backup@example.test', 'imap', '', '', '', '')",
            "INSERT INTO folders (id, account_id, name, full_path, folder_type, created_at, updated_at)
             VALUES ('f', 'a', 'Inbox', 'INBOX', 'inbox', '', '')",
            "INSERT INTO messages (id, account_id, folder_id, from_address, to_addresses, date, created_at, updated_at)
             VALUES ('new', 'a', 'f', 'sender@example.test', '[]', '', '', ''),
                    ('claimed', 'a', 'f', 'sender@example.test', '[]', '', '', ''),
                    ('shown', 'a', 'f', 'sender@example.test', '[]', '', '', '')",
            "INSERT OR REPLACE INTO notification_outbox (message_id, state, created_at, dispatch_token, dispatch_at)
             VALUES ('new', 'pending', '', NULL, NULL), ('claimed', 'dispatching', '', 'old-token', ''),
                    ('shown', 'delivered', '', NULL, NULL)",
            r#"INSERT INTO sync_queue (id, account_id, action_type, payload, status, last_error, created_at, updated_at)
               VALUES ('send', 'a', 'send_message', '{"input":{"subject":"Original draft"}}', 'sending', NULL, '', ''),
                      ('copy', 'a', 'remote_mutation', '{"kind":"append_sent","raw_base64":"bWFpbA=="}', 'failed', 'network offline', '', ''),
                      ('flags', 'a', 'remote_mutation', '{"kind":"flags","refs":[{"remote_id":"1","source_folder":"INBOX","kind":"append_sent"}],"read":true}', 'pending', NULL, '', ''),
                      ('bad', 'a', 'remote_mutation', 'malformed append_sent payload', 'failed', 'old error', '', '')"#,
        ] {
            sqlx::query(statement).execute(&mut conn).await.unwrap();
        }
        conn.close().await.unwrap();
    }

    async fn assert_older_work_held(path: &Path) -> String {
        let pool = SqlitePoolOptions::new()
            .connect_with(SqliteConnectOptions::new().filename(path))
            .await
            .unwrap();
        let rows: Vec<(String, String, Option<String>, String)> =
            sqlx::query_as("SELECT id, status, last_error, payload FROM sync_queue ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            [
                (
                    "bad".into(),
                    "failed".into(),
                    Some("old error".into()),
                    "malformed append_sent payload".into()
                ),
                (
                    "copy".into(),
                    "failed".into(),
                    Some("backend.errors.sentCopyHeldAfterRestore".into()),
                    r#"{"kind":"append_sent","raw_base64":"bWFpbA=="}"#.into()
                ),
                (
                    "flags".into(),
                    "pending".into(),
                    None,
                    r#"{"kind":"flags","refs":[{"remote_id":"1","source_folder":"INBOX","kind":"append_sent"}],"read":true}"#.into()
                ),
                (
                    "send".into(),
                    "delivery_unknown".into(),
                    Some("backend.errors.messageDeliveryStatusUnknown".into()),
                    r#"{"input":{"subject":"Original draft"}}"#.into()
                ),
            ]
        );
        let notifications: Vec<String> =
            sqlx::query_scalar("SELECT message_id FROM notification_outbox")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(notifications, ["shown"]);
        let message_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(message_count, 3);
        crate::db::run_migrations(&pool).await.unwrap();
        let cutoff: String = sqlx::query_scalar("SELECT incoming_effects_cutoff FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, account_id, folder_id, from_address,
                                   to_addresses, date, created_at, updated_at)
             VALUES ('fresh', 'a', 'f', 'sender@example.test', '[]', datetime(?, '+1 second'), '', ''),
                    ('catchup', 'a', 'f', 'sender@example.test', '[]', datetime(?, '-1 second'), '', '')",
        )
        .bind(&cutoff)
        .bind(&cutoff)
        .execute(&pool)
        .await
        .unwrap();
        let fresh: String =
            sqlx::query_scalar("SELECT state FROM notification_outbox WHERE message_id = 'fresh'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(fresh, "pending");
        let catchup: String = sqlx::query_scalar(
            "SELECT state FROM notification_outbox WHERE message_id = 'catchup'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(catchup, "dismissed");
        pool.close().await;
        cutoff
    }

    #[tokio::test]
    async fn restore_holds_work_from_before_submission_markers() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("old-backup.db");
        older_backup_file(&source).await;
        let hash = file_hash(&source).unwrap();

        let profile = directory.path().join("profile");
        stage_restore(&profile, &source, &[], None).await.unwrap();
        assert!(apply_pending_restore(&profile, None).await.unwrap());
        assert_older_work_held(&profile.join(DATABASE)).await;
        assert_eq!(file_hash(&source).unwrap(), hash);
    }

    #[tokio::test]
    async fn old_applying_manifest_preserves_an_installed_database_and_holds_its_work() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        let live = profile.join(DATABASE);
        older_backup_file(&live).await;
        let previous = profile.join("satchel.db.replaced-test");
        fs::create_dir(&previous).unwrap();
        fs::write(previous.join(DATABASE), b"previous profile").unwrap();
        let old_manifest = serde_json::json!({
            "database_hash": file_hash(&live).unwrap(),
            "key_hash": null,
            "previous": previous.file_name().unwrap().to_str().unwrap(),
        });
        let json = serde_json::to_vec(&old_manifest).unwrap();
        atomic_write(&profile.join(READY), &json).unwrap();
        atomic_write(&profile.join(APPLYING), &json).unwrap();

        assert!(apply_pending_restore(profile, None).await.unwrap());
        assert_older_work_held(&live).await;
        assert_eq!(
            fs::read(previous.join(DATABASE)).unwrap(),
            b"previous profile"
        );
        assert!(!restore_pending(profile));
    }

    #[tokio::test]
    async fn interrupted_legacy_cleanup_retries_after_the_live_hash_changes() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        let live = profile.join(DATABASE);
        older_backup_file(&live).await;
        let manifest = RestoreManifest {
            database_hash: file_hash(&live).unwrap(),
            key_hash: None,
            previous: Some("satchel.db.replaced-test".into()),
            replay_protection: ReplayProtection::RecoveringInstalled,
            activation_cutoff: Some("2026-01-01T01:00:00Z".into()),
        };
        write_manifest(&profile.join(APPLYING), &manifest).unwrap();
        // SQLite committed the holds; the process died before clearing APPLYING.
        let pool = SqlitePoolOptions::new()
            .connect_with(SqliteConnectOptions::new().filename(&live))
            .await
            .unwrap();
        suppress_copied_work(&pool, manifest.activation_cutoff.as_deref())
            .await
            .unwrap();
        pool.close().await;
        assert_ne!(file_hash(&live).unwrap(), manifest.database_hash);

        assert!(apply_pending_restore(profile, None).await.unwrap());
        assert_eq!(assert_older_work_held(&live).await, "2026-01-01T01:00:00Z");
        assert!(!restore_pending(profile));
    }

    #[tokio::test]
    async fn a_missing_staging_file_and_unverified_live_hash_leave_recovery_pending() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path();
        let live = profile.join(DATABASE);
        let previous = "satchel.db.replaced-test";
        fs::create_dir(profile.join(previous)).unwrap();
        fs::write(
            profile.join(previous).join(DATABASE),
            b"retained previous database",
        )
        .unwrap();
        fs::write(&live, b"unverified live database").unwrap();
        write_manifest(
            &profile.join(APPLYING),
            &RestoreManifest {
                database_hash: "expected hash".into(),
                key_hash: None,
                previous: Some(previous.into()),
                replay_protection: ReplayProtection::Legacy,
                activation_cutoff: None,
            },
        )
        .unwrap();
        assert!(apply_pending_restore(profile, None).await.is_err());
        assert_eq!(fs::read(live).unwrap(), b"unverified live database");
        assert!(recovery_in_progress(profile));
    }
}
