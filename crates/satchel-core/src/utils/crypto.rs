use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// No associated data: a blob decrypts wherever it is pasted. Read, never written.
const LEGACY_PREFIX: &str = "enc:v1:";
/// Bound to its column and row through AES-GCM associated data.
const ENCRYPTED_PREFIX: &str = "enc:v2:";

/// Where a secret is stored. A ciphertext only decrypts at the place it was
/// written for, so a row copied onto another account's row is refused rather
/// than handed to that account's provider.
#[derive(Debug, Clone, Copy)]
pub struct SecretContext<'a> {
    column: &'static str,
    row: &'a str,
}

impl<'a> SecretContext<'a> {
    pub fn account_config(account_id: &'a str) -> Self {
        Self {
            column: "accounts.config",
            row: account_id,
        }
    }
    // Key material is bound to its column only: re-import replaces key rows
    // under new ids, and S/MIME rows get theirs inside the repository.
    pub fn pgp_armored_key() -> Self {
        Self {
            column: "pgp_keys.armored_key",
            row: "",
        }
    }
    pub fn pgp_passphrase() -> Self {
        Self {
            column: "pgp_keys.passphrase",
            row: "",
        }
    }
    pub fn smime_private_key() -> Self {
        Self {
            column: "smime_certificates.pem_private_key_encrypted",
            row: "",
        }
    }
    pub fn oauth_code_verifier(state: &'a str) -> Self {
        Self {
            column: "oauth_flow_sessions.encrypted_code_verifier",
            row: state,
        }
    }
    /// A master key wrapped for a backup; one per file, so no row.
    pub fn wrapped_master_key() -> Self {
        Self {
            column: "backup.master_key",
            row: "",
        }
    }

    fn associated_data(&self) -> Vec<u8> {
        format!("{}|{}", self.column, self.row).into_bytes()
    }
}

/// Cached master key.
///
/// Populated once during startup (which knows the profile directory for the
/// file fallback) so background workers — the IDLE supervisor, the outbox, the
/// RSS poller — can decrypt account credentials without threading a path
/// through every call site.
static MASTER_KEY: super::master_key_cache::MasterKeyCache =
    super::master_key_cache::MasterKeyCache::new();

/// Start the shared load off the UI thread before registering credential users.
pub fn start_master_key_load(profile_dir: std::path::PathBuf) {
    MASTER_KEY.start(move || load_master_key_uncached(&profile_dir));
}

pub fn init_master_key(profile_dir: &Path) -> Result<(), String> {
    load_or_create_master_key(profile_dir).map(|_| ())
}

fn cached_master_key() -> Result<[u8; 32], String> {
    MASTER_KEY.load(load_key_without_profile)
}

/// Await startup without holding an async executor thread during keychain prompts.
pub async fn wait_for_master_key() -> Result<(), String> {
    MASTER_KEY
        .load_async(load_key_without_profile)
        .await
        .map(|_| ())
}

fn load_key_without_profile() -> Result<[u8; 32], String> {
    if cfg!(any(test, feature = "test-support")) {
        return Ok([7; 32]);
    }
    #[cfg(desktop)]
    let loaded = load_master_key_from_keychain()
        .and_then(|key| key.ok_or_else(|| "master key missing".to_string()));
    #[cfg(mobile)]
    let loaded = load_or_create_mobile_master_key();
    loaded.map_err(|err| format!("credential key unavailable: {err}"))
}

/// Encrypt a secret without the profile directory after startup initialization.
pub fn encrypt_secret(value: &str, context: SecretContext<'_>) -> Result<String, String> {
    encrypt_aead_with_key(value, &cached_master_key()?, context)
}

/// Counterpart to `encrypt_secret`. A value without Satchel's ciphertext
/// marker is an error, not a plaintext to pass through.
pub fn decrypt_secret(value: &str, context: SecretContext<'_>) -> Result<String, String> {
    if !is_encrypted(value) {
        return Err("protected secret is not encrypted".to_string());
    }
    decrypt_with_key(value, &cached_master_key()?, context)
}

pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENCRYPTED_PREFIX) || value.starts_with(LEGACY_PREFIX)
}

/// Written before secrets were bound to their row; rewritten on the next write.
pub fn is_legacy_encrypted(value: &str) -> bool {
    value.starts_with(LEGACY_PREFIX)
}

/// Inject a deterministic master key so tests exercise the real encrypt and
/// decrypt paths without depending on a reachable system keychain.
#[cfg(any(test, feature = "test-support"))]
pub fn seed_master_key_for_tests() {
    let _ = MASTER_KEY.load(|| Ok([7_u8; 32]));
}
#[cfg(not(any(test, feature = "test-support")))]
const KEYRING_SERVICE: &str = "satchel";
#[cfg(any(test, feature = "test-support"))]
const KEYRING_SERVICE: &str = "satchel-test-credentials";
const KEYRING_ACCOUNT: &str = "pgp-master-key-v1";

