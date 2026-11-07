//! Translating mail through an outside engine.
//!
//! [`TranslationEngine`] is the seam: DeepL ([`deepl`]) is the one engine
//! today, and an AI provider (ADR 0016) plugs in behind the same trait later.
//! [`service`] holds what every engine shares and no engine may skip: the
//! consent check, the local cache, the second pass through the sanitizer and
//! the request log.
//!
//! Language codes are DeepL's, upper case: `DE`, `EN-GB`, `PT-BR`, `ZH-HANS`.
//! A region or script variant only matters as a target; comparisons go
//! through [`base_language`].

pub mod deepl;
pub mod languages;
pub mod service;

use async_trait::async_trait;
use serde::Serialize;

/// The engines Satchel knows. Stored by [`EngineKind::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKind {
    DeepL,
}

impl EngineKind {
    pub const ALL: [EngineKind; 1] = [EngineKind::DeepL];

    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::DeepL => "deepl",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value.trim())
    }
}

/// Where the text goes. Informational for now: the "local models only"
/// switch of ADR 0016 that will admit only `Local` does not exist yet, and
/// nothing checks this value today. DeepL is never local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locality {
    Local,
    Remote,
}

/// What an engine is asked to translate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationContent {
    Text(String),
    /// Already sanitized mail HTML. Only for engines that say
    /// [`TranslationEngine::accepts_html`]; the result is sanitized again.
    Html(String),
}

impl TranslationContent {
    pub fn as_str(&self) -> &str {
        match self {
            TranslationContent::Text(text) | TranslationContent::Html(text) => text,
        }
    }

    pub fn is_html(&self) -> bool {
        matches!(self, TranslationContent::Html(_))
    }

    /// Characters as engines bill them: Unicode code points.
    pub fn characters(&self) -> i64 {
        i64::try_from(self.as_str().chars().count()).unwrap_or(i64::MAX)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationRequest {
    pub content: TranslationContent,
    /// `None`: the engine detects it.
    pub source_language: Option<String>,
    pub target_language: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translated {
    /// Same kind as the request's content: HTML in, HTML out.
    pub content: String,
    /// What the engine says the text was in. Authoritative over local
    /// detection.
    pub detected_source_language: Option<String>,
    /// What the engine billed, in [`TranslationEngine::billing_unit`].
    pub billed_units: Option<i64>,
    /// The model the engine reports, for engines that have one.
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/translation/"))]
pub enum UsageUnit {
    Characters,
    Tokens,
}

impl UsageUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            UsageUnit::Characters => "characters",
            UsageUnit::Tokens => "tokens",
        }
    }
}

/// How much of the account's allowance this billing period has used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/translation/"))]
pub struct TranslationUsage {
    pub used: i64,
    /// `None`: no limit is set.
    pub limit: Option<i64>,
    pub unit: UsageUnit,
}

/// A language an engine translates into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "ipc-types", ts(export, export_to = "core/translation/"))]
pub struct TranslationLanguage {
    pub code: String,
    /// The engine's own (English) name; the webview prefers the name its
    /// locale gives the code.
    pub name: Option<String>,
}

/// Why a translation did not happen. Each maps to one message key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranslationError {
    #[error("no translation engine is set up")]
    NotConfigured,
    #[error("the engine has no API key")]
    KeyMissing,
    #[error("the API key is malformed")]
    KeyMalformed,
    #[error("the engine rejected the API key")]
    InvalidKey,
    #[error("the engine's usage limit is reached")]
    QuotaExceeded,
    #[error("the engine asks to slow down")]
    RateLimited,
    #[error("the text is too long for one request")]
    TooLarge,
    #[error("the engine does not translate into this language")]
    UnsupportedLanguage,
    #[error("the engine refused the request")]
    Rejected,
    #[error("the engine is unavailable")]
    Unavailable,
    #[error("the engine could not be reached")]
    Network,
    #[error("the engine's answer could not be read")]
    InvalidResponse,
    #[error("this account's mail may not go to this engine")]
    ConsentRequired,
    #[error("the message is not stored on this device")]
    MessageNotFound,
    #[error("the message has no text to translate")]
    NothingToTranslate,
    #[error("encrypted mail is not sent for translation")]
    Encrypted,
    #[error("the engine's plan does not allow personal data")]
    PlanNotForMail,
    #[error("the credential key is unavailable")]
    CredentialsUnavailable,
    #[error("translation storage failed: {0}")]
    Persistence(String),
}

