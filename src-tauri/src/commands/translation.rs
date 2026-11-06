//! Translating mail: the settings page's engine, key, target language and
//! consents, the reader's Translate action, and the request log.
//!
//! The key goes in and never comes back: the webview learns whether one is
//! stored and which DeepL plan it belongs to, nothing more.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::commands::error::{CommandError, CommandResult};
use satchel_core::db::translation::{
    RequestLogEntry, TranslationConsentRecord, TranslationRepository, TranslationSettingsRecord,
    REQUEST_LOG_PAGE,
};
use satchel_core::translation::{
    deepl::{validate_key, DeepLPlan},
    normalize_language,
    service::{self, MessageTranslation, MessageTranslationOffer},
    EngineKind, Locality, TranslationError, TranslationLanguage, TranslationUsage,
};

impl From<TranslationError> for CommandError {
    fn from(error: TranslationError) -> Self {
        if let TranslationError::Persistence(detail) = &error {
            tracing::error!(error = %detail, "translation storage failed");
        }
        Self::new(error.message_key())
    }
}

fn persistence(error: sqlx::Error) -> CommandError {
    TranslationError::from(error).into()
}

fn engine_kind(value: &str) -> CommandResult<EngineKind> {
    EngineKind::parse(value)
        .ok_or_else(|| CommandError::new("backend.errors.translation.engineUnknown"))
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/translation/")
)]
pub struct TranslationEngineDto {
    /// `deepl`.
    pub id: String,
    pub has_key: bool,
    /// DeepL: `free` or `pro`, read from the stored key's suffix.
    pub plan: Option<String>,
    /// False for every engine that sends text off the device.
    pub local: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/translation/")
)]
pub struct TranslationSettingsDto {
    /// The engine translations go to; `None` while translation is off.
    pub engine: Option<String>,
    /// `None`: the webview's UI language.
    pub target_language: Option<String>,
    pub log_sent_text: bool,
    pub engines: Vec<TranslationEngineDto>,
    pub consents: Vec<TranslationConsentRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/translation/", optional_fields = nullable)
)]
pub struct SaveTranslationSettingsInput {
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub target_language: Option<String>,
    #[serde(default)]
    pub log_sent_text: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/translation/", optional_fields = nullable)
)]
pub struct SetTranslationApiKeyInput {
    pub engine: String,
    /// `None` or blank removes the stored key.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/translation/")
)]
pub struct SetTranslationConsentInput {
    pub engine: String,
    pub account_id: String,
    pub granted: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/translation/")
)]
pub struct MessageTranslationInput {
    pub message_id: String,
    /// The language to translate into; the webview resolves "follow the UI".
    pub target_language: String,
}

pub(crate) async fn settings_dto(pool: &SqlitePool) -> CommandResult<TranslationSettingsDto> {
    let repo = TranslationRepository::new(pool);
    let settings = repo.settings().await.map_err(persistence)?;
    let mut engines = Vec::new();
    for kind in EngineKind::ALL {
        let has_key = repo.has_key(kind.as_str()).await.map_err(persistence)?;
        let plan = match (kind, has_key) {
            (EngineKind::DeepL, true) => repo
                .key(kind.as_str())
                .await
                .ok()
                .flatten()
                .map(|key| DeepLPlan::for_key(&key).as_str().to_owned()),
            _ => None,
        };
        let local = match kind {
            EngineKind::DeepL => Locality::Remote,
        } == Locality::Local;
        engines.push(TranslationEngineDto {
            id: kind.as_str().to_owned(),
            has_key,
            plan,
            local,
        });
    }
    Ok(TranslationSettingsDto {
        engine: settings.engine,
        target_language: settings.target_language,
        log_sent_text: settings.log_sent_text,
        engines,
        consents: repo.consents().await.map_err(persistence)?,
    })
}

