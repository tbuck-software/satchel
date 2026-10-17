ALTER TABLE folders ADD COLUMN absent_runs INTEGER NOT NULL DEFAULT 0;
ALTER TABLE attachments ADD COLUMN content_base64 TEXT;

CREATE TABLE ingest_effects (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    run_rules INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_ingest_effects_account ON ingest_effects(account_id, created_at);
