//! What a mail server's certificate said when the handshake failed, and the
//! one exception a person may grant: an expired certificate that is otherwise
//! valid, accepted for at most [`EXCEPTION_DAYS`] days.
//!
//! [`MailCertVerifier`] wraps the platform verifier (bundled roots on desktop,
//! the OS trust store on mobile). It never relaxes anything on its own:
//!
//! - A rejection is recorded per host, so the handshake error can name the
//!   reason, the certificate and its dates instead of a rustls error chain.
//! - An expired certificate passes only when an exception pins its exact
//!   SHA-256 fingerprint for that host, and only when the chain and the host
//!   name verify at a moment inside the certificate's validity window. A wrong
//!   name, an unknown issuer or a certificate that is not yet valid always
//!   fails.
//! - Once the host presents a valid certificate again, its exceptions end
//!   ([`settle`]). They also end on their expiry date or when revoked.
//!
//! Exceptions live in `tls_certificate_exceptions` and are mirrored into
//! process memory, because a verifier runs synchronously inside the handshake.
//! See `docs/adr/0014-abgelaufene-zertifikate.md`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::db::tls_exceptions::{TlsExceptionRecord, TlsExceptionRepository};

/// How long an exception lasts at most.
pub const EXCEPTION_DAYS: i64 = 7;

/// The key a rejected certificate travels under, inside the account's sync error.
pub const REJECTION_KEY: &str = "backend.errors.tlsCertificateRejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    Expired,
    NotYetValid,
    NameMismatch,
    UnknownIssuer,
    Other,
}

impl RejectionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::NotYetValid => "notYetValid",
            Self::NameMismatch => "nameMismatch",
            Self::UnknownIssuer => "unknownIssuer",
            Self::Other => "other",
        }
    }

    fn of(error: &rustls::Error) -> Option<Self> {
        let rustls::Error::InvalidCertificate(error) = error else {
            return None;
        };
        Some(match error {
            CertificateError::Expired | CertificateError::ExpiredContext { .. } => Self::Expired,
            CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                Self::NotYetValid
            }
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                Self::NameMismatch
            }
            CertificateError::UnknownIssuer => Self::UnknownIssuer,
            _ => Self::Other,
        })
    }
}

/// What the server presented when its certificate was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateRejection {
    pub host: String,
    pub reason: RejectionReason,
    /// Common name of the certificate's subject.
    pub subject: Option<String>,
    /// Common name (or organisation) of the issuer.
    pub issuer: Option<String>,
    /// Unix seconds.
    pub not_before: Option<i64>,
    /// Unix seconds.
    pub not_after: Option<i64>,
    pub fingerprint: String,
    /// Expiry is the only fault: chain and name verify inside the validity window.
    pub exception_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateException {
    pub account_id: String,
    pub host: String,
    pub fingerprint: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Default)]
struct TrustState {
    exceptions: Vec<CertificateException>,
    rejections: HashMap<String, CertificateRejection>,
    /// Hosts with an exception that presented a valid certificate since the last [`settle`].
    valid_again: HashSet<String>,
    /// When an exception was last reported as used, per host, to keep the log readable.
    last_use_logged: HashMap<String, DateTime<Utc>>,
}

static STATE: LazyLock<Mutex<TrustState>> = LazyLock::new(Mutex::default);

fn state() -> MutexGuard<'static, TrustState> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn host_key(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// RFC 3339 in UTC with whole seconds, so stored values sort as text.
pub fn timestamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn unix_to_rfc3339(seconds: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(seconds, 0).map(timestamp)
}

/// SHA-256 of the DER bytes, as upper-case hex pairs joined by `:`.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The last rejection recorded for a host in this process.
pub fn last_rejection(host: &str) -> Option<CertificateRejection> {
    state().rejections.get(&host_key(host)).cloned()
}

/// The exceptions currently in force (as last loaded from the database).
pub fn active_exceptions() -> Vec<CertificateException> {
    state().exceptions.clone()
}

/// Turns a failed handshake into the error string the sync path carries.
///
/// A rejected certificate becomes an encoded [`REJECTION_KEY`] with the facts
/// the UI shows. Anything else keeps the wording it always had.
pub fn handshake_error(protocol: &str, host: &str, port: u16, error: &std::io::Error) -> String {
    let rejected = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .and_then(RejectionReason::of)
        .is_some();
    match rejected.then(|| last_rejection(host)).flatten() {
        Some(rejection) => encode_rejection(protocol, port, &rejection),
        None => format!("{protocol} tls handshake failed: {error}"),
    }
}

fn encode_rejection(protocol: &str, port: u16, rejection: &CertificateRejection) -> String {
    serde_json::json!({
        "key": REJECTION_KEY,
        "params": {
            "protocol": protocol,
            "host": rejection.host,
            "port": port.to_string(),
            "reason": rejection.reason.as_str(),
            "subject": rejection.subject,
            "issuer": rejection.issuer,
            "notBefore": rejection.not_before.and_then(unix_to_rfc3339),
            "notAfter": rejection.not_after.and_then(unix_to_rfc3339),
            "fingerprint": rejection.fingerprint,
            "exceptionAllowed": rejection.exception_allowed,
        }
    })
    .to_string()
}

