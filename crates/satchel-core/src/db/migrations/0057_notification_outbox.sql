CREATE TABLE notification_outbox (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'dispatching', 'delivered', 'dismissed')),
    created_at TEXT NOT NULL,
    dispatch_token TEXT,
    dispatch_at TEXT,
    delivered_at TEXT,
    last_error TEXT
);

CREATE INDEX idx_notification_outbox_pending
    ON notification_outbox(created_at)
    WHERE state = 'pending';

-- WorkManager runs in a dedicated Android process. A renewable database lease
-- complements the in-process mutex so an Activity restart cannot race a
-- headless worker on the same provider cursor. A crashed owner expires without
-- manual repair.
CREATE TABLE account_sync_leases (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    owner_token TEXT NOT NULL,
    lease_expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Notification intent is committed in the same SQLite transaction as the
-- message row. A process death after ingestion therefore cannot leave a new
-- Inbox message in the database without a durable delivery record.
--
-- `accounts.last_sync_at` still contains the previous successful provider
-- snapshot while messages for the current run are inserted. That distinguishes
-- delayed catch-up from initial backfill. Once an account has completed a sync,
-- its previous successful snapshot is the watermark; there is deliberately no
-- fixed age window that could silently discard mail after a long sleep.
CREATE TRIGGER messages_notification_outbox_ai
AFTER INSERT ON messages
WHEN COALESCE(new.is_sent, 0) = 0
 AND COALESCE(new.is_draft, 0) = 0
 AND EXISTS (
     SELECT 1
     FROM accounts a
     JOIN folders f ON f.id = new.folder_id AND f.account_id = a.id
     WHERE a.id = new.account_id
       AND a.kind = 'email'
       AND a.is_active = 1
       AND LOWER(COALESCE(f.folder_type, '')) = 'inbox'
       AND LOWER(TRIM(new.from_address)) <> LOWER(TRIM(a.email))
       AND COALESCE((
           SELECT LOWER(TRIM(value)) FROM app_settings
           WHERE key = 'notifications.enabled'
       ), 'true') IN ('1', 'true', 'yes')
       AND COALESCE((
           SELECT LOWER(TRIM(value)) FROM app_settings
           WHERE key = 'notifications.scope'
       ), 'inbox') <> 'off'
       AND julianday(new.date) IS NOT NULL
       AND julianday(new.date) >= julianday(
           CASE
               WHEN a.last_sync_at IS NULL THEN datetime('now', '-15 minutes')
               ELSE a.last_sync_at
           END
       )
 )
BEGIN
    INSERT OR IGNORE INTO notification_outbox (message_id, state, created_at)
    VALUES (new.id, 'pending', datetime('now'));
END;
