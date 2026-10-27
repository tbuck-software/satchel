//! Typed provider and sync failures.
//!
//! Provider code keeps the error variant intact until the queue worker has
//! made its retry decision. Strings remain at storage and Tauri seams only.
//! Untyped third-party failures still pass through [`SyncError::from_message`],
//! which classifies the SMTP, IMAP, and OAuth messages Satchel already sees.

use thiserror::Error;

pub type SyncResult<T> = Result<T, SyncError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SyncError {
    /// Provider asked us to slow down. The queue waits for the provider block
    /// deadline without consuming an attempt.
    #[error("{message}")]
    RateLimited {
        message: String,
        retry_after: Option<std::time::Duration>,
    },
    /// Retrying cannot succeed without a user or configuration change.
    #[error("{0}")]
    Permanent(String),
    /// The provider accepted the mail, but local finalization failed. Retrying
    /// would send a duplicate, so the queue keeps this as a terminal warning.
    #[error("{0}")]
    Delivered(String),
    /// The message body was handed to the server and the connection broke
    /// before it answered. It may have been delivered. Retrying could deliver
    /// it twice, so this is terminal and waits for the user.
    #[error("{0}")]
    DeliveryUnknown(String),
    /// Network failures, timeouts, server errors, and unclassified failures.
    #[error("{0}")]
    Transient(String),
}

impl SyncError {
    pub fn rate_limited(
        message: impl Into<String>,
        retry_after: Option<std::time::Duration>,
    ) -> Self {
        Self::RateLimited {
            message: message.into(),
            retry_after,
        }
    }

    pub fn from_message(message: impl Into<String>) -> Self {
        let message = message.into();
        if is_rate_limit_message(&message) {
            Self::rate_limited(message, None)
        } else if is_permanent_message(&message) {
            Self::Permanent(message)
        } else {
            Self::Transient(message)
        }
    }

    /// Preserve structured provider causes; otherwise store a plain, translatable
    /// reason alongside the diagnostic used by support and legacy recovery.
    pub fn encoded(&self) -> String {
        let diagnostic = self.to_string();
        if diagnostic.starts_with("backend.errors.")
            || serde_json::from_str::<serde_json::Value>(&diagnostic)
                .ok()
                .is_some_and(|value| {
                    value["key"]
                        .as_str()
                        .is_some_and(|key| key.starts_with("backend.errors."))
                })
        {
            return diagnostic;
        }
        let lower = diagnostic.to_ascii_lowercase();
        let key = if self.is_rate_limited() {
            "backend.errors.queueRateLimited"
        } else if !self.is_permanent() {
            "backend.errors.queueTemporary"
        } else if lower.contains("invalid recipient")
            || lower.contains("missing recipient")
            || lower.contains("recipient")
            || lower.contains("no such user")
        {
            "backend.errors.queueRecipientRejected"
        } else if lower.contains("too large")
            || lower.contains("size exceeds")
            || lower.contains("status 413")
        {
            "backend.errors.queueMessageTooLarge"
        } else if lower.contains("folder")
            || lower.contains("mailbox")
            || lower.contains("uidvalidity")
        {
            "backend.errors.queueFolderChanged"
        } else if lower.contains("auth")
            || lower.contains("token")
            || lower.contains("invalid_grant")
        {
            "backend.errors.queueAuthentication"
        } else {
            "backend.errors.queueRejected"
        };
        serde_json::json!({"key": key, "params": {"message": diagnostic}}).to_string()
    }

