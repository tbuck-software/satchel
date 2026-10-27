# Satchel Wasmtime cache patch

Satchel vendors the crates.io release of `wasmtime-internal-cache` 36.0.19 to
retain its plugin compilation disk cache while using patched zstd crates.
Wasmtime releases through 50.0.0 still require zstd 0.13 in their cache crate.
The upstream archive's SHA-256 is
`4108e1964b3a02b4b5604ea1a2918a0cbcab512e5de0ccf4c062fc6c06b2eb8e`,
matching the registry entry removed from Satchel's lockfile by this patch.

## Diff from the registry release

Only `[dependencies.zstd].version` in the normalized `Cargo.toml` changes,
from `0.13.0` to `0.14.3`. `default-features = false` remains in place.
`Cargo.toml.orig` preserves the original upstream workspace manifest as a
reference; Cargo builds this fork from the normalized `Cargo.toml`.

The source, build script, tests and Apache-2.0 WITH LLVM-exception license are
unchanged. The registry's standalone `Cargo.lock` and Cargo registry bookkeeping
files are omitted. Satchel resolves this path dependency in its root lockfile.

The lockfile selects zstd 0.14.3, zstd-safe 8.2.0 and zstd-sys 2.1.2+zstd.1.5.7.
They fix RUSTSEC-2026-0391, RUSTSEC-2026-0398, RUSTSEC-2026-0403 and
RUSTSEC-2026-0410, plus the zstd-safe unsoundness advisories reported against
7.2.4. No zstd advisory is ignored.

The cache only calls `zstd::encode_all`, `zstd::decode_all` and
`zstd::DEFAULT_COMPRESSION_LEVEL`. These APIs remain available in 0.14.3.
The zstd frame format remains compatible. Wasmtime's cache key and compiled
component validation code is unchanged. The unchanged upstream build script now
runs inside Satchel's checkout and uses its Git revision for the compiler cache
directory, so existing installations may recompile once under a new directory.
The existing
`plugins::runtime::compilation_cache_tests::unchanged_components_hit_the_disk_cache_across_engines`
test exercises a disk-cache miss, a hit from a new engine and a changed component.

To compare against the upstream archive:

```sh
curl -fsSL -o /tmp/wasmtime-internal-cache-36.0.19.crate \
  https://static.crates.io/crates/wasmtime-internal-cache/wasmtime-internal-cache-36.0.19.crate
mkdir -p /tmp/satchel-wasmtime-cache-upstream
tar xzf /tmp/wasmtime-internal-cache-36.0.19.crate -C /tmp/satchel-wasmtime-cache-upstream
diff -ru /tmp/satchel-wasmtime-cache-upstream/wasmtime-internal-cache-36.0.19 \
  vendor/wasmtime-internal-cache
```

## Removal condition

Drop this fork when Satchel upgrades to a Wasmtime release whose cache crate
requires zstd >=0.14.3 and resolves zstd-safe >=8.2.0 and zstd-sys >=2.1.2.
Remove `vendor/wasmtime-internal-cache` and its root `[patch.crates-io]` entry,
update the lockfile, and rerun both lockfile audits, dependency policy checks
and the plugin runtime tests with built components. Keep the compilation cache
enabled throughout the upgrade.
