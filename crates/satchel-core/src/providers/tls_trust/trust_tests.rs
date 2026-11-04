use std::sync::Arc;

use openssl::asn1::{Asn1Integer, Asn1Time};
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use openssl::x509::{X509Builder, X509NameBuilder, X509};
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sqlx::sqlite::SqlitePoolOptions;

use super::*;

/// The trust state is process-wide; tests that touch it take turns.
static SERIAL: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

struct Ca {
    key: PKey<Private>,
    certificate: X509,
}

fn key() -> PKey<Private> {
    PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap()
}

fn name(common_name: &str) -> openssl::x509::X509Name {
    let mut builder = X509NameBuilder::new().unwrap();
    builder
        .append_entry_by_nid(Nid::COMMONNAME, common_name)
        .unwrap();
    builder.build()
}

fn serial(value: u32) -> Asn1Integer {
    BigNum::from_u32(value).unwrap().to_asn1_integer().unwrap()
}

fn ca() -> Ca {
    named_ca("Satchel Test Root")
}

fn named_ca(common_name: &str) -> Ca {
    let key = key();
    let subject = name(common_name);
    let mut builder = X509Builder::new().unwrap();
    builder.set_version(2).unwrap();
    builder.set_serial_number(&serial(1)).unwrap();
    builder.set_subject_name(&subject).unwrap();
    builder.set_issuer_name(&subject).unwrap();
    builder.set_pubkey(&key).unwrap();
    builder
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::days_from_now(3650).unwrap())
        .unwrap();
    builder
        .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
        .unwrap();
    builder.sign(&key, MessageDigest::sha256()).unwrap();
    Ca {
        key,
        certificate: builder.build(),
    }
}

/// A leaf for `dns_name`, valid from `from` to `until` (unix seconds).
fn leaf(ca: &Ca, dns_name: &str, from: i64, until: i64) -> CertificateDer<'static> {
    let key = key();
    let mut builder = X509Builder::new().unwrap();
    builder.set_version(2).unwrap();
    builder.set_serial_number(&serial(2)).unwrap();
    builder.set_subject_name(&name(dns_name)).unwrap();
    builder
        .set_issuer_name(ca.certificate.subject_name())
        .unwrap();
    builder.set_pubkey(&key).unwrap();
    builder
        .set_not_before(&Asn1Time::from_unix(from).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::from_unix(until).unwrap())
        .unwrap();
    builder
        .append_extension(
            SubjectAlternativeName::new()
                .dns(dns_name)
                .build(&builder.x509v3_context(Some(&ca.certificate), None))
                .unwrap(),
        )
        .unwrap();
    builder.sign(&ca.key, MessageDigest::sha256()).unwrap();
    CertificateDer::from(builder.build().to_der().unwrap())
}

/// The login the test account signs in with.
const LOGIN: &str = "me@example.test";

fn verifier(ca: &Ca) -> MailCertVerifier {
    verifier_for(ca, LOGIN)
}

fn verifier_for(ca: &Ca, login: &str) -> MailCertVerifier {
    MailCertVerifier::new(webpki(ca), login)
}

fn webpki(ca: &Ca) -> Arc<dyn ServerCertVerifier> {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(ca.certificate.to_der().unwrap()))
        .unwrap();
    rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap()
}

fn verify(
    verifier: &MailCertVerifier,
    certificate: &CertificateDer<'_>,
    host: &str,
) -> Result<ServerCertVerified, rustls::Error> {
    let server_name = ServerName::try_from(host.to_string()).unwrap();
    verifier.verify_server_cert(certificate, &[], &server_name, &[], UnixTime::now())
}

fn now() -> i64 {
    Utc::now().timestamp()
}

const DAY: i64 = 86_400;

async fn pool_with_account(account_id: &str) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::init(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO accounts (id, display_name, email, provider, config, credentials_ref, created_at, updated_at)
         VALUES (?, 'Work', 'me@example.test', 'imap', '{}', '', '2026-01-01', '2026-01-01')",
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn an_expired_but_otherwise_valid_certificate_is_rejected_and_may_be_excepted() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "expired.example.test";
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);

    let error = verify(&verifier(&ca), &certificate, host).expect_err("expired");

    assert_eq!(RejectionReason::of(&error), Some(RejectionReason::Expired));
    let rejection = last_rejection(host).expect("recorded");
    assert_eq!(rejection.reason, RejectionReason::Expired);
    assert!(rejection.exception_allowed);
    assert_eq!(rejection.subject.as_deref(), Some(host));
    assert_eq!(rejection.issuer.as_deref(), Some("Satchel Test Root"));
    assert_eq!(rejection.fingerprint, fingerprint(certificate.as_ref()));
    assert!(rejection
        .not_after
        .is_some_and(|value| (value - (now() - DAY)).abs() < 5));
}

