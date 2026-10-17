//! The vendored lettre fork has to stay accountable to its upstream.
//!
//! `[patch.crates-io]` freezes the SMTP library: `cargo update` does not touch
//! a patched path, and `cargo audit` only reports advisories, not correctness
//! fixes. Address parsing and header encoding are exactly where such fixes land,
//! so something has to notice when upstream moves on.
//!
//! Two halves: the local invariants run always, the upstream comparison runs
//! where a network call is expected (the weekly audit job sets
//! `SATCHEL_VENDOR_UPSTREAM_CHECK=1`).

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn read(relative: &str) -> String {
    let path = repository_root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {path:?}: {err}"))
}

/// `version = "…"` from the `[package]` section of the vendored manifest.
fn pinned_version() -> String {
    let manifest = read("vendor/lettre/Cargo.toml");
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .find_map(|line| line.trim().strip_prefix("version = "))
        .map(|value| value.trim().trim_matches('"').to_string())
        .expect("vendored lettre has a version")
}

/// The version `SATCHEL_PATCH.md` claims the fork is based on.
fn documented_version() -> String {
    let patch = read("vendor/lettre/SATCHEL_PATCH.md");
    patch
        .split_whitespace()
        .skip_while(|word| *word != "Lettre")
        .nth(1)
        .map(|word| word.trim_end_matches(['.', ',']).to_string())
        .expect("SATCHEL_PATCH.md names the vendored version")
}

/// `left` is behind `right`, comparing the numeric release parts.
///
/// Deliberately not a semver dependency: this is one comparison, and a version
/// that does not parse has to be loud rather than silently equal.
fn is_behind(left: &str, right: &str) -> bool {
    fn parts(version: &str) -> Vec<u64> {
        version
            .split('-')
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|part| part.parse().unwrap_or_else(|_| panic!("version {version}")))
            .collect()
    }
    parts(left) < parts(right)
}

/// The documentation and the manifest have to name the same release, or the
/// record of what was reviewed is worthless.
#[test]
fn the_documented_version_is_the_pinned_one() {
    assert_eq!(
        documented_version(),
        pinned_version(),
        "SATCHEL_PATCH.md and vendor/lettre/Cargo.toml disagree about the version"
    );
}

/// The patch note has to name every file that deviates, so the next diff has
/// something to compare against.
#[test]
fn the_patch_note_records_the_deviating_files() {
    let patch = read("vendor/lettre/SATCHEL_PATCH.md");
    for expected in [
        "src/transport/smtp/client/tls.rs",
        "src/transport/smtp/error.rs",
        "src/transport/smtp/client/mod.rs",
        "src/transport/smtp/client/connection.rs",
        "src/transport/smtp/client/async_connection.rs",
        "Cargo.toml",
        "rustls/rustls-platform-verifier",
    ] {
        assert!(
            patch.contains(expected),
            "SATCHEL_PATCH.md no longer mentions {expected}"
        );
    }
}

/// The fork only takes effect through the patch table; without it the vendor
/// tree is dead weight and the Android build breaks again.
#[test]
fn the_workspace_still_patches_lettre_to_the_vendor_tree() {
    let manifest = read("Cargo.toml");
    assert!(
        manifest.contains(r#"lettre = { path = "vendor/lettre" }"#),
        "[patch.crates-io] no longer points at vendor/lettre"
    );
}

#[test]
fn the_version_comparison_notices_a_newer_release() {
    assert!(is_behind("0.11.18", "0.11.19"));
    assert!(is_behind("0.11.19", "0.12.0"));
    assert!(!is_behind("0.11.19", "0.11.19"));
    // A pre-release is not a reason to move.
    assert!(!is_behind("0.11.19", "0.11.19-rc.1"));
}

/// Newest stable lettre on crates.io, or `None` when the network is not there.
fn latest_release() -> Option<String> {
    let output = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "20",
            "--user-agent",
            "satchel-vendor-check (https://github.com/tbuck/satchel)",
            "https://crates.io/api/v1/crates/lettre",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        eprintln!(
            "crates.io unreachable: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout);
    // Hand-picked out of the response rather than pulling in a JSON parser for
    // one field: `"max_stable_version":"0.11.19"`.
    let key = "\"max_stable_version\":\"";
    let start = body.find(key)? + key.len();
    let end = start + body[start..].find('"')?;
    Some(body[start..end].to_string())
}

/// Upstream has not released past the pinned version.
///
/// Gated: a new release must turn the weekly audit red, not every pull request
/// on a laptop without a network.
#[test]
fn upstream_has_no_newer_release() {
    if std::env::var("SATCHEL_VENDOR_UPSTREAM_CHECK").as_deref() != Ok("1") {
        eprintln!("skipped: set SATCHEL_VENDOR_UPSTREAM_CHECK=1 to query crates.io");
        return;
    }
    let Some(latest) = latest_release() else {
        panic!("upstream check requested, but crates.io did not answer");
    };
    let pinned = pinned_version();
    let exception = upstream_exception();
    assert!(
        !is_behind(&pinned, &latest)
            || drift_is_reviewed(
                &pinned,
                &latest,
                &exception,
                chrono::Utc::now().date_naive()
            ),
        "lettre {latest} is out and the fork is still on {pinned}. Re-diff the \
         vendor tree against the new release, carry the Android patch over, and \
         update vendor/lettre/SATCHEL_PATCH.md — or drop the fork if upstream \
         fixed it."
    );
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpstreamException {
    pinned: String,
    reviewed_upstream: String,
    review_by: chrono::NaiveDate,
    reason: String,
}

fn upstream_exception() -> UpstreamException {
    serde_json::from_str(&read(".cargo/lettre-upstream-exception.json"))
        .expect("dated lettre exception")
}

fn drift_is_reviewed(
    pinned: &str,
    latest: &str,
    exception: &UpstreamException,
    today: chrono::NaiveDate,
) -> bool {
    pinned == exception.pinned
        && !is_behind(&exception.reviewed_upstream, latest)
        && today <= exception.review_by
        && !exception.reason.trim().is_empty()
}

#[test]
fn the_upstream_exception_has_a_current_review_deadline() {
    let exception = upstream_exception();
    assert_eq!(exception.pinned, pinned_version());
    assert!(
        chrono::Utc::now().date_naive() <= exception.review_by,
        "lettre upstream exception expired on {}; review the vendor diff",
        exception.review_by
    );
    assert!(!exception.reason.trim().is_empty());
}

#[test]
fn reviewed_drift_does_not_hide_new_releases_or_expired_exceptions() {
    let exception = UpstreamException {
        pinned: "0.11.19".into(),
        reviewed_upstream: "0.11.23".into(),
        review_by: chrono::NaiveDate::from_ymd_opt(2026, 11, 12).unwrap(),
        reason: "Android verifier compatibility".into(),
    };
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    assert!(drift_is_reviewed("0.11.19", "0.11.23", &exception, today));
    assert!(!drift_is_reviewed("0.11.19", "0.11.24", &exception, today));
    assert!(!drift_is_reviewed("0.11.18", "0.11.23", &exception, today));
    assert!(!drift_is_reviewed(
        "0.11.19",
        "0.11.23",
        &exception,
        chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
    ));
}
