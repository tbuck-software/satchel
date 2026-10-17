CREATE TABLE IF NOT EXISTS signatures (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    body_text TEXT NOT NULL DEFAULT '',
    body_html TEXT,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_signatures_account ON signatures(account_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_signatures_account_default ON signatures(account_id) WHERE is_default = 1;