/// Marks an exported master key that is wrapped under a user passphrase.
/// Format: `wrapped:v1:<m_cost>:<t_cost>:<p_cost>:<salt_b64>:<enc:v2:…>`.
/// Costs and salt travel with the file so raising them later stays readable.
const WRAPPED_KEY_PREFIX: &str = "wrapped:v1:";

/// Message key for a wrapped export handed no passphrase.
pub const BACKUP_PASSPHRASE_REQUIRED: &str = "backend.errors.backupPassphraseRequired";
/// Message key for a wrapped export the passphrase does not open.
pub const BACKUP_PASSPHRASE_REJECTED: &str = "backend.errors.backupPassphraseRejected";

/// New backups use 64 MiB and three passes; older cost profiles remain readable.
const WRAP_M_COST: u32 = 64 * 1024;
const WRAP_T_COST: u32 = 3;
const WRAP_P_COST: u32 = 1;
pub const BACKUP_PASSPHRASE_TOO_SHORT: &str = "backend.errors.backupPassphraseTooShort";
pub const BACKUP_PASSPHRASE_MIN_CHARS: usize = 12;

pub fn validate_backup_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.chars().count() < BACKUP_PASSPHRASE_MIN_CHARS {
        return Err(BACKUP_PASSPHRASE_TOO_SHORT.to_string());
    }
    Ok(())
}

fn derive_wrapping_key(
    passphrase: &str,
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<[u8; 32], String> {
    // Reject untrusted costs before Argon2 allocates or starts work.
    if !(8..=128 * 1024).contains(&m_cost)
        || !(1..=6).contains(&t_cost)
        || !(1..=4).contains(&p_cost)
        || !(8..=64).contains(&salt.len())
    {
        return Err("invalid key derivation parameters".to_string());
    }
    let params = argon2::Params::new(m_cost, t_cost, p_cost, Some(32))
        .map_err(|err| format!("invalid key derivation parameters: {err}"))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut derived = [0_u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut derived)
        .map_err(|err| format!("key derivation failed: {err}"))?;
    Ok(derived)
}

fn wrap_master_key(key: &[u8; 32], passphrase: &str) -> Result<String, String> {
    validate_backup_passphrase(passphrase)?;
    let mut salt = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let wrapping = derive_wrapping_key(passphrase, &salt, WRAP_M_COST, WRAP_T_COST, WRAP_P_COST)?;
    let sealed = encrypt_aead_with_key(
        &STANDARD.encode(key),
        &wrapping,
        SecretContext::wrapped_master_key(),
    )?;
    Ok(format!(
        "{WRAPPED_KEY_PREFIX}{WRAP_M_COST}:{WRAP_T_COST}:{WRAP_P_COST}:{}:{sealed}",
        STANDARD.encode(salt)
    ))
}

fn unwrap_master_key(value: &str, passphrase: Option<&str>) -> Result<[u8; 32], String> {
    let Some(passphrase) = passphrase.filter(|value| !value.is_empty()) else {
        return Err(BACKUP_PASSPHRASE_REQUIRED.to_string());
    };
    let payload = value
        .strip_prefix(WRAPPED_KEY_PREFIX)
        .ok_or_else(|| "not a wrapped master key".to_string())?;
    let parts: Vec<&str> = payload.splitn(5, ':').collect();
    let [m_cost, t_cost, p_cost, salt, sealed] = parts[..] else {
        return Err("malformed wrapped master key".to_string());
    };
    let parse = |value: &str| {
        value
            .parse::<u32>()
            .map_err(|_| "malformed wrapped master key".to_string())
    };
    let salt = STANDARD
        .decode(salt)
        .map_err(|_| "malformed wrapped master key".to_string())?;
    let wrapping = derive_wrapping_key(
        passphrase,
        &salt,
        parse(m_cost)?,
        parse(t_cost)?,
        parse(p_cost)?,
    )?;
    // AES-GCM rejects a wrong key at the tag, so this is the passphrase check.
    let decoded = decrypt_with_key(sealed, &wrapping, SecretContext::wrapped_master_key())
        .map_err(|_| BACKUP_PASSPHRASE_REJECTED.to_string())?;
    decode_key_material(decoded.trim())
}

/// Whether a backup directory carries a passphrase-wrapped master key.
pub fn backup_master_key_is_wrapped(directory: &Path) -> bool {
    crate::db::MASTER_KEY_FILES.iter().any(|name| {
        std::fs::read_to_string(directory.join(name))
            .is_ok_and(|value| value.trim_start().starts_with(WRAPPED_KEY_PREFIX))
    })
}

/// Automatic macOS backups reference a separate keychain item instead of exporting plaintext.
pub const LOCAL_BACKUP_KEY_MARKER: &str = "master-key.keychain";

pub fn backup_master_key_count(directory: &Path) -> usize {
    crate::db::MASTER_KEY_FILES
        .iter()
        .filter(|name| directory.join(name).is_file())
        .count()
        + usize::from(directory.join(LOCAL_BACKUP_KEY_MARKER).is_file())
}

#[cfg(desktop)]
fn local_backup_key_account(directory: &Path) -> Result<Option<String>, String> {
    let marker = directory.join(LOCAL_BACKUP_KEY_MARKER);
    if !marker.is_file() {
        return Ok(None);
    }
    let account = std::fs::read_to_string(marker)
        .map_err(|_| "backend.errors.restoreStagingChanged".to_string())?;
    let account = account.trim();
    let suffix = account.strip_prefix("pgp-master-key-v1:").unwrap_or("");
    if suffix.len() != 64
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("backend.errors.restoreStagingChanged".to_string());
    }
    Ok(Some(account.to_string()))
}

