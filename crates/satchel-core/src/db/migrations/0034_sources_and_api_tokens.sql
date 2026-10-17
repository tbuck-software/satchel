ALTER TABLE accounts ADD COLUMN kind TEXT NOT NULL DEFAULT 'email';
ALTER TABLE messages ADD COLUMN source_payload TEXT;
CREATE INDEX IF NOT EXISTS idx_accounts_kind ON accounts(kind, is_active);

CREATE TABLE api_tokens (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    profile TEXT NOT NULL,
    scopes TEXT NOT NULL,
    source_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_api_tokens_source ON api_tokens(source_id);

CREATE TABLE api_audit_log (
    id TEXT PRIMARY KEY,
    token_id TEXT,
    method TEXT NOT NULL,
    decision TEXT NOT NULL,
    detail TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_api_audit_created ON api_audit_log(created_at DESC);
