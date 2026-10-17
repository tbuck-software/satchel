use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    commands::aliases::alias_error_key,
    commands::error::{CommandError, CommandResult},
    email::sanitizer::sanitize_email_html,
    providers::traits::OutgoingAttachment,
};
use satchel_core::db::{
    accounts::AccountRepository,
    aliases::AliasRepository,
    messages::{SignatureMessageError, SignatureMessageRepository},
    signatures::{
        FirstContactMode, ResolvedSignatureRecord, SignatureAssetRecord, SignatureAssetWrite,
        SignatureAssignmentRecord, SignatureAssignmentUpsertRequest, SignatureError,
        SignaturePreference as CoreSignaturePreference, SignatureRecord, SignatureRepository,
        SignatureResolveRequest, SignatureSuggestionRecord, SignatureSuggestionStatus,
        SignatureSuggestionUpsertRequest, SignatureUpsertRequest,
        SignatureVariant as CoreSignatureVariant,
    },
};

const MAX_ASSET_BYTES: usize = 5 * 1024 * 1024;
const MAX_SIGNATURE_ASSET_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureAssetInput {
    pub filename: String,
    pub mime_type: String,
    pub content_id: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureAssetDto {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub content_id: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureInput {
    pub id: Option<String>,
    pub account_id: String,
    pub name: String,
    #[serde(default)]
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub short_body_text: String,
    #[serde(default)]
    pub short_body_html: Option<String>,
    #[serde(default)]
    pub assets: Vec<SignatureAssetInput>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureDto {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub short_body_text: String,
    pub short_body_html: Option<String>,
    pub assets: Vec<SignatureAssetDto>,
    pub is_default: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl From<SignatureAssetRecord> for SignatureAssetDto {
    fn from(record: SignatureAssetRecord) -> Self {
        Self {
            id: record.id,
            filename: record.filename,
            mime_type: record.mime_type,
            content_id: record.content_id,
            content_base64: base64::engine::general_purpose::STANDARD.encode(record.content),
        }
    }
}

impl From<SignatureRecord> for SignatureDto {
    fn from(record: SignatureRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            name: record.name,
            body_text: record.body_text,
            body_html: record.body_html,
            short_body_text: record.short_body_text,
            short_body_html: record.short_body_html,
            assets: record.assets.into_iter().map(Into::into).collect(),
            is_default: record.is_default,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub enum SignaturePreference {
    Auto,
    Long,
    Short,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub enum SignatureVariant {
    Long,
    Short,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct ResolveSignatureInput {
    pub account_id: String,
    pub identity_email: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub signature_id: Option<String>,
    pub preference: SignaturePreference,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct ResolvedSignature {
    pub signature_id: String,
    pub variant: SignatureVariant,
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<OutgoingAttachment>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureAssignmentInput {
    pub account_id: String,
    pub identity_email: String,
    pub signature_id: Option<String>,
    #[serde(default = "default_first_contact_mode")]
    pub first_contact_mode: String,
}

fn default_first_contact_mode() -> String {
    "always_full".into()
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureAssignmentDto {
    pub account_id: String,
    pub identity_email: String,
    pub signature_id: Option<String>,
    pub first_contact_mode: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<SignatureAssignmentRecord> for SignatureAssignmentDto {
    fn from(record: SignatureAssignmentRecord) -> Self {
        Self {
            account_id: record.account_id,
            identity_email: record.identity_email,
            signature_id: record.signature_id,
            first_contact_mode: record.first_contact_mode,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureDetectionMessage {
    pub thread_id: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
}

impl SignatureDetectionMessage {
    pub fn text(thread_id: impl Into<String>, body_text: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            body_text: body_text.into(),
            body_html: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureCandidate {
    pub body_text: String,
    pub body_html: Option<String>,
    pub evidence_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "app/commands/signatures/")
)]
pub struct SignatureSuggestionDto {
    pub id: String,
    pub account_id: String,
    pub identity_email: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub evidence_count: i64,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<SignatureSuggestionRecord> for SignatureSuggestionDto {
    fn from(record: SignatureSuggestionRecord) -> Self {
        Self {
            id: record.id,
            account_id: record.account_id,
            identity_email: record.identity_email,
            body_text: record.body_text,
            body_html: record.body_html,
            evidence_count: record.evidence_count,
            status: record.status,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

pub(crate) fn signature_error_key(error: SignatureError) -> String {
    match error {
        SignatureError::NameEmpty => "backend.errors.signatureNameRequired",
        SignatureError::NotFoundForAccount => "backend.errors.signatureNotFound",
        SignatureError::OwnedByAnotherAccount => "backend.errors.signatureAccountMismatch",
        SignatureError::InvalidFirstContactMode => {
            "backend.errors.signatureFirstContactModeInvalid"
        }
        SignatureError::InvalidSuggestionStatus => {
            "backend.errors.signatureSuggestionStatusInvalid"
        }
        SignatureError::Persistence(error) => {
            tracing::error!(error = ?error, "signature persistence failed");
            "backend.errors.signaturePersistenceFailed"
        }
    }
    .to_owned()
}

impl From<SignatureError> for CommandError {
    fn from(error: SignatureError) -> Self {
        Self::new(signature_error_key(error))
    }
}

fn signature_message_error(error: SignatureMessageError) -> CommandError {
    tracing::error!(error = ?error, "signature message history load failed");
    CommandError::new("backend.errors.signaturePersistenceFailed")
}

fn resolved_signature(record: ResolvedSignatureRecord) -> ResolvedSignature {
    let body_html = record.body_html.as_deref().map(sanitize_email_html);
    let attachments = record
        .assets
        .into_iter()
        .filter(|asset| {
            body_html
                .as_deref()
                .map(|html| html.contains(&format!("cid:{}", asset.content_id)))
                .unwrap_or(false)
        })
        .map(|asset| OutgoingAttachment {
            filename: asset.filename,
            mime_type: asset.mime_type,
            content_base64: base64::engine::general_purpose::STANDARD.encode(asset.content),
            content_id: Some(asset.content_id),
            inline: true,
        })
        .collect();
    ResolvedSignature {
        signature_id: record.signature_id,
        variant: match record.variant {
            CoreSignatureVariant::Long => SignatureVariant::Long,
            CoreSignatureVariant::Short => SignatureVariant::Short,
        },
        body_text: record.body_text,
        body_html,
        attachments,
    }
}

#[tauri::command]
pub async fn list_signatures(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<SignatureDto>> {
    SignatureRepository::new(pool.inner().as_ref())
        .list(account_id.as_deref())
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_signature(
    pool: tauri::State<'_, SqlitePool>,
    input: SignatureInput,
) -> CommandResult<SignatureDto> {
    if !AccountRepository::new(pool.inner())
        .exists(&input.account_id)
        .await
        .map_err(CommandError::from)?
    {
        return Err(CommandError::new("backend.errors.accountNotFound"));
    }
    let decoded_assets = decode_assets(&input.assets)?;
    let body_html = input.body_html.as_deref().map(sanitize_email_html);
    let short_body_html = input.short_body_html.as_deref().map(sanitize_email_html);
    SignatureRepository::new(pool.inner())
        .upsert(SignatureUpsertRequest {
            id: input.id,
            account_id: input.account_id,
            name: input.name,
            body_text: input.body_text,
            body_html,
            short_body_text: input.short_body_text,
            short_body_html,
            assets: decoded_assets,
            is_default: input.is_default,
        })
        .await
        .map(Into::into)
        .map_err(Into::into)
}

fn decode_assets(assets: &[SignatureAssetInput]) -> CommandResult<Vec<SignatureAssetWrite>> {
    let mut decoded = Vec::with_capacity(assets.len());
    let mut total = 0usize;
    let mut content_ids = HashSet::new();
    for asset in assets {
        let filename = asset.filename.trim();
        let mime_type = asset.mime_type.trim().to_ascii_lowercase();
        let content_id = asset.content_id.trim().trim_matches(['<', '>']).to_string();
        if filename.is_empty() || content_id.is_empty() || content_id.contains(['\r', '\n']) {
            return Err(CommandError::new(
                "backend.errors.signatureAssetIdentityInvalid",
            ));
        }
        if !mime_type.starts_with("image/") {
            return Err(CommandError::new(
                "backend.errors.signatureAssetMimeInvalid",
            ));
        }
        if !content_ids.insert(content_id.clone()) {
            return Err(CommandError::new(
                "backend.errors.signatureAssetContentIdDuplicate",
            ));
        }
        let content = base64::engine::general_purpose::STANDARD
            .decode(
                asset
                    .content_base64
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>(),
            )
            .map_err(|error| {
                tracing::warn!(error = ?error, "invalid base64 signature asset rejected");
                CommandError::new("backend.errors.signatureAssetContentInvalid")
            })?;
        if content.len() > MAX_ASSET_BYTES {
            return Err(CommandError::new("backend.errors.signatureAssetTooLarge")
                .with_param("max", MAX_ASSET_BYTES as i64));
        }
        total = total.saturating_add(content.len());
        if total > MAX_SIGNATURE_ASSET_BYTES {
            return Err(CommandError::new("backend.errors.signatureAssetsTooLarge")
                .with_param("max", MAX_SIGNATURE_ASSET_BYTES as i64));
        }
        decoded.push(SignatureAssetWrite {
            filename: filename.to_string(),
            mime_type,
            content_id,
            content,
        });
    }
    Ok(decoded)
}

#[tauri::command]
pub async fn delete_signature(pool: tauri::State<'_, SqlitePool>, id: String) -> CommandResult<()> {
    SignatureRepository::new(pool.inner())
        .delete(&id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_signature_assignments(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<SignatureAssignmentDto>> {
    SignatureRepository::new(pool.inner().as_ref())
        .list_assignments(account_id.as_deref())
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn upsert_signature_assignment(
    pool: tauri::State<'_, SqlitePool>,
    input: SignatureAssignmentInput,
) -> CommandResult<SignatureAssignmentDto> {
    let identity_email = normalize_email(&input.identity_email)?;
    assert_identity_owned(pool.inner(), &input.account_id, &identity_email).await?;
    let first_contact_mode = FirstContactMode::parse(&input.first_contact_mode)?;
    SignatureRepository::new(pool.inner())
        .upsert_assignment(SignatureAssignmentUpsertRequest {
            account_id: input.account_id,
            identity_email,
            signature_id: input.signature_id,
            first_contact_mode,
        })
        .await
        .map(Into::into)
        .map_err(Into::into)
}

#[tauri::command]
pub async fn resolve_signature(
    pool: tauri::State<'_, crate::db::ReadPool>,
    input: ResolveSignatureInput,
) -> CommandResult<Option<ResolvedSignature>> {
    resolve_signature_for_compose(pool.inner().as_ref(), input)
        .await
        .map_err(CommandError::from)
}

pub async fn resolve_signature_for_compose(
    pool: &SqlitePool,
    input: ResolveSignatureInput,
) -> Result<Option<ResolvedSignature>, String> {
    if input.preference == SignaturePreference::None {
        return Ok(None);
    }
    let identity_email = normalize_email(&input.identity_email).map_err(|error| error.encoded())?;
    assert_identity_owned(pool, &input.account_id, &identity_email)
        .await
        .map_err(|error| error.encoded())?;
    let sent_from_identity_in_thread = SignatureMessageRepository::new(pool)
        .sent_from_identity_in_thread(
            &input.account_id,
            &identity_email,
            input.thread_id.as_deref(),
        )
        .await
        .map_err(|error| signature_message_error(error).encoded())?;
    let resolved = SignatureRepository::new(pool)
        .resolve(SignatureResolveRequest {
            account_id: input.account_id,
            identity_email,
            signature_id: input.signature_id,
            preference: match input.preference {
                SignaturePreference::Auto => CoreSignaturePreference::Auto,
                SignaturePreference::Long => CoreSignaturePreference::Long,
                SignaturePreference::Short => CoreSignaturePreference::Short,
                SignaturePreference::None => CoreSignaturePreference::None,
            },
            sent_from_identity_in_thread,
        })
        .await
        .map_err(|error| CommandError::from(error).encoded())?;
    Ok(resolved.map(resolved_signature))
}

async fn assert_identity_owned(
    pool: &SqlitePool,
    account_id: &str,
    identity_email: &str,
) -> CommandResult<()> {
    let primary_identity = AccountRepository::new(pool)
        .owns_primary_identity(account_id, identity_email)
        .await
        .map_err(CommandError::from)?;
    let alias_exists = AliasRepository::new(pool)
        .contains(account_id, identity_email)
        .await
        .map_err(|error| CommandError::new(alias_error_key(error)))?;
    if !primary_identity && !alias_exists {
        return Err(CommandError::new(
            "backend.errors.signatureIdentityNotOwned",
        ));
    }
    Ok(())
}

fn normalize_email(value: &str) -> CommandResult<String> {
    let normalized = value.trim().to_lowercase();
    if normalized.is_empty() || !normalized.contains('@') {
        return Err(CommandError::new(
            "backend.errors.signatureIdentityEmailInvalid",
        ));
    }
    Ok(normalized)
}

pub fn detect_signature_candidates(
    messages: &[SignatureDetectionMessage],
    minimum_evidence: usize,
) -> Vec<SignatureCandidate> {
    let minimum_evidence = minimum_evidence.max(2);
    let mut grouped: HashMap<String, (String, HashSet<String>)> = HashMap::new();
    for message in messages {
        for candidate in signature_tail_candidates(&message.body_text) {
            let key = candidate
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let entry = grouped
                .entry(key)
                .or_insert_with(|| (candidate.clone(), HashSet::new()));
            entry.1.insert(message.thread_id.clone());
        }
    }
    let qualified = grouped
        .into_iter()
        .filter_map(|(key, (body_text, threads))| {
            (threads.len() >= minimum_evidence).then_some((key, body_text, threads.len()))
        })
        .collect::<Vec<_>>();
    // Signatures often span several blank-line-separated blocks (greeting,
    // company block, contact line). Each message contributed every paragraph
    // suffix above, so the full signature and its fragments all qualify.
    // Keep the longest version that carries the bulk of the evidence and
    // drop its fragments; drop rare extensions the other way around.
    let mut suppressed: HashSet<&str> = HashSet::new();
    for (short_key, _, short_count) in &qualified {
        for (long_key, _, long_count) in &qualified {
            if !is_proper_suffix_block(short_key, long_key) {
                continue;
            }
            if long_count * 2 >= *short_count {
                suppressed.insert(short_key);
            } else {
                suppressed.insert(long_key);
            }
        }
    }
    let mut candidates = qualified
        .iter()
        .filter(|(key, _, _)| !suppressed.contains(key.as_str()))
        .map(|(_, body_text, evidence_count)| SignatureCandidate {
            body_text: body_text.clone(),
            body_html: None,
            evidence_count: *evidence_count,
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.evidence_count
            .cmp(&a.evidence_count)
            .then_with(|| a.body_text.cmp(&b.body_text))
    });
    candidates
}

/// True when `short` is `long` minus at least one leading word — i.e. the
/// normalized keys share their tail on a word boundary.
fn is_proper_suffix_block(short: &str, long: &str) -> bool {
    long.len() > short.len()
        && long.ends_with(short)
        && long.as_bytes()[long.len() - short.len() - 1] == b' '
}

/// Paragraph suffixes of the unquoted message text, shortest first: the last
/// blank-line-separated block, the last two, … up to five. Multi-block
/// signatures (greeting / company block / contact line) only ever repeat as a
/// whole, so the caller needs the longer suffixes to see them.
fn signature_tail_candidates(body: &str) -> Vec<String> {
    let normalized = body.replace("\r\n", "\n");
    let unquoted = normalized
        .lines()
        .take_while(|line| {
            let trimmed = line.trim();
            // Stop at the first quoted/forwarded line — everything below it
            // belongs to the previous message, not to this signature.
            !(trimmed.starts_with('>')
                || trimmed.starts_with("On ")
                || trimmed.starts_with("---------- Weitergeleitete Nachricht")
                // Spark and Satchel prefix quotes with an unindented intro line.
                || trimmed.starts_with("Nachricht von ")
                || trimmed.starts_with("-----Ursprüngliche Nachricht")
                || (trimmed.starts_with("Am ") && trimmed.contains("schrieb")))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let paragraphs = unquoted
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for depth in 1..=paragraphs.len().min(5) {
        let candidate = paragraphs[paragraphs.len() - depth..].join("\n\n");
        let line_count = candidate
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        if line_count > 12 || candidate.len() > 1200 {
            // Longer suffixes only grow further past the caps.
            break;
        }
        if candidate.len() < 8 {
            continue;
        }
        // A single repeated line is only a signature when it looks like
        // contact data ("mail@example.de | 0176 …"); otherwise it is a closing
        // like "Viele Grüße" that would drown the real suggestions.
        if line_count == 1 && !looks_like_contact_line(&candidate) {
            continue;
        }
        candidates.push(candidate);
    }
    candidates
}

fn looks_like_contact_line(value: &str) -> bool {
    value.contains('@')
        || value.contains("http")
        || value.chars().filter(char::is_ascii_digit).count() >= 5
}

fn candidate_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Find the HTML fragment whose text content equals the detected signature
/// text. Works on a window of sibling blocks because the signature usually
/// sits between the message body and the quoted history (blockquote), not at
/// the very end of the document.
pub fn extract_signature_html(body_html: &str, signature_text: &str) -> Option<String> {
    let target = candidate_key(signature_text);
    if target.is_empty() {
        return None;
    }
    let document = scraper::Html::parse_document(body_html);
    let body_selector = scraper::Selector::parse("body").ok()?;
    let mut scope = document.select(&body_selector).next()?;

    for _depth in 0..6 {
        let entries: Vec<(String, String)> = scope
            .children()
            .filter_map(|node| {
                if let Some(element) = scraper::ElementRef::wrap(node) {
                    let key = candidate_key(&element.text().collect::<String>());
                    Some((element.html(), key))
                } else if let Some(text) = node.value().as_text() {
                    let raw: &str = text;
                    let key = candidate_key(raw);
                    (!key.is_empty()).then(|| (escape_html_text(raw), key))
                } else {
                    None
                }
            })
            .collect();

        if let Some(html) = match_block_window(&entries, &target) {
            return Some(html);
        }

        // No match among these siblings — the signature may live one level
        // deeper (e.g. everything wrapped in a single container div).
        let next = scope.children().rev().find_map(|node| {
            scraper::ElementRef::wrap(node)
                .filter(|element| !candidate_key(&element.text().collect::<String>()).is_empty())
        })?;
        scope = next;
    }
    None
}

/// Search sibling blocks for the window whose combined text equals `target`,
/// preferring the match closest to the end. Window edges must carry text so
/// leading/trailing spacer blocks stay out of the fragment.
fn match_block_window(entries: &[(String, String)], target: &str) -> Option<String> {
    for end in (1..=entries.len()).rev() {
        if entries[end - 1].1.is_empty() {
            continue;
        }
        let mut key = String::new();
        for start in (0..end).rev() {
            if key.is_empty() {
                key = entries[start].1.clone();
            } else if !entries[start].1.is_empty() {
                key = format!("{} {}", entries[start].1, key);
            }
            if key.len() > target.len() {
                break;
            }
            if key == target && !entries[start].1.is_empty() {
                return Some(
                    entries[start..end]
                        .iter()
                        .map(|(html, _)| html.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
        }
    }
    None
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Drop inline `cid:` images from an extracted signature: the referenced
/// attachment bytes belong to the source message and do not travel with the
/// suggestion. Operates on ammonia-sanitized markup (quoted attributes).
fn strip_cid_images(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<img") {
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        let tag = &rest[start..start + end + 1];
        result.push_str(&rest[..start]);
        if !tag.contains("src=\"cid:") && !tag.contains("src='cid:") {
            result.push_str(tag);
        }
        rest = &rest[start + end + 1..];
    }
    result.push_str(rest);
    result
}

#[tauri::command]
pub async fn discover_signature_suggestions(
    pool: tauri::State<'_, SqlitePool>,
    account_id: String,
    identity_email: String,
) -> CommandResult<Vec<SignatureSuggestionDto>> {
    let identity_email = normalize_email(&identity_email)?;
    assert_identity_owned(pool.inner(), &account_id, &identity_email).await?;
    let messages = SignatureMessageRepository::new(pool.inner())
        .recent_sent_from_identity(&account_id, &identity_email, 200)
        .await
        .map_err(signature_message_error)?
        .into_iter()
        .map(|message| SignatureDetectionMessage {
            thread_id: message.thread_id,
            body_text: message.body_text,
            body_html: message.body_html,
        })
        .collect::<Vec<_>>();
    let mut suggestions = Vec::new();
    for candidate in detect_signature_candidates(&messages, 3) {
        // Recover the formatted version from a message that contains this
        // signature, so adopting it keeps links, styling and layout.
        let key = candidate_key(&candidate.body_text);
        let body_html = messages
            .iter()
            .filter(|message| {
                message.body_html.is_some() && candidate_key(&message.body_text).contains(&key)
            })
            .take(5)
            .find_map(|message| {
                let extracted =
                    extract_signature_html(message.body_html.as_deref()?, &candidate.body_text)?;
                let sanitized = strip_cid_images(&sanitize_email_html(&extracted));
                (!sanitized.trim().is_empty()).then_some(sanitized)
            });
        suggestions.push(SignatureSuggestionUpsertRequest {
            body_text: candidate.body_text,
            body_html,
            evidence_count: candidate.evidence_count as i64,
        });
    }
    let repository = SignatureRepository::new(pool.inner());
    repository
        .upsert_suggestions(&account_id, &identity_email, suggestions)
        .await?;
    repository
        .list_suggestions(
            Some(&account_id),
            Some(&identity_email),
            SignatureSuggestionStatus::Pending,
        )
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_signature_suggestions(
    pool: tauri::State<'_, crate::db::ReadPool>,
    account_id: Option<String>,
) -> CommandResult<Vec<SignatureSuggestionDto>> {
    SignatureRepository::new(pool.inner().as_ref())
        .list_suggestions(
            account_id.as_deref(),
            None,
            SignatureSuggestionStatus::Pending,
        )
        .await
        .map(|rows| rows.into_iter().map(Into::into).collect())
        .map_err(Into::into)
}

#[tauri::command]
pub async fn set_signature_suggestion_status(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
    status: String,
) -> CommandResult<()> {
    let status = SignatureSuggestionStatus::parse(&status)?;
    SignatureRepository::new(pool.inner())
        .set_suggestion_status(&id, status)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistence_errors_use_a_stable_key_without_sql_details() {
        let detail = "near SELECT: secret signature SQL";
        let error = CommandError::from(SignatureError::Persistence(sqlx::Error::Protocol(
            detail.into(),
        )));
        let serialized = serde_json::to_value(error).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({ "key": "backend.errors.signaturePersistenceFailed" })
        );
        assert!(!serialized.to_string().contains(detail));
    }

    #[test]
    fn typed_validation_errors_use_stable_keys() {
        assert_eq!(
            CommandError::from(SignatureError::NameEmpty).key,
            "backend.errors.signatureNameRequired"
        );
        assert_eq!(
            CommandError::from(SignatureError::InvalidFirstContactMode).key,
            "backend.errors.signatureFirstContactModeInvalid"
        );
        assert_eq!(
            CommandError::from(SignatureError::InvalidSuggestionStatus).key,
            "backend.errors.signatureSuggestionStatusInvalid"
        );
        assert_eq!(
            normalize_email("not-an-address").unwrap_err().key,
            "backend.errors.signatureIdentityEmailInvalid"
        );
    }

    #[test]
    fn asset_validation_never_exposes_decoder_details() {
        let error = decode_assets(&[SignatureAssetInput {
            filename: "logo.png".into(),
            mime_type: "image/png".into(),
            content_id: "logo".into(),
            content_base64: "%%%".into(),
        }])
        .unwrap_err();
        assert_eq!(error.key, "backend.errors.signatureAssetContentInvalid");
        assert!(error.params.is_empty());
    }

    #[test]
    fn resolution_sanitizes_html_and_only_maps_referenced_cids() {
        let resolved = resolved_signature(ResolvedSignatureRecord {
            signature_id: "sig".into(),
            variant: CoreSignatureVariant::Long,
            body_text: "Regards".into(),
            body_html: Some(
                r#"<p onclick="bad()">Regards<img src="cid:used"></p><script>bad()</script>"#
                    .into(),
            ),
            assets: vec![
                SignatureAssetRecord {
                    id: "used".into(),
                    filename: "used.png".into(),
                    mime_type: "image/png".into(),
                    content_id: "used".into(),
                    content: vec![1],
                },
                SignatureAssetRecord {
                    id: "unused".into(),
                    filename: "unused.png".into(),
                    mime_type: "image/png".into(),
                    content_id: "unused".into(),
                    content: vec![2],
                },
            ],
        });
        let html = resolved.body_html.unwrap();
        assert!(!html.contains("onclick"));
        assert!(!html.contains("script"));
        assert_eq!(resolved.attachments.len(), 1);
        assert_eq!(resolved.attachments[0].content_id.as_deref(), Some("used"));
        assert_eq!(resolved.attachments[0].content_base64, "AQ==");
    }

    #[test]
    fn detection_counts_distinct_threads_and_ignores_quoted_history() {
        let messages = vec![
            SignatureDetectionMessage::text(
                "one",
                "Hello\n\nRegards\nAlice\nalice@example.test\n\n> old quote",
            ),
            SignatureDetectionMessage::text("two", "Update\n\nRegards\nAlice\nalice@example.test"),
            SignatureDetectionMessage::text("three", "Again\n\nRegards\nAlice\nalice@example.test"),
        ];
        let candidates = detect_signature_candidates(&messages, 3);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].body_text,
            "Regards\nAlice\nalice@example.test"
        );
        assert_eq!(candidates[0].evidence_count, 3);
    }

    #[test]
    fn extracted_html_keeps_formatting_and_cid_strip_only_removes_cid_images() {
        let html = "<html><body><p>Message</p><div><b>Regards</b><br>Alice</div><img src=\"cid:logo\"><img src=\"https://example.test/logo.png\"></body></html>";
        let extracted = extract_signature_html(html, "RegardsAlice").unwrap();
        assert!(extracted.contains("<b>Regards</b>"));
        let stripped = strip_cid_images(
            "<p>Regards</p><img src=\"cid:logo\"><img src=\"https://example.test/logo.png\">",
        );
        assert!(!stripped.contains("cid:logo"));
        assert!(stripped.contains("https://example.test/logo.png"));
    }
}
