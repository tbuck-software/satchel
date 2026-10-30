use std::{
    fmt::Write as _,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    sync::OnceLock,
};

use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::{BigNum, MsbOption},
    cms::CmsContentInfo,
    error::ErrorStack,
    hash::MessageDigest,
    nid::Nid,
    pkcs12::Pkcs12,
    pkcs7::{Pkcs7, Pkcs7Flags},
    pkey::{PKey, Private},
    rsa::Rsa,
    stack::Stack,
    symm::Cipher,
    x509::{
        extension::{AuthorityKeyIdentifier, BasicConstraints, SubjectKeyIdentifier},
        store::{X509Store, X509StoreBuilder},
        X509Builder, X509NameBuilder, X509NameRef, X509Ref, X509,
    },
};

#[derive(Debug, Clone)]
pub struct SmimeCertMetadata {
    pub fingerprint: String,
    pub subject_email: Option<String>,
    pub subject_cn: Option<String>,
    pub issuer_cn: Option<String>,
    pub not_before: Option<String>,
    pub not_after: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SmimeVerifyResult {
    /// The signature maths held. On its own this says nothing about trust.
    pub valid: bool,
    /// The signer certificate chained to a trust anchor. Never collapse this
    /// into `valid`: an unchained signature must not look like a checked one.
    pub chain_verified: bool,
    pub signer_cert_pem: Option<String>,
    pub signer: Option<SmimeCertMetadata>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeneratedSmimeKeyPair {
    pub pem_certificate: String,
    pub pem_private_key: String,
    pub metadata: SmimeCertMetadata,
}

#[derive(Debug, Clone)]
pub struct GenerateSmimeRequest<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub passphrase: Option<&'a str>,
}

pub trait SmimeEngine: Send + Sync {
    fn id(&self) -> &'static str;
    fn ensure_available(&self) -> Result<(), String>;
    fn sign(
        &self,
        pem_private_key: &str,
        pem_certificate: &str,
        passphrase: Option<&str>,
        payload: &[u8],
    ) -> Result<Vec<u8>, String>;
    fn encrypt(&self, recipient_pems: &[String], payload: &[u8]) -> Result<Vec<u8>, String>;
    fn decrypt(
        &self,
        pem_private_key: &str,
        pem_certificate: &str,
        passphrase: Option<&str>,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, String>;
    fn verify(&self, signed_payload: &[u8]) -> Result<SmimeVerifyResult, String>;
    fn certificate_metadata(&self, pem_certificate: &str) -> Result<SmimeCertMetadata, String>;
    fn extract_pkcs12(&self, p12_der: &[u8], passphrase: &str) -> Result<(String, String), String>;
    fn generate_keypair(
        &self,
        request: &GenerateSmimeRequest<'_>,
    ) -> Result<GeneratedSmimeKeyPair, String>;
    fn verify_private_key_passphrase(
        &self,
        pem_private_key: &str,
        passphrase: Option<&str>,
    ) -> Result<bool, String>;
    fn extract_certificates(&self, signed_payload: &[u8]) -> Result<Vec<String>, String>;
}

pub fn ensure_engine_available() -> Result<(), String> {
    active_engine().ensure_available()
}

pub fn sign(
    pem_private_key: &str,
    pem_certificate: &str,
    passphrase: Option<&str>,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    active_engine().sign(pem_private_key, pem_certificate, passphrase, payload)
}

pub fn encrypt(recipient_pems: &[String], payload: &[u8]) -> Result<Vec<u8>, String> {
    active_engine().encrypt(recipient_pems, payload)
}

pub fn decrypt(
    pem_private_key: &str,
    pem_certificate: &str,
    passphrase: Option<&str>,
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    active_engine().decrypt(pem_private_key, pem_certificate, passphrase, ciphertext)
}

pub fn verify(signed_payload: &[u8]) -> Result<SmimeVerifyResult, String> {
    active_engine().verify(signed_payload)
}

/// Extract readable signed content only. This deliberately grants no signature
/// trust; callers must run `verify` and apply chain and sender identity rules.
pub fn signed_content(payload: &[u8]) -> Result<Vec<u8>, String> {
    let (pkcs7, detached) = read_pkcs7(payload)?;
    let store = X509StoreBuilder::new().map_err(stack_error)?.build();
    let certificates = Stack::new().map_err(stack_error)?;
    let mut content = Vec::new();
    pkcs7
        .verify(
            &certificates,
            &store,
            detached.as_deref(),
            Some(&mut content),
            Pkcs7Flags::NOVERIFY | Pkcs7Flags::NOSIGS | Pkcs7Flags::BINARY,
        )
        .map_err(stack_error)?;
    Ok(content)
}

pub fn certificate_metadata(pem_certificate: &str) -> Result<SmimeCertMetadata, String> {
    active_engine().certificate_metadata(pem_certificate)
}

pub fn extract_pkcs12(p12_der: &[u8], passphrase: &str) -> Result<(String, String), String> {
    active_engine().extract_pkcs12(p12_der, passphrase)
}

pub fn generate_keypair(
    request: &GenerateSmimeRequest<'_>,
) -> Result<GeneratedSmimeKeyPair, String> {
    active_engine().generate_keypair(request)
}

pub fn verify_private_key_passphrase(
    pem_private_key: &str,
    passphrase: Option<&str>,
) -> Result<bool, String> {
    active_engine().verify_private_key_passphrase(pem_private_key, passphrase)
}

pub fn extract_certificates(signed_payload: &[u8]) -> Result<Vec<String>, String> {
    active_engine().extract_certificates(signed_payload)
}

/// `SATCHEL_SMIME_ENGINE` mirrors `SATCHEL_PGP_ENGINE`. The in-process OpenSSL
/// library is the default; the `openssl` binary stays reachable as a fallback
/// for comparing against a system OpenSSL.
fn active_engine() -> Box<dyn SmimeEngine> {
    let wanted = std::env::var("SATCHEL_SMIME_ENGINE")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();

    match wanted.as_str() {
        "openssl-cli" | "cli" | "binary" | "openssl-bin" => Box::new(OpensslCliEngine),
        _ => Box::new(OpensslLibEngine),
    }
}

// ---------------------------------------------------------------------------
// In-process engine
// ---------------------------------------------------------------------------

/// S/MIME through the linked OpenSSL library.
///
/// Every operation maps onto the same C entry point the `openssl` binary uses,
/// with the same flags, so the wire format is the one the CLI engine produced:
/// `sign` is opaque (`smime -sign -nodetach`), `encrypt` is AES-256-CBC
/// (`smime -encrypt -aes256`), both are written as S/MIME.
struct OpensslLibEngine;

impl OpensslLibEngine {
    /// `anchors` is `None` when the platform offered no trust store at all.
    /// The signature maths is still reported, but fail closed: `chain_verified`
    /// stays false, so nothing downstream can show it as a checked signature.
    #[cfg(test)]
    fn verify_with_anchors(
        &self,
        signed_payload: &[u8],
        anchors: Option<&X509Store>,
    ) -> Result<SmimeVerifyResult, String> {
        match anchors {
            Some(store) => {
                let chained = self.verify_against(signed_payload, store, Pkcs7Flags::empty());
                if chained.valid {
                    return Ok(chained);
                }
                // Chain validation can fail before OpenSSL checks the signed
                // bytes. Check those separately, without granting any trust.
                let mut signature =
                    self.verify_against(signed_payload, store, Pkcs7Flags::NOVERIFY);
                if signature.valid {
                    signature.error = chained.error;
                }
                Ok(signature)
            }
            None => {
                let store = X509StoreBuilder::new().map_err(stack_error)?.build();
                Ok(self.verify_against(signed_payload, &store, Pkcs7Flags::NOVERIFY))
            }
        }
    }

    fn verify_against(
        &self,
        signed_payload: &[u8],
        store: &X509Store,
        flags: Pkcs7Flags,
    ) -> SmimeVerifyResult {
        let failure = |error: String| SmimeVerifyResult {
            valid: false,
            chain_verified: false,
            signer_cert_pem: None,
            signer: None,
            error: Some(error),
        };
        let (pkcs7, detached_content) = match read_pkcs7(signed_payload) {
            Ok(parsed) => parsed,
            Err(err) => return failure(err),
        };
        let empty = match Stack::new() {
            Ok(stack) => stack,
            Err(err) => return failure(stack_error(err)),
        };
        let outcome = pkcs7.verify(&empty, store, detached_content.as_deref(), None, flags);
        // Report the signer either way: an invalid signature still tells the
        // user whose certificate was attached.
        let signer_cert_pem = pkcs7
            .signers(&empty, Pkcs7Flags::empty())
            .ok()
            .and_then(|signers| {
                signers
                    .iter()
                    .next()
                    .and_then(|cert| cert_to_pem(cert).ok())
            })
            .or_else(|| {
                signed_certificates(&pkcs7)
                    .ok()
                    .and_then(|mut certs| certs.drain(..).next())
            });
        let signer = signer_cert_pem
            .as_deref()
            .and_then(|pem| self.certificate_metadata(pem).ok());
        // NOVERIFY skips chain, expiry and purpose, so such a run can only
        // ever report the signature maths.
        let chain_verified = !flags.contains(Pkcs7Flags::NOVERIFY);
        match outcome {
            Ok(()) => SmimeVerifyResult {
                valid: true,
                chain_verified,
                signer_cert_pem,
                signer,
                error: None,
            },
            Err(err) => SmimeVerifyResult {
                valid: false,
                chain_verified: false,
                signer_cert_pem,
                signer,
                error: Some(stack_error(err)),
            },
        }
    }
}

impl SmimeEngine for OpensslLibEngine {
    fn id(&self) -> &'static str {
        "openssl-lib"
    }

    fn ensure_available(&self) -> Result<(), String> {
        Ok(())
    }

    fn sign(
        &self,
        pem_private_key: &str,
        pem_certificate: &str,
        passphrase: Option<&str>,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        let key = load_private_key(pem_private_key, passphrase)?;
        let cert = X509::from_pem(pem_certificate.as_bytes()).map_err(stack_error)?;
        let chain = Stack::new().map_err(stack_error)?;
        // No DETACHED flag: the payload travels inside the structure, which is
        // what `-nodetach` produced.
        let flags = Pkcs7Flags::empty();
        let signed = Pkcs7::sign(&cert, &key, &chain, payload, flags).map_err(stack_error)?;
        signed.to_smime(payload, flags).map_err(stack_error)
    }