/// Loads the exceptions in force into memory and drops the ones that ran out.
///
/// Cheap (a handful of rows at most), so every sync run calls it: a mobile
/// background worker runs in its own process and has no other way to learn
/// about an exception granted in the app.
pub async fn refresh(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let repository = TlsExceptionRepository::new(pool);
    for ended in repository.delete_expired(&timestamp(Utc::now())).await? {
        tracing::info!(account_id = %ended.account_id, host = %ended.host,
            "certificate exception expired");
    }
    let exceptions = repository
        .list()
        .await?
        .into_iter()
        .filter_map(|record| exception_from_record(&record))
        .collect();
    state().exceptions = exceptions;
    Ok(())
}

/// Ends the exceptions of every host that presented a valid certificate since
/// the last call. Returns the exceptions it removed.
pub async fn settle(pool: &SqlitePool) -> Result<Vec<CertificateException>, sqlx::Error> {
    let hosts: Vec<String> = state().valid_again.drain().collect();
    if hosts.is_empty() {
        return Ok(Vec::new());
    }
    let repository = TlsExceptionRepository::new(pool);
    let mut ended = Vec::new();
    for host in hosts {
        for record in repository.delete_host(&host).await? {
            tracing::info!(account_id = %record.account_id, host = %record.host,
                "server presents a valid certificate again, certificate exception ended");
            if let Some(exception) = exception_from_record(&record) {
                ended.push(exception);
            }
        }
    }
    refresh(pool).await?;
    Ok(ended)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// No rejection of this host is on record in this process.
    NoRejection,
    /// The server now presents a different certificate than the one confirmed.
    CertificateChanged,
    /// The certificate has a fault besides its expiry.
    NotAllowed,
    Persistence(String),
}

/// Accepts the expired certificate the host presented last, for this account,
/// for [`EXCEPTION_DAYS`] days. `fingerprint` is the one the person confirmed;
/// it has to be the one on record.
pub async fn grant(
    pool: &SqlitePool,
    account_id: &str,
    host: &str,
    fingerprint: &str,
) -> Result<CertificateException, GrantError> {
    let rejection = last_rejection(host).ok_or(GrantError::NoRejection)?;
    if !rejection.fingerprint.eq_ignore_ascii_case(fingerprint) {
        return Err(GrantError::CertificateChanged);
    }
    if !rejection.exception_allowed {
        return Err(GrantError::NotAllowed);
    }
    let now = Utc::now();
    let record = TlsExceptionRecord {
        account_id: account_id.to_string(),
        host: host_key(host),
        fingerprint: rejection.fingerprint.clone(),
        expires_at: timestamp(now + chrono::Duration::days(EXCEPTION_DAYS)),
        created_at: timestamp(now),
    };
    let repository = TlsExceptionRepository::new(pool);
    repository
        .upsert(&record)
        .await
        .map_err(|error| GrantError::Persistence(error.to_string()))?;
    refresh(pool)
        .await
        .map_err(|error| GrantError::Persistence(error.to_string()))?;
    tracing::info!(account_id, host = %record.host, fingerprint = %record.fingerprint,
        expires_at = %record.expires_at, "certificate exception granted");
    exception_from_record(&record)
        .ok_or_else(|| GrantError::Persistence("stored exception is unreadable".to_string()))
}

/// Ends an exception before its time. Returns whether there was one.
pub async fn revoke(pool: &SqlitePool, account_id: &str, host: &str) -> Result<bool, sqlx::Error> {
    let removed = TlsExceptionRepository::new(pool)
        .delete(account_id, &host_key(host))
        .await?;
    refresh(pool).await?;
    if removed > 0 {
        tracing::info!(account_id, host = %host_key(host), "certificate exception revoked");
    }
    Ok(removed > 0)
}

fn exception_from_record(record: &TlsExceptionRecord) -> Option<CertificateException> {
    let parse = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|time| time.with_timezone(&Utc))
    };
    Some(CertificateException {
        account_id: record.account_id.clone(),
        host: host_key(&record.host),
        fingerprint: record.fingerprint.clone(),
        expires_at: parse(&record.expires_at)?,
        created_at: parse(&record.created_at)?,
    })
}

/// Whether an exception in force pins this certificate for this host.
fn exception_covers(host: &str, fingerprint: &str, now: DateTime<Utc>) -> bool {
    state().exceptions.iter().any(|exception| {
        exception.host == host
            && exception.fingerprint.eq_ignore_ascii_case(fingerprint)
            && exception.expires_at > now
    })
}

fn note_valid_certificate(host: &str) {
    let mut state = state();
    if state
        .exceptions
        .iter()
        .any(|exception| exception.host == host)
    {
        state.valid_again.insert(host.to_string());
    }
    state.rejections.remove(host);
}

fn note_exception_used(host: &str, now: DateTime<Utc>) {
    let mut state = state();
    let due = state
        .last_use_logged
        .get(host)
        .is_none_or(|last| now - *last >= chrono::Duration::hours(1));
    if due {
        state.last_use_logged.insert(host.to_string(), now);
        drop(state);
        tracing::info!(
            host,
            "accepted an expired certificate under a certificate exception"
        );
    }
}

