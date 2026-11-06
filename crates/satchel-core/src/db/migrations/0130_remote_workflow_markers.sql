-- Done and Snooze on the mail server (ADR 0009): IMAP keywords, Gmail labels.
--
-- `messages.remote_workflow_marker` is what the server last reported for this
-- copy: NULL for no marker (so a mailbox without markers is never written),
-- 'done', or 'snoozed:<generation>:<RFC 3339 wake time, empty on IMAP>'.
--
-- `threads.remote_workflow_marker` is the marker the server last accepted
-- from this device or showed to it. Sync applies a server marker only when it
-- differs from this value, so a local state the server cannot express (Todo,
-- Waiting) is never reset by a server that simply says nothing.
ALTER TABLE messages ADD COLUMN remote_workflow_marker TEXT;
ALTER TABLE threads ADD COLUMN remote_workflow_marker TEXT;

-- Whether the account's server takes Satchel's keywords, decided by INBOX.
-- 'server', or 'local_only' with a `reason`: 'no_custom_keywords' (INBOX
-- PERMANENTFLAGS without `\*`; a later snapshot with `\*` lifts it),
-- 'gmail_imap' (Gmail's IMAP host) or 'store_refused' (INBOX answered a
-- keyword STORE with a definite NO/BAD; holds for some hours and until the
-- app restarts, an accepted INBOX store lifts it). `baseline_at`: the first
-- completed INBOX snapshot on this device; before it, IMAP snoozes set
-- elsewhere are recorded but not adopted. No row: not known yet.
CREATE TABLE IF NOT EXISTS account_workflow_sync (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    mode TEXT NOT NULL,
    reason TEXT,
    checked_at TEXT NOT NULL,
    baseline_at TEXT
);
