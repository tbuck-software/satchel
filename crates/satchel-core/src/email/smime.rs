//! S/MIME protection for outgoing mail. Decrypted payloads are decoded by
//! `email::decrypted`, which OpenPGP shares.

/// The outbox's sign-then-encrypt transform, independent of credential storage.
pub fn protect_message(
    bytes: &[u8],
    signing: Option<(&str, &str, Option<&str>)>,
    recipients: Option<&[String]>,
) -> Result<Vec<u8>, String> {
    let signed = match signing {
        Some((key, cert, passphrase)) => {
            crate::utils::smime_engine::sign(key, cert, passphrase, bytes)?
        }
        None => bytes.to_vec(),
    };
    match recipients {
        Some(certs) => crate::utils::smime_engine::encrypt(certs, &signed),
        None => Ok(signed),
    }
}
