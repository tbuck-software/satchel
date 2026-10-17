-- Pin/Star (separat von priority — kein Workflow-Change)
ALTER TABLE threads ADD COLUMN is_starred INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_threads_starred ON threads(is_starred) WHERE is_starred = 1;

-- Block Sender List (per account or global)
CREATE TABLE IF NOT EXISTS blocked_senders (
    id TEXT PRIMARY KEY,
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    pattern TEXT NOT NULL,
    pattern_type TEXT NOT NULL DEFAULT 'address',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_blocked_account ON blocked_senders(account_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_blocked_pattern ON blocked_senders(COALESCE(account_id, ''), pattern);

-- Smart Folders (saved searches)
CREATE TABLE IF NOT EXISTS smart_folders (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    icon TEXT,
    query TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Vacation / Out-of-Office settings (per account)
CREATE TABLE IF NOT EXISTS vacation_settings (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 0,
    subject TEXT,
    body TEXT NOT NULL DEFAULT '',
    starts_at TEXT,
    ends_at TEXT,
    only_contacts INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

-- List-Unsubscribe header tracking (for auto-unsubscribe UI)
ALTER TABLE messages ADD COLUMN list_unsubscribe TEXT;
ALTER TABLE messages ADD COLUMN list_unsubscribe_post TEXT;

-- Cached attachment storage path (after first download)
ALTER TABLE attachments ADD COLUMN cached_path TEXT;
ALTER TABLE attachments ADD COLUMN remote_uid TEXT;
ALTER TABLE attachments ADD COLUMN remote_part_id TEXT;
