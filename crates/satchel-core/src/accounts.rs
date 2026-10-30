//! Account credentials at rest.
//!
//! Lives in the core crate because the sync, provider and OAuth paths all
//! read `accounts.config` and must not reach back into the Tauri layer for
//! the accessor. `commands::accounts` re-exports it under its old name.

use sqlx::SqlitePool;

// ---------------------------------------------------------------------------
// Account credentials at rest
// ---------------------------------------------------------------------------
//
// `accounts.config` holds IMAP/SMTP passwords and OAuth refresh tokens. It used
// to sit in the database as plaintext JSON, readable by any process running as
// the user. It is now encrypted with the AES-GCM master key from the system
// keychain.
//
// Everything that reads the column MUST go through `decrypt_account_config`,
// and everything that writes it through `encrypt_account_config`. There is no
// second path — `accounts.credential-choke-point.test.ts` fails the build if a
// new raw read appears.

/// A decrypted `accounts.config` JSON.
///
/// Its own type because a stored config and a decrypted one are both strings,
/// and helpers that accepted either decrypted "just in case" — which is how a
/// plaintext row planted by another process reached a provider. Only
/// [`decrypt_account_config`] and [`AccountConfig::from_plaintext`] make one,
/// so a raw column value handed to a provider does not compile.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct AccountConfig(String);

impl AccountConfig {
    /// A config built in memory (onboarding, tests), never one read from the column.
    pub fn from_plaintext(json: impl Into<String>) -> Self {
        Self(json.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::ops::Deref for AccountConfig {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

/// Holds passwords and tokens; never printed.
impl std::fmt::Debug for AccountConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AccountConfig(..)")
    }
}

/// Message key for a config row that is not Satchel's ciphertext.
pub const ACCOUNT_CONFIG_NOT_ENCRYPTED: &str = "backend.errors.accountConfigNotEncrypted";

/// Decrypt the stored `accounts.config` of `account_id`.
///
/// A value without the ciphertext marker is refused: it was not written by
/// Satchel. An `enc:v2` value only opens for the account it was written for.
pub fn decrypt_account_config(account_id: &str, raw: &str) -> Result<AccountConfig, String> {
    if !crate::utils::crypto::is_encrypted(raw) {
        return Err(ACCOUNT_CONFIG_NOT_ENCRYPTED.to_string());
    }
    crate::utils::crypto::decrypt_secret(
        raw,
        crate::utils::crypto::SecretContext::account_config(account_id),
    )
    .map(AccountConfig)
}

/// Encrypt a config JSON for storage in the row of `account_id`.
///
/// Fails when no key is reachable. It used to fall back to plaintext so that an
/// unreachable keychain could not block adding or refreshing an account — but
/// that let an installation slide from encrypted to plaintext mid-session,
/// filling the database with IMAP passwords and refresh tokens, visible only as
/// a log line. A refused write is recoverable; a leaked credential is not.
pub fn encrypt_account_config(account_id: &str, config_json: &str) -> Result<String, String> {
    crate::utils::crypto::encrypt_secret(
        config_json,
        crate::utils::crypto::SecretContext::account_config(account_id),
    )
}

/// `(total, encrypted)` account rows. Counts only; nothing is decrypted.
pub async fn count_account_config_encryption(pool: &SqlitePool) -> Result<(i64, i64), String> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(pool)
        .await
        .map_err(|err| err.to_string())?;
    let encrypted: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM accounts WHERE config LIKE 'enc:v1:%' OR config LIKE 'enc:v2:%'",
    )
    .fetch_one(pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok((total, encrypted))
}

/// Report account rows whose config is not Satchel's ciphertext.
///
/// This used to encrypt them. That was the right move while rows written before
/// the July 2026 startup migration were still around, and the wrong one
/// afterwards: a plaintext row can no longer be told apart from one a process
/// planted, and encrypting it made the app the notary of somebody else's
/// configuration. Every install that has started since then is encrypted, so
/// the remainder is reported and refused rather than adopted.
///
/// Returns how many rows are unusable; `credentials_status` counts the same
/// rows for the banner.
pub async fn report_plaintext_account_configs(pool: &SqlitePool) -> Result<u64, String> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT id, config FROM accounts")
        .fetch_all(pool)
        .await
        .map_err(|err| err.to_string())?;

    let mut unusable = 0_u64;
    for (id, config) in rows {
        if crate::utils::crypto::is_encrypted(&config) {
            continue;
        }
        tracing::error!(
            account_id = %id,
            "account credentials are not encrypted; the account stays locked until it is set up again"
        );
        unusable += 1;
    }

    Ok(unusable)
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    const IMAP_CONFIG: &str = r#"{"imap":{"host":"mail.example.test","port":993,"tls":true,"username":"me@example.test","password":"hunter2"}}"#;
    const OAUTH_CONFIG: &str = r#"{"oauth":{"access_token":"at-123","refresh_token":"rt-456","expires_at":"2020-01-01T00:00:00Z"}}"#;

