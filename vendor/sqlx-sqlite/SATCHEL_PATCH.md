# Satchel sqlx-sqlite patch

Satchel vendors the crates.io release of `sqlx-sqlite` 0.8.6 so that it links
`libsqlite3-sys` 0.39.1, which bundles SQLite 3.54.1. The upstream archive's
SHA-256 is
`c2d12fe70b2c1b4401038055f90f151b78208de1f9f89a7dbfd41587a10c3eea`,
matching the `cksum` of 0.8.6 in the crates.io registry index.

sqlx 0.8.6 requires `libsqlite3-sys` ^0.30.1 (SQLite 3.46) and sqlx 0.9.0
requires `>=0.30.1, <0.38.0` (at most SQLite 3.53). Both bundle a SQLite before
3.54.0, which is affected by
[RUSTSEC-2026-0459](https://rustsec.org/advisories/RUSTSEC-2026-0459.html)
(CVE-2026-119375): a database opened beside a `-wal` file with a different
page size can return uninitialized page-cache memory. Satchel opens such pairs
from outside its profile when it stages a restore (`backup::stage_restore`
reads the chosen file and its WAL to take a consistent snapshot).

## Diff from the registry release

Only `[dependencies.libsqlite3-sys].version` in the normalized `Cargo.toml`
changes, from `0.30.1` to `0.39.1`. Its features (`pkg-config`, `vcpkg`,
`unlock_notify`, `default-features = false`) and the crate's own `bundled`,
`unbundled` and `preupdate-hook` features are unchanged; they all still exist
in 0.39.1. `Cargo.toml.orig` preserves the original upstream workspace
manifest as a reference; Cargo builds this fork from the normalized
`Cargo.toml`.

The source and licenses are unchanged and compile against 0.39.1 as they are.
The registry bookkeeping files (`.cargo-ok`, `.cargo_vcs_info.json`) are
omitted. Satchel resolves this path dependency in the root and fuzz lockfiles,
both patched through `[patch.crates-io]`.

`db::tests::the_bundled_sqlite_has_the_wal_page_size_fix` asserts that the
linked SQLite is 3.54.0 or newer, so dropping the patch fails the normal test
suite and not only the weekly audit.

To compare against the upstream archive:

```sh
curl -fsSL -o /tmp/sqlx-sqlite-0.8.6.crate \
  https://static.crates.io/crates/sqlx-sqlite/sqlx-sqlite-0.8.6.crate
mkdir -p /tmp/satchel-sqlx-sqlite-upstream
tar xzf /tmp/sqlx-sqlite-0.8.6.crate -C /tmp/satchel-sqlx-sqlite-upstream
diff -ru /tmp/satchel-sqlx-sqlite-upstream/sqlx-sqlite-0.8.6 vendor/sqlx-sqlite
```

## Removal condition

Drop this fork when a sqlx release accepts `libsqlite3-sys` >=0.39.0. Remove
`vendor/sqlx-sqlite` and both `[patch.crates-io]` entries, upgrade sqlx, update
both lockfiles, and rerun both lockfile audits, `cargo deny` and the Core test
suite. Re-vendor instead whenever sqlx is bumped within 0.8, since the version
must match or Cargo drops the patch and falls back to the registry crate.
