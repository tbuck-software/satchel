-- Persist remote calendar metadata independently from events. A calendar can
-- be visible even when it has no events inside the local sync window.
CREATE TABLE IF NOT EXISTS calendar_sources (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    name TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, source, remote_id)
);

CREATE INDEX IF NOT EXISTS idx_calendar_sources_account
    ON calendar_sources(account_id, source);
