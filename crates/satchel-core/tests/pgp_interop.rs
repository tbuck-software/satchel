//! End-to-end interoperability between Satchel's PGP engine and GnuPG.
//!
//! The unit tests around `email::pgp_mime` only prove the MIME envelope is
//! shaped correctly. They say nothing about whether the bytes inside are a
//! signature GnuPG accepts, or ciphertext it can open — which is the part that
//! actually determines whether a recipient can read the mail.
//!
//! These tests drive the real `gpg` binary in a throwaway home directory:
//! Satchel signs, GnuPG verifies; Satchel encrypts, GnuPG decrypts; and the
//! reverse for decryption. They skip when no gpg is installed. Set
//! `SATCHEL_REQUIRE_GPG=1` in an environment that explicitly promises the
//! optional binary to turn a missing installation into a failure.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use satchel_core::utils::pgp_engine::{self, GenerateKeyRequest};

const PASSPHRASE: &str = "interop-test-passphrase";
const UID: &str = "Satchel Interop <interop@example.test>";
const GPG_FIXTURE_PASSPHRASE: &str = "fixture-passphrase";
const GPG_FIXTURE_FINGERPRINT: &str = "C29160B0BADC8557E5FEF30C5A4E7D912785BF5C";
const GPG_FIXTURE_PRIVATE: &str = include_str!("corpus/pgp/gpg-private.asc");
const GPG_FIXTURE_PUBLIC: &str = include_str!("corpus/pgp/gpg-public.asc");
const GPG_FIXTURE_PAYLOAD: &str = include_str!("corpus/pgp/gpg-payload.txt");
const GPG_FIXTURE_SIGNATURE: &str = include_str!("corpus/pgp/gpg-payload.sig.asc");
const GPG_FIXTURE_CIPHERTEXT: &str = include_str!("corpus/pgp/gpg-payload.enc.asc");
const GPG_FIXTURE_BZIP2_CIPHERTEXT: &str = include_str!("corpus/pgp/gpg-payload.bzip2.enc.asc");
// Test-only key and ciphertext from a fresh rPGP CV25519 key whose first
// GnuPG unlock failed immediately after import. The passphrase is PASSPHRASE.
const RPGP_IMPORT_PRIVATE: &str = include_str!("corpus/pgp/rpgp-cv25519-import.sec.asc");
const RPGP_IMPORT_CIPHERTEXT: &str = include_str!("corpus/pgp/rpgp-cv25519-import.enc.asc");

fn gpg_available() -> bool {
    gpg_binary().is_some()
}

