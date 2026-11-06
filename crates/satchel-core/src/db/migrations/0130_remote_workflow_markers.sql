-- Done and Snooze on the mail server (ADR 0009): IMAP keywords, Gmail labels.
--
-- `messages.remote_workflow_marker` is what the server last reported for this
-- copy: NULL when the provider has not said (older rows, Outlook, POP3), ''
-- for no marker, 'done', or 'snoozed:<RFC 3339 wake time>'.
--
-- `threads.remote_workflow_marker` is the marker this device last pushed or
-- observed for the thread. Sync applies a server marker only when it differs
-- from this value, so a local state the server cannot express (Todo, Waiting)
-- is never reset by a server that simply says nothing.
ALTER TABLE messages ADD COLUMN remote_workflow_marker TEXT;
ALTER TABLE threads ADD COLUMN remote_workflow_marker TEXT;

-- Whether the account's server takes Satchel's keywords. 'server' once a
-- mailbox allowed them, 'local_only' when PERMANENTFLAGS lacks `\*` or a push
-- was refused for that reason. No row: not known yet.
CREATE TABLE IF NOT EXISTS account_workflow_sync (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    mode TEXT NOT NULL,
    checked_at TEXT NOT NULL
);
