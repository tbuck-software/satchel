//! Certificate trust for S/MIME, separate from verification of signed bytes.
//! Message certificates are untrusted chain candidates, never trust anchors.

use openssl::{pkcs7::Pkcs7Ref, stack::Stack};

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(any(
    test,
    not(any(target_os = "macos", target_os = "ios", target_os = "windows"))
))]
pub(super) mod portable;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(target_os = "macos", target_os = "ios"))]
use apple as platform;
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "windows")))]
use portable as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// Call only after every PKCS#7 signature has passed mathematical verification.
/// Resolve actual signers rather than relying on the order of embedded certs.
pub(super) fn verify(pkcs7: &Pkcs7Ref) -> Result<(), String> {
    let empty = Stack::new().map_err(|e| e.to_string())?;
    let signers = pkcs7
        .signers(&empty, openssl::pkcs7::Pkcs7Flags::empty())
        .map_err(|e| e.to_string())?;
    if signers.is_empty() {
        return Err("S/MIME payload has no signer certificate".into());
    }
    let certificates = pkcs7
        .signed()
        .and_then(|signed| signed.certificates())
        .ok_or("S/MIME payload has no certificates")?
        .iter()
        .map(|cert| cert.to_der().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    for signer in signers {
        platform::verify(&signer.to_der().map_err(|e| e.to_string())?, &certificates)?;
    }
    Ok(())
}

#[cfg(all(test, any(target_os = "macos", target_os = "ios")))]
pub(super) fn verify_apple_with_anchors(
    leaf: &[u8],
    certificates: &[Vec<u8>],
    anchors: &[Vec<u8>],
) -> Result<(), String> {
    let mut trust = apple::create_trust(leaf, certificates)?;
    let anchors = anchors
        .iter()
        .map(|der| security_framework::certificate::SecCertificate::from_der(der))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    trust
        .set_anchor_certificates(&anchors)
        .map_err(|e| e.to_string())?;
    trust
        .set_trust_anchor_certificates_only(true)
        .map_err(|e| e.to_string())?;
    trust.evaluate_with_error().map_err(|e| e.to_string())
}
