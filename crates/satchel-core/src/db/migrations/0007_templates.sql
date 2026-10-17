CREATE TABLE IF NOT EXISTS templates (
    id TEXT PRIMARY KEY,
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    subject TEXT,
    body_text TEXT NOT NULL DEFAULT '',
    body_html TEXT,
    shortcut TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_templates_account ON templates(account_id);
CREATE INDEX IF NOT EXISTS idx_templates_shortcut ON templates(shortcut) WHERE shortcut IS NOT NULL;