#[cfg(desktop)]
fn local_backup_master_key(
    directory: &Path,
    load: impl FnOnce(&str) -> Result<Option<[u8; 32]>, String>,
) -> Result<Option<[u8; 32]>, String> {
    let Some(account) = local_backup_key_account(directory)? else {
        return Ok(None);
    };
    load(&account)
        .map_err(|_| "backend.errors.restoreKeyUnavailable".to_string())?
        .map(Some)
        .ok_or_else(|| "backend.errors.restoreKeyUnavailable".to_string())
}

/// Remove the snapshot only when the marker belongs to this backup directory.
pub fn remove_local_backup_master_key(directory: &Path) -> Result<(), String> {
    #[cfg(desktop)]
    remove_local_backup_master_key_with(directory, |account| {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(|_| "backend.errors.restoreKeyUnavailable".to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("backend.errors.restoreKeyUnavailable".to_string()),
        }
    })?;
    #[cfg(mobile)]
    let _ = directory;
    Ok(())
}

#[cfg(desktop)]
fn remove_local_backup_master_key_with(
    directory: &Path,
    remove: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let Some(account) = local_backup_key_account(directory)? else {
        return Ok(());
    };
    if account != profile_keyring_account(directory) {
        return Err("backend.errors.restoreStagingChanged".to_string());
    }
    remove(&account)
}

/// Read exported keys or an automatic backup's local keychain snapshot. This never changes
/// the active OS key or the process-wide key cache.
///
/// Both shapes are accepted: the wrapped form written since SEC-08 and the
/// bare base64 an older Satchel wrote, so an existing backup still restores.
pub fn backup_master_keys(
    directory: &Path,
    passphrase: Option<&str>,
) -> Result<Vec<[u8; 32]>, String> {
    let mut keys = Vec::new();
    for name in crate::db::MASTER_KEY_FILES {
        let path = directory.join(name);
        if path.is_file() {
            let value = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let value = value.trim();
            if value.starts_with(WRAPPED_KEY_PREFIX) {
                keys.push(unwrap_master_key(value, passphrase)?);
            } else {
                keys.push(decode_key_material(value)?);
            }
        }
    }
    #[cfg(desktop)]
    if let Some(key) = local_backup_master_key(directory, load_keychain_account)? {
        keys.push(key);
    }
    #[cfg(mobile)]
    if directory.join(LOCAL_BACKUP_KEY_MARKER).is_file() {
        return Err("backend.errors.restoreKeyUnavailable".to_string());
    }
    Ok(keys)
}

/// Export the key actually used by this profile, including keychain-only
/// installations. The caller owns the user-selected backup directory.
///
/// With a passphrase the file holds no key bytes at all. Without one it holds
/// the bare key, which is why the caller has to have taken an explicit opt-out
/// from the user first.
pub fn export_master_key(
    profile: &Path,
    destination: &Path,
    passphrase: Option<&str>,
) -> Result<usize, String> {
    let key = load_or_create_master_key(profile)?;
    let contents = match passphrase.filter(|value| !value.is_empty()) {
        Some(passphrase) => wrap_master_key(&key, passphrase)?,
        None => STANDARD.encode(key),
    };
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(destination).map_err(|e| e.to_string())?;
    file.write_all(contents.as_bytes())
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist(destination.join(crate::db::MASTER_KEY_FILES[0]))
        .map_err(|e| e.to_string())?;
    Ok(1)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRef {
    pub account_id: String,
    pub key: String,
}

pub fn encrypt_aead(
    value: &str,
    profile_dir: &Path,
    context: SecretContext<'_>,
) -> Result<String, String> {
    let key = load_or_create_master_key(profile_dir)?;
    encrypt_aead_with_key(value, &key, context)
}

/// Values without a ciphertext marker are returned unchanged; PGP rows from
/// before encryption existed still rely on that.
pub fn decrypt_aead(
    value: &str,
    profile_dir: &Path,
    context: SecretContext<'_>,
) -> Result<String, String> {
    if !is_encrypted(value) {
        return Ok(value.to_string());
    }
    let key = load_or_create_master_key(profile_dir)?;
    decrypt_with_key(value, &key, context)
}

pub fn decrypt_with_key(
    value: &str,
    key: &[u8; 32],
    context: SecretContext<'_>,
) -> Result<String, String> {
    let (payload, associated_data) = if let Some(payload) = value.strip_prefix(ENCRYPTED_PREFIX) {
        (payload, context.associated_data())
    } else if let Some(payload) = value.strip_prefix(LEGACY_PREFIX) {
        (payload, Vec::new())
    } else {
        return Err("invalid encrypted secret format".to_string());
    };
    let (nonce_encoded, ciphertext_encoded) = payload
        .split_once(':')
        .ok_or_else(|| "invalid encrypted secret format".to_string())?;
    let nonce_bytes = STANDARD
        .decode(nonce_encoded)
        .map_err(|err| err.to_string())?;
    let ciphertext = STANDARD
        .decode(ciphertext_encoded)
        .map_err(|err| err.to_string())?;
    if nonce_bytes.len() != 12 {
        return Err("invalid encrypted secret nonce".to_string());
    }

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| err.to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plain = cipher
        .decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: &ciphertext,
                aad: &associated_data,
            },
        )
        .map_err(|_| "failed to decrypt protected secret".to_string())?;
    String::from_utf8(plain).map_err(|err| err.to_string())
}