fn require_gpg() -> bool {
    std::env::var("SATCHEL_REQUIRE_GPG")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Returns false when the test should be skipped.
fn ensure_gpg(test: &str) -> bool {
    if gpg_available() {
        return true;
    }
    assert!(
        !require_gpg(),
        "{test}: gpg is required (SATCHEL_REQUIRE_GPG=1) but was not found"
    );
    eprintln!("[skip] {test}: no gpg binary available");
    false
}

fn gpg_binary() -> Option<String> {
    let configured = std::env::var("SATCHEL_GPG_BIN").ok();
    let found = configured
        .iter()
        .map(String::as_str)
        .chain([
            "gpg",
            "gpg2",
            "/opt/homebrew/bin/gpg",
            "/usr/local/bin/gpg",
            "/usr/bin/gpg",
        ])
        .find(|candidate| {
            !candidate.trim().is_empty()
                && Command::new(candidate)
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
        })
        .map(str::to_string);
    found
}

/// Run gpg against an isolated home so the developer's real keyring is never
/// read or modified.
fn run_gpg(home: &Path, args: &[&str], stdin: Option<&[u8]>) -> (bool, String, String) {
    let binary = gpg_binary().expect("gpg availability was checked before this test");
    let mut cmd = Command::new(binary);
    // Force a C locale: gpg translates its diagnostics, and the assertions
    // below match on them. Without this the test passes or fails depending on
    // the developer's language settings.
    cmd.env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "")
        .arg("--homedir")
        .arg(home)
        .arg("--batch")
        .arg("--yes")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("gpg should start");
    if let Some(data) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(data)
            .expect("write stdin");
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("gpg should finish");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn stop_gpg_processes(home: &Path, target: &str) -> bool {
    let configured = std::env::var("SATCHEL_GPGCONF_BIN").ok();
    let sibling = gpg_binary().and_then(|binary| {
        Path::new(&binary)
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| {
                path.join(if cfg!(windows) {
                    "gpgconf.exe"
                } else {
                    "gpgconf"
                })
            })
    });
    configured
        .map(std::path::PathBuf::from)
        .into_iter()
        .chain(sibling)
        .chain(std::iter::once(std::path::PathBuf::from(
            if cfg!(windows) {
                "gpgconf.exe"
            } else {
                "gpgconf"
            },
        )))
        .any(|binary| {
            Command::new(binary)
                .arg("--homedir")
                .arg(home)
                .args(["--kill", target])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
}

fn multipart_boundary(message: &str) -> &str {
    let marker = "boundary=\"";
    let start = message.find(marker).expect("multipart boundary") + marker.len();
    let rest = &message[start..];
    &rest[..rest.find('"').expect("boundary closing quote")]
}

fn first_multipart_part(message: &str) -> &str {
    let boundary = multipart_boundary(message);
    let opening = format!("--{boundary}\r\n");
    let start = message.find(&opening).expect("first multipart delimiter") + opening.len();
    let closing = format!("\r\n--{boundary}");
    let end = message[start..]
        .find(&closing)
        .map(|offset| start + offset)
        .expect("second multipart delimiter");
    &message[start..end]
}

fn armored_message_part(message: &str) -> &str {
    let start = message
        .find("-----BEGIN PGP MESSAGE-----")
        .expect("encrypted armor start");
    let end = message
        .find("-----END PGP MESSAGE-----")
        .expect("encrypted armor end")
        + "-----END PGP MESSAGE-----".len();
    &message[start..end]
}

struct Fixture {
    _home: tempfile::TempDir,
    home: std::path::PathBuf,
    private_key: String,
    public_key: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = stop_gpg_processes(&self.home, "all");
    }
}

#[test]
fn native_engine_reads_the_gnupg_corpus_without_a_binary() {
    let engine = pgp_engine::engine_by_id("rpgp");
    let metadata = engine
        .key_metadata(GPG_FIXTURE_PRIVATE)
        .expect("read GnuPG private key");
    assert_eq!(metadata.fingerprint, GPG_FIXTURE_FINGERPRINT);
    assert_eq!(
        metadata.user_ids,
        ["Satchel GPG Fixture <gpg-fixture@example.test>"]
    );
    assert!(metadata.can_sign);
    assert!(metadata.can_encrypt);
    assert!(engine
        .verify_private_key_passphrase(GPG_FIXTURE_PRIVATE, GPG_FIXTURE_PASSPHRASE)
        .expect("check fixture passphrase"));
    assert!(!engine
        .verify_private_key_passphrase(GPG_FIXTURE_PRIVATE, "wrong")
        .expect("reject wrong fixture passphrase"));

    for ciphertext in [GPG_FIXTURE_CIPHERTEXT, GPG_FIXTURE_BZIP2_CIPHERTEXT] {
        let plaintext = engine
            .decrypt_with_private_key(GPG_FIXTURE_PRIVATE, GPG_FIXTURE_PASSPHRASE, ciphertext)
            .expect("decrypt GnuPG fixture");
        assert_eq!(plaintext, GPG_FIXTURE_PAYLOAD);
    }
}

#[test]
fn native_engine_signs_with_a_gnupg_generated_key() {
    let engine = pgp_engine::engine_by_id("rpgp");
    let signature = engine
        .sign_detached(
            GPG_FIXTURE_PRIVATE,
            GPG_FIXTURE_PASSPHRASE,
            GPG_FIXTURE_PAYLOAD.as_bytes(),
        )
        .expect("sign with GnuPG fixture key");
    let (signature, _) = DetachedSignature::from_string(&signature).expect("parse signature");
    let (public_key, _) =
        SignedPublicKey::from_string(GPG_FIXTURE_PUBLIC).expect("parse public key");
    signature
        .verify(&public_key.primary_key, GPG_FIXTURE_PAYLOAD.as_bytes())
        .expect("verify native signature against GnuPG key");
}

#[test]
fn native_engine_verifies_the_gnupg_detached_signature_fixture() {
    let (signature, _) =
        DetachedSignature::from_string(GPG_FIXTURE_SIGNATURE).expect("parse GnuPG signature");
    let (public_key, _) =
        SignedPublicKey::from_string(GPG_FIXTURE_PUBLIC).expect("parse public key");
    signature
        .verify(&public_key.primary_key, GPG_FIXTURE_PAYLOAD.as_bytes())
        .expect("verify GnuPG signature");
    assert!(
        signature
            .verify(&public_key.primary_key, b"tampered")
            .is_err(),
        "the fixture signature must bind the exact payload"
    );
}

#[test]
fn optional_gpg_engine_covers_its_core_operations_when_installed() {
    if !ensure_gpg("optional_gpg_engine_covers_its_core_operations_when_installed") {
        return;
    }
    let engine = pgp_engine::engine_by_id("gpg");
    let generated = engine
        .generate_keypair(&GenerateKeyRequest {
            uid: UID,
            algorithm: "default",
            expires_in: "0",
            passphrase: Some(PASSPHRASE),
        })
        .expect("GnuPG key generation");
    assert!(engine
        .verify_private_key_passphrase(&generated.private_armored, PASSPHRASE)
        .expect("GnuPG passphrase check"));
    assert_eq!(
        engine
            .key_metadata(&generated.public_armored)
            .expect("GnuPG metadata")
            .fingerprint,
        generated.fingerprint
    );

    let payload = b"optional CLI engine";
    let signature = engine
        .sign_detached(&generated.private_armored, PASSPHRASE, payload)
        .expect("GnuPG detached signature");
    let (signature, _) = DetachedSignature::from_string(&signature).expect("parse signature");
    let (public_key, _) =
        SignedPublicKey::from_string(&generated.public_armored).expect("parse public key");
    signature
        .verify(&public_key.primary_key, payload)
        .expect("verify GnuPG signature");

    let ciphertext = engine
        .encrypt_to(std::slice::from_ref(&generated.public_armored), payload)
        .expect("GnuPG encryption");
    let plaintext = engine
        .decrypt_with_private_key(&generated.private_armored, PASSPHRASE, &ciphertext)
        .expect("GnuPG decryption");
    assert_eq!(plaintext.as_bytes(), payload);
}

/// Generate a key with Satchel's engine and import it into a fresh gpg home.
fn fixture() -> Fixture {
    let generated = pgp_engine::generate_keypair(&GenerateKeyRequest {
        uid: UID,
        algorithm: "default",
        expires_in: "0",
        passphrase: Some(PASSPHRASE),
    })
    .expect("keypair generation should succeed");

    let home = tempfile::Builder::new()
        .prefix("satchel-interop-")
        .tempdir()
        .expect("tempdir");
    let home_path = home.path().to_path_buf();
    // gpg warns (and on some versions refuses) when the home directory is
    // group- or world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&home_path, std::fs::Permissions::from_mode(0o700));
    }

    let (ok, _, stderr) = run_gpg(
        &home_path,
        &["--import"],
        Some(generated.private_armored.as_bytes()),
    );
    assert!(ok, "gpg should import the generated key: {stderr}");

    // Mark the key ultimately trusted so encryption to it needs no prompting.
    let (_, fingerprints, _) = run_gpg(&home_path, &["--with-colons", "--list-keys"], None);
    let fpr = fingerprints
        .lines()
        .find(|line| line.starts_with("fpr:"))
        .and_then(|line| line.split(':').nth(9))
        .expect("imported key should have a fingerprint")
        .to_string();
    let ownertrust = format!("{fpr}:6:\n");
    run_gpg(
        &home_path,
        &["--import-ownertrust"],
        Some(ownertrust.as_bytes()),
    );

    Fixture {
        _home: home,
        home: home_path,
        private_key: generated.private_armored,
        public_key: generated.public_armored,
    }
}

