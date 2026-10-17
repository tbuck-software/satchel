//! OpenPGP operations behind one trait, with two implementations.
//!
//! [`RpgpEngine`] runs in-process on top of the pure-Rust `pgp` crate and is
//! the default: Android and iOS ship no `gpg` binary to shell out to (ADR
//! 0007). [`GpgCliEngine`] stays selectable through `SATCHEL_PGP_ENGINE=gpg`
//! for people whose secret key lives on a hardware token that only `gpg-agent`
//! can talk to.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::OnceLock,
    time::{Duration as StdDuration, SystemTime, UNIX_EPOCH},
};

use pgp::{
    composed::{
        ArmorOptions, CleartextSignedMessage, DecryptionOptions, Deserializable, DetachedSignature,
        EncryptionCaps, KeyType, Message, MessageBuilder, SecretKeyParamsBuilder, SignedPublicKey,
        SignedPublicSubKey, SignedSecretKey, SubkeyParamsBuilder, TheRing,
    },
    crypto::{
        ecc_curve::ECCCurve, hash::HashAlgorithm, public_key::PublicKeyAlgorithm,
        sym::SymmetricKeyAlgorithm,
    },
    packet::{self, Signature, SignatureConfig, SignatureType, Subpacket, SubpacketData},
    ser::Serialize as _,
    types::{
        Duration as PgpDuration, KeyDetails as _, Password, PublicParams, SecretParams, SignedUser,
        Tag,
    },
};

#[derive(Debug, Clone)]
pub struct GenerateKeyRequest<'a> {
    pub uid: &'a str,
    pub algorithm: &'a str,
    pub expires_in: &'a str,
    pub passphrase: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct GeneratedKeyPair {
    pub fingerprint: String,
    pub private_armored: String,
    pub public_armored: String,
}

/// What a key says about itself, read from its self-signatures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgpKeyMetadata {
    /// Uppercase hex, no separators.
    pub fingerprint: String,
    pub user_ids: Vec<String>,
    /// gpg-style name, e.g. `ed25519`, `rsa3072`, `cv25519`.
    pub algorithm: String,
    pub created_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_expired: bool,
    pub has_secret_key: bool,
    pub can_sign: bool,
    pub can_encrypt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCleartextMessage {
    pub cleartext: String,
    pub signer_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleartextVerification {
    Valid(VerifiedCleartextMessage),
    /// The signature does not identify this certificate.
    NotSignedByCertificate {
        cleartext: String,
    },
    /// The signature identifies this certificate but does not verify.
    Invalid {
        cleartext: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachedVerification {
    Valid { signer_fingerprint: String },
    NotSignedByCertificate,
    Invalid,
}

const MAX_COMPRESSION_DEPTH: usize = 8;
const MAX_DECRYPTED_BYTES: usize = 64 * 1024 * 1024;

pub trait PgpEngine: Send + Sync {
    fn id(&self) -> &'static str;
    fn ensure_available(&self) -> Result<(), String>;
    fn verify_private_key_passphrase(
        &self,
        armored_key: &str,
        passphrase: &str,
    ) -> Result<bool, String>;
    fn decrypt_with_private_key(
        &self,
        armored_key: &str,
        passphrase: &str,
        ciphertext: &str,
    ) -> Result<String, String>;
    fn generate_keypair(
        &self,
        request: &GenerateKeyRequest<'_>,
    ) -> Result<GeneratedKeyPair, String>;

    /// Detached OpenPGP signature over `payload`, armored.
    ///
    /// Detached rather than inline: RFC 3156 multipart/signed keeps the signed
    /// body readable by clients that do not understand PGP.
    fn sign_detached(
        &self,
        armored_private_key: &str,
        passphrase: &str,
        payload: &[u8],
    ) -> Result<String, String>;

    /// Verify a cleartext-signed message against exactly one certificate.
    fn verify_cleartext(
        &self,
        armored_certificate: &str,
        signed_message: &str,
    ) -> Result<CleartextVerification, String>;

    /// Encrypt `payload` to every recipient public key, armored.
    fn encrypt_to(&self, armored_public_keys: &[String], payload: &[u8]) -> Result<String, String>;

    /// Read fingerprint, user ids, algorithm and expiry off a key block.
    fn key_metadata(&self, armored_key: &str) -> Result<PgpKeyMetadata, String>;
}

/// Identifier of the engine currently backing PGP operations.
pub fn active_engine_id() -> &'static str {
    active_engine().id()
}

pub fn ensure_engine_available() -> Result<(), String> {
    active_engine().ensure_available()
}

pub fn verify_private_key_passphrase(armored_key: &str, passphrase: &str) -> Result<bool, String> {
    active_engine().verify_private_key_passphrase(armored_key, passphrase)
}

pub fn decrypt_with_private_key(
    armored_key: &str,
    passphrase: &str,
    ciphertext: &str,
) -> Result<String, String> {
    active_engine().decrypt_with_private_key(armored_key, passphrase, ciphertext)
}

pub fn generate_keypair(request: &GenerateKeyRequest<'_>) -> Result<GeneratedKeyPair, String> {
    active_engine().generate_keypair(request)
}

pub fn sign_detached(
    armored_private_key: &str,
    passphrase: &str,
    payload: &[u8],
) -> Result<String, String> {
    active_engine().sign_detached(armored_private_key, passphrase, payload)
}

pub fn verify_cleartext(
    armored_certificate: &str,
    signed_message: &str,
) -> Result<CleartextVerification, String> {
    active_engine().verify_cleartext(armored_certificate, signed_message)
}

/// Extract the readable text from a cleartext-signed OpenPGP message without
/// claiming that its signature is trusted. Callers use this when no matching
/// certificate is installed and must still show the message as unverified.
pub fn parse_cleartext_signed_message(signed: &str) -> Result<String, String> {
    parse_cleartext_signed(signed).map(|message| message.signed_text())
}

pub fn verify_detached(
    armored_certificate: &str,
    payload: &[u8],
    armored_signature: &str,
) -> Result<DetachedVerification, String> {
    verify_detached_with_certificate(armored_certificate, payload, armored_signature)
}

pub fn encrypt_to(armored_public_keys: &[String], payload: &[u8]) -> Result<String, String> {
    active_engine().encrypt_to(armored_public_keys, payload)
}

pub fn key_metadata(armored_key: &str) -> Result<PgpKeyMetadata, String> {
    active_engine().key_metadata(armored_key)
}

/// Split a public or private armor block into one independently armored
/// certificate per entry. Keyring exports commonly place several certificates
/// in one armor block, but callers must store and address them separately.
pub fn split_armored_keys(armored: &str) -> Result<Vec<String>, String> {
    let trimmed = armored.trim();
    if trimmed.to_ascii_uppercase().contains("PRIVATE KEY BLOCK") {
        return RpgpEngine::parse_secret_keys(trimmed)?
            .into_iter()
            .map(|key| {
                key.to_armored_string(ArmorOptions::default())
                    .map_err(|err| format!("could not armor a private key: {err}"))
            })
            .collect();
    }

    RpgpEngine::parse_certificates(trimmed)?
        .into_iter()
        .map(|cert| {
            cert.to_armored_string(ArmorOptions::default())
                .map_err(|err| format!("could not armor a public key: {err}"))
        })
        .collect()
}

/// Pick an engine by name. Unknown names fall back to the in-process one.
///
/// Public so tests can drive both engines in one process; `SATCHEL_PGP_ENGINE`
/// is process-wide and cannot be flipped per test.
pub fn engine_by_id(id: &str) -> Box<dyn PgpEngine> {
    match id.trim().to_ascii_lowercase().as_str() {
        "gpg" | "gpg-cli" | "cli" | "binary" => Box::new(GpgCliEngine),
        _ => Box::new(RpgpEngine),
    }
}

/// `SATCHEL_PGP_ENGINE` mirrors `SATCHEL_SMIME_ENGINE`: in-process by default,
/// the binary on request.
fn active_engine() -> Box<dyn PgpEngine> {
    engine_by_id(&std::env::var("SATCHEL_PGP_ENGINE").unwrap_or_default())
}

// ---------------------------------------------------------------------------
// In-process engine
// ---------------------------------------------------------------------------

/// OpenPGP through the `pgp` crate (rPGP), no binary and no keyring on disk.
///
/// Every operation takes the armored key material it needs as an argument, the
/// way the callers already hold it: Satchel's keys live encrypted in the
/// `pgp_keys` table, not in `~/.gnupg`.
struct RpgpEngine;

/// Which component of a certificate an operation is going to use. Primary key
/// and subkey are distinct packet types, so the choice cannot be a trait
/// object — `SigningKey` and `EncryptionKey` are generic over `Sized`.
enum SecretComponent<'a> {
    Primary(&'a packet::SecretKey),
    Subkey(&'a packet::SecretSubkey),
}

impl RpgpEngine {
    fn parse_secret_keys(armored: &str) -> Result<Vec<SignedSecretKey>, String> {
        let (keys, _) = SignedSecretKey::from_string_many(armored.trim())
            .map_err(|err| format!("not a usable OpenPGP private key: {err}"))?;
        let keys = keys
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("not a usable OpenPGP private key: {err}"))?;
        non_empty(keys)
    }

    fn parse_secret_key(armored: &str) -> Result<SignedSecretKey, String> {
        Self::parse_secret_keys(armored)?
            .into_iter()
            .next()
            .ok_or_else(|| "key block contained no OpenPGP key".to_string())
    }

    /// Certificates in one armor block. `gpg --export` packs several keys into
    /// a single block, so this must not stop after the first.
    fn parse_certificates(armored: &str) -> Result<Vec<SignedPublicKey>, String> {
        let trimmed = armored.trim();
        if trimmed.to_ascii_uppercase().contains("PRIVATE KEY BLOCK") {
            let (keys, _) = SignedSecretKey::from_string_many(trimmed)
                .map_err(|err| format!("not a usable OpenPGP key: {err}"))?;
            let certs = keys
                .map(|key| key.map(|key| key.to_public_key()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| format!("not a usable OpenPGP key: {err}"))?;
            return non_empty(certs);
        }

        let (keys, _) = SignedPublicKey::from_string_many(trimmed)
            .map_err(|err| format!("not a usable OpenPGP public key: {err}"))?;
        let certs = keys
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("not a usable OpenPGP public key: {err}"))?;
        non_empty(certs)
    }

    fn parse_one_certificate(armored: &str) -> Result<SignedPublicKey, String> {
        let mut certs = Self::parse_certificates(armored)?;
        if certs.len() != 1 {
            return Err(format!(
                "expected exactly one OpenPGP certificate, found {}",
                certs.len()
            ));
        }
        Ok(certs.remove(0))
    }

    /// The component that signs: the primary when it carries the sign flag,
    /// otherwise the first signing-capable subkey. Keys generated by gpg sign
    /// with the primary; keys from other tools often carry a signing subkey.
    fn signing_component(key: &SignedSecretKey) -> Result<SecretComponent<'_>, String> {
        let cert = key.to_public_key();
        ensure_certificate_active(&cert)?;

        if primary_can_sign(&cert) {
            return Ok(SecretComponent::Primary(&key.primary_key));
        }

        key.secret_subkeys
            .iter()
            .find(|sub| {
                sub.key.algorithm().can_sign()
                    && active_subkey_binding(
                        &cert.primary_key,
                        sub.key.public_key(),
                        &sub.signatures,
                    )
                    .is_some_and(|sig| {
                        sig.key_flags().sign()
                            && valid_back_signature(sig, sub.key.public_key(), &cert.primary_key)
                    })
            })
            .map(|sub| SecretComponent::Subkey(&sub.key))
            .ok_or_else(|| "key has no component that can sign".to_string())
    }

    /// The subkey a message should be encrypted to, with its binding signature
    /// checked. An unverified binding would let anyone graft their own subkey
    /// onto someone else's certificate and read the mail.
    fn encryption_subkey(cert: &SignedPublicKey) -> Option<&SignedPublicSubKey> {
        cert.public_subkeys.iter().find(|sub| {
            sub.key.algorithm().can_encrypt()
                && active_subkey_binding(&cert.primary_key, &sub.key, &sub.signatures).is_some_and(
                    |sig| sig.key_flags().encrypt_comms() || sig.key_flags().encrypt_storage(),
                )
        })
    }

    fn password(passphrase: &str) -> Password {
        if passphrase.is_empty() {
            Password::empty()
        } else {
            Password::from(passphrase)
        }
    }

    /// Key shape per `algorithm`, as `(primary, encryption subkey)`.
    ///
    /// `default` matches what `gpg --quick-gen-key … default` produces since
    /// GnuPG 2.3: an Ed25519 primary with a Curve25519 encryption subkey.
    fn key_types(algorithm: &str) -> Result<(KeyType, KeyType), String> {
        let curve25519 = (
            KeyType::Ed25519Legacy,
            KeyType::ECDH(ECCCurve::Curve25519Legacy),
        );
        let normalized = algorithm
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "");

        match normalized.as_str() {
            "" | "default" | "futuredefault" | "ed25519" | "ed25519cv25519" | "eddsa"
            | "cv25519" => Ok(curve25519),
            "rsa" | "rsa3072" => Ok((KeyType::Rsa(3072), KeyType::Rsa(3072))),
            "rsa2048" => Ok((KeyType::Rsa(2048), KeyType::Rsa(2048))),
            "rsa4096" => Ok((KeyType::Rsa(4096), KeyType::Rsa(4096))),
            other => Err(format!("unsupported key algorithm '{other}'")),
        }
    }

    fn build_keypair(
        request: &GenerateKeyRequest<'_>,
        created_at: SystemTime,
    ) -> Result<SignedSecretKey, String> {
        Self::build_keypair_with_symmetric_preferences(
            request,
            created_at,
            &[
                SymmetricKeyAlgorithm::AES256,
                SymmetricKeyAlgorithm::AES192,
                SymmetricKeyAlgorithm::AES128,
            ],
        )
    }

    fn build_keypair_with_symmetric_preferences(
        request: &GenerateKeyRequest<'_>,
        created_at: SystemTime,
        symmetric_preferences: &[SymmetricKeyAlgorithm],
    ) -> Result<SignedSecretKey, String> {
        let (primary_type, subkey_type) = Self::key_types(request.algorithm)?;
        let created_at = created_at
            .try_into()
            .map_err(|_| "key creation time is out of range".to_string())?;
        let passphrase = request
            .passphrase
            .map(str::to_string)
            .filter(|value| !value.is_empty());

        let mut subkey = SubkeyParamsBuilder::default();
        subkey
            .key_type(subkey_type)
            .created_at(created_at)
            .can_encrypt(EncryptionCaps::All)
            .can_sign(false)
            .can_authenticate(false)
            .passphrase(passphrase.clone());

        let mut params = SecretKeyParamsBuilder::default();
        params
            .key_type(primary_type)
            .created_at(created_at)
            .can_certify(true)
            .can_sign(true)
            .can_encrypt(EncryptionCaps::None)
            .primary_user_id(request.uid.to_string())
            .passphrase(passphrase)
            // Without preferences a peer must assume TripleDES and SHA-1 per
            // RFC 4880 §13.
            .preferred_symmetric_algorithms(symmetric_preferences.iter().copied().collect())
            .preferred_hash_algorithms(
                [
                    HashAlgorithm::Sha512,
                    HashAlgorithm::Sha384,
                    HashAlgorithm::Sha256,
                ]
                .into_iter()
                .collect(),
            )
            .subkeys(vec![subkey
                .build()
                .map_err(|err| format!("invalid subkey parameters: {err}"))?]);

        let key = params
            .build()
            .map_err(|err| format!("invalid key parameters: {err}"))?
            .generate(rand::thread_rng())
            .map_err(|err| format!("key generation failed: {err}"))?;

        match parse_expiry(request.expires_in)? {
            Some(expiry) => apply_expiration(key, expiry, request.passphrase.unwrap_or_default()),
            None => Ok(key),
        }
    }
}