#[tauri::command]
pub async fn get_translation_settings(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<TranslationSettingsDto> {
    settings_dto(pool.inner().as_ref()).await
}

pub(crate) async fn save_translation_settings_inner(
    pool: &SqlitePool,
    input: SaveTranslationSettingsInput,
) -> CommandResult<TranslationSettingsDto> {
    let engine = match input.engine.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(engine) => Some(engine_kind(engine)?.as_str().to_owned()),
    };
    let target_language = match input.target_language.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(code) => Some(normalize_language(code).ok_or(TranslationError::UnsupportedLanguage)?),
    };
    TranslationRepository::new(pool)
        .save_settings(
            &TranslationSettingsRecord {
                engine,
                target_language,
                log_sent_text: input.log_sent_text,
            },
            &chrono::Utc::now().to_rfc3339(),
        )
        .await
        .map_err(persistence)?;
    settings_dto(pool).await
}

#[tauri::command]
pub async fn save_translation_settings(
    pool: tauri::State<'_, SqlitePool>,
    input: SaveTranslationSettingsInput,
) -> CommandResult<TranslationSettingsDto> {
    save_translation_settings_inner(pool.inner(), input).await
}

pub(crate) async fn set_translation_api_key_inner(
    pool: &SqlitePool,
    input: SetTranslationApiKeyInput,
) -> CommandResult<TranslationSettingsDto> {
    let kind = engine_kind(&input.engine)?;
    let key = match input.api_key.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(key) => Some(validate_key(key)?),
    };
    TranslationRepository::new(pool)
        .set_key(kind.as_str(), key, &chrono::Utc::now().to_rfc3339())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "translation key not saved");
            CommandError::new(TranslationError::CredentialsUnavailable.message_key())
        })?;
    settings_dto(pool).await
}

#[tauri::command]
pub async fn set_translation_api_key(
    pool: tauri::State<'_, SqlitePool>,
    input: SetTranslationApiKeyInput,
) -> CommandResult<TranslationSettingsDto> {
    set_translation_api_key_inner(pool.inner(), input).await
}

/// The settings page's Test button: one cheap authenticated request that
/// proves the key and shows what is left of the allowance.
#[tauri::command]
pub async fn check_translation_engine(
    pool: tauri::State<'_, crate::db::ReadPool>,
    engine: String,
) -> CommandResult<TranslationUsage> {
    let engine = service::engine_for(pool.inner().as_ref(), engine_kind(&engine)?).await?;
    Ok(engine.usage().await?)
}

/// The languages the engine translates into. Without a key, or offline, the
/// list the engine is known to support.
#[tauri::command]
pub async fn list_translation_languages(
    pool: tauri::State<'_, crate::db::ReadPool>,
    engine: String,
) -> CommandResult<Vec<TranslationLanguage>> {
    let kind = engine_kind(&engine)?;
    let fallback = || match kind {
        EngineKind::DeepL => satchel_core::translation::languages::deepl_fallback_languages(),
    };
    match service::engine_for(pool.inner().as_ref(), kind).await {
        Ok(engine) => match engine.languages().await {
            Ok(languages) => Ok(languages),
            Err(TranslationError::InvalidKey) => Err(TranslationError::InvalidKey.into()),
            Err(_) => Ok(fallback()),
        },
        Err(_) => Ok(fallback()),
    }
}

pub(crate) async fn set_translation_consent_inner(
    pool: &SqlitePool,
    input: SetTranslationConsentInput,
) -> CommandResult<TranslationSettingsDto> {
    let kind = engine_kind(&input.engine)?;
    let known = TranslationRepository::new(pool)
        .set_consent(
            kind.as_str(),
            input.account_id.trim(),
            input.granted,
            &chrono::Utc::now().to_rfc3339(),
        )
        .await
        .map_err(persistence)?;
    if !known {
        return Err(CommandError::new("backend.errors.accountNotFound"));
    }
    settings_dto(pool).await
}

#[tauri::command]
pub async fn set_translation_consent(
    pool: tauri::State<'_, SqlitePool>,
    input: SetTranslationConsentInput,
) -> CommandResult<TranslationSettingsDto> {
    set_translation_consent_inner(pool.inner(), input).await
}

