# Changelog

## Unreleased

- Rework the interface across inbox, reader, composer, search, contacts, settings, calendar and command palette: quieter collections, one reading column with drafts in place, recipients before subject, grouped settings, shared search fields and keycaps.
- Fix English labels showing German text, German weekdays in the week view, the sidebar highlighting the wrong page, and the calendar filter for local and CalDAV calendars.
- Update the plugin runtime to Wasmtime 36.0.17 and patch ammonia and quinn-proto security advisories.
- Bound SMTP response lines and complete replies in the vendored lettre fork.
- Apply the macOS sqlx macro linker workaround to local checks and CI.
- Guard additive migration versions and stable installs; verify upgrades from an older database.
- Restore mobile builds on relevant pull requests and nightly Android runtime checks.
- Keep fork pull requests off self-hosted runners and check dependency licenses and sources.
- Run the complete release gate and use the lockfile-installed Tauri CLI.
