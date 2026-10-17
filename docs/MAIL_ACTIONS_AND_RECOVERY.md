# Mail actions and recovery

Commands and notification callbacks call `satchel_core::mail_actions`. The module
selects message copies, records provider intent and updates the local projection
in one SQLite write transaction. Archive excludes Sent, Drafts, Trash and existing
Archive copies from both the queued work and the local change. Explicit folder
moves validate the destination account within that transaction.

`messages.folder_id` and `remote_id` describe the confirmed provider identity.
`pending_folder_id` is a presentation override while a move is queued. Mailbox
lists use the pending destination; provider operations continue to resolve the
confirmed source. Follow-up flags and moves are rejected until reconciliation
finishes. Renaming or deleting a folder cannot invalidate a queued move; these
checks and folder mutations share a write transaction. Deactivating an account
clears pending destinations together with its discarded queue. IMAP moves use the existing workflow executor's UIDVALIDITY snapshots,
completion markers and exact destination reconciliation. Providers with stable
message IDs clear the pending destination after applying the confirmed move.

The Core queue worker runs at most one action per account and bounds concurrency
across accounts. Its Tauri adapter tries an account's sync permit without waiting.
A busy account therefore does not occupy a worker slot. Pending mutations keep
insertion order across retries; due sends receive priority. Retry classification
and terminal delivery bookkeeping live in Core and are exercised by the real
worker tests, with a fake provider executor.

The frontend persistence layer serializes optimistic flag/category changes and
restores their previous fields when native persistence fails. Browser fixture
mode keeps its explicit local fallback. UI actions turn failures into an error
toast and return a success indicator; bulk success feedback depends on that
result.

## Restore files

Restore preparation creates a consistent SQLite snapshot, checks integrity and
rewraps encrypted account configs, PGP keys/passphrases, S/MIME private keys and
OAuth verifiers under the destination's active key. It does not replace the OS
keychain key of the destination profile. Backup export includes the active key even
when the installation has no fallback key file. Automatic macOS install/import
backups keep a separate Keychain snapshot and record its account in
`master-key.keychain`. Restore reads that snapshot without changing the active
key. These backups require the original Mac's Keychain; portable backups use a
passphrase-wrapped `pgp-master-key-v1` export. Existing plaintext keys remain a
file fallback for automatic backups if Keychain is unavailable.

A prepared restore consists of `satchel.db.restore-pending` and
`satchel.restore-ready.json`. The manifest records the database hash and, when
protected fields exist, the destination key fingerprint. A partial file without
this manifest is never activated.

Before opening the pool, activation checks both fingerprints and preserves the
previous database and its WAL/SHM in a `satchel.db.replaced-*` directory. These
files are a startup rollback image; normal user backups still use `VACUUM INTO`.
An applying journal is synced before sidecars are removed and the database is
atomically replaced. Recovery uses the consumption of the staging file as well
as its hash to distinguish an interrupted swap from a completed one. Before the
swap it restores the previous files; after the swap it completes cleanup.

An invalid prepared restore leaves the existing profile available for cancellation
or restaging. Failure to recover an interrupted swap prevents opening the pool.
