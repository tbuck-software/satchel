use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::sync::errors::SyncResult;

pub use postroom::SourceCheckpoint as SyncCheckpoint;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMessageRemoval {
    pub remote_id: String,
    pub source_folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMessageState {
    pub remote_id: String,
    /// None means the snapshot only establishes presence and must not clobber
    /// locally known flag state.
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

/// A presence/flag snapshot plus the identity generation in which its remote
/// ids are meaningful. IMAP supplies UIDVALIDITY; providers with globally
/// stable ids leave it empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMessageStateSnapshot {
    pub states: Vec<RemoteMessageState>,
    pub remote_uidvalidity: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteIdTranslation {
    pub source_id: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub remote_id: String,
    /// IMAP UIDs are only stable inside one UIDVALIDITY generation. Other
    /// providers leave this empty.
    #[serde(default)]
    pub remote_uidvalidity: Option<u32>,
    pub remote_thread_id: Option<String>,
    /// Complete wire source for download-only providers whose server copy may
    /// be removed after ingestion. Other providers fetch originals on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_source: Option<Vec<u8>>,
    /// Provider mailbox/label ids currently attached to this message.
    /// Gmail uses these to prevent account-wide history events from being
    /// persisted into whichever folder cursor happened to observe them.
    #[serde(default)]
    pub mailbox_ids: Vec<String>,
    #[serde(default)]
    pub message_id_header: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub to_addresses: Vec<String>,
    pub cc_addresses: Vec<String>,
    #[serde(default)]
    pub bcc_addresses: Vec<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub remote_draft_id: Option<String>,
    pub date_rfc3339: String,
    /// Provider receive time, independent of the sender's Date header.
    #[serde(default)]
    pub received_at_rfc3339: Option<String>,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    #[serde(default)]
    pub is_read: bool,
    #[serde(default)]
    pub is_starred: bool,
    /// Raw VCALENDAR text extracted from a `text/calendar` MIME part. Used by the
    /// ingest pipeline to populate `calendar_invites` even when the calendar is
    /// not embedded in the visible body. Defaults to None for providers that don't
    /// surface attachment content (Gmail/Outlook HTTP APIs).
    #[serde(default)]
    pub body_calendar: Option<String>,
    /// Raw S/MIME enveloped-data payload extracted from application/pkcs7-mime.
    #[serde(default)]
    pub smime_encrypted: Option<Vec<u8>>,
    /// Raw S/MIME signed MIME bundle extracted from multipart/signed or pkcs7 MIME.
    #[serde(default)]
    pub smime_signed: Option<Vec<u8>>,
    /// Raw OpenPGP signed MIME bundle extracted from RFC 3156 multipart/signed.
    #[serde(default)]
    pub pgp_signed: Option<Vec<u8>>,
    #[serde(default)]
    pub trust: MessageTrust,
    /// Raw `List-Unsubscribe` header value (RFC 2369) — used by the auto-unsubscribe
    /// banner to offer one-click unsubscribe.
    #[serde(default)]
    pub list_unsubscribe: Option<String>,
    /// Raw `List-Unsubscribe-Post` header value (RFC 8058) — presence signals
    /// one-click HTTP POST is available without user confirmation.
    #[serde(default)]
    pub list_unsubscribe_post: Option<String>,
    /// SPF verdict from the receiving provider's Authentication-Results header (RFC 8601).
    #[serde(default)]
    pub auth_spf: Option<String>,
    /// DKIM verdict from the receiving provider's Authentication-Results header (RFC 8601).
    #[serde(default)]
    pub auth_dkim: Option<String>,
    /// DMARC verdict from the receiving provider's Authentication-Results header (RFC 8601).
    #[serde(default)]
    pub auth_dmarc: Option<String>,
    #[serde(default)]
    pub attachments: Vec<RemoteAttachment>,
}

/// Parse only the receiver's topmost Authentication-Results header.
/// Unknown providers fail closed because their header-stripping boundary is unknown.
pub fn trusted_authentication_results(
    value: &str,
    receiver: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let cleaned = authentication_tokens(value);
    let Some((authserv, _)) = cleaned.split_once(';') else {
        return (None, None, None);
    };
    let authserv = authserv.trim().to_ascii_lowercase();
    let receiver = receiver.to_ascii_lowercase();
    let trusted = match receiver.as_str() {
        "gmail" | "imap.gmail.com" | "pop.gmail.com" => authserv == "mx.google.com",
        // Outlook's usual header has no authserv-id; a sender-supplied ID is
        // insufficient until its stripping boundary is established.
        _ => false,
    };
    if !trusted {
        return (None, None, None);
    }
    parse_authentication_results(&cleaned)
}

/// Remove comments, including nested/escaped comments, and quoted property values.
fn authentication_tokens(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if (depth > 0 || quoted) && character == '\\' {
            escaped = true;
            continue;
        }
        if depth > 0 {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
        } else if character == '"' {
            quoted = !quoted;
            output.push(' ');
        } else if !quoted {
            if character == '(' {
                depth = 1;
                output.push(' ');
            } else {
                output.push(character);
            }
        }
    }
    output
}

pub fn parse_authentication_results(
    value: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut spf = None;
    let mut dkim = None;
    let mut dmarc = None;
    let cleaned = authentication_tokens(value);
    for result in cleaned.split(';').skip(1) {
        let Some((method, result)) = result.trim().split_once('=') else {
            continue;
        };
        let verdict = result
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let method = method.trim().to_ascii_lowercase();
        let recognised = match method.as_str() {
            "spf" => matches!(
                verdict.as_str(),
                "pass" | "fail" | "softfail" | "neutral" | "none" | "temperror" | "permerror"
            ),
            "dkim" => matches!(
                verdict.as_str(),
                "pass" | "fail" | "none" | "neutral" | "policy" | "temperror" | "permerror"
            ),
            "dmarc" => matches!(
                verdict.as_str(),
                "pass" | "fail" | "none" | "temperror" | "permerror"
            ),
            _ => false,
        };
        if !recognised {
            continue;
        }
        match method.as_str() {
            "spf" if spf.is_none() => spf = Some(verdict),
            "dkim" if dkim.is_none() => dkim = Some(verdict),
            "dmarc" if dmarc.is_none() => dmarc = Some(verdict),
            _ => {}
        }
    }
    (spf, dkim, dmarc)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageTrust {
    /// Set only while parsing a complete MIME tree with a covered display body.
    #[serde(default)]
    pub root_crypto: bool,
    #[serde(default)]
    pub root_encryption: bool,
    #[serde(default)]
    pub root_inline_signature: bool,
    /// The root is `multipart/signed`, and the strict splitter wrote the
    /// stored body as the display of its signed entity, together with the
    /// bundle. Rows without it predate that splitter; their stored body is
    /// never shown under the signature.
    #[serde(default)]
    pub signed_binding: bool,
    /// Header verdicts came through the receiving provider's trust boundary.
    #[serde(default)]
    pub auth_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBatch {
    pub messages: Vec<MessageEnvelope>,
    pub removals: Vec<RemoteMessageRemoval>,
    /// The identity generation of the selected remote folder. IMAP supplies
    /// UIDVALIDITY so folder-scoped UIDs can be persisted safely even when a
    /// sync returns no messages; other providers leave it empty.
    #[serde(default)]
    pub remote_uidvalidity: Option<u32>,
    /// The provider's identity generation changed, so locally cached rows for
    /// this folder must be discarded before ingesting the batch.
    #[serde(default)]
    pub reset_folder: bool,
    /// Request a full presence snapshot after this batch. Providers set this
    /// only when their incremental cursor expired (or flags require it), so an
    /// ordinary poll stays proportional to the changes.
    pub snapshot_required: bool,
    pub next: SyncCheckpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAttachment {
    pub remote_part_id: String,
    pub filename: Option<String>,
    pub content_type: String,
    pub size: Option<i64>,
    pub content_id: Option<String>,
    #[serde(default)]
    pub is_inline: bool,
    #[serde(default)]
    pub content_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedAttachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ipc-types", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "ipc-types",
    ts(export, export_to = "core/providers/traits/")
)]
pub struct OutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    /// Base64-encoded content (standard alphabet, no URL-safe).
    pub content_base64: String,
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub inline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutgoingMessage {
    /// Optional override for the From-header (used for aliases / send-as).
    /// When None, providers fall back to the account's primary address.
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    /// Optional Reply-To header. Used when sending as alias to keep replies on the
    /// primary account address.
    pub reply_to: Option<String>,
    /// Bare addresses: the SMTP envelope, crypto recipients and contacts use these.
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    /// Display names for the header only, keyed by lowercased address.
    #[serde(default)]
    pub recipient_names: std::collections::BTreeMap<String, String>,
    pub subject: String,
    pub body: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<OutgoingAttachment>,
    /// Stable Message-ID header for idempotency. Without it, a crash between
    /// provider.send() and the local "sent" bookkeeping would resend the mail.
    /// Set by `build_send_action` and forwarded by all providers verbatim.
    #[serde(default)]
    pub message_id_header: Option<String>,
    /// When set, providers add a `Disposition-Notification-To: <addr>` header
    /// asking the recipient's mail client for a read receipt. Off by default
    /// (privacy-respectful — Spark also has this off by default).
    #[serde(default)]
    pub request_read_receipt_to: Option<String>,
    /// Complete RFC822 message used by pre-send crypto transforms such as S/MIME.
    #[serde(default)]
    pub raw_rfc822_override: Option<String>,
    #[serde(default)]
    pub smime_sign_with_cert_id: Option<String>,
    #[serde(default)]
    pub smime_encrypt_to_recipients: bool,
    /// Id of an unlocked private PGP key to sign with (RFC 3156).
    #[serde(default)]
    pub pgp_sign_with_key_id: Option<String>,
    #[serde(default)]
    pub pgp_encrypt_to_recipients: bool,
    /// RFC 4021 importance. "high" | "normal" | "low"; None sends no header.
    #[serde(default)]
    pub priority: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRef {
    pub remote_id: String,
    pub source_folder: Option<String>,
    /// IMAP UIDs are scoped to this mailbox generation. Other providers have
    /// account-wide message ids and leave this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uidvalidity: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlagUpdate {
    pub read: Option<bool>,
    pub starred: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MailEvent {
    // Boxed: the envelope is ~580 bytes while every other variant is under 30,
    // so an unboxed variant would size the whole broadcast channel to the
    // largest case.
    NewMessage(Box<MessageEnvelope>),
    FlagsChanged { uid: u32, flags: Vec<String> },
    MessageExpunged(u32),
    FolderChanged(String),
}

#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn sync_folders(&self) -> SyncResult<Vec<RemoteFolder>>;
    async fn sync_messages(&self, folder: &str, since: SyncCheckpoint) -> SyncResult<SyncBatch>;
    /// Return the current lightweight message state for a folder when the
    /// provider can do so efficiently. This lets incremental sync reconcile
    /// flag changes, moves, and deletions for messages below the new-mail cursor.
    ///
    /// The snapshot carries the generation its remote ids belong to. Dropping
    /// it once let a snapshot from a new IMAP UIDVALIDITY delete every local
    /// message of the folder, because none of the old UIDs was in it.
    async fn sync_message_state_snapshot(
        &self,
        _folder: &str,
    ) -> SyncResult<Option<RemoteMessageStateSnapshot>> {
        Ok(None)
    }
    /// Resolve an account-wide ID absent from a folder snapshot. `None` is
    /// proof of global deletion, never a transient failure or label removal.
    /// Providers without globally stable IDs must not implement this seam.
    async fn resolve_snapshot_message(
        &self,
        _remote_id: &str,
    ) -> SyncResult<Option<MessageEnvelope>> {
        Err("provider does not support global snapshot resolution".into())
    }
    /// Exactly these messages of `folder`, with the folder's generation in
    /// `remote_uidvalidity`. `None`: the provider cannot address single
    /// messages, and the caller falls back to a folder sync.
    async fn fetch_messages_by_remote_ids(
        &self,
        _folder: &str,
        _remote_ids: &[String],
    ) -> SyncResult<Option<SyncBatch>> {
        Ok(None)
    }
    async fn translate_message_ids(
        &self,
        _source_ids: &[String],
    ) -> SyncResult<Vec<RemoteIdTranslation>> {
        Err("provider does not support remote id translation".into())
    }
    async fn send(&self, message: OutgoingMessage) -> SyncResult<()>;

    /// Send and hand back the exact bytes the transport wrote, when it has
    /// them. The Sent copy is taken from here instead of being rebuilt: a
    /// second build produces different boundaries, a different `Date`, and —
    /// for a crypto send — the plaintext the wire form no longer carries.
    async fn send_returning_wire_copy(
        &self,
        message: OutgoingMessage,
    ) -> SyncResult<Option<Vec<u8>>> {
        self.send(message).await?;
        Ok(None)
    }

    async fn set_flags(&self, messages: &[MessageRef], flags: FlagUpdate) -> SyncResult<()>;
    async fn move_messages(&self, messages: &[MessageRef], target: &str) -> SyncResult<()>;
    async fn delete_messages(&self, messages: &[MessageRef]) -> SyncResult<()>;

    /// Create a remote folder/label. `path` is the full remote path using the
    /// provider's own separator convention.
    ///
    /// Default is an explicit error rather than a silent no-op: a folder that
    /// exists locally but not remotely would silently swallow moved mail.
    async fn create_folder(&self, _path: &str) -> SyncResult<()> {
        Err("provider does not support creating folders".into())
    }

    async fn rename_folder(&self, _from_path: &str, _to_path: &str) -> SyncResult<()> {
        Err("provider does not support renaming folders".into())
    }

    async fn delete_folder(&self, _path: &str) -> SyncResult<()> {
        Err("provider does not support deleting folders".into())
    }

    async fn fetch_attachment(
        &self,
        _message: &MessageRef,
        _remote_part_id: &str,
    ) -> SyncResult<FetchedAttachment> {
        Err("provider does not support remote attachment fetch".into())
    }

    /// Append a sent message to the remote Sent folder. SMTP-only providers need
    /// this — the SMTP server doesn't file a copy, so without it sent mail is
    /// invisible in webmail and other clients. API providers (Gmail/Outlook)
    /// file sent mail themselves; the default no-op is correct for them.
    async fn append_sent(&self, _raw_rfc822: &[u8], _folder_hint: Option<&str>) -> SyncResult<()> {
        Ok(())
    }

    /// Append a draft message to the remote drafts folder.
    /// Default impl is a no-op so providers can opt in incrementally.
    async fn save_draft(
        &self,
        _raw_rfc822: &[u8],
        _folder_hint: Option<&str>,
    ) -> SyncResult<Option<String>> {
        Ok(None)
    }

    async fn upsert_draft(
        &self,
        message: OutgoingMessage,
        remote_draft_id: Option<&str>,
        folder_hint: Option<&str>,
    ) -> SyncResult<Option<String>> {
        let raw = crate::email::build::build_rfc822(&message, None, None)?;
        let _ = remote_draft_id;
        self.save_draft(raw.as_bytes(), folder_hint).await
    }

    async fn resolve_draft_id(
        &self,
        _message_remote_id: &str,
        _folder_hint: Option<&str>,
    ) -> SyncResult<Option<String>> {
        Ok(None)
    }

    async fn delete_draft(&self, _remote_draft_id: &str) -> SyncResult<()> {
        Ok(())
    }

    /// Start a long-lived IDLE loop on the given folder. Returns the join-handle's
    /// inner result; implementations are expected to manage their own reconnect logic.
    /// Default impl is a no-op so providers can opt in (used by IMAP, not Gmail/Outlook).
    async fn idle(&self, _folder: &str, _on_event: IdleCallback) -> SyncResult<()> {
        Ok(())
    }
}

pub type IdleCallback = std::sync::Arc<dyn Fn(IdleEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum IdleEvent {
    NewMessages,
    FlagsChanged,
    Expunged,
}

#[cfg(test)]
mod tests {
    use super::{parse_authentication_results, trusted_authentication_results};

    #[test]
    fn rejects_sender_authentication_results_and_unknown_receivers() {
        let forged = "attacker.example; spf=pass; dkim=pass; dmarc=pass";
        assert_eq!(
            trusted_authentication_results(forged, "gmail"),
            (None, None, None)
        );
        let forged_provider = "mx.google.com; spf=pass; dkim=pass; dmarc=pass";
        assert_eq!(
            trusted_authentication_results(forged_provider, "mail.example"),
            (None, None, None)
        );
        assert_eq!(
            trusted_authentication_results(forged_provider, "outlook"),
            (None, None, None)
        );
        assert_eq!(
            trusted_authentication_results(
                "protection.outlook.com; spf=pass; dkim=pass; dmarc=pass",
                "outlook",
            ),
            (None, None, None)
        );
    }

    #[test]
    fn ignores_verdicts_in_nested_comments_quotes_and_properties() {
        let attack = r#"mx.google.com; spf=fail (envelope sender dmarc=pass@attacker.example (dkim=pass) escaped \) comment); dkim=none header.d="dmarc=pass"; dmarc=fail"#;
        let (spf, dkim, dmarc) = trusted_authentication_results(attack, "imap.gmail.com");
        assert_eq!(spf.as_deref(), Some("fail"));
        assert_eq!(dkim.as_deref(), Some("none"));
        assert_eq!(dmarc.as_deref(), Some("fail"));
        assert_eq!(parse_authentication_results("mx.google.com; header.dmarc=pass; dmarc=superpass; dkim=probably; spf=pass@evil.example"), (None, None, None));
    }

    #[test]
    fn only_the_topmost_header_is_considered() {
        let headers = "Authentication-Results: attacker.example; dmarc=pass\r\nAuthentication-Results: mx.google.com; dmarc=pass\r\n";
        let value =
            crate::providers::imap::parse_header_value(headers, "Authentication-Results").unwrap();
        assert_eq!(
            trusted_authentication_results(&value, "gmail"),
            (None, None, None)
        );
        let headers = "Authentication-Results: mx.google.com; spf=pass (dmarc=pass@evil.example);\r\n dmarc=fail\r\nAuthentication-Results: attacker.example; dmarc=pass\r\n";
        let value =
            crate::providers::imap::parse_header_value(headers, "Authentication-Results").unwrap();
        assert_eq!(
            trusted_authentication_results(&value, "gmail").2.as_deref(),
            Some("fail")
        );
    }

    #[test]
    fn parses_spf_dkim_and_dmarc_verdicts() {
        let value = "mx.google.com; spf=pass (sender SPF authorized) smtp.mailfrom=example.com; dkim=pass header.d=example.com; dmarc=pass action=none header.from=example.com;";
        let (spf, dkim, dmarc) = parse_authentication_results(value);
        assert_eq!(spf.as_deref(), Some("pass"));
        assert_eq!(dkim.as_deref(), Some("pass"));
        assert_eq!(dmarc.as_deref(), Some("pass"));
    }

    #[test]
    fn parses_fail_verdicts_without_comments() {
        let (spf, dkim, dmarc) = parse_authentication_results(
            "server.example.net; spf=fail (domain of sender does not designate permitted hosts); dkim=temperror; dmarc=fail (p=quarantine)",
        );
        assert_eq!(spf.as_deref(), Some("fail"));
        assert_eq!(dkim.as_deref(), Some("temperror"));
        assert_eq!(dmarc.as_deref(), Some("fail"));
    }

    #[test]
    fn keeps_the_first_verdict_per_mechanism_and_ignores_resampling() {
        let value =
            "hop1; dkim=fail; dmarc=none; spf=neutral; header.dkim=pass (resampled); dmarc=pass";
        let (spf, dkim, dmarc) = parse_authentication_results(value);
        assert_eq!(spf.as_deref(), Some("neutral"));
        assert_eq!(dkim.as_deref(), Some("fail"));
        assert_eq!(dmarc.as_deref(), Some("none"));
    }

    #[test]
    fn returns_nothing_without_mechanism_tokens() {
        let (spf, dkim, dmarc) = parse_authentication_results("none; no signature data");
        assert_eq!(spf, None);
        assert_eq!(dkim, None);
        assert_eq!(dmarc, None);
    }
}
