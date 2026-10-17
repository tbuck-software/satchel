# Rust dependency audit

CI runs `cargo audit --file Cargo.lock` on dependency changes and every Monday.
The scan covers the full lockfile, including platform dependencies and plugin
build dependencies. New RustSec vulnerabilities fail the workflow.

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

`.cargo/audit.toml` ignores only `RUSTSEC-2023-0071`. The audit workflow fails
after 2026-12-31 UTC while the exception remains. The deadline is enforced in
`scripts/check-rust-audit-exceptions.py`. Review earlier when `rsa` or `pgp` publishes a fix.
Remove the exception as soon as the lockfile can use a fixed version. The
weekly workflow still reports other advisories.

## Remaining audit warnings

The 2026-10-18 lockfile scan reports no other vulnerabilities after the compatible dependency updates. It still reports
warnings, which `cargo audit` does not treat as vulnerabilities:

- `fxhash` is unmaintained through Wasmtime's profiling dependency.
- `proc-macro-error`, `zbus_names`, `zvariant`, and `zvariant_derive` have
  maintenance advisories in the desktop dependency graph.
- `glib` 0.18.5 has an unsoundness advisory in the Linux GTK stack used by
  Tauri.

These are upstream dependency paths. Recheck them on each lockfile update and
remove the warnings by updating their parents when compatible releases exist.

## Wasmtime and the vendored SMTP library

Wasmtime and wasmtime-wasi 36.0.18 also fix RUSTSEC-2026-0363 and
RUSTSEC-2026-0352, in addition to the fixes in 36.0.17. Plugin runtime tests run with built components required.

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
