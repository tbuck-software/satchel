//! DeepL API (`/v2/translate`, `/v2/usage`, `/v3/languages`).
//!
//! Free keys end in `:fx` and go to `api-free.deepl.com`, every other key to
//! `api.deepl.com`. The key travels in the `Authorization: DeepL-Auth-Key`
//! header, never in a URL or a form field. HTML is sent with
//! `tag_handling=html`, so DeepL translates the text and keeps the markup;
//! what comes back is sanitized again before anything renders it.
//!
//! Redirects are not followed: a 3xx from DeepL is an error, not a hint to
//! send the key somewhere else.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use super::{
    languages::deepl_fallback_languages, normalize_language, EngineKind, Locality, Translated,
    TranslationEngine, TranslationError, TranslationLanguage, TranslationRequest, TranslationUsage,
    UsageUnit,
};

const FREE_ENDPOINT: &str = "https://api-free.deepl.com";
const PRO_ENDPOINT: &str = "https://api.deepl.com";

/// DeepL's limit for a whole request body is 128 KiB. Leave room for the
/// JSON around the text, and refuse before sending rather than after.
pub const MAX_REQUEST_BYTES: usize = 128 * 1024 - 1024;

/// A translated newsletter is about as long as the original; markup can grow
/// a little. Anything far past the request limit is not a translation.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// A long HTML mail takes DeepL a while; the default 30 s is for small calls.
const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(60);

/// What `/v2/usage` reports when the account has no limit.
const NO_LIMIT: i64 = 1_000_000_000_000;

/// Which DeepL plan a key belongs to, read from the key itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeepLPlan {
    Free,
    Pro,
}

impl DeepLPlan {
    pub fn for_key(key: &str) -> Self {
        if key.trim().ends_with(":fx") {
            DeepLPlan::Free
        } else {
            DeepLPlan::Pro
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DeepLPlan::Free => "free",
            DeepLPlan::Pro => "pro",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "free" => Some(DeepLPlan::Free),
            "pro" => Some(DeepLPlan::Pro),
            _ => None,
        }
    }

    /// Whether mail may go through this plan at all. DeepL's Pro T&C
    /// (https://www.deepl.com/en/pro-license) say not to process personal
    /// data with the free API (8.3.11) and reserve the right to store what it
    /// is sent perpetually (3.3.2). Pro keeps texts only as long as the
    /// translation needs (3.1.2) and does not use them to improve the service
    /// (https://www.deepl.com/en/privacy, 4).
    pub fn accepts_personal_data(self) -> bool {
        matches!(self, DeepLPlan::Pro)
    }

    fn endpoint(self) -> &'static str {
        match self {
            DeepLPlan::Free => FREE_ENDPOINT,
            DeepLPlan::Pro => PRO_ENDPOINT,
        }
    }
}

/// A key worth sending: printable ASCII without spaces, of a sane length.
/// DeepL's keys are a UUID plus an optional `:fx`; this only refuses what
/// cannot be one (a pasted sentence, a newline, a multi-kilobyte blob).
pub fn validate_key(key: &str) -> Result<&str, TranslationError> {
    let key = key.trim();
    if key.is_empty() {
        return Err(TranslationError::KeyMissing);
    }
    if key.len() > 128 || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(TranslationError::KeyMalformed);
    }
    Ok(key)
}

pub struct DeepL {
    client: reqwest::Client,
    endpoint: &'static str,
    plan: DeepLPlan,
    key: String,
}

impl DeepL {
    pub fn new(key: &str) -> Result<Self, TranslationError> {
        let key = validate_key(key)?;
        let client = crate::utils::http::client_builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| TranslationError::Network)?;
        let plan = DeepLPlan::for_key(key);
        Ok(Self {
            client,
            endpoint: plan.endpoint(),
            plan,
            key: key.to_owned(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.endpoint)
    }

    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.header(
            reqwest::header::AUTHORIZATION,
            format!("DeepL-Auth-Key {}", self.key),
        )
    }