#[tokio::test]
async fn an_expired_certificate_for_another_name_may_not_be_excepted() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let certificate = leaf(&ca, "other.example.test", now() - 365 * DAY, now() - DAY);

    verify(&verifier(&ca), &certificate, "named.example.test").expect_err("expired");

    let rejection = last_rejection("named.example.test").expect("recorded");
    assert_eq!(rejection.reason, RejectionReason::Expired);
    assert!(
        !rejection.exception_allowed,
        "name mismatch hides behind the expiry"
    );
}

#[tokio::test]
async fn an_unknown_issuer_is_named_and_never_excepted() {
    let _turn = SERIAL.lock().await;
    let trusted = ca();
    let stranger = named_ca("Somebody Else's Root");
    let host = "stranger.example.test";
    let certificate = leaf(&stranger, host, now() - DAY, now() + DAY);

    verify(&verifier(&trusted), &certificate, host).expect_err("untrusted");

    let rejection = last_rejection(host).expect("recorded");
    assert_eq!(rejection.reason, RejectionReason::UnknownIssuer);
    assert!(!rejection.exception_allowed);
}

#[tokio::test]
async fn a_granted_exception_accepts_exactly_that_certificate() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "pinned.example.test";
    let pool = pool_with_account("acc-pinned").await;
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    let verifier = verifier(&ca);
    verify(&verifier, &certificate, host).expect_err("expired");

    let exception = grant(
        &pool,
        "acc-pinned",
        host,
        LOGIN,
        &fingerprint(certificate.as_ref()),
    )
    .await
    .expect("granted");

    assert!(exception.expires_at > Utc::now() + chrono::Duration::days(EXCEPTION_DAYS - 1));
    verify(&verifier, &certificate, host).expect("accepted under the exception");
    let replacement = leaf(&ca, host, now() - 365 * DAY, now() - 2 * DAY);
    verify(&verifier, &replacement, host).expect_err("a different expired certificate");
    let stored = TlsExceptionRepository::new(&pool).list().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].fingerprint, fingerprint(certificate.as_ref()));

    assert!(revoke(&pool, "acc-pinned", host).await.unwrap());
    verify(&verifier, &certificate, host).expect_err("revoked");
}

#[tokio::test]
async fn a_grant_needs_the_certificate_that_was_confirmed() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "confirm.example.test";
    let pool = pool_with_account("acc-confirm").await;

    assert_eq!(
        grant(&pool, "acc-confirm", host, LOGIN, "00").await,
        Err(GrantError::NoRejection)
    );
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    verify(&verifier(&ca), &certificate, host).expect_err("expired");
    assert_eq!(
        grant(&pool, "acc-confirm", host, LOGIN, "00").await,
        Err(GrantError::CertificateChanged)
    );

    let other = leaf(
        &ca,
        "elsewhere.example.test",
        now() - 365 * DAY,
        now() - DAY,
    );
    verify(&verifier(&ca), &other, host).expect_err("expired and misnamed");
    assert_eq!(
        grant(
            &pool,
            "acc-confirm",
            host,
            LOGIN,
            &fingerprint(other.as_ref())
        )
        .await,
        Err(GrantError::NotAllowed)
    );
}

/// Moves an exception's creation and last use into the past, as if it had
/// been granted long ago and its certificate not been needed since.
async fn age_exception(pool: &SqlitePool, host: &str, hours: i64) {
    let then = timestamp(Utc::now() - chrono::Duration::hours(hours));
    sqlx::query(
        "UPDATE tls_certificate_exceptions SET created_at = ?, last_used_at = ? WHERE host = ?",
    )
    .bind(&then)
    .bind(&then)
    .bind(host)
    .execute(pool)
    .await
    .unwrap();
    refresh(pool).await.unwrap();
}