pub fn encrypt_aead_with_key(
    value: &str,
    key: &[u8; 32],
    context: SecretContext<'_>,
) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| err.to_string())?;
    let mut nonce_bytes = [0_u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let associated_data = context.associated_data();
    let ciphertext = cipher
        .encrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: value.as_bytes(),
                aad: &associated_data,
            },
        )
        .map_err(|err| err.to_string())?;

    let encoded_nonce = STANDARD.encode(nonce_bytes);
    let encoded_ciphertext = STANDARD.encode(ciphertext);
    Ok(format!(
        "{ENCRYPTED_PREFIX}{encoded_nonce}:{encoded_ciphertext}"
    ))
}

/// `profile_dir` is `<AppData>/satchel`: the desktop file fallback lives there
/// beside the database. Mobile never touches it.
pub fn load_or_create_master_key(profile_dir: &Path) -> Result<[u8; 32], String> {
    MASTER_KEY.load(|| load_master_key_uncached(profile_dir))
}

fn load_master_key_uncached(profile_dir: &Path) -> Result<[u8; 32], String> {
    if cfg!(any(test, feature = "test-support")) {
        return Ok([7; 32]);
    }
    #[cfg(mobile)]
    {
        let _ = profile_dir;
        load_or_create_mobile_master_key()
    }
    #[cfg(desktop)]
    {
        let account = profile_keyring_account(profile_dir);
        load_desktop_master_key(
            profile_dir,
            || load_keychain_account(&account),
            |key| store_keychain_account(&account, key),
            load_master_key_from_keychain,
            plaintext_fallback_allowed(),
        )
    }
}

#[cfg(desktop)]
fn plaintext_fallback_allowed() -> bool {
    std::env::var("SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK").is_ok_and(|value| {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        )
    })
}

/// Point `keyring-core` at the platform's own secure store.
///
/// Unlike desktop `keyring`, `keyring-core` ships no backend of its own — the
/// Android Keystore and the iOS Keychain are registered here, once, before the
/// first entry is opened.
#[cfg(mobile)]
fn initialize_mobile_keyring() -> Result<(), String> {
    use std::sync::OnceLock;
    static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

    INITIALIZED
        .get_or_init(|| {
            #[cfg(target_os = "android")]
            {
                let store = android_native_keyring_store::Store::new()
                    .map_err(|err| format!("Android Keystore unavailable: {err}"))?;
                keyring_core::set_default_store(store);
            }
            #[cfg(target_os = "ios")]
            {
                let store = apple_native_keyring_store::protected::Store::new()
                    .map_err(|err| format!("iOS Keychain unavailable: {err}"))?;
                keyring_core::set_default_store(store);
            }
            Ok(())
        })
        .clone()
}

/// Master key on mobile, with no file fallback.
///
/// Desktop retains existing fallback files when the keychain is unreachable;
/// creating a new plaintext key requires explicit opt-in. On a phone the OS store is
/// always available, and app-private storage is not a place to keep the key
/// that protects every stored mail password — so an error stays an error.
#[cfg(mobile)]
fn load_or_create_mobile_master_key() -> Result<[u8; 32], String> {
    initialize_mobile_keyring()?;
    let entry = keyring_core::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|err| err.to_string())?;

    match entry.get_secret() {
        Ok(bytes) => decode_raw_key_material(&bytes),
        Err(keyring_core::Error::NoEntry) => {
            let mut key = [0_u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            entry.set_secret(&key).map_err(|err| err.to_string())?;
            Ok(key)
        }
        Err(err) => Err(format!("native credential store unavailable: {err}")),
    }
}