    /// Only recognise known temporary legacy diagnostics. An unknown stored
    /// failure may be a permanent rejection whose typed variant was lost.
    pub fn recoverable_stored_failure(error: &str) -> bool {
        let value = serde_json::from_str::<serde_json::Value>(error).ok();
        let key = value.as_ref().and_then(|v| v["key"].as_str());
        if key.is_some_and(|k| {
            k != "backend.errors.queueActionFailed" && k != "backend.errors.queueTemporary"
        }) {
            return false;
        }
        let diagnostic = value
            .as_ref()
            .and_then(|v| v["params"]["message"].as_str())
            .unwrap_or(error);
        // This legacy diagnostic was only written after the sender observed
        // a fresh token committed by another refresh. It is safe to try again.
        if diagnostic.contains("oauth refresh failed from stale config:") {
            return true;
        }
        if Self::from_message(diagnostic).is_permanent() {
            return false;
        }
        let lower = diagnostic.to_ascii_lowercase();
        // HTTP send responses can hide acceptance. Never infer safe delivery
        // from a stored string where submission progress has been lost.
        if lower.contains("send") && (lower.contains("status 5") || lower.contains("status 408")) {
            return false;
        }
        [
            "offline",
            "dns",
            "lookup address",
            "name resolution",
            "no such host",
            "network is unreachable",
            "network unreachable",
            "connection reset",
            "connection refused",
            "connect error",
            "connection error",
            "timed out",
            "timeout",
            "status 5",
            "status 401",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }

    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::Permanent(_) | Self::Delivered(_) | Self::DeliveryUnknown(_)
        )
    }

    pub fn was_delivered(&self) -> bool {
        matches!(self, Self::Delivered(_))
    }

    /// The send may or may not have gone out. No automatic retry either way.
    pub fn is_delivery_unknown(&self) -> bool {
        matches!(self, Self::DeliveryUnknown(_))
    }

    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimited { retry_after, .. } => *retry_after,
            Self::Permanent(_)
            | Self::Delivered(_)
            | Self::DeliveryUnknown(_)
            | Self::Transient(_) => None,
        }
    }

    pub fn with_retry_after(self, retry_after: std::time::Duration) -> Self {
        match self {
            Self::RateLimited { message, .. } => Self::rate_limited(message, Some(retry_after)),
            other => other,
        }
    }
}

impl From<String> for SyncError {
    fn from(message: String) -> Self {
        Self::from_message(message)
    }
}

/// A message that cannot be built fails the same way on every retry.
impl From<crate::email::build::BuildError> for SyncError {
    fn from(error: crate::email::build::BuildError) -> Self {
        Self::Permanent(error.encoded())
    }
}

impl From<&str> for SyncError {
    fn from(message: &str) -> Self {
        Self::from_message(message)
    }
}

/// Classify an HTTP failure while the status code and provider response are
/// still structured. Gmail uses 403 for both permission failures and quota
/// throttling, so the response reason takes precedence over the status alone.
pub fn http_failure(provider: &str, action: &str, status: u16, body: &str) -> SyncError {
    let message = format!("{provider} {action} failed with status {status}: {body}");
    let rate_limited = status == 429
        || (status == 403 && body.contains("rateLimitExceeded"))
        || (status == 403 && body.contains("userRateLimitExceeded"));
    if rate_limited {
        SyncError::rate_limited(message, None)
    } else if matches!(status, 400 | 403 | 404 | 410 | 413 | 422)
        || (status == 401 && is_permanent_message(body))
    {
        SyncError::Permanent(message)
    } else if action == "send" && (status >= 500 || status == 408) {
        SyncError::DeliveryUnknown(message)
    } else {
        SyncError::Transient(message)
    }
}

/// A connection failure precedes submission; later transport errors may hide acceptance.
///
/// `body_handed_over` says whether the whole request body reached the
/// transport ([`crate::utils::http::UploadProgress`]). A failure before that —
/// an upload cut off by its deadline or a dropped uplink — delivered nothing,
/// because the server never had the complete message, and stays retryable.
pub fn http_submission_transport_failure(
    provider: &str,
    error: reqwest::Error,
    body_handed_over: bool,
) -> SyncError {
    let message = format!("{provider} send failed: {error}");
    if error.is_connect() {
        SyncError::Transient(message)
    } else if error.is_builder() {
        SyncError::Permanent(message)
    } else if !body_handed_over {
        SyncError::Transient(message)
    } else {
        SyncError::DeliveryUnknown(message)
    }
}

/// Classify a failure of the submission itself — the call that hands the
/// message over.
///
/// What matters is whether anything can be concluded about delivery.
///
/// A reply code is a verdict, and it is read instead of the error text: 5xx
/// refused the mail, 4xx did not queue it, and either way nothing was
/// delivered — 4xx stays retryable, 5xx does not. Reading the text instead got
/// a hard `550 Recipient address rejected` five retries over three hours, and
/// read the queue id in `451 4.3.0 ok: queued as 8B4291C3` as a rate limit.
/// Failing to reach the server at all is the same: nothing left the machine,
/// so retrying is right. A client-side error is our own message being wrong,
/// which a second attempt will not fix.
///
/// Everything else is silence on a connection that was already talking to the
/// server: the body may have gone over in full and the answer to the final dot
/// been lost. Retrying then delivers the message twice. Those land in
/// `delivery_unknown` and wait for the user, which is the conservative side of
/// a question that cannot be answered from here — an auth-time network drop
/// ends up there too, and asking once is cheaper than a duplicate.
pub fn smtp_submission_failure(context: &str, error: &lettre::transport::smtp::Error) -> SyncError {
    let message = match error.status() {
        Some(code) => format!("{context}: {code} {error}"),
        None => format!("{context}: {error}"),
    };
    if error.is_permanent() {
        return SyncError::Permanent(message);
    }
    if is_rate_limit_message(&message) {
        return SyncError::rate_limited(message, None);
    }
    if error.is_transient() {
        return SyncError::Transient(message);
    }
    // Never reached the server, so nothing was handed over.
    if error.is_connection() || error.is_tls() {
        return SyncError::Transient(message);
    }
    // Our own message would not become valid on a second attempt.
    if error.is_client() {
        return SyncError::Permanent(message);
    }
    SyncError::DeliveryUnknown(message)
}

