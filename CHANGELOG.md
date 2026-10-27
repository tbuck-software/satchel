# Changelog

## Unreleased

- Merge Snoozed and Waiting into one Later view: snoozed mail by return time, mail waiting for a reply longest wait first.
- File mail into folders and topics from one File… picker (`v`, `t` still works), in place of Move and Add to topic.
- Take tags out of the mail interface; they stay in the backend and in rules.
- Make the mail keys work in every mail list (search page, topics, sender review) and give every view a `g` key.
- Keep your place after Undo: the restored mail and the selection come back in focus.
- Act on several selected mails from one bar with the common actions first.
- Schedule a send from a menu anchored at the Send button, to the minute.
- Pick dates and times to the minute, without a seconds column.
- Open a sender's or contact's mail in the list search instead of a separate page.
- Search the whole Inbox, and every other list, with the list's own filter instead of the first pages of hits.
- Offer newer stable releases to nightly users.
- Trim the startup bundle by loading native APIs and hidden inbox controls on demand.
- Patch the plugin cache to use fixed zstd dependencies and document a crypto-bigint audit exception through 2026-12-31 UTC.
- Rework the interface across inbox, reader, composer, search, contacts, settings, calendar and command palette: quieter collections, one reading column with drafts in place, recipients before subject, grouped settings, shared search fields and keycaps.
- Fix English labels showing German text, German weekdays in the week view, the sidebar highlighting the wrong page, and the calendar filter for local and CalDAV calendars.
- Update the plugin runtime to Wasmtime 36.0.17 and patch ammonia and quinn-proto security advisories.
- Bound SMTP response lines and complete replies in the vendored lettre fork.
- Apply the macOS sqlx macro linker workaround to local checks and CI.
- Guard additive migration versions and stable installs; verify upgrades from an older database.
- Restore mobile builds on relevant pull requests and nightly Android runtime checks.
- Keep fork pull requests off self-hosted runners and check dependency licenses and sources.
- Run the complete release gate and use the lockfile-installed Tauri CLI.
