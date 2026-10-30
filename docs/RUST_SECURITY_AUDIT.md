# Rust dependency audit

CI runs `cargo audit` on dependency changes and every Monday. It scans the root
and fuzzing lockfiles, including platform and plugin
build dependencies. New RustSec vulnerabilities fail the workflow. Dependabot
also checks both workspaces against GitHub's advisory database.

OpenSSL 0.10.81 replaces the old 0.10.73 pin in the core,
and both lockfiles use it. This includes the buffer-sizing fixes through
[GHSA-phqj-4mhp-q6mq](https://github.com/rust-openssl/rust-openssl/security/advisories/GHSA-phqj-4mhp-q6mq).
Serde-with 3.21.0 fixes
[GHSA-7gcf-g7xr-8hxj](https://github.com/jonasbb/serde_with/security/advisories/GHSA-7gcf-g7xr-8hxj).

The obsolete standalone S/MIME harness was removed after the Core split.
Its RSA-OAEP/SHA-256 interoperability regression is covered by
`utils::smime_engine::tests::in_process_reads_rsa_oaep_envelopes` in the normal
Core test suite.

## Temporary exception: RUSTSEC-2023-0071

[`RUSTSEC-2023-0071`](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)
reports a timing side channel in `rsa` 0.9.10 private-key operations. The package
is pulled in by `pgp` for OpenPGP support and by the unused `sqlx-mysql`
dependency. Satchel does use OpenPGP private-key decryption when the user opens
an encrypted message, so this is a real code path. The PGP plugin's sync
transform only labels encrypted mail and does not decrypt it. The advisory has
no patched version as of 2026-09-26. Removing `rsa` would remove existing
OpenPGP support.

The reported attack requires an attacker to submit many chosen ciphertexts and
measure private-key operation times over a network. Satchel decrypts cached
mail on the user's device when the user opens it; the sender does not receive
individual decryption timing responses. This limits the remote timing channel,
though it does not prove the implementation safe against a local observer.

`.cargo/audit.toml` ignores `RUSTSEC-2023-0071`. The audit workflow fails
after 2026-12-31 UTC while the exception remains. The deadline is enforced in
`scripts/check-rust-audit-exceptions.py`. Review earlier when `rsa` or `pgp` publishes a fix.
Remove the exception as soon as the lockfile can use a fixed version. The
weekly workflow still reports other advisories.

## Temporary exception: RUSTSEC-2026-0387

[`RUSTSEC-2026-0387`](https://rustsec.org/advisories/RUSTSEC-2026-0387.html)
reports timing variability in `crypto-bigint` operations intended to be
constant-time. Satchel resolves 0.5.5 through `pgp` 0.20.0, both directly through
`cx448` and through `elliptic-curve` 0.13.8 and its curve/signature crates.
Secret-dependent timing or microarchitectural observations may expose
cryptographic secrets; exploitability depends on the operation, compiler,
CPU and observer. This exception accepts that unresolved risk.

The in-process `RpgpEngine` is the default on desktop and mobile. Its
`decrypt_with_private_key` and `sign_detached` methods in
`crates/satchel-core/src/utils/pgp_engine.rs` accept imported keys, so the
exposure is broader than Satchel's key-generation choices:

- Imported NIST P-256, P-384 and P-521 ECDH keys reach the affected dependency
  during private-key session-key decryption through `p256`, `p384`, `p521`
  and `elliptic-curve`. Outgoing ECDSA signing with P-256 or secp256k1 keys
  reaches it through `p256`, `k256` and `ecdsa`. Satchel selects SHA-256,
  so `pgp` rejects outgoing P-384 and P-521 signatures before signing because
  those require at least 384-bit and 512-bit digests respectively. Importing
  and unlocking those secret keys still reaches their curve arithmetic.
- Imported X448 decryption keys reach `cx448`. Ed448 also uses `cx448` for
  signing and verification, but Satchel's outgoing detached signatures select
  SHA-256, which `pgp` rejects for Ed448 before signing because it requires at
  least a 512-bit hash. Importing/unlocking an Ed448 secret key still constructs
  its public key through `cx448`; this is not a claim that Ed448 secret-key
  arithmetic is unreachable.
- Signature and certificate verification on these curves uses public values.
  Outgoing encryption to NIST ECDH or X448 recipients also uses secret ephemeral
  scalars, even though it does not use the sender's long-term private key.
- Satchel generates Ed25519/Curve25519 keys by default, or RSA when selected.
  The Ed25519 and X25519 arithmetic uses the Dalek crates rather than
  `crypto-bigint`; selecting these defaults does not eliminate the imported-key
  paths. RSA has the separate exception above. The optional GnuPG engine
  delegates signing and decryption to GnuPG instead of these Rust operations.

Private-key decryption runs on cached mail when the user requests it; signing
runs locally while preparing outgoing mail. The PGP sync plugin only labels
messages. A remote sender can supply ciphertexts, but Satchel does not return
per-operation decryption timings over the mail transport. This limits that
remote observation channel. It does not establish constant-time execution or
exclude local timing/cache observations, nor does it prove that an observer of
outgoing messages cannot obtain useful aggregate timing information. No
end-to-end exploitability assessment has established these paths safe.

The patched release is `crypto-bigint` 0.7.6 or later. As of 2026-10-27, even
`pgp` 0.21.0 still requires `elliptic-curve` ^0.13, whose `crypto-bigint`
requirement cannot select 0.7.6. A `pgp` version bump alone cannot fix the graph,
and forcing a semver-incompatible arithmetic crate is not a safe substitution.
The owner approved this temporary exception to preserve OpenPGP support.

`.cargo/audit.toml` ignores `RUSTSEC-2026-0387` for both lockfile scans.
`scripts/check-rust-audit-exceptions.py` allows it through 2026-12-31 UTC and
fails from 2027-01-01 UTC while it remains configured, using the same deadline
mechanism as the RSA exception. Review when `pgp`, `elliptic-curve` or `cx448`
ships compatible fixes. Remove the ignore and its deadline entry as soon as
both lockfiles can resolve only patched versions, then rerun both audits and
the OpenPGP interoperability tests.

## Remaining audit warnings

The 2026-10-27 scans pass with the two temporary exceptions above. The root
lockfile reports ten warnings and the fuzz lockfile four, which `cargo audit`
does not treat as vulnerabilities. The zstd-family warnings are gone. The
remaining warnings are:

- `fxhash` is unmaintained through Wasmtime's profiling dependency.
- `proc-macro-error`, `zbus_names`, `zvariant`, and `zvariant_derive` have
  maintenance advisories in the desktop dependency graph.
- `glib` 0.18.5 has an unsoundness advisory in the Linux GTK stack used by
  Tauri.
- `tokio` has an I/O-safety unsoundness advisory for `AsyncFd` constructors,
  RUSTSEC-2026-0386, in both lockfiles, at 1.49.0 in the root and 1.53.2 in fuzz.

These are upstream dependency paths. Recheck them on each lockfile update and
remove the warnings by updating their parents when compatible releases exist.

## Wasmtime and vendored dependencies

Wasmtime and wasmtime-wasi 36.0.20 fix RUSTSEC-2026-0424, unchecked memory
use when transferring borrowed resources between components. 36.0.19 fixed
RUSTSEC-2026-0379 and RUSTSEC-2026-0380 in Cranelift, and 36.0.18 fixed
RUSTSEC-2026-0363 and RUSTSEC-2026-0352. Plugin runtime tests run with built
components required.

The plugin compilation cache remains enabled. Satchel vendors
`wasmtime-internal-cache` 36.0.20 with only its zstd dependency requirement
changed to 0.14.3. The root lockfile now uses zstd 0.14.3, zstd-safe 8.2.0 and
zstd-sys 2.1.2, fixing RUSTSEC-2026-0391, RUSTSEC-2026-0398,
RUSTSEC-2026-0403 and RUSTSEC-2026-0410 and removing the zstd-safe unsoundness
warnings. The vendored crate's version must match the Wasmtime release, or
Cargo drops the patch and falls back to zstd 0.13: re-vendor it with every
Wasmtime bump. The fuzz workspace does not depend on Wasmtime or zstd, so its
lockfile needs no update. See
`vendor/wasmtime-internal-cache/SATCHEL_PATCH.md` for the exact diff and removal
condition.

Both lockfiles use rustls-webpki 0.103.16, webpki-roots 1.0.10 and
webpki-root-certs 1.0.10. These fix RUSTSEC-2026-0382, RUSTSEC-2026-0381
and RUSTSEC-2026-0383 in certificate validation and the bundled trust stores.
The 0.26 webpki-roots compatibility wrapper delegates to the patched 1.0.10
trust store. No additional audit exception is needed.

The lettre fork remains based on 0.11.19 for Android verifier compatibility.
Upstream's 0.11.22 SMTP response limits are backported. Reviewed release drift
through 0.11.23 is accepted until 2026-11-12 UTC in
`.cargo/lettre-upstream-exception.json`; a later release or deadline fails the
weekly comparison. See `vendor/lettre/SATCHEL_PATCH.md` for the remaining diff.

`cargo deny --locked check licenses sources` checks all platforms and development
dependencies. `deny.toml` allows only the licenses needed by the current tree,
crates.io, and the postroom Git repository with revision pins. First-party crates
declare MIT, matching the root LICENSE. The security workflow runs on hosted Linux runners.

Ammonia 4.2.4 fixes the CSS URL handling advisories through RUSTSEC-2026-0373.
Rustls 0.23.46 fixes RUSTSEC-2026-0346 through RUSTSEC-2026-0349. The updater
uses its Rustls transport without legacy ZIP support: releases ship raw NSIS
and AppImage payloads, with macOS tar support supplied independently. This
removes the unused ZIP 4 extractor affected by RUSTSEC-2026-0367 through
RUSTSEC-2026-0369. No new audit exception is needed.
The existing quinn-proto 0.11.18 fixes RUSTSEC-2026-0338 through RUSTSEC-2026-0342.
