//! Persistence for expired-certificate exceptions (`tls_certificate_exceptions`).
//!
//! The policy lives in [`crate::providers::tls_trust`]; this module only reads
//! and writes rows.

use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct TlsExceptionRecord {
    pub account_id: String,
    pub host: String,
    /// Sign-in name of the account; only its connections use the exception.
    pub login: String,
    /// SHA-256 of the DER certificate, upper-case hex pairs joined by `:`.
    pub fingerprint: String,
    /// RFC 3339.
    pub expires_at: String,
    /// RFC 3339.
    pub created_at: String,
    /// RFC 3339; when the pinned certificate was last accepted.
    pub last_used_at: Option<String>,
}

const COLUMNS: &str = "account_id, host, login, fingerprint, expires_at, created_at, last_used_at";

pub struct TlsExceptionRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TlsExceptionRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<TlsExceptionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TlsExceptionRecord>(&format!(
            "SELECT {COLUMNS} FROM tls_certificate_exceptions ORDER BY created_at"
        ))
        .fetch_all(self.pool)
        .await
    }

    /// Replaces an earlier exception for the same account and host: there is
    /// only ever one certificate a server should be presenting.
    pub async fn upsert(&self, record: &TlsExceptionRecord) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO tls_certificate_exceptions
                 (account_id, host, login, fingerprint, expires_at, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, host) DO UPDATE SET
                 login = excluded.login,
                 fingerprint = excluded.fingerprint,
                 expires_at = excluded.expires_at,
                 created_at = excluded.created_at,
                 last_used_at = excluded.last_used_at",
        )
        .bind(&record.account_id)
        .bind(&record.host)
        .bind(&record.login)
        .bind(&record.fingerprint)
        .bind(&record.expires_at)
        .bind(&record.created_at)
        .bind(&record.last_used_at)
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

    /// Notes that the pinned certificate of `host` was accepted at `at`.
    pub async fn mark_used(&self, host: &str, at: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tls_certificate_exceptions SET last_used_at = ?
             WHERE host = ? COLLATE NOCASE",
        )
        .bind(at)
        .bind(host)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// The exceptions for a host whose pinned certificate has not been needed
    /// since `quiet_since` (RFC 3339 UTC): the host presents valid ones now.
    pub async fn delete_quiet_host(
        &self,
        host: &str,
        quiet_since: &str,
    ) -> Result<Vec<TlsExceptionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TlsExceptionRecord>(&format!(
            "DELETE FROM tls_certificate_exceptions
             WHERE host = ? COLLATE NOCASE
               AND COALESCE(last_used_at, created_at) <= ?
             RETURNING {COLUMNS}"
        ))
        .bind(host)
        .bind(quiet_since)
        .fetch_all(self.pool)
        .await
    }

    /// Rows whose `expires_at` lies before `now` (both RFC 3339 UTC, so the
    /// string order is the time order).
    pub async fn delete_expired(&self, now: &str) -> Result<Vec<TlsExceptionRecord>, sqlx::Error> {
        sqlx::query_as::<_, TlsExceptionRecord>(&format!(
            "DELETE FROM tls_certificate_exceptions
             WHERE expires_at <= ?
             RETURNING {COLUMNS}"
        ))
        .bind(now)
        .fetch_all(self.pool)
        .await
    }
}
