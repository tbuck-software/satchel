//! Durable OAuth authorization-flow state.
//!
//! The database is the sole authority for PKCE verifiers. Keeping a second
//! in-memory copy creates a race in which two callbacks can both spend the
//! same verifier even though the database row was consumed atomically.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

use crate::utils::{crypto, oauth::OAuthGrantRequest};

#[derive(Clone)]
pub struct NewOAuthFlowSession {
    pub state: String,
    pub account_id: String,
    pub provider: String,
    pub code_verifier: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub redirect_uri: String,
    pub authorization_context: Option<OAuthGrantRequest>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthFlowSession {
    pub account_id: String,
    pub provider: String,
    pub code_verifier: String,
    pub created_at: DateTime<Utc>,
    pub redirect_uri: Option<String>,
    pub authorization_context: Option<OAuthGrantRequest>,
}

/// How far an authorization flow got, as far as the database can tell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OAuthFlowProgress {
    /// The verifier is unspent: the user is still at the provider.
    Authorizing,
    /// The verifier was spent, so a token exchange started.
    Exchanging,
    /// The flow ended for this message key.
    Failed(String),
}

/// A flow seen without spending it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthFlowStatus {
    pub state: String,
    pub account_id: String,
    pub provider: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub progress: OAuthFlowProgress,
}

/// Outcome rows are kept this long past their session expiry so a poll after a
/// restart still finds an answer instead of "unknown flow".
const OUTCOME_RETENTION_HOURS: i64 = 24;

#[derive(FromRow)]
struct StoredOAuthFlowSession {
    account_id: String,
    provider: String,
    encrypted_code_verifier: String,
    created_at: String,
    expires_at: String,
    redirect_uri: Option<String>,
    authorization_context: Option<String>,
}

#[derive(FromRow)]
struct StoredOAuthFlowStatus {
    state: String,
    account_id: String,
    provider: String,
    created_at: String,
    expires_at: String,
    error_key: Option<String>,
}

