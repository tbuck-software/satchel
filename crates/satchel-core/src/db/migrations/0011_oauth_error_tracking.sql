ALTER TABLE accounts ADD COLUMN last_oauth_error TEXT;
ALTER TABLE accounts ADD COLUMN last_oauth_error_at TEXT;
ALTER TABLE accounts ADD COLUMN last_sync_at TEXT;

CREATE INDEX IF NOT EXISTS idx_accounts_oauth_error ON accounts(last_oauth_error) WHERE last_oauth_error IS NOT NULL;