fn timestamp_secs(signature: &Signature) -> u64 {
    signature
        .created()
        .map(|created| u64::from(created.as_secs()))
        .unwrap_or_default()
}

fn signature_is_current(signature: &Signature) -> bool {
    let Some(expiry) = signature.signature_expiration_time() else {
        return true;
    };
    if expiry.as_secs() == 0 {
        return true;
    }

    timestamp_secs(signature)
        .checked_add(u64::from(expiry.as_secs()))
        .is_some_and(|expires| expires > now_secs())
}

const ALLOWED_CLOCK_SKEW_SECS: u64 = 5 * 60;

fn data_signature_is_current(signature: &Signature) -> bool {
    matches!(
        signature.typ(),
        Some(SignatureType::Binary | SignatureType::Text)
    ) && signature.created().is_some_and(|created| {
        u64::from(created.as_secs()) <= now_secs().saturating_add(ALLOWED_CLOCK_SKEW_SECS)
    }) && signature_is_current(signature)
}

fn key_is_current(signature: &Signature, created_at: u32) -> bool {
    let Some(expiry) = signature.key_expiration_time() else {
        return true;
    };
    if expiry.as_secs() == 0 {
        return true;
    }

    u64::from(created_at)
        .checked_add(u64::from(expiry.as_secs()))
        .is_some_and(|expires| expires > now_secs())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// The newest usable self-signature controls current primary-key policy.
/// Third-party certifications are ignored because they do not carry the key
/// owner's capability statement.
fn is_user_certification(signature: &Signature) -> bool {
    matches!(
        signature.typ(),
        Some(
            SignatureType::CertGeneric
                | SignatureType::CertPersona
                | SignatureType::CertCasual
                | SignatureType::CertPositive
        )
    )
}

fn valid_user_self_signature<'a>(
    cert: &SignedPublicKey,
    user: &'a SignedUser,
) -> Option<&'a Signature> {
    let revoked = user.signatures.iter().any(|signature| {
        signature.typ() == Some(SignatureType::CertRevocation)
            && signature
                .verify_certification(&cert.primary_key, Tag::UserId, &user.id)
                .is_ok()
    });
    if revoked {
        return None;
    }

    user.signatures
        .iter()
        .filter(|signature| {
            is_user_certification(signature)
                && signature
                    .verify_certification(&cert.primary_key, Tag::UserId, &user.id)
                    .is_ok()
                && signature_is_current(signature)
        })
        .max_by_key(|signature| timestamp_secs(signature))
}

fn latest_primary_signature(cert: &SignedPublicKey) -> Option<&Signature> {
    cert.details
        .users
        .iter()
        .filter_map(|user| valid_user_self_signature(cert, user))
        .chain(cert.details.direct_signatures.iter().filter(|signature| {
            signature.typ() == Some(SignatureType::Key)
                && signature.verify_key(&cert.primary_key).is_ok()
        }))
        .max_by_key(|signature| timestamp_secs(signature))
}

fn active_primary_signature(cert: &SignedPublicKey) -> Option<&Signature> {
    latest_primary_signature(cert).filter(|signature| {
        signature_is_current(signature)
            && key_is_current(signature, cert.primary_key.created_at().as_secs())
    })
}

fn certificate_is_revoked(cert: &SignedPublicKey) -> bool {
    cert.details.revocation_signatures.iter().any(|signature| {
        signature.typ() == Some(SignatureType::KeyRevocation)
            && signature.verify_key(&cert.primary_key).is_ok()
    })
}

fn ensure_certificate_active(cert: &SignedPublicKey) -> Result<(), String> {
    if certificate_is_revoked(cert) {
        return Err(format!(
            "OpenPGP key {} is revoked",
            hex_fingerprint(&cert.fingerprint())
        ));
    }
    if active_primary_signature(cert).is_none() {
        return Err(format!(
            "OpenPGP key {} has no current valid self-signature",
            hex_fingerprint(&cert.fingerprint())
        ));
    }
    Ok(())
}

fn primary_can_sign(cert: &SignedPublicKey) -> bool {
    cert.primary_key.algorithm().can_sign()
        && active_primary_signature(cert).is_some_and(|signature| signature.key_flags().sign())
}

fn primary_can_encrypt(cert: &SignedPublicKey) -> bool {
    cert.primary_key.algorithm().can_encrypt()
        && active_primary_signature(cert).is_some_and(|signature| {
            signature.key_flags().encrypt_comms() || signature.key_flags().encrypt_storage()
        })
}

fn active_subkey_binding<'a>(
    primary: &packet::PublicKey,
    subkey: &packet::PublicSubkey,
    signatures: &'a [Signature],
) -> Option<&'a Signature> {
    let revoked = signatures.iter().any(|signature| {
        signature.typ() == Some(SignatureType::SubkeyRevocation)
            && signature.verify_subkey_binding(primary, subkey).is_ok()
    });
    if revoked {
        return None;
    }

    signatures
        .iter()
        .filter(|signature| {
            signature.typ() == Some(SignatureType::SubkeyBinding)
                && signature.verify_subkey_binding(primary, subkey).is_ok()
                && signature_is_current(signature)
                && key_is_current(signature, subkey.created_at().as_secs())
        })
        .max_by_key(|signature| timestamp_secs(signature))
}

fn valid_back_signature(
    binding: &Signature,
    subkey: &packet::PublicSubkey,
    primary: &packet::PublicKey,
) -> bool {
    binding.embedded_signature().is_some_and(|signature| {
        signature
            .verify_primary_key_binding(subkey, primary)
            .is_ok()
    })
}

fn signature_packet_names_certificate(signature: &Signature, cert: &SignedPublicKey) -> bool {
    let fingerprints = std::iter::once(&cert.primary_key)
        .map(|key| key.fingerprint())
        .chain(cert.public_subkeys.iter().map(|sub| sub.key.fingerprint()))
        .collect::<Vec<_>>();
    let key_ids = std::iter::once(&cert.primary_key)
        .map(|key| key.legacy_key_id())
        .chain(
            cert.public_subkeys
                .iter()
                .map(|sub| sub.key.legacy_key_id()),
        )
        .collect::<Vec<_>>();

    signature
        .issuer_fingerprint()
        .into_iter()
        .any(|issuer| fingerprints.iter().any(|known| issuer == known))
        || signature
            .issuer_key_id()
            .into_iter()
            .any(|issuer| key_ids.iter().any(|known| issuer == known))
}

fn signature_names_certificate(message: &CleartextSignedMessage, cert: &SignedPublicKey) -> bool {
    message
        .signatures()
        .iter()
        .any(|signature| signature_packet_names_certificate(signature, cert))
}

fn parse_cleartext_signed(signed: &str) -> Result<CleartextSignedMessage, String> {
    CleartextSignedMessage::from_string(signed)
        .map(|(message, _)| message)
        .map_err(|err| format!("not a usable cleartext-signed OpenPGP message: {err}"))
}

fn verify_cleartext_with_certificate(
    armored_certificate: &str,
    signed_message: &str,
) -> Result<CleartextVerification, String> {
    let cert = RpgpEngine::parse_one_certificate(armored_certificate)?;
    let message = parse_cleartext_signed(signed_message)?;
    let cleartext = message.signed_text();
    if !signature_names_certificate(&message, &cert) {
        return Ok(CleartextVerification::NotSignedByCertificate { cleartext });
    }
    ensure_certificate_active(&cert)?;

    let verified = (primary_can_sign(&cert)
        && message
            .verify(&cert.primary_key)
            .is_ok_and(data_signature_is_current))
        || cert.public_subkeys.iter().any(|sub| {
            active_subkey_binding(&cert.primary_key, &sub.key, &sub.signatures).is_some_and(
                |binding| {
                    binding.key_flags().sign()
                        && valid_back_signature(binding, &sub.key, &cert.primary_key)
                        && message
                            .verify(&sub.key)
                            .is_ok_and(data_signature_is_current)
                },
            )
        });

    if verified {
        return Ok(CleartextVerification::Valid(VerifiedCleartextMessage {
            cleartext,
            signer_fingerprint: hex_fingerprint(&cert.fingerprint()),
        }));
    }

    Ok(CleartextVerification::Invalid { cleartext })
}

fn verify_detached_with_certificate(
    armored_certificate: &str,
    payload: &[u8],
    armored_signature: &str,
) -> Result<DetachedVerification, String> {
    let cert = RpgpEngine::parse_one_certificate(armored_certificate)?;
    let (signature, _) = DetachedSignature::from_string(armored_signature)
        .map_err(|error| format!("not a usable detached OpenPGP signature: {error}"))?;
    let names_certificate = signature_packet_names_certificate(&signature.signature, &cert);
    if !names_certificate {
        return Ok(DetachedVerification::NotSignedByCertificate);
    }
    ensure_certificate_active(&cert)?;

    let verified = data_signature_is_current(&signature.signature)
        && ((primary_can_sign(&cert) && signature.verify(&cert.primary_key, payload).is_ok())
            || cert.public_subkeys.iter().any(|sub| {
                active_subkey_binding(&cert.primary_key, &sub.key, &sub.signatures).is_some_and(
                    |binding| {
                        binding.key_flags().sign()
                            && valid_back_signature(binding, &sub.key, &cert.primary_key)
                            && signature.verify(&sub.key, payload).is_ok()
                    },
                )
            }));

    if verified {
        Ok(DetachedVerification::Valid {
            signer_fingerprint: hex_fingerprint(&cert.fingerprint()),
        })
    } else {
        Ok(DetachedVerification::Invalid)
    }
}

fn preferred_symmetric_algorithms(cert: &SignedPublicKey) -> Vec<SymmetricKeyAlgorithm> {
    let preferred = latest_primary_signature(cert)
        .map(Signature::preferred_symmetric_algs)
        .unwrap_or_default();
    if preferred.is_empty() {
        vec![SymmetricKeyAlgorithm::TripleDES]
    } else {
        preferred.to_vec()
    }
}

fn select_symmetric_algorithm(
    certificates: &[SignedPublicKey],
) -> Result<SymmetricKeyAlgorithm, String> {
    let preferences = certificates
        .iter()
        .map(preferred_symmetric_algorithms)
        .collect::<Vec<_>>();
    let supported = [
        SymmetricKeyAlgorithm::AES256,
        SymmetricKeyAlgorithm::AES192,
        SymmetricKeyAlgorithm::AES128,
        SymmetricKeyAlgorithm::Camellia256,
        SymmetricKeyAlgorithm::Twofish,
        SymmetricKeyAlgorithm::Camellia192,
        SymmetricKeyAlgorithm::Camellia128,
        SymmetricKeyAlgorithm::CAST5,
        SymmetricKeyAlgorithm::TripleDES,
        SymmetricKeyAlgorithm::Blowfish,
        SymmetricKeyAlgorithm::IDEA,
    ];

    supported
        .into_iter()
        .find(|candidate| {
            preferences
                .iter()
                .all(|recipient| recipient.contains(candidate))
        })
        .ok_or_else(|| {
            "recipient certificates have no mutually supported symmetric algorithm".to_string()
        })
}

