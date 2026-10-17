# Changelog

## Unreleased

- Update the plugin runtime to Wasmtime 36.0.17 and patch ammonia and quinn-proto security advisories.
- Bound SMTP response lines and complete replies in the vendored lettre fork.
- Apply the macOS sqlx macro linker workaround to local checks and CI.
- Guard additive migration versions and stable installs; verify upgrades from an older database.
- Restore mobile builds on relevant pull requests and nightly Android runtime checks.
- Keep fork pull requests off self-hosted runners and check dependency licenses and sources.
- Run the complete release gate and use the lockfile-installed Tauri CLI.
