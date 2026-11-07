//! Autocrypt peer state, per own account and peer address.
//!
//! The rules are `crate::autocrypt`'s; this module only loads and stores the
//! state they work on.
use chrono::{DateTime, Utc};
use sqlx::{Sqlite, SqlitePool};

use crate::autocrypt::{self, AutocryptHeader, PeerKey, PeerState, PreferEncrypt};

#[derive(Debug, sqlx::FromRow)]
struct PeerRow {
    last_seen: String,
    autocrypt_timestamp: Option<String>,
    public_key: Option<String>,
    fingerprint: Option<String>,
    prefer_encrypt: String,
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

impl PeerRow {
    fn into_state(self) -> Option<PeerState> {
        Some(PeerState {
            last_seen: parse_time(&self.last_seen)?,
            autocrypt_timestamp: self.autocrypt_timestamp.as_deref().and_then(parse_time),
            public_key: self.public_key,
            fingerprint: self.fingerprint,
            prefer_encrypt: PreferEncrypt::parse(&self.prefer_encrypt),
        })
    }
}

pub async fn load_peer<'e, E>(
    executor: E,
    account_id: &str,
    addr: &str,
) -> Result<Option<PeerState>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let row: Option<PeerRow> = sqlx::query_as(
        "SELECT last_seen, autocrypt_timestamp, public_key, fingerprint, prefer_encrypt
         FROM autocrypt_peers WHERE account_id = ? AND addr = ?",
    )
    .bind(account_id)
    .bind(autocrypt::canonical_addr(addr))
    .fetch_optional(executor)
    .await?;
    Ok(row.and_then(PeerRow::into_state))
}

async fn store_peer(
    executor: &mut sqlx::SqliteConnection,
    account_id: &str,
    addr: &str,
    state: &PeerState,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO autocrypt_peers
            (account_id, addr, last_seen, autocrypt_timestamp, public_key, fingerprint, prefer_encrypt, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(account_id, addr) DO UPDATE SET
            last_seen = excluded.last_seen,
            autocrypt_timestamp = excluded.autocrypt_timestamp,
            public_key = excluded.public_key,
            fingerprint = excluded.fingerprint,
            prefer_encrypt = excluded.prefer_encrypt,
            updated_at = excluded.updated_at",
    )
    .bind(account_id)
    .bind(autocrypt::canonical_addr(addr))
    .bind(state.last_seen.to_rfc3339())
    .bind(state.autocrypt_timestamp.map(|time| time.to_rfc3339()))
    .bind(state.public_key.as_deref())
    .bind(state.fingerprint.as_deref())
    .bind(state.prefer_encrypt.as_str())
    .execute(executor)
    .await?;
    Ok(())
}

/// Turn a header's key data into a key Satchel may encrypt to. A header whose
/// key does not parse, verify or encrypt is treated as no header at all.
pub fn checked_key(header: &AutocryptHeader) -> Option<PeerKey> {
    let (armored, metadata) =
        crate::utils::pgp_engine::certificate_from_keydata(&header.keydata).ok()?;
    Some(PeerKey {
        armored,
        fingerprint: metadata.fingerprint,
        prefer_encrypt: header.prefer_encrypt,
    })
}

/// Apply one incoming message to its sender's state ("Updating Autocrypt Peer
/// State"). `header` is the message's one valid header for the sender, if any.
pub async fn apply_incoming(
    executor: &mut sqlx::SqliteConnection,
    account_id: &str,
    from_addr: &str,
    effective: DateTime<Utc>,
    header: Option<&AutocryptHeader>,
) -> Result<(), sqlx::Error> {
    let key = header.and_then(checked_key);
    let current = load_peer(&mut *executor, account_id, from_addr).await?;
    let unchanged = current.clone();
    if let Some(next) = autocrypt::update_peer(current, effective, key) {
        if Some(&next) != unchanged.as_ref() {
            store_peer(executor, account_id, from_addr, &next).await?;
        }
    }
    Ok(())
}

