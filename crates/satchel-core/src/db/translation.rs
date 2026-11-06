//! Translation settings, keys, consents, cached translations and the
//! outbound request log (migration 0135). All of it is per device.

use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use crate::utils::crypto::{self, SecretContext};

/// How long a request log entry is kept (ADR 0016).
pub const REQUEST_LOG_RETENTION_DAYS: i64 = 30;

/// The newest entries the settings page lists.
pub const REQUEST_LOG_PAGE: i64 = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranslationSettingsRecord {
    pub engine: Option<String>,
    pub target_language: Option<String>,
    pub log_sent_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/translation/"))]
pub struct TranslationConsentRecord {
    pub engine: String,
    pub account_id: String,
    /// RFC 3339.
    pub granted_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct CachedTranslationRecord {
    pub message_id: String,
    pub target_language: String,
    pub engine: String,
    pub source_language: Option<String>,
    pub source_hash: String,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub created_at: String,
}

/// One request that left the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/db/translation/"))]
pub struct RequestLogEntry {
    pub id: i64,
    /// RFC 3339.
    pub created_at: String,
    /// `translation`; later the AI features of ADR 0016.
    pub feature: String,
    /// The engine or provider, e.g. `deepl`.
    pub provider: String,
    pub model: Option<String>,
    pub account_id: Option<String>,
    pub message_id: Option<String>,
    /// What the provider billed, in `unit`.
    pub units: i64,
    /// `characters` or `tokens`.
    pub unit: String,
    /// `ok`, or the message key the request ended in.
    pub outcome: String,
    /// The sent text was kept (the person asked for that). It is loaded one
    /// entry at a time with [`TranslationRepository::sent_text`].
    pub has_sent_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequestLogEntry<'a> {
    pub created_at: &'a str,
    pub feature: &'a str,
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub account_id: Option<&'a str>,
    pub message_id: Option<&'a str>,
    pub units: i64,
    pub unit: &'a str,
    pub outcome: &'a str,
    pub sent_text: Option<&'a str>,
}

/// What a message offers for translation, straight from its row.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct MessageSourceRecord {
    pub account_id: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    /// S/MIME enveloped, or a PGP/MIME `multipart/encrypted` container.
    pub encrypted_container: bool,
}

pub struct TranslationRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TranslationRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn settings(&self) -> Result<TranslationSettingsRecord, sqlx::Error> {
        let row: Option<(Option<String>, Option<String>, i64)> = sqlx::query_as(
            "SELECT engine, target_language, log_sent_text FROM translation_settings WHERE id = 1",
        )
        .fetch_optional(self.pool)
        .await?;
        Ok(row
            .map(
                |(engine, target_language, log_sent_text)| TranslationSettingsRecord {
                    engine,
                    target_language,
                    log_sent_text: log_sent_text != 0,
                },
            )
            .unwrap_or_default())
    }

    pub async fn save_settings(
        &self,
        settings: &TranslationSettingsRecord,
        now: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO translation_settings (id, engine, target_language, log_sent_text, updated_at)
             VALUES (1, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                engine = excluded.engine,
                target_language = excluded.target_language,
                log_sent_text = excluded.log_sent_text,
                updated_at = excluded.updated_at",
        )
        .bind(settings.engine.as_deref())
        .bind(settings.target_language.as_deref())
        .bind(i64::from(settings.log_sent_text))
        .bind(now)
        .execute(self.pool)
        .await?;
        if !settings.log_sent_text {
            // Turning it off forgets what was kept, not only what comes next.
            sqlx::query(
                "UPDATE outbound_request_log SET sent_text = NULL WHERE sent_text IS NOT NULL",
            )
            .execute(self.pool)
            .await?;
        }
        Ok(())
    }

    pub async fn has_key(&self, engine: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM translation_engine_keys WHERE engine = ? AND encrypted_api_key <> '')",
        )
        .bind(engine)
        .fetch_one(self.pool)
        .await
    }

    /// The plan stored with the key, without decrypting it.
    pub async fn plan(&self, engine: &str) -> Result<Option<String>, sqlx::Error> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT plan FROM translation_engine_keys WHERE engine = ? AND encrypted_api_key <> ''",
        )
        .bind(engine)
        .fetch_optional(self.pool)
        .await?
        .flatten())
    }

    /// The stored key in clear, for the engine's own requests only.
    pub async fn key(&self, engine: &str) -> Result<Option<String>, KeyError> {
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT encrypted_api_key FROM translation_engine_keys WHERE engine = ?",
        )
        .bind(engine)
        .fetch_optional(self.pool)
        .await
        .map_err(KeyError::Persistence)?;
        let Some(stored) = stored.filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        // Off the executor while a keychain prompt is open.
        crypto::wait_for_master_key()
            .await
            .map_err(KeyError::Crypto)?;
        crypto::decrypt_secret(&stored, SecretContext::translation_api_key(engine))
            .map(Some)
            .map_err(KeyError::Crypto)
    }

    /// Stores `key` encrypted with its `plan`, or removes the engine's key for `None`.
    pub async fn set_key(
        &self,
        engine: &str,
        key: Option<&str>,
        plan: Option<&str>,
        now: &str,
    ) -> Result<(), KeyError> {
        let Some(key) = key else {
            sqlx::query("DELETE FROM translation_engine_keys WHERE engine = ?")
                .bind(engine)
                .execute(self.pool)
                .await
                .map_err(KeyError::Persistence)?;
            return Ok(());
        };
        crypto::wait_for_master_key()
            .await
            .map_err(KeyError::Crypto)?;
        let encrypted = crypto::encrypt_secret(key, SecretContext::translation_api_key(engine))
            .map_err(KeyError::Crypto)?;
        sqlx::query(
            "INSERT INTO translation_engine_keys (engine, encrypted_api_key, plan, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(engine) DO UPDATE SET
                encrypted_api_key = excluded.encrypted_api_key,
                plan = excluded.plan,
                updated_at = excluded.updated_at",
        )
        .bind(engine)
        .bind(encrypted)
        .bind(plan)
        .bind(now)
        .execute(self.pool)
        .await
        .map_err(KeyError::Persistence)?;
        Ok(())
    }

    pub async fn consents(&self) -> Result<Vec<TranslationConsentRecord>, sqlx::Error> {
        sqlx::query_as(
            "SELECT c.engine, c.account_id, c.granted_at
             FROM translation_consents c
             JOIN accounts a ON a.id = c.account_id
             ORDER BY c.engine, c.account_id",
        )
        .fetch_all(self.pool)
        .await
    }

    pub async fn has_consent(&self, engine: &str, account_id: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM translation_consents WHERE engine = ? AND account_id = ?)",
        )
        .bind(engine)
        .bind(account_id)
        .fetch_one(self.pool)
        .await
    }

    /// Returns false when the account does not exist.
    pub async fn set_consent(
        &self,
        engine: &str,
        account_id: &str,
        granted: bool,
        now: &str,
    ) -> Result<bool, sqlx::Error> {
        if !granted {
            sqlx::query("DELETE FROM translation_consents WHERE engine = ? AND account_id = ?")
                .bind(engine)
                .bind(account_id)
                .execute(self.pool)
                .await?;
            return Ok(true);
        }
        let inserted = sqlx::query(
            "INSERT INTO translation_consents (engine, account_id, granted_at)
             SELECT ?, id, ? FROM accounts WHERE id = ?
             ON CONFLICT(engine, account_id) DO NOTHING",
        )
        .bind(engine)
        .bind(now)
        .bind(account_id)
        .execute(self.pool)
        .await?;
        if inserted.rows_affected() > 0 {
            return Ok(true);
        }
        self.has_consent(engine, account_id).await
    }

    pub async fn message_source(
        &self,
        message_id: &str,
    ) -> Result<Option<MessageSourceRecord>, sqlx::Error> {
        sqlx::query_as(
            "SELECT m.account_id, m.body_text, m.body_html,
                    (m.smime_encrypted IS NOT NULL
                     OR LOWER(COALESCE(m.raw_headers, '')) LIKE '%multipart/encrypted%')
                        AS encrypted_container
             FROM messages m
             WHERE m.id = ?",
        )
        .bind(message_id)
        .fetch_optional(self.pool)
        .await
    }

    pub async fn cached(
        &self,
        message_id: &str,
        target_language: &str,
    ) -> Result<Option<CachedTranslationRecord>, sqlx::Error> {
        sqlx::query_as(
            "SELECT message_id, target_language, engine, source_language, source_hash,
                    body_html, body_text, created_at
             FROM message_translations WHERE message_id = ? AND target_language = ?",
        )
        .bind(message_id)
        .bind(target_language)
        .fetch_optional(self.pool)
        .await
    }

    pub async fn store(&self, record: &CachedTranslationRecord) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO message_translations
                (message_id, target_language, engine, source_language, source_hash,
                 body_html, body_text, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(message_id, target_language) DO UPDATE SET
                engine = excluded.engine,
                source_language = excluded.source_language,
                source_hash = excluded.source_hash,
                body_html = excluded.body_html,
                body_text = excluded.body_text,
                created_at = excluded.created_at",
        )
        .bind(&record.message_id)
        .bind(&record.target_language)
        .bind(&record.engine)
        .bind(record.source_language.as_deref())
        .bind(&record.source_hash)
        .bind(record.body_html.as_deref())
        .bind(record.body_text.as_deref())
        .bind(&record.created_at)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn log(&self, entry: &NewRequestLogEntry<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO outbound_request_log
                (created_at, feature, provider, model, account_id, message_id, units, unit,
                 outcome, sent_text)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(entry.created_at)
        .bind(entry.feature)
        .bind(entry.provider)
        .bind(entry.model)
        .bind(entry.account_id)
        .bind(entry.message_id)
        .bind(entry.units)
        .bind(entry.unit)
        .bind(entry.outcome)
        .bind(entry.sent_text)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Newest first.
    pub async fn request_log(&self, limit: i64) -> Result<Vec<RequestLogEntry>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, created_at, feature, provider, model, account_id, message_id, units, unit,
                    outcome, sent_text IS NOT NULL AS has_sent_text
             FROM outbound_request_log ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(limit.clamp(1, 1000))
        .fetch_all(self.pool)
        .await
    }

    /// The text one log entry kept, if it kept any.
    ///
    /// Scoped to `feature`: a caller reads only what its own feature sent,
    /// so a later feature cannot read translation text by guessing ids.
    pub async fn sent_text(&self, feature: &str, id: i64) -> Result<Option<String>, sqlx::Error> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT sent_text FROM outbound_request_log WHERE id = ? AND feature = ?",
        )
        .bind(id)
        .bind(feature)
        .fetch_optional(self.pool)
        .await?
        .flatten())
    }

    /// Drops translations no body backs any more: the message is gone, or
    /// its body was released (the body window, the storage budget).
    pub async fn prune_unreachable_translations(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "DELETE FROM message_translations
             WHERE NOT EXISTS (
                 SELECT 1 FROM messages m
                 WHERE m.id = message_translations.message_id
                   AND (NULLIF(m.body_text, '') IS NOT NULL OR NULLIF(m.body_html, '') IS NOT NULL)
             )",
        )
        .execute(self.pool)
        .await?
        .rows_affected())
    }

    pub async fn clear_request_log(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM outbound_request_log")
            .execute(self.pool)
            .await?
            .rows_affected())
    }

    /// Drops entries older than [`REQUEST_LOG_RETENTION_DAYS`].
    pub async fn prune_request_log(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, sqlx::Error> {
        let cutoff = (now - chrono::Duration::days(REQUEST_LOG_RETENTION_DAYS)).to_rfc3339();
        Ok(
            sqlx::query("DELETE FROM outbound_request_log WHERE created_at < ?")
                .bind(cutoff)
                .execute(self.pool)
                .await?
                .rows_affected(),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("translation key storage failed: {0}")]
    Persistence(sqlx::Error),
    #[error("translation key could not be protected: {0}")]
    Crypto(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sender_sweep::tests::{insert_account, pool};

    const NOW: &str = "2026-11-06T10:00:00+00:00";

    #[tokio::test]
    async fn settings_default_to_off_and_round_trip() {
        let pool = pool().await;
        let repo = TranslationRepository::new(&pool);
        assert_eq!(
            repo.settings().await.unwrap(),
            TranslationSettingsRecord::default()
        );
        let saved = TranslationSettingsRecord {
            engine: Some("deepl".into()),
            target_language: Some("DE".into()),
            log_sent_text: true,
        };
        repo.save_settings(&saved, NOW).await.unwrap();
        assert_eq!(repo.settings().await.unwrap(), saved);
    }

    #[tokio::test]
    async fn keys_are_stored_encrypted_and_bound_to_their_engine() {
        crypto::seed_master_key_for_tests();
        let pool = pool().await;
        let repo = TranslationRepository::new(&pool);
        assert!(!repo.has_key("deepl").await.unwrap());
        repo.set_key("deepl", Some("secret-key:fx"), Some("free"), NOW)
            .await
            .unwrap();
        let stored: String =
            sqlx::query_scalar("SELECT encrypted_api_key FROM translation_engine_keys")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(stored.starts_with("enc:v2:"));
        assert!(!stored.contains("secret-key"));
        assert_eq!(
            repo.key("deepl").await.unwrap().as_deref(),
            Some("secret-key:fx")
        );
        assert_eq!(repo.plan("deepl").await.unwrap().as_deref(), Some("free"));

        // Copied onto another engine's row, the ciphertext does not decrypt.
        sqlx::query("INSERT INTO translation_engine_keys VALUES ('other', ?, 'free', ?)")
            .bind(&stored)
            .bind(NOW)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(repo.key("other").await, Err(KeyError::Crypto(_))));

        repo.set_key("deepl", None, None, NOW).await.unwrap();
        assert!(!repo.has_key("deepl").await.unwrap());
        assert_eq!(repo.key("deepl").await.unwrap(), None);
    }

    #[tokio::test]
    async fn consent_is_per_account_and_leaves_with_the_account() {
        let pool = pool().await;
        insert_account(&pool, "private").await;
        insert_account(&pool, "work").await;
        let repo = TranslationRepository::new(&pool);
        assert!(repo
            .set_consent("deepl", "private", true, NOW)
            .await
            .unwrap());
        assert!(!repo
            .set_consent("deepl", "missing", true, NOW)
            .await
            .unwrap());
        assert!(repo.has_consent("deepl", "private").await.unwrap());
        assert!(!repo.has_consent("deepl", "work").await.unwrap());
        assert_eq!(repo.consents().await.unwrap().len(), 1);

        for statement in [
            "DELETE FROM folders WHERE account_id = 'private'",
            "DELETE FROM accounts WHERE id = 'private'",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        assert!(repo.consents().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_removed_account_takes_its_log_entries_along() {
        let pool = pool().await;
        insert_account(&pool, "gone").await;
        let repo = TranslationRepository::new(&pool);
        repo.log(&NewRequestLogEntry {
            created_at: NOW,
            feature: "translation",
            provider: "deepl",
            model: None,
            account_id: Some("gone"),
            message_id: None,
            units: 1,
            unit: "characters",
            outcome: "ok",
            sent_text: None,
        })
        .await
        .unwrap();
        for statement in [
            "DELETE FROM folders WHERE account_id = 'gone'",
            "DELETE FROM accounts WHERE id = 'gone'",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        assert!(repo.request_log(REQUEST_LOG_PAGE).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn translations_without_a_body_are_pruned() {
        let pool = pool().await;
        insert_account(&pool, "acc").await;
        for (thread, body) in [("kept", Some("Text")), ("released", None)] {
            crate::db::sender_sweep::tests::insert_thread(
                &pool,
                "acc",
                thread,
                "a@example.test",
                1,
                "inbox",
                "inbox",
                true,
            )
            .await;
            sqlx::query("UPDATE messages SET body_text = ? WHERE id = ?")
                .bind(body)
                .bind(format!("{thread}-m"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO message_translations
                    (message_id, target_language, engine, source_hash, body_text, created_at)
                 VALUES (?, 'DE', 'deepl', 'h', 'Text', ?)",
            )
            .bind(format!("{thread}-m"))
            .bind(NOW)
            .execute(&pool)
            .await
            .unwrap();
        }
        let repo = TranslationRepository::new(&pool);
        assert_eq!(repo.prune_unreachable_translations().await.unwrap(), 1);
        let left: Vec<String> = sqlx::query_scalar("SELECT message_id FROM message_translations")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, ["kept-m"]);
    }

    #[tokio::test]
    async fn the_log_keeps_thirty_days_and_forgets_text_on_request() {
        let pool = pool().await;
        let repo = TranslationRepository::new(&pool);
        let now = chrono::DateTime::parse_from_rfc3339(NOW).unwrap().to_utc();
        for (days_ago, text) in [(31, Some("old")), (2, Some("recent"))] {
            let created_at = (now - chrono::Duration::days(days_ago)).to_rfc3339();
            repo.log(&NewRequestLogEntry {
                created_at: &created_at,
                feature: "translation",
                provider: "deepl",
                model: None,
                account_id: None,
                message_id: None,
                units: 10,
                unit: "characters",
                outcome: "ok",
                sent_text: text,
            })
            .await
            .unwrap();
        }
        assert_eq!(repo.prune_request_log(now).await.unwrap(), 1);
        let entries = repo.request_log(REQUEST_LOG_PAGE).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].has_sent_text);
        assert_eq!(
            repo.sent_text("translation", entries[0].id)
                .await
                .unwrap()
                .as_deref(),
            Some("recent")
        );
        assert_eq!(
            repo.sent_text("other-feature", entries[0].id)
                .await
                .unwrap(),
            None,
            "another feature reads nothing"
        );

        repo.save_settings(&TranslationSettingsRecord::default(), NOW)
            .await
            .unwrap();
        assert!(!repo.request_log(REQUEST_LOG_PAGE).await.unwrap()[0].has_sent_text);
        assert_eq!(
            repo.sent_text("translation", entries[0].id).await.unwrap(),
            None
        );
        assert_eq!(repo.clear_request_log().await.unwrap(), 1);
    }
}
