use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use tauri::AppHandle;

use crate::{
    commands::error::{CommandError, CommandResult},
    utils::crypto,
    utils::mime::{extract_email_address, mailbox_matches},
    utils::pgp_engine::{self, GenerateKeyRequest},
    AppState, UnlockedPgpSession,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingPgpSignature {
    Valid {
        cleartext: String,
        signer_name: Option<String>,
        signer_fingerprint: String,
    },
    /// The maths held, but no self-certified user ID of that key carries the
    /// `From:` mailbox — the message is signed by somebody else.
    IdentityMismatch {
        cleartext: String,
        signer_name: Option<String>,
        signer_fingerprint: String,
    },
    Invalid {
        cleartext: String,
    },
    UnknownKey {
        cleartext: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingPgpDetachedSignature {
    Valid {
        signer_name: Option<String>,
        signer_fingerprint: String,
    },
    /// See [`IncomingPgpSignature::IdentityMismatch`].
    IdentityMismatch {
        signer_name: Option<String>,
        signer_fingerprint: String,
    },
    Invalid,
    UnknownKey,
}

pub struct DecryptedIncomingPgp {
    pub prefix: String,
    pub suffix: String,
    pub plaintext: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct PgpKeyDto {
    pub id: String,
    pub label: String,
    pub key_type: String,
    pub fingerprint: String,
    pub user_ids: Vec<String>,
    pub has_passphrase: bool,
    pub is_unlocked: bool,
    pub is_expired: bool,
    pub can_encrypt: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/", optional_fields = nullable))]
pub struct ImportPgpKeysInput {
    pub label_prefix: Option<String>,
    pub armored_bundle: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct ImportPgpKeysResult {
    pub imported: Vec<PgpKeyDto>,
    pub skipped: usize,
    pub failed: usize,
    pub failures: Vec<ImportPgpKeyFailure>,
    pub detected: usize,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct ImportPgpKeyFailure {
    pub index: usize,
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/", optional_fields = nullable))]
pub struct GeneratePgpKeyInput {
    pub name: String,
    pub email: String,
    pub label: Option<String>,
    pub passphrase: Option<String>,
    pub algorithm: Option<String>,
    pub expires_in: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct GeneratePgpKeyResult {
    pub key: PgpKeyDto,
    pub public_key: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/", optional_fields = nullable))]
pub struct ValidatePgpBundlePassphraseInput {
    pub armored_bundle: String,
    pub passphrase: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct ValidatePgpBundlePassphraseResult {
    pub private_keys: usize,
    pub matching_keys: usize,
    pub failing_keys: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/", optional_fields = nullable))]
pub struct UnlockPgpKeysInput {
    pub key_id: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct UnlockPgpKeysResult {
    pub unlocked: usize,
    pub key_ids: Vec<String>,
}

#[derive(Debug, FromRow)]
struct PgpKeyRow {
    id: String,
    label: String,
    key_type: String,
    fingerprint: String,
    armored_key: String,
    has_passphrase: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, FromRow)]
struct PgpKeySecretRow {
    id: String,
    key_type: String,
    armored_key: String,
    passphrase: Option<String>,
}

#[derive(Debug, FromRow)]
struct PgpKeyMaterialRow {
    id: String,
    armored_key: String,
}

#[derive(Debug, FromRow)]
struct LegacyPgpKeyRow {
    id: String,
    armored_key: String,
    passphrase: Option<String>,
}

#[derive(Debug, FromRow)]
struct PgpFingerprintRow {
    id: String,
    key_type: String,
    fingerprint: String,
    armored_key: String,
}

#[derive(Debug, FromRow)]
struct PgpStoredKeyRow {
    id: String,
    label: String,
    key_type: String,
    armored_key: String,
    passphrase: Option<String>,
    created_at: String,
}

#[tauri::command]
pub async fn list_pgp_keys(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
) -> CommandResult<Vec<PgpKeyDto>> {
    prepare_pgp_key_storage(pool.inner(), &app).await?;
    let unlocked = state.pgp_unlock_sessions.lock().await;
    let unlocked_ids = unlocked.keys().cloned().collect::<HashSet<_>>();
    drop(unlocked);
    list_pgp_keys_inner(pool.inner(), &app, &unlocked_ids)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn import_pgp_keys(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: ImportPgpKeysInput,
) -> CommandResult<ImportPgpKeysResult> {
    prepare_pgp_key_storage(pool.inner(), &app).await?;
    let armored_blocks = parse_armored_key_blocks(&input.armored_bundle);
    if armored_blocks.is_empty() {
        return Err(CommandError::new("backend.errors.pgpNoKeyBlocks"));
    }

    let mut blocks = Vec::new();
    let mut failures = Vec::new();
    let mut detected = 0_usize;
    for armored_block in armored_blocks {
        match pgp_engine::split_armored_keys(&armored_block) {
            Ok(certificates) => {
                for certificate in certificates {
                    detected += 1;
                    blocks.push((detected, certificate));
                }
            }
            Err(_) => {
                detected += 1;
                failures.push(ImportPgpKeyFailure {
                    index: detected,
                    key: "backend.errors.pgpKeyBlockInvalid".to_string(),
                });
            }
        }
    }

    let passphrase = normalize_optional(input.passphrase);
    let label_prefix = normalize_optional(input.label_prefix);
    let mut imported = Vec::new();
    let mut skipped = 0_usize;

    for (index, block) in blocks {
        let label = label_prefix.as_ref().map(|prefix| {
            if detected == 1 {
                prefix.clone()
            } else {
                format!("{prefix} {index}")
            }
        });

        let result = insert_pgp_key(
            pool.inner(),
            &app,
            &block,
            label.as_deref(),
            passphrase.as_deref(),
        )
        .await;

        match result {
            Ok(Some(dto)) => imported.push(dto),
            Ok(None) => skipped += 1,
            Err(error) => failures.push(ImportPgpKeyFailure {
                index,
                key: import_failure_key(&error).to_string(),
            }),
        }
    }

    Ok(ImportPgpKeysResult {
        skipped,
        failed: failures.len(),
        failures,
        detected,
        imported,
    })
}

#[tauri::command]
pub async fn validate_pgp_bundle_passphrase(
    input: ValidatePgpBundlePassphraseInput,
) -> CommandResult<ValidatePgpBundlePassphraseResult> {
    let passphrase = input.passphrase.trim();
    if passphrase.is_empty() {
        return Err(CommandError::new("backend.errors.pgpPassphraseRequired"));
    }

    let private_blocks = parse_armored_key_blocks(&input.armored_bundle)
        .into_iter()
        .filter(|block| detect_key_type(block) == Some("private"))
        .flat_map(|block| pgp_engine::split_armored_keys(&block).unwrap_or_else(|_| vec![block]))
        .collect::<Vec<_>>();

    if private_blocks.is_empty() {
        return Ok(ValidatePgpBundlePassphraseResult {
            private_keys: 0,
            matching_keys: 0,
            failing_keys: 0,
        });
    }

    let mut matching = 0_usize;
    let mut failing = 0_usize;
    // Passphrase verification is a KDF: slow on purpose, and CPU-bound.
    let verdicts = {
        let blocks = private_blocks.clone();
        let candidate = passphrase.to_string();
        tokio::task::spawn_blocking(move || {
            blocks
                .iter()
                .map(|block| pgp_engine::verify_private_key_passphrase(block, &candidate))
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| format!("OpenPGP passphrase check failed: {error}"))?
    };
    for verdict in verdicts {
        match verdict {
            Ok(true) => matching += 1,
            Ok(false) => failing += 1,
            Err(err) => {
                failing += 1;
                if failing == 1 {
                    // Preserve one concrete error to help diagnostics.
                    return Err(CommandError::from(err));
                }
            }
        }
    }

    Ok(ValidatePgpBundlePassphraseResult {
        private_keys: private_blocks.len(),
        matching_keys: matching,
        failing_keys: failing,
    })
}

#[tauri::command]
pub async fn generate_pgp_key(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: GeneratePgpKeyInput,
) -> CommandResult<GeneratePgpKeyResult> {
    prepare_pgp_key_storage(pool.inner(), &app).await?;

    let name = input.name.trim();
    let email = input.email.trim();
    if name.is_empty() {
        return Err(CommandError::new("backend.errors.pgpNameRequired"));
    }
    if !email.contains('@') {
        return Err(CommandError::new("backend.errors.pgpEmailInvalid"));
    }
    pgp_engine::ensure_engine_available()?;

    let uid = format!("{name} <{email}>");
    let passphrase = normalize_optional(input.passphrase);
    let algorithm = input
        .algorithm
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let expires_in = input
        .expires_in
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "2y".to_string());

    // Key generation is seconds of CPU; it must not sit on an async worker.
    let generated = {
        let (uid, algorithm, expires_in, passphrase) = (
            uid.clone(),
            algorithm.clone(),
            expires_in.clone(),
            passphrase.clone(),
        );
        tokio::task::spawn_blocking(move || {
            pgp_engine::generate_keypair(&GenerateKeyRequest {
                uid: &uid,
                algorithm: &algorithm,
                expires_in: &expires_in,
                passphrase: passphrase.as_deref(),
            })
        })
        .await
        .map_err(|error| format!("OpenPGP key generation failed: {error}"))??
    };
    let fingerprint = generated.fingerprint;
    let private_armored = generated.private_armored;
    let public_armored = generated.public_armored;

    let created = insert_pgp_key(
        pool.inner(),
        &app,
        &private_armored,
        normalize_optional(input.label).as_deref(),
        passphrase.as_deref(),
    )
    .await?;

    let key = match created {
        Some(dto) => dto,
        None => fetch_key_by_fingerprint(pool.inner(), &app, &fingerprint, false)
            .await?
            .ok_or_else(|| "generated key already exists but could not be loaded".to_string())?,
    };

    Ok(GeneratePgpKeyResult {
        key,
        public_key: public_armored,
        fingerprint,
    })
}

#[tauri::command]
pub async fn unlock_pgp_keys(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: UnlockPgpKeysInput,
) -> CommandResult<UnlockPgpKeysResult> {
    prepare_pgp_key_storage(pool.inner(), &app).await?;
    pgp_engine::ensure_engine_available()?;

    let key_rows = load_private_key_rows(pool.inner(), input.key_id.as_deref()).await?;
    if key_rows.is_empty() {
        if let Some(key_id) = input.key_id.as_deref() {
            match load_key_type_by_id(pool.inner(), key_id).await? {
                Some(key_type) if key_type != "private" => {
                    return Err(CommandError::new("backend.errors.pgpPrivateKeyRequired"));
                }
                Some(_) => {}
                None => return Err(CommandError::new("backend.errors.pgpKeyNotFound")),
            }
        }
        return Err(CommandError::new("backend.errors.pgpNoPrivateKeys"));
    }

    let provided_passphrase = normalize_optional(input.passphrase);
    let mut unlocked_key_ids = Vec::new();

    for row in key_rows {
        if row.key_type != "private" {
            continue;
        }

        let armored_key = decrypt_secret(
            &row.armored_key,
            &app,
            crypto::SecretContext::pgp_armored_key(),
        )?;
        let stored_passphrase = if provided_passphrase.is_none() {
            row.passphrase
                .as_deref()
                .map(|value| decrypt_secret(value, &app, crypto::SecretContext::pgp_passphrase()))
                .transpose()?
        } else {
            None
        };
        let passphrase =
            unlock_passphrase(provided_passphrase.as_deref(), stored_passphrase.as_deref());

        let matches = {
            let (key, candidate) = (armored_key.clone(), passphrase.clone());
            tokio::task::spawn_blocking(move || {
                pgp_engine::verify_private_key_passphrase(&key, &candidate)
            })
            .await
            .map_err(|error| format!("OpenPGP passphrase check failed: {error}"))??
        };
        if !matches {
            continue;
        }

        let mut sessions = state.pgp_unlock_sessions.lock().await;
        sessions.insert(
            row.id.clone(),
            UnlockedPgpSession {
                key_id: row.id.clone(),
                passphrase,
                unlocked_at: chrono::Utc::now(),
            },
        );
        drop(sessions);
        unlocked_key_ids.push(row.id);
    }

    if unlocked_key_ids.is_empty() {
        if provided_passphrase.is_some() {
            return Err(CommandError::new("backend.errors.pgpPassphraseMismatch"));
        }
        return Err(CommandError::new(
            "backend.errors.pgpStoredPassphraseRejected",
        ));
    }

    Ok(UnlockPgpKeysResult {
        unlocked: unlocked_key_ids.len(),
        key_ids: unlocked_key_ids,
    })
}

#[tauri::command]
pub async fn delete_pgp_key(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    key_id: String,
) -> CommandResult<()> {
    let result = sqlx::query("DELETE FROM pgp_keys WHERE id = ?")
        .bind(&key_id)
        .execute(pool.inner())
        .await
        .map_err(|err| err.to_string())?;

    if result.rows_affected() == 0 {
        return Err(CommandError::new("backend.errors.pgpKeyNotFound"));
    }

    let mut sessions = state.pgp_unlock_sessions.lock().await;
    sessions.remove(&key_id);
    Ok(())
}

pub async fn decrypt_message_with_unlocked_keys(
    app: &AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    ciphertext: &str,
) -> Result<Option<DecryptedIncomingPgp>, String> {
    let normalized_body = normalize_newlines(ciphertext);
    let message_blocks = extract_pgp_message_blocks(&normalized_body);
    if message_blocks.is_empty() {
        return Ok(None);
    }

    let sessions = state.pgp_unlock_sessions.lock().await;
    if sessions.is_empty() {
        return Ok(None);
    }

    let unlocked = sessions
        .iter()
        .map(|(key_id, session)| (key_id.clone(), session.passphrase.clone()))
        .collect::<HashMap<_, _>>();
    drop(sessions);

    let key_ids = unlocked.keys().cloned().collect::<Vec<_>>();
    let rows = load_key_material_rows(pool, &key_ids).await?;

    let mut key_materials = Vec::new();
    for row in rows {
        let Some(passphrase) = unlocked.get(&row.id) else {
            continue;
        };

        let armored = decrypt_secret(
            &row.armored_key,
            app,
            crypto::SecretContext::pgp_armored_key(),
        )?;
        key_materials.push((armored, passphrase.clone()));
    }

    // Off the async worker: this is a nested loop of in-process OpenPGP
    // decryptions, the same class of CPU-bound work that `list_message_bodies`
    // already hands to `spawn_blocking` for WASM plugins. A thread with ten
    // encrypted messages otherwise held one Tokio worker for the whole run —
    // a quarter of the runtime on a four-core device, and more on mobile.
    tokio::task::spawn_blocking(move || {
        try_unlocked_keys(&normalized_body, &message_blocks, &key_materials)
    })
    .await
    .map_err(|error| format!("OpenPGP decryption task failed: {error}"))?
}

/// Try every unlocked key against every block, first success wins.
///
/// Owned and free of `AppHandle` so it can run on a blocking thread — and be
/// tested without one.
fn try_unlocked_keys(
    normalized_body: &str,
    message_blocks: &[PgpMessageBlock],
    key_materials: &[(String, String)],
) -> Result<Option<DecryptedIncomingPgp>, String> {
    let mut attempted = false;
    let mut last_error = None;
    for block in message_blocks {
        for (armored, passphrase) in key_materials.iter() {
            attempted = true;
            match pgp_engine::decrypt_with_private_key(armored, passphrase, &block.normalized) {
                Ok(decrypted) => {
                    let body = decrypted.trim();
                    if !body.is_empty() {
                        return Ok(Some(DecryptedIncomingPgp {
                            prefix: normalized_body[..block.start].to_string(),
                            suffix: normalized_body[block.end..].to_string(),
                            plaintext: decrypted,
                        }));
                    }
                    last_error = Some("decrypted OpenPGP message was empty".to_string());
                }
                Err(error) => last_error = Some(error),
            }
        }
    }

    if attempted {
        Err(last_error.unwrap_or_else(|| "no unlocked key could decrypt the message".to_string()))
    } else {
        Ok(None)
    }
}

/// A key speaks for an address only through a self-certified user ID carrying
/// it. `key_metadata` already drops user IDs without a valid self-signature,
/// so a user ID an attacker appended to someone else's certificate is gone
/// before it gets here.
fn key_speaks_for(user_ids: &[String], from_address: &str) -> bool {
    let Some(from) = extract_email_address(from_address) else {
        return false;
    };
    user_ids
        .iter()
        .filter_map(|user_id| extract_email_address(user_id))
        .any(|user_id| mailbox_matches(&user_id, &from))
}

/// Keys as the verification loops need them, already off the keychain, so the
/// decision below is testable without an `AppHandle`.
async fn load_verification_keys(app: &AppHandle, pool: &SqlitePool) -> Result<Vec<String>, String> {
    let rows = sqlx::query_as::<_, PgpKeySecretRow>(
        "SELECT id, key_type, armored_key, passphrase
         FROM pgp_keys WHERE key_type IN ('public', 'private') ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        rows.into_iter()
            .filter_map(|row| {
                decrypt_secret(
                    &row.armored_key,
                    &app,
                    crypto::SecretContext::pgp_armored_key(),
                )
                .ok()
            })
            .collect()
    })
    .await
    .map_err(|error| format!("PGP key loading task failed: {error}"))
}

pub async fn verify_incoming_cleartext_signature(
    app: &AppHandle,
    pool: &SqlitePool,
    message_id: &str,
    signed_message: &str,
    from_address: &str,
) -> Result<IncomingPgpSignature, String> {
    let keys = load_verification_keys(app, pool).await?;
    let signed_message = signed_message.to_string();
    let from_address = from_address.to_string();
    let message_id = message_id.to_string();
    static CACHE: std::sync::LazyLock<
        std::sync::Mutex<
            super::message_processing::ProcessingCache<Result<IncomingPgpSignature, String>>,
        >,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(Default::default()));
    tokio::task::spawn_blocking(move || {
        let mut parts: Vec<&[u8]> = keys.iter().map(|key| key.as_bytes()).collect();
        parts.extend([signed_message.as_bytes(), from_address.as_bytes()]);
        super::message_processing::cached(
            &CACHE,
            message_id,
            super::message_processing::fingerprint(&parts),
            || {
                let result = cleartext_signature_of(&keys, &signed_message, &from_address);
                let bytes = signed_message.len() + keys.iter().map(String::len).sum::<usize>();
                (result, bytes)
            },
        )
    })
    .await
    .map_err(|error| format!("PGP verification task failed: {error}"))?
}

fn cleartext_signature_of(
    keys: &[String],
    signed_message: &str,
    from_address: &str,
) -> Result<IncomingPgpSignature, String> {
    let parsed_cleartext = pgp_engine::parse_cleartext_signed_message(signed_message)?;

    let mut invalid_cleartext = None;
    let mut identity_mismatch = None;
    let mut verification_error = None;
    for armored in keys {
        let Ok(metadata) = pgp_engine::key_metadata(armored) else {
            continue;
        };
        match pgp_engine::verify_cleartext(armored, signed_message) {
            Ok(pgp_engine::CleartextVerification::Valid(verified)) => {
                let signer_name = metadata.user_ids.first().cloned();
                let signer_fingerprint = format_fingerprint(&verified.signer_fingerprint);
                if key_speaks_for(&metadata.user_ids, from_address) {
                    return Ok(IncomingPgpSignature::Valid {
                        cleartext: verified.cleartext,
                        signer_name,
                        signer_fingerprint,
                    });
                }
                // Another installed key may still be the sender's own, so keep
                // looking and fall back to the mismatch only at the end.
                if identity_mismatch.is_none() {
                    identity_mismatch = Some(IncomingPgpSignature::IdentityMismatch {
                        cleartext: verified.cleartext,
                        signer_name,
                        signer_fingerprint,
                    });
                }
            }
            Ok(pgp_engine::CleartextVerification::Invalid { cleartext }) => {
                invalid_cleartext = Some(cleartext);
            }
            Ok(pgp_engine::CleartextVerification::NotSignedByCertificate { cleartext }) => {
                debug_assert_eq!(cleartext, parsed_cleartext);
            }
            Err(error) => verification_error = Some(error),
        }
    }

    // Broken maths outranks a foreign signer: it is the harder failure and the
    // states are ordered by how much of the claim survives.
    if let Some(cleartext) = invalid_cleartext {
        Ok(IncomingPgpSignature::Invalid { cleartext })
    } else if let Some(mismatch) = identity_mismatch {
        Ok(mismatch)
    } else if let Some(error) = verification_error {
        Err(error)
    } else {
        Ok(IncomingPgpSignature::UnknownKey {
            cleartext: parsed_cleartext,
        })
    }
}

pub async fn verify_incoming_detached_signature(
    app: &AppHandle,
    pool: &SqlitePool,
    message_id: &str,
    payload: &[u8],
    armored_signature: &str,
    from_address: &str,
) -> Result<IncomingPgpDetachedSignature, String> {
    let keys = load_verification_keys(app, pool).await?;
    let payload = payload.to_vec();
    let armored_signature = armored_signature.to_string();
    let from_address = from_address.to_string();
    let message_id = message_id.to_string();
    static CACHE: std::sync::LazyLock<
        std::sync::Mutex<
            super::message_processing::ProcessingCache<
                Result<IncomingPgpDetachedSignature, String>,
            >,
        >,
    > = std::sync::LazyLock::new(|| std::sync::Mutex::new(Default::default()));
    tokio::task::spawn_blocking(move || {
        let mut parts: Vec<&[u8]> = keys.iter().map(|key| key.as_bytes()).collect();
        parts.extend([
            payload.as_slice(),
            armored_signature.as_bytes(),
            from_address.as_bytes(),
        ]);
        super::message_processing::cached(
            &CACHE,
            message_id,
            super::message_processing::fingerprint(&parts),
            || {
                let result =
                    detached_signature_of(&keys, &payload, &armored_signature, &from_address);
                let bytes = payload.len() + keys.iter().map(String::len).sum::<usize>();
                (result, bytes)
            },
        )
    })
    .await
    .map_err(|error| format!("PGP verification task failed: {error}"))?
}

fn detached_signature_of(
    keys: &[String],
    payload: &[u8],
    armored_signature: &str,
    from_address: &str,
) -> Result<IncomingPgpDetachedSignature, String> {
    let mut invalid = false;
    let mut identity_mismatch = None;
    let mut verification_error = None;
    for armored in keys {
        let Ok(metadata) = pgp_engine::key_metadata(armored) else {
            continue;
        };
        match pgp_engine::verify_detached(armored, payload, armored_signature) {
            Ok(pgp_engine::DetachedVerification::Valid { signer_fingerprint }) => {
                let signer_name = metadata.user_ids.first().cloned();
                let signer_fingerprint = format_fingerprint(&signer_fingerprint);
                if key_speaks_for(&metadata.user_ids, from_address) {
                    return Ok(IncomingPgpDetachedSignature::Valid {
                        signer_name,
                        signer_fingerprint,
                    });
                }
                if identity_mismatch.is_none() {
                    identity_mismatch = Some(IncomingPgpDetachedSignature::IdentityMismatch {
                        signer_name,
                        signer_fingerprint,
                    });
                }
            }
            Ok(pgp_engine::DetachedVerification::Invalid) => invalid = true,
            Ok(pgp_engine::DetachedVerification::NotSignedByCertificate) => {}
            Err(error) => verification_error = Some(error),
        }
    }

    // Broken maths outranks a foreign signer: it is the harder failure and the
    // states are ordered by how much of the claim survives.
    if invalid {
        Ok(IncomingPgpDetachedSignature::Invalid)
    } else if let Some(mismatch) = identity_mismatch {
        Ok(mismatch)
    } else if let Some(error) = verification_error {
        Err(error)
    } else {
        Ok(IncomingPgpDetachedSignature::UnknownKey)
    }
}

#[derive(Debug, Clone)]
struct PgpMessageBlock {
    start: usize,
    end: usize,
    normalized: String,
}

fn extract_pgp_message_blocks(input: &str) -> Vec<PgpMessageBlock> {
    if input.trim().is_empty() {
        return vec![];
    }

    const BEGIN: &str = "-----BEGIN PGP MESSAGE-----";
    const END: &str = "-----END PGP MESSAGE-----";

    let mut out = Vec::new();
    let mut dedupe = HashSet::new();
    let mut collecting = false;
    let mut block_start = 0_usize;
    let mut normalized_block = String::new();
    let mut offset = 0_usize;
    for segment in input.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let prepared = strip_email_quote_prefix(line);
        let mut first_line_piece: Option<String> = None;

        if !collecting {
            if let Some(begin_idx) = prepared.find(BEGIN) {
                collecting = true;
                block_start = offset;
                normalized_block.clear();
                first_line_piece = Some(prepared[begin_idx..].trim_end().to_string());
            }
        }

        if collecting {
            if let Some(piece) = first_line_piece {
                normalized_block.push_str(piece.as_str());
            } else {
                normalized_block.push_str(prepared.trim_end());
            }
            normalized_block.push('\n');

            if prepared.contains(END) {
                let block_end = offset + segment.len();
                let normalized = normalized_block.trim().to_string();
                if !normalized.is_empty() && dedupe.insert((block_start, block_end)) {
                    out.push(PgpMessageBlock {
                        start: block_start,
                        end: block_end,
                        normalized,
                    });
                }
                collecting = false;
                normalized_block.clear();
            }
        }
        offset += segment.len();
    }

    out
}

fn strip_email_quote_prefix(line: &str) -> &str {
    let mut current = line.trim_start();
    while let Some(rest) = current.strip_prefix('>') {
        current = rest.trim_start();
    }
    current
}

fn normalize_newlines(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

async fn list_pgp_keys_inner(
    pool: &SqlitePool,
    app: &AppHandle,
    unlocked_ids: &HashSet<String>,
) -> Result<Vec<PgpKeyDto>, String> {
    let rows = sqlx::query_as::<_, PgpKeyRow>(
        "SELECT id,
                label,
                key_type,
                fingerprint,
                armored_key,
                CASE
                    WHEN passphrase IS NULL OR TRIM(passphrase) = '' THEN 0
                    ELSE 1
                END AS has_passphrase,
                created_at,
                updated_at
         FROM pgp_keys
         ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let is_unlocked = unlocked_ids.contains(&row.id);
            map_row_to_dto(row, app, is_unlocked)
        })
        .collect())
}

fn map_row_to_dto(row: PgpKeyRow, app: &AppHandle, is_unlocked: bool) -> PgpKeyDto {
    let metadata = decrypt_secret(
        &row.armored_key,
        app,
        crypto::SecretContext::pgp_armored_key(),
    )
    .ok()
    .and_then(|armored| pgp_engine::key_metadata(&armored).ok());
    let fingerprint = metadata
        .as_ref()
        .map(|value| format_fingerprint(&value.fingerprint))
        .unwrap_or_else(|| format_fingerprint(&row.fingerprint));
    let user_ids = metadata
        .as_ref()
        .map(|value| value.user_ids.clone())
        .unwrap_or_default();
    let is_expired = metadata.as_ref().is_none_or(|value| value.is_expired);
    let can_encrypt = metadata.as_ref().is_some_and(|value| value.can_encrypt);

    PgpKeyDto {
        id: row.id,
        label: row.label,
        key_type: row.key_type,
        fingerprint,
        user_ids,
        has_passphrase: row.has_passphrase > 0,
        is_unlocked,
        is_expired,
        can_encrypt,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn fetch_key_by_fingerprint(
    pool: &SqlitePool,
    app: &AppHandle,
    fingerprint: &str,
    is_unlocked: bool,
) -> Result<Option<PgpKeyDto>, String> {
    let normalized = format_fingerprint(fingerprint);
    let row = sqlx::query_as::<_, PgpKeyRow>(
        "SELECT id,
                label,
                key_type,
                fingerprint,
                armored_key,
                CASE
                    WHEN passphrase IS NULL OR TRIM(passphrase) = '' THEN 0
                    ELSE 1
                END AS has_passphrase,
                created_at,
                updated_at
         FROM pgp_keys
         WHERE fingerprint = ?
         LIMIT 1",
    )
    .bind(&normalized)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?;

    Ok(row.map(|item| map_row_to_dto(item, app, is_unlocked)))
}

async fn load_private_key_rows(
    pool: &SqlitePool,
    key_id: Option<&str>,
) -> Result<Vec<PgpKeySecretRow>, String> {
    if let Some(key_id) = key_id {
        return sqlx::query_as::<_, PgpKeySecretRow>(
            "SELECT id, key_type, armored_key, passphrase
             FROM pgp_keys
             WHERE id = ? AND key_type = 'private'
             LIMIT 1",
        )
        .bind(key_id)
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string());
    }

    sqlx::query_as::<_, PgpKeySecretRow>(
        "SELECT id, key_type, armored_key, passphrase
         FROM pgp_keys
         WHERE key_type = 'private'
         ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())
}

async fn load_key_type_by_id(pool: &SqlitePool, key_id: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar::<_, String>(
        "SELECT key_type
         FROM pgp_keys
         WHERE id = ?
         LIMIT 1",
    )
    .bind(key_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())
}

async fn load_key_material_rows(
    pool: &SqlitePool,
    key_ids: &[String],
) -> Result<Vec<PgpKeyMaterialRow>, String> {
    if key_ids.is_empty() {
        return Ok(vec![]);
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, armored_key
         FROM pgp_keys
         WHERE key_type = 'private' AND id IN (",
    );
    let mut separated = query.separated(", ");
    for key_id in key_ids {
        separated.push_bind(key_id);
    }
    separated.push_unseparated(")");

    query
        .build_query_as::<PgpKeyMaterialRow>()
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())
}

async fn insert_pgp_key(
    pool: &SqlitePool,
    app: &AppHandle,
    armored_key: &str,
    label: Option<&str>,
    passphrase: Option<&str>,
) -> Result<Option<PgpKeyDto>, String> {
    let armored = armored_key.trim();
    if armored.is_empty() {
        return Err("armored key is required".to_string());
    }

    let key_type = detect_key_type(armored)
        .ok_or_else(|| "invalid key block: expected public or private PGP key".to_string())?;
    let metadata = pgp_engine::key_metadata(armored)?;
    if metadata.has_secret_key != (key_type == "private") {
        return Err("armored key type does not match its OpenPGP packets".to_string());
    }

    if key_type == "private" {
        if let Some(candidate) = passphrase.map(str::trim).filter(|value| !value.is_empty()) {
            let matches = {
                let (key, candidate) = (armored.to_string(), candidate.to_string());
                tokio::task::spawn_blocking(move || {
                    pgp_engine::verify_private_key_passphrase(&key, &candidate)
                })
                .await
                .map_err(|error| format!("OpenPGP passphrase check failed: {error}"))??
            };
            if !matches {
                return Err("provided passphrase does not match this private key".to_string());
            }
        }
    }

    let fingerprint = format_fingerprint(&metadata.fingerprint);
    let id = format!("pgp-key-{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    let encrypted_armored = encrypt_secret(armored, app, crypto::SecretContext::pgp_armored_key())?;

    let encrypted_passphrase = if key_type == "private" {
        passphrase
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| encrypt_secret(value, app, crypto::SecretContext::pgp_passphrase()))
            .transpose()?
    } else {
        None
    };

    let fallback_label = {
        let suffix = fingerprint
            .chars()
            .rev()
            .filter(|ch| ch.is_ascii_hexdigit())
            .take(8)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("{key_type} key {suffix}")
    };
    let default_label = metadata
        .user_ids
        .first()
        .map(String::as_str)
        .unwrap_or(fallback_label.as_str());
    let label = label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_label);

    let result = sqlx::query(
        "INSERT OR IGNORE INTO pgp_keys (id, label, key_type, fingerprint, armored_key, passphrase, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(label)
    .bind(key_type)
    .bind(&fingerprint)
    .bind(encrypted_armored)
    .bind(encrypted_passphrase.as_deref())
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| err.to_string())?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    Ok(Some(PgpKeyDto {
        id,
        label: label.to_string(),
        key_type: key_type.to_string(),
        fingerprint,
        user_ids: metadata.user_ids,
        has_passphrase: encrypted_passphrase.is_some(),
        is_unlocked: false,
        is_expired: metadata.is_expired,
        can_encrypt: metadata.can_encrypt,
        created_at: now.clone(),
        updated_at: now,
    }))
}

async fn prepare_pgp_key_storage(pool: &SqlitePool, app: &AppHandle) -> Result<(), String> {
    encrypt_legacy_secret_rows(pool, app).await?;
    split_legacy_keyring_rows(pool, app).await?;
    repair_legacy_fingerprints(pool, app).await
}

async fn split_legacy_keyring_rows(pool: &SqlitePool, app: &AppHandle) -> Result<(), String> {
    let rows = sqlx::query_as::<_, PgpStoredKeyRow>(
        "SELECT id, label, key_type, armored_key, passphrase, created_at FROM pgp_keys",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    for row in rows {
        let Ok(armored) = decrypt_secret(
            &row.armored_key,
            app,
            crypto::SecretContext::pgp_armored_key(),
        ) else {
            continue;
        };
        let Ok(certificates) = pgp_engine::split_armored_keys(&armored) else {
            continue;
        };
        if certificates.len() <= 1 {
            continue;
        }

        let mut replacements = Vec::with_capacity(certificates.len());
        for (index, certificate) in certificates.into_iter().enumerate() {
            let metadata = pgp_engine::key_metadata(&certificate)?;
            if metadata.has_secret_key != (row.key_type == "private") {
                return Err("split OpenPGP certificate changed its key type".to_string());
            }
            replacements.push((
                format!("pgp-key-{}", uuid::Uuid::new_v4()),
                if index == 0 {
                    row.label.clone()
                } else {
                    format!("{} {}", row.label, index + 1)
                },
                format_fingerprint(&metadata.fingerprint),
                encrypt_secret(&certificate, app, crypto::SecretContext::pgp_armored_key())?,
            ));
        }

        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
        let (_, first_label, first_fingerprint, first_armored) = &replacements[0];
        // A previous re-import may already have created this exact row. It is
        // redundant material; keep the original row id because compose
        // preferences and active unlock sessions can refer to it.
        sqlx::query("DELETE FROM pgp_keys WHERE fingerprint = ? AND key_type = ? AND id <> ?")
            .bind(first_fingerprint)
            .bind(&row.key_type)
            .bind(&row.id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        sqlx::query(
            "UPDATE pgp_keys
             SET label = ?, fingerprint = ?, armored_key = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(first_label)
        .bind(first_fingerprint)
        .bind(first_armored)
        .bind(&now)
        .bind(&row.id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;

        for (id, label, fingerprint, encrypted_armored) in replacements.into_iter().skip(1) {
            sqlx::query(
                "INSERT OR IGNORE INTO pgp_keys
                 (id, label, key_type, fingerprint, armored_key, passphrase, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(label)
            .bind(&row.key_type)
            .bind(fingerprint)
            .bind(encrypted_armored)
            .bind(row.passphrase.as_deref())
            .bind(&row.created_at)
            .bind(&now)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn repair_legacy_fingerprints(pool: &SqlitePool, app: &AppHandle) -> Result<(), String> {
    let rows = sqlx::query_as::<_, PgpFingerprintRow>(
        "SELECT id, key_type, fingerprint, armored_key FROM pgp_keys",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let repairs = rows
        .into_iter()
        .filter_map(|row| {
            let armored = decrypt_secret(
                &row.armored_key,
                app,
                crypto::SecretContext::pgp_armored_key(),
            )
            .ok()?;
            let metadata = pgp_engine::key_metadata(&armored).ok()?;
            if metadata.has_secret_key != (row.key_type == "private") {
                return None;
            }

            let actual = format_fingerprint(&metadata.fingerprint);
            (actual != format_fingerprint(&row.fingerprint)).then_some((row.id, actual))
        })
        .collect::<Vec<_>>();

    apply_fingerprint_repairs(pool, &repairs).await
}

async fn apply_fingerprint_repairs(
    pool: &SqlitePool,
    repairs: &[(String, String)],
) -> Result<(), String> {
    for (id, fingerprint) in repairs {
        // Old releases derived fingerprints from the armored text. If that
        // allowed a duplicate import, preserve both deletable rows and let the
        // unique index choose one canonical row for the real fingerprint.
        sqlx::query("UPDATE OR IGNORE pgp_keys SET fingerprint = ? WHERE id = ?")
            .bind(fingerprint)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

async fn encrypt_legacy_secret_rows(pool: &SqlitePool, app: &AppHandle) -> Result<(), String> {
    let rows = sqlx::query_as::<_, LegacyPgpKeyRow>(
        "SELECT id, armored_key, passphrase
         FROM pgp_keys
         WHERE (armored_key NOT LIKE 'enc:v1:%' AND armored_key NOT LIKE 'enc:v2:%')
            OR (passphrase IS NOT NULL
                AND passphrase NOT LIKE 'enc:v1:%' AND passphrase NOT LIKE 'enc:v2:%')",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    if rows.is_empty() {
        return Ok(());
    }

    let key = load_or_create_master_key(app)?;
    for row in rows {
        let armored_key = if is_encrypted_secret(&row.armored_key) {
            row.armored_key
        } else {
            encrypt_secret_with_key(
                &row.armored_key,
                &key,
                crypto::SecretContext::pgp_armored_key(),
            )?
        };

        let passphrase = row
            .passphrase
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| {
                if is_encrypted_secret(&value) {
                    Ok(value)
                } else {
                    encrypt_secret_with_key(&value, &key, crypto::SecretContext::pgp_passphrase())
                }
            })
            .transpose()?;

        sqlx::query(
            "UPDATE pgp_keys SET armored_key = ?, passphrase = ?, updated_at = ? WHERE id = ?",
        )
        .bind(armored_key)
        .bind(passphrase)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(row.id)
        .execute(pool)
        .await
        .map_err(|err| err.to_string())?;
    }

    Ok(())
}

fn parse_armored_key_blocks(input: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = String::new();
    let mut end_marker: Option<&str> = None;

    for line in input.lines() {
        if end_marker.is_none() {
            if line.contains("-----BEGIN PGP PRIVATE KEY BLOCK-----") {
                current.clear();
                current.push_str(line);
                current.push('\n');
                end_marker = Some("-----END PGP PRIVATE KEY BLOCK-----");
            } else if line.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----") {
                current.clear();
                current.push_str(line);
                current.push('\n');
                end_marker = Some("-----END PGP PUBLIC KEY BLOCK-----");
            }
            continue;
        }

        current.push_str(line);
        current.push('\n');
        if line.contains(end_marker.unwrap_or_default()) {
            blocks.push(current.trim().to_string());
            current.clear();
            end_marker = None;
        }
    }

    blocks
}

fn detect_key_type(armored: &str) -> Option<&'static str> {
    let upper = armored.to_ascii_uppercase();
    if upper.contains("-----BEGIN PGP PRIVATE KEY BLOCK-----") {
        return Some("private");
    }
    if upper.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----") {
        return Some("public");
    }
    None
}

fn format_fingerprint(raw: &str) -> String {
    let compact = raw
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase();

    compact
        .chars()
        .enumerate()
        .fold(String::new(), |mut acc, (idx, ch)| {
            if idx > 0 && idx % 4 == 0 {
                acc.push(' ');
            }
            acc.push(ch);
            acc
        })
}

fn normalize_optional(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn import_failure_key(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("passphrase") || normalized.contains("unlock") {
        "backend.errors.pgpPassphraseMismatch"
    } else if normalized.contains("key")
        || normalized.contains("certificate")
        || normalized.contains("openpgp")
        || normalized.contains("armor")
    {
        "backend.errors.pgpKeyBlockInvalid"
    } else {
        "backend.errors.unexpected"
    }
}

fn unlock_passphrase(provided: Option<&str>, stored: Option<&str>) -> String {
    provided.or(stored).unwrap_or_default().to_string()
}

fn is_encrypted_secret(raw: &str) -> bool {
    crypto::is_encrypted(raw)
}

/// PGP secret protection rides on the one master key in `utils::crypto`.
///
/// This module used to carry its own copy — same keychain entry, but a second
/// key file of its own. Two implementations of the same secret is
/// one too many, and mobile has no file fallback at all.
fn decrypt_secret(
    value: &str,
    app: &AppHandle,
    context: crypto::SecretContext<'_>,
) -> Result<String, String> {
    crypto::decrypt_aead(value, &crate::utils::profile_dir(app)?, context)
}

fn encrypt_secret(
    value: &str,
    app: &AppHandle,
    context: crypto::SecretContext<'_>,
) -> Result<String, String> {
    crypto::encrypt_aead(value, &crate::utils::profile_dir(app)?, context)
}

fn encrypt_secret_with_key(
    value: &str,
    key: &[u8; 32],
    context: crypto::SecretContext<'_>,
) -> Result<String, String> {
    crypto::encrypt_aead_with_key(value, key, context)
}

fn load_or_create_master_key(app: &AppHandle) -> Result<[u8; 32], String> {
    crypto::load_or_create_master_key(&crate::utils::profile_dir(app)?)
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/pgp/"))]
pub struct PgpEngineStatus {
    /// True when the selected engine is callable.
    pub available: bool,
    /// Engine identifier, either "rpgp" or "gpg".
    pub engine: String,
    pub can_decrypt: bool,
    pub can_sign: bool,
    pub can_encrypt: bool,
}

/// Report whether PGP operations can work at all.
///
/// The in-process engine is always available. The optional CLI engine can
/// still report a missing configured binary.
#[tauri::command]
pub async fn pgp_engine_status() -> CommandResult<PgpEngineStatus> {
    let available = crate::utils::pgp_engine::ensure_engine_available().is_ok();
    Ok(PgpEngineStatus {
        available,
        engine: crate::utils::pgp_engine::active_engine_id().to_string(),
        can_decrypt: available,
        can_sign: available,
        can_encrypt: available,
    })
}

/// Load an unlocked private key for signing outgoing mail.
///
/// The key must have been unlocked this session: Satchel never stores a
/// usable passphrase, so signing without an unlock is impossible by design.
pub async fn load_private_signing_key(
    app: &AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    key_id: &str,
) -> Result<(String, String), String> {
    let sessions = state.pgp_unlock_sessions.lock().await;
    let passphrase = sessions
        .get(key_id)
        .map(|session| session.passphrase.clone())
        .ok_or_else(|| "the PGP key is locked; unlock it in the OpenPGP settings".to_string())?;
    drop(sessions);

    let row = sqlx::query_as::<_, PgpKeySecretRow>(
        "SELECT id, key_type, armored_key, passphrase
         FROM pgp_keys WHERE id = ? AND key_type = 'private' LIMIT 1",
    )
    .bind(key_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| err.to_string())?
    .ok_or_else(|| "private PGP key not found".to_string())?;

    let armored = decrypt_secret(
        &row.armored_key,
        app,
        crypto::SecretContext::pgp_armored_key(),
    )?;
    Ok((armored, passphrase))
}

/// Resolve public keys for every recipient.
///
/// Returns an error naming the recipients without a key rather than silently
/// sending some copies unencrypted.
pub async fn resolve_recipient_public_keys(
    pool: &SqlitePool,
    app: &AppHandle,
    emails: &[String],
) -> Result<Vec<String>, String> {
    prepare_pgp_key_storage(pool, app).await?;
    let rows = sqlx::query_as::<_, PgpKeySecretRow>(
        "SELECT id, key_type, armored_key, passphrase
         FROM pgp_keys WHERE key_type IN ('public', 'private') ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| err.to_string())?;

    let mut keys = Vec::new();
    let mut missing = Vec::new();
    let mut added_fingerprints = HashSet::new();

    let mut key_material = Vec::with_capacity(rows.len());
    for row in rows {
        let Ok(armored) = decrypt_secret(
            &row.armored_key,
            app,
            crypto::SecretContext::pgp_armored_key(),
        ) else {
            continue;
        };
        let Ok(metadata) = pgp_engine::key_metadata(&armored) else {
            continue;
        };
        key_material.push((armored, metadata));
    }

    for email in emails {
        let needle = email.trim().to_ascii_lowercase();
        if needle.is_empty() {
            continue;
        }
        let matched = key_material.iter().find(|(_, metadata)| {
            metadata.can_encrypt
                && !metadata.is_expired
                && metadata
                    .user_ids
                    .iter()
                    .any(|user_id| user_id_matches_email(user_id, &needle))
        });

        match matched {
            Some((armored, metadata)) => {
                if added_fingerprints.insert(format_fingerprint(&metadata.fingerprint)) {
                    keys.push(armored.clone());
                }
            }
            None => missing.push(email.clone()),
        }
    }

    if !missing.is_empty() {
        return Err(format!("no public PGP key for: {}", missing.join(", ")));
    }

    Ok(keys)
}

fn user_id_matches_email(user_id: &str, email: &str) -> bool {
    let normalized = user_id.trim().to_ascii_lowercase();
    normalized == email || normalized.contains(&format!("<{email}>"))
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{
        apply_fingerprint_repairs, cleartext_signature_of, detached_signature_of,
        import_failure_key, key_speaks_for, pgp_engine, unlock_passphrase, user_id_matches_email,
        GenerateKeyRequest, IncomingPgpDetachedSignature, IncomingPgpSignature,
    };

    const TEST_PASSPHRASE: &str = "satchel-test";

    fn generate_key(uid: &str) -> (String, String) {
        let generated = pgp_engine::generate_keypair(&GenerateKeyRequest {
            uid,
            algorithm: "default",
            expires_in: "1y",
            passphrase: Some(TEST_PASSPHRASE),
        })
        .expect("generate test key");
        (generated.private_armored, generated.public_armored)
    }

    #[test]
    fn decrypted_content_keeps_its_exact_cover_and_suffix_without_trusting_markers() {
        let (private, public) = generate_key("Synthetic <sender@example.test>");
        let plaintext = "secret [[SATCHEL_SMIME_DECRYPTED_BLOCK_START]]literal[[SATCHEL_SMIME_DECRYPTED_BLOCK_END]]";
        let encrypted = pgp_engine::encrypt_to(&[public], plaintext.as_bytes()).unwrap();
        let cover =
            "[[SATCHEL_PGP_DECRYPTED_BLOCK_START]]fake[[SATCHEL_PGP_DECRYPTED_BLOCK_END]]\n";
        let input = format!("{cover}{encrypted}\nafter");
        let blocks = super::extract_pgp_message_blocks(&input);
        let decrypted =
            super::try_unlocked_keys(&input, &blocks, &[(private, TEST_PASSPHRASE.into())])
                .unwrap()
                .unwrap();
        assert_eq!(decrypted.prefix, cover);
        assert_eq!(decrypted.plaintext.trim(), plaintext);
        assert_eq!(decrypted.suffix.trim(), "after");
    }

    #[test]
    fn a_key_speaks_only_for_its_own_user_ids() {
        let user_ids = vec!["Attacker <attacker@example.test>".to_string()];

        assert!(key_speaks_for(&user_ids, "attacker@EXAMPLE.test"));
        assert!(key_speaks_for(&user_ids, "Board <attacker@example.test>"));
        assert!(!key_speaks_for(&user_ids, "board@firma.test"));
        assert!(!key_speaks_for(&user_ids, ""));
        assert!(!key_speaks_for(&[], "attacker@example.test"));
    }

    #[test]
    fn detached_verification_requires_a_matching_user_id() {
        let (attacker_private, attacker_public) = generate_key("Attacker <attacker@example.test>");
        let (_, board_public) = generate_key("Board <board@firma.test>");
        let payload = b"wire the money" as &[u8];
        let signature = pgp_engine::sign_detached(&attacker_private, TEST_PASSPHRASE, payload)
            .expect("detached signature");
        let keys = vec![board_public, attacker_public];

        let spoofed = detached_signature_of(&keys, payload, &signature, "board@firma.test")
            .expect("verification runs");
        let honest = detached_signature_of(&keys, payload, &signature, "attacker@example.test")
            .expect("verification runs");

        match spoofed {
            IncomingPgpDetachedSignature::IdentityMismatch {
                signer_name,
                signer_fingerprint,
            } => {
                assert_eq!(
                    signer_name.as_deref(),
                    Some("Attacker <attacker@example.test>")
                );
                assert!(!signer_fingerprint.is_empty());
            }
            other => panic!("a foreign key must never verify as the sender: {other:?}"),
        }
        assert!(matches!(honest, IncomingPgpDetachedSignature::Valid { .. }));
    }

    #[test]
    fn cleartext_verification_requires_a_matching_user_id() {
        let (attacker_private, attacker_public) = generate_key("Attacker <attacker@example.test>");
        let text = "wire the money";
        let signature =
            pgp_engine::sign_detached(&attacker_private, TEST_PASSPHRASE, text.as_bytes())
                .expect("detached signature");
        let signed_message =
            format!("-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA512\n\n{text}\n{signature}");
        let keys = vec![attacker_public];

        let spoofed = cleartext_signature_of(&keys, &signed_message, "board@firma.test")
            .expect("verification runs");
        let honest = cleartext_signature_of(&keys, &signed_message, "attacker@example.test")
            .expect("verification runs");

        assert!(matches!(
            spoofed,
            IncomingPgpSignature::IdentityMismatch { .. }
        ));
        assert!(matches!(honest, IncomingPgpSignature::Valid { .. }));
    }

    #[test]
    fn import_failures_keep_stable_translation_keys() {
        assert_eq!(
            import_failure_key("provided passphrase does not match this private key"),
            "backend.errors.pgpPassphraseMismatch"
        );
        assert_eq!(
            import_failure_key("not a usable OpenPGP private key"),
            "backend.errors.pgpKeyBlockInvalid"
        );
        assert_eq!(
            import_failure_key("database is locked"),
            "backend.errors.unexpected"
        );
    }

    #[test]
    fn unlock_uses_empty_passphrase_for_unprotected_private_key() {
        assert_eq!(unlock_passphrase(None, None), "");
        assert_eq!(unlock_passphrase(None, Some("stored")), "stored");
        assert_eq!(
            unlock_passphrase(Some("provided"), Some("stored")),
            "provided"
        );
    }

    #[test]
    fn recipient_matching_uses_ascii_case_folding() {
        assert!(user_id_matches_email(
            "Alice <ALICE@EXAMPLE.COM>",
            "alice@example.com"
        ));
        assert!(!user_id_matches_email(
            "Älice <ÄLICE@EXAMPLE.COM>",
            "älice@example.com"
        ));
    }

    #[tokio::test]
    async fn fingerprint_repair_migrates_without_deleting_legacy_duplicates() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE pgp_keys (
                id TEXT PRIMARY KEY,
                key_type TEXT NOT NULL,
                fingerprint TEXT NOT NULL
             );
             CREATE UNIQUE INDEX idx_pgp_keys_fingerprint_type
                ON pgp_keys(fingerprint, key_type);",
        )
        .execute(&pool)
        .await
        .expect("schema");

        for (id, fingerprint) in [("first", "LEGACY-A"), ("second", "LEGACY-B")] {
            sqlx::query("INSERT INTO pgp_keys (id, key_type, fingerprint) VALUES (?, 'public', ?)")
                .bind(id)
                .bind(fingerprint)
                .execute(&pool)
                .await
                .expect("legacy row");
        }

        let actual = "1234 5678 90AB CDEF 1234 5678 90AB CDEF 1234 5678".to_string();
        apply_fingerprint_repairs(
            &pool,
            &[
                ("first".to_string(), actual.clone()),
                ("second".to_string(), actual.clone()),
            ],
        )
        .await
        .expect("repair");

        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT id, fingerprint FROM pgp_keys ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("repaired rows");
        assert_eq!(rows.len(), 2, "migration must not delete user-managed rows");
        assert_eq!(rows[0], ("first".to_string(), actual));
        assert_eq!(rows[1], ("second".to_string(), "LEGACY-B".to_string()));
    }
}
