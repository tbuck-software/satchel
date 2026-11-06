-- The last mailbox quota a server reported, one row per account. Storage is in
-- bytes (IMAP reports units of 1024 octets), messages are a count; either pair
-- is NULL when the server has no such limit. `checked_at` is the last attempt
-- and paces the next one; `fetched_at` is the last successful read, so a failed
-- attempt keeps the previous numbers instead of wiping them.
CREATE TABLE IF NOT EXISTS account_quota (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    storage_used_bytes INTEGER,
    storage_limit_bytes INTEGER,
    messages_used INTEGER,
    messages_limit INTEGER,
    fetched_at TEXT,
    checked_at TEXT NOT NULL
);