/// Whether an untyped provider error means "stop asking", not "this item
/// failed". New HTTP provider code should construct the variant directly.
fn is_rate_limit_message(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("rate limit")
        || lower.contains("ratelimitexceeded")
        || lower.contains("too many requests")
        || contains_status_429(&lower)
}

/// Whether `429` stands on its own rather than sitting inside a longer token.
///
/// SMTP servers quote their queue id back at you (`451 4.3.0 ok: queued as
/// 8B4291C3`). A bare `contains("429")` read that as a rate limit, and a
/// rate-limited action never consumes a retry — so the mail circulated forever
/// and never reached `failed`.
fn contains_status_429(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    lower.match_indices("429").any(|(index, _)| {
        let before = index.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(index + 3).copied();
        !before.is_some_and(is_token_char) && !after.is_some_and(is_token_char)
    })
}

fn is_token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

fn is_permanent_message(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    if is_rate_limit_message(&lower) {
        return false;
    }
    lower.contains("status 400")
        || lower.contains("status 403")
        || lower.contains("status 404")
        || lower.contains("status 410")
        || lower.contains("status 422")
        || lower.contains("missing recipient")
        || lower.contains("invalid recipient")
        || lower.contains("invalid from")
        || lower.contains("authentication failed")
        || lower.contains("invalid_grant")
        || lower.contains("token revoked")
        || lower.contains("access revoked")
        || lower.contains("message too large")
        || lower.contains("message size exceeds")
        || lower.contains("status 413")
        || lower.contains("folder not found")
        || lower.contains("folder no longer exists")
        || lower.contains("mailbox does not exist")
        || lower.contains("invalid credentials")
        || lower.contains("no such mailbox")
        || lower.contains("imapuidvalidity")
        || lower.contains("uid validity changed")
        || ((lower.contains("folder") || lower.contains("mailbox"))
            && (lower.contains("no longer exists")
                || lower.contains("not found")
                || lower.contains("doesn't exist")
                || lower.contains("nonexistent")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_variant_wins_over_message_heuristics() {
        let permanent = SyncError::Permanent(
            "gmail send failed with status 429: quoted upstream response".into(),
        );
        assert!(permanent.is_permanent());
        assert!(!permanent.is_rate_limited());

        let throttled = SyncError::rate_limited(
            "gmail labels failed with status 403: rateLimitExceeded",
            Some(std::time::Duration::from_secs(120)),
        );
        assert!(throttled.is_rate_limited());
        assert!(!throttled.is_permanent());
        assert_eq!(
            throttled.retry_after(),
            Some(std::time::Duration::from_secs(120))
        );
    }

    #[test]
    fn delivered_failures_are_terminal_without_becoming_unsent() {
        let delivered = SyncError::Delivered("local sent record failed".into());
        assert!(delivered.was_delivered());
        assert!(delivered.is_permanent());
        assert!(!delivered.is_rate_limited());
        assert_eq!(delivered.retry_after(), None);
    }

    #[test]
    fn legacy_rate_limit_messages_are_recognised() {
        for message in [
            "User-rate limit exceeded",
            "gmail x failed with status 429 Too Many Requests",
            "outlook y failed with status 403: rateLimitExceeded",
        ] {
            assert!(SyncError::from_message(message).is_rate_limited());
        }
    }

    #[test]
    fn legacy_permanent_messages_are_recognised() {
        for message in [
            "smtp send failed: authentication failed",
            "rejected: missing recipient",
            "gmail x failed with status 403: forbidden",
            "outlook delta failed with status 410: gone",
        ] {
            assert!(SyncError::from_message(message).is_permanent());
        }
        assert!(SyncError::from_message(
            "gmail x failed with status 403: User-rate limit exceeded."
        )
        .is_rate_limited());
    }

    #[test]
    fn a_queue_id_that_happens_to_contain_429_is_not_a_rate_limit() {
        for message in [
            "smtp send failed: 451 4.3.0 ok: queued as 8B4291C3",
            "smtp send failed: 250 2.0.0 Ok: queued as B429F1",
            "imap fetch failed: uid 1429 unavailable",
        ] {
            assert!(
                !SyncError::from_message(message).is_rate_limited(),
                "{message}"
            );
        }
    }

    #[test]
    fn a_standalone_429_still_reads_as_a_rate_limit() {
        for message in [
            "HTTP status 429",
            "gmail send failed with status 429: quota",
            "outlook returned (429)",
        ] {
            assert!(
                SyncError::from_message(message).is_rate_limited(),
                "{message}"
            );
        }
    }

    #[test]
    fn unknown_messages_are_transient() {
        for message in [
            "connection reset by peer",
            "imap read timeout",
            "gmail z failed with status 500: backend error",
        ] {
            assert!(matches!(
                SyncError::from_message(message),
                SyncError::Transient(_)
            ));
        }
    }

    #[test]
    fn http_failures_keep_the_status_classification_without_tags() {
        assert!(http_failure("gmail", "labels", 429, "RESOURCE_EXHAUSTED").is_rate_limited());
        assert_eq!(
            http_failure("gmail", "labels", 429, "RESOURCE_EXHAUSTED")
                .with_retry_after(std::time::Duration::from_secs(90))
                .retry_after(),
            Some(std::time::Duration::from_secs(90))
        );
        assert!(http_failure(
            "gmail",
            "labels",
            403,
            r#"{"error":{"reason":"rateLimitExceeded"}}"#,
        )
        .is_rate_limited());
        assert!(http_failure("gmail", "send", 400, "bad request").is_permanent());
        assert!(http_failure("gmail", "send", 500, "backend").is_delivery_unknown());
        assert!(http_failure("outlook", "send", 504, "timeout").is_delivery_unknown());
        assert!(http_failure("outlook", "send", 408, "timeout").is_delivery_unknown());
        assert!(matches!(
            http_failure("gmail", "history", 500, "backend"),
            SyncError::Transient(_)
        ));
    }

    #[test]
    fn temporary_failures_and_token_refresh_races_stay_retryable() {
        for message in [
            "offline",
            "DNS lookup failed",
            "connection reset",
            "timeout",
            "gmail labels failed with status 503",
            "oauth refresh failed with status 500",
            "gmail labels failed with status 401: expired access token",
        ] {
            assert!(
                !SyncError::from_message(message).is_permanent(),
                "{message}"
            );
            assert!(SyncError::recoverable_stored_failure(message), "{message}");
        }
        assert!(SyncError::recoverable_stored_failure(
            "oauth refresh failed from stale config: invalid_grant"
        ));
        assert!(!http_failure("gmail", "labels", 401, "expired token").is_permanent());
        assert!(
            SyncError::Transient("oauth stale refresh: invalid_grant".into())
                .encoded()
                .contains("queueTemporary")
        );
        assert!(http_failure("gmail", "labels", 401, "access revoked").is_permanent());
    }

    #[test]
    fn permanent_causes_have_translatable_reasons_and_are_not_recovered() {
        for (message, key) in [
            ("message too large", "queueMessageTooLarge"),
            ("invalid recipient", "queueRecipientRejected"),
            ("folder not found", "queueFolderChanged"),
            (
                "imap COPY failed: [NONEXISTENT] Mailbox doesn't exist",
                "queueFolderChanged",
            ),
            (
                "oauth refresh status 400: invalid_grant",
                "queueAuthentication",
            ),
            ("authentication failed", "queueAuthentication"),
        ] {
            let error = SyncError::from_message(message);
            assert!(error.is_permanent(), "{message}");
            assert!(error.encoded().contains(key));
            assert!(!SyncError::recoverable_stored_failure(&error.encoded()));
        }
        assert!(!SyncError::recoverable_stored_failure(
            "unknown provider rejection"
        ));
        assert!(!SyncError::recoverable_stored_failure(
            "gmail send failed with status 503"
        ));
        let structured =
            r#"{"key":"backend.errors.imapUidValidityChanged","params":{"folder":"Inbox"}}"#;
        assert_eq!(
            SyncError::Permanent(structured.into()).encoded(),
            structured
        );
    }

    #[test]
    fn string_conversion_is_only_the_outer_seam() {
        let error = SyncError::Permanent("invalid recipient".into());
        let stored = error.to_string();
        assert_eq!(stored, "invalid recipient");
        assert!(!stored.contains("[permanent]"));
    }
}