    async fn read(response: reqwest::Response) -> Result<Vec<u8>, TranslationError> {
        let status = response.status();
        if !status.is_success() {
            let trace = response
                .headers()
                .get("x-trace-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = crate::utils::public_http::read_limited(response, 16 * 1024)
                .await
                .unwrap_or_default();
            let error = map_status(status, &body);
            tracing::warn!(
                status = status.as_u16(),
                trace_id = trace.as_deref().unwrap_or(""),
                message = error_message(&body).as_deref().unwrap_or(""),
                "DeepL request failed"
            );
            return Err(error);
        }
        crate::utils::public_http::read_limited(response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|_| TranslationError::InvalidResponse)
    }
}

#[derive(Serialize)]
struct TranslateBody<'a> {
    text: [&'a str; 1],
    target_lang: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lang: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag_handling: Option<&'static str>,
    show_billed_characters: bool,
}

/// The JSON DeepL gets for `request`, or `TooLarge` when it would not fit
/// into one request.
pub fn translate_body(request: &TranslationRequest) -> Result<Vec<u8>, TranslationError> {
    let target = normalize_language(&request.target_language)
        .ok_or(TranslationError::UnsupportedLanguage)?;
    // DeepL names source languages without a region.
    let source = request
        .source_language
        .as_deref()
        .and_then(normalize_language)
        .map(|code| super::base_language(&code));
    let body = TranslateBody {
        text: [request.content.as_str()],
        target_lang: &target,
        source_lang: source.as_deref(),
        tag_handling: request.content.is_html().then_some("html"),
        show_billed_characters: true,
    };
    let bytes = serde_json::to_vec(&body).map_err(|_| TranslationError::InvalidResponse)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(TranslationError::TooLarge);
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct TranslateResponse {
    translations: Vec<TranslationItem>,
}

#[derive(Deserialize)]
struct TranslationItem {
    text: String,
    #[serde(default)]
    detected_source_language: Option<String>,
    #[serde(default)]
    billed_characters: Option<i64>,
}

pub fn parse_translate_response(body: &[u8]) -> Result<Translated, TranslationError> {
    let response: TranslateResponse =
        serde_json::from_slice(body).map_err(|_| TranslationError::InvalidResponse)?;
    let item = response
        .translations
        .into_iter()
        .next()
        .ok_or(TranslationError::InvalidResponse)?;
    Ok(Translated {
        content: item.text,
        detected_source_language: item
            .detected_source_language
            .as_deref()
            .and_then(normalize_language),
        billed_units: item.billed_characters,
        model: None,
    })
}

#[derive(Deserialize)]
struct UsageResponse {
    character_count: i64,
    character_limit: i64,
    /// Pro keys can carry a limit of their own, below the account's.
    #[serde(default)]
    api_key_character_count: Option<i64>,
    #[serde(default)]
    api_key_character_limit: Option<i64>,
}

pub fn parse_usage_response(body: &[u8]) -> Result<TranslationUsage, TranslationError> {
    let usage: UsageResponse =
        serde_json::from_slice(body).map_err(|_| TranslationError::InvalidResponse)?;
    let limited = |limit: i64| (limit > 0 && limit < NO_LIMIT).then_some(limit);
    // The tighter of the two limits is the one a translation runs into.
    let key_limit = usage.api_key_character_limit.and_then(limited);
    let account_limit = limited(usage.character_limit);
    let (used, limit) = match (key_limit, account_limit) {
        (Some(key), Some(account)) if key < account => (
            usage
                .api_key_character_count
                .unwrap_or(usage.character_count),
            Some(key),
        ),
        (Some(key), None) => (
            usage
                .api_key_character_count
                .unwrap_or(usage.character_count),
            Some(key),
        ),
        _ => (usage.character_count, account_limit),
    };
    Ok(TranslationUsage {
        used: used.max(0),
        limit,
        unit: UsageUnit::Characters,
    })
}

#[derive(Deserialize)]
struct LanguageItem {
    lang: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    usable_as_target: bool,
}

pub fn parse_languages_response(body: &[u8]) -> Result<Vec<TranslationLanguage>, TranslationError> {
    let items: Vec<LanguageItem> =
        serde_json::from_slice(body).map_err(|_| TranslationError::InvalidResponse)?;
    let mut languages: Vec<TranslationLanguage> = items
        .into_iter()
        .filter(|item| item.usable_as_target)
        .filter_map(|item| {
            Some(TranslationLanguage {
                code: normalize_language(&item.lang)?,
                name: item.name.filter(|name| !name.trim().is_empty()),
            })
        })
        .collect();
    languages.sort_by(|a, b| a.code.cmp(&b.code));
    languages.dedup_by(|a, b| a.code == b.code);
    if languages.is_empty() {
        return Err(TranslationError::InvalidResponse);
    }
    Ok(languages)
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error: Option<NestedError>,
}

#[derive(Deserialize)]
struct NestedError {
    #[serde(default)]
    message: Option<String>,
}

/// DeepL's `message`, or the edge's `error.message`. For the log only: it
/// is English and DeepL's to word, so it never reaches the person.
fn error_message(body: &[u8]) -> Option<String> {
    let body: ErrorBody = serde_json::from_slice(body).ok()?;
    body.message.or(body.error.and_then(|error| error.message))
}

pub fn map_status(status: StatusCode, body: &[u8]) -> TranslationError {
    match status.as_u16() {
        401 | 403 => TranslationError::InvalidKey,
        456 => TranslationError::QuotaExceeded,
        429 | 529 => TranslationError::RateLimited,
        413 | 414 => TranslationError::TooLarge,
        400 => {
            let message = error_message(body).unwrap_or_default().to_ascii_lowercase();
            if message.contains("target_lang") || message.contains("source_lang") {
                TranslationError::UnsupportedLanguage
            } else {
                TranslationError::Rejected
            }
        }
        500..=599 => TranslationError::Unavailable,
        _ => TranslationError::Rejected,
    }
}

fn network(error: reqwest::Error) -> TranslationError {
    tracing::warn!(error = %error, "DeepL request did not complete");
    TranslationError::Network
}

#[async_trait]
impl TranslationEngine for DeepL {
    fn kind(&self) -> EngineKind {
        EngineKind::DeepL
    }

    fn locality(&self) -> Locality {
        Locality::Remote
    }

    fn accepts_html(&self) -> bool {
        true
    }

    fn billing_unit(&self) -> UsageUnit {
        UsageUnit::Characters
    }

    fn fits(&self, request: &TranslationRequest) -> bool {
        !matches!(translate_body(request), Err(TranslationError::TooLarge))
    }

    fn accepts_personal_data(&self) -> bool {
        self.plan.accepts_personal_data()
    }

    async fn translate(
        &self,
        request: &TranslationRequest,
    ) -> Result<Translated, TranslationError> {
        let body = translate_body(request)?;
        let response = self
            .authorized(self.client.post(self.url("/v2/translate")))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(TRANSLATE_TIMEOUT)
            .body(body)
            .send()
            .await
            .map_err(network)?;
        parse_translate_response(&Self::read(response).await?)
    }

    async fn languages(&self) -> Result<Vec<TranslationLanguage>, TranslationError> {
        let response = self
            .authorized(
                self.client
                    .get(self.url("/v3/languages?resource=translate_text")),
            )
            .send()
            .await
            .map_err(network)?;
        match Self::read(response).await {
            Ok(body) => parse_languages_response(&body),
            // The key is what the person has to fix; say so.
            Err(TranslationError::InvalidKey) => Err(TranslationError::InvalidKey),
            // `/v3/languages` is newer than `/v2`; an account or proxy that
            // does not know it still translates fine with the known list.
            Err(_) => Ok(deepl_fallback_languages()),
        }
    }

    async fn usage(&self) -> Result<TranslationUsage, TranslationError> {
        let response = self
            .authorized(self.client.get(self.url("/v2/usage")))
            .send()
            .await
            .map_err(network)?;
        parse_usage_response(&Self::read(response).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::TranslationContent;

    fn request(content: TranslationContent) -> TranslationRequest {
        TranslationRequest {
            content,
            source_language: None,
            target_language: "de".into(),
        }
    }

    #[test]
    fn free_keys_go_to_the_free_endpoint() {
        assert_eq!(DeepLPlan::for_key("abc-123:fx"), DeepLPlan::Free);
        assert_eq!(DeepLPlan::for_key(" abc-123:fx\n"), DeepLPlan::Free);
        assert_eq!(DeepLPlan::for_key("abc-123"), DeepLPlan::Pro);
        assert_eq!(
            DeepL::new("abc-123:fx").unwrap().endpoint,
            "https://api-free.deepl.com"
        );
        assert_eq!(
            DeepL::new("abc-123").unwrap().endpoint,
            "https://api.deepl.com"
        );
    }

    #[test]
    fn malformed_keys_are_refused_before_any_request() {
        assert_eq!(
            validate_key("  ").unwrap_err(),
            TranslationError::KeyMissing
        );
        assert_eq!(
            validate_key("two words").unwrap_err(),
            TranslationError::KeyMalformed
        );
        assert_eq!(
            validate_key(&"x".repeat(200)).unwrap_err(),
            TranslationError::KeyMalformed
        );
        assert_eq!(validate_key(" k:fx ").unwrap(), "k:fx");
    }

    #[test]
    fn html_asks_for_tag_handling_and_text_does_not() {
        let html: serde_json::Value = serde_json::from_slice(
            &translate_body(&request(TranslationContent::Html("<p>Hi</p>".into()))).unwrap(),
        )
        .unwrap();
        assert_eq!(html["tag_handling"], "html");
        assert_eq!(html["text"][0], "<p>Hi</p>");
        assert_eq!(html["target_lang"], "DE");
        assert!(html.get("source_lang").is_none());

        let mut text_request = request(TranslationContent::Text("Hi".into()));
        text_request.source_language = Some("en-gb".into());
        let text: serde_json::Value =
            serde_json::from_slice(&translate_body(&text_request).unwrap()).unwrap();
        assert!(text.get("tag_handling").is_none());
        assert_eq!(text["source_lang"], "EN");
    }

    #[test]
    fn oversized_bodies_are_refused_before_sending() {
        let long = "a".repeat(MAX_REQUEST_BYTES);
        assert_eq!(
            translate_body(&request(TranslationContent::Text(long))).unwrap_err(),
            TranslationError::TooLarge
        );
    }

    #[test]
    fn bad_target_languages_are_refused() {
        let mut bad = request(TranslationContent::Text("Hi".into()));
        bad.target_language = "german".into();
        assert_eq!(
            translate_body(&bad).unwrap_err(),
            TranslationError::UnsupportedLanguage
        );
    }

    #[test]
    fn reads_a_translation() {
        let translated = parse_translate_response(
            br#"{"translations":[{"detected_source_language":"EN","text":"<p>Hallo</p>","billed_characters":12}]}"#,
        )
        .unwrap();
        assert_eq!(translated.content, "<p>Hallo</p>");
        assert_eq!(translated.detected_source_language.as_deref(), Some("EN"));
        assert_eq!(translated.billed_units, Some(12));
        assert_eq!(
            parse_translate_response(br#"{"translations":[]}"#).unwrap_err(),
            TranslationError::InvalidResponse
        );
        assert_eq!(
            parse_translate_response(b"<html>").unwrap_err(),
            TranslationError::InvalidResponse
        );
    }

    #[test]
    fn reads_usage_and_treats_the_sentinel_as_no_limit() {
        let free =
            parse_usage_response(br#"{"character_count":1200,"character_limit":500000}"#).unwrap();
        assert_eq!((free.used, free.limit), (1200, Some(500_000)));
        let unlimited =
            parse_usage_response(br#"{"character_count":5,"character_limit":1000000000000}"#)
                .unwrap();
        assert_eq!(unlimited.limit, None);
        let key_capped = parse_usage_response(
            br#"{"character_count":900000,"character_limit":1000000000000,
                 "api_key_character_count":300,"api_key_character_limit":10000}"#,
        )
        .unwrap();
        assert_eq!((key_capped.used, key_capped.limit), (300, Some(10_000)));
    }

    #[test]
    fn reads_target_languages_from_v3() {
        let languages = parse_languages_response(
            br#"[{"lang":"de","name":"German","usable_as_source":true,"usable_as_target":true},
                 {"lang":"en","name":"English","usable_as_source":true,"usable_as_target":false},
                 {"lang":"en-US","name":"English (American)","usable_as_source":false,"usable_as_target":true},
                 {"lang":"zh-Hans","name":"","usable_as_target":true}]"#,
        )
        .unwrap();
        let codes: Vec<_> = languages.iter().map(|l| l.code.as_str()).collect();
        assert_eq!(codes, ["DE", "EN-US", "ZH-HANS"]);
        assert_eq!(languages[2].name, None);
    }

    #[test]
    fn statuses_map_to_what_the_person_can_do() {
        assert_eq!(
            map_status(StatusCode::FORBIDDEN, b""),
            TranslationError::InvalidKey
        );
        assert_eq!(
            map_status(StatusCode::from_u16(456).unwrap(), b""),
            TranslationError::QuotaExceeded
        );
        assert_eq!(
            map_status(StatusCode::TOO_MANY_REQUESTS, b""),
            TranslationError::RateLimited
        );
        assert_eq!(
            map_status(StatusCode::from_u16(529).unwrap(), b""),
            TranslationError::RateLimited
        );
        assert_eq!(
            map_status(StatusCode::PAYLOAD_TOO_LARGE, b""),
            TranslationError::TooLarge
        );
        assert_eq!(
            map_status(
                StatusCode::BAD_REQUEST,
                br#"{"message":"Value for 'target_lang' not supported."}"#
            ),
            TranslationError::UnsupportedLanguage
        );
        assert_eq!(
            map_status(StatusCode::BAD_REQUEST, br#"{"error":{"message":"bad"}}"#),
            TranslationError::Rejected
        );
        assert_eq!(
            map_status(StatusCode::SERVICE_UNAVAILABLE, b""),
            TranslationError::Unavailable
        );
        assert_eq!(
            map_status(StatusCode::MOVED_PERMANENTLY, b""),
            TranslationError::Rejected
        );
    }
}