#[cfg(mobile)]
fn decode_raw_key_material(bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() != 32 {
        return Err("invalid master key length".to_string());
    }
    let mut out = [0_u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

#[cfg(desktop)]
fn load_master_key_from_keychain() -> Result<Option<[u8; 32]>, String> {
    load_keychain_account(KEYRING_ACCOUNT)
}

#[cfg(desktop)]
fn profile_keyring_account(profile_dir: &Path) -> String {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hash.update(profile_dir.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in profile_dir.as_os_str().encode_wide() {
            hash.update(unit.to_le_bytes());
        }
    }
    format!("{KEYRING_ACCOUNT}:{:x}", hash.finalize())
}

#[cfg(desktop)]
fn load_keychain_account(account: &str) -> Result<Option<[u8; 32]>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|err| err.to_string())?;
    match entry.get_password() {
        Ok(encoded) => decode_key_material(&encoded).map(Some),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(desktop)]
fn store_keychain_account(account: &str, key: &[u8; 32]) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|err| err.to_string())?;
    entry
        .set_password(&STANDARD.encode(key))
        .map_err(|err| err.to_string())
}

#[cfg(desktop)]
fn load_desktop_master_key(
    profile_dir: &Path,
    mut load: impl FnMut() -> Result<Option<[u8; 32]>, String>,
    mut store: impl FnMut(&[u8; 32]) -> Result<(), String>,
    mut legacy: impl FnMut() -> Result<Option<[u8; 32]>, String>,
    allow_plaintext: bool,
) -> Result<[u8; 32], String> {
    // The existing file is authoritative until its migration is verified.
    if let Some(key) = load_existing_file_master_key(profile_dir)? {
        let migrated = store(&key).and_then(|()| verify_keychain_key(&key, load()));
        match migrated {
            Ok(()) => {
                if let Err(err) = std::fs::remove_file(master_key_file_path(profile_dir)) {
                    tracing::warn!(%err, "verified key migration could not remove plaintext file");
                }
            }
            Err(err) => {
                tracing::warn!(%err, "keychain migration failed; retaining existing file key")
            }
        }
        return Ok(key);
    }

    let loaded = load().and_then(|existing| {
        if let Some(key) = existing {
            return Ok(key);
        }
        // Adopt an older shared key without replacing it for another profile.
        let key = legacy()?.unwrap_or_else(|| {
            let mut key = [0_u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            key
        });
        store(&key)?;
        verify_keychain_key(&key, load())?;
        Ok(key)
    });
    let failure = match loaded {
        Ok(key) => return Ok(key),
        Err(err) => err,
    };
    if allow_plaintext {
        tracing::warn!(%failure, "keychain unavailable; explicit plaintext fallback enabled");
        return create_file_master_key(profile_dir);
    }
    Err(format!(
        "system keychain unavailable: {failure}; refusing to write a plaintext master key. \
         Make the keychain reachable or set SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK=1 to opt in."
    ))
}

#[cfg(desktop)]
fn verify_keychain_key(
    expected: &[u8; 32],
    actual: Result<Option<[u8; 32]>, String>,
) -> Result<(), String> {
    if actual? != Some(*expected) {
        return Err("keychain master key read-back did not match".to_string());
    }
    Ok(())
}

#[cfg(desktop)]
fn master_key_file_path(profile_dir: &Path) -> std::path::PathBuf {
    profile_dir.join("pgp-master-key-v1")
}

/// Reads a previously created fallback key, if one exists. Returns `Ok(None)`
/// when no fallback key was ever written — the caller then decides whether
/// creating one is acceptable.
#[cfg(desktop)]
fn load_existing_file_master_key(profile_dir: &Path) -> Result<Option<[u8; 32]>, String> {
    let key_path = master_key_file_path(profile_dir);
    if !key_path.exists() {
        return Ok(None);
    }
    let encoded = std::fs::read_to_string(&key_path)
        .map_err(|err| format!("fallback credential key {}: {err}", key_path.display()))?;
    // A key written by an older build went through `fs::write` and may still
    // carry the umask default; narrow it on the way past.
    restrict_fallback_key_permissions(&key_path);
    decode_key_material(encoded.trim())
        .map_err(|err| format!("fallback credential key {}: {err}", key_path.display()))
        .map(Some)
}

/// Creates and persists a plaintext fallback master key. Only reachable when
/// the caller has already decided that writing it is acceptable (explicit
/// opt-out). The file gets owner-only permissions on Unix.
#[cfg(desktop)]
fn create_file_master_key(profile_dir: &Path) -> Result<[u8; 32], String> {
    let key_path = master_key_file_path(profile_dir);
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let mut key = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    create_owner_only_file(&key_path, STANDARD.encode(key).as_bytes())?;
    tracing::warn!(
        path = %key_path.display(),
        "credential master key written as plaintext file fallback; \
         anyone with this file can decrypt stored passwords"
    );
    Ok(key)
}

/// Creates the fallback key file owner-only from the first byte.
///
/// `fs::write` applies the umask (0644 on most systems) and can only be
/// narrowed afterwards, so the key is world-readable for the length of a
/// `chmod` — and stays that way if the `chmod` fails. `create_new` also means
/// a second process racing for the same path loses instead of overwriting a
/// key that already protects stored credentials.
#[cfg(desktop)]
fn create_owner_only_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| format!("create {}: {err}", path.display()))?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|err| format!("write {}: {err}", path.display()))
}

/// Owner-only permissions for a fallback key file an older build left behind.
/// New files get their mode at creation time instead.
#[cfg(desktop)]
fn restrict_fallback_key_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(
                path = %path.display(),
                %err,
                "could not restrict fallback key permissions to owner-only"
            );
        }
    }
}

