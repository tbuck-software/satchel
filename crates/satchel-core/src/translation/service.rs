//! What every engine shares: which engine is set up, whether this account's
//! mail may go to it, the local cache, the second pass through the sanitizer
//! and the request log.
//!
//! Order matters. The cache is read before anything else, because a cached
//! translation leaves nothing on the wire and needs neither key nor consent.
//! Consent is checked before the engine is asked; every request that reaches
//! the engine, failed or not, is logged.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use super::{
    deepl::DeepL, languages::detect_language, normalize_language, same_language, EngineKind,
    TranslationContent, TranslationEngine, TranslationError, TranslationRequest,
};
use crate::db::translation::{
    CachedTranslationRecord, KeyError, MessageSourceRecord, NewRequestLogEntry,
    TranslationRepository,
};

/// The feature name translation requests carry in the request log.
pub const FEATURE: &str = "translation";

/// A message's body in another language, ready for the reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/translation/"))]
pub struct MessageTranslation {
    pub message_id: String,
    pub target_language: String,
    pub engine: String,
    /// What the engine detected; local detection never fills this.
    pub source_language: Option<String>,
    /// Sanitized again on every read. `None` when only text was translated.
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    /// True when it came from the local cache and nothing was sent.
    pub cached: bool,
    /// RFC 3339; when the engine produced it.
    pub created_at: String,
}

/// Whether Translate is worth offering for a message, without sending it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/translation/"))]
pub struct MessageTranslationOffer {
    /// The local guess at the body's language, ISO 639-1 upper case.
    pub detected_language: Option<String>,
    /// False when the guess says the body already is in the target language,
    /// or there is nothing that could be sent.
    pub offered: bool,
    /// A translation into the target language is stored on this device.
    pub cached: bool,
    /// The cached translation's source language as the engine reported it.
    pub cached_source_language: Option<String>,
    /// The message's account may send mail to the active engine.
    pub consented: bool,
}

fn key_error(error: KeyError) -> TranslationError {
    match error {
        KeyError::Persistence(error) => TranslationError::Persistence(error.to_string()),
        KeyError::Crypto(error) => {
            tracing::warn!(error = %error, "translation key unavailable");
            TranslationError::CredentialsUnavailable
        }
    }
}

/// The engine `kind` with its stored key, whether or not it is the active one
/// (the settings page tests a key before switching translation on).
pub async fn engine_for(
    pool: &SqlitePool,
    kind: EngineKind,
) -> Result<Box<dyn TranslationEngine>, TranslationError> {
    let key = TranslationRepository::new(pool)
        .key(kind.as_str())
        .await
        .map_err(key_error)?
        .ok_or(TranslationError::KeyMissing)?;
    match kind {
        EngineKind::DeepL => Ok(Box::new(DeepL::new(&key)?)),
    }
}

/// The engine translation is switched to.
pub async fn active_engine(pool: &SqlitePool) -> Result<EngineKind, TranslationError> {
    TranslationRepository::new(pool)
        .settings()
        .await?
        .engine
        .as_deref()
        .and_then(EngineKind::parse)
        .ok_or(TranslationError::NotConfigured)
}

fn target(target_language: &str) -> Result<String, TranslationError> {
    normalize_language(target_language).ok_or(TranslationError::UnsupportedLanguage)
}