#[tokio::test]
async fn a_valid_certificate_ends_an_exception_that_is_no_longer_needed() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "renewed.example.test";
    let pool = pool_with_account("acc-renewed").await;
    let verifier = verifier(&ca);
    let expired = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    verify(&verifier, &expired, host).expect_err("expired");
    grant(
        &pool,
        "acc-renewed",
        host,
        LOGIN,
        &fingerprint(expired.as_ref()),
    )
    .await
    .unwrap();
    age_exception(&pool, host, QUIET_HOURS + 1).await;

    let renewed = leaf(&ca, host, now() - DAY, now() + 365 * DAY);
    verify(&verifier, &renewed, host).expect("valid");
    let ended = settle(&pool).await.unwrap();

    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].host, host);
    assert!(TlsExceptionRepository::new(&pool)
        .list()
        .await
        .unwrap()
        .is_empty());
    assert!(active_exceptions()
        .iter()
        .all(|exception| exception.host != host));
    assert!(last_rejection(host).is_none());
}

// One node behind a load balancer is renewed, another still serves the
// expired certificate. Meeting the renewed one must not take the exception
// away from the connections that still land on the old one.
#[tokio::test]
async fn an_exception_survives_a_renewed_node_while_the_old_one_still_answers() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "balanced.example.test";
    let pool = pool_with_account("acc-balanced").await;
    let verifier = verifier(&ca);
    let stale_node = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    let renewed_node = leaf(&ca, host, now() - DAY, now() + 365 * DAY);
    verify(&verifier, &stale_node, host).expect_err("expired");
    grant(
        &pool,
        "acc-balanced",
        host,
        LOGIN,
        &fingerprint(stale_node.as_ref()),
    )
    .await
    .unwrap();
    age_exception(&pool, host, QUIET_HOURS + 1).await;

    verify(&verifier, &stale_node, host).expect("pinned node, under the exception");
    verify(&verifier, &renewed_node, host).expect("renewed node");
    assert!(settle(&pool).await.unwrap().is_empty());

    verify(&verifier, &stale_node, host).expect("the old node still works");
    let stored = TlsExceptionRepository::new(&pool).list().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert!(stored[0].last_used_at.is_some());
}

#[tokio::test]
async fn an_exception_covers_only_the_account_that_granted_it() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "shared.example.test";
    let pool = pool_with_account("acc-mine").await;
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    verify(&verifier(&ca), &certificate, host).expect_err("expired");
    grant(
        &pool,
        "acc-mine",
        host,
        LOGIN,
        &fingerprint(certificate.as_ref()),
    )
    .await
    .unwrap();

    verify(&verifier_for(&ca, "ME@EXAMPLE.TEST"), &certificate, host).expect("same login");
    verify(
        &verifier_for(&ca, "colleague@example.test"),
        &certificate,
        host,
    )
    .expect_err("another account on the same server");
}

#[tokio::test]
async fn hosts_are_matched_without_regard_to_case() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "case.example.test";
    let pool = pool_with_account("acc-case").await;
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    verify(&verifier(&ca), &certificate, "CASE.example.test").expect_err("expired");
    assert!(last_rejection("Case.Example.Test").is_some());
    grant(
        &pool,
        "acc-case",
        "Case.Example.Test",
        LOGIN,
        &fingerprint(certificate.as_ref()),
    )
    .await
    .unwrap();

    verify(&verifier(&ca), &certificate, host).expect("same host, other spelling");
}

#[tokio::test]
async fn an_exception_past_its_end_is_refused_before_any_refresh() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "lapsed.example.test";
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    state().exceptions.push(CertificateException {
        account_id: "acc-lapsed".into(),
        host: host.into(),
        login: LOGIN.into(),
        fingerprint: fingerprint(certificate.as_ref()),
        expires_at: Utc::now() - chrono::Duration::minutes(1),
        created_at: Utc::now() - chrono::Duration::days(EXCEPTION_DAYS),
        last_used_at: None,
    });

    let refused = verify(&verifier(&ca), &certificate, host);
    state()
        .exceptions
        .retain(|exception| exception.host != host);

    refused.expect_err("expired exception");
}

#[tokio::test]
async fn a_certificate_that_is_not_valid_yet_is_named_and_never_excepted() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "early.example.test";
    let certificate = leaf(&ca, host, now() + DAY, now() + 365 * DAY);

    verify(&verifier(&ca), &certificate, host).expect_err("not valid yet");

    let rejection = last_rejection(host).expect("recorded");
    assert_eq!(rejection.reason, RejectionReason::NotYetValid);
    assert!(!rejection.exception_allowed);
}