    fn encrypt(&self, recipient_pems: &[String], payload: &[u8]) -> Result<Vec<u8>, String> {
        if recipient_pems.is_empty() {
            return Err("at least one S/MIME recipient certificate is required".to_string());
        }
        let mut recipients = Stack::new().map_err(stack_error)?;
        for pem in recipient_pems {
            let cert = X509::from_pem(pem.as_bytes()).map_err(stack_error)?;
            recipients.push(cert).map_err(stack_error)?;
        }
        let flags = Pkcs7Flags::empty();
        let enveloped = Pkcs7::encrypt(&recipients, payload, Cipher::aes_256_cbc(), flags)
            .map_err(stack_error)?;
        enveloped.to_smime(payload, flags).map_err(stack_error)
    }

    fn decrypt(
        &self,
        pem_private_key: &str,
        pem_certificate: &str,
        passphrase: Option<&str>,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, String> {
        let key = load_private_key(pem_private_key, passphrase)?;
        let cert = X509::from_pem(pem_certificate.as_bytes()).map_err(stack_error)?;
        // CMS rather than PKCS#7: senders increasingly use RSAES-OAEP key
        // transport, which the PKCS#7 decryptor rejects.
        let enveloped = read_cms(ciphertext)?;
        enveloped.decrypt(&key, &cert).map_err(stack_error)
    }

    fn verify(&self, signed_payload: &[u8]) -> Result<SmimeVerifyResult, String> {
        let store = X509StoreBuilder::new().map_err(stack_error)?.build();
        let mut result = self.verify_against(signed_payload, &store, Pkcs7Flags::NOVERIFY);
        apply_platform_trust(signed_payload, &mut result);
        Ok(result)
    }

    fn certificate_metadata(&self, pem_certificate: &str) -> Result<SmimeCertMetadata, String> {
        let cert = X509::from_pem(pem_certificate.as_bytes()).map_err(stack_error)?;
        certificate_metadata_of(&cert)
    }

    fn extract_pkcs12(&self, p12_der: &[u8], passphrase: &str) -> Result<(String, String), String> {
        let bundle = Pkcs12::from_der(p12_der).map_err(stack_error)?;
        let parsed = bundle.parse2(passphrase).map_err(stack_error)?;
        let cert = parsed
            .cert
            .ok_or_else(|| "PKCS#12 bundle contains no certificate".to_string())?;
        let key = parsed
            .pkey
            .ok_or_else(|| "PKCS#12 bundle contains no private key".to_string())?;
        Ok((cert_to_pem(&cert)?, private_key_to_pem(&key, None)?))
    }

    fn generate_keypair(
        &self,
        request: &GenerateSmimeRequest<'_>,
    ) -> Result<GeneratedSmimeKeyPair, String> {
        let key = PKey::from_rsa(Rsa::generate(2048).map_err(stack_error)?).map_err(stack_error)?;
        let mut subject = X509NameBuilder::new().map_err(stack_error)?;
        subject
            .append_entry_by_nid(Nid::COMMONNAME, request.name)
            .map_err(stack_error)?;
        subject
            .append_entry_by_nid(Nid::PKCS9_EMAILADDRESS, request.email)
            .map_err(stack_error)?;
        let subject = subject.build();

        // Same shape as `req -x509 -newkey rsa:2048 -days 730`: self-signed,
        // 159-bit random serial, critical CA:TRUE, subject and authority key id.
        let serial = random_serial()?;
        let not_before = Asn1Time::days_from_now(0).map_err(stack_error)?;
        let not_after = Asn1Time::days_from_now(730).map_err(stack_error)?;
        let mut builder = X509Builder::new().map_err(stack_error)?;
        builder.set_version(2).map_err(stack_error)?;
        builder.set_serial_number(&serial).map_err(stack_error)?;
        builder.set_subject_name(&subject).map_err(stack_error)?;
        builder.set_issuer_name(&subject).map_err(stack_error)?;
        builder.set_pubkey(&key).map_err(stack_error)?;
        builder.set_not_before(&not_before).map_err(stack_error)?;
        builder.set_not_after(&not_after).map_err(stack_error)?;
        builder
            .append_extension(
                BasicConstraints::new()
                    .critical()
                    .ca()
                    .build()
                    .map_err(stack_error)?,
            )
            .map_err(stack_error)?;
        let subject_key_id = SubjectKeyIdentifier::new()
            .build(&builder.x509v3_context(None, None))
            .map_err(stack_error)?;
        builder
            .append_extension(subject_key_id)
            .map_err(stack_error)?;
        let authority_key_id = AuthorityKeyIdentifier::new()
            .keyid(false)
            .build(&builder.x509v3_context(None, None))
            .map_err(stack_error)?;
        builder
            .append_extension(authority_key_id)
            .map_err(stack_error)?;
        builder
            .sign(&key, MessageDigest::sha256())
            .map_err(stack_error)?;
        let certificate = builder.build();

        let metadata = certificate_metadata_of(&certificate)?;
        Ok(GeneratedSmimeKeyPair {
            pem_certificate: cert_to_pem(&certificate)?,
            pem_private_key: private_key_to_pem(&key, request.passphrase)?,
            metadata,
        })
    }

    fn verify_private_key_passphrase(
        &self,
        pem_private_key: &str,
        passphrase: Option<&str>,
    ) -> Result<bool, String> {
        match load_private_key(pem_private_key, passphrase) {
            Ok(_) => Ok(true),
            Err(err) if is_wrong_passphrase(pem_private_key, &err) => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn extract_certificates(&self, signed_payload: &[u8]) -> Result<Vec<String>, String> {
        let (pkcs7, _) = read_pkcs7(signed_payload)?;
        signed_certificates(&pkcs7)
    }
}

fn load_private_key(pem: &str, passphrase: Option<&str>) -> Result<PKey<Private>, String> {
    // Always take the passphrase entry point, even without one: the plain
    // variant installs no callback and OpenSSL would prompt on the terminal
    // for an encrypted key.
    let passphrase = passphrase
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    PKey::private_key_from_pem_passphrase(pem.as_bytes(), passphrase.as_bytes())
        .map_err(stack_error)
}

fn private_key_to_pem(key: &PKey<Private>, passphrase: Option<&str>) -> Result<String, String> {
    let pem = match passphrase.map(str::trim).filter(|value| !value.is_empty()) {
        Some(passphrase) => key
            .private_key_to_pem_pkcs8_passphrase(Cipher::aes_256_cbc(), passphrase.as_bytes())
            .map_err(stack_error)?,
        None => key.private_key_to_pem_pkcs8().map_err(stack_error)?,
    };
    String::from_utf8(pem).map_err(|err| err.to_string())
}

fn cert_to_pem(cert: &X509Ref) -> Result<String, String> {
    String::from_utf8(cert.to_pem().map_err(stack_error)?).map_err(|err| err.to_string())
}

fn random_serial() -> Result<Asn1Integer, String> {
    let mut serial = BigNum::new().map_err(stack_error)?;
    serial
        .rand(159, MsbOption::MAYBE_ZERO, false)
        .map_err(stack_error)?;
    serial.to_asn1_integer().map_err(stack_error)
}

/// The engine is handed whatever the MIME parser pulled out of the message:
/// S/MIME for `multipart/signed`, bare DER for an opaque
/// `application/pkcs7-mime` part. Accept all three encodings.
fn read_pkcs7(input: &[u8]) -> Result<(Pkcs7, Option<Vec<u8>>), String> {
    if let Ok(parsed) = Pkcs7::from_smime(input) {
        return Ok(parsed);
    }
    if let Ok(pkcs7) = Pkcs7::from_der(input) {
        return Ok((pkcs7, None));
    }
    Pkcs7::from_pem(input)
        .map(|pkcs7| (pkcs7, None))
        .map_err(|err| {
            format!(
                "input is not a readable S/MIME, DER or PEM PKCS#7 structure: {}",
                stack_error(err)
            )
        })
}

fn read_cms(input: &[u8]) -> Result<CmsContentInfo, String> {
    if let Ok(cms) = CmsContentInfo::smime_read_cms(input) {
        return Ok(cms);
    }
    if let Ok(cms) = CmsContentInfo::from_der(input) {
        return Ok(cms);
    }
    CmsContentInfo::from_pem(input).map_err(|err| {
        format!(
            "input is not a readable S/MIME, DER or PEM CMS structure: {}",
            stack_error(err)
        )
    })
}

fn signed_certificates(pkcs7: &Pkcs7) -> Result<Vec<String>, String> {
    let Some(signed) = pkcs7.signed() else {
        return Ok(Vec::new());
    };
    let Some(certs) = signed.certificates() else {
        return Ok(Vec::new());
    };
    certs.iter().map(cert_to_pem).collect()
}

fn certificate_metadata_of(cert: &X509Ref) -> Result<SmimeCertMetadata, String> {
    let fingerprint = cert
        .digest(MessageDigest::sha256())
        .map_err(stack_error)?
        .iter()
        .fold(String::new(), |mut acc, byte| {
            let _ = write!(acc, "{byte:02X}");
            acc
        });
    Ok(SmimeCertMetadata {
        fingerprint,
        subject_email: certificate_email(cert),
        subject_cn: common_name(cert.subject_name()),
        issuer_cn: common_name(cert.issuer_name()),
        not_before: Some(cert.not_before().to_string()),
        not_after: Some(cert.not_after().to_string()),
    })
}

/// `X509_get1_email` order: the subject DN first, then subjectAltName.
fn certificate_email(cert: &X509Ref) -> Option<String> {
    let from_subject = cert
        .subject_name()
        .entries_by_nid(Nid::PKCS9_EMAILADDRESS)
        .filter_map(|entry| entry.data().to_string().ok())
        .find(|value| value.contains('@'));
    from_subject.or_else(|| {
        cert.subject_alt_names()?
            .iter()
            .filter_map(|name| name.email())
            .find(|value| value.contains('@'))
            .map(str::to_string)
    })
}

/// Every mailbox a certificate speaks for: the subject DN's `emailAddress`
/// plus every `rfc822Name` of the `subjectAltName` extension.
///
/// `certificate_email` stops at the first hit, which cannot answer whether the
/// signer is allowed to speak for a `From:` header — modern certificates carry
/// their addresses in the SAN and often more than one.
pub fn certificate_rfc822_identities(pem_certificate: &str) -> Vec<String> {
    let Ok(cert) = X509::from_pem(pem_certificate.as_bytes()) else {
        return Vec::new();
    };
    let mut identities = cert
        .subject_name()
        .entries_by_nid(Nid::PKCS9_EMAILADDRESS)
        .filter_map(|entry| entry.data().to_string().ok())
        .collect::<Vec<_>>();
    if let Some(names) = cert.subject_alt_names() {
        identities.extend(
            names
                .iter()
                .filter_map(|name| name.email())
                .map(str::to_string),
        );
    }
    identities.retain(|value| value.contains('@'));
    identities
}

/// RFC 2253 prints RDNs in reverse, so its first `CN=` is the last one in the
/// DN. Keep that pick so metadata written by the CLI engine still matches.
fn common_name(name: &X509NameRef) -> Option<String> {
    name.entries_by_nid(Nid::COMMONNAME)
        .filter_map(|entry| entry.data().to_string().ok())
        .filter(|value| !value.is_empty())
        .last()
}

fn stack_error(err: ErrorStack) -> String {
    let rendered = err
        .errors()
        .iter()
        .filter_map(|error| error.reason())
        .collect::<Vec<_>>()
        .join(": ");
    if rendered.is_empty() {
        err.to_string()
    } else {
        rendered
    }
}

fn looks_like_passphrase_failure(err: &str) -> bool {
    let err = err.to_ascii_lowercase();
    err.contains("pass") || err.contains("decrypt") || err.contains("mac verify failure")
}

fn is_encrypted_pem(pem: &str) -> bool {
    pem.contains("ENCRYPTED")
}

/// A wrong passphrase usually fails the padding check ("bad decrypt"). About
/// one guess in 400 pads out by chance and OpenSSL then chokes on the garbage
/// as an ASN.1 decode error, which reads the same to the user.
fn is_wrong_passphrase(pem: &str, err: &str) -> bool {
    if looks_like_passphrase_failure(err) {
        return true;
    }
    let lowered = err.to_ascii_lowercase();
    is_encrypted_pem(pem) && (lowered.contains("asn1") || lowered.contains("decode error"))
}

// ---------------------------------------------------------------------------
// Binary fallback (SATCHEL_SMIME_ENGINE=cli)
// ---------------------------------------------------------------------------

/// Where the CLI engine hands openssl a passphrase.
///
/// `pass:<value>` puts the secret into the process arguments, which every
/// local process reads through `ps` or `/proc/<pid>/cmdline`. All six call
/// sites stage their payload in the temp directory and leave stdin free, so
/// the passphrase goes down the pipe instead — nothing on disk, nothing in the
/// argument vector.
const PASSPHRASE_SOURCE: &str = "fd:0";

/// openssl reads one line from the descriptor and strips the newline.
fn passphrase_stdin(passphrase: &str) -> Vec<u8> {
    let mut bytes = passphrase.as_bytes().to_vec();
    bytes.push(b'\n');
    bytes
}

fn optional_passphrase_stdin(passphrase: Option<&str>) -> Option<Vec<u8>> {
    passphrase
        .filter(|value| !value.trim().is_empty())
        .map(passphrase_stdin)
}

struct OpensslCliEngine;

static OPENSSL_BINARY_CACHE: OnceLock<Result<String, String>> = OnceLock::new();

impl OpensslCliEngine {
    fn resolve_openssl_binary(&self) -> Result<String, String> {
        let cached = OPENSSL_BINARY_CACHE.get_or_init(|| self.detect_openssl_binary());
        match cached {
            Ok(path) => Ok(path.clone()),
            Err(err) => Err(err.clone()),
        }
    }

    fn detect_openssl_binary(&self) -> Result<String, String> {
        if let Some(env_bin) = crate::utils::environment::debug_env("SATCHEL_OPENSSL_BIN") {
            let candidate = env_bin.trim();
            if !candidate.is_empty() {
                if self.openssl_binary_works(candidate) {
                    return Ok(candidate.to_string());
                }
                return Err(format!(
                    "SATCHEL_OPENSSL_BIN points to '{candidate}', but it is not executable."
                ));
            }
        }

        for candidate in [
            "openssl",
            "/opt/homebrew/bin/openssl",
            "/usr/local/bin/openssl",
            "/usr/bin/openssl",
        ] {
            if self.openssl_binary_works(candidate) {
                return Ok(candidate.to_string());
            }
        }

        Err("openssl not found. Install OpenSSL or set SATCHEL_OPENSSL_BIN=/path/to/openssl and restart Satchel.".to_string())
    }

    fn openssl_binary_works(&self, binary: &str) -> bool {
        Command::new(binary)
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn run(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let openssl = self.resolve_openssl_binary()?;
        let mut cmd = Command::new(&openssl);
        cmd.args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|err| format!("failed to execute {openssl}: {err}"))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // A child that exits before draining stdin closes the pipe; its
            // exit status and stderr say why, so EPIPE itself is not the error.
            if let Err(err) = pipe.write_all(input) {
                if err.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(err.to_string());
                }
            }
        }
        let output: Output = child.wait_with_output().map_err(|err| err.to_string())?;
        if !output.status.success() {
            return Err(openssl_error(output.stderr));
        }
        Ok(output.stdout)
    }

    fn write_file(path: &Path, data: impl AsRef<[u8]>) -> Result<(), String> {
        std::fs::write(path, data).map_err(|err| err.to_string())
    }
}

impl SmimeEngine for OpensslCliEngine {
    fn id(&self) -> &'static str {
        "openssl-cli"
    }

