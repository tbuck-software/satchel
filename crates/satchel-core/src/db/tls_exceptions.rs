//! Persistence for expired-certificate exceptions (`tls_certificate_exceptions`).
//!
//! The policy lives in [`crate::providers::tls_trust`]; this module only reads
//! and writes rows.

use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TlsExceptionRecord {
    pub account_id: String,
    pub host: String,
    /// SHA-256 of the DER certificate, upper-case hex pairs joined by `:`.
    pub fingerprint: String,
    /// RFC 3339.
    pub expires_at: String,
    /// RFC 3339.
    pub created_at: String,
}

pub struct TlsExceptionRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TlsExceptionRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<TlsExceptionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TlsExceptionRecord>(
            "SELECT account_id, host, fingerprint, expires_at, created_at
             FROM tls_certificate_exceptions
             ORDER BY created_at",
        )
        .fetch_all(self.pool)
        .await
    }

    /// Replaces an earlier exception for the same account and host: there is
    /// only ever one certificate a server should be presenting.
    pub async fn upsert(&self, record: &TlsExceptionRecord) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO tls_certificate_exceptions
                 (account_id, host, fingerprint, expires_at, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(account_id, host) DO UPDATE SET
                 fingerprint = excluded.fingerprint,
                 expires_at = excluded.expires_at,
                 created_at = excluded.created_at",
        )
        .bind(&record.account_id)
        .bind(&record.host)
        .bind(&record.fingerprint)
        .bind(&record.expires_at)
        .bind(&record.created_at)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete(&self, account_id: &str, host: &str) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "DELETE FROM tls_certificate_exceptions
             WHERE account_id = ? AND host = ? COLLATE NOCASE",
        )
        .bind(account_id)
        .bind(host)
        .execute(self.pool)
        .await?
        .rows_affected())
    }

    /// Every exception for a host, whichever account granted it: once the
    /// server presents a valid certificate, none of them is needed.
    pub async fn delete_host(&self, host: &str) -> Result<Vec<TlsExceptionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TlsExceptionRecord>(
            "DELETE FROM tls_certificate_exceptions
             WHERE host = ? COLLATE NOCASE
             RETURNING account_id, host, fingerprint, expires_at, created_at",
        )
        .bind(host)
        .fetch_all(self.pool)
        .await
    }

    /// Rows whose `expires_at` lies before `now` (both RFC 3339 UTC, so the
    /// string order is the time order).
    pub async fn delete_expired(&self, now: &str) -> Result<Vec<TlsExceptionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TlsExceptionRecord>(
            "DELETE FROM tls_certificate_exceptions
             WHERE expires_at <= ?
             RETURNING account_id, host, fingerprint, expires_at, created_at",
        )
        .bind(now)
        .fetch_all(self.pool)
        .await
    }
}