#[derive(Debug, Error)]
pub enum OAuthFlowError {
    #[error("oauth flow was not found or has expired")]
    NotFoundOrExpired,
    #[error("oauth flow credentials could not be accessed")]
    CredentialAccess { detail: String },
    #[error("oauth flow data is invalid")]
    InvalidSession { detail: String },
    #[error("oauth flow persistence failed")]
    Persistence(#[source] sqlx::Error),
}

impl From<sqlx::Error> for OAuthFlowError {
    fn from(error: sqlx::Error) -> Self {
        Self::Persistence(error)
    }
}

pub struct OAuthFlowRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> OAuthFlowRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Prune expired flows and persist a fresh encrypted PKCE verifier as one
    /// transaction. A verifier is never written in plaintext.
    pub async fn create(&self, session: NewOAuthFlowSession) -> Result<(), OAuthFlowError> {
        let authorization_context = session
            .authorization_context
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| OAuthFlowError::InvalidSession {
                detail: error.to_string(),
            })?;
        let encrypted_code_verifier = crypto::encrypt_secret(
            &session.code_verifier,
            crypto::SecretContext::oauth_code_verifier(&session.state),
        )
        .map_err(|detail| OAuthFlowError::CredentialAccess { detail })?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query("DELETE FROM oauth_flow_sessions WHERE expires_at <= ?")
            .bind(session.created_at.to_rfc3339())
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM oauth_flow_outcomes WHERE expires_at <= ?")
            .bind(
                (session.created_at - chrono::Duration::hours(OUTCOME_RETENTION_HOURS))
                    .to_rfc3339(),
            )
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO oauth_flow_sessions
             (state, account_id, provider, encrypted_code_verifier, created_at, expires_at,
              redirect_uri, authorization_context)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.state)
        .bind(session.account_id)
        .bind(session.provider)
        .bind(encrypted_code_verifier)
        .bind(session.created_at.to_rfc3339())
        .bind(session.expires_at.to_rfc3339())
        .bind(session.redirect_uri)
        .bind(authorization_context)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Atomically consume one non-expired verifier.
    ///
    /// The row is deleted before parsing or decryption, so corrupt data cannot
    /// be retried and concurrent callbacks cannot both receive the secret. The
    /// outcome row is written in the same transaction: without it a crash
    /// during the token exchange would leave no trace of the flow at all.
    pub async fn consume(
        &self,
        state: &str,
        now: DateTime<Utc>,
    ) -> Result<OAuthFlowSession, OAuthFlowError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let stored = sqlx::query_as::<_, StoredOAuthFlowSession>(
            "DELETE FROM oauth_flow_sessions
             WHERE state = ? AND expires_at > ?
             RETURNING account_id, provider, encrypted_code_verifier, created_at, expires_at,
                       redirect_uri, authorization_context",
        )
        .bind(state)
        .bind(now.to_rfc3339())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(OAuthFlowError::NotFoundOrExpired)?;
        sqlx::query(
            "INSERT INTO oauth_flow_outcomes
             (state, account_id, provider, created_at, expires_at, error_key)
             VALUES (?, ?, ?, ?, ?, NULL)
             ON CONFLICT(state) DO NOTHING",
        )
        .bind(state)
        .bind(&stored.account_id)
        .bind(&stored.provider)
        .bind(&stored.created_at)
        .bind(&stored.expires_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        let created_at = DateTime::parse_from_rfc3339(&stored.created_at)
            .map_err(|error| OAuthFlowError::InvalidSession {
                detail: error.to_string(),
            })?
            .with_timezone(&Utc);
        if !crypto::is_encrypted(&stored.encrypted_code_verifier) {
            return Err(OAuthFlowError::InvalidSession {
                detail: "stored PKCE verifier is not encrypted".to_string(),
            });
        }
        let code_verifier = crypto::decrypt_secret(
            &stored.encrypted_code_verifier,
            crypto::SecretContext::oauth_code_verifier(state),
        )
        .map_err(|detail| OAuthFlowError::CredentialAccess { detail })?;

        Ok(OAuthFlowSession {
            account_id: stored.account_id,
            provider: stored.provider,
            code_verifier,
            created_at,
            redirect_uri: stored.redirect_uri,
            authorization_context: stored
                .authorization_context
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|error| OAuthFlowError::InvalidSession {
                    detail: error.to_string(),
                })?,
        })
    }

    /// End a flow for good and record why, as one message key.
    ///
    /// The verifier goes first so a later callback cannot revive an already
    /// failed flow; the key stays behind so a poll after a restart can say
    /// what happened instead of "unknown flow".
    pub async fn fail(&self, state: &str, error_key: &str) -> Result<(), OAuthFlowError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let removed = sqlx::query_as::<_, StoredOAuthFlowStatus>(
            "DELETE FROM oauth_flow_sessions
             WHERE state = ?
             RETURNING state, account_id, provider, created_at, expires_at, NULL AS error_key",
        )
        .bind(state)
        .fetch_optional(&mut *transaction)
        .await?;
        match removed {
            Some(session) => {
                sqlx::query(
                    "INSERT INTO oauth_flow_outcomes
                     (state, account_id, provider, created_at, expires_at, error_key)
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT(state) DO UPDATE SET error_key = excluded.error_key",
                )
                .bind(state)
                .bind(session.account_id)
                .bind(session.provider)
                .bind(session.created_at)
                .bind(session.expires_at)
                .bind(error_key)
                .execute(&mut *transaction)
                .await?;
            }
            // The verifier was already spent, so only the outcome row is left.
            // A recorded failure is never overwritten by a later one.
            None => {
                sqlx::query(
                    "UPDATE oauth_flow_outcomes SET error_key = ?
                     WHERE state = ? AND error_key IS NULL",
                )
                .bind(error_key)
                .bind(state)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Drop what is left of a flow that finished.
    ///
    /// From here on the account itself is the answer; a row left behind would
    /// make a completed sign-in look like one still worth resuming.
    pub async fn complete(&self, state: &str) -> Result<(), OAuthFlowError> {
        sqlx::query("DELETE FROM oauth_flow_outcomes WHERE state = ?")
            .bind(state)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Read a flow without spending it. `consume` stays the only way to the
    /// verifier, so this can be called on every poll.
    pub async fn status(
        &self,
        state: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<OAuthFlowStatus>, OAuthFlowError> {
        let live = sqlx::query_as::<_, StoredOAuthFlowStatus>(
            "SELECT state, account_id, provider, created_at, expires_at, NULL AS error_key
             FROM oauth_flow_sessions
             WHERE state = ? AND expires_at > ?",
        )
        .bind(state)
        .bind(now.to_rfc3339())
        .fetch_optional(self.pool)
        .await?;
        if let Some(live) = live {
            return decode_status(live, OAuthFlowProgress::Authorizing).map(Some);
        }

        let outcome = sqlx::query_as::<_, StoredOAuthFlowStatus>(
            "SELECT state, account_id, provider, created_at, expires_at, error_key
             FROM oauth_flow_outcomes
             WHERE state = ?",
        )
        .bind(state)
        .fetch_optional(self.pool)
        .await?;
        outcome
            .map(|outcome| {
                let progress = match outcome.error_key.clone() {
                    Some(key) => OAuthFlowProgress::Failed(key),
                    None => OAuthFlowProgress::Exchanging,
                };
                decode_status(outcome, progress)
            })
            .transpose()
    }

    /// Every flow a restarted app could still finish, newest first.
    pub async fn open_flows(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<OAuthFlowStatus>, OAuthFlowError> {
        let now = now.to_rfc3339();
        let mut flows = Vec::new();
        for row in sqlx::query_as::<_, StoredOAuthFlowStatus>(
            "SELECT state, account_id, provider, created_at, expires_at, NULL AS error_key
             FROM oauth_flow_sessions
             WHERE expires_at > ?
             ORDER BY created_at DESC",
        )
        .bind(&now)
        .fetch_all(self.pool)
        .await?
        {
            flows.push(decode_status(row, OAuthFlowProgress::Authorizing)?);
        }
        for row in sqlx::query_as::<_, StoredOAuthFlowStatus>(
            "SELECT state, account_id, provider, created_at, expires_at, error_key
             FROM oauth_flow_outcomes
             WHERE error_key IS NULL AND expires_at > ?
             ORDER BY created_at DESC",
        )
        .bind(&now)
        .fetch_all(self.pool)
        .await?
        {
            flows.push(decode_status(row, OAuthFlowProgress::Exchanging)?);
        }
        flows.sort_by_key(|flow| std::cmp::Reverse(flow.created_at));
        Ok(flows)
    }
}

fn decode_status(
    stored: StoredOAuthFlowStatus,
    progress: OAuthFlowProgress,
) -> Result<OAuthFlowStatus, OAuthFlowError> {
    Ok(OAuthFlowStatus {
        state: stored.state,
        account_id: stored.account_id,
        provider: stored.provider,
        created_at: parse_timestamp(&stored.created_at)?,
        expires_at: parse_timestamp(&stored.expires_at)?,
        progress,
    })
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, OAuthFlowError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| OAuthFlowError::InvalidSession {
            detail: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool(max_connections: u32) -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect("sqlite::memory:")
            .await
            .expect("memory db");
        crate::db::init(&pool).await.expect("migrations");
        pool
    }

    async fn account(pool: &SqlitePool) {
        sqlx::query(
            "INSERT INTO accounts
             (id, display_name, email, provider, config, credentials_ref, sync_state,
              color, is_active, created_at, updated_at)
             VALUES ('account', 'Account', '', 'gmail', '{}', '', '{}', '#000000', 1, ?, ?)",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    }

    fn new_session(now: DateTime<Utc>) -> NewOAuthFlowSession {
        NewOAuthFlowSession {
            state: "state".into(),
            account_id: "account".into(),
            provider: "gmail".into(),
            code_verifier: "pkce-secret".into(),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(15),
            redirect_uri: "http://127.0.0.1:1234/oauth/callback".into(),
            authorization_context: Some(OAuthGrantRequest {
                purpose: crate::utils::oauth::OAuthPurpose::Calendar,
                scopes: vec![
                    crate::utils::oauth::GOOGLE_MAIL_SCOPE.into(),
                    crate::utils::oauth::GOOGLE_CALENDAR_SCOPE.into(),
                ],
            }),
        }
    }

    #[tokio::test]
    async fn create_encrypts_verifier_and_survives_repository_restart() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();

        let stored: String = sqlx::query_scalar(
            "SELECT encrypted_code_verifier FROM oauth_flow_sessions WHERE state = 'state'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(stored.starts_with("enc:v2:"));
        assert!(!stored.contains("pkce-secret"));

        let restarted_repository = OAuthFlowRepository::new(&pool);
        let consumed = restarted_repository.consume("state", now).await.unwrap();
        assert_eq!(consumed.code_verifier, "pkce-secret");
        assert_eq!(
            consumed.authorization_context,
            new_session(now).authorization_context
        );
        assert_eq!(
            consumed.redirect_uri.as_deref(),
            Some("http://127.0.0.1:1234/oauth/callback")
        );
    }

    #[tokio::test]
    async fn create_prunes_expired_sessions_and_consume_rejects_expiry() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        let mut expired = new_session(now - chrono::Duration::hours(1));
        expired.state = "expired".into();
        OAuthFlowRepository::new(&pool)
            .create(expired)
            .await
            .unwrap();
        assert!(matches!(
            OAuthFlowRepository::new(&pool)
                .consume("expired", now)
                .await,
            Err(OAuthFlowError::NotFoundOrExpired)
        ));

        let mut fresh = new_session(now);
        fresh.state = "fresh".into();
        OAuthFlowRepository::new(&pool).create(fresh).await.unwrap();
        let expired_count: i64 =
            sqlx::query_scalar("SELECT COUNT(1) FROM oauth_flow_sessions WHERE state = 'expired'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(expired_count, 0);
    }

    #[tokio::test]
    async fn concurrent_callbacks_can_consume_a_verifier_only_once() {
        crate::utils::crypto::seed_master_key_for_tests();
        // A shared-cache URI is needed for multiple SQLite connections.
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite:file:oauth-consume?mode=memory&cache=shared")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();

        let left_repository = OAuthFlowRepository::new(&pool);
        let right_repository = OAuthFlowRepository::new(&pool);
        let (left, right) = tokio::join!(
            left_repository.consume("state", now),
            right_repository.consume("state", now),
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    }

    #[tokio::test]
    async fn corrupt_verifier_is_consumed_before_decryption_fails() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE oauth_flow_sessions SET encrypted_code_verifier = 'enc:v1:broken' WHERE state = 'state'",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(matches!(
            OAuthFlowRepository::new(&pool).consume("state", now).await,
            Err(OAuthFlowError::CredentialAccess { .. })
        ));
        assert!(matches!(
            OAuthFlowRepository::new(&pool).consume("state", now).await,
            Err(OAuthFlowError::NotFoundOrExpired)
        ));
    }

    #[tokio::test]
    async fn plaintext_verifier_is_rejected_and_consumed_without_being_returned() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE oauth_flow_sessions
             SET encrypted_code_verifier = 'legacy-plaintext-verifier'
             WHERE state = 'state'",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(matches!(
            OAuthFlowRepository::new(&pool).consume("state", now).await,
            Err(OAuthFlowError::InvalidSession { .. })
        ));
        assert!(matches!(
            OAuthFlowRepository::new(&pool).consume("state", now).await,
            Err(OAuthFlowError::NotFoundOrExpired)
        ));
    }
    #[tokio::test]
    async fn status_reads_a_live_session_without_spending_it() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();

        let status = OAuthFlowRepository::new(&pool)
            .status("state", now)
            .await
            .unwrap()
            .expect("open flow");
        assert_eq!(status.progress, OAuthFlowProgress::Authorizing);
        assert_eq!(status.account_id, "account");

        // The reading method must not have touched the verifier.
        let session = OAuthFlowRepository::new(&pool)
            .consume("state", now)
            .await
            .unwrap();
        assert_eq!(session.code_verifier, "pkce-secret");
    }

    #[tokio::test]
    async fn an_expired_session_is_no_longer_reported() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();

        let later = now + chrono::Duration::minutes(16);
        assert!(OAuthFlowRepository::new(&pool)
            .status("state", later)
            .await
            .unwrap()
            .is_none());
        assert!(OAuthFlowRepository::new(&pool)
            .open_flows(later)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_consumed_session_stays_visible_as_an_exchange_in_progress() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        OAuthFlowRepository::new(&pool)
            .create(new_session(now))
            .await
            .unwrap();
        OAuthFlowRepository::new(&pool)
            .consume("state", now)
            .await
            .unwrap();

        let status = OAuthFlowRepository::new(&pool)
            .status("state", now)
            .await
            .unwrap()
            .expect("outcome row");
        assert_eq!(status.progress, OAuthFlowProgress::Exchanging);
        assert_eq!(status.created_at, now);
    }

    #[tokio::test]
    async fn a_failure_key_outlives_the_verifier_in_both_orders() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        let repository = OAuthFlowRepository::new(&pool);

        repository.create(new_session(now)).await.unwrap();
        repository.fail("state", "backend.errors.a").await.unwrap();
        assert_eq!(
            repository
                .status("state", now)
                .await
                .unwrap()
                .unwrap()
                .progress,
            OAuthFlowProgress::Failed("backend.errors.a".into())
        );
        // The verifier is gone with it: a late callback cannot revive the flow.
        assert!(matches!(
            repository.consume("state", now).await,
            Err(OAuthFlowError::NotFoundOrExpired)
        ));

        let mut second = new_session(now);
        second.state = "second".into();
        repository.create(second).await.unwrap();
        repository.consume("second", now).await.unwrap();
        repository.fail("second", "backend.errors.b").await.unwrap();
        assert_eq!(
            repository
                .status("second", now)
                .await
                .unwrap()
                .unwrap()
                .progress,
            OAuthFlowProgress::Failed("backend.errors.b".into())
        );
    }

    #[tokio::test]
    async fn a_finished_flow_is_no_longer_offered_for_resuming() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        let repository = OAuthFlowRepository::new(&pool);
        repository.create(new_session(now)).await.unwrap();
        repository.consume("state", now).await.unwrap();
        repository.complete("state").await.unwrap();

        assert!(repository.status("state", now).await.unwrap().is_none());
        assert!(repository.open_flows(now).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn open_flows_lists_what_a_restart_could_still_finish() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        let repository = OAuthFlowRepository::new(&pool);

        repository.create(new_session(now)).await.unwrap();
        let mut exchanging = new_session(now + chrono::Duration::seconds(1));
        exchanging.state = "exchanging".into();
        repository.create(exchanging).await.unwrap();
        repository.consume("exchanging", now).await.unwrap();
        let mut failed = new_session(now + chrono::Duration::seconds(2));
        failed.state = "failed".into();
        repository.create(failed).await.unwrap();
        repository.fail("failed", "backend.errors.a").await.unwrap();

        let open = repository.open_flows(now).await.unwrap();
        let states: Vec<&str> = open.iter().map(|flow| flow.state.as_str()).collect();
        assert_eq!(states, vec!["exchanging", "state"]);
    }

    #[tokio::test]
    async fn outcome_rows_are_pruned_once_they_are_long_past_use() {
        crate::utils::crypto::seed_master_key_for_tests();
        let pool = pool(1).await;
        account(&pool).await;
        let now = Utc::now();
        let repository = OAuthFlowRepository::new(&pool);
        repository.create(new_session(now)).await.unwrap();
        repository.consume("state", now).await.unwrap();

        let mut later = new_session(now + chrono::Duration::hours(25));
        later.state = "later".into();
        later.expires_at = now + chrono::Duration::hours(25) + chrono::Duration::minutes(15);
        repository.create(later).await.unwrap();

        assert!(repository.status("state", now).await.unwrap().is_none());
    }
}