    fn ensure_available(&self) -> Result<(), String> {
        self.resolve_openssl_binary().map(|_| ())
    }

    fn sign(
        &self,
        pem_private_key: &str,
        pem_certificate: &str,
        passphrase: Option<&str>,
        payload: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-sign-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let key_path = tmp.path().join("key.pem");
        let cert_path = tmp.path().join("cert.pem");
        let input_path = tmp.path().join("payload.eml");
        Self::write_file(&key_path, pem_private_key)?;
        Self::write_file(&cert_path, pem_certificate)?;
        Self::write_file(&input_path, payload)?;
        let key = key_path.to_string_lossy().to_string();
        let cert = cert_path.to_string_lossy().to_string();
        let input = input_path.to_string_lossy().to_string();
        let pass_stdin = optional_passphrase_stdin(passphrase);
        let mut args = vec![
            "smime",
            "-sign",
            "-nodetach",
            "-signer",
            cert.as_str(),
            "-inkey",
            key.as_str(),
            "-in",
            input.as_str(),
            "-outform",
            "SMIME",
        ];
        if pass_stdin.is_some() {
            args.push("-passin");
            args.push(PASSPHRASE_SOURCE);
        }
        self.run(&args, pass_stdin.as_deref())
    }

    fn encrypt(&self, recipient_pems: &[String], payload: &[u8]) -> Result<Vec<u8>, String> {
        self.ensure_available()?;
        if recipient_pems.is_empty() {
            return Err("at least one S/MIME recipient certificate is required".to_string());
        }
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-encrypt-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let input_path = tmp.path().join("payload.eml");
        Self::write_file(&input_path, payload)?;
        let input = input_path.to_string_lossy().to_string();
        let mut cert_paths = Vec::new();
        for (idx, pem) in recipient_pems.iter().enumerate() {
            let path = tmp.path().join(format!("recipient-{idx}.pem"));
            Self::write_file(&path, pem)?;
            cert_paths.push(path);
        }
        let mut args = vec![
            "smime",
            "-encrypt",
            "-aes256",
            "-outform",
            "SMIME",
            "-in",
            input.as_str(),
        ];
        let cert_path_strings = cert_paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        for path in &cert_path_strings {
            args.push(path.as_str());
        }
        self.run(&args, None)
    }

    fn decrypt(
        &self,
        pem_private_key: &str,
        pem_certificate: &str,
        passphrase: Option<&str>,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-decrypt-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let key_path = tmp.path().join("key.pem");
        let cert_path = tmp.path().join("cert.pem");
        let input_path = tmp.path().join("encrypted.p7m");
        Self::write_file(&key_path, pem_private_key)?;
        Self::write_file(&cert_path, pem_certificate)?;
        Self::write_file(&input_path, ciphertext)?;
        let key = key_path.to_string_lossy().to_string();
        let cert = cert_path.to_string_lossy().to_string();
        let input = input_path.to_string_lossy().to_string();
        let pass_stdin = optional_passphrase_stdin(passphrase);
        // `openssl smime` speaks PKCS#7 and rejects RSAES-OAEP key transport,
        // which senders increasingly use over the legacy PKCS#1 v1.5 padding.
        // `openssl cms` reads the same enveloped data and handles both, so the
        // decrypt side reads CMS while sign and encrypt stay on `smime` for
        // maximum compatibility with older receivers.
        let mut args = vec![
            "cms",
            "-decrypt",
            "-recip",
            cert.as_str(),
            "-inkey",
            key.as_str(),
            "-inform",
            "SMIME",
            "-in",
            input.as_str(),
        ];
        if pass_stdin.is_some() {
            args.push("-passin");
            args.push(PASSPHRASE_SOURCE);
        }
        self.run(&args, pass_stdin.as_deref())
    }

