-- Where an incremental provider sync of one calendar resumes: the Microsoft
-- Graph delta link of an Outlook calendar. NULL enumerates the calendar from
-- scratch on the next sync, which is also what every existing row means.
ALTER TABLE calendar_sources ADD COLUMN sync_cursor TEXT;
