use std::collections::{HashMap, HashSet};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use satchel_core::db::smime::{
    SmimeCertificateInsert, SmimeCertificateMetadataRecord, SmimeCertificateRepository,
    SmimeRepositoryError, SmimeTrustOrigin,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::{
    commands::error::{CommandError, CommandResult},
    utils::{
        crypto::{decrypt_aead, encrypt_aead, SecretContext},
        mime::{extract_email_address, mailbox_matches},
        smime_engine::{self, GenerateSmimeRequest, SmimeCertMetadata, SmimeVerifyResult},
    },
    AppState, UnlockedSmimeSession,
};

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/"))]
pub struct SmimeCertificateDto {
    pub id: String,
    pub label: String,
    pub cert_type: String,
    pub fingerprint: String,
    pub subject_email: Option<String>,
    pub subject_cn: Option<String>,
    pub issuer_cn: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
    pub has_passphrase: bool,
    pub is_unlocked: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/", optional_fields = nullable))]
pub struct AddSmimeCertificateInput {
    pub label: Option<String>,
    pub pem: String,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/", optional_fields = nullable))]
pub struct ImportSmimePkcs12Input {
    pub label: Option<String>,
    pub p12_base64: String,
    pub passphrase: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/", optional_fields = nullable))]
pub struct GenerateSmimeKeypairInput {
    pub name: String,
    pub email: String,
    pub label: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/"))]
pub struct GenerateSmimeResult {
    pub certificate: SmimeCertificateDto,
    pub pem_certificate: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/", optional_fields = nullable))]
pub struct UnlockSmimeCertificatesInput {
    pub cert_id: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/"))]
pub struct UnlockSmimeResult {
    pub unlocked: usize,
    pub cert_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "app/commands/smime/"))]
pub struct SmimeVerifyMessageResult {
    pub state: String,
    pub signer_email: Option<String>,
    pub signer_cn: Option<String>,
    pub error_detail: Option<String>,
}

impl From<SmimeRepositoryError> for CommandError {
    fn from(error: SmimeRepositoryError) -> Self {
        match error {
            SmimeRepositoryError::NotFound => Self::new("backend.errors.smimeCertificateNotFound"),
            SmimeRepositoryError::MissingRecipients { recipients } => {
                Self::new("backend.errors.smimeRecipientCertificateMissing")
                    .with_param("recipients", recipients.join(", "))
            }
            SmimeRepositoryError::AmbiguousRecipients { recipients } => {
                Self::new("backend.errors.smimeRecipientCertificateAmbiguous")
                    .with_param("recipients", recipients.join(", "))
            }
            SmimeRepositoryError::Persistence(error) => {
                tracing::error!(error = ?error, "S/MIME certificate persistence failed");
                Self::new("backend.errors.smimeCertificatePersistenceFailed")
            }
        }
    }
}

fn repository_error_string(error: SmimeRepositoryError) -> String {
    CommandError::from(error).encoded()
}

#[tauri::command]
pub async fn list_smime_certificates(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<SmimeCertificateDto>> {
    let unlocked = state.smime_unlock_sessions.lock().await;
    let unlocked_ids = unlocked.keys().cloned().collect::<HashSet<_>>();
    drop(unlocked);
    SmimeCertificateRepository::new(pool.inner().as_ref())
        .list_metadata()
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| map_record_to_dto(row, &unlocked_ids))
                .collect()
        })
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn add_smime_certificate(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: AddSmimeCertificateInput,
) -> CommandResult<SmimeCertificateDto> {
    smime_engine::ensure_engine_available()?;
    let cert = first_pem_block(&input.pem, "CERTIFICATE")
        .ok_or_else(|| "PEM certificate block is required".to_string())?;
    let private_key = first_private_key_block(&input.pem);
    insert_smime_certificate(
        pool.inner(),
        &app,
        normalize_optional(input.label).as_deref(),
        SmimeTrustOrigin::Manual,
        &cert,
        private_key.as_deref(),
        normalize_optional(input.passphrase).as_deref(),
    )
    .await?
    .ok_or_else(|| CommandError::new("backend.errors.smimeCertificateExists"))
}

#[tauri::command]
pub async fn import_smime_pkcs12(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: ImportSmimePkcs12Input,
) -> CommandResult<SmimeCertificateDto> {
    let passphrase = input.passphrase.trim();
    if passphrase.is_empty() {
        return Err(CommandError::from("PKCS#12 passphrase is required"));
    }
    let p12 = STANDARD
        .decode(
            input
                .p12_base64
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>(),
        )
        .map_err(|err| err.to_string())?;
    // PKCS#12 unwrapping is a PBKDF — slow by design, and CPU-bound.
    let (cert, key) = {
        let (bundle, candidate) = (p12.clone(), passphrase.to_string());
        tokio::task::spawn_blocking(move || smime_engine::extract_pkcs12(&bundle, &candidate))
            .await
            .map_err(|error| format!("S/MIME bundle import failed: {error}"))??
    };
    insert_smime_certificate(
        pool.inner(),
        &app,
        normalize_optional(input.label).as_deref(),
        SmimeTrustOrigin::Manual,
        &cert,
        Some(&key),
        Some(passphrase),
    )
    .await?
    .ok_or_else(|| CommandError::new("backend.errors.smimeCertificateExists"))
}

#[tauri::command]
pub async fn generate_smime_keypair(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    input: GenerateSmimeKeypairInput,
) -> CommandResult<GenerateSmimeResult> {
    let name = input.name.trim();
    let email = input.email.trim();
    if name.is_empty() {
        return Err(CommandError::from("name is required"));
    }
    if !email.contains('@') {
        return Err(CommandError::from("valid email is required"));
    }
    let passphrase = normalize_optional(input.passphrase);
    // Key generation is seconds of CPU; it must not sit on an async worker.
    let generated = {
        let (name, email, passphrase) = (name.to_string(), email.to_string(), passphrase.clone());
        tokio::task::spawn_blocking(move || {
            smime_engine::generate_keypair(&GenerateSmimeRequest {
                name: &name,
                email: &email,
                passphrase: passphrase.as_deref(),
            })
        })
        .await
        .map_err(|error| format!("S/MIME key generation failed: {error}"))??
    };
    let certificate = insert_smime_certificate_with_metadata(
        pool.inner(),
        &app,
        normalize_optional(input.label).as_deref(),
        SmimeTrustOrigin::Manual,
        &generated.pem_certificate,
        Some(&generated.pem_private_key),
        passphrase.as_deref(),
        &generated.metadata,
    )
    .await?
    .ok_or_else(|| CommandError::new("backend.errors.smimeCertificateExists"))?;
    Ok(GenerateSmimeResult {
        fingerprint: certificate.fingerprint.clone(),
        pem_certificate: generated.pem_certificate,
        certificate,
    })
}

#[tauri::command]
pub async fn unlock_smime_certificates(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    input: UnlockSmimeCertificatesInput,
) -> CommandResult<UnlockSmimeResult> {
    smime_engine::ensure_engine_available()?;
    let rows = SmimeCertificateRepository::new(pool.inner())
        .private_certificates_for_unlock(input.cert_id.as_deref())
        .await
        .map_err(CommandError::from)?;
    if rows.is_empty() {
        return Err(CommandError::from(
            "no private S/MIME certificates available to unlock",
        ));
    }
    let provided_passphrase = normalize_optional(input.passphrase);
    let mut unlocked = Vec::new();
    for row in rows {
        let Some(encrypted_key) = row.pem_private_key_encrypted.as_deref() else {
            continue;
        };
        let pem_private_key = decrypt_aead(
            encrypted_key,
            &crate::utils::profile_dir(&app)?,
            SecretContext::smime_private_key(),
        )?;
        let candidate = if row.has_passphrase {
            provided_passphrase.as_deref()
        } else {
            provided_passphrase.as_deref().or(Some(""))
        };
        let unlocks = {
            let (key, candidate) = (pem_private_key.clone(), candidate.map(str::to_string));
            tokio::task::spawn_blocking(move || {
                smime_engine::verify_private_key_passphrase(&key, candidate.as_deref())
            })
            .await
            .map_err(|error| format!("S/MIME passphrase check failed: {error}"))??
        };
        if unlocks {
            let mut sessions = state.smime_unlock_sessions.lock().await;
            sessions.insert(
                row.id.clone(),
                UnlockedSmimeSession {
                    cert_id: row.id.clone(),
                    passphrase: candidate
                        .map(str::to_string)
                        .filter(|value| !value.trim().is_empty()),
                    unlocked_at: chrono::Utc::now(),
                },
            );
            drop(sessions);
            unlocked.push(row.id);
        }
    }
    if unlocked.is_empty() {
        return Err(CommandError::from(
            "passphrase does not match imported S/MIME certificates",
        ));
    }
    Ok(UnlockSmimeResult {
        unlocked: unlocked.len(),
        cert_ids: unlocked,
    })
}

#[tauri::command]
pub async fn delete_smime_certificate(
    state: tauri::State<'_, AppState>,
    pool: tauri::State<'_, SqlitePool>,
    cert_id: String,
) -> CommandResult<()> {
    SmimeCertificateRepository::new(pool.inner())
        .delete(&cert_id)
        .await
        .map_err(CommandError::from)?;
    let mut sessions = state.smime_unlock_sessions.lock().await;
    sessions.remove(&cert_id);
    Ok(())
}

pub async fn decrypt_message_with_unlocked_smime_certs(
    app: &AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    ciphertext: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let sessions = state.smime_unlock_sessions.lock().await;
    if sessions.is_empty() {
        return Ok(None);
    }
    let unlocked = sessions
        .iter()
        .map(|(cert_id, session)| (cert_id.clone(), session.passphrase.clone()))
        .collect::<HashMap<_, _>>();
    drop(sessions);
    let cert_ids = unlocked.keys().cloned().collect::<Vec<_>>();
    let rows = SmimeCertificateRepository::new(pool)
        .private_decryption_material(&cert_ids)
        .await
        .map_err(repository_error_string)?;
    let profile = crate::utils::profile_dir(app)?;
    let mut materials = Vec::new();
    for row in rows {
        let Some(encrypted_key) = row.pem_private_key_encrypted.as_deref() else {
            continue;
        };
        materials.push((
            decrypt_aead(encrypted_key, &profile, SecretContext::smime_private_key())?,
            row.pem_certificate,
            unlocked.get(&row.id).cloned().flatten(),
        ));
    }

    // Off the async worker, for the same reason the plugin path already is:
    // CMS decryption is CPU-bound and in-process, and a thread of encrypted
    // mail otherwise held a Tokio worker for its whole run.
    let payload = ciphertext.to_vec();
    tokio::task::spawn_blocking(move || try_unlocked_certificates(&materials, &payload))
        .await
        .map_err(|error| format!("S/MIME decryption task failed: {error}"))?
}

/// Try every unlocked certificate, first non-empty plaintext wins.
///
/// Owned and free of `AppHandle`, so it runs on a blocking thread.
fn try_unlocked_certificates(
    materials: &[(String, String, Option<String>)],
    ciphertext: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    for (pem_private_key, pem_certificate, passphrase) in materials {
        if let Ok(decrypted) = smime_engine::decrypt(
            pem_private_key,
            pem_certificate,
            passphrase.as_deref(),
            ciphertext,
        ) {
            if !decrypted.is_empty() {
                return Ok(Some(decrypted));
            }
        }
    }
    Ok(None)
}

/// Does the signer certificate speak for the mailbox in the `From:` header?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignerIdentity {
    MatchesSender,
    ForeignMailbox,
}

/// Signature maths, trust chain and signer identity are three answers, so they
/// get three states, ranked by how much of the claim survives. A signature
/// nobody vouches for says nothing about identity either, so the chain is
/// answered first; `signature_identity_mismatch` therefore means the one
/// precise thing: chained certificate, wrong mailbox.
fn signature_state(result: &SmimeVerifyResult, identity: SignerIdentity) -> &'static str {
    match (result.valid, result.chain_verified, identity) {
        (true, true, SignerIdentity::MatchesSender) => "signature_valid",
        (true, true, SignerIdentity::ForeignMailbox) => "signature_identity_mismatch",
        (true, false, _) => "signature_unchained",
        _ => "signature_invalid",
    }
}

/// A certificate speaks for the sender only through one of its own rfc822
/// names. Anything unreadable counts as foreign: fail closed.
fn signer_identity(result: &SmimeVerifyResult, from_address: &str) -> SignerIdentity {
    let Some(from) = extract_email_address(from_address) else {
        return SignerIdentity::ForeignMailbox;
    };
    let matches = result
        .signer_cert_pem
        .as_deref()
        .map(smime_engine::certificate_rfc822_identities)
        .unwrap_or_default()
        .iter()
        .any(|identity| mailbox_matches(identity, &from));
    if matches {
        SignerIdentity::MatchesSender
    } else {
        SignerIdentity::ForeignMailbox
    }
}

/// An auto-imported certificate becomes the encryption target for that address
/// later on, so only the one state that answers maths, chain and identity at
/// once may write to the store.
fn may_auto_import(state: &str) -> bool {
    state == "signature_valid"
}

pub async fn verify_smime_message(
    app: &AppHandle,
    pool: &SqlitePool,
    message_id: &str,
    signed_payload: &[u8],
    from_address: &str,
) -> SmimeVerifyMessageResult {
    let verified = {
        let payload = signed_payload.to_vec();
        let message_id = message_id.to_string();
        // Content, replacement keys and trust-origin edits all change this token.
        let certificates = sqlx::query_as::<_, (String, String)>(
            "SELECT pem_certificate, trust_origin FROM smime_certificates ORDER BY id",
        )
        .fetch_all(pool)
        .await;
        static CACHE: std::sync::LazyLock<
            std::sync::Mutex<
                super::message_processing::ProcessingCache<
                    Result<smime_engine::SmimeVerifyResult, String>,
                >,
            >,
        > = std::sync::LazyLock::new(|| std::sync::Mutex::new(Default::default()));
        tokio::task::spawn_blocking(move || {
            let Ok(certificates) = certificates else {
                return smime_engine::verify(&payload);
            };
            let mut parts: Vec<&[u8]> = certificates
                .iter()
                .flat_map(|(pem, trust)| [pem.as_bytes(), trust.as_bytes()])
                .collect();
            parts.push(&payload);
            super::message_processing::cached(
                &CACHE,
                message_id,
                super::message_processing::fingerprint(&parts),
                || {
                    let result = smime_engine::verify(&payload);
                    let bytes = payload.len()
                        + certificates
                            .iter()
                            .map(|(pem, trust)| pem.len() + trust.len())
                            .sum::<usize>();
                    (result, bytes)
                },
            )
        })
        .await
    };
    let verified = match verified {
        Ok(result) => result,
        Err(error) => Err(format!("S/MIME verification task failed: {error}")),
    };
    match verified {
        Ok(result) => {
            let identity = signer_identity(&result, from_address);
            let state = signature_state(&result, identity);
            if may_auto_import(state) {
                if let Some(cert) = result.signer_cert_pem.as_deref() {
                    let _ = auto_import_public_certificate(app, pool, cert).await;
                }
            }
            SmimeVerifyMessageResult {
                state: state.to_string(),
                signer_email: result.signer.as_ref().and_then(|m| m.subject_email.clone()),
                signer_cn: result.signer.as_ref().and_then(|m| m.subject_cn.clone()),
                error_detail: result.error,
            }
        }
        Err(err) => SmimeVerifyMessageResult {
            state: "error".to_string(),
            signer_email: None,
            signer_cn: None,
            error_detail: Some(err),
        },
    }
}

pub async fn resolve_recipient_certs(
    pool: &SqlitePool,
    emails: &[String],
) -> Result<Vec<(String, String)>, SmimeRepositoryError> {
    SmimeCertificateRepository::new(pool)
        .resolve_recipient_certificates(emails)
        .await
        .map(|certificates| {
            certificates
                .into_iter()
                .map(|certificate| (certificate.subject_email, certificate.pem_certificate))
                .collect()
        })
}

pub async fn load_private_signing_material(
    app: &AppHandle,
    state: &AppState,
    pool: &SqlitePool,
    cert_id: &str,
) -> Result<(String, String, Option<String>), String> {
    let sessions = state.smime_unlock_sessions.lock().await;
    let passphrase = sessions
        .get(cert_id)
        .map(|session| session.passphrase.clone())
        .ok_or_else(|| "S/MIME certificate is locked".to_string())?;
    drop(sessions);
    let row = SmimeCertificateRepository::new(pool)
        .private_signing_material(cert_id)
        .await
        .map_err(repository_error_string)?
        .ok_or_else(|| "S/MIME private certificate not found".to_string())?;
    let encrypted_key = row
        .pem_private_key_encrypted
        .as_deref()
        .ok_or_else(|| "S/MIME private key material is missing".to_string())?;
    Ok((
        decrypt_aead(
            encrypted_key,
            &crate::utils::profile_dir(app)?,
            SecretContext::smime_private_key(),
        )?,
        row.pem_certificate,
        passphrase,
    ))
}

fn map_record_to_dto(
    row: SmimeCertificateMetadataRecord,
    unlocked_ids: &HashSet<String>,
) -> SmimeCertificateDto {
    SmimeCertificateDto {
        is_unlocked: unlocked_ids.contains(&row.id),
        id: row.id,
        label: row.label,
        cert_type: row.cert_type,
        fingerprint: format_fingerprint(&row.fingerprint),
        subject_email: row.subject_email,
        subject_cn: row.subject_cn,
        issuer_cn: row.issuer_cn,
        not_before: row.not_before,
        not_after: row.not_after,
        has_passphrase: row.has_passphrase,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn insert_smime_certificate(
    pool: &SqlitePool,
    app: &AppHandle,
    label: Option<&str>,
    trust_origin: SmimeTrustOrigin,
    pem_certificate: &str,
    pem_private_key: Option<&str>,
    passphrase: Option<&str>,
) -> Result<Option<SmimeCertificateDto>, String> {
    let metadata = smime_engine::certificate_metadata(pem_certificate)?;
    insert_smime_certificate_with_metadata(
        pool,
        app,
        label,
        trust_origin,
        pem_certificate,
        pem_private_key,
        passphrase,
        &metadata,
    )
    .await
}

async fn insert_smime_certificate_with_metadata(
    pool: &SqlitePool,
    app: &AppHandle,
    label: Option<&str>,
    trust_origin: SmimeTrustOrigin,
    pem_certificate: &str,
    pem_private_key: Option<&str>,
    passphrase: Option<&str>,
    metadata: &SmimeCertMetadata,
) -> Result<Option<SmimeCertificateDto>, String> {
    if let Some(key) = pem_private_key {
        let matches = {
            let (key, candidate) = (key.to_string(), passphrase.map(str::to_string));
            tokio::task::spawn_blocking(move || {
                smime_engine::verify_private_key_passphrase(&key, candidate.as_deref())
            })
            .await
            .map_err(|error| format!("S/MIME passphrase check failed: {error}"))??
        };
        if !matches {
            return Err("provided passphrase does not match this private key".to_string());
        }
    }
    let profile_dir = crate::utils::profile_dir(app)?;
    let encrypted_key = pem_private_key
        .map(|key| encrypt_aead(key, &profile_dir, SecretContext::smime_private_key()))
        .transpose()?;
    let has_passphrase = passphrase
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    SmimeCertificateRepository::new(pool)
        .insert(SmimeCertificateInsert {
            label: label.map(str::to_owned),
            trust_origin,
            fingerprint: metadata.fingerprint.clone(),
            subject_email: metadata.subject_email.clone(),
            subject_cn: metadata.subject_cn.clone(),
            issuer_cn: metadata.issuer_cn.clone(),
            not_before: metadata.not_before.clone(),
            not_after: metadata.not_after.clone(),
            pem_certificate: pem_certificate.to_owned(),
            pem_private_key_encrypted: encrypted_key,
            has_passphrase,
        })
        .await
        .map(|record| record.map(|record| map_record_to_dto(record, &HashSet::new())))
        .map_err(repository_error_string)
}

/// Records the certificate as `auto`: recipient resolution ranks it below
/// anything the user imported by hand.
async fn auto_import_public_certificate(
    app: &AppHandle,
    pool: &SqlitePool,
    pem_certificate: &str,
) -> Result<(), String> {
    let _ = insert_smime_certificate(
        pool,
        app,
        None,
        SmimeTrustOrigin::Auto,
        pem_certificate,
        None,
        None,
    )
    .await?;
    Ok(())
}

fn first_pem_block(input: &str, label: &str) -> Option<String> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut current = String::new();
    let mut collecting = false;
    for line in input.lines() {
        if line.contains(&begin) {
            collecting = true;
            current.clear();
        }
        if collecting {
            current.push_str(line);
            current.push('\n');
        }
        if collecting && line.contains(&end) {
            return Some(current.trim().to_string());
        }
    }
    None
}

fn first_private_key_block(input: &str) -> Option<String> {
    [
        "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
    ]
    .iter()
    .find_map(|label| first_pem_block(input, label))
}

fn normalize_optional(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

#[cfg(test)]
mod tests {
    #[test]
    fn unlocked_decryption_preserves_non_utf8_mime_bytes() {
        let key = super::smime_engine::generate_keypair(&super::GenerateSmimeRequest {
            name: "Synthetic",
            email: "recipient@example.test",
            passphrase: None,
        })
        .unwrap();
        let payload = b"Content-Type: text/plain; charset=iso-8859-1\r\n\r\nGr\xfc\xdfe";
        let encrypted =
            super::smime_engine::encrypt(std::slice::from_ref(&key.pem_certificate), payload)
                .unwrap();
        let bytes = super::try_unlocked_certificates(
            &[(key.pem_private_key, key.pem_certificate, None)],
            &encrypted,
        )
        .unwrap()
        .unwrap();
        assert_eq!(bytes, payload);
        let body = satchel_core::email::decrypted::parse_decrypted_body(&bytes).unwrap();
        assert_eq!(body.body.text.as_deref(), Some("Grüße"));
    }

    use std::collections::HashSet;

    use satchel_core::db::smime::{SmimeCertificateMetadataRecord, SmimeRepositoryError};

    use super::{
        map_record_to_dto, may_auto_import, repository_error_string, signature_state,
        signer_identity, CommandError, SignerIdentity,
    };
    use crate::utils::smime_engine::{self, GenerateSmimeRequest, SmimeVerifyResult};

    #[test]
    fn repository_errors_use_stable_redacted_ipc_keys() {
        assert_eq!(
            CommandError::from(SmimeRepositoryError::NotFound).key,
            "backend.errors.smimeCertificateNotFound"
        );

        let detail = "near SELECT: encrypted-private-key";
        let error = CommandError::from(SmimeRepositoryError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_string(&error).expect("command error JSON");
        assert_eq!(
            error.key,
            "backend.errors.smimeCertificatePersistenceFailed"
        );
        assert!(!serialized.contains(detail));

        let nested = repository_error_string(SmimeRepositoryError::Persistence(
            sqlx::Error::Protocol(detail.into()),
        ));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&nested).expect("encoded command error"),
            serde_json::json!({ "key": "backend.errors.smimeCertificatePersistenceFailed" })
        );
        assert!(!nested.contains(detail));
    }

    #[test]
    fn metadata_mapping_formats_fingerprints_and_applies_unlock_state() {
        let record = SmimeCertificateMetadataRecord {
            id: "cert-1".to_owned(),
            label: "Person".to_owned(),
            cert_type: "private".to_owned(),
            fingerprint: "aa:bb:cc:dd:ee".to_owned(),
            subject_email: Some("person@example.test".to_owned()),
            subject_cn: Some("Person".to_owned()),
            issuer_cn: Some("Test CA".to_owned()),
            not_before: None,
            not_after: None,
            has_passphrase: true,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let dto = map_record_to_dto(record, &HashSet::from(["cert-1".to_owned()]));

        assert_eq!(dto.fingerprint, "AABB CCDD EE");
        assert!(dto.is_unlocked);
        assert!(dto.has_passphrase);
    }

    fn verify_result(valid: bool, chain_verified: bool) -> SmimeVerifyResult {
        SmimeVerifyResult {
            valid,
            chain_verified,
            signer_cert_pem: None,
            signer: None,
            error: None,
        }
    }

    fn certificate_for(email: &str) -> String {
        smime_engine::generate_keypair(&GenerateSmimeRequest {
            name: "Signer",
            email,
            passphrase: None,
        })
        .expect("generate test certificate")
        .pem_certificate
    }

    #[test]
    fn an_unchained_signature_gets_its_own_state() {
        assert_eq!(
            signature_state(&verify_result(true, true), SignerIdentity::MatchesSender),
            "signature_valid"
        );
        assert_eq!(
            signature_state(&verify_result(true, false), SignerIdentity::MatchesSender),
            "signature_unchained"
        );
        assert_eq!(
            signature_state(&verify_result(false, false), SignerIdentity::MatchesSender),
            "signature_invalid"
        );
    }

    /// An unchained or foreign signature must leave the certificate store
    /// untouched: the import is what turns a signature into an encryption key.
    #[test]
    fn only_a_fully_verified_signature_may_import_its_certificate() {
        let mut result = verify_result(true, true);
        result.signer_cert_pem = Some(certificate_for("signer@example.test"));
        let matching = signer_identity(&result, "signer@example.test");

        assert!(may_auto_import(signature_state(&result, matching)));
        for state in [
            signature_state(&verify_result(true, false), matching),
            signature_state(&verify_result(false, true), matching),
            signature_state(&result, SignerIdentity::ForeignMailbox),
        ] {
            assert!(!may_auto_import(state), "{state} must not import");
        }
    }

    #[test]
    fn smime_signature_from_a_foreign_mailbox_is_reported_as_mismatch() {
        let mut result = verify_result(true, true);
        result.signer_cert_pem = Some(certificate_for("attacker@example.test"));

        let foreign = signer_identity(&result, "board@firma.test");
        let own = signer_identity(&result, "Attacker <attacker@EXAMPLE.test>");

        assert_eq!(foreign, SignerIdentity::ForeignMailbox);
        assert_eq!(
            signature_state(&result, foreign),
            "signature_identity_mismatch"
        );
        assert_eq!(own, SignerIdentity::MatchesSender);
        assert_eq!(signature_state(&result, own), "signature_valid");
    }

    #[test]
    fn a_signature_without_a_readable_signer_never_matches_the_sender() {
        let result = verify_result(true, true);

        assert_eq!(
            signer_identity(&result, "board@firma.test"),
            SignerIdentity::ForeignMailbox
        );
        assert_eq!(
            signer_identity(&verify_result(true, true), ""),
            SignerIdentity::ForeignMailbox
        );
    }

    #[test]
    fn a_self_signed_message_never_reaches_the_valid_state() {
        let generated = smime_engine::generate_keypair(&GenerateSmimeRequest {
            name: "Board",
            email: "board@satchel.local",
            passphrase: None,
        })
        .expect("generate test certificate");
        let signed = smime_engine::sign(
            &generated.pem_private_key,
            &generated.pem_certificate,
            None,
            b"Subject: payment\r\n\r\nwire the money",
        )
        .expect("sign payload");

        let result = smime_engine::verify(&signed).expect("verify payload");

        let identity = signer_identity(&result, "board@satchel.local");
        assert_eq!(identity, SignerIdentity::MatchesSender);
        assert_ne!(signature_state(&result, identity), "signature_valid");
    }

    #[test]
    fn smime_sign_extracts_signer_certificate() {
        smime_engine::ensure_engine_available()
            .expect("openssl should be available for S/MIME tests");
        let generated = smime_engine::generate_keypair(&GenerateSmimeRequest {
            name: "Satchel Test",
            email: "smime-test@satchel.local",
            passphrase: None,
        })
        .expect("generate test certificate");

        let signed = smime_engine::sign(
            &generated.pem_private_key,
            &generated.pem_certificate,
            None,
            b"Subject: test\r\n\r\nhello",
        )
        .expect("sign payload");
        let certs = smime_engine::extract_certificates(&signed).expect("extract certificates");
        assert!(!certs.is_empty());
    }

    #[test]
    fn smime_encrypt_decrypt_roundtrip() {
        smime_engine::ensure_engine_available()
            .expect("openssl should be available for S/MIME tests");
        let generated = smime_engine::generate_keypair(&GenerateSmimeRequest {
            name: "Satchel Test",
            email: "smime-test@satchel.local",
            passphrase: None,
        })
        .expect("generate test certificate");
        let plaintext = b"hello encrypted world";
        let encrypted =
            smime_engine::encrypt(std::slice::from_ref(&generated.pem_certificate), plaintext)
                .expect("encrypt payload");
        let decrypted = smime_engine::decrypt(
            &generated.pem_private_key,
            &generated.pem_certificate,
            None,
            &encrypted,
        )
        .expect("decrypt payload");
        assert_eq!(String::from_utf8_lossy(&decrypted), "hello encrypted world");
    }
}
