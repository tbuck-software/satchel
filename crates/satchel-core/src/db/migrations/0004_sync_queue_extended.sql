ALTER TABLE sync_queue ADD COLUMN next_retry_at TEXT;
ALTER TABLE sync_queue ADD COLUMN scheduled_for TEXT;
ALTER TABLE sync_queue ADD COLUMN updated_at_ms INTEGER;

CREATE INDEX IF NOT EXISTS idx_sync_queue_status_due ON sync_queue(status, next_retry_at);
CREATE INDEX IF NOT EXISTS idx_sync_queue_account ON sync_queue(account_id);
CREATE INDEX IF NOT EXISTS idx_sync_queue_scheduled ON sync_queue(scheduled_for) WHERE scheduled_for IS NOT NULL;
