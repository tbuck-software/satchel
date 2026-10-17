ALTER TABLE accounts ADD COLUMN sync_status TEXT NOT NULL DEFAULT 'idle';
ALTER TABLE accounts ADD COLUMN last_sync_error TEXT;
ALTER TABLE accounts ADD COLUMN last_sync_error_at TEXT;
ALTER TABLE accounts ADD COLUMN sync_progress_json TEXT;

CREATE INDEX IF NOT EXISTS idx_accounts_sync_status ON accounts(sync_status);