/// Whether the reader should offer Translate for a message. Local only.
#[tauri::command]
pub async fn message_translation_offer(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: MessageTranslationInput,
) -> CommandResult<MessageTranslationOffer> {
    Ok(service::offer(
        pool.inner().as_ref(),
        input.message_id.trim(),
        &input.target_language,
    )
    .await?)
}

pub(crate) async fn translate_message_inner(
    pool: &SqlitePool,
    input: MessageTranslationInput,
) -> CommandResult<MessageTranslation> {
    let message_id = input.message_id.trim();
    // A stored translation needs no engine, no key and no network.
    if let Some(cached) =
        service::cached_translation(pool, message_id, &input.target_language).await?
    {
        return Ok(cached);
    }
    let engine = service::engine_for(pool, service::active_engine(pool).await?).await?;
    Ok(service::translate_message(
        pool,
        engine.as_ref(),
        message_id,
        &input.target_language,
        chrono::Utc::now(),
    )
    .await?)
}

#[tauri::command]
pub async fn translate_message(
    pool: tauri::State<'_, SqlitePool>,
    input: MessageTranslationInput,
) -> CommandResult<MessageTranslation> {
    translate_message_inner(pool.inner(), input).await
}

/// What left the device in the last 30 days, newest first.
#[tauri::command]
pub async fn list_outbound_requests(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<Vec<RequestLogEntry>> {
    TranslationRepository::new(pool.inner().as_ref())
        .request_log(REQUEST_LOG_PAGE)
        .await
        .map_err(persistence)
}

#[tauri::command]
pub async fn clear_outbound_requests(pool: tauri::State<'_, SqlitePool>) -> CommandResult<()> {
    TranslationRepository::new(pool.inner())
        .clear_request_log()
        .await
        .map_err(persistence)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        satchel_core::utils::crypto::seed_master_key_for_tests();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::init(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn the_key_never_comes_back() {
        let pool = pool().await;
        let dto = set_translation_api_key_inner(
            &pool,
            SetTranslationApiKeyInput {
                engine: "deepl".into(),
                api_key: Some(" 0000-secret:fx ".into()),
            },
        )
        .await
        .unwrap();
        let wire = serde_json::to_string(&dto).unwrap();
        assert!(!wire.contains("secret"));
        assert!(dto.engines[0].has_key);
        assert_eq!(dto.engines[0].plan.as_deref(), Some("free"));
        assert!(!dto.engines[0].local);

        let cleared = set_translation_api_key_inner(
            &pool,
            SetTranslationApiKeyInput {
                engine: "deepl".into(),
                api_key: Some("  ".into()),
            },
        )
        .await
        .unwrap();
        assert!(!cleared.engines[0].has_key);
    }

    #[tokio::test]
    async fn settings_refuse_unknown_engines_and_languages() {
        let pool = pool().await;
        let engine = save_translation_settings_inner(
            &pool,
            SaveTranslationSettingsInput {
                engine: Some("babelfish".into()),
                target_language: None,
                log_sent_text: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(engine.key, "backend.errors.translation.engineUnknown");
        let language = save_translation_settings_inner(
            &pool,
            SaveTranslationSettingsInput {
                engine: Some("deepl".into()),
                target_language: Some("Klingon".into()),
                log_sent_text: false,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            language.key,
            "backend.errors.translation.unsupportedLanguage"
        );
        let saved = save_translation_settings_inner(
            &pool,
            SaveTranslationSettingsInput {
                engine: Some("deepl".into()),
                target_language: Some("en-gb".into()),
                log_sent_text: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(saved.engine.as_deref(), Some("deepl"));
        assert_eq!(saved.target_language.as_deref(), Some("EN-GB"));
    }

    #[tokio::test]
    async fn translating_without_an_engine_says_so() {
        let pool = pool().await;
        let error = translate_message_inner(
            &pool,
            MessageTranslationInput {
                message_id: "missing".into(),
                target_language: "DE".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.key, "backend.errors.translation.notConfigured");
    }
}
