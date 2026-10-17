#[path = "../../../crates/satchel-core/src/utils/smime_engine.rs"]
pub mod smime_engine;

#[cfg(test)]
mod tests {
    use super::smime_engine::{self, GenerateSmimeRequest};
    use std::process::Command;

    #[test]
    fn desktop_decrypt_accepts_rsa_oaep_sha256() {
        smime_engine::ensure_engine_available()
            .expect("openssl should be available for S/MIME tests");
        let generated = smime_engine::generate_keypair(&GenerateSmimeRequest {
            name: "Satchel OAEP Test",
            email: "smime-oaep@satchel.local",
            passphrase: None,
        })
        .expect("generate test certificate");
        let temp = tempfile::tempdir().expect("create temporary test directory");
        let cert_path = temp.path().join("recipient.pem");
        let input_path = temp.path().join("payload.txt");
        let encrypted_path = temp.path().join("encrypted.eml");
        let plaintext = b"Satchel OAEP regression payload";
        std::fs::write(&cert_path, &generated.pem_certificate).expect("write test certificate");
        std::fs::write(&input_path, plaintext).expect("write plaintext fixture");

        let output = Command::new("openssl")
            .args(["cms", "-encrypt", "-binary", "-aes-128-cbc", "-in"])
            .arg(&input_path)
            .arg("-out")
            .arg(&encrypted_path)
            .args(["-outform", "SMIME", "-recip"])
            .arg(&cert_path)
            .args([
                "-keyopt",
                "rsa_padding_mode:oaep",
                "-keyopt",
                "rsa_oaep_md:sha256",
                "-keyopt",
                "rsa_mgf1_md:sha256",
            ])
            .output()
            .expect("run openssl cms fixture generator");
        assert!(
            output.status.success(),
            "fixture generation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let encrypted = std::fs::read(&encrypted_path).expect("read encrypted fixture");
        let decrypted = smime_engine::decrypt(
            &generated.pem_private_key,
            &generated.pem_certificate,
            None,
            &encrypted,
        )
        .expect("decrypt RSA-OAEP/SHA-256 S/MIME message");

        assert_eq!(decrypted, plaintext);
    }
}
