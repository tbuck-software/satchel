//! Lowercase hex for digests that are stored or compared as text.

use std::fmt::Write as _;

/// Two lowercase hex digits per byte, no separator: the text `{:x}` produced
/// for sha2 0.10 digests. Persisted identifiers depend on this exact form.
pub fn lower_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::lower_hex;
    use sha2::{Digest, Sha256};

    #[test]
    fn sha256_of_abc_matches_the_fips_vector() {
        assert_eq!(
            lower_hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(lower_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }
}