    fn verify(&self, signed_payload: &[u8]) -> Result<SmimeVerifyResult, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-verify-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let certs_path = tmp.path().join("signers.pem");
        let input_path = tmp.path().join("signed.eml");
        Self::write_file(&input_path, signed_payload)?;
        let certs_path_string = certs_path.to_string_lossy().to_string();
        let input = input_path.to_string_lossy().to_string();
        let args = [
            "smime",
            "-verify",
            "-noverify",
            "-inform",
            "SMIME",
            "-in",
            input.as_str(),
            "-signer",
            certs_path_string.as_str(),
        ];
        let outcome = self.run(&args, None);
        let signer_cert_pem = std::fs::read_to_string(&certs_path)
            .ok()
            .filter(|s| !s.trim().is_empty());
        let signer = signer_cert_pem
            .as_deref()
            .and_then(|pem| self.certificate_metadata(pem).ok());
        let mut result = SmimeVerifyResult {
            valid: outcome.is_ok(),
            chain_verified: false,
            signer_cert_pem,
            signer,
            error: outcome.err(),
        };
        apply_platform_trust(signed_payload, &mut result);
        Ok(result)
    }

    fn certificate_metadata(&self, pem_certificate: &str) -> Result<SmimeCertMetadata, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-cert-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let cert_path = tmp.path().join("cert.pem");
        Self::write_file(&cert_path, pem_certificate)?;
        let cert_path_string = cert_path.to_string_lossy().to_string();
        let fingerprint = String::from_utf8(self.run(
            &[
                "x509",
                "-in",
                cert_path_string.as_str(),
                "-noout",
                "-fingerprint",
                "-sha256",
            ],
            None,
        )?)
        .map_err(|err| err.to_string())?;
        let subject = String::from_utf8(self.run(
            &[
                "x509",
                "-in",
                cert_path_string.as_str(),
                "-noout",
                "-subject",
                "-nameopt",
                "RFC2253",
            ],
            None,
        )?)
        .map_err(|err| err.to_string())?;
        let issuer = String::from_utf8(self.run(
            &[
                "x509",
                "-in",
                cert_path_string.as_str(),
                "-noout",
                "-issuer",
                "-nameopt",
                "RFC2253",
            ],
            None,
        )?)
        .map_err(|err| err.to_string())?;
        let dates = String::from_utf8(self.run(
            &["x509", "-in", cert_path_string.as_str(), "-noout", "-dates"],
            None,
        )?)
        .map_err(|err| err.to_string())?;
        let email = String::from_utf8(
            self.run(
                &["x509", "-in", cert_path_string.as_str(), "-noout", "-email"],
                None,
            )
            .unwrap_or_default(),
        )
        .unwrap_or_default();

        Ok(SmimeCertMetadata {
            fingerprint: format_fingerprint(
                fingerprint.split('=').nth(1).unwrap_or(fingerprint.trim()),
            ),
            subject_email: email
                .lines()
                .find(|line| line.contains('@'))
                .map(|line| line.trim().to_string())
                .or_else(|| extract_rfc2253_value(&subject, "emailAddress")),
            subject_cn: extract_rfc2253_value(&subject, "CN"),
            issuer_cn: extract_rfc2253_value(&issuer, "CN"),
            not_before: dates
                .lines()
                .find_map(|line| line.strip_prefix("notBefore=").map(str::to_string)),
            not_after: dates
                .lines()
                .find_map(|line| line.strip_prefix("notAfter=").map(str::to_string)),
        })
    }

    fn extract_pkcs12(&self, p12_der: &[u8], passphrase: &str) -> Result<(String, String), String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-p12-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let p12_path = tmp.path().join("bundle.p12");
        let cert_path = tmp.path().join("cert.pem");
        let key_path = tmp.path().join("key.pem");
        Self::write_file(&p12_path, p12_der)?;
        let p12 = p12_path.to_string_lossy().to_string();
        let cert = cert_path.to_string_lossy().to_string();
        let key = key_path.to_string_lossy().to_string();
        let pass_stdin = passphrase_stdin(passphrase);
        self.run(
            &[
                "pkcs12",
                "-in",
                p12.as_str(),
                "-clcerts",
                "-nokeys",
                "-out",
                cert.as_str(),
                "-passin",
                PASSPHRASE_SOURCE,
            ],
            Some(&pass_stdin),
        )?;
        self.run(
            &[
                "pkcs12",
                "-in",
                p12.as_str(),
                "-nocerts",
                "-nodes",
                "-out",
                key.as_str(),
                "-passin",
                PASSPHRASE_SOURCE,
            ],
            Some(&pass_stdin),
        )?;
        Ok((
            std::fs::read_to_string(cert_path).map_err(|err| err.to_string())?,
            std::fs::read_to_string(key_path).map_err(|err| err.to_string())?,
        ))
    }

    fn generate_keypair(
        &self,
        request: &GenerateSmimeRequest<'_>,
    ) -> Result<GeneratedSmimeKeyPair, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-gen-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let key_path = tmp.path().join("key.pem");
        let cert_path = tmp.path().join("cert.pem");
        let subj = format!(
            "/CN={}/emailAddress={}",
            escape_subject_value(request.name),
            escape_subject_value(request.email)
        );
        let key = key_path.to_string_lossy().to_string();
        let cert = cert_path.to_string_lossy().to_string();
        let pass_stdin = optional_passphrase_stdin(request.passphrase);
        let mut args = vec![
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-days",
            "730",
            "-subj",
            subj.as_str(),
            "-keyout",
            key.as_str(),
            "-out",
            cert.as_str(),
        ];
        if pass_stdin.is_some() {
            args.push("-passout");
            args.push(PASSPHRASE_SOURCE);
        } else {
            args.push("-nodes");
        }
        self.run(&args, pass_stdin.as_deref())?;
        let pem_certificate = std::fs::read_to_string(&cert_path).map_err(|err| err.to_string())?;
        let pem_private_key = std::fs::read_to_string(&key_path).map_err(|err| err.to_string())?;
        let metadata = self.certificate_metadata(&pem_certificate)?;
        Ok(GeneratedSmimeKeyPair {
            pem_certificate,
            pem_private_key,
            metadata,
        })
    }

    fn verify_private_key_passphrase(
        &self,
        pem_private_key: &str,
        passphrase: Option<&str>,
    ) -> Result<bool, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-keycheck-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let key_path = tmp.path().join("key.pem");
        Self::write_file(&key_path, pem_private_key)?;
        let key = key_path.to_string_lossy().to_string();
        let mut args = vec!["pkey", "-in", key.as_str(), "-noout"];
        let pass_stdin = optional_passphrase_stdin(passphrase);
        if pass_stdin.is_some() {
            args.push("-passin");
            args.push(PASSPHRASE_SOURCE);
        }
        match self.run(&args, pass_stdin.as_deref()) {
            Ok(_) => Ok(true),
            Err(err) if is_wrong_passphrase(pem_private_key, &err) => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn extract_certificates(&self, signed_payload: &[u8]) -> Result<Vec<String>, String> {
        self.ensure_available()?;
        let tmp = tempfile::Builder::new()
            .prefix("satchel-smime-certs-")
            .tempdir()
            .map_err(|err| err.to_string())?;
        let input_path = tmp.path().join("signed.eml");
        let pk7_path = tmp.path().join("signed.p7b");
        Self::write_file(&input_path, signed_payload)?;
        let input = input_path.to_string_lossy().to_string();
        let pk7 = pk7_path.to_string_lossy().to_string();
        self.run(
            &[
                "smime",
                "-pk7out",
                "-inform",
                "SMIME",
                "-in",
                input.as_str(),
                "-out",
                pk7.as_str(),
            ],
            None,
        )?;
        let certs = self.run(&["pkcs7", "-print_certs", "-in", pk7.as_str()], None)?;
        Ok(parse_pem_certificates(
            &String::from_utf8(certs).map_err(|err| err.to_string())?,
        ))
    }
}

/// Native trust is authoritative on platforms that expose an S/MIME policy.
/// A failed native verdict must never fall back to a different set of roots.
fn apply_platform_trust(signed_payload: &[u8], result: &mut SmimeVerifyResult) {
    if !result.valid {
        return;
    }
    let trust =
        read_pkcs7(signed_payload).and_then(|(pkcs7, _)| super::smime_trust::verify(&pkcs7));
    result.chain_verified = trust.is_ok();
    result.error = trust.err();
}

fn openssl_error(stderr: Vec<u8>) -> String {
    let err = String::from_utf8_lossy(&stderr).trim().to_string();
    if err.is_empty() {
        "openssl command failed".to_string()
    } else {
        err
    }
}

fn format_fingerprint(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_uppercase()
}

fn extract_rfc2253_value(input: &str, key: &str) -> Option<String> {
    let raw = input
        .trim()
        .trim_start_matches("subject=")
        .trim_start_matches("issuer=");
    raw.split(',').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        if name.trim().eq_ignore_ascii_case(key) {
            let value = value.trim().replace("\\,", ",");
            (!value.is_empty()).then_some(value)
        } else {
            None
        }
    })
}

fn escape_subject_value(value: &str) -> String {
    value.replace('/', "\\/")
}