impl TranslationError {
    /// The message key the webview shows for this error.
    pub fn message_key(&self) -> &'static str {
        match self {
            TranslationError::NotConfigured => "backend.errors.translation.notConfigured",
            TranslationError::KeyMissing => "backend.errors.translation.keyMissing",
            TranslationError::KeyMalformed => "backend.errors.translation.keyMalformed",
            TranslationError::InvalidKey => "backend.errors.translation.invalidKey",
            TranslationError::QuotaExceeded => "backend.errors.translation.quotaExceeded",
            TranslationError::RateLimited => "backend.errors.translation.rateLimited",
            TranslationError::TooLarge => "backend.errors.translation.tooLarge",
            TranslationError::UnsupportedLanguage => {
                "backend.errors.translation.unsupportedLanguage"
            }
            TranslationError::Rejected => "backend.errors.translation.rejected",
            TranslationError::Unavailable => "backend.errors.translation.unavailable",
            TranslationError::Network => "backend.errors.translation.network",
            TranslationError::InvalidResponse => "backend.errors.translation.invalidResponse",
            TranslationError::ConsentRequired => "backend.errors.translation.consentRequired",
            TranslationError::MessageNotFound => "backend.errors.translation.messageNotFound",
            TranslationError::NothingToTranslate => "backend.errors.translation.nothingToTranslate",
            TranslationError::Encrypted => "backend.errors.translation.encrypted",
            TranslationError::PlanNotForMail => "backend.errors.translation.freePlanNotForMail",
            TranslationError::CredentialsUnavailable => {
                "backend.errors.translation.credentialsUnavailable"
            }
            TranslationError::Persistence(_) => "backend.errors.translation.persistenceFailed",
        }
    }
}

impl From<sqlx::Error> for TranslationError {
    fn from(error: sqlx::Error) -> Self {
        TranslationError::Persistence(error.to_string())
    }
}

/// One translation engine. Implementations talk to their service and map
/// its answers; consent, caching, sanitizing and logging live in
/// [`service`], so a new engine cannot forget them.
#[async_trait]
pub trait TranslationEngine: Send + Sync {
    fn kind(&self) -> EngineKind;

    fn locality(&self) -> Locality;

    /// Whether the engine keeps markup intact. One that does not (a language
    /// model, which must see mail as plain text) gets the text body.
    fn accepts_html(&self) -> bool;

    fn billing_unit(&self) -> UsageUnit;

    /// Whether `request` fits into one request to this engine. Checked
    /// before sending, so a body that is too large never leaves the device.
    fn fits(&self, request: &TranslationRequest) -> bool;

    /// Whether the engine's terms allow personal data, which every mail is.
    /// DeepL API Free does not (Pro T&C 8.3.11); with it, no mail is sent
    /// (ADR 0018).
    fn accepts_personal_data(&self) -> bool;

    async fn translate(&self, request: &TranslationRequest)
        -> Result<Translated, TranslationError>;

    async fn languages(&self) -> Result<Vec<TranslationLanguage>, TranslationError>;

    async fn usage(&self) -> Result<TranslationUsage, TranslationError>;
}

/// `EN-GB` → `EN`, `zh-hans` → `ZH`.
pub fn base_language(code: &str) -> String {
    code.trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
}

/// A language code in the engines' shape, or `None` when it is not one:
/// two or three letters, optionally a region or script of two to four.
pub fn normalize_language(code: &str) -> Option<String> {
    let code = code.trim().replace('_', "-").to_ascii_uppercase();
    let mut parts = code.split('-');
    let primary = parts.next()?;
    if !(2..=3).contains(&primary.len()) || !primary.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    if let Some(variant) = parts.next() {
        if !(2..=4).contains(&variant.len()) || !variant.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(code)
}

/// The same language, ignoring region and script: no point translating
/// British English into American English.
pub fn same_language(a: &str, b: &str) -> bool {
    let (a, b) = (base_language(a), base_language(b));
    !a.is_empty() && a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_normalized_to_the_engine_shape() {
        assert_eq!(normalize_language("de").as_deref(), Some("DE"));
        assert_eq!(normalize_language(" en-gb ").as_deref(), Some("EN-GB"));
        assert_eq!(normalize_language("zh_Hans").as_deref(), Some("ZH-HANS"));
        assert_eq!(normalize_language(""), None);
        assert_eq!(normalize_language("english"), None);
        assert_eq!(normalize_language("en-gb-x"), None);
        assert_eq!(normalize_language("e1"), None);
    }

    #[test]
    fn variants_count_as_the_same_language() {
        assert!(same_language("EN-GB", "en"));
        assert!(same_language("PT-BR", "PT-PT"));
        assert!(!same_language("DE", "EN-US"));
        assert!(!same_language("", ""));
    }

    #[test]
    fn engine_ids_round_trip() {
        for kind in EngineKind::ALL {
            assert_eq!(EngineKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EngineKind::parse("google"), None);
    }

    #[test]
    fn every_error_has_a_translation_key() {
        for error in [
            TranslationError::NotConfigured,
            TranslationError::InvalidKey,
            TranslationError::QuotaExceeded,
            TranslationError::Network,
            TranslationError::Persistence("x".into()),
        ] {
            assert!(error
                .message_key()
                .starts_with("backend.errors.translation."));
        }
    }
}
