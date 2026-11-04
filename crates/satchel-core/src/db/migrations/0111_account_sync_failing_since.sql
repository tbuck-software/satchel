-- When an account's current run of failed syncs began. `last_sync_error_at`
-- moves with every attempt; this one stays put until a sync succeeds, so the
-- UI can say "has not synced since Wednesday" instead of "failed just now".
ALTER TABLE accounts ADD COLUMN sync_failing_since TEXT;