fn record_rejection(rejection: CertificateRejection) {
    let mut state = state();
    // Every handshake meets the same certificate again; the log hears about it
    // once per certificate and reason.
    let repeated = state
        .rejections
        .get(&rejection.host)
        .is_some_and(|previous| {
            previous.fingerprint == rejection.fingerprint && previous.reason == rejection.reason
        });
    if repeated {
        tracing::debug!(host = %rejection.host, "mail server certificate rejected again");
    } else {
        tracing::warn!(
            host = %rejection.host,
            reason = rejection.reason.as_str(),
            subject = rejection.subject.as_deref().unwrap_or(""),
            not_after = rejection.not_after.and_then(unix_to_rfc3339).as_deref().unwrap_or(""),
            fingerprint = %rejection.fingerprint,
            exception_allowed = rejection.exception_allowed,
            "mail server certificate rejected"
        );
    }
    state.rejections.insert(rejection.host.clone(), rejection);
}

/// Subject, issuer and validity, read with the OpenSSL that S/MIME already ships.
#[derive(Debug, Default)]
struct CertificateFacts {
    subject: Option<String>,
    issuer: Option<String>,
    not_before: Option<i64>,
    not_after: Option<i64>,
}

impl CertificateFacts {
    fn parse(der: &[u8]) -> Self {
        use openssl::asn1::{Asn1Time, Asn1TimeRef};
        use openssl::nid::Nid;
        use openssl::x509::{X509NameRef, X509};

        let Ok(certificate) = X509::from_der(der) else {
            return Self::default();
        };
        let name = |name: &X509NameRef| {
            [Nid::COMMONNAME, Nid::ORGANIZATIONNAME]
                .into_iter()
                .find_map(|nid| name.entries_by_nid(nid).next())
                .and_then(|entry| entry.data().to_string().ok())
        };
        let epoch = Asn1Time::from_unix(0).ok();
        let unix = |time: &Asn1TimeRef| {
            let diff = epoch.as_ref()?.diff(time).ok()?;
            Some(i64::from(diff.days) * 86_400 + i64::from(diff.secs))
        };
        Self {
            subject: name(certificate.subject_name()),
            issuer: name(certificate.issuer_name()),
            not_before: unix(certificate.not_before()),
            not_after: unix(certificate.not_after()),
        }
    }
}

/// The verifier every IMAP and POP3 connection uses.
#[derive(Debug)]
pub struct MailCertVerifier {
    inner: Arc<dyn ServerCertVerifier>,
}

impl MailCertVerifier {
    pub fn new(inner: Arc<dyn ServerCertVerifier>) -> Self {
        Self { inner }
    }

    /// Whether the chain and the name verify at a moment the certificate was
    /// still valid. That separates "someone forgot to renew it" from "this is
    /// not the server's certificate".
    fn only_expired(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        not_after: Option<i64>,
    ) -> bool {
        let Some(moment) = not_after
            .and_then(|seconds| u64::try_from(seconds - 1).ok())
            .map(|seconds| UnixTime::since_unix_epoch(Duration::from_secs(seconds)))
        else {
            return false;
        };
        self.inner
            .verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                moment,
            )
            .is_ok()
    }
}

impl ServerCertVerifier for MailCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let host = host_key(&server_name.to_str());
        let error = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => {
                note_valid_certificate(&host);
                return Ok(verified);
            }
            Err(error) => error,
        };
        let Some(reason) = RejectionReason::of(&error) else {
            return Err(error);
        };
        let facts = CertificateFacts::parse(end_entity.as_ref());
        let fingerprint = fingerprint(end_entity.as_ref());
        // Apple's verifier reports an expired certificate as an untyped
        // "other" error. The dates say what it was; the second verification
        // below still decides whether an exception may be offered.
        let checked_at = now.as_secs() as i64;
        let reason = match reason {
            RejectionReason::Other if facts.not_after.is_some_and(|end| end < checked_at) => {
                RejectionReason::Expired
            }
            RejectionReason::Other if facts.not_before.is_some_and(|start| start > checked_at) => {
                RejectionReason::NotYetValid
            }
            reason => reason,
        };
        let exception_allowed = reason == RejectionReason::Expired
            && self.only_expired(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                facts.not_after,
            );
        let wall_clock = DateTime::<Utc>::from_timestamp(checked_at, 0).unwrap_or_else(Utc::now);
        if exception_allowed && exception_covers(&host, &fingerprint, wall_clock) {
            note_exception_used(&host, wall_clock);
            return Ok(ServerCertVerified::assertion());
        }
        record_rejection(CertificateRejection {
            host,
            reason,
            subject: facts.subject,
            issuer: facts.issuer,
            not_before: facts.not_before,
            not_after: facts.not_after,
            fingerprint,
            exception_allowed,
        });
        Err(error)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

#[cfg(test)]
#[path = "tls_trust/trust_tests.rs"]
mod tests;
