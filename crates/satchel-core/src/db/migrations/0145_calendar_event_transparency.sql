-- Whether an event leaves its time free: Google's `transparency`, Graph's
-- `showAs` and iCalendar's TRANSP. Free/busy and "insert my availability"
-- skip these events. Every existing row reads as busy until its calendar's
-- next sync rewrites it, which is what free/busy assumed before.
ALTER TABLE calendar_events ADD COLUMN transparent INTEGER NOT NULL DEFAULT 0;
