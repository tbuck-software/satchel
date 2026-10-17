# Satchel lettre patches

Satchel vendors Lettre 0.11.19 because its `rustls-platform-verifier` feature calls
`Verifier::new_with_extra_roots` on Android, while that API is intentionally not
available for Android's TrustManager-backed verifier.

The local patch uses `Verifier::new` when the extra-root list is empty (Lettre's
default path) and fails closed when custom SMTP roots are requested. Remove this
vendor override once upstream Lettre provides an Android-compatible path.

## Upstream condition

The missing piece is upstream of Lettre:
[rustls/rustls-platform-verifier#58](https://github.com/rustls/rustls-platform-verifier/issues/58)
— `new_with_extra_roots` exists for Apple and Windows, and the issue records that
Android needs a `TrustManager` that combines certificates before it can follow.
Once that lands and Lettre picks it up, this fork has no reason to exist:
port the SMTP keepalive settings to an upstream socket option, then delete
`vendor/lettre`, drop the `[patch.crates-io]` entry in the root
`Cargo.toml`, and let the registry version back in.

## What deviates from the registry release

Verified 2026-10-12 against the crates.io source of 0.11.19, with the response cap backported from [upstream PR #1143](https://github.com/lettre/lettre/pull/1143):

```sh
curl -sSL -o lettre.crate https://static.crates.io/crates/lettre/lettre-0.11.19.crate
tar xzf lettre.crate
diff -r -q lettre-0.11.19 vendor/lettre
```

Eight files deviate, all deliberately (six in the table, plus the two keepalive files below):

| File | Deviation |
|---|---|
| `src/transport/smtp/client/tls.rs` | The Android patch described above: the `new_with_extra_roots` branch is gated `not(target_os = "android")`, and a second `target_os = "android"` branch uses `Verifier::new` for the empty-root case and refuses custom roots. |
| `Cargo.toml` | `[lints.rust] dead_code = "allow"`, so the fork's feature-gated dead code (`SpawnHandle::shutdown`, `Kind::TransportShutdown`, `transport_shutdown`) does not surface as three warnings in every build. The items exist upstream too; Satchel builds without the feature combination that uses them. |
| `src/transport/smtp/error.rs` | `Error::is_connection()` and `Error::is_network()`, two predicates over the existing `Kind`. Upstream exposes `is_response`, `is_client`, `is_transient`, `is_permanent`, `is_tls` and `is_transport_shutdown`, but not these two, so "could not reach the server" is indistinguishable from "the connection died while carrying a message". Satchel needs the difference: the first is safe to retry, the second may have delivered the mail (see `sync::errors::smtp_submission_failure`). Additive and upstreamable. |
| `src/transport/smtp/client/mod.rs` | The upstream response limits: 1,000 bytes per line and 100,000 bytes per complete reply. |
| `src/transport/smtp/client/connection.rs` | Upstream #1143 rejects oversized lines and continuation replies before parsing in the synchronous reader. Each read is additionally limited to 1,001 bytes, so an unterminated line cannot bypass the cap. |
| `src/transport/smtp/client/async_connection.rs` | The same reply limits and bounded reads in the asynchronous reader. |

The synchronous and Tokio SMTP connection paths also enable TCP keepalive
with a 60-second idle delay and 15-second probe interval. Lettre's transport
builder does not expose socket keepalive, so Satchel sets it before TLS in
`src/transport/smtp/client/net.rs` and `async_net.rs`.

Nothing else in `src/` differs. The vendor tree carries source only — the
release archive's `tests/`, `benches/`, `examples/`, `docs/`, CI configuration
and changelog are not vendored, which `diff -r -q` reports as "Only in
lettre-0.11.19".

## Reviewed upstream drift as of 2026-10-12

The check below found 0.11.23 released while this fork sits on 0.11.19 — the
ageing this whole file exists to prevent. What Satchel is missing, read off
upstream's changelog:

| Release | Change | Reaches Satchel's build? |
|---|---|---|
| 0.11.20 | Email address parsing rewritten from `chumsky` to `nom` (#1099) | Yes — `builder` is enabled |
| 0.11.20 | `Address::new_dangerous`; MSRV 1.85, Rust 2024 edition | Toolchain is 1.97, so no obstacle |
| 0.11.21 | `rustls-no-provider`, `message_iter` on connections | Additive |
| 0.11.22 | `read_response` buffer capped against unbounded memory growth (#1143) | Yes, backported on 2026-10-12 |
| 0.11.22 | Inverted TLS hostname-verification flag in the `boring-tls` backend | No — Satchel builds rustls only |
| 0.11.22 | `rustls-platform-verifier` to v0.7 (#1136) | Blocks the upgrade, see below |
| 0.11.23 | Multiline SMTP error replies split into separate lines (#1153) | Yes |
| 0.11.23 | `base64` to v0.23 (#1152) | Dependency update; retained with the older release |

The upgrade is not just a re-vendor. 0.11.22 requires
`rustls-platform-verifier` 0.7, and both `crates/satchel-core/Cargo.toml` and
`src-tauri/Cargo.toml` pin `=0.6.2` because the Android JNI init call is taken
from that exact version. Moving to 0.7 changes mobile TLS setup and needs a run
on a real device, which is why it is not folded in here.

The fork itself is still required: 0.11.23 calls `new_with_extra_roots`
unconditionally under the `rustls-platform-verifier` feature, and 0.7 still
implements that constructor for Apple, Windows and the generic backend only —
not for Android.

## What keeps this honest

`crates/satchel-core/tests/vendor_lettre.rs` holds the version pinned in
`vendor/lettre/Cargo.toml` against the one this file names, checks that the
patch table still points at the vendor tree, and — when
`SATCHEL_VENDOR_UPSTREAM_CHECK=1` is set — asks crates.io whether a newer
release exists. The weekly `Security audit` workflow sets that variable. The reviewed drift through
0.11.23 is temporarily accepted by `.cargo/lettre-upstream-exception.json` until
2026-11-12 UTC. A newer release or an expired deadline fails the check.
`crates/satchel-core/tests/smtp_response_caps.rs` exercises the response limits
against a hostile peer in both readers.
