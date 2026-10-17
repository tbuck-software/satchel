-- Remote calendar metadata for CalDAV-backed events. NULL values preserve all
-- existing local/ICS-only rows without changing their behaviour.
ALTER TABLE calendar_events ADD COLUMN remote_href TEXT;
ALTER TABLE calendar_events ADD COLUMN remote_etag TEXT;
ALTER TABLE calendar_events ADD COLUMN collection_href TEXT;
ALTER TABLE calendar_events ADD COLUMN raw_ics TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_calendar_events_remote
    ON calendar_events(account_id, source, remote_href);