fn decode_key_material(encoded: &str) -> Result<[u8; 32], String> {
    let bytes = STANDARD.decode(encoded).map_err(|err| err.to_string())?;
    if bytes.len() != 32 {
        return Err("invalid master key length".to_string());
    }
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

#[cfg(all(test, desktop))]
mod plaintext_fallback_tests {
    use super::*;

    #[test]
    fn plaintext_fallback_requires_explicit_opt_in() {
        std::env::remove_var("SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK");
        assert!(!plaintext_fallback_allowed(), "unset must deny");

        for denied in ["", "0", "false", "no", "off"] {
            std::env::set_var("SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK", denied);
            assert!(!plaintext_fallback_allowed(), "`{denied}` must deny");
        }
        for allowed in ["1", "true", "yes"] {
            std::env::set_var("SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK", allowed);
            assert!(plaintext_fallback_allowed(), "`{allowed}` must allow");
        }
        std::env::remove_var("SATCHEL_ALLOW_PLAINTEXT_KEY_FALLBACK");
    }
}

#[cfg(all(test, desktop, unix))]
mod fallback_key_file_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn fallback_key_file_is_created_with_0600() {
        let profile = tempfile::tempdir().expect("temp profile");
        create_file_master_key(profile.path()).expect("fallback key");

        let mode = std::fs::metadata(master_key_file_path(profile.path()))
            .expect("fallback key file")
            .permissions()
            .mode();

        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    #[test]
    fn an_existing_fallback_key_is_never_overwritten() {
        let profile = tempfile::tempdir().expect("temp profile");
        let first = create_file_master_key(profile.path()).expect("fallback key");

        let error = create_file_master_key(profile.path())
            .expect_err("a second create must not clobber the key");
        assert!(error.contains("create"), "{error}");

        let reloaded = load_existing_file_master_key(profile.path())
            .expect("readable")
            .expect("present");
        assert_eq!(reloaded, first);
    }

    #[test]
    fn a_legacy_world_readable_key_is_narrowed_on_load() {
        let profile = tempfile::tempdir().expect("temp profile");
        let path = master_key_file_path(profile.path());
        std::fs::write(&path, STANDARD.encode([3_u8; 32])).expect("legacy key file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("legacy permissions");

        load_existing_file_master_key(profile.path())
            .expect("readable")
            .expect("present");

        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }
}

#[cfg(test)]
mod wrapped_export_tests {
    use super::*;

    fn write_legacy_key(dir: &Path, key: &[u8; 32]) {
        std::fs::write(
            dir.join(crate::db::MASTER_KEY_FILES[0]),
            STANDARD.encode(key),
        )
        .expect("legacy key file");
    }

    #[test]
    fn new_exports_require_twelve_unicode_characters() {
        for short in ["x", "elevenchars", "😀😀😀😀😀😀"] {
            assert_eq!(
                wrap_master_key(&[42; 32], short).unwrap_err(),
                BACKUP_PASSPHRASE_TOO_SHORT
            );
        }
        assert!(validate_backup_passphrase("😀😀😀😀😀😀😀😀😀😀😀😀").is_ok());
    }

    #[test]
    fn excessive_or_invalid_costs_are_rejected_before_key_derivation() {
        for (m, t, p) in [
            (u32::MAX, 2, 1),
            (19 * 1024, u32::MAX, 1),
            (19 * 1024, 2, u32::MAX),
            (0, 2, 1),
            (19 * 1024, 0, 1),
        ] {
            let value = format!(
                "wrapped:v1:{m}:{t}:{p}:{}:enc:v2:unused",
                STANDARD.encode([1; 16])
            );
            assert_eq!(
                unwrap_master_key(&value, Some("x")).unwrap_err(),
                "invalid key derivation parameters"
            );
        }
    }

    #[test]
    fn old_costs_and_short_legacy_passphrases_remain_readable() {
        let key = [42; 32];
        let salt = [1; 16];
        let wrapping = derive_wrapping_key("x", &salt, 19 * 1024, 2, 1).unwrap();
        let sealed = encrypt_aead_with_key(
            &STANDARD.encode(key),
            &wrapping,
            SecretContext::wrapped_master_key(),
        )
        .unwrap();
        let wrapped = format!("wrapped:v1:19456:2:1:{}:{sealed}", STANDARD.encode(salt));
        assert_eq!(unwrap_master_key(&wrapped, Some("x")).unwrap(), key);
    }

    #[test]
    fn keychain_only_profile_exports_the_active_cached_key() {
        seed_master_key_for_tests();
        let profile = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        assert_eq!(
            export_master_key(
                profile.path(),
                destination.path(),
                Some("backup passphrase")
            )
            .unwrap(),
            1
        );
        assert_eq!(
            backup_master_keys(destination.path(), Some("backup passphrase")).unwrap(),
            vec![[7; 32]]
        );
        assert!(!profile.path().join(crate::db::MASTER_KEY_FILES[0]).exists());
    }

    #[test]
    fn wrapped_master_key_survives_export_and_import() {
        let key = [42_u8; 32];
        let wrapped = wrap_master_key(&key, "correct horse").expect("wrap");

        assert_eq!(
            unwrap_master_key(&wrapped, Some("correct horse")).unwrap(),
            key
        );
    }

    #[test]
    fn a_wrapped_export_holds_no_key_bytes() {
        let key = [42_u8; 32];
        let wrapped = wrap_master_key(&key, "correct horse").expect("wrap");

        assert!(!wrapped.contains(&STANDARD.encode(key)));
        assert!(!wrapped.contains(STANDARD.encode(key).trim_end_matches('=')));
    }

    #[test]
    fn a_wrong_passphrase_is_rejected_with_its_own_key() {
        let wrapped = wrap_master_key(&[42_u8; 32], "correct horse").expect("wrap");

        assert_eq!(
            unwrap_master_key(&wrapped, Some("wrong")).unwrap_err(),
            BACKUP_PASSPHRASE_REJECTED
        );
        assert_eq!(
            unwrap_master_key(&wrapped, None).unwrap_err(),
            BACKUP_PASSPHRASE_REQUIRED
        );
    }

    #[test]
    fn legacy_plain_key_file_is_still_importable() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = [9_u8; 32];
        write_legacy_key(dir.path(), &key);

        assert!(!backup_master_key_is_wrapped(dir.path()));
        assert_eq!(backup_master_keys(dir.path(), None).unwrap(), vec![key]);
    }

    #[test]
    fn a_wrapped_backup_needs_its_passphrase_before_anything_else() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = [9_u8; 32];
        std::fs::write(
            dir.path().join(crate::db::MASTER_KEY_FILES[0]),
            wrap_master_key(&key, "backup passphrase").expect("wrap"),
        )
        .expect("wrapped key file");

        assert!(backup_master_key_is_wrapped(dir.path()));
        assert_eq!(
            backup_master_keys(dir.path(), None).unwrap_err(),
            BACKUP_PASSPHRASE_REQUIRED
        );
        assert_eq!(
            backup_master_keys(dir.path(), Some("backup passphrase")).unwrap(),
            vec![key]
        );
    }
}

