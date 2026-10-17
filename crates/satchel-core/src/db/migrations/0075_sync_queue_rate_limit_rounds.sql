-- A rate-limited action deliberately does not consume a retry, so retry_count
-- cannot bound it. Without a bound of its own it circulates forever, never
-- reaches 'failed', and therefore never appears in any failure count.
ALTER TABLE sync_queue ADD COLUMN rate_limit_rounds INTEGER NOT NULL DEFAULT 0;
