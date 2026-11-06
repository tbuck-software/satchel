//! What every engine shares: which engine is set up, whether this account's
//! mail may go to it, the local cache, the second pass through the sanitizer
//! and the request log.
//!
//! A translation is two steps. [`prepare`] reads only this device: a stored
//! translation of the body as it is now comes back without key, consent or
//! network. Otherwise it hands back a [`PendingTranslation`], and [`send`]
//! checks the refusals and the consent, builds the request, asks the engine,
//! logs the request and stores the result. Every request that reaches the
//! engine, failed or not, is logged; one refused before sending is not.
//!
//! Sanitizing, hashing, language detection and URL withholding are CPU work
//! on whole mail bodies and run on the blocking pool.

use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use super::{
    deepl::{DeepL, DeepLPlan},
    languages::detect_language,
    normalize_language, same_language, EngineKind, TranslationContent, TranslationEngine,
    TranslationError, TranslationRequest,
};
use crate::db::translation::{
    CachedTranslationRecord, KeyError, MessageSourceRecord, NewRequestLogEntry,
    TranslationRepository,
};
use crate::email::sanitizer::{
    restore_references, sanitize_stored_email_html, withhold_references,
};

/// The feature name translation requests carry in the request log.
pub const FEATURE: &str = "translation";

/// The provider of accounts whose mail Satchel reads through Google's API.
const GOOGLE_PROVIDER: &str = "gmail";

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
    /// Sanitized again on every read. `None` when only text was translated
    /// (an engine without markup, or HTML too large for one request).
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
    /// The guess says the body is not in the target language and nothing
    /// refuses it: worth a button of its own.
    pub offered: bool,
    /// A translation of the body as it is now, into the target language, is
    /// stored on this device.
    pub cached: bool,
    /// The cached translation's source language as the engine reported it.
    pub cached_source_language: Option<String>,
    /// The message's account may send mail to the active engine.
    pub consented: bool,
    /// Why the active engine would not get this message, as a message key:
    /// encrypted, no text, or Gmail mail with an engine that keeps text.
    pub refusal: Option<String>,
}

/// A translation [`prepare`] could not answer from the cache.
#[derive(Debug, Clone)]
pub struct PendingTranslation {
    message_id: String,
    target: String,
    source: MessageSourceRecord,
    hashes: BodyHashes,
}

pub enum Prepared {
    Cached(MessageTranslation),
    Pending(PendingTranslation),
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, TranslationError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| TranslationError::Persistence(error.to_string()))
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

/// [`TranslationEngine::may_retain_text`] for a stored key's plan, without
/// building the engine. An unknown plan counts as keeping text.
fn plan_may_retain_text(kind: EngineKind, plan: Option<&str>) -> bool {
    match kind {
        EngineKind::DeepL => plan
            .and_then(DeepLPlan::parse)
            .is_none_or(DeepLPlan::may_retain_text),
    }
}

