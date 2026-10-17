-- Remember that an attachment has been examined for a calendar invite.
--
-- The startup backfill selected every .ics attachment without a row in
-- calendar_invites and re-fetched it from the provider on every launch. An
-- attachment that yields no invite — unparseable, or gone from the server —
-- never gets such a row, so it was downloaded again on every start, forever.
-- That is what made a rate-limited provider the first thing the app hit after
-- booting. The marker makes the pass converge: looked at once, not once a run.
ALTER TABLE attachments ADD COLUMN calendar_scan_at TEXT;

CREATE INDEX IF NOT EXISTS idx_attachments_calendar_scan
ON attachments(calendar_scan_at)
WHERE calendar_scan_at IS NULL;
