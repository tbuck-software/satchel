# Satchel lettre patches

Satchel vendors Lettre 0.11.24. On 2026-11-21 the
[crates.io API](https://crates.io/api/v1/crates/lettre) reported 0.11.24 as the
latest stable release. The published archive has SHA-256
`f294f3d7cdb313e578acef7d7727d4e8d897e0fcd0d6d58c19959cec95073f19`
and names commit `a410c3949488b348b48ea82a63a83f970c56515d`.

The previous 0.11.19 vendor source and manifest differed from the registry
archive in the eight documented files. The 0.11.24 release replaces the local
Android TLS patch and the response-limit backport. It also fixes STARTTLS reply
handling, SMTP dot-stuffing, address parsing and other issues documented in the
[release notes](https://github.com/lettre/lettre/releases/tag/v0.11.24).
The dated exception for release drift has been removed. The weekly check in
`crates/satchel-core/tests/vendor_lettre.rs` fails when crates.io has a newer
stable release.

## Android TLS

[Lettre #1178](https://github.com/lettre/lettre/pull/1178) uses
`Verifier::new` with no custom roots and rejects custom roots on Android.
The [verifier's Android implementation](https://github.com/rustls/rustls-platform-verifier/blob/252e25161a91af476cbea620e29d277592d27ec7/rustls-platform-verifier/src/verification/android.rs)
still lacks `new_with_extra_roots`. Support for combining custom and system
roots is tracked in [rustls/rustls-platform-verifier#58](https://github.com/rustls/rustls-platform-verifier/issues/58).

The vendored manifest accepts verifier 0.6.2 so SMTP shares the same verifier
as IMAP/POP and the JNI initialization in the Android app and notification
worker. The upstream Lettre code uses constructors available in 0.6.2.
[Verifier 0.7.1](https://github.com/rustls/rustls-platform-verifier/blob/252e25161a91af476cbea620e29d277592d27ec7/rustls-platform-verifier/Cargo.toml)
uses JNI 0.22 and Android Maven artifact 0.2.0.
Satchel's current initialization uses JNI 0.21 and artifact 0.1.1. Moving all
callers to 0.7 together requires checking both Android processes on a device.

## Local changes against the registry archive

| File | Change |
|---|---|
| `Cargo.toml` | Accept platform verifier 0.6.2 and allow dead code in disabled feature combinations. |
| `src/transport/smtp/error.rs` | Expose `is_connection` and `is_network` so Satchel can distinguish a failed connection from a potentially delivered message. |
| `src/transport/smtp/client/net.rs` and `src/transport/smtp/client/async_net.rs` | Set TCP keepalive before TLS, with a 60-second idle delay and 15-second probe interval. |

The rest of the source matches the published archive. In particular, both
SMTP readers use upstream's 100,000-byte total reply limit, including for
unterminated lines. `crates/satchel-core/tests/smtp_response_caps.rs` exercises
both readers over TCP.

To remove the vendor override, move the verifier and its Android integration
to upstream's supported version and find upstream equivalents for the SMTP
error classification and keepalive settings.
