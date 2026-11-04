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

fn verifier(ca: &Ca) -> MailCertVerifier {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from(ca.certificate.to_der().unwrap()))
        .unwrap();
    let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::ring::default_provider()),
    )
    .build()
    .unwrap();
    MailCertVerifier::new(inner)
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
        grant(&pool, "acc-confirm", host, "00").await,
        Err(GrantError::NoRejection)
    );
    let certificate = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    verify(&verifier(&ca), &certificate, host).expect_err("expired");
    assert_eq!(
        grant(&pool, "acc-confirm", host, "00").await,
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
        grant(&pool, "acc-confirm", host, &fingerprint(other.as_ref())).await,
        Err(GrantError::NotAllowed)
    );
}

#[tokio::test]
async fn a_valid_certificate_ends_the_exception_for_its_host() {
    let _turn = SERIAL.lock().await;
    let ca = ca();
    let host = "renewed.example.test";
    let pool = pool_with_account("acc-renewed").await;
    let verifier = verifier(&ca);
    let expired = leaf(&ca, host, now() - 365 * DAY, now() - DAY);
    verify(&verifier, &expired, host).expect_err("expired");
    grant(&pool, "acc-renewed", host, &fingerprint(expired.as_ref()))
        .await
        .unwrap();

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

#[tokio::test]
async fn refresh_drops_exceptions_that_ran_out() {
    let _turn = SERIAL.lock().await;
    let pool = pool_with_account("acc-old").await;
    TlsExceptionRepository::new(&pool)
        .upsert(&TlsExceptionRecord {
            account_id: "acc-old".into(),
            host: "old.example.test".into(),
            fingerprint: "AA".into(),
            expires_at: timestamp(Utc::now() - chrono::Duration::minutes(1)),
            created_at: timestamp(Utc::now() - chrono::Duration::days(EXCEPTION_DAYS)),
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