#[tokio::test]
async fn an_expired_certificate_from_an_unknown_issuer_may_not_be_excepted() {
    let _turn = SERIAL.lock().await;
    let trusted = ca();
    let stranger = named_ca("Somebody Else's Root");
    let host = "stranger-expired.example.test";
    let certificate = leaf(&stranger, host, now() - 365 * DAY, now() - DAY);

    verify(&verifier(&trusted), &certificate, host).expect_err("untrusted and expired");

    assert!(!last_rejection(host).expect("recorded").exception_allowed);
}

/// Apple's platform verifier: any expiry comes back as an untyped error.
#[derive(Debug)]
struct UntypedExpiry(Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for UntypedExpiry {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map_err(|error| match error {
                rustls::Error::InvalidCertificate(
                    CertificateError::Expired
                    | CertificateError::ExpiredContext { .. }
                    | CertificateError::NotValidYet
                    | CertificateError::NotValidYetContext { .. },
                ) => rustls::Error::InvalidCertificate(CertificateError::Other(
                    rustls::OtherError(Arc::new(std::io::Error::other("errSecCertificateExpired"))),
                )),
                other => other,
            })
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

#[tokio::test]
async fn an_untyped_platform_error_is_read_from_the_certificate_dates() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let apple = MailCertVerifier::new(Arc::new(UntypedExpiry(webpki(&ca))), LOGIN);
    let expired_host = "apple-expired.example.test";
    let early_host = "apple-early.example.test";
    let expired = leaf(&ca, expired_host, now() - 365 * DAY, now() - DAY);
    let early = leaf(&ca, early_host, now() + DAY, now() + 365 * DAY);

    verify(&apple, &expired, expired_host).expect_err("expired");
    verify(&apple, &early, early_host).expect_err("not valid yet");

    let expired = last_rejection(expired_host).expect("recorded");
    assert_eq!(expired.reason, RejectionReason::Expired);
    assert!(
        expired.exception_allowed,
        "chain and name verify inside the window"
    );
    assert_eq!(
        last_rejection(early_host).expect("recorded").reason,
        RejectionReason::NotYetValid
    );
}

#[tokio::test]
async fn refresh_drops_exceptions_that_ran_out() {
    let _turn = SERIAL.lock().await;
    let pool = pool_with_account("acc-old").await;
    TlsExceptionRepository::new(&pool)
        .upsert(&TlsExceptionRecord {
            account_id: "acc-old".into(),
            host: "old.example.test".into(),
            login: LOGIN.into(),
            fingerprint: "AA".into(),
            expires_at: timestamp(Utc::now() - chrono::Duration::minutes(1)),
            created_at: timestamp(Utc::now() - chrono::Duration::days(EXCEPTION_DAYS)),
            last_used_at: None,
        })
        .await
        .unwrap();

    refresh(&pool).await.unwrap();

    assert!(TlsExceptionRepository::new(&pool)
        .list()
        .await
        .unwrap()
        .is_empty());
    assert!(active_exceptions()
        .iter()
        .all(|exception| exception.host != "old.example.test"));
}

#[tokio::test]
async fn a_rejected_handshake_becomes_a_keyed_error_with_the_certificate_facts() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "keyed.example.test";
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    let error = verify(&verifier(&ca), &certificate, host).expect_err("expired");
    let io_error = std::io::Error::new(std::io::ErrorKind::InvalidData, error);

    let encoded: serde_json::Value =
        serde_json::from_str(&handshake_error("imap", host, 993, &io_error)).unwrap();

    assert_eq!(encoded["key"], REJECTION_KEY);
    let params = &encoded["params"];
    assert_eq!(params["host"], host);
    assert_eq!(params["port"], "993");
    assert_eq!(params["protocol"], "imap");
    assert_eq!(params["reason"], "expired");
    assert_eq!(params["exceptionAllowed"], true);
    assert_eq!(params["fingerprint"], fingerprint(certificate.as_ref()));
    assert!(params["notAfter"]
        .as_str()
        .is_some_and(|value| value.ends_with('Z')));
}

#[test]
fn other_handshake_failures_keep_their_wording() {
    let error = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");

    assert_eq!(
        handshake_error("imap", "plain.example.test", 993, &error),
        "imap tls handshake failed: reset by peer"
    );
}
