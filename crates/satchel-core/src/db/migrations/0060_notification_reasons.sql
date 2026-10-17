-- Every "no notification" decision leaves a row behind.
--
-- The 0057 trigger encoded its filters in a WHEN clause, so a message that
-- failed any of them produced nothing at all: no row, no log line, no way to
-- tell "Satchel decided against it" from "Satchel never saw it". The rebuilt
-- trigger computes a reason instead of a boolean and writes it to
-- `last_error` as `<reason_code>: <detail>`, with state 'dismissed'.
--
-- One exception, marked `__skip__` below: mail that predates the account's
-- watermark by more than a day is backfill, and a row per backfilled message
-- would put a second mailbox-sized table next to the mailbox. Mail that missed
-- the watermark by less than a day is exactly the interesting case — a sender
-- clock a few minutes behind, or a relay that held the message across a sync
-- run — so that keeps its row.

DROP TRIGGER IF EXISTS messages_notification_outbox_ai;

CREATE TRIGGER messages_notification_outbox_ai
AFTER INSERT ON messages
BEGIN
    INSERT OR IGNORE INTO notification_outbox (message_id, state, created_at, last_error)
    SELECT
        new.id,
        CASE WHEN reason IS NULL THEN 'pending' ELSE 'dismissed' END,
        datetime('now'),
        reason
    FROM (
        SELECT CASE
            WHEN julianday(new.date) IS NOT NULL
                 AND julianday(new.date) < julianday(watermark, '-1 day')
                THEN '__skip__'
            WHEN COALESCE(new.is_sent, 0) <> 0
                THEN 'filter_own_sent: is_sent'
            WHEN COALESCE(new.is_draft, 0) <> 0
                THEN 'filter_own_sent: is_draft'
            WHEN NOT EXISTS (
                    SELECT 1 FROM accounts a
                    WHERE a.id = new.account_id
                      AND a.kind = 'email'
                      AND a.is_active = 1)
                THEN 'filter_account_inactive: ' || new.account_id
            WHEN NOT EXISTS (
                    SELECT 1 FROM folders f
                    WHERE f.id = new.folder_id
                      AND f.account_id = new.account_id
                      AND LOWER(COALESCE(f.folder_type, '')) = 'inbox')
                THEN 'filter_not_inbox: ' || COALESCE(
                        (SELECT COALESCE(NULLIF(f.folder_type, ''), f.full_path)
                         FROM folders f WHERE f.id = new.folder_id),
                        new.folder_id)
            WHEN EXISTS (
                    SELECT 1 FROM accounts a
                    WHERE a.id = new.account_id
                      AND LOWER(TRIM(new.from_address)) = LOWER(TRIM(a.email)))
                THEN 'filter_own_sent: ' || new.from_address
            WHEN COALESCE((
                    SELECT LOWER(TRIM(value)) FROM app_settings
                    WHERE key = 'notifications.enabled'), 'true')
                 NOT IN ('1', 'true', 'yes')
                THEN 'setting_disabled: notifications.enabled'
            WHEN COALESCE((
                    SELECT LOWER(TRIM(value)) FROM app_settings
                    WHERE key = 'notifications.scope'), 'inbox') = 'off'
                THEN 'setting_disabled: notifications.scope=off'
            WHEN julianday(new.date) IS NULL
                THEN 'filter_no_date: ' || COALESCE(new.date, '(leer)')
            WHEN julianday(new.date) < julianday(watermark)
                THEN 'filter_too_old: ' || COALESCE(new.date, '') || ' < ' || watermark
            ELSE NULL
        END AS reason
        FROM (
            SELECT COALESCE(
                NULLIF((SELECT a.last_sync_at FROM accounts a WHERE a.id = new.account_id), ''),
                datetime('now', '-15 minutes')
            ) AS watermark
        )
    )
    WHERE reason IS NULL OR reason <> '__skip__';
END;

-- Serves the retention trim, the state counts and the reason histogram; the
-- 0057 partial index only covers pending rows.
CREATE INDEX IF NOT EXISTS idx_notification_outbox_state_created
    ON notification_outbox(state, created_at);