#[cfg(all(test, desktop))]
mod desktop_store_tests {
    #[cfg(unix)]
    #[test]
    fn profile_accounts_match_the_macos_script_vectors() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/profile-keyring-accounts.json"
        ))
        .unwrap();
        for vector in vectors.as_array().unwrap() {
            assert_eq!(
                super::profile_keyring_account(std::path::Path::new(
                    vector["path"].as_str().unwrap()
                )),
                vector["account"].as_str().unwrap()
            );
        }
    }

    #[test]
    fn automatic_backup_reads_its_snapshot_without_changing_the_active_key() {
        let backup = tempfile::tempdir().unwrap();
        let account = super::profile_keyring_account(backup.path());
        std::fs::write(backup.path().join(super::LOCAL_BACKUP_KEY_MARKER), &account).unwrap();
        assert_eq!(super::backup_master_key_count(backup.path()), 1);
        assert_eq!(
            super::local_backup_master_key(backup.path(), |requested| {
                assert_eq!(requested, account);
                Ok(Some([42; 32]))
            })
            .unwrap(),
            Some([42; 32])
        );
        assert!(super::local_backup_master_key(backup.path(), |_| Ok(None)).is_err());
        assert!(super::local_backup_master_key(backup.path(), |_| Err("locked".into())).is_err());
    }

    #[test]
    fn pruning_only_removes_the_backup_directories_own_key() {
        let backup = tempfile::tempdir().unwrap();
        let marker = backup.path().join(super::LOCAL_BACKUP_KEY_MARKER);
        let account = super::profile_keyring_account(backup.path());
        std::fs::write(&marker, &account).unwrap();
        super::remove_local_backup_master_key_with(backup.path(), |requested| {
            assert_eq!(requested, account);
            Ok(())
        })
        .unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(&marker, super::profile_keyring_account(other.path())).unwrap();
        assert!(
            super::remove_local_backup_master_key_with(backup.path(), |_| panic!(
                "must not delete another profile key"
            ))
            .is_err()
        );
    }

    #[test]
    fn automatic_backup_rejects_an_arbitrary_keychain_account() {
        let backup = tempfile::tempdir().unwrap();
        std::fs::write(
            backup.path().join(super::LOCAL_BACKUP_KEY_MARKER),
            "unrelated-account",
        )
        .unwrap();
        assert!(super::local_backup_master_key(backup.path(), |_| panic!(
            "must not access keychain"
        ))
        .is_err());
    }

    #[test]
    fn credential_tests_never_use_the_production_service() {
        assert_ne!(super::KEYRING_SERVICE, "satchel");
    }

    #[test]
    fn desktop_default_store_is_not_mock() {
        let entry = keyring::Entry::new("satchel-test-backend", "backend-selection").unwrap();
        assert!(!entry.get_credential().is::<keyring::mock::MockCredential>());
    }
}