fn read_decrypted_message(
    mut message: Message<'_>,
    max_compression_depth: usize,
    max_plaintext_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut depth = 0_usize;
    while message.is_compressed() {
        if depth >= max_compression_depth {
            return Err(format!(
                "decompression nesting exceeds the limit of {max_compression_depth}"
            ));
        }
        message = message
            .decompress()
            .map_err(|err| format!("decompression failed: {err}"))?;
        depth += 1;
    }

    let mut limited = message.take(max_plaintext_bytes.saturating_add(1) as u64);
    let mut data = Vec::new();
    limited
        .read_to_end(&mut data)
        .map_err(|err| format!("reading the decrypted message failed: {err}"))?;
    if data.len() > max_plaintext_bytes {
        return Err(format!(
            "decrypted message exceeds the limit of {max_plaintext_bytes} bytes"
        ));
    }
    Ok(data)
}

impl PgpEngine for RpgpEngine {
    fn id(&self) -> &'static str {
        "rpgp"
    }

    fn ensure_available(&self) -> Result<(), String> {
        Ok(())
    }

    fn verify_private_key_passphrase(
        &self,
        armored_key: &str,
        passphrase: &str,
    ) -> Result<bool, String> {
        let keys = Self::parse_secret_keys(armored_key)?;
        let password = Self::password(passphrase);
        Ok(keys.iter().all(|key| {
            let primary_matches = matches!(key.primary_key.secret_params(), SecretParams::Plain(_))
                || key.primary_key.unlock(&password, |_, _| Ok(())).is_ok();
            let subkeys_match = key.secret_subkeys.iter().all(|subkey| {
                matches!(subkey.key.secret_params(), SecretParams::Plain(_))
                    || subkey.key.unlock(&password, |_, _| Ok(())).is_ok()
            });
            primary_matches && subkeys_match
        }))
    }

    fn decrypt_with_private_key(
        &self,
        armored_key: &str,
        passphrase: &str,
        ciphertext: &str,
    ) -> Result<String, String> {
        let keys = Self::parse_secret_keys(armored_key)?;
        let password = Self::password(passphrase);
        let mut last_error = None;

        for key in keys {
            let message = Message::from_reader(ciphertext.trim().as_bytes())
                .map_err(|err| format!("not a usable OpenPGP message: {err}"))?
                .0;
            let ring = TheRing {
                secret_keys: vec![&key],
                key_passwords: vec![&password],
                decrypt_options: DecryptionOptions::new().enable_gnupg_aead(),
                ..TheRing::default()
            };
            let decrypted = match message.decrypt_the_ring(ring, true) {
                Ok((decrypted, _)) => decrypted,
                Err(err) => {
                    last_error = Some(err.to_string());
                    continue;
                }
            };

            let data =
                read_decrypted_message(decrypted, MAX_COMPRESSION_DEPTH, MAX_DECRYPTED_BYTES)?;
            return String::from_utf8(data)
                .map_err(|err| format!("decrypted message is not UTF-8: {err}"));
        }

        Err(format!(
            "decryption failed: {}",
            last_error.unwrap_or_else(|| "no private key matched the message".to_string())
        ))
    }

    fn generate_keypair(
        &self,
        request: &GenerateKeyRequest<'_>,
    ) -> Result<GeneratedKeyPair, String> {
        let key = Self::build_keypair(request, SystemTime::now())?;
        let private_armored = key
            .to_armored_string(ArmorOptions::default())
            .map_err(|err| format!("could not export the generated private key: {err}"))?;
        let public_armored = key
            .to_public_key()
            .to_armored_string(ArmorOptions::default())
            .map_err(|err| format!("could not export the generated public key: {err}"))?;

        Ok(GeneratedKeyPair {
            fingerprint: hex_fingerprint(&key.fingerprint()),
            private_armored,
            public_armored,
        })
    }

    fn sign_detached(
        &self,
        armored_private_key: &str,
        passphrase: &str,
        payload: &[u8],
    ) -> Result<String, String> {
        let key = Self::parse_secret_key(armored_private_key)?;
        ensure_certificate_active(&key.to_public_key())?;
        let password = Self::password(passphrase);
        let mut rng = rand::thread_rng();

        // SHA-256 and no weaker: `pgp_mime` announces micalg=pgp-sha256, and
        // SHA-1 signatures are rejected by current verifiers.
        let signature = match Self::signing_component(&key)? {
            SecretComponent::Primary(component) => DetachedSignature::sign_binary_data(
                &mut rng,
                component,
                &password,
                HashAlgorithm::Sha256,
                payload,
            ),
            SecretComponent::Subkey(component) => DetachedSignature::sign_binary_data(
                &mut rng,
                component,
                &password,
                HashAlgorithm::Sha256,
                payload,
            ),
        }
        .map_err(|err| format!("signing failed: {err}"))?;

        signature
            .to_armored_string(ArmorOptions::default())
            .map_err(|err| format!("could not armor the signature: {err}"))
    }

    fn verify_cleartext(
        &self,
        armored_certificate: &str,
        signed_message: &str,
    ) -> Result<CleartextVerification, String> {
        verify_cleartext_with_certificate(armored_certificate, signed_message)
    }

    fn encrypt_to(&self, armored_public_keys: &[String], payload: &[u8]) -> Result<String, String> {
        if armored_public_keys.is_empty() {
            return Err("no recipient public keys supplied".to_string());
        }

        let mut certs = Vec::new();
        for block in armored_public_keys {
            certs.push(Self::parse_one_certificate(block)?);
        }
        for cert in &certs {
            ensure_certificate_active(cert)?;
        }
        let symmetric_algorithm = select_symmetric_algorithm(&certs)?;

        let mut rng = rand::thread_rng();
        // SEIPDv1 with an MDC: understood by every OpenPGP implementation in
        // use, unlike the v2/AEAD container.
        let mut builder = MessageBuilder::from_bytes("", payload.to_vec())
            .seipd_v1(&mut rng, symmetric_algorithm);

        let mut recipients = 0_usize;
        for cert in &certs {
            let added = match RpgpEngine::encryption_subkey(cert) {
                Some(sub) => builder.encrypt_to_key(&mut rng, &sub.key),
                // Single-key certificates (RSA without a subkey) encrypt to
                // the primary.
                None if primary_can_encrypt(cert) => {
                    builder.encrypt_to_key(&mut rng, &cert.primary_key)
                }
                None => {
                    return Err(format!(
                        "recipient key {} has no current encryption component",
                        hex_fingerprint(&cert.fingerprint())
                    ));
                }
            };
            added.map_err(|err| format!("could not encrypt to a recipient key: {err}"))?;
            recipients += 1;
        }

        if recipients == 0 {
            return Err("recipient keys contained no usable encryption key".to_string());
        }

        builder
            .to_armored_string(&mut rng, ArmorOptions::default())
            .map_err(|err| format!("encryption failed: {err}"))
    }

    fn key_metadata(&self, armored_key: &str) -> Result<PgpKeyMetadata, String> {
        let trimmed = armored_key.trim();
        let has_secret_key = trimmed.to_ascii_uppercase().contains("PRIVATE KEY BLOCK");
        let cert = Self::parse_one_certificate(trimmed)?;

        let created_at = SystemTime::UNIX_EPOCH
            + StdDuration::from_secs(u64::from(cert.primary_key.created_at().as_secs()));
        let primary_signature = latest_primary_signature(&cert);
        let expires_at = primary_signature
            .and_then(|signature| signature.key_expiration_time())
            .filter(|duration| duration.as_secs() != 0)
            .map(|duration| created_at + StdDuration::from_secs(u64::from(duration.as_secs())));

        let certificate_active = ensure_certificate_active(&cert).is_ok();
        let can_sign = certificate_active
            && (primary_can_sign(&cert)
                || cert.public_subkeys.iter().any(|sub| {
                    sub.key.algorithm().can_sign()
                        && active_subkey_binding(&cert.primary_key, &sub.key, &sub.signatures)
                            .is_some_and(|signature| signature.key_flags().sign())
                }));
        let can_encrypt = certificate_active
            && (Self::encryption_subkey(&cert).is_some() || primary_can_encrypt(&cert));

        Ok(PgpKeyMetadata {
            fingerprint: hex_fingerprint(&cert.fingerprint()),
            user_ids: cert
                .details
                .users
                .iter()
                .filter(|user| valid_user_self_signature(&cert, user).is_some())
                .filter_map(|user| user.id.as_str().map(str::to_string))
                .collect(),
            algorithm: algorithm_name(cert.primary_key.algorithm(), cert.public_params()),
            created_at: Some(rfc3339(created_at)),
            expires_at: expires_at.map(rfc3339),
            is_expired: certificate_is_revoked(&cert)
                || active_primary_signature(&cert).is_none()
                || expires_at
                    .map(|at| at <= SystemTime::now())
                    .unwrap_or(false),
            has_secret_key,
            can_sign,
            can_encrypt,
        })
    }
}

fn non_empty<T>(items: Vec<T>) -> Result<Vec<T>, String> {
    if items.is_empty() {
        return Err("key block contained no OpenPGP key".to_string());
    }
    Ok(items)
}

/// Re-issue the user id self-signatures with a key expiration time.
///
/// The key builder has no expiry setting, and expiry is not a property of the
/// key packet: it lives in the self-signature, so setting it means signing
/// again.
fn apply_expiration(
    mut key: SignedSecretKey,
    expiry: StdDuration,
    passphrase: &str,
) -> Result<SignedSecretKey, String> {
    let password = RpgpEngine::password(passphrase);
    let seconds =
        u32::try_from(expiry.as_secs()).map_err(|_| "expiry is too far out".to_string())?;
    let mut rng = rand::thread_rng();

    for index in 0..key.details.users.len() {
        let previous = key.details.users[index]
            .signatures
            .first()
            .and_then(|sig| sig.config().cloned())
            .ok_or_else(|| "generated key carries no usable self-signature".to_string())?;

        let mut config =
            SignatureConfig::from_key(&mut rng, &key.primary_key, SignatureType::CertPositive)
                .map_err(|err| format!("could not prepare the self-signature: {err}"))?;
        config.hashed_subpackets = previous
            .hashed_subpackets
            .into_iter()
            .filter(|sub| !matches!(sub.data, SubpacketData::KeyExpirationTime(_)))
            .collect();
        config.hashed_subpackets.push(
            Subpacket::regular(SubpacketData::KeyExpirationTime(PgpDuration::from_secs(
                seconds,
            )))
            .map_err(|err| format!("could not build the expiry subpacket: {err}"))?,
        );
        config.unhashed_subpackets = previous.unhashed_subpackets;

        let signature = config
            .sign_certification(
                &key.primary_key,
                key.primary_key.public_key(),
                &password,
                Tag::UserId,
                &key.details.users[index].id,
            )
            .map_err(|err| format!("could not sign the expiry into the key: {err}"))?;
        key.details.users[index].signatures = vec![signature];
    }

    Ok(key)
}

/// gpg's `--quick-gen-key` expiry syntax: `0`/`none` never expires, a bare
/// number counts days, and `s`, `d`, `w`, `m`, `y` suffixes are honoured.
fn parse_expiry(raw: &str) -> Result<Option<StdDuration>, String> {
    const DAY: u64 = 86_400;
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() || value == "0" || value == "none" || value == "never" {
        return Ok(None);
    }

    let (digits, seconds_per_unit) = match value.strip_suffix(['s', 'd', 'w', 'm', 'y']) {
        Some(digits) => {
            let unit = match value.as_bytes()[value.len() - 1] {
                b's' => 1,
                b'd' => DAY,
                b'w' => 7 * DAY,
                b'm' => 30 * DAY,
                _ => 365 * DAY,
            };
            (digits, unit)
        }
        None => (value.as_str(), DAY),
    };

    let amount = digits
        .parse::<u64>()
        .map_err(|_| format!("could not read '{raw}' as an expiry"))?;
    if amount == 0 {
        return Ok(None);
    }
    let seconds = amount
        .checked_mul(seconds_per_unit)
        .ok_or_else(|| "expiry is too far out".to_string())?;
    Ok(Some(StdDuration::from_secs(seconds)))
}

fn hex_fingerprint(fingerprint: &pgp::types::Fingerprint) -> String {
    fingerprint
        .as_bytes()
        .iter()
        .fold(String::new(), |mut acc, byte| {
            acc.push_str(&format!("{byte:02X}"));
            acc
        })
}