fn target(target_language: &str) -> Result<String, TranslationError> {
    normalize_language(target_language).ok_or(TranslationError::UnsupportedLanguage)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn is_encrypted(source: &MessageSourceRecord) -> bool {
    const ARMOR: &str = "-----BEGIN PGP MESSAGE-----";
    source.encrypted_container
        || source
            .body_text
            .as_deref()
            .is_some_and(|text| text.contains(ARMOR))
        || source
            .body_html
            .as_deref()
            .is_some_and(|html| html.contains(ARMOR))
}

/// Why `source` must not go to an engine with these terms, if it must not.
fn refusal(source: &MessageSourceRecord, retains_text: bool) -> Option<TranslationError> {
    if is_encrypted(source) {
        return Some(TranslationError::Encrypted);
    }
    if non_empty(source.body_html.as_deref()).is_none()
        && non_empty(source.body_text.as_deref()).is_none()
    {
        return Some(TranslationError::NothingToTranslate);
    }
    // Google's Limited Use rules forbid handing Gmail data to a party that
    // may keep it or learn from it (ADR 0018). An owner decision; it can be
    // loosened once a free plan's terms allow it.
    if retains_text && source.provider == GOOGLE_PROVIDER {
        return Some(TranslationError::GmailNeedsRetentionFreeEngine);
    }
    None
}

/// Identifies the body a translation was made from, per kind, so a text
/// translation does not stand in for an HTML one.
#[derive(Debug, Clone, Default)]
struct BodyHashes {
    html: Option<String>,
    text: Option<String>,
}

impl BodyHashes {
    fn of(source: &MessageSourceRecord) -> Self {
        fn hash(kind: &[u8], body: &str) -> String {
            let mut hasher = Sha256::new();
            hasher.update(kind);
            hasher.update(body.as_bytes());
            crate::utils::hex::lower_hex(&hasher.finalize())
        }
        Self {
            html: non_empty(source.body_html.as_deref()).map(|html| hash(b"html\0", html)),
            text: non_empty(source.body_text.as_deref()).map(|text| hash(b"text\0", text)),
        }
    }

    fn matches(&self, stored: &str) -> bool {
        self.html.as_deref() == Some(stored) || self.text.as_deref() == Some(stored)
    }
}

fn from_record(record: CachedTranslationRecord, cached: bool) -> MessageTranslation {
    MessageTranslation {
        message_id: record.message_id,
        target_language: record.target_language,
        engine: record.engine,
        source_language: record.source_language,
        body_html: record.body_html,
        body_text: record.body_text,
        cached,
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

/// The stored translation of the body as it is now, or what [`send`] needs.
pub async fn prepare(
    pool: &SqlitePool,
    message_id: &str,
    target_language: &str,
) -> Result<Prepared, TranslationError> {
    let target = target(target_language)?;
    let repo = TranslationRepository::new(pool);
    let source = source(&repo, message_id).await?;
    let cached = repo.cached(message_id, &target).await?;
    let (source, hashes, cached) = blocking(move || {
        let hashes = BodyHashes::of(&source);
        let cached = cached
            .filter(|record| hashes.matches(&record.source_hash))
            .map(|mut record| {
                // The sanitizer may have learned something since this was stored.
                record.body_html = record
                    .body_html
                    .map(|html| sanitize_stored_email_html(&html));
                record
            });
        (source, hashes, cached)
    })
    .await?;
    if let Some(record) = cached {
        return Ok(Prepared::Cached(from_record(record, true)));
    }
    Ok(Prepared::Pending(PendingTranslation {
        message_id: message_id.to_owned(),
        target,
        source,
        hashes,
    }))
}

/// Sends `pending` to `engine` and stores the result.
///
/// The HTML goes with its URLs withheld when the engine keeps markup and it
/// fits into one request; otherwise the text body goes, and the translation
/// renders as text. Neither fitting is refused before the wire, unlogged.
pub async fn send(
    pool: &SqlitePool,
    engine: &dyn TranslationEngine,
    pending: PendingTranslation,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<MessageTranslation, TranslationError> {
    let repo = TranslationRepository::new(pool);
    let engine_id = engine.kind().as_str();
    if let Some(refused) = refusal(&pending.source, engine.may_retain_text()) {
        return Err(refused);
    }
    if !repo
        .has_consent(engine_id, &pending.source.account_id)
        .await?
    {
        return Err(TranslationError::ConsentRequired);
    }
    let html = engine
        .accepts_html()
        .then(|| non_empty(pending.source.body_html.as_deref()).map(str::to_owned))
        .flatten();
    let had_html = html.is_some();
    let mut chosen = None;
    if let (Some(html), Some(hash)) = (html, pending.hashes.html.clone()) {
        let (withheld_html, withheld) = blocking(move || withhold_references(&html)).await?;
        let request = TranslationRequest {
            content: TranslationContent::Html(withheld_html),
            source_language: None,
            target_language: pending.target.clone(),
        };
        if engine.fits(&request) {
            chosen = Some((request, Some(withheld), hash));
        }
    }
    let (request, withheld, source_hash) = match chosen {
        Some(chosen) => chosen,
        None => {
            let (Some(text), Some(hash)) = (
                non_empty(pending.source.body_text.as_deref()),
                pending.hashes.text.clone(),
            ) else {
                return Err(if had_html {
                    TranslationError::TooLarge
                } else {
                    TranslationError::NothingToTranslate
                });
            };
            let request = TranslationRequest {
                content: TranslationContent::Text(text.to_owned()),
                source_language: None,
                target_language: pending.target.clone(),
            };
            if !engine.fits(&request) {
                return Err(TranslationError::TooLarge);
            }
            (request, None, hash)
        }
    };

    let settings = repo.settings().await?;
    let result = engine.translate(&request).await;
    let created_at = now.to_rfc3339();
    let units = match &result {
        Ok(translated) => translated
            .billed_units
            .unwrap_or_else(|| request.content.characters()),
        Err(_) => 0,
    };
    repo.log(&NewRequestLogEntry {
        created_at: &created_at,
        feature: FEATURE,
        provider: engine_id,
        model: result
            .as_ref()
            .ok()
            .and_then(|translated| translated.model.as_deref()),
        account_id: Some(&pending.source.account_id),
        message_id: Some(&pending.message_id),
        units,
        unit: engine.billing_unit().as_str(),
        outcome: match &result {
            Ok(_) => "ok",
            Err(error) => error.message_key(),
        },
        sent_text: settings.log_sent_text.then_some(request.content.as_str()),
    })
    .await?;
    let translated = result?;
    let source_language = translated.detected_source_language.clone();
    let is_html = request.content.is_html();
    let content = translated.content;
    let (body_html, body_text) = blocking(move || {
        if !is_html {
            return (None, Some(content));
        }
        let restored = match withheld {
            Some(withheld) => restore_references(&content, &withheld),
            None => content,
        };
        (Some(sanitize_stored_email_html(&restored)), None)
    })
    .await?;
    let record = CachedTranslationRecord {
        message_id: pending.message_id,
        target_language: pending.target,
        engine: engine_id.to_owned(),
        source_language,
        source_hash,
        body_html,
        body_text,
        created_at,
    };
    repo.store(&record).await?;
    Ok(from_record(record, false))
}

/// [`prepare`], then [`send`] when nothing is stored.
pub async fn translate_message(
    pool: &SqlitePool,
    engine: &dyn TranslationEngine,
    message_id: &str,
    target_language: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<MessageTranslation, TranslationError> {
    match prepare(pool, message_id, target_language).await? {
        Prepared::Cached(cached) => Ok(cached),
        Prepared::Pending(pending) => send(pool, engine, pending, now).await,
    }
}

/// Whether to offer Translate for `message_id`. Reads only this device and
/// compares hashes; nothing is sanitized or decrypted.
pub async fn offer(
    pool: &SqlitePool,
    message_id: &str,
    target_language: &str,
) -> Result<MessageTranslationOffer, TranslationError> {
    let target = target(target_language)?;
    let repo = TranslationRepository::new(pool);
    let source = source(&repo, message_id).await?;
    let cached = repo.cached(message_id, &target).await?;
    let engine = repo
        .settings()
        .await?
        .engine
        .as_deref()
        .and_then(EngineKind::parse);
    let (consented, retains_text) = match engine {
        Some(kind) => (
            repo.has_consent(kind.as_str(), &source.account_id).await?,
            plan_may_retain_text(kind, repo.plan(kind.as_str()).await?.as_deref()),
        ),
        None => (false, false),
    };
    let refused = refusal(&source, retains_text).map(|error| error.message_key().to_owned());
    let target_for_guess = target.clone();
    let (detected_language, cached) = blocking(move || {
        let hashes = BodyHashes::of(&source);
        let cached = cached.filter(|record| hashes.matches(&record.source_hash));
        let detected = non_empty(source.body_text.as_deref())
            .and_then(detect_language)
            .map(str::to_owned);
        (detected, cached)
    })
    .await?;
    let already_target = detected_language
        .as_deref()
        .is_some_and(|language| same_language(language, &target_for_guess));
    Ok(MessageTranslationOffer {
        offered: refused.is_none() && !already_target,
        detected_language,
        cached: cached.is_some(),
        cached_source_language: cached.and_then(|record| record.source_language),
        consented,
        refusal: refused,
    })
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

    /// Records what it was sent and answers with a fixed result; or, with
    /// `echo`, with what it was sent.
    struct FakeEngine {
        html: bool,
        retains: bool,
        max_chars: usize,
        echo: bool,
        sent: Arc<Mutex<Vec<TranslationRequest>>>,
        answer: Result<String, TranslationError>,
    }

    impl FakeEngine {
        fn answering(answer: Result<&str, TranslationError>) -> Self {
            Self {
                html: true,
                retains: false,
                max_chars: usize::MAX,
                echo: false,
                sent: Arc::default(),
                answer: answer.map(str::to_owned),
            }
        }

        fn requests(&self) -> Vec<TranslationRequest> {
            self.sent.lock().unwrap().clone()
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
        fn fits(&self, request: &TranslationRequest) -> bool {
            request.content.as_str().chars().count() <= self.max_chars
        }
        fn may_retain_text(&self) -> bool {
            self.retains
        }
        async fn translate(
            &self,
            request: &TranslationRequest,
        ) -> Result<Translated, TranslationError> {
            self.sent.lock().unwrap().push(request.clone());
            let answer = if self.echo {
                Ok(request.content.as_str().replace("Hi", "Hallo"))
            } else {
                self.answer.clone()
            };
            answer.map(|content| Translated {
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
        save(&pool, false).await;
        pool
    }

    async fn save(pool: &SqlitePool, log_sent_text: bool) {
        TranslationRepository::new(pool)
            .save_settings(
                &crate::db::translation::TranslationSettingsRecord {
                    engine: Some("deepl".into()),
                    target_language: None,
                    log_sent_text,
                },
                "2026-11-06T00:00:00Z",
            )
            .await
            .unwrap();
    }

    async fn consent(pool: &SqlitePool) {
        TranslationRepository::new(pool)
            .set_consent("deepl", "acc", true, "2026-11-06T00:00:00Z")
            .await
            .unwrap();
    }

    async fn log_rows(pool: &SqlitePool) -> Vec<(String, i64, Option<String>)> {
        sqlx::query_as("SELECT outcome, units, sent_text FROM outbound_request_log ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn translate(
        pool: &SqlitePool,
        engine: &FakeEngine,
        target: &str,
    ) -> Result<MessageTranslation, TranslationError> {
        translate_message(pool, engine, "t1-m", target, chrono::Utc::now()).await
    }

    #[tokio::test]
    async fn nothing_leaves_without_consent() {
        let pool = setup(Some("<p>Hi</p>")).await;
        let engine = FakeEngine::answering(Ok("<p>Hallo</p>"));
        assert_eq!(
            translate(&pool, &engine, "DE").await.unwrap_err(),
            TranslationError::ConsentRequired
        );
        assert!(engine.requests().is_empty());
        assert!(log_rows(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn translates_html_sanitizes_the_answer_and_caches_it() {
        let pool = setup(Some("<p>Hi</p>")).await;
        consent(&pool).await;
        let engine = FakeEngine::answering(Ok(
            "<p onclick=\"steal()\">Hallo</p><script>alert(1)</script>",
        ));
        let translation = translate(&pool, &engine, "de").await.unwrap();
        assert!(!translation.cached);
        assert_eq!(translation.target_language, "DE");
        assert_eq!(translation.source_language.as_deref(), Some("EN"));
        let html = translation.body_html.unwrap();
        assert!(html.contains("Hallo"));
        assert!(!html.contains("onclick"));
        assert!(!html.contains("<script"));

        // Reopening is free: no second request, and offline works the same.
        let again = translate(&pool, &engine, "DE").await.unwrap();
        assert!(again.cached);
        assert_eq!(engine.requests().len(), 1);
        assert_eq!(log_rows(&pool).await, vec![("ok".into(), 42, None)]);
    }

    #[tokio::test]
    async fn urls_stay_on_the_device_and_come_back_in_the_translation() {
        let html = r#"<p>Hi <a href="https://click.example/r?u=42">there</a></p><img src="https://t.example/open.gif?u=42">"#;
        let pool = setup(Some(html)).await;
        consent(&pool).await;
        let mut engine = FakeEngine::answering(Ok(""));
        engine.echo = true;
        let translation = translate(&pool, &engine, "DE").await.unwrap();
        let sent = engine.requests()[0].content.as_str().to_owned();
        assert!(!sent.contains("example"), "a URL left: {sent}");
        let html = translation.body_html.unwrap();
        assert!(html.contains("Hallo"));
        assert!(html.contains("https://click.example/r?u=42"));
        assert!(html.contains("https://t.example/open.gif?u=42"));
    }

    #[tokio::test]
    async fn html_too_large_goes_as_text_and_text_too_large_does_not_go() {
        let pool = setup(Some(&format!("<p>{}</p>", "Hi ".repeat(200)))).await;
        consent(&pool).await;
        let mut engine = FakeEngine::answering(Ok("Hallo zusammen"));
        engine.max_chars = ENGLISH.chars().count();
        let translation = translate(&pool, &engine, "DE").await.unwrap();
        assert_eq!(translation.body_html, None);
        assert_eq!(translation.body_text.as_deref(), Some("Hallo zusammen"));
        assert_eq!(
            engine.requests()[0].content,
            TranslationContent::Text(ENGLISH.into())
        );

        let mut tight = FakeEngine::answering(Ok("x"));
        tight.max_chars = 10;
        assert_eq!(
            translate(&pool, &tight, "FR").await.unwrap_err(),
            TranslationError::TooLarge
        );
        assert!(tight.requests().is_empty());
        assert_eq!(log_rows(&pool).await.len(), 1, "the refusal is not logged");
    }

    #[tokio::test]
    async fn a_changed_body_makes_the_cached_translation_stale() {
        let pool = setup(None).await;
        consent(&pool).await;
        let engine = FakeEngine::answering(Ok("Hallo zusammen"));
        translate(&pool, &engine, "DE").await.unwrap();
        assert!(offer(&pool, "t1-m", "DE").await.unwrap().cached);
        sqlx::query("UPDATE messages SET body_text = 'Something else entirely' WHERE id = 't1-m'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!offer(&pool, "t1-m", "DE").await.unwrap().cached);
        translate(&pool, &engine, "DE").await.unwrap();
        assert_eq!(engine.requests().len(), 2);
    }

    #[tokio::test]
    async fn engines_without_html_get_the_text_body() {
        let pool = setup(Some("<p>Hi</p>")).await;
        consent(&pool).await;
        let mut engine = FakeEngine::answering(Ok("Hallo"));
        engine.html = false;
        let translation = translate(&pool, &engine, "DE").await.unwrap();
        assert_eq!(translation.body_html, None);
        assert_eq!(translation.body_text.as_deref(), Some("Hallo"));
        assert_eq!(
            engine.requests()[0].content,
            TranslationContent::Text(ENGLISH.into())
        );
    }

    #[tokio::test]
    async fn failures_are_logged_with_their_key_and_cache_nothing() {
        let pool = setup(None).await;
        consent(&pool).await;
        save(&pool, true).await;
        let engine = FakeEngine::answering(Err(TranslationError::QuotaExceeded));
        assert_eq!(
            translate(&pool, &engine, "DE").await.unwrap_err(),
            TranslationError::QuotaExceeded
        );
        assert_eq!(
            log_rows(&pool).await,
            vec![(
                "backend.errors.translation.quotaExceeded".into(),
                0,
                Some(ENGLISH.into())
            )]
        );
        assert!(!offer(&pool, "t1-m", "DE").await.unwrap().cached);
    }

    async fn assert_refused_as_encrypted(pool: &SqlitePool) {
        let engine = FakeEngine::answering(Ok("x"));
        assert_eq!(
            translate(pool, &engine, "DE").await.unwrap_err(),
            TranslationError::Encrypted
        );
        assert!(engine.requests().is_empty());
        let offer = offer(pool, "t1-m", "DE").await.unwrap();
        assert!(!offer.offered);
        assert_eq!(
            offer.refusal.as_deref(),
            Some("backend.errors.translation.encrypted")
        );
    }

    #[tokio::test]
    async fn inline_pgp_is_never_sent() {
        let pool = setup(None).await;
        consent(&pool).await;
        sqlx::query(
            "UPDATE messages SET body_text = '-----BEGIN PGP MESSAGE-----\nhQEMA...' WHERE id = 't1-m'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_refused_as_encrypted(&pool).await;
    }

    #[tokio::test]
    async fn smime_enveloped_mail_is_never_sent() {
        let pool = setup(None).await;
        consent(&pool).await;
        sqlx::query("UPDATE messages SET smime_encrypted = zeroblob(16) WHERE id = 't1-m'")
            .execute(&pool)
            .await
            .unwrap();
        assert_refused_as_encrypted(&pool).await;
    }

    #[tokio::test]
    async fn pgp_mime_containers_are_never_sent() {
        let pool = setup(None).await;
        consent(&pool).await;
        sqlx::query(
            "UPDATE messages SET raw_headers = 'Content-Type: Multipart/Encrypted; protocol=\"application/pgp-encrypted\"' WHERE id = 't1-m'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_refused_as_encrypted(&pool).await;
    }

    #[tokio::test]
    async fn gmail_mail_does_not_go_to_an_engine_that_keeps_text() {
        let pool = setup(None).await;
        consent(&pool).await;
        sqlx::query("UPDATE accounts SET provider = 'gmail' WHERE id = 'acc'")
            .execute(&pool)
            .await
            .unwrap();
        let mut free = FakeEngine::answering(Ok("Hallo"));
        free.retains = true;
        assert_eq!(
            translate(&pool, &free, "DE").await.unwrap_err(),
            TranslationError::GmailNeedsRetentionFreeEngine
        );
        assert!(free.requests().is_empty());

        crate::utils::crypto::seed_master_key_for_tests();
        let repo = TranslationRepository::new(&pool);
        repo.set_key("deepl", Some("k:fx"), Some("free"), "2026-11-06T00:00:00Z")
            .await
            .unwrap();
        let offer_free = offer(&pool, "t1-m", "DE").await.unwrap();
        assert!(!offer_free.offered);
        assert_eq!(
            offer_free.refusal.as_deref(),
            Some("backend.errors.translation.gmailNeedsPro")
        );
        repo.set_key("deepl", Some("k"), Some("pro"), "2026-11-06T00:00:00Z")
            .await
            .unwrap();
        assert!(offer(&pool, "t1-m", "DE").await.unwrap().offered);

        let pro = FakeEngine::answering(Ok("Hallo"));
        assert!(translate(&pool, &pro, "DE").await.is_ok());
    }

    #[tokio::test]
    async fn the_offer_skips_mail_already_in_the_target_language() {
        let pool = setup(None).await;
        let english = offer(&pool, "t1-m", "EN-GB").await.unwrap();
        assert_eq!(english.detected_language.as_deref(), Some("EN"));
        assert!(!english.offered);
        assert!(!english.consented);
        assert_eq!(english.refusal, None);
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
