-- Whether an event leaves its time free: Google's `transparency`, Graph's
-- `showAs` and iCalendar's TRANSP. Free/busy and "insert my availability"
-- skip these events. Every existing row reads as busy until its calendar's
-- next sync rewrites it, which is what free/busy assumed before.
ALTER TABLE calendar_events ADD COLUMN transparent INTEGER NOT NULL DEFAULT 0;

-- Outlook calendars resume from a delta link, so rows mirrored before this
-- column would keep reading as busy until the delta window restarts. The
-- `outlook_calendar_resync` repair forgets their cursors once; Google and
-- CalDAV mirrors are listed whole on every sync already.
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.outlook_calendar_resync', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