#[cfg(all(test, desktop))]
mod desktop_migration_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn profiles_migrate_independent_file_keys_without_overwriting_each_other() {
        let directory = tempfile::tempdir().unwrap();
        let store = std::cell::RefCell::new(std::collections::HashMap::new());
        for (name, key) in [("stable", [1; 32]), ("daily", [2; 32])] {
            let profile = directory.path().join(name);
            std::fs::create_dir(&profile).unwrap();
            std::fs::write(master_key_file_path(&profile), STANDARD.encode(key)).unwrap();
            let account = profile_keyring_account(&profile);
            assert_eq!(
                load_desktop_master_key(
                    &profile,
                    || Ok(store.borrow().get(&account).copied()),
                    |key| {
                        store.borrow_mut().insert(account.clone(), *key);
                        Ok(())
                    },
                    || panic!("file migration must not replace the legacy shared key"),
                    false
                )
                .unwrap(),
                key
            );
        }
        assert_eq!(store.borrow().len(), 2);
        for (name, key) in [("stable", [1; 32]), ("daily", [2; 32])] {
            let profile = directory.path().join(name);
            let account = profile_keyring_account(&profile);
            assert_eq!(
                load_desktop_master_key(
                    &profile,
                    || Ok(store.borrow().get(&account).copied()),
                    |_| panic!("existing profile key must not be replaced"),
                    || panic!("existing profile needs no legacy key"),
                    false
                )
                .unwrap(),
                key
            );
            assert!(!master_key_file_path(&profile).exists());
        }
    }

    #[test]
    fn legacy_keychain_only_profile_is_copied_into_the_profile_account() {
        let profile = tempfile::tempdir().unwrap();
        let stored = Cell::new(None);
        assert_eq!(
            load_desktop_master_key(
                profile.path(),
                || Ok(stored.get()),
                |key| {
                    stored.set(Some(*key));
                    Ok(())
                },
                || Ok(Some([3; 32])),
                false
            )
            .unwrap(),
            [3; 32]
        );
        assert_eq!(stored.get(), Some([3; 32]));
        assert!(!master_key_file_path(profile.path()).exists());
    }

    #[test]
    fn missing_entry_mints_and_verifies_a_key_without_a_file() {
        let profile = tempfile::tempdir().unwrap();
        let stored = Cell::new(None);
        let key = load_desktop_master_key(
            profile.path(),
            || Ok(stored.get()),
            |key| {
                stored.set(Some(*key));
                Ok(())
            },
            || Ok(None),
            false,
        )
        .unwrap();
        assert_eq!(stored.get(), Some(key));
        assert!(!master_key_file_path(profile.path()).exists());
    }

    #[test]
    fn file_is_removed_only_after_a_matching_read_back() {
        let profile = tempfile::tempdir().unwrap();
        let key = create_file_master_key(profile.path()).unwrap();
        let stored = Cell::new(None);
        assert_eq!(
            load_desktop_master_key(
                profile.path(),
                || {
                    assert!(master_key_file_path(profile.path()).exists());
                    Ok(stored.get())
                },
                |value| {
                    stored.set(Some(*value));
                    Ok(())
                },
                || Ok(None),
                false
            )
            .unwrap(),
            key
        );
        assert!(!master_key_file_path(profile.path()).exists());
    }

    #[test]
    fn write_failure_read_failure_and_mismatch_keep_the_file_key() {
        for (read, write) in [
            (Ok(Some([5; 32])), Err("locked".to_string())),
            (Err("locked".to_string()), Ok(())),
            (Ok(Some([5; 32])), Ok(())),
            (Ok(None), Ok(())),
        ] {
            let profile = tempfile::tempdir().unwrap();
            let key = create_file_master_key(profile.path()).unwrap();
            assert_eq!(
                load_desktop_master_key(
                    profile.path(),
                    || read.clone(),
                    |_| write.clone(),
                    || Ok(None),
                    false
                )
                .unwrap(),
                key
            );
            assert_eq!(
                load_existing_file_master_key(profile.path()).unwrap(),
                Some(key)
            );
        }
    }

    #[test]
    fn fresh_install_with_unreachable_store_fails_closed() {
        let profile = tempfile::tempdir().unwrap();
        let result = load_desktop_master_key(
            profile.path(),
            || Err("locked".into()),
            |_| panic!("no write on inaccessible store"),
            || Ok(None),
            false,
        );
        assert!(result.unwrap_err().contains("refusing to write"));
        assert!(!master_key_file_path(profile.path()).exists());
    }

    #[test]
    fn failed_fresh_write_requires_explicit_file_opt_in() {
        for allowed in [false, true] {
            let profile = tempfile::tempdir().unwrap();
            let result = load_desktop_master_key(
                profile.path(),
                || Ok(None),
                |_| Err("write denied".into()),
                || Ok(None),
                allowed,
            );
            assert_eq!(result.is_ok(), allowed);
            assert_eq!(master_key_file_path(profile.path()).exists(), allowed);
        }
    }
}