pub struct AutocryptPeerRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AutocryptPeerRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(
        &self,
        account_id: &str,
        addr: &str,
    ) -> Result<Option<PeerState>, sqlx::Error> {
        load_peer(self.pool, account_id, addr).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
             VALUES ('acc', 'Me', 'me@example.test', 'imap', ?, '', '', '')",
        )
        .bind(crate::accounts::encrypt_account_config("acc", "{}").unwrap())
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn generated_keydata(addr: &str) -> Vec<u8> {
        let pair = crate::utils::pgp_engine::generate_keypair(
            &crate::utils::pgp_engine::GenerateKeyRequest {
                uid: &format!("Lena <{addr}>"),
                algorithm: "ed25519",
                expires_in: "never",
                passphrase: None,
            },
        )
        .unwrap();
        crate::utils::pgp_engine::autocrypt_keydata(&pair.private_armored, addr).unwrap()
    }

    #[tokio::test]
    async fn stores_a_valid_key_and_ignores_garbage() {
        let pool = pool().await;
        let when = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
        let mut conn = pool.acquire().await.unwrap();

        let garbage = AutocryptHeader {
            addr: "lena@example.test".into(),
            prefer_encrypt: PreferEncrypt::Mutual,
            keydata: vec![1, 2, 3],
        };
        apply_incoming(&mut conn, "acc", "lena@example.test", when, Some(&garbage))
            .await
            .unwrap();
        assert!(load_peer(&mut *conn, "acc", "lena@example.test")
            .await
            .unwrap()
            .is_none());

        let header = AutocryptHeader {
            addr: "lena@example.test".into(),
            prefer_encrypt: PreferEncrypt::Mutual,
            keydata: generated_keydata("lena@example.test"),
        };
        apply_incoming(&mut conn, "acc", "Lena@Example.test", when, Some(&header))
            .await
            .unwrap();
        let state = load_peer(&mut *conn, "acc", "lena@example.test")
            .await
            .unwrap()
            .unwrap();
        assert!(state
            .public_key
            .as_deref()
            .unwrap()
            .contains("PUBLIC KEY BLOCK"));
        assert_eq!(state.prefer_encrypt, PreferEncrypt::Mutual);
        assert_eq!(
            autocrypt::recommend(Some(&state), PreferEncrypt::Mutual, false),
            autocrypt::Recommendation::Encrypt
        );

        // A later mail without a header keeps the key and moves last_seen.
        let later = when + chrono::Duration::days(3);
        apply_incoming(&mut conn, "acc", "lena@example.test", later, None)
            .await
            .unwrap();
        drop(conn);
        let state = AutocryptPeerRepository::new(&pool)
            .get("acc", "lena@example.test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.last_seen, later);
        assert!(state.public_key.is_some());
    }

    #[test]
    fn keydata_keeps_only_the_packets_autocrypt_names() {
        let keydata = generated_keydata("lena@example.test");
        let (armored, metadata) =
            crate::utils::pgp_engine::certificate_from_keydata(&keydata).unwrap();
        assert!(armored.contains("PUBLIC KEY BLOCK"));
        assert_eq!(
            metadata.user_ids,
            vec!["Lena <lena@example.test>".to_string()]
        );
        assert!(metadata.can_encrypt);
        assert!(!metadata.has_secret_key);
        // No user id for another address: no key data.
        let pair = crate::utils::pgp_engine::generate_keypair(
            &crate::utils::pgp_engine::GenerateKeyRequest {
                uid: "Lena <lena@example.test>",
                algorithm: "ed25519",
                expires_in: "never",
                passphrase: None,
            },
        )
        .unwrap();
        assert!(crate::utils::pgp_engine::autocrypt_keydata(
            &pair.private_armored,
            "x@example.test"
        )
        .is_err());
    }
}
