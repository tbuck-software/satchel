# Changelog

## Unreleased

- Drag a mail from the list onto the composer to attach the original as an `.eml` file, one per selected conversation.
- File mail from the keyboard: `v` focuses the File… picker, Enter files into the highlighted folder or topic, and the footer says what Enter will do.
- Keep a thread's draft card in step with the composer: it shows the saved text and time after closing, and steps aside while the draft is open. Rows mark drafts with "Draft," and a pencil, and counts leave the draft out.
- Show a multi-selection as what it holds: the reader lists the selected conversations instead of one of them under "Done (n)". Selected rows show a check.
- Place receipts beside the sidebar and, in the split inbox, at the reader instead of over the list; no queue toasts on the Outbox itself; undo reads "Marked as done · Undo"; "Saved as draft" offers Continue editing.
- Say which account rests and until when: "Work resting" in the list header, "Resting" in its row, a clock while it is shown until midnight, and a menu that leads with its state.
- Read your own unformatted replies as text in the app's type, without the mail frame or its view switch.
- Mark list search hits, show the inbox's unread count beside its scope, and keep a selected row's time and pin visible.
- Set quoted history in the composer as reference, drop the second view of a conversation the quote already holds, and mark the focused field.
- Mark timed calendar events with a dot and all-day events with a tinted bar, and straighten the edge of week blocks.
- On a phone, show failed items as a banner with Review issues, and fade the reader's last line above the action bar.

- Show each account in the sidebar with its unread Inbox count and an avatar: its initial on its colour, or a picture you chose; All accounts shows the visible accounts' colours. Change an account's colour from the sidebar's right-click menu or in its settings, and new accounts start in a colour no other account wears.
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