#[test]
fn gpg_verifies_a_signature_satchel_produced() {
    if !ensure_gpg("gpg_verifies_a_signature_satchel_produced") {
        return;
    }
    let fx = fixture();
    let payload = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHallo Welt\r\n";

    let signature = pgp_engine::sign_detached(&fx.private_key, PASSPHRASE, payload)
        .expect("Satchel should produce a signature");
    assert!(
        signature.contains("-----BEGIN PGP SIGNATURE-----"),
        "signature should be armored"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let sig_path = dir.path().join("sig.asc");
    let data_path = dir.path().join("data.bin");
    std::fs::write(&sig_path, &signature).expect("write sig");
    std::fs::write(&data_path, payload).expect("write data");

    let (ok, _, stderr) = run_gpg(
        &fx.home,
        &[
            "--verify",
            sig_path.to_string_lossy().as_ref(),
            data_path.to_string_lossy().as_ref(),
        ],
        None,
    );
    assert!(ok, "gpg should verify the signature: {stderr}");
    assert!(
        stderr.contains("Good signature"),
        "gpg should report a good signature, got: {stderr}"
    );
}

#[test]
fn gpg_rejects_a_signature_over_tampered_content() {
    if !ensure_gpg("gpg_rejects_a_signature_over_tampered_content") {
        return;
    }
    let fx = fixture();
    let signature =
        pgp_engine::sign_detached(&fx.private_key, PASSPHRASE, b"original text").expect("sign");

    let dir = tempfile::tempdir().expect("tempdir");
    let sig_path = dir.path().join("sig.asc");
    let data_path = dir.path().join("data.bin");
    std::fs::write(&sig_path, &signature).expect("write sig");
    std::fs::write(&data_path, b"tampered text").expect("write data");

    let (ok, _, _) = run_gpg(
        &fx.home,
        &[
            "--verify",
            sig_path.to_string_lossy().as_ref(),
            data_path.to_string_lossy().as_ref(),
        ],
        None,
    );
    assert!(!ok, "a signature must not validate over modified content");
}

#[test]
fn gpg_decrypts_what_satchel_encrypted() {
    if !ensure_gpg("gpg_decrypts_what_satchel_encrypted") {
        return;
    }
    let fx = fixture();
    let plaintext = "Content-Type: text/plain\r\n\r\nStreng geheim: Grüße mit Umlauten\r\n";

    let ciphertext =
        pgp_engine::encrypt_to(std::slice::from_ref(&fx.public_key), plaintext.as_bytes())
            .expect("Satchel should encrypt");
    assert!(ciphertext.contains("-----BEGIN PGP MESSAGE-----"));
    assert!(
        !ciphertext.contains("Streng geheim"),
        "the plaintext must not survive in the ciphertext"
    );

    let (ok, stdout, stderr) = run_gpg(
        &fx.home,
        &[
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            PASSPHRASE,
            "--decrypt",
        ],
        Some(ciphertext.as_bytes()),
    );
    assert!(ok, "gpg should decrypt: {stderr}");
    assert_eq!(
        stdout, plaintext,
        "decrypted content must match the original byte for byte"
    );
}

#[test]
fn gpg_engine_unlocks_an_imported_rpgp_cv25519_subkey() {
    if !ensure_gpg("gpg_engine_unlocks_an_imported_rpgp_cv25519_subkey") {
        return;
    }
    let engine = pgp_engine::engine_by_id("gpg");
    assert!(engine
        .decrypt_with_private_key(RPGP_IMPORT_PRIVATE, "wrong", RPGP_IMPORT_CIPHERTEXT)
        .is_err());
    let plaintext = engine
        .decrypt_with_private_key(RPGP_IMPORT_PRIVATE, PASSPHRASE, RPGP_IMPORT_CIPHERTEXT)
        .expect("GnuPG should unlock the imported rPGP CV25519 subkey on the first attempt");
    assert_eq!(plaintext, "GnuPG control plaintext\n");
}

#[test]
fn satchel_decrypts_what_gpg_encrypted() {
    if !ensure_gpg("satchel_decrypts_what_gpg_encrypted") {
        return;
    }
    let fx = fixture();
    let plaintext = "Von gpg verschlüsselt, für Satchel bestimmt.\n";

    let (ok, ciphertext, stderr) = run_gpg(
        &fx.home,
        &[
            "--armor",
            "--trust-model",
            "always",
            "--recipient",
            UID,
            "--encrypt",
        ],
        Some(plaintext.as_bytes()),
    );
    assert!(ok, "gpg should encrypt: {stderr}");

    let decrypted = pgp_engine::decrypt_with_private_key(&fx.private_key, PASSPHRASE, &ciphertext)
        .expect("Satchel should decrypt gpg's ciphertext");
    assert_eq!(decrypted.trim_end(), plaintext.trim_end());
}

#[test]
fn encrypting_to_several_recipients_lets_each_one_decrypt() {
    if !ensure_gpg("encrypting_to_several_recipients_lets_each_one_decrypt") {
        return;
    }
    let alice = fixture();
    let bob = fixture();
    let plaintext = "An zwei Empfänger.\n";

    let ciphertext = pgp_engine::encrypt_to(
        &[alice.public_key.clone(), bob.public_key.clone()],
        plaintext.as_bytes(),
    )
    .expect("encrypt to two recipients");

    for (who, fx) in [("alice", &alice), ("bob", &bob)] {
        let (ok, stdout, stderr) = run_gpg(
            &fx.home,
            &[
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                PASSPHRASE,
                "--decrypt",
            ],
            Some(ciphertext.as_bytes()),
        );
        assert!(ok, "{who} should be able to decrypt: {stderr}");
        assert_eq!(stdout, plaintext, "{who} got the wrong plaintext");
    }
}

/// The full outgoing path: what actually goes on the wire must verify.
#[test]
fn a_signed_mime_message_verifies_against_its_own_signature_part() {
    if !ensure_gpg("a_signed_mime_message_verifies_against_its_own_signature_part") {
        return;
    }
    let fx = fixture();
    let rfc822 = "From: me@example.test\r\nTo: you@example.test\r\nSubject: Test\r\n\
                  MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
                  Signierter Text mit Umlauten: äöü\r\n";

    let entity = satchel_core::email::pgp_mime::signable_entity(rfc822);
    let signature =
        pgp_engine::sign_detached(&fx.private_key, PASSPHRASE, entity.as_bytes()).expect("sign");
    let signed = satchel_core::email::pgp_mime::build_signed(rfc822, &signature);

    // Pull both parts back out of the assembled message, the way a receiving
    // client would, and check they still verify together.
    let sig_start = signed
        .find("-----BEGIN PGP SIGNATURE-----")
        .expect("assembled message should carry the signature");
    let sig_end = signed
        .find("-----END PGP SIGNATURE-----")
        .expect("signature end")
        + "-----END PGP SIGNATURE-----".len();
    let extracted_signature = &signed[sig_start..sig_end];

    let dir = tempfile::tempdir().expect("tempdir");
    let sig_path = dir.path().join("sig.asc");
    let data_path = dir.path().join("entity.bin");
    std::fs::write(&sig_path, extracted_signature).expect("write sig");
    let serialized_entity = first_multipart_part(&signed);
    assert_eq!(serialized_entity, entity);
    std::fs::write(&data_path, serialized_entity.as_bytes()).expect("write entity");

    let (ok, _, stderr) = run_gpg(
        &fx.home,
        &[
            "--verify",
            sig_path.to_string_lossy().as_ref(),
            data_path.to_string_lossy().as_ref(),
        ],
        None,
    );
    assert!(
        ok,
        "the signature shipped in the MIME part must verify: {stderr}"
    );
    assert!(stderr.contains("Good signature"), "got: {stderr}");
}

#[test]
fn a_signed_then_encrypted_mime_message_round_trips_through_gpg() {
    if !ensure_gpg("a_signed_then_encrypted_mime_message_round_trips_through_gpg") {
        return;
    }
    let fx = fixture();
    let rfc822 = "From: me@example.test\r\nTo: you@example.test\r\nSubject: Geheim\r\n\
                  MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
                  Signiert und verschlüsselt.\r\n";

    let entity = satchel_core::email::pgp_mime::signable_entity(rfc822);
    let signature =
        pgp_engine::sign_detached(&fx.private_key, PASSPHRASE, entity.as_bytes()).expect("sign");
    let nested = satchel_core::email::pgp_mime::signed_entity_for_encryption(rfc822, &signature);
    let ciphertext =
        pgp_engine::encrypt_to(std::slice::from_ref(&fx.public_key), nested.as_bytes())
            .expect("encrypt");
    let wire = satchel_core::email::pgp_mime::build_encrypted(rfc822, &ciphertext);

    let (ok, decrypted, stderr) = run_gpg(
        &fx.home,
        &[
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            PASSPHRASE,
            "--decrypt",
        ],
        Some(armored_message_part(&wire).as_bytes()),
    );
    assert!(ok, "gpg should decrypt the wire message: {stderr}");
    assert_eq!(decrypted, nested);
    assert!(decrypted.contains("Content-Type: multipart/signed"));
    assert!(decrypted.contains("-----BEGIN PGP SIGNATURE-----"));
}