fn is_encrypted(source: &MessageSourceRecord) -> bool {
    const ARMOR: &str = "-----BEGIN PGP MESSAGE-----";
    source.smime_encrypted
        || source
            .body_text
            .as_deref()
            .is_some_and(|text| text.contains(ARMOR))
        || source
            .body_html
            .as_deref()
            .is_some_and(|html| html.contains(ARMOR))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

/// What an engine would be sent for this message: the sanitized HTML for an
/// engine that keeps markup, the text body otherwise.
fn content_for(
    source: &MessageSourceRecord,
    accepts_html: bool,
) -> Result<TranslationContent, TranslationError> {
    if is_encrypted(source) {
        return Err(TranslationError::Encrypted);
    }
    if accepts_html {
        if let Some(html) = non_empty(source.body_html.as_deref()) {
            return Ok(TranslationContent::Html(html.to_owned()));
        }
    }
    non_empty(source.body_text.as_deref())
        .map(|text| TranslationContent::Text(text.to_owned()))
        .ok_or(TranslationError::NothingToTranslate)
}

/// Identifies the body a translation was made from. Includes the kind, so a
/// text translation does not stand in for an HTML one.
fn source_hash(content: &TranslationContent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(if content.is_html() {
        b"html\0"
    } else {
        b"text\0"
    });
    hasher.update(content.as_str().as_bytes());
    crate::utils::hex::lower_hex(&hasher.finalize())
}

fn from_cache(record: CachedTranslationRecord) -> MessageTranslation {
    MessageTranslation {
        message_id: record.message_id,
        target_language: record.target_language,
        engine: record.engine,
        source_language: record.source_language,
        // The sanitizer may have learned something since this was stored.
        body_html: record
            .body_html
            .map(|html| crate::email::sanitizer::sanitize_stored_email_html(&html)),
        body_text: record.body_text,
        cached: true,
        created_at: record.created_at,
    }
}

async fn source(
    repo: &TranslationRepository<'_>,
    message_id: &str,
) -> Result<MessageSourceRecord, TranslationError> {
    repo.message_source(message_id)
        .await?
        .ok_or(TranslationError::MessageNotFound)
}

/// The stored translation of `message_id` into `target_language`, if one
/// exists and was made from the body the message has now.
pub async fn cached_translation(
    pool: &SqlitePool,
    message_id: &str,
    target_language: &str,
) -> Result<Option<MessageTranslation>, TranslationError> {
    let target = target(target_language)?;
    let repo = TranslationRepository::new(pool);
    let Some(record) = repo.cached(message_id, &target).await? else {
        return Ok(None);
    };
    let source = source(&repo, message_id).await?;
    // Cached from HTML or from text; either may match the body as it is now.
    let current = [true, false]
        .into_iter()
        .filter_map(|html| content_for(&source, html).ok())
        .any(|content| source_hash(&content) == record.source_hash);
    Ok(current.then(|| from_cache(record)))
}

/// Whether to offer Translate for `message_id`. Reads only this device.
pub async fn offer(
    pool: &SqlitePool,
    message_id: &str,
    target_language: &str,
) -> Result<MessageTranslationOffer, TranslationError> {
    let target = target(target_language)?;
    let repo = TranslationRepository::new(pool);
    let source = source(&repo, message_id).await?;
    let cached = cached_translation(pool, message_id, &target).await?;
    let sendable = content_for(&source, false).is_ok() || content_for(&source, true).is_ok();
    let detected_language = non_empty(source.body_text.as_deref())
        .and_then(detect_language)
        .map(str::to_owned);
    let already_target = detected_language
        .as_deref()
        .is_some_and(|language| same_language(language, &target));
    let consented = match repo.settings().await?.engine.as_deref() {
        Some(engine) => repo.has_consent(engine, &source.account_id).await?,
        None => false,
    };
    Ok(MessageTranslationOffer {
        detected_language,
        offered: sendable && !already_target,
        cached: cached.is_some(),
        cached_source_language: cached.and_then(|cached| cached.source_language),
        consented,
    })
}

/// Translates `message_id` into `target_language` with `engine`, or returns
/// the stored translation.
pub async fn translate_message(
    pool: &SqlitePool,
    engine: &dyn TranslationEngine,
    message_id: &str,
    target_language: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<MessageTranslation, TranslationError> {
    let target = target(target_language)?;
    if let Some(cached) = cached_translation(pool, message_id, &target).await? {
        return Ok(cached);
    }
    let repo = TranslationRepository::new(pool);
    let source = source(&repo, message_id).await?;
    let engine_id = engine.kind().as_str();
    if !repo.has_consent(engine_id, &source.account_id).await? {
        return Err(TranslationError::ConsentRequired);
    }
    let content = content_for(&source, engine.accepts_html())?;
    let settings = repo.settings().await?;
    let request = TranslationRequest {
        content,
        source_language: None,
        target_language: target.clone(),
    };
    let result = engine.translate(&request).await;
    let created_at = now.to_rfc3339();
    let outcome = match &result {
        Ok(_) => "ok",
        Err(error) => error.message_key(),
    };
    let units = result
        .as_ref()
        .ok()
        .and_then(|translated| translated.billed_units)
        .unwrap_or_else(|| {
            if result.is_ok() {
                request.content.characters()
            } else {
                0
            }
        });
    // Every attempt is logged, including one the engine refused before
    // sending (too large): the outcome says which it was.
    repo.log(&NewRequestLogEntry {
        created_at: &created_at,
        feature: FEATURE,
        provider: engine_id,
        model: result
            .as_ref()
            .ok()
            .and_then(|translated| translated.model.as_deref()),
        account_id: Some(&source.account_id),
        message_id: Some(message_id),
        units,
        unit: engine.billing_unit().as_str(),
        outcome,
        sent_text: settings.log_sent_text.then_some(request.content.as_str()),
    })
    .await?;
    let translated = result?;
    let (body_html, body_text) = match &request.content {
        TranslationContent::Html(_) => (
            Some(crate::email::sanitizer::sanitize_stored_email_html(
                &translated.content,
            )),
            None,
        ),
        TranslationContent::Text(_) => (None, Some(translated.content)),
    };
    let record = CachedTranslationRecord {
        message_id: message_id.to_owned(),
        target_language: target,
        engine: engine_id.to_owned(),
        source_language: translated.detected_source_language,
        source_hash: source_hash(&request.content),
        body_html,
        body_text,
        created_at,
    };
    repo.store(&record).await?;
    let mut translation = from_cache(record);
    translation.cached = false;
    Ok(translation)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::db::sender_sweep::tests::{insert_account, insert_thread, pool};
    use crate::translation::{
        Locality, Translated, TranslationLanguage, TranslationUsage, UsageUnit,
    };

    /// Records what it was sent and answers with a fixed result.
    struct FakeEngine {
        html: bool,
        sent: Arc<Mutex<Vec<TranslationRequest>>>,
        answer: Result<String, TranslationError>,
    }

    impl FakeEngine {
        fn answering(answer: Result<&str, TranslationError>) -> Self {
            Self {
                html: true,
                sent: Arc::default(),
                answer: answer.map(str::to_owned),
            }
        }

        fn requests(&self) -> usize {
            self.sent.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl TranslationEngine for FakeEngine {
        fn kind(&self) -> EngineKind {
            EngineKind::DeepL
        }
        fn locality(&self) -> Locality {
            Locality::Remote
        }
        fn accepts_html(&self) -> bool {
            self.html
        }
        fn billing_unit(&self) -> UsageUnit {
            UsageUnit::Characters
        }
        async fn translate(
            &self,
            request: &TranslationRequest,
        ) -> Result<Translated, TranslationError> {
            self.sent.lock().unwrap().push(request.clone());
            self.answer.clone().map(|content| Translated {
                content,
                detected_source_language: Some("EN".into()),
                billed_units: Some(42),
                model: None,
            })
        }
        async fn languages(&self) -> Result<Vec<TranslationLanguage>, TranslationError> {
            Ok(Vec::new())
        }
        async fn usage(&self) -> Result<TranslationUsage, TranslationError> {
            Err(TranslationError::Unavailable)
        }
    }

    const ENGLISH: &str = "Hi everyone, attached are the documents for Monday's meeting. \
                           Please let me know by Friday whether the time works for you.";

    async fn setup(html: Option<&str>) -> SqlitePool {
        let pool = pool().await;
        insert_account(&pool, "acc").await;
        insert_thread(
            &pool,
            "acc",
            "t1",
            "a@example.com",
            1,
            "inbox",
            "inbox",
            false,
        )
        .await;
        sqlx::query("UPDATE messages SET body_text = ?, body_html = ? WHERE id = 't1-m'")
            .bind(ENGLISH)
            .bind(html)
            .execute(&pool)
            .await
            .unwrap();
        let repo = TranslationRepository::new(&pool);
        repo.save_settings(
            &crate::db::translation::TranslationSettingsRecord {
                engine: Some("deepl".into()),
                target_language: None,
                log_sent_text: false,
            },
            "2026-11-06T00:00:00Z",
        )
        .await
        .unwrap();
        pool
    }

    async fn consent(pool: &SqlitePool) {
        TranslationRepository::new(pool)
            .set_consent("deepl", "acc", true, "2026-11-06T00:00:00Z")
            .await
            .unwrap();
    }

    async fn log_rows(pool: &SqlitePool) -> Vec<(String, Option<String>)> {
        sqlx::query_as("SELECT outcome, sent_text FROM outbound_request_log ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn nothing_leaves_without_consent() {
        let pool = setup(Some("<p>Hi</p>")).await;
        let engine = FakeEngine::answering(Ok("<p>Hallo</p>"));
        let error = translate_message(&pool, &engine, "t1-m", "DE", chrono::Utc::now())
            .await
            .unwrap_err();
        assert_eq!(error, TranslationError::ConsentRequired);
        assert_eq!(engine.requests(), 0);
        assert!(log_rows(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn translates_html_sanitizes_the_answer_and_caches_it() {
        let pool = setup(Some("<p>Hi</p>")).await;
        consent(&pool).await;
        let engine = FakeEngine::answering(Ok(
            "<p onclick=\"steal()\">Hallo</p><script>alert(1)</script><img src=\"https://t.example/p.gif\">",
        ));
        let translation = translate_message(&pool, &engine, "t1-m", "de", chrono::Utc::now())
            .await
            .unwrap();
        assert!(!translation.cached);
        assert_eq!(translation.target_language, "DE");
        assert_eq!(translation.source_language.as_deref(), Some("EN"));
        let html = translation.body_html.unwrap();
        assert!(html.contains("Hallo"));
        assert!(!html.contains("onclick"));
        assert!(!html.contains("<script"));
        assert_eq!(
            engine.sent.lock().unwrap()[0].content,
            TranslationContent::Html("<p>Hi</p>".into())
        );

        // Reopening is free: no second request, and offline works the same.
        let again = translate_message(&pool, &engine, "t1-m", "DE", chrono::Utc::now())
            .await
            .unwrap();
        assert!(again.cached);
        assert_eq!(engine.requests(), 1);
        assert_eq!(log_rows(&pool).await, vec![("ok".into(), None)]);
    }

    #[tokio::test]
    async fn a_changed_body_makes_the_cached_translation_stale() {
        let pool = setup(None).await;
        consent(&pool).await;
        let engine = FakeEngine::answering(Ok("Hallo zusammen"));
        translate_message(&pool, &engine, "t1-m", "DE", chrono::Utc::now())
            .await
            .unwrap();
        assert!(cached_translation(&pool, "t1-m", "DE")
            .await
            .unwrap()
            .is_some());
        sqlx::query("UPDATE messages SET body_text = 'Something else entirely' WHERE id = 't1-m'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(cached_translation(&pool, "t1-m", "DE")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn engines_without_html_get_the_text_body() {
        let pool = setup(Some("<p>Hi</p>")).await;
        consent(&pool).await;
        let mut engine = FakeEngine::answering(Ok("Hallo"));
        engine.html = false;
        let translation = translate_message(&pool, &engine, "t1-m", "DE", chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(translation.body_html, None);
        assert_eq!(translation.body_text.as_deref(), Some("Hallo"));
        assert_eq!(
            engine.sent.lock().unwrap()[0].content,
            TranslationContent::Text(ENGLISH.into())
        );
    }

    #[tokio::test]
    async fn failures_are_logged_with_their_key_and_cache_nothing() {
        let pool = setup(None).await;
        consent(&pool).await;
        TranslationRepository::new(&pool)
            .save_settings(
                &crate::db::translation::TranslationSettingsRecord {
                    engine: Some("deepl".into()),
                    target_language: None,
                    log_sent_text: true,
                },
                "2026-11-06T00:00:00Z",
            )
            .await
            .unwrap();
        let engine = FakeEngine::answering(Err(TranslationError::QuotaExceeded));
        let error = translate_message(&pool, &engine, "t1-m", "DE", chrono::Utc::now())
            .await
            .unwrap_err();
        assert_eq!(error, TranslationError::QuotaExceeded);
        assert_eq!(
            log_rows(&pool).await,
            vec![(
                "backend.errors.translation.quotaExceeded".into(),
                Some(ENGLISH.into())
            )]
        );
        assert!(cached_translation(&pool, "t1-m", "DE")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn encrypted_mail_is_never_sent() {
        let pool = setup(None).await;
        consent(&pool).await;
        sqlx::query(
            "UPDATE messages SET body_text = '-----BEGIN PGP MESSAGE-----\nhQEMA...' WHERE id = 't1-m'",
        )
        .execute(&pool)
        .await
        .unwrap();
        let engine = FakeEngine::answering(Ok("x"));
        assert_eq!(
            translate_message(&pool, &engine, "t1-m", "DE", chrono::Utc::now())
                .await
                .unwrap_err(),
            TranslationError::Encrypted
        );
        assert_eq!(engine.requests(), 0);
        assert!(!offer(&pool, "t1-m", "DE").await.unwrap().offered);
    }

    #[tokio::test]
    async fn the_offer_skips_mail_already_in_the_target_language() {
        let pool = setup(None).await;
        let english = offer(&pool, "t1-m", "EN-GB").await.unwrap();
        assert_eq!(english.detected_language.as_deref(), Some("EN"));
        assert!(!english.offered);
        assert!(!english.consented);
        let german = offer(&pool, "t1-m", "DE").await.unwrap();
        assert!(german.offered);
        assert!(!german.cached);
        consent(&pool).await;
        assert!(offer(&pool, "t1-m", "DE").await.unwrap().consented);
        assert_eq!(
            offer(&pool, "missing", "DE").await.unwrap_err(),
            TranslationError::MessageNotFound
        );
    }
}