fn rfc3339(time: SystemTime) -> String {
    let seconds = time
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(seconds as i64, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

/// gpg's colon-format algorithm names, so the UI can show what a key is
/// regardless of which engine read it.
fn algorithm_name(algorithm: PublicKeyAlgorithm, params: &PublicParams) -> String {
    match params {
        PublicParams::RSA(rsa) => match rsa_bits(rsa) {
            Some(bits) => format!("rsa{bits}"),
            None => "rsa".to_string(),
        },
        PublicParams::ECDH(ecdh) => match ecdh.curve() {
            ECCCurve::Curve25519Legacy => "cv25519".to_string(),
            curve => curve.name().to_ascii_lowercase(),
        },
        PublicParams::ECDSA(ecdsa) => ecdsa.curve().name().to_ascii_lowercase(),
        // gpg calls the legacy EdDSA/ECDH curve pair ed25519 and cv25519.
        PublicParams::EdDSALegacy(eddsa) => match eddsa.curve() {
            ECCCurve::Ed25519Legacy => "ed25519".to_string(),
            curve => curve.name().to_ascii_lowercase(),
        },
        PublicParams::Ed25519(_) => "ed25519".to_string(),
        PublicParams::Ed448(_) => "ed448".to_string(),
        PublicParams::X25519(_) => "x25519".to_string(),
        PublicParams::X448(_) => "x448".to_string(),
        PublicParams::DSA(_) => "dsa".to_string(),
        PublicParams::Elgamal(_) => "elgamal".to_string(),
        _ => format!("{algorithm:?}").to_ascii_lowercase(),
    }
}

/// The modulus size, read back off the serialized MPI whose first two bytes
/// are its bit count — the `rsa` crate is not a dependency of this crate.
fn rsa_bits(params: &pgp::types::RsaPublicParams) -> Option<u16> {
    let mut buf = Vec::new();
    params.to_writer(&mut buf).ok()?;
    if buf.len() < 2 {
        return None;
    }
    Some(u16::from_be_bytes([buf[0], buf[1]]))
}

// ---------------------------------------------------------------------------
// gpg engine
// ---------------------------------------------------------------------------

/// Pull primary-key fingerprints out of `gpg --with-colons` output.
///
/// Colon format: a `fpr:` record belongs to the most recent `pub:` record, so
/// only the first `fpr:` after each `pub:` is the primary key; later ones are
/// subkeys.
fn primary_fingerprints(listing: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut expecting = false;
    for line in listing.lines() {
        if line.starts_with("pub:") {
            expecting = true;
            continue;
        }
        if line.starts_with("sub:") {
            expecting = false;
            continue;
        }
        if expecting && line.starts_with("fpr:") {
            if let Some(fpr) = line.split(':').nth(9) {
                if !fpr.is_empty() {
                    out.push(fpr.to_string());
                }
            }
            expecting = false;
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GpgSecretStorage {
    Local,
    Offline,
    Token(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpgSecretComponent {
    fingerprint: String,
    storage: GpgSecretStorage,
    can_sign: bool,
    can_decrypt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpgTokenKey {
    primary_fingerprint: String,
    serial: String,
    signing_fingerprints: Vec<String>,
    decryption_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GpgPrivateKeyMaterial {
    Software { primary_fingerprint: String },
    Token(GpgTokenKey),
}

#[derive(Debug)]
struct PendingSecretComponent {
    storage: GpgSecretStorage,
    can_sign: bool,
    can_decrypt: bool,
    primary: bool,
}

/// Read the stable `--with-colons --list-secret-keys` form after import.
/// Field 15 distinguishes a local secret (`+`), an unusable offline stub
/// (`#`) and a smartcard stub (the token serial number).
fn parse_gpg_private_key_material(listing: &str) -> Result<GpgPrivateKeyMaterial, String> {
    let mut pending = None;
    let mut components = Vec::new();
    let mut primary_fingerprints = Vec::new();

    for line in listing.lines() {
        let fields = line.split(':').collect::<Vec<_>>();
        match fields.first().copied() {
            Some(kind @ ("sec" | "ssb")) => {
                if pending.is_some() {
                    return Err("a secret-key record carried no fingerprint".to_string());
                }
                let token_field = fields.get(14).copied().unwrap_or_default().trim();
                let storage = match token_field {
                    "+" => GpgSecretStorage::Local,
                    "" | "#" => GpgSecretStorage::Offline,
                    serial => GpgSecretStorage::Token(serial.to_ascii_uppercase()),
                };
                let capabilities = fields.get(11).copied().unwrap_or_default();
                pending = Some(PendingSecretComponent {
                    storage,
                    can_sign: capabilities.contains('s'),
                    can_decrypt: capabilities.contains('e'),
                    primary: kind == "sec",
                });
            }
            Some("fpr") if pending.is_some() => {
                let component = pending.take().expect("guarded by is_some");
                let fingerprint = fields
                    .get(9)
                    .map(|value| compact_fingerprint(value))
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "a secret-key record carried no fingerprint".to_string())?;
                if component.primary {
                    primary_fingerprints.push(fingerprint.clone());
                }
                components.push(GpgSecretComponent {
                    fingerprint,
                    storage: component.storage,
                    can_sign: component.can_sign,
                    can_decrypt: component.can_decrypt,
                });
            }
            _ => {}
        }
    }

    if pending.is_some() {
        return Err("a secret-key record carried no fingerprint".to_string());
    }
    if primary_fingerprints.len() != 1 {
        return Err(format!(
            "expected exactly one OpenPGP secret certificate, found {}",
            primary_fingerprints.len()
        ));
    }

    let has_local = components
        .iter()
        .any(|component| component.storage == GpgSecretStorage::Local);
    let token_components = components
        .iter()
        .filter_map(|component| match &component.storage {
            GpgSecretStorage::Token(serial) => Some((component, serial)),
            _ => None,
        })
        .collect::<Vec<_>>();

    if token_components.is_empty() {
        if has_local
            && components
                .iter()
                .all(|component| component.storage == GpgSecretStorage::Local)
        {
            return Ok(GpgPrivateKeyMaterial::Software {
                primary_fingerprint: primary_fingerprints.remove(0),
            });
        }
        return Err(
            "the OpenPGP secret key contains only offline stubs or mixed secret material"
                .to_string(),
        );
    }
    if has_local {
        return Err(
            "an OpenPGP key may not mix local private material with hardware-token stubs"
                .to_string(),
        );
    }

    let serial = token_components[0].1.clone();
    if token_components
        .iter()
        .any(|(_, component_serial)| *component_serial != &serial)
    {
        return Err("one OpenPGP key may not reference multiple hardware tokens".to_string());
    }
    let signing_fingerprints = token_components
        .iter()
        .filter(|(component, _)| component.can_sign)
        .map(|(component, _)| component.fingerprint.clone())
        .collect::<Vec<_>>();
    let decryption_fingerprints = token_components
        .iter()
        .filter(|(component, _)| component.can_decrypt)
        .map(|(component, _)| component.fingerprint.clone())
        .collect::<Vec<_>>();
    if signing_fingerprints.is_empty() && decryption_fingerprints.is_empty() {
        return Err("the hardware token has no signing or decryption key".to_string());
    }

    Ok(GpgPrivateKeyMaterial::Token(GpgTokenKey {
        primary_fingerprint: primary_fingerprints.remove(0),
        serial,
        signing_fingerprints,
        decryption_fingerprints,
    }))
}

fn gpg_status_records(status: &str) -> impl Iterator<Item = Vec<&str>> {
    status.lines().filter_map(|line| {
        line.trim()
            .strip_prefix("[GNUPG:]")
            .map(|record| record.split_whitespace().collect())
    })
}

fn gpg_signature_status_matches(status: &str, expected_fingerprint: &str) -> bool {
    gpg_status_records(status).any(|fields| {
        fields.first() == Some(&"SIG_CREATED")
            && fields
                .last()
                .is_some_and(|fingerprint| compact_fingerprint(fingerprint) == expected_fingerprint)
    })
}

fn gpg_decryption_status_matches(status: &str, expected: &GpgTokenKey) -> bool {
    let okay = gpg_status_records(status).any(|fields| fields.first() == Some(&"DECRYPTION_OKAY"));
    let expected_key = gpg_status_records(status).any(|fields| {
        fields.first() == Some(&"DECRYPTION_KEY")
            && fields.get(1).is_some_and(|fingerprint| {
                expected
                    .decryption_fingerprints
                    .contains(&compact_fingerprint(fingerprint))
            })
            && fields.get(2).is_some_and(|fingerprint| {
                compact_fingerprint(fingerprint) == expected.primary_fingerprint
            })
    });
    okay && expected_key
}

fn gpg_card_status_matches(status: &str, expected_serial: &str) -> bool {
    gpg_status_records(status).any(|fields| {
        fields.first() == Some(&"CARDCTRL")
            && fields.get(1) == Some(&"3")
            && fields
                .get(2)
                .is_some_and(|serial| compact_fingerprint(serial) == expected_serial)
    })
}

struct GpgScratchHome {
    directory: tempfile::TempDir,
    gpgconf_candidates: Vec<PathBuf>,
}

impl GpgScratchHome {
    fn new(prefix: &str, gpg_binary: &str) -> Result<Self, String> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(prefix);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let directory = builder.tempdir().map_err(|err| err.to_string())?;
        let mut gpgconf_candidates = Vec::new();
        if let Some(configured) = crate::utils::environment::debug_env("SATCHEL_GPGCONF_BIN") {
            if !configured.trim().is_empty() {
                gpgconf_candidates.push(PathBuf::from(configured));
            }
        }
        if let Some(parent) = Path::new(gpg_binary).parent() {
            if !parent.as_os_str().is_empty() {
                gpgconf_candidates.push(parent.join(if cfg!(windows) {
                    "gpgconf.exe"
                } else {
                    "gpgconf"
                }));
            }
        }
        gpgconf_candidates.push(PathBuf::from(if cfg!(windows) {
            "gpgconf.exe"
        } else {
            "gpgconf"
        }));
        gpgconf_candidates.dedup();
        Ok(Self {
            directory,
            gpgconf_candidates,
        })
    }

    fn path(&self) -> &Path {
        self.directory.path()
    }
}

impl Drop for GpgScratchHome {
    fn drop(&mut self) {
        for candidate in &self.gpgconf_candidates {
            let stopped = Command::new(candidate)
                .arg("--homedir")
                .arg(self.directory.path())
                .args(["--kill", "all"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if stopped {
                break;
            }
        }
    }
}

struct GpgOperationOutput {
    stdout: String,
    status: String,
}

struct GpgCliEngine;

static GPG_BINARY_CACHE: OnceLock<Result<String, String>> = OnceLock::new();

impl GpgCliEngine {
    fn resolve_gpg_binary(&self) -> Result<String, String> {
        let cached = GPG_BINARY_CACHE.get_or_init(|| self.detect_gpg_binary());
        match cached {
            Ok(path) => Ok(path.clone()),
            Err(err) => Err(err.clone()),
        }
    }

    fn detect_gpg_binary(&self) -> Result<String, String> {
        if let Some(env_bin) = crate::utils::environment::debug_env("SATCHEL_GPG_BIN") {
            let candidate = env_bin.trim();
            if !candidate.is_empty() {
                if self.gpg_binary_works(candidate) {
                    return Ok(candidate.to_string());
                }
                return Err(format!(
                    "SATCHEL_GPG_BIN points to '{candidate}', but it is not executable."
                ));
            }
        }

        let candidates = [
            "gpg",
            "gpg2",
            "/opt/homebrew/bin/gpg",
            "/usr/local/bin/gpg",
            "/usr/bin/gpg",
        ];

        for candidate in candidates {
            if self.gpg_binary_works(candidate) {
                return Ok(candidate.to_string());
            }
        }

        Err(
            "gpg not found. Install GnuPG (`brew install gnupg`) or set SATCHEL_GPG_BIN=/path/to/gpg and restart Satchel."
                .to_string(),
        )
    }

    fn gpg_binary_works(&self, binary: &str) -> bool {
        Command::new(binary)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn run_gpg_command(
        &self,
        gpg_home: &Path,
        args: &[&str],
        passphrase: Option<&str>,
    ) -> Result<String, String> {
        let gpg_bin = self.resolve_gpg_binary()?;
        Self::run_gpg_command_with_binary(&gpg_bin, gpg_home, args, passphrase)
    }

    fn run_gpg_command_with_binary(
        gpg_bin: &str,
        gpg_home: &Path,
        args: &[&str],
        passphrase: Option<&str>,
    ) -> Result<String, String> {
        let mut cmd = Command::new(gpg_bin);
        cmd.arg("--homedir")
            .arg(gpg_home)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|err| format!("failed to execute {gpg_bin}: {err}"))?;
        let stdin_error = child.stdin.take().and_then(|mut stdin| {
            let written = passphrase
                .map_or(Ok(()), |value| stdin.write_all(value.as_bytes()))
                .and_then(|()| stdin.write_all(b"\n"));
            written.err()
        });

        let output: Output = child.wait_with_output().map_err(|err| err.to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                "gpg command failed".to_string()
            } else {
                stderr
            });
        }
        if let Some(error) = stdin_error {
            return Err(format!("failed to write gpg stdin: {error}"));
        }

        String::from_utf8(output.stdout).map_err(|err| err.to_string())
    }

    /// A throwaway `--homedir` so the user's real keyring is never touched.
    /// Its private agent and scdaemon are stopped before the directory goes
    /// away; otherwise every operation leaves both processes running.
    fn scratch_home(&self, prefix: &str) -> Result<GpgScratchHome, String> {
        GpgScratchHome::new(prefix, &self.resolve_gpg_binary()?)
    }

    fn run_gpg_agent_command(
        &self,
        gpg_home: &Path,
        operation: &str,
        args: &[String],
    ) -> Result<GpgOperationOutput, String> {
        let gpg_bin = self.resolve_gpg_binary()?;
        let status_path = gpg_home.join(format!("{operation}.status"));
        let status_path_arg = status_path.to_string_lossy().to_string();
        let mut command_args = vec![
            "--no-options".to_string(),
            "--batch".to_string(),
            "--yes".to_string(),
            "--no-tty".to_string(),
            "--no-auto-key-locate".to_string(),
            "--no-auto-key-import".to_string(),
            "--no-auto-key-retrieve".to_string(),
            "--disable-dirmngr".to_string(),
            "--pinentry-mode".to_string(),
            "ask".to_string(),
            "--status-file".to_string(),
            status_path_arg,
        ];
        command_args.extend(args.iter().cloned());

        let output = Command::new(&gpg_bin)
            .arg("--homedir")
            .arg(gpg_home)
            .args(&command_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|err| format!("failed to execute {gpg_bin}: {err}"))?;
        let status = std::fs::read_to_string(status_path).unwrap_or_default();
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if diagnostic.is_empty() {
                format!("gpg hardware-token {operation} failed")
            } else {
                diagnostic
            });
        }
        Ok(GpgOperationOutput {
            stdout: String::from_utf8(output.stdout).map_err(|err| err.to_string())?,
            status,
        })
    }

    fn private_key_material(&self, gpg_home: &Path) -> Result<GpgPrivateKeyMaterial, String> {
        let listing = self.run_gpg_command(
            gpg_home,
            &[
                "--no-options",
                "--batch",
                "--with-colons",
                "--with-subkey-fingerprint",
                "--list-secret-keys",
            ],
            None,
        )?;
        parse_gpg_private_key_material(&listing)
    }

    fn check_token_available(&self, gpg_home: &Path, token: &GpgTokenKey) -> Result<(), String> {
        let output =
            self.run_gpg_agent_command(gpg_home, "card-status", &["--card-status".to_string()])?;
        if !gpg_card_status_matches(&output.status, &token.serial) {
            return Err(
                "the hardware token referenced by this OpenPGP key is not available".to_string(),
            );
        }
        Ok(())
    }

    fn import_into(&self, gpg_home: &Path, name: &str, armored: &str) -> Result<(), String> {
        let path = gpg_home.join(name);
        std::fs::write(&path, armored).map_err(|err| err.to_string())?;
        self.run_gpg_command(
            gpg_home,
            &["--batch", "--import", path.to_string_lossy().as_ref()],
            None,
        )?;
        Ok(())
    }

    fn load_generated_fingerprint(&self, gpg_home: &Path, uid: &str) -> Result<String, String> {
        let output = self.run_gpg_command(
            gpg_home,
            &["--with-colons", "--list-secret-keys", uid],
            None,
        )?;
        self.extract_first_secret_fingerprint(&output)
    }

    fn extract_first_secret_fingerprint(&self, output: &str) -> Result<String, String> {
        let mut saw_secret_key = false;
        for line in output.lines() {
            let parts = line.split(':').collect::<Vec<_>>();
            if parts.is_empty() {
                continue;
            }
            if parts[0] == "sec" {
                saw_secret_key = true;
                continue;
            }
            if saw_secret_key && parts[0] == "fpr" {
                let fpr = parts.get(9).copied().unwrap_or("").trim();
                if !fpr.is_empty() {
                    return Ok(compact_fingerprint(fpr));
                }
            }
        }
        Err("could not determine key fingerprint".to_string())
    }
}

impl PgpEngine for GpgCliEngine {
    fn id(&self) -> &'static str {
        "gpg"
    }

    fn ensure_available(&self) -> Result<(), String> {
        self.resolve_gpg_binary().map(|_| ())
    }

    fn verify_private_key_passphrase(
        &self,
        armored_key: &str,
        passphrase: &str,
    ) -> Result<bool, String> {
        self.ensure_available()?;
        let gpg_home = self.scratch_home("satchel-pgp-verify-")?;
        self.import_into(gpg_home.path(), "key.asc", armored_key)?;

        let material = self.private_key_material(gpg_home.path())?;
        if let GpgPrivateKeyMaterial::Token(token) = &material {
            if !passphrase.is_empty() {
                return Err(
                    "enter the hardware-token PIN through GnuPG Pinentry, not Satchel".to_string(),
                );
            }
            self.check_token_available(gpg_home.path(), token)?;
            return Ok(true);
        }
        let GpgPrivateKeyMaterial::Software {
            primary_fingerprint: fingerprint,
        } = material
        else {
            unreachable!("the token case returned above")
        };
        let sample_path = gpg_home.path().join("sample.txt");
        std::fs::write(&sample_path, "satchel-passphrase-check").map_err(|err| err.to_string())?;
        let signature_path = gpg_home.path().join("sample.sig");
        let signature_path_owned = signature_path.to_string_lossy().to_string();
        let sample_path_owned = sample_path.to_string_lossy().to_string();

        let sign_args = [
            "--batch",
            "--yes",
            "--pinentry-mode",
            "loopback",
            "--passphrase-fd",
            "0",
            "--local-user",
            fingerprint.as_str(),
            "--output",
            signature_path_owned.as_str(),
            "--sign",
            sample_path_owned.as_str(),
        ];

        match self.run_gpg_command(gpg_home.path(), &sign_args, Some(passphrase)) {
            Ok(_) => Ok(true),
            Err(err) => {
                let lower = err.to_ascii_lowercase();
                if lower.contains("bad passphrase")
                    || lower.contains("decryption failed")
                    || lower.contains("passphrase")
                {
                    Ok(false)
                } else {
                    Err(err)
                }
            }
        }
    }

    fn decrypt_with_private_key(
        &self,
        armored_key: &str,
        passphrase: &str,
        ciphertext: &str,
    ) -> Result<String, String> {
        self.ensure_available()?;
        let gpg_home = self.scratch_home("satchel-pgp-decrypt-")?;
        self.import_into(gpg_home.path(), "key.asc", armored_key)?;
        let material = self.private_key_material(gpg_home.path())?;

        let encrypted_path = gpg_home.path().join("message.asc");
        std::fs::write(&encrypted_path, ciphertext).map_err(|err| err.to_string())?;
        let encrypted_path_owned = encrypted_path.to_string_lossy().to_string();
        let max_output = MAX_DECRYPTED_BYTES.to_string();

        if let GpgPrivateKeyMaterial::Token(token) = material {
            if !passphrase.is_empty() {
                return Err(
                    "enter the hardware-token PIN through GnuPG Pinentry, not Satchel".to_string(),
                );
            }
            if token.decryption_fingerprints.is_empty() {
                return Err("the hardware token has no decryption key".to_string());
            }
            let mut args = vec!["--max-output".to_string(), max_output];
            for fingerprint in &token.decryption_fingerprints {
                args.push("--try-secret-key".to_string());
                args.push(format!("{fingerprint}!"));
            }
            args.push("--decrypt".to_string());
            args.push(encrypted_path_owned);
            let output = self.run_gpg_agent_command(gpg_home.path(), "decrypt", &args)?;
            if !gpg_decryption_status_matches(&output.status, &token) {
                return Err("gpg did not decrypt with the expected hardware-token key".to_string());
            }
            if output.stdout.len() > MAX_DECRYPTED_BYTES {
                return Err(format!(
                    "decrypted message exceeds the limit of {MAX_DECRYPTED_BYTES} bytes"
                ));
            }
            return Ok(output.stdout);
        }

        let decrypt_args = [
            "--batch",
            "--yes",
            "--max-output",
            max_output.as_str(),
            "--pinentry-mode",
            "loopback",
            "--passphrase-fd",
            "0",
            "--decrypt",
            encrypted_path_owned.as_str(),
        ];
        let plaintext = self.run_gpg_command(gpg_home.path(), &decrypt_args, Some(passphrase))?;
        if plaintext.len() > MAX_DECRYPTED_BYTES {
            return Err(format!(
                "decrypted message exceeds the limit of {MAX_DECRYPTED_BYTES} bytes"
            ));
        }
        Ok(plaintext)
    }

    fn sign_detached(
        &self,
        armored_private_key: &str,
        passphrase: &str,
        payload: &[u8],
    ) -> Result<String, String> {
        self.ensure_available()?;
        let gpg_home = self.scratch_home("satchel-pgp-sign-")?;
        self.import_into(gpg_home.path(), "key.asc", armored_private_key)?;
        let material = self.private_key_material(gpg_home.path())?;

        let payload_path = gpg_home.path().join("payload.bin");
        std::fs::write(&payload_path, payload).map_err(|err| err.to_string())?;
        let payload_arg = payload_path.to_string_lossy().to_string();

        if let GpgPrivateKeyMaterial::Token(token) = material {
            if !passphrase.is_empty() {
                return Err(
                    "enter the hardware-token PIN through GnuPG Pinentry, not Satchel".to_string(),
                );
            }
            let signing_fingerprint = token
                .signing_fingerprints
                .first()
                .ok_or_else(|| "the hardware token has no signing key".to_string())?;
            let output = self.run_gpg_agent_command(
                gpg_home.path(),
                "sign",
                &[
                    "--armor".to_string(),
                    "--local-user".to_string(),
                    format!("{signing_fingerprint}!"),
                    "--digest-algo".to_string(),
                    "SHA256".to_string(),
                    "--detach-sign".to_string(),
                    "--output".to_string(),
                    "-".to_string(),
                    payload_arg,
                ],
            )?;
            if !gpg_signature_status_matches(&output.status, signing_fingerprint) {
                return Err("gpg did not sign with the expected hardware-token key".to_string());
            }
            return Ok(output.stdout);
        }

        self.run_gpg_command(
            gpg_home.path(),
            &[
                "--batch",
                "--yes",
                "--armor",
                "--pinentry-mode",
                "loopback",
                "--passphrase-fd",
                "0",
                // SHA-256 or better: SHA-1 signatures are rejected by modern
                // verifiers.
                "--digest-algo",
                "SHA256",
                "--detach-sign",
                "--output",
                "-",
                payload_arg.as_str(),
            ],
            Some(passphrase),
        )
    }

    fn verify_cleartext(
        &self,
        armored_certificate: &str,
        signed_message: &str,
    ) -> Result<CleartextVerification, String> {
        // Signature verification needs no token access. Keep it in-process so
        // both engine choices enforce the same certificate and binding checks.
        verify_cleartext_with_certificate(armored_certificate, signed_message)
    }

    fn encrypt_to(&self, armored_public_keys: &[String], payload: &[u8]) -> Result<String, String> {
        if armored_public_keys.is_empty() {
            return Err("no recipient public keys supplied".to_string());
        }
        for key in armored_public_keys {
            RpgpEngine::parse_one_certificate(key)?;
        }
        self.ensure_available()?;

        let gpg_home = self.scratch_home("satchel-pgp-encrypt-")?;

        // Import every recipient key and collect the fingerprints actually
        // imported, so we encrypt to exactly those and never silently to a
        // key that happens to sit in the temporary keyring.
        let mut recipients: Vec<String> = Vec::new();
        for (index, key) in armored_public_keys.iter().enumerate() {
            let key_path = gpg_home.path().join(format!("recipient-{index}.asc"));
            std::fs::write(&key_path, key).map_err(|err| err.to_string())?;
            self.run_gpg_command(
                gpg_home.path(),
                &["--batch", "--import", key_path.to_string_lossy().as_ref()],
                None,
            )?;

            // Ask gpg for the fingerprints rather than parsing OpenPGP packets
            // ourselves, and encrypt to exactly those — never to whatever else
            // happens to be in the throwaway keyring.
            let listing = self.run_gpg_command(
                gpg_home.path(),
                &[
                    "--batch",
                    "--with-colons",
                    "--show-keys",
                    key_path.to_string_lossy().as_ref(),
                ],
                None,
            )?;
            let fingerprints = primary_fingerprints(&listing);
            if fingerprints.len() != 1 {
                return Err(format!(
                    "expected exactly one OpenPGP certificate, found {}",
                    fingerprints.len()
                ));
            }
            recipients.push(fingerprints[0].clone());
        }

        if recipients.is_empty() {
            return Err("recipient keys contained no usable OpenPGP key".to_string());
        }

        let payload_path = gpg_home.path().join("payload.bin");
        std::fs::write(&payload_path, payload).map_err(|err| err.to_string())?;
        let payload_arg = payload_path.to_string_lossy().to_string();

        let mut args: Vec<String> = vec![
            "--batch".into(),
            "--yes".into(),
            "--armor".into(),
            // The recipient keys are unsigned in this throwaway keyring; without
            // this gpg refuses to encrypt to them.
            "--trust-model".into(),
            "always".into(),
        ];
        for recipient in &recipients {
            args.push("--recipient".into());
            args.push(recipient.clone());
        }
        args.push("--encrypt".into());
        args.push("--output".into());
        args.push("-".into());
        args.push(payload_arg);

        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run_gpg_command(gpg_home.path(), &arg_refs, None)
    }

    fn key_metadata(&self, armored_key: &str) -> Result<PgpKeyMetadata, String> {
        let has_secret_key = armored_key
            .to_ascii_uppercase()
            .contains("PRIVATE KEY BLOCK");
        if !has_secret_key {
            RpgpEngine::parse_one_certificate(armored_key)?;
        }
        self.ensure_available()?;
        let gpg_home = self.scratch_home("satchel-pgp-meta-")?;
        let listing = if has_secret_key {
            self.import_into(gpg_home.path(), "key.asc", armored_key)?;
            let listing = self.run_gpg_command(
                gpg_home.path(),
                &[
                    "--no-options",
                    "--batch",
                    "--with-colons",
                    "--with-subkey-fingerprint",
                    "--list-secret-keys",
                ],
                None,
            )?;
            parse_gpg_private_key_material(&listing)?;
            listing
        } else {
            let key_path = gpg_home.path().join("key.asc");
            std::fs::write(&key_path, armored_key).map_err(|err| err.to_string())?;
            self.run_gpg_command(
                gpg_home.path(),
                &[
                    "--no-options",
                    "--batch",
                    "--with-colons",
                    "--show-keys",
                    key_path.to_string_lossy().as_ref(),
                ],
                None,
            )?
        };

        parse_colon_metadata(&listing, has_secret_key)
    }

    fn generate_keypair(
        &self,
        request: &GenerateKeyRequest<'_>,
    ) -> Result<GeneratedKeyPair, String> {
        self.ensure_available()?;
        let gpg_home = self.scratch_home("satchel-pgp-")?;

        let keygen_args = [
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase-fd",
            "0",
            "--quick-gen-key",
            request.uid,
            request.algorithm,
            "default",
            request.expires_in,
        ];
        self.run_gpg_command(gpg_home.path(), &keygen_args, request.passphrase)?;

        let fingerprint = self.load_generated_fingerprint(gpg_home.path(), request.uid)?;
        let secret_export_args = [
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase-fd",
            "0",
            "--armor",
            "--export-secret-keys",
            fingerprint.as_str(),
        ];
        let private_armored =
            self.run_gpg_command(gpg_home.path(), &secret_export_args, request.passphrase)?;

        let public_export_args = ["--batch", "--armor", "--export", fingerprint.as_str()];
        let public_armored = self.run_gpg_command(gpg_home.path(), &public_export_args, None)?;

        Ok(GeneratedKeyPair {
            fingerprint,
            private_armored,
            public_armored,
        })
    }
}

/// Read `gpg --with-colons --show-keys` into the same shape the in-process
/// engine reports, so callers cannot tell the engines apart.
fn parse_colon_metadata(listing: &str, has_secret_key: bool) -> Result<PgpKeyMetadata, String> {
    let mut fingerprint = None;
    let mut user_ids = Vec::new();
    let mut algorithm = String::new();
    let mut created_at = None;
    let mut expires_at = None;
    let mut can_sign = false;
    let mut can_encrypt = false;
    let mut in_primary = false;
    let mut saw_secret_key = false;
    let mut primary_unusable = false;

    for line in listing.lines() {
        let fields = line.split(':').collect::<Vec<_>>();
        match fields.first().copied() {
            Some("pub" | "sec") => {
                saw_secret_key |= fields.first() == Some(&"sec");
                in_primary = true;
                primary_unusable = colon_record_is_unusable(&fields);
                created_at = fields.get(5).and_then(|value| unix_time(value));
                expires_at = fields.get(6).and_then(|value| unix_time(value));
                algorithm = colon_algorithm_name(&fields);
                let caps = fields.get(11).unwrap_or(&"");
                can_sign |= !primary_unusable && caps.contains('s');
                can_encrypt |= !primary_unusable && caps.contains('e');
            }
            Some("sub" | "ssb") => {
                saw_secret_key |= fields.first() == Some(&"ssb");
                in_primary = false;
                let caps = fields.get(11).unwrap_or(&"");
                can_sign |=
                    !primary_unusable && !colon_record_is_unusable(&fields) && caps.contains('s');
                can_encrypt |=
                    !primary_unusable && !colon_record_is_unusable(&fields) && caps.contains('e');
            }
            Some("fpr") if in_primary && fingerprint.is_none() => {
                fingerprint = fields.get(9).map(|value| compact_fingerprint(value));
            }
            Some("uid") => {
                if let Some(uid) = fields.get(9) {
                    if !primary_unusable && !colon_record_is_unusable(&fields) && !uid.is_empty() {
                        user_ids.push(uid.replace("\\x3a", ":"));
                    }
                }
            }
            _ => {}
        }
    }

    Ok(PgpKeyMetadata {
        fingerprint: fingerprint.ok_or_else(|| "key block contained no OpenPGP key".to_string())?,
        user_ids,
        algorithm,
        created_at: created_at.map(rfc3339),
        expires_at: expires_at.map(rfc3339),
        is_expired: primary_unusable
            || expires_at
                .map(|at| at <= SystemTime::now())
                .unwrap_or(false),
        has_secret_key: has_secret_key || saw_secret_key,
        can_sign,
        can_encrypt,
    })
}

fn colon_record_is_unusable(fields: &[&str]) -> bool {
    matches!(fields.get(1).copied(), Some("r" | "e" | "d" | "i"))
}

fn colon_algorithm_name(fields: &[&str]) -> String {
    let algorithm = fields.get(3).copied().unwrap_or_default();
    let bits = fields.get(2).copied().unwrap_or_default();
    let curve = fields.get(16).copied().unwrap_or_default();

    match algorithm {
        "1" | "2" | "3" if !bits.is_empty() => format!("rsa{bits}"),
        "16" | "20" => "elgamal".to_string(),
        "17" => "dsa".to_string(),
        "18" if curve.eq_ignore_ascii_case("curve25519") => "cv25519".to_string(),
        "18" if !curve.is_empty() => curve.to_ascii_lowercase(),
        "19" if !curve.is_empty() => curve.to_ascii_lowercase(),
        "22" if curve.eq_ignore_ascii_case("ed25519") => "ed25519".to_string(),
        "22" if !curve.is_empty() => curve.to_ascii_lowercase(),
        _ if !curve.is_empty() => curve.to_ascii_lowercase(),
        _ => algorithm.to_string(),
    }
}

fn unix_time(value: &str) -> Option<SystemTime> {
    let seconds = value.trim().parse::<u64>().ok()?;
    if seconds == 0 {
        return None;
    }
    Some(UNIX_EPOCH + StdDuration::from_secs(seconds))
}

fn compact_fingerprint(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(|ch| ch.to_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSPHRASE: &str = "unit-test-passphrase";
    const UID: &str = "Satchel Unit <unit@example.test>";

    fn generate(algorithm: &str, expires_in: &str) -> GeneratedKeyPair {
        RpgpEngine
            .generate_keypair(&GenerateKeyRequest {
                uid: UID,
                algorithm,
                expires_in,
                passphrase: Some(PASSPHRASE),
            })
            .expect("key generation should succeed")
    }

    fn armored_pair(key: SignedSecretKey) -> GeneratedKeyPair {
        GeneratedKeyPair {
            fingerprint: hex_fingerprint(&key.fingerprint()),
            private_armored: key
                .to_armored_string(ArmorOptions::default())
                .expect("private armor"),
            public_armored: key
                .to_public_key()
                .to_armored_string(ArmorOptions::default())
                .expect("public armor"),
        }
    }

    fn armor_public_certificates(pairs: &[&GeneratedKeyPair]) -> String {
        let certificates = pairs
            .iter()
            .map(|pair| RpgpEngine::parse_one_certificate(&pair.public_armored))
            .collect::<Result<Vec<_>, _>>()
            .expect("certificates");
        let mut armored = Vec::new();
        pgp::armor::write(
            &certificates,
            pgp::armor::BlockType::PublicKey,
            &mut armored,
            None,
            true,
        )
        .expect("armor certificate list");
        String::from_utf8(armored).expect("UTF-8 armor")
    }

    fn cleartext_sign(pair: &GeneratedKeyPair, text: &str) -> String {
        let key = RpgpEngine::parse_secret_key(&pair.private_armored).expect("private key");
        CleartextSignedMessage::sign(
            rand::thread_rng(),
            text,
            &key.primary_key,
            &Password::from(PASSPHRASE),
        )
        .expect("cleartext sign")
        .to_armored_string(ArmorOptions::default())
        .expect("cleartext armor")
    }

    fn new_packet(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut packet = vec![0xc0 | tag];
        match body.len() {
            len @ 0..=191 => packet.push(len as u8),
            len @ 192..=8383 => {
                let adjusted = len - 192;
                packet.push(((adjusted >> 8) + 192) as u8);
                packet.push(adjusted as u8);
            }
            len => {
                packet.push(255);
                packet.extend_from_slice(&(len as u32).to_be_bytes());
            }
        }
        packet.extend_from_slice(body);
        packet
    }

    fn wrap_uncompressed(inner: &[u8]) -> Vec<u8> {
        let mut body = Vec::with_capacity(inner.len() + 1);
        body.push(0); // CompressionAlgorithm::Uncompressed
        body.extend_from_slice(inner);
        new_packet(8, &body) // Compressed Data packet
    }

    fn signature_verifies(public_key: &str, payload: &[u8], armored_signature: &str) -> bool {
        let cert = RpgpEngine::parse_certificates(public_key)
            .expect("public key")
            .remove(0);
        let (signature, _) =
            DetachedSignature::from_string(armored_signature).expect("detached signature");

        if primary_can_sign(&cert) && signature.verify(&cert.primary_key, payload).is_ok() {
            return true;
        }
        cert.public_subkeys.iter().any(|subkey| {
            active_subkey_binding(&cert.primary_key, &subkey.key, &subkey.signatures).is_some_and(
                |binding| {
                    binding.key_flags().sign()
                        && valid_back_signature(binding, &subkey.key, &cert.primary_key)
                        && signature.verify(&subkey.key, payload).is_ok()
                },
            )
        })
    }

    #[test]
    fn the_default_key_matches_what_gpg_generates_by_default() {
        let pair = generate("default", "0");
        let metadata = RpgpEngine
            .key_metadata(&pair.public_armored)
            .expect("metadata");

        // GnuPG 2.3+ generates ed25519 + cv25519 for `default`.
        assert_eq!(metadata.algorithm, "ed25519");
        assert!(metadata.can_sign, "the primary must be able to sign");
        assert!(metadata.can_encrypt, "there must be an encryption subkey");
        assert_eq!(metadata.user_ids, vec![UID.to_string()]);
        assert_eq!(metadata.fingerprint.len(), 40, "v4 fingerprints are 40 hex");
    }

    #[test]
    fn generated_public_and_private_keys_describe_the_same_certificate() {
        let pair = generate("default", "0");
        assert!(pair
            .private_armored
            .starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----"));
        assert!(pair
            .public_armored
            .starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
        // Armor bodies are wrapped at 64 characters, like GnuPG's.
        assert!(pair
            .public_armored
            .lines()
            .all(|line| line.len() <= 64 || line.starts_with("-----")));

        let from_secret = RpgpEngine
            .key_metadata(&pair.private_armored)
            .expect("meta");
        let from_public = RpgpEngine.key_metadata(&pair.public_armored).expect("meta");
        assert_eq!(from_secret.fingerprint, pair.fingerprint);
        assert_eq!(from_secret.fingerprint, from_public.fingerprint);
        assert!(from_secret.has_secret_key);
        assert!(!from_public.has_secret_key);

        let secret = RpgpEngine::parse_secret_key(&pair.private_armored).expect("secret key");
        assert!(matches!(
            secret.primary_key.secret_params(),
            SecretParams::Encrypted(_)
        ));
        assert!(secret
            .secret_subkeys
            .iter()
            .all(|subkey| matches!(subkey.key.secret_params(), SecretParams::Encrypted(_))));
    }

    #[test]
    fn an_rsa_key_reports_its_modulus_size() {
        let pair = generate("rsa2048", "0");
        let metadata = RpgpEngine.key_metadata(&pair.public_armored).expect("meta");
        assert_eq!(metadata.algorithm, "rsa2048");
    }

    #[test]
    fn signing_and_verifying_round_trips() {
        let pair = generate("default", "0");
        let payload = b"Content-Type: text/plain\r\n\r\nHallo Welt\r\n";

        let signature = RpgpEngine
            .sign_detached(&pair.private_armored, PASSPHRASE, payload)
            .expect("sign");
        assert!(signature.starts_with("-----BEGIN PGP SIGNATURE-----"));

        assert!(signature_verifies(
            &pair.public_armored,
            payload,
            &signature
        ));
    }

    #[test]
    fn verification_rejects_tampered_content() {
        let pair = generate("default", "0");
        let signature = RpgpEngine
            .sign_detached(&pair.private_armored, PASSPHRASE, b"original text")
            .expect("sign");

        assert!(
            !signature_verifies(&pair.public_armored, b"tampered text", &signature),
            "a changed payload must not verify"
        );
    }

    #[test]
    fn verification_rejects_a_signature_from_another_key() {
        let signer = generate("default", "0");
        let stranger = generate("default", "0");
        let signature = RpgpEngine
            .sign_detached(&signer.private_armored, PASSPHRASE, b"payload")
            .expect("sign");

        assert!(!signature_verifies(
            &stranger.public_armored,
            b"payload",
            &signature
        ));
    }

    #[test]
    fn cleartext_verification_distinguishes_valid_tampered_and_unknown_signers() {
        let signer = generate("default", "0");
        let stranger = generate("default", "0");
        let signed = cleartext_sign(&signer, "Hello\n- dash escaped\n");

        for engine in [
            &RpgpEngine as &dyn PgpEngine,
            &GpgCliEngine as &dyn PgpEngine,
        ] {
            let valid = engine
                .verify_cleartext(&signer.public_armored, &signed)
                .expect("verification");
            let CleartextVerification::Valid(valid) = valid else {
                panic!("the signer certificate must verify")
            };
            assert_eq!(valid.signer_fingerprint, signer.fingerprint);
            assert_eq!(
                valid.cleartext.replace("\r\n", "\n"),
                "Hello\n- dash escaped\n"
            );

            let tampered = signed.replace("Hello", "Jello");
            assert!(matches!(
                engine
                    .verify_cleartext(&signer.public_armored, &tampered)
                    .expect("tampered result"),
                CleartextVerification::Invalid { cleartext }
                    if cleartext.starts_with("Jello")
            ));
            assert!(matches!(
                engine
                    .verify_cleartext(&stranger.public_armored, &signed)
                    .expect("unknown signer result"),
                CleartextVerification::NotSignedByCertificate { cleartext }
                    if cleartext.starts_with("Hello")
            ));
        }
    }

    #[test]
    fn detached_verification_distinguishes_valid_tampered_and_unknown_signers() {
        let signer = generate("default", "0");
        let stranger = generate("default", "0");
        let payload = b"Content-Type: text/plain\r\n\r\nSigned body\r\n";
        let signature = RpgpEngine
            .sign_detached(&signer.private_armored, PASSPHRASE, payload)
            .expect("detached signature");

        assert!(matches!(
            verify_detached(&signer.public_armored, payload, &signature).expect("verification"),
            DetachedVerification::Valid { signer_fingerprint }
                if signer_fingerprint == signer.fingerprint
        ));
        assert_eq!(
            verify_detached(&signer.public_armored, b"tampered", &signature)
                .expect("tampered result"),
            DetachedVerification::Invalid
        );
        assert_eq!(
            verify_detached(&stranger.public_armored, payload, &signature)
                .expect("unknown signer result"),
            DetachedVerification::NotSignedByCertificate
        );
    }

    #[test]
    fn detached_verification_rejects_expired_and_future_signatures() {
        use pgp::{composed::SubpacketConfig, types::Timestamp};

        let signer = generate("default", "0");
        let key = RpgpEngine::parse_secret_key(&signer.private_armored).expect("private key");
        let payload = b"signed payload";
        let now = u32::try_from(now_secs()).expect("current OpenPGP timestamp");

        for created_at in [now.saturating_sub(3600), now.saturating_add(3600)] {
            let mut hashed = vec![
                Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint()))
                    .expect("issuer fingerprint"),
                Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::from_secs(
                    created_at,
                )))
                .expect("creation time"),
            ];
            if created_at < now {
                hashed.push(
                    Subpacket::regular(SubpacketData::SignatureExpirationTime(
                        PgpDuration::from_secs(1),
                    ))
                    .expect("signature expiry"),
                );
            }
            let signature = DetachedSignature::sign_binary_data_with_subpackets(
                rand::thread_rng(),
                &key.primary_key,
                &Password::from(PASSPHRASE),
                HashAlgorithm::Sha256,
                payload.as_slice(),
                SubpacketConfig::UserDefined {
                    hashed,
                    unhashed: vec![],
                },
            )
            .expect("sign")
            .to_armored_string(ArmorOptions::default())
            .expect("armor");

            assert_eq!(
                verify_detached(&signer.public_armored, payload, &signature)
                    .expect("verification result"),
                DetachedVerification::Invalid
            );
        }
    }

    #[test]
    fn revoked_user_id_is_not_recipient_addressable() {
        let pair = generate("default", "0");
        let mut key = RpgpEngine::parse_secret_key(&pair.private_armored).expect("private key");
        let mut rng = rand::thread_rng();
        let config =
            SignatureConfig::from_key(&mut rng, &key.primary_key, SignatureType::CertRevocation)
                .expect("revocation config");
        let revocation = config
            .sign_certification(
                &key.primary_key,
                key.primary_key.public_key(),
                &Password::from(PASSPHRASE),
                Tag::UserId,
                &key.details.users[0].id,
            )
            .expect("user id revocation");
        key.details.users[0].signatures.push(revocation);
        let armored = key
            .to_public_key()
            .to_armored_string(ArmorOptions::default())
            .expect("armor");

        let metadata = RpgpEngine.key_metadata(&armored).expect("metadata");
        assert!(metadata.user_ids.is_empty());
        assert!(metadata.is_expired);
        assert!(!metadata.can_encrypt);
    }

    #[test]
    fn cleartext_can_be_read_without_an_installed_signer_certificate() {
        let signer = generate("default", "0");
        let signed = cleartext_sign(&signer, "Readable without trust\n");

        assert_eq!(
            parse_cleartext_signed_message(&signed)
                .expect("the cleartext wrapper should be readable")
                .replace("\r\n", "\n"),
            "Readable without trust\n"
        );
        assert!(parse_cleartext_signed_message("not a signed message").is_err());
    }

    #[test]
    fn encrypting_to_two_recipients_lets_each_one_decrypt() {
        let alice = generate("default", "0");
        let bob = generate("default", "0");
        let plaintext = "Streng geheim: Grüße mit Umlauten\n";

        let ciphertext = RpgpEngine
            .encrypt_to(
                &[alice.public_armored.clone(), bob.public_armored.clone()],
                plaintext.as_bytes(),
            )
            .expect("encrypt");
        assert!(ciphertext.starts_with("-----BEGIN PGP MESSAGE-----"));
        assert!(!ciphertext.contains("Streng geheim"));

        for (who, key) in [("alice", &alice), ("bob", &bob)] {
            let decrypted = RpgpEngine
                .decrypt_with_private_key(&key.private_armored, PASSPHRASE, &ciphertext)
                .unwrap_or_else(|err| panic!("{who} should decrypt: {err}"));
            assert_eq!(decrypted, plaintext, "{who} got the wrong plaintext");
        }
    }

    #[test]
    fn encryption_fails_if_any_recipient_has_no_encryption_component() {
        let recipient = generate("default", "0");
        let signing_only = generate("default", "0");
        let mut signing_only_cert = RpgpEngine::parse_certificates(&signing_only.public_armored)
            .expect("certificate")
            .remove(0);
        signing_only_cert.public_subkeys.clear();
        let signing_only_armored = signing_only_cert
            .to_armored_string(ArmorOptions::default())
            .expect("armor signing-only certificate");

        let result = RpgpEngine.encrypt_to(
            &[recipient.public_armored, signing_only_armored],
            b"must reach everyone",
        );
        assert!(
            result.is_err(),
            "one valid recipient must not hide an unusable second recipient"
        );
    }

    #[test]
    fn multi_certificate_armor_must_be_split_before_encryption_or_metadata_lookup() {
        let alice = generate("default", "0");
        let bob = generate("default", "0");
        let keyring = armor_public_certificates(&[&alice, &bob]);

        assert!(RpgpEngine.key_metadata(&keyring).is_err());
        assert!(RpgpEngine
            .encrypt_to(std::slice::from_ref(&keyring), b"for alice only")
            .is_err());
        assert!(GpgCliEngine.key_metadata(&keyring).is_err());
        assert!(GpgCliEngine
            .encrypt_to(std::slice::from_ref(&keyring), b"for alice only")
            .is_err());

        let split = split_armored_keys(&keyring).expect("split keyring");
        assert_eq!(split.len(), 2);
        let ciphertext = RpgpEngine
            .encrypt_to(std::slice::from_ref(&split[0]), b"for alice only")
            .expect("encrypt to one split certificate");
        assert_eq!(
            RpgpEngine
                .decrypt_with_private_key(&alice.private_armored, PASSPHRASE, &ciphertext)
                .expect("alice decrypts"),
            "for alice only"
        );
        assert!(
            RpgpEngine
                .decrypt_with_private_key(&bob.private_armored, PASSPHRASE, &ciphertext)
                .is_err(),
            "a second certificate from the original keyring must not become a recipient"
        );
    }

    #[test]
    fn private_keyrings_split_into_individual_private_keys() {
        let alice = generate("default", "0");
        let bob = generate("default", "0");
        let keys = [&alice, &bob]
            .into_iter()
            .map(|pair| RpgpEngine::parse_secret_key(&pair.private_armored))
            .collect::<Result<Vec<_>, _>>()
            .expect("secret keys");
        let mut armored = Vec::new();
        pgp::armor::write(
            &keys,
            pgp::armor::BlockType::PrivateKey,
            &mut armored,
            None,
            true,
        )
        .expect("armor private keyring");

        let split = split_armored_keys(&String::from_utf8(armored).expect("UTF-8 armor"))
            .expect("split private keyring");
        assert_eq!(split.len(), 2);
        assert!(split
            .iter()
            .all(|key| key.starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----")));
    }

    #[test]
    fn encryption_uses_a_symmetric_algorithm_accepted_by_every_recipient() {
        let aes128_only = armored_pair(
            RpgpEngine::build_keypair_with_symmetric_preferences(
                &GenerateKeyRequest {
                    uid: UID,
                    algorithm: "default",
                    expires_in: "0",
                    passphrase: Some(PASSPHRASE),
                },
                SystemTime::now(),
                &[SymmetricKeyAlgorithm::AES128],
            )
            .expect("AES128-only key"),
        );
        let cert =
            RpgpEngine::parse_one_certificate(&aes128_only.public_armored).expect("certificate");
        assert_eq!(
            select_symmetric_algorithm(std::slice::from_ref(&cert)).expect("compatible cipher"),
            SymmetricKeyAlgorithm::AES128
        );

        let ciphertext = RpgpEngine
            .encrypt_to(
                std::slice::from_ref(&aes128_only.public_armored),
                b"AES128 compatibility",
            )
            .expect("encrypt");
        assert_eq!(
            RpgpEngine
                .decrypt_with_private_key(&aes128_only.private_armored, PASSPHRASE, &ciphertext,)
                .expect("decrypt"),
            "AES128 compatibility"
        );
    }

    #[test]
    fn decompression_depth_and_plaintext_size_are_bounded() {
        let mut nested = MessageBuilder::from_bytes("", b"small".to_vec())
            .to_vec(rand::thread_rng())
            .expect("literal message");
        for _ in 0..=MAX_COMPRESSION_DEPTH {
            nested = wrap_uncompressed(&nested);
        }
        let nested = Message::from_reader(nested.as_slice())
            .expect("nested message")
            .0;
        assert!(read_decrypted_message(nested, MAX_COMPRESSION_DEPTH, 1024)
            .expect_err("depth limit")
            .contains("nesting"));

        let mut compressed = MessageBuilder::from_bytes("", vec![0_u8; 1024]);
        compressed.compression(pgp::types::CompressionAlgorithm::ZLIB);
        let compressed = compressed
            .to_vec(rand::thread_rng())
            .expect("compressed message");
        let compressed = Message::from_reader(compressed.as_slice())
            .expect("parse compressed message")
            .0;
        assert!(
            read_decrypted_message(compressed, MAX_COMPRESSION_DEPTH, 32)
                .expect_err("plaintext limit")
                .contains("32 bytes")
        );
    }

    #[test]
    fn a_multi_key_secret_block_can_decrypt_for_its_second_key() {
        let alice = generate("default", "0");
        let bob = generate("default", "0");
        let keys = vec![
            RpgpEngine::parse_secret_key(&alice.private_armored).expect("alice secret"),
            RpgpEngine::parse_secret_key(&bob.private_armored).expect("bob secret"),
        ];
        let mut armored = Vec::new();
        pgp::armor::write(
            &keys,
            pgp::armor::BlockType::PrivateKey,
            &mut armored,
            None,
            true,
        )
        .expect("armor multi-key block");
        let armored = String::from_utf8(armored).expect("UTF-8 armor");

        let ciphertext = RpgpEngine
            .encrypt_to(std::slice::from_ref(&bob.public_armored), b"for bob")
            .expect("encrypt for bob");
        let plaintext = RpgpEngine
            .decrypt_with_private_key(&armored, PASSPHRASE, &ciphertext)
            .expect("second key decrypts");
        assert_eq!(plaintext, "for bob");
    }

    #[test]
    fn an_rsa_keypair_encrypts_and_decrypts() {
        let pair = generate("rsa2048", "0");
        let ciphertext = RpgpEngine
            .encrypt_to(std::slice::from_ref(&pair.public_armored), b"geheim")
            .expect("encrypt");
        let decrypted = RpgpEngine
            .decrypt_with_private_key(&pair.private_armored, PASSPHRASE, &ciphertext)
            .expect("decrypt");
        assert_eq!(decrypted, "geheim");
    }

    #[test]
    fn the_wrong_passphrase_is_refused() {
        let pair = generate("default", "0");

        assert!(RpgpEngine
            .verify_private_key_passphrase(&pair.private_armored, PASSPHRASE)
            .expect("check"));
        assert!(!RpgpEngine
            .verify_private_key_passphrase(&pair.private_armored, "not the passphrase")
            .expect("check"));

        assert!(
            RpgpEngine
                .sign_detached(&pair.private_armored, "not the passphrase", b"x")
                .is_err(),
            "signing must not work without the passphrase"
        );

        let ciphertext = RpgpEngine
            .encrypt_to(std::slice::from_ref(&pair.public_armored), b"geheim")
            .expect("encrypt");
        assert!(
            RpgpEngine
                .decrypt_with_private_key(&pair.private_armored, "not the passphrase", &ciphertext)
                .is_err(),
            "decryption must not work without the passphrase"
        );
    }

    #[test]
    fn modified_ciphertext_fails_integrity_check() {
        let pair = generate("default", "0");
        let mut ciphertext = RpgpEngine
            .encrypt_to(std::slice::from_ref(&pair.public_armored), b"authentic")
            .expect("encrypt")
            .into_bytes();
        let body_start = ciphertext
            .windows(2)
            .position(|window| window == b"\n\n")
            .expect("armor body")
            + 2;
        let index = ciphertext[body_start..]
            .iter()
            .position(|byte| byte.is_ascii_alphanumeric())
            .map(|offset| body_start + offset)
            .expect("armored body byte");
        ciphertext[index] = if ciphertext[index] == b'A' {
            b'B'
        } else {
            b'A'
        };
        let ciphertext = String::from_utf8(ciphertext).expect("armor remains UTF-8");

        assert!(RpgpEngine
            .decrypt_with_private_key(&pair.private_armored, PASSPHRASE, &ciphertext)
            .is_err());
    }

    #[test]
    fn expiry_is_written_into_the_key_and_read_back() {
        let pair = generate("default", "2y");
        let metadata = RpgpEngine.key_metadata(&pair.public_armored).expect("meta");

        let expires_at = metadata.expires_at.expect("an expiring key reports expiry");
        let created_at = metadata.created_at.expect("creation time");
        let expires = chrono::DateTime::parse_from_rfc3339(&expires_at).expect("rfc3339");
        let created = chrono::DateTime::parse_from_rfc3339(&created_at).expect("rfc3339");
        assert_eq!((expires - created).num_days(), 730);
        assert!(!metadata.is_expired);

        // The re-signed user id must still verify, or gpg would reject the key.
        let signature = RpgpEngine
            .sign_detached(&pair.private_armored, PASSPHRASE, b"payload")
            .expect("sign");
        assert!(signature_verifies(
            &pair.public_armored,
            b"payload",
            &signature
        ));
    }

    #[test]
    fn a_key_whose_expiry_has_passed_is_reported_as_expired() {
        // Backdated creation instead of a sleep: expiry is relative to the
        // key's creation time, so this is deterministic.
        let key = RpgpEngine::build_keypair(
            &GenerateKeyRequest {
                uid: UID,
                algorithm: "default",
                expires_in: "1d",
                passphrase: Some(PASSPHRASE),
            },
            SystemTime::now() - StdDuration::from_secs(30 * 86_400),
        )
        .expect("generate");
        let armored = key
            .to_public_key()
            .to_armored_string(ArmorOptions::default())
            .expect("armor");

        let metadata = RpgpEngine.key_metadata(&armored).expect("meta");
        assert!(metadata.is_expired, "a passed expiry must be reported");
        assert!(RpgpEngine
            .encrypt_to(std::slice::from_ref(&armored), b"too late")
            .is_err());
        let private_armored = key
            .to_armored_string(ArmorOptions::default())
            .expect("private armor");
        assert!(RpgpEngine
            .sign_detached(&private_armored, PASSPHRASE, b"too late")
            .is_err());
    }

    #[test]
    fn expiry_strings_follow_gpg_syntax() {
        assert_eq!(parse_expiry("0").expect("parse"), None);
        assert_eq!(parse_expiry("").expect("parse"), None);
        assert_eq!(parse_expiry("never").expect("parse"), None);
        assert_eq!(
            parse_expiry("2y").expect("parse"),
            Some(StdDuration::from_secs(730 * 86_400))
        );
        assert_eq!(
            parse_expiry("90").expect("parse"),
            Some(StdDuration::from_secs(90 * 86_400))
        );
        assert_eq!(
            parse_expiry("3w").expect("parse"),
            Some(StdDuration::from_secs(21 * 86_400))
        );
        assert!(parse_expiry("bald").is_err());
        assert!(parse_expiry("18446744073709551615y").is_err());
    }

    #[test]
    fn the_default_engine_is_the_in_process_one() {
        assert_eq!(engine_by_id("").id(), "rpgp");
        assert_eq!(engine_by_id("rust").id(), "rpgp");
        assert_eq!(engine_by_id("gpg").id(), "gpg");
        assert_eq!(engine_by_id("gpg-cli").id(), "gpg");
        // The in-process engine needs nothing installed.
        assert!(RpgpEngine.ensure_available().is_ok());
    }

    #[test]
    fn garbage_is_rejected_rather_than_silently_ignored() {
        assert!(RpgpEngine.key_metadata("not a key").is_err());
        assert!(RpgpEngine
            .verify_private_key_passphrase("not a key", PASSPHRASE)
            .is_err());
        assert!(RpgpEngine.encrypt_to(&[], b"x").is_err());
    }

    #[test]
    fn colon_metadata_is_read_like_the_in_process_engine_reports_it() {
        let listing = "\
tru::1:1700000000:0:3:1:5
pub:u:255:22:248078E5F78699AC:1700000000:1800000000::u:::scESC:::::ed25519:::0:
fpr:::::::::DBDFFD78DE863986839CB421248078E5F78699AC:
uid:u::::1700000000::E45BAA1A::Default Test <d@example.test>::::::::
sub:u:255:18:7CCC5F807BA984B2:1700000000::::::e:::::cv25519::
fpr:::::::::F81FD3B6FC54F33EA5393EA47CCC5F807BA984B2:
";
        let metadata = parse_colon_metadata(listing, false).expect("parse");
        assert_eq!(
            metadata.fingerprint,
            "DBDFFD78DE863986839CB421248078E5F78699AC"
        );
        assert_eq!(metadata.algorithm, "ed25519");
        assert_eq!(metadata.user_ids, vec!["Default Test <d@example.test>"]);
        assert!(metadata.can_sign);
        assert!(metadata.can_encrypt);
        assert!(metadata.expires_at.is_some());
    }

    #[test]
    fn colon_metadata_recognizes_secret_primary_keys_and_subkeys() {
        let listing = "\
sec:u:255:22:248078E5F78699AC:1700000000:1800000000::u:::scESC:::::ed25519:::0:
fpr:::::::::DBDFFD78DE863986839CB421248078E5F78699AC:
uid:u::::1700000000::E45BAA1A::Private Test <private@example.test>::::::::
ssb:u:255:18:7CCC5F807BA984B2:1700000000::::::e:::::cv25519::
fpr:::::::::F81FD3B6FC54F33EA5393EA47CCC5F807BA984B2:
";
        let metadata = parse_colon_metadata(listing, false).expect("parse secret listing");

        assert_eq!(
            metadata.fingerprint,
            "DBDFFD78DE863986839CB421248078E5F78699AC"
        );
        assert_eq!(metadata.algorithm, "ed25519");
        assert!(metadata.has_secret_key);
        assert!(metadata.can_sign);
        assert!(metadata.can_encrypt);
    }

    #[test]
    fn colon_metadata_excludes_revoked_identities_and_components() {
        let listing = "\
pub:r:255:22:248078E5F78699AC:1700000000::::::sc:::::ed25519:::0:
fpr:::::::::DBDFFD78DE863986839CB421248078E5F78699AC:
uid:r::::1700000000::E45BAA1A::Revoked <revoked@example.test>::::::::
sub:u:255:18:7CCC5F807BA984B2:1700000000::::::e:::::cv25519::
";
        let metadata = parse_colon_metadata(listing, false).expect("parse revoked listing");

        assert!(metadata.user_ids.is_empty());
        assert!(metadata.is_expired);
        assert!(!metadata.can_sign);
        assert!(!metadata.can_encrypt);
    }

    fn secret_record(kind: &str, capabilities: &str, token_field: &str) -> String {
        let mut fields = vec![""; 21];
        fields[0] = kind;
        fields[1] = "u";
        fields[11] = capabilities;
        fields[14] = token_field;
        fields.join(":")
    }

    #[test]
    fn token_colon_listing_binds_capabilities_to_exact_component_fingerprints() {
        const PRIMARY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        const SIGNING: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
        const DECRYPTION: &str = "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
        const SERIAL: &str = "D2760001240103040006160691240000";
        let listing = [
            secret_record("sec", "c", "#"),
            format!("fpr:::::::::{PRIMARY}:"),
            secret_record("ssb", "s", SERIAL),
            format!("fpr:::::::::{SIGNING}:"),
            secret_record("ssb", "e", SERIAL),
            format!("fpr:::::::::{DECRYPTION}:"),
        ]
        .join("\n");

        assert_eq!(
            parse_gpg_private_key_material(&listing).expect("token stub"),
            GpgPrivateKeyMaterial::Token(GpgTokenKey {
                primary_fingerprint: PRIMARY.to_string(),
                serial: SERIAL.to_string(),
                signing_fingerprints: vec![SIGNING.to_string()],
                decryption_fingerprints: vec![DECRYPTION.to_string()],
            })
        );
    }

    #[test]
    fn offline_and_mixed_secret_stubs_are_rejected() {
        const PRIMARY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        const SUBKEY: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
        const SERIAL: &str = "D2760001240103040006160691240000";
        let offline = [
            secret_record("sec", "s", "#"),
            format!("fpr:::::::::{PRIMARY}:"),
            secret_record("ssb", "e", "#"),
            format!("fpr:::::::::{SUBKEY}:"),
        ]
        .join("\n");
        assert!(parse_gpg_private_key_material(&offline)
            .expect_err("offline stub")
            .contains("offline"));

        let mixed = [
            secret_record("sec", "s", "+"),
            format!("fpr:::::::::{PRIMARY}:"),
            secret_record("ssb", "e", SERIAL),
            format!("fpr:::::::::{SUBKEY}:"),
        ]
        .join("\n");
        assert!(parse_gpg_private_key_material(&mixed)
            .expect_err("mixed secret material")
            .contains("mix"));
    }

    #[test]
    fn token_operation_status_must_name_the_expected_keys() {
        let token = GpgTokenKey {
            primary_fingerprint: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
            serial: "D2760001240103040006160691240000".to_string(),
            signing_fingerprints: vec!["BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string()],
            decryption_fingerprints: vec!["CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC".to_string()],
        };
        let sign_status =
            "[GNUPG:] SIG_CREATED D 22 10 00 1700000000 BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB\n";
        assert!(gpg_signature_status_matches(
            sign_status,
            &token.signing_fingerprints[0]
        ));
        assert!(!gpg_signature_status_matches(
            sign_status,
            "DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD"
        ));

        let decrypt_status = "\
[GNUPG:] DECRYPTION_KEY CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA u
[GNUPG:] DECRYPTION_OKAY
";
        assert!(gpg_decryption_status_matches(decrypt_status, &token));
        assert!(!gpg_decryption_status_matches(
            "[GNUPG:] DECRYPTION_KEY DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA u\n[GNUPG:] DECRYPTION_OKAY\n",
            &token
        ));
        assert!(gpg_card_status_matches(
            "[GNUPG:] CARDCTRL 3 D2760001240103040006160691240000\n",
            &token.serial
        ));
    }

    #[cfg(unix)]
    #[test]
    fn scratch_home_stops_its_agent_before_the_directory_is_deleted() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = tempfile::tempdir().expect("fixture directory");
        let fake_gpgconf = fixture.path().join("gpgconf");
        std::fs::write(&fake_gpgconf, "#!/bin/sh\n: > \"$0.called\"\n").expect("fake gpgconf");
        let mut permissions = std::fs::metadata(&fake_gpgconf)
            .expect("fake metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fake_gpgconf, permissions).expect("make fake executable");
        let marker = PathBuf::from(format!("{}.called", fake_gpgconf.display()));

        let scratch_path;
        {
            let scratch = GpgScratchHome::new(
                "satchel-pgp-test-",
                fake_gpgconf.to_str().expect("gpgconf path"),
            )
            .expect("scratch directory");
            scratch_path = scratch.path().to_path_buf();
            assert!(scratch_path.exists());
            assert_eq!(
                std::fs::metadata(&scratch_path)
                    .expect("scratch metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "GnuPG homes contain private keys and must be owner-only"
            );
        }

        assert!(marker.exists(), "Drop must ask gpgconf to stop the agent");
        assert!(
            !scratch_path.exists(),
            "the agent must be stopped before TempDir removes its home"
        );
    }

    #[cfg(unix)]
    #[test]
    fn gpg_early_stdin_close_reports_child_stderr() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = tempfile::tempdir().expect("fixture directory");
        let fake_gpg = fixture.path().join("gpg");
        std::fs::write(
            &fake_gpg,
            "#!/bin/sh\nsleep 0.05\nprintf 'gpg: deliberate early failure\\n' >&2\nexit 7\n",
        )
        .expect("fake gpg");
        std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o700))
            .expect("make fake executable");

        let passphrase = "x".repeat(2 * 1024 * 1024);
        let error = GpgCliEngine::run_gpg_command_with_binary(
            fake_gpg.to_str().expect("gpg path"),
            fixture.path(),
            &[],
            Some(&passphrase),
        )
        .expect_err("fake gpg exits before reading its passphrase");
        assert!(error.contains("gpg: deliberate early failure"), "{error}");
    }
}