    #[test]
    fn encryption_round_trips_byte_for_byte() {
        crate::utils::crypto::seed_master_key_for_tests();
        let encrypted = encrypt_account_config("acc-a", IMAP_CONFIG).unwrap();
        assert!(encrypted.starts_with("enc:v2:"));
        assert!(
            !encrypted.contains("hunter2"),
            "password must not be readable"
        );
        assert_eq!(
            decrypt_account_config("acc-a", &encrypted)
                .unwrap()
                .as_str(),
            IMAP_CONFIG
        );
    }

    /// SEC-11: a ciphertext copied onto another account's row is refused.
    #[test]
    fn a_config_only_opens_for_the_account_it_was_written_for() {
        crate::utils::crypto::seed_master_key_for_tests();
        let for_a = encrypt_account_config("acc-a", IMAP_CONFIG).unwrap();

        assert!(decrypt_account_config("acc-b", &for_a).is_err());
        assert!(decrypt_account_config("acc-a", &for_a).is_ok());
    }

    /// Written before SEC-11: still readable, so nobody loses credentials.
    #[test]
    fn an_unbound_legacy_ciphertext_still_opens() {
        crate::utils::crypto::seed_master_key_for_tests();
        let legacy = legacy_ciphertext(IMAP_CONFIG);

        assert_eq!(
            decrypt_account_config("any-account", &legacy)
                .unwrap()
                .as_str(),
            IMAP_CONFIG
        );
    }

    /// SEC-10: plaintext in the column is not a config, whoever put it there.
    #[test]
    fn a_plaintext_row_is_refused_with_its_own_key() {
        let planted = r#"{"imap":{"host":"attacker.example","password":"x"}}"#;

        assert_eq!(
            decrypt_account_config("acc-a", planted).unwrap_err(),
            ACCOUNT_CONFIG_NOT_ENCRYPTED
        );
    }

    #[test]
    fn providers_build_from_a_decrypted_config() {
        crate::utils::crypto::seed_master_key_for_tests();
        let imap = decrypt_account_config(
            "acc-a",
            &encrypt_account_config("acc-a", IMAP_CONFIG).unwrap(),
        )
        .unwrap();
        crate::providers::provider_for_account("imap", &imap, "me@example.test", None)
            .expect("an IMAP provider");
        let oauth = AccountConfig::from_plaintext(OAUTH_CONFIG);
        for kind in ["gmail", "outlook"] {
            crate::providers::provider_for_account(kind, &oauth, "me@example.test", None)
                .unwrap_or_else(|err| panic!("{kind}: {err}"));
        }
    }

    /// A detached config must never refresh credentials that it cannot persist.
    #[tokio::test]
    async fn oauth_token_lookup_refuses_to_refresh_a_detached_snapshot() {
        let error = crate::utils::oauth::access_token_from_account_config(
            crate::utils::oauth::OAuthProvider::Gmail,
            &AccountConfig::from_plaintext(OAUTH_CONFIG),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "[transient] oauth access token expired; reload account configuration"
        );
    }

    #[test]
    fn a_corrupt_ciphertext_fails_loudly_rather_than_silently() {
        crate::utils::crypto::seed_master_key_for_tests();
        assert!(decrypt_account_config("acc-a", "enc:v1:AAAA:BBBB").is_err());
        assert!(decrypt_account_config("acc-a", "enc:v2:AAAA:BBBB").is_err());
    }

    #[test]
    fn a_config_never_prints_its_secrets() {
        let config = AccountConfig::from_plaintext(IMAP_CONFIG);
        assert!(!format!("{config:?}").contains("hunter2"));
    }

    fn legacy_ciphertext(plain: &str) -> String {
        use aes_gcm::aead::{Aead, KeyInit};
        use base64::Engine as _;
        let cipher = aes_gcm::Aes256Gcm::new_from_slice(&[7_u8; 32]).unwrap();
        let nonce = [3_u8; 12];
        let sealed = cipher
            .encrypt(aes_gcm::Nonce::from_slice(&nonce), plain.as_bytes())
            .unwrap();
        let engine = base64::engine::general_purpose::STANDARD;
        format!("enc:v1:{}:{}", engine.encode(nonce), engine.encode(sealed))
    }

    /// The startup pass does not adopt a plaintext row; it reports it.
    #[tokio::test]
    async fn a_plaintext_row_is_reported_and_left_unusable() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        crate::db::init(&pool).await.expect("migrations");

        let planted = r#"{"imap":{"host":"attacker.example","password":"hunter2"}}"#;
        sqlx::query(
            "INSERT INTO accounts(id, display_name, email, provider, config, credentials_ref, is_active, created_at, updated_at)
             VALUES('a1', 'A', 'a@x.test', 'imap', ?, '', 1, '2026-01-01', '2026-01-01')",
        )
        .bind(planted)
        .execute(&pool)
        .await
        .expect("seed");
        crate::utils::crypto::seed_master_key_for_tests();

        assert_eq!(
            report_plaintext_account_configs(&pool).await.expect("pass"),
            1
        );
        let stored: String = sqlx::query_scalar("SELECT config FROM accounts WHERE id = 'a1'")
            .fetch_one(&pool)
            .await
            .expect("read back");
        assert_eq!(stored, planted, "the row must not be rewritten");
        assert_eq!(
            super::count_account_config_encryption(&pool)
                .await
                .expect("counts"),
            (1, 0)
        );
    }
}