pub fn parse_pem_certificates(input: &str) -> Vec<String> {
    let mut certs = Vec::new();
    let mut current = String::new();
    let mut collecting = false;
    for line in input.lines() {
        if line.contains("-----BEGIN CERTIFICATE-----") {
            collecting = true;
            current.clear();
        }
        if collecting {
            current.push_str(line);
            current.push('\n');
        }
        if line.contains("-----END CERTIFICATE-----") && collecting {
            certs.push(current.trim().to_string());
            collecting = false;
            current.clear();
        }
    }
    certs
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::x509::extension::{ExtendedKeyUsage, KeyUsage, SubjectAlternativeName};

    struct Identity {
        cert: X509,
        cert_pem: String,
        key_pem: String,
    }

    /// Throwaway CA. No fixture ever holds a private key on disk: every test
    /// mints its own chain.
    fn issue_ca() -> (X509, PKey<Private>) {
        issue_ca_signed_by(None)
    }

    fn issue_ca_signed_by(parent: Option<&(X509, PKey<Private>)>) -> (X509, PKey<Private>) {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_nid(
            Nid::COMMONNAME,
            if parent.is_some() {
                "Satchel Intermediate CA"
            } else {
                "Satchel Test CA"
            },
        )
        .unwrap();
        let name = name.build();

        let mut builder = X509Builder::new().unwrap();
        builder.set_version(2).unwrap();
        builder
            .set_serial_number(&random_serial().unwrap())
            .unwrap();
        builder.set_subject_name(&name).unwrap();
        builder
            .set_issuer_name(parent.map(|ca| ca.0.subject_name()).unwrap_or(&name))
            .unwrap();
        builder.set_pubkey(&key).unwrap();
        builder
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        builder
            .set_not_after(&Asn1Time::days_from_now(2).unwrap())
            .unwrap();
        builder
            .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .key_cert_sign()
                    .crl_sign()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let skid = SubjectKeyIdentifier::new()
            .build(&builder.x509v3_context(None, None))
            .unwrap();
        builder.append_extension(skid).unwrap();
        builder
            .sign(
                parent.map(|ca| &ca.1).unwrap_or(&key),
                MessageDigest::sha256(),
            )
            .unwrap();
        (builder.build(), key)
    }

    fn issue_leaf(ca: &(X509, PKey<Private>), cn: &str, email: &str) -> Identity {
        issue_leaf_with_alt_emails(ca, cn, email, &[])
    }

    fn issue_leaf_with_alt_emails(
        ca: &(X509, PKey<Private>),
        cn: &str,
        email: &str,
        alt_emails: &[&str],
    ) -> Identity {
        issue_leaf_with_profile(ca, cn, email, alt_emails, true, false, None)
    }

    fn issue_leaf_with_profile(
        ca: &(X509, PKey<Private>),
        cn: &str,
        email: &str,
        alt_emails: &[&str],
        email_protection: bool,
        expired: bool,
        issuer_url: Option<&str>,
    ) -> Identity {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_nid(Nid::COMMONNAME, cn).unwrap();
        name.append_entry_by_nid(Nid::PKCS9_EMAILADDRESS, email)
            .unwrap();
        let name = name.build();

        let mut builder = X509Builder::new().unwrap();
        builder.set_version(2).unwrap();
        builder
            .set_serial_number(&random_serial().unwrap())
            .unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(ca.0.subject_name()).unwrap();
        builder.set_pubkey(&key).unwrap();
        builder
            .set_not_before(&Asn1Time::from_unix(chrono::Utc::now().timestamp() - 172_800).unwrap())
            .unwrap();
        builder
            .set_not_after(
                &Asn1Time::from_unix(
                    chrono::Utc::now().timestamp() + if expired { -86_400 } else { 86_400 },
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .append_extension(BasicConstraints::new().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .digital_signature()
                    .key_encipherment()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let mut eku = ExtendedKeyUsage::new();
        if email_protection {
            eku.email_protection();
        } else {
            eku.server_auth();
        }
        builder.append_extension(eku.build().unwrap()).unwrap();
        if let Some(url) = issuer_url {
            // AuthorityInfoAccess ::= SEQUENCE OF AccessDescription. Use a short
            // test URI so every DER length fits in one octet.
            assert!(url.len() < 100);
            let mut description = vec![
                0x06,
                0x08,
                0x2b,
                0x06,
                0x01,
                0x05,
                0x05,
                0x07,
                0x30,
                0x02,
                0x86,
                url.len() as u8,
            ];
            description.extend_from_slice(url.as_bytes());
            let mut der = vec![
                0x30,
                (description.len() + 2) as u8,
                0x30,
                description.len() as u8,
            ];
            der.extend(description);
            let oid = openssl::asn1::Asn1Object::from_str("1.3.6.1.5.5.7.1.1").unwrap();
            let data = openssl::asn1::Asn1OctetString::new_from_bytes(&der).unwrap();
            builder
                .append_extension(
                    openssl::x509::X509Extension::new_from_der(&oid, false, &data).unwrap(),
                )
                .unwrap();
        }
        let context = builder.x509v3_context(Some(&ca.0), None);
        let mut san = SubjectAlternativeName::new();
        san.email(email);
        for alt in alt_emails {
            san.email(alt);
        }
        builder
            .append_extension(san.build(&context).unwrap())
            .unwrap();
        let akid = AuthorityKeyIdentifier::new()
            .keyid(false)
            .build(&builder.x509v3_context(Some(&ca.0), None))
            .unwrap();
        builder.append_extension(akid).unwrap();
        builder.sign(&ca.1, MessageDigest::sha256()).unwrap();
        let cert = builder.build();
        Identity {
            cert_pem: cert_to_pem(&cert).unwrap(),
            key_pem: private_key_to_pem(&key, None).unwrap(),
            cert,
        }
    }

    fn store_trusting(ca: &X509) -> X509Store {
        let mut builder = X509StoreBuilder::new().unwrap();
        builder.add_cert(ca.clone()).unwrap();
        builder
            .set_purpose(openssl::x509::X509PurposeId::SMIME_SIGN)
            .unwrap();
        builder.build()
    }

    /// Self-signed and carrying the S/MIME purpose — what an attacker mints to
    /// dress a forged sender address in a seal.
    fn issue_self_signed(cn: &str, email: &str) -> Identity {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_nid(Nid::COMMONNAME, cn).unwrap();
        name.append_entry_by_nid(Nid::PKCS9_EMAILADDRESS, email)
            .unwrap();
        let name = name.build();

        let mut builder = X509Builder::new().unwrap();
        builder.set_version(2).unwrap();
        builder
            .set_serial_number(&random_serial().unwrap())
            .unwrap();
        builder.set_subject_name(&name).unwrap();
        builder.set_issuer_name(&name).unwrap();
        builder.set_pubkey(&key).unwrap();
        builder
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        builder
            .set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        builder
            .append_extension(BasicConstraints::new().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .digital_signature()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        builder
            .append_extension(ExtendedKeyUsage::new().email_protection().build().unwrap())
            .unwrap();
        builder.sign(&key, MessageDigest::sha256()).unwrap();
        let cert = builder.build();
        Identity {
            cert_pem: cert_to_pem(&cert).unwrap(),
            key_pem: private_key_to_pem(&key, None).unwrap(),
            cert,
        }
    }

    fn openssl_binary() -> Option<String> {
        OpensslCliEngine.resolve_openssl_binary().ok()
    }

    fn headers_of(smime: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(smime)
            .lines()
            .take_while(|line| !line.trim().is_empty())
            .map(|line| line.to_string())
            .collect()
    }

    #[test]
    fn sign_produces_opaque_smime_headers() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let signed = OpensslLibEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, b"hello signed world")
            .unwrap();
        assert_eq!(
            headers_of(&signed),
            vec![
                "MIME-Version: 1.0".to_string(),
                "Content-Disposition: attachment; filename=\"smime.p7m\"".to_string(),
                "Content-Type: application/x-pkcs7-mime; smime-type=signed-data; name=\"smime.p7m\""
                    .to_string(),
                "Content-Transfer-Encoding: base64".to_string(),
            ]
        );
    }

    #[test]
    fn encrypt_produces_enveloped_smime_headers() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Recipient", "recipient@satchel.local");
        let encrypted = OpensslLibEngine
            .encrypt(std::slice::from_ref(&leaf.cert_pem), b"hello secret world")
            .unwrap();
        assert_eq!(
            headers_of(&encrypted),
            vec![
                "MIME-Version: 1.0".to_string(),
                "Content-Disposition: attachment; filename=\"smime.p7m\"".to_string(),
                "Content-Type: application/x-pkcs7-mime; smime-type=enveloped-data; name=\"smime.p7m\""
                    .to_string(),
                "Content-Transfer-Encoding: base64".to_string(),
            ]
        );
    }

    /// Synthetic certs only; exercises the builder and crypto functions used by
    /// the outbox, then the exact MIME helper used by the message command.
    #[test]
    fn encrypted_mime_roundtrips_text_html_attachments_and_inner_signatures() {
        use crate::{
            email::{build::build_rfc822, smime::parse_decrypted_body},
            providers::traits::{OutgoingAttachment, OutgoingMessage},
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Synthetic Signer", "sender@example.test");
        for html in [false, true] {
            for signed in [false, true] {
                let outgoing = OutgoingMessage {
                    from_address: Some("sender@example.test".into()),
                    to: vec!["recipient@example.test".into()],
                    subject: "Synthetic encrypted MIME".into(),
                    body: "Readable secret ü".into(),
                    body_html: html
                        .then(|| "<p>Readable <b>secret</b> ü</p><script>bad()</script>".into()),
                    attachments: vec![OutgoingAttachment {
                        filename: "bytes.bin".into(),
                        mime_type: "application/octet-stream".into(),
                        content_base64: "AP+AQQ==".into(),
                        content_id: None,
                        inline: false,
                    }],
                    ..Default::default()
                };
                let original = build_rfc822(&outgoing, None, None).unwrap();
                let encrypted = crate::email::smime::protect_message(
                    original.as_bytes(),
                    signed.then_some((leaf.key_pem.as_str(), leaf.cert_pem.as_str(), None)),
                    Some(std::slice::from_ref(&leaf.cert_pem)),
                )
                .unwrap();
                // The read side receives the decoded CMS bytes from IMAP.
                let envelope = crate::providers::imap::parse_raw_message_body(&encrypted);
                let bytes = decrypt(
                    &leaf.key_pem,
                    &leaf.cert_pem,
                    None,
                    envelope.smime_encrypted.as_deref().unwrap(),
                )
                .unwrap();
                if signed {
                    assert_eq!(signed_content(&bytes).unwrap(), original.as_bytes());
                } else {
                    assert_eq!(
                        bytes,
                        original.as_bytes(),
                        "decryption must retain exact MIME bytes"
                    );
                }
                let parsed = parse_decrypted_body(&bytes).unwrap();
                assert!(parsed
                    .body
                    .text
                    .as_deref()
                    .unwrap()
                    .contains("Readable secret ü"));
                assert_eq!(parsed.body.html.is_some(), html);
                if let Some(body) = parsed.body.html {
                    assert!(!body.contains("<script"));
                }
                assert_eq!(parsed.body.attachments.len(), 1);
                assert_eq!(
                    parsed.body.attachments[0].content_base64.as_deref(),
                    Some("AP+AQQ==")
                );
                assert_eq!(parsed.signatures.len(), usize::from(signed));
                if signed {
                    let (payload, root) = &parsed.signatures[0];
                    assert!(root);
                    let trusted = OpensslLibEngine.verify_against(
                        payload,
                        &store_trusting(&ca.0),
                        Pkcs7Flags::empty(),
                    );
                    assert!(trusted.valid && trusted.chain_verified);
                    assert!(verify(payload).unwrap().valid);
                    assert!(
                        !verify(payload).unwrap().chain_verified,
                        "fixture CA is not a system anchor"
                    );
                }
            }
        }
        // A charset byte invalid as UTF-8 must survive until MIME decoding.
        let payload = b"Content-Type: text/plain; charset=iso-8859-1\r\n\r\nGr\xfc\xdfe";
        let encrypted = encrypt(std::slice::from_ref(&leaf.cert_pem), payload).unwrap();
        let bytes = decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted).unwrap();
        assert_eq!(bytes, payload);
        assert_eq!(
            parse_decrypted_body(&bytes).unwrap().body.text.as_deref(),
            Some("Grüße")
        );
    }

    #[test]
    fn encrypted_detached_signature_and_html_only_body_are_decoded() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Synthetic Signer", "sender@example.test");
        let content = b"Content-Type: text/html; charset=utf-8\r\n\r\n<p>HTML <b>only</b></p><script>unsafe()</script>";
        let key = load_private_key(&leaf.key_pem, None).unwrap();
        let flags = Pkcs7Flags::DETACHED | Pkcs7Flags::BINARY;
        let signature =
            Pkcs7::sign(&leaf.cert, &key, &Stack::new().unwrap(), content, flags).unwrap();
        let signed = signature.to_smime(content, flags).unwrap();
        let encrypted = encrypt(std::slice::from_ref(&leaf.cert_pem), &signed).unwrap();
        let bytes = decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted).unwrap();
        let parsed = crate::email::smime::parse_decrypted_body(&bytes).unwrap();
        assert_eq!(parsed.body.text.as_deref(), Some("HTML only"));
        assert!(!parsed.body.html.unwrap().contains("<script"));
        assert_eq!(parsed.signatures.len(), 1);
        assert!(parsed.signatures[0].1);
        let payload = &parsed.signatures[0].0;
        assert!(
            OpensslLibEngine
                .verify_against(payload, &store_trusting(&ca.0), Pkcs7Flags::empty())
                .valid
        );
    }

    #[test]
    fn decrypted_mixed_content_keeps_unsigned_cover_and_partial_signature_scope() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Synthetic Signer", "sender@example.test");
        let signed = sign(
            &leaf.key_pem,
            &leaf.cert_pem,
            None,
            b"Content-Type: text/plain\r\n\r\nSigned text",
        )
        .unwrap();
        // Only inline signed content participates in body-signature scope;
        // an attached CMS object is intentionally excluded by the MIME parser.
        let signed = std::str::from_utf8(&signed).unwrap().replace(
            "Content-Disposition: attachment",
            "Content-Disposition: inline",
        );
        let mut mixed = b"Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/html\r\n\r\n<p>Unsigned cover</p>\r\n--m\r\n".to_vec();
        mixed.extend_from_slice(signed.as_bytes());
        mixed.extend_from_slice(b"\r\n--m--\r\n");
        let encrypted = encrypt(std::slice::from_ref(&leaf.cert_pem), &mixed).unwrap();
        let bytes = decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted).unwrap();
        let parsed = crate::email::smime::parse_decrypted_body(&bytes).unwrap();
        assert!(parsed.body.html.unwrap().contains("Unsigned cover"));
        assert_eq!(parsed.signatures.len(), 1);
        assert!(
            !parsed.signatures[0].1,
            "a nested signature does not cover the cover note"
        );
    }

    #[test]
    fn encrypted_broken_detached_signature_keeps_readable_body() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Synthetic Signer", "sender@example.test");
        let payload = b"Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; boundary=s\r\n\r\n--s\r\nContent-Type: text/plain\r\n\r\nReadable message\r\n--s\r\nContent-Type: application/pkcs7-signature\r\nContent-Transfer-Encoding: base64\r\n\r\nAQID\r\n--s--\r\n";
        let encrypted = encrypt(std::slice::from_ref(&leaf.cert_pem), payload).unwrap();
        let bytes = decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted).unwrap();
        let parsed = crate::email::smime::parse_decrypted_body(&bytes).unwrap();
        assert_eq!(parsed.body.text.as_deref(), Some("Readable message"));
        assert_eq!(parsed.signatures.len(), 1);
        assert!(!verify(&parsed.signatures[0].0).unwrap().valid);
    }

    #[test]
    fn encrypted_damaged_inner_signature_is_readable_but_invalid() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Synthetic Signer", "sender@example.test");
        let original = b"Content-Type: text/plain\r\n\r\nOriginal secret";
        let signed = sign(&leaf.key_pem, &leaf.cert_pem, None, original).unwrap();
        let (pkcs7, _) = read_pkcs7(&signed).unwrap();
        let mut der = pkcs7.to_der().unwrap();
        let offset = der
            .windows(original.len())
            .position(|part| part == original)
            .unwrap();
        der[offset + original.len() - 1] = b'X';
        let damaged = Pkcs7::from_der(&der)
            .unwrap()
            .to_smime(&[], Pkcs7Flags::empty())
            .unwrap();
        let encrypted = encrypt(std::slice::from_ref(&leaf.cert_pem), &damaged).unwrap();
        let bytes = decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted).unwrap();
        let parsed = crate::email::smime::parse_decrypted_body(&bytes).unwrap();
        assert_eq!(parsed.body.text.as_deref(), Some("Original secreX"));
        let payload = &parsed.signatures[0].0;
        assert!(!verify(payload).unwrap().valid);
        assert!(
            !OpensslLibEngine
                .verify_against(payload, &store_trusting(&ca.0), Pkcs7Flags::empty())
                .valid
        );
    }

    #[test]
    fn sign_verify_roundtrip() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let signed = OpensslLibEngine
            .sign(
                &leaf.key_pem,
                &leaf.cert_pem,
                None,
                b"Subject: test\r\n\r\nhello",
            )
            .unwrap();

        let result =
            OpensslLibEngine.verify_against(&signed, &store_trusting(&ca.0), Pkcs7Flags::empty());
        assert!(result.valid, "verify failed: {:?}", result.error);
        let signer = result.signer.expect("signer metadata");
        assert_eq!(signer.subject_cn.as_deref(), Some("Satchel Signer"));
        assert_eq!(
            signer.subject_email.as_deref(),
            Some("signer@satchel.local")
        );
        assert_eq!(signer.issuer_cn.as_deref(), Some("Satchel Test CA"));

        let certs = OpensslLibEngine.extract_certificates(&signed).unwrap();
        assert_eq!(certs.len(), 1);
        assert_eq!(certs[0].trim(), leaf.cert_pem.trim());
    }

    #[test]
    fn captured_valid_smime_entities_are_partial_or_opaque_when_embedded() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Bob", "bob@bank.example");
        let signed = OpensslLibEngine
            .sign(
                &leaf.key_pem,
                &leaf.cert_pem,
                None,
                b"Content-Type: text/plain\r\n\r\nOriginal signed message",
            )
            .unwrap();
        let encrypted = OpensslLibEngine
            .encrypt(
                std::slice::from_ref(&leaf.cert_pem),
                b"Secret captured message",
            )
            .unwrap();
        for (entity, is_signed) in [(&signed, true), (&encrypted, false)] {
            // Change only the outer MIME disposition, leaving signed bytes intact.
            let entity = std::str::from_utf8(entity).unwrap().replace(
                "Content-Disposition: attachment",
                "Content-Disposition: inline",
            );
            let root = crate::providers::imap::parse_raw_message_body(entity.as_bytes());
            assert!(root.trust.root_crypto);
            assert_eq!(root.trust.root_encryption, !is_signed);
            let mut attack = b"From: Bob <bob@bank.example>\r\nContent-Type: multipart/mixed; boundary=attack\r\n\r\n--attack\r\nContent-Type: text/html\r\n\r\n<p>Please wire EUR 10000</p>\r\n--attack\r\n".to_vec();
            attack.extend_from_slice(entity.as_bytes());
            attack.extend_from_slice(b"\r\n--attack--\r\n");
            let parsed = crate::providers::imap::parse_raw_message_body(&attack);
            assert!(!parsed.trust.root_crypto);
            assert!(!parsed.trust.root_encryption);
            assert!(parsed.smime_encrypted.is_none());
            assert!(parsed.html.as_deref().unwrap().contains("wire"));
            if is_signed {
                let result = OpensslLibEngine.verify_against(
                    parsed.smime_signed.as_deref().unwrap(),
                    &store_trusting(&ca.0),
                    Pkcs7Flags::empty(),
                );
                assert!(result.valid);
                assert!(result.chain_verified);
            }
        }
    }

    #[test]
    fn verify_rejects_tampered_content() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let payload = b"tamper-me-please";
        let signed = OpensslLibEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, payload)
            .unwrap();

        // Swap one byte of the encapsulated content. The length stays the same,
        // so the structure still parses and the messageDigest attribute is what
        // fails.
        let (pkcs7, _) = read_pkcs7(&signed).unwrap();
        let mut der = pkcs7.to_der().unwrap();
        let offset = der
            .windows(payload.len())
            .position(|window| window == payload)
            .expect("payload is embedded verbatim");
        der[offset] = b'T';
        let tampered = Pkcs7::from_der(&der)
            .unwrap()
            .to_smime(&[], Pkcs7Flags::empty())
            .unwrap();

        let result =
            OpensslLibEngine.verify_against(&tampered, &store_trusting(&ca.0), Pkcs7Flags::empty());
        assert!(!result.valid);
        assert!(result.error.is_some());
        // The signer is still reported so the UI can name the sender.
        assert!(result.signer_cert_pem.is_some());
        let unrelated_ca = issue_ca();
        let store = store_trusting(&unrelated_ca.0);
        let result = OpensslLibEngine
            .verify_with_anchors(&tampered, Some(&store))
            .unwrap();
        assert!(
            !result.valid,
            "an untrusted certificate must not mask tampering"
        );
        assert!(!result.chain_verified);
        let result = OpensslLibEngine.verify(&tampered).unwrap();
        assert!(
            !result.valid && !result.chain_verified,
            "the production verifier must reject tampering before checking trust"
        );
    }

    #[test]
    fn certificate_identities_cover_every_rfc822_name() {
        let ca = issue_ca();
        let leaf =
            issue_leaf_with_alt_emails(&ca, "Board", "board@firma.test", &["chair@firma.test"]);

        let identities = certificate_rfc822_identities(&leaf.cert_pem);

        assert!(identities.iter().any(|value| value == "board@firma.test"));
        assert!(identities.iter().any(|value| value == "chair@firma.test"));
        assert!(certificate_rfc822_identities("not a certificate").is_empty());
    }

    #[test]
    fn certificate_subjects_do_not_truncate_at_embedded_nul() {
        let cn = "Board\0Untrusted suffix";
        let email = "board@example.com\0.attacker.test";
        let identity = issue_self_signed(cn, email);

        assert_eq!(certificate_email(&identity.cert), Some(email.to_string()));
        assert_eq!(
            certificate_rfc822_identities(&identity.cert_pem),
            vec![email.to_string()]
        );
        assert_eq!(
            common_name(identity.cert.subject_name()),
            Some(cn.to_string())
        );
    }

    #[test]
    fn verify_without_trust_store_never_reports_a_valid_chain() {
        let attacker = issue_self_signed("Board", "board@satchel.local");
        let signed = OpensslLibEngine
            .sign(
                &attacker.key_pem,
                &attacker.cert_pem,
                None,
                b"Subject: payment\r\n\r\nwire the money",
            )
            .unwrap();

        let result = OpensslLibEngine.verify_with_anchors(&signed, None).unwrap();

        assert!(
            !result.chain_verified,
            "no trust store means no chain, whatever the signature maths says"
        );
        assert!(result.signer.is_some(), "the signer is still reported");
    }

    #[test]
    fn verify_never_calls_a_self_signed_certificate_chained() {
        let attacker = issue_self_signed("Board", "board@satchel.local");
        let signed = OpensslLibEngine
            .sign(
                &attacker.key_pem,
                &attacker.cert_pem,
                None,
                b"wire the money",
            )
            .unwrap();

        // The real anchor set, whatever this platform resolved.
        let result = OpensslLibEngine.verify(&signed).unwrap();

        assert!(!result.chain_verified);
        assert!(
            result.valid,
            "an intact signature must remain valid even when its certificate is untrusted: {:?}",
            result.error
        );
    }

    #[test]
    fn cli_distinguishes_an_intact_untrusted_signature_from_corruption() {
        if openssl_binary().is_none() {
            return;
        }
        let signer = issue_self_signed("Signer", "signer@satchel.local");
        let signed = OpensslLibEngine
            .sign(&signer.key_pem, &signer.cert_pem, None, b"intact")
            .unwrap();
        let result = OpensslCliEngine.verify(&signed).unwrap();
        assert!(result.valid, "{:?}", result.error);
        assert!(!result.chain_verified);
        assert!(result.error.is_some());
        assert!(!OpensslCliEngine.verify(b"corrupted").unwrap().valid);
    }

    #[test]
    fn verify_with_platform_anchors_accepts_a_chained_certificate() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let signed = OpensslLibEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, b"hello signed world")
            .unwrap();

        let store = store_trusting(&ca.0);
        let result = OpensslLibEngine
            .verify_with_anchors(&signed, Some(&store))
            .unwrap();

        assert!(result.valid, "verify failed: {:?}", result.error);
        assert!(result.chain_verified);
    }

    #[test]
    fn verify_reports_unreadable_input_without_erroring() {
        let result = OpensslLibEngine
            .verify(b"not an S/MIME message at all")
            .unwrap();
        assert!(!result.valid);
        assert!(result.error.is_some());
    }

    #[test]
    fn portable_trust_is_offline_even_with_attacker_aia() {
        use super::super::smime_trust::portable::verify_with_store;
        let root = issue_ca();
        let issuer = issue_ca_signed_by(Some(&root));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/victim-id", listener.local_addr().unwrap());
        let leaf = issue_leaf_with_profile(
            &issuer,
            "Signer",
            "signer@example.test",
            &[],
            true,
            false,
            Some(&url),
        );
        let der = leaf.cert.to_der().unwrap();
        let store = store_trusting(&root.0);
        assert!(verify_with_store(&der, &[], &store).is_err());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        verify_with_store(&der, &[issuer.0.to_der().unwrap()], &store).unwrap();
        assert!(verify_with_store(
            &der,
            &[issuer.0.to_der().unwrap()],
            &store_trusting(&issue_ca().0)
        )
        .is_err());
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let anchors = vec![root.0.to_der().unwrap()];
            assert!(
                super::super::smime_trust::verify_apple_with_anchors(&der, &[], &anchors).is_err()
            );
            super::super::smime_trust::verify_apple_with_anchors(
                &der,
                &[issuer.0.to_der().unwrap()],
                &anchors,
            )
            .unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn trust_rejects_expired_and_wrong_purpose_certificates() {
        let ca = issue_ca();
        for (email_protection, expired, expected) in [
            (true, false, true),
            (true, true, false),
            (false, false, false),
        ] {
            let leaf = issue_leaf_with_profile(
                &ca,
                "Signer",
                "signer@example.test",
                &[],
                email_protection,
                expired,
                None,
            );
            let der = leaf.cert.to_der().unwrap();
            let result = super::super::smime_trust::portable::verify_with_store(
                &der,
                &[],
                &store_trusting(&ca.0),
            );
            assert_eq!(result.is_ok(), expected, "portable: {result:?}");
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                let result = super::super::smime_trust::verify_apple_with_anchors(
                    &der,
                    &[],
                    &[ca.0.to_der().unwrap()],
                );
                assert_eq!(result.is_ok(), expected, "Apple: {result:?}");
            }
        }
    }

    #[test]
    #[ignore = "requires SATCHEL_SMIME_FIXTURE pointing to a locally held, trusted signed mail"]
    fn verifies_a_local_mail_against_platform_and_portable_trust() {
        let payload = std::fs::read(std::env::var("SATCHEL_SMIME_FIXTURE").unwrap()).unwrap();
        for engine in [&OpensslLibEngine as &dyn SmimeEngine, &OpensslCliEngine] {
            let result = engine.verify(&payload).unwrap();
            assert!(
                result.valid && result.chain_verified,
                "{}: {result:?}",
                engine.id()
            );
        }
        let (pkcs7, _) = read_pkcs7(&payload).unwrap();
        let empty = Stack::new().unwrap();
        let signers = pkcs7.signers(&empty, Pkcs7Flags::empty()).unwrap();
        let certificates = pkcs7
            .signed()
            .unwrap()
            .certificates()
            .unwrap()
            .iter()
            .map(|cert| cert.to_der().unwrap())
            .collect::<Vec<_>>();
        for signer in signers {
            super::super::smime_trust::portable::verify(&signer.to_der().unwrap(), &certificates)
                .unwrap();
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Recipient", "recipient@satchel.local");
        let plaintext = b"hello encrypted world";
        let encrypted = OpensslLibEngine
            .encrypt(std::slice::from_ref(&leaf.cert_pem), plaintext)
            .unwrap();
        let decrypted = OpensslLibEngine
            .decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted)
            .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let ca = issue_ca();
        let recipient = issue_leaf(&ca, "Satchel Recipient", "recipient@satchel.local");
        let stranger = issue_leaf(&ca, "Satchel Stranger", "stranger@satchel.local");
        let encrypted = OpensslLibEngine
            .encrypt(
                std::slice::from_ref(&recipient.cert_pem),
                b"not for the stranger",
            )
            .unwrap();
        assert!(OpensslLibEngine
            .decrypt(&stranger.key_pem, &stranger.cert_pem, None, &encrypted)
            .is_err());
    }

    #[test]
    fn encrypt_rejects_empty_recipient_list() {
        assert!(OpensslLibEngine.encrypt(&[], b"payload").is_err());
    }

    #[test]
    fn pkcs12_import_roundtrip() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Bundle", "bundle@satchel.local");
        let key = PKey::private_key_from_pem(leaf.key_pem.as_bytes()).unwrap();
        let bundle = Pkcs12::builder()
            .name("satchel")
            .pkey(&key)
            .cert(&leaf.cert)
            .build2("hunter2")
            .unwrap();

        let (cert_pem, key_pem) = OpensslLibEngine
            .extract_pkcs12(&bundle.to_der().unwrap(), "hunter2")
            .unwrap();
        assert_eq!(cert_pem.trim(), leaf.cert_pem.trim());
        assert!(key_pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert!(OpensslLibEngine
            .verify_private_key_passphrase(&key_pem, None)
            .unwrap());

        // The extracted material must still sign for its certificate.
        let signed = OpensslLibEngine
            .sign(&key_pem, &cert_pem, None, b"bundled")
            .unwrap();
        assert!(
            OpensslLibEngine
                .verify_against(&signed, &store_trusting(&ca.0), Pkcs7Flags::empty())
                .valid
        );
    }

    #[test]
    fn pkcs12_import_rejects_wrong_passphrase() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Bundle", "bundle@satchel.local");
        let key = PKey::private_key_from_pem(leaf.key_pem.as_bytes()).unwrap();
        let bundle = Pkcs12::builder()
            .name("satchel")
            .pkey(&key)
            .cert(&leaf.cert)
            .build2("hunter2")
            .unwrap();
        assert!(OpensslLibEngine
            .extract_pkcs12(&bundle.to_der().unwrap(), "wrong")
            .is_err());
    }

    #[test]
    fn generate_keypair_without_passphrase() {
        let generated = OpensslLibEngine
            .generate_keypair(&GenerateSmimeRequest {
                name: "Satchel Test",
                email: "smime-test@satchel.local",
                passphrase: None,
            })
            .unwrap();
        assert!(generated
            .pem_private_key
            .starts_with("-----BEGIN PRIVATE KEY-----"));
        assert_eq!(
            generated.metadata.subject_cn.as_deref(),
            Some("Satchel Test")
        );
        assert_eq!(
            generated.metadata.subject_email.as_deref(),
            Some("smime-test@satchel.local")
        );
        assert_eq!(generated.metadata.fingerprint.len(), 64);
        assert!(generated
            .metadata
            .not_after
            .as_deref()
            .unwrap_or_default()
            .ends_with("GMT"));
        assert!(OpensslLibEngine
            .verify_private_key_passphrase(&generated.pem_private_key, None)
            .unwrap());
    }

    #[test]
    fn wrong_passphrase_never_reads_as_an_engine_error() {
        let generated = OpensslLibEngine
            .generate_keypair(&GenerateSmimeRequest {
                name: "Satchel Test",
                email: "smime-test@satchel.local",
                passphrase: Some("hunter2"),
            })
            .unwrap();
        // Roughly one guess in 400 pads out by chance; 3000 guesses make a
        // regression practically certain to surface.
        for i in 0..3000 {
            let guess = format!("wrong{i}");
            let verdict = OpensslLibEngine
                .verify_private_key_passphrase(&generated.pem_private_key, Some(&guess))
                .unwrap_or_else(|err| panic!("{guess}: {err}"));
            assert!(!verdict, "accepted {guess}");
        }
    }

    #[test]
    fn generate_keypair_with_passphrase() {
        let generated = OpensslLibEngine
            .generate_keypair(&GenerateSmimeRequest {
                name: "Satchel Test",
                email: "smime-test@satchel.local",
                passphrase: Some("hunter2"),
            })
            .unwrap();
        assert!(generated
            .pem_private_key
            .starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----"));
        assert!(OpensslLibEngine
            .verify_private_key_passphrase(&generated.pem_private_key, Some("hunter2"))
            .unwrap());
        assert!(!OpensslLibEngine
            .verify_private_key_passphrase(&generated.pem_private_key, Some("wrong"))
            .unwrap());
        assert!(!OpensslLibEngine
            .verify_private_key_passphrase(&generated.pem_private_key, None)
            .unwrap());

        let signed = OpensslLibEngine
            .sign(
                &generated.pem_private_key,
                &generated.pem_certificate,
                Some("hunter2"),
                b"passphrase protected",
            )
            .unwrap();
        assert!(!OpensslLibEngine
            .extract_certificates(&signed)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn certificate_metadata_matches_openssl_fingerprint() {
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let metadata = OpensslLibEngine
            .certificate_metadata(&leaf.cert_pem)
            .unwrap();
        let expected = leaf
            .cert
            .digest(MessageDigest::sha256())
            .unwrap()
            .iter()
            .fold(String::new(), |mut acc, byte| {
                let _ = write!(acc, "{byte:02X}");
                acc
            });
        assert_eq!(metadata.fingerprint, expected);
        assert_eq!(metadata.issuer_cn.as_deref(), Some("Satchel Test CA"));
    }

    #[test]
    fn opaque_der_input_is_readable() {
        // `application/pkcs7-mime` parts reach the engine as bare DER, without
        // the S/MIME headers.
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let signed = OpensslLibEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, b"bare der")
            .unwrap();
        let der = read_pkcs7(&signed).unwrap().0.to_der().unwrap();
        assert_eq!(
            OpensslLibEngine.extract_certificates(&der).unwrap().len(),
            1
        );
        assert!(
            OpensslLibEngine
                .verify_against(&der, &store_trusting(&ca.0), Pkcs7Flags::empty())
                .valid
        );
    }

    #[test]
    fn engine_selection_defaults_to_in_process() {
        assert_eq!(active_engine().id(), "openssl-lib");
    }

    // --- cross-checks against the openssl binary ---------------------------

    #[test]
    fn binary_verifies_in_process_signature() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let signed = OpensslLibEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, b"cross-check payload")
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let signed_path = tmp.path().join("signed.eml");
        let ca_path = tmp.path().join("ca.pem");
        std::fs::write(&signed_path, &signed).unwrap();
        std::fs::write(&ca_path, cert_to_pem(&ca.0).unwrap()).unwrap();
        let verified = OpensslCliEngine
            .run(
                &[
                    "smime",
                    "-verify",
                    "-inform",
                    "SMIME",
                    "-in",
                    &signed_path.to_string_lossy(),
                    "-CAfile",
                    &ca_path.to_string_lossy(),
                ],
                None,
            )
            .expect("openssl binary verifies the in-process signature");
        assert_eq!(verified, b"cross-check payload");
    }

    #[test]
    fn in_process_verifies_binary_signature() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let signed = OpensslCliEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, b"cross-check payload")
            .unwrap();

        let result =
            OpensslLibEngine.verify_against(&signed, &store_trusting(&ca.0), Pkcs7Flags::empty());
        assert!(result.valid, "verify failed: {:?}", result.error);
        assert_eq!(
            result.signer_cert_pem.as_deref().map(str::trim),
            Some(leaf.cert_pem.trim())
        );
    }

    #[test]
    fn in_process_decrypts_binary_envelope() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Recipient", "recipient@satchel.local");
        let plaintext = b"binary enveloped payload";
        let encrypted = OpensslCliEngine
            .encrypt(std::slice::from_ref(&leaf.cert_pem), plaintext)
            .unwrap();
        let decrypted = OpensslLibEngine
            .decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted)
            .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn binary_decrypts_in_process_envelope() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Recipient", "recipient@satchel.local");
        let plaintext = b"in-process enveloped payload";
        let encrypted = OpensslLibEngine
            .encrypt(std::slice::from_ref(&leaf.cert_pem), plaintext)
            .unwrap();
        let decrypted = OpensslCliEngine
            .decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted)
            .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn in_process_reads_rsa_oaep_envelopes() {
        // The regression the CLI engine switched to `openssl cms` for: OAEP key
        // transport, which the PKCS#7 decryptor rejects.
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Recipient", "recipient@satchel.local");
        let tmp = tempfile::tempdir().unwrap();
        let cert_path = tmp.path().join("recipient.pem");
        let input_path = tmp.path().join("payload.txt");
        let encrypted_path = tmp.path().join("encrypted.eml");
        let plaintext = b"Satchel OAEP regression payload";
        std::fs::write(&cert_path, &leaf.cert_pem).unwrap();
        std::fs::write(&input_path, plaintext).unwrap();

        OpensslCliEngine
            .run(
                &[
                    "cms",
                    "-encrypt",
                    "-binary",
                    "-aes-128-cbc",
                    "-in",
                    &input_path.to_string_lossy(),
                    "-out",
                    &encrypted_path.to_string_lossy(),
                    "-outform",
                    "SMIME",
                    "-recip",
                    &cert_path.to_string_lossy(),
                    "-keyopt",
                    "rsa_padding_mode:oaep",
                    "-keyopt",
                    "rsa_oaep_md:sha256",
                    "-keyopt",
                    "rsa_mgf1_md:sha256",
                ],
                None,
            )
            .expect("generate the OAEP fixture");

        let encrypted = std::fs::read(&encrypted_path).unwrap();
        let decrypted = OpensslLibEngine
            .decrypt(&leaf.key_pem, &leaf.cert_pem, None, &encrypted)
            .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn both_engines_agree_on_certificate_metadata() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let from_lib = OpensslLibEngine
            .certificate_metadata(&leaf.cert_pem)
            .unwrap();
        let from_cli = OpensslCliEngine
            .certificate_metadata(&leaf.cert_pem)
            .unwrap();
        assert_eq!(from_lib.fingerprint, from_cli.fingerprint);
        assert_eq!(from_lib.subject_cn, from_cli.subject_cn);
        assert_eq!(from_lib.subject_email, from_cli.subject_email);
        assert_eq!(from_lib.issuer_cn, from_cli.issuer_cn);
        assert_eq!(from_lib.not_before, from_cli.not_before);
        assert_eq!(from_lib.not_after, from_cli.not_after);
    }

    #[test]
    fn both_engines_write_the_same_smime_headers() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let ca = issue_ca();
        let leaf = issue_leaf(&ca, "Satchel Signer", "signer@satchel.local");
        let payload = b"header comparison";

        let lib_signed = OpensslLibEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, payload)
            .unwrap();
        let cli_signed = OpensslCliEngine
            .sign(&leaf.key_pem, &leaf.cert_pem, None, payload)
            .unwrap();
        assert_eq!(headers_of(&lib_signed), headers_of(&cli_signed));

        let lib_encrypted = OpensslLibEngine
            .encrypt(std::slice::from_ref(&leaf.cert_pem), payload)
            .unwrap();
        let cli_encrypted = OpensslCliEngine
            .encrypt(std::slice::from_ref(&leaf.cert_pem), payload)
            .unwrap();
        assert_eq!(headers_of(&lib_encrypted), headers_of(&cli_encrypted));
    }

    #[test]
    fn cli_passphrases_never_reach_the_argument_vector() {
        // The six `-passin`/`-passout` sites are spread across the CLI engine;
        // pin the whole file rather than one of them.
        let needle = format!("{}{}", "pass", ":{");
        let source = include_str!("smime_engine.rs");
        assert!(
            !source.contains(&needle),
            "a call site builds an openssl passphrase argument again"
        );
    }

    #[test]
    fn cli_reads_the_passphrase_from_the_pipe() {
        let Some(_) = openssl_binary() else {
            eprintln!("skipping: no openssl binary on PATH");
            return;
        };
        let passphrase = "correct horse battery staple";
        let generated = OpensslCliEngine
            .generate_keypair(&GenerateSmimeRequest {
                name: "Satchel Signer",
                email: "signer@satchel.local",
                passphrase: Some(passphrase),
            })
            .expect("-passout over the pipe produces an encrypted key");
        assert!(generated.pem_private_key.contains("ENCRYPTED"));

        assert!(OpensslCliEngine
            .verify_private_key_passphrase(&generated.pem_private_key, Some(passphrase))
            .expect("-passin over the pipe reaches openssl"));
        assert!(!OpensslCliEngine
            .verify_private_key_passphrase(&generated.pem_private_key, Some("wrong"))
            .expect("a wrong passphrase is an answer, not an error"));

        let signed = OpensslCliEngine
            .sign(
                &generated.pem_private_key,
                &generated.pem_certificate,
                Some(passphrase),
                b"pipe-fed passphrase payload",
            )
            .expect("signing unlocks the key over the pipe");
        assert!(!signed.is_empty());
    }
}
