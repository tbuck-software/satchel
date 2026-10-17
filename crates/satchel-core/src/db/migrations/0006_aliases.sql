CREATE TABLE IF NOT EXISTS account_aliases (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    display_name TEXT,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_aliases_account ON account_aliases(account_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aliases_account_email ON account_aliases(account_id, email);
