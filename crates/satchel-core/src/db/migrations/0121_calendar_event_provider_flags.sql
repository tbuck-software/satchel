-- What a provider says about one mirrored event, so the calendar can decline
-- an edit before sending it: read_only for events the account may not change
-- (read-only calendars, meetings it only attends), recurrence_instance for one
-- occurrence of a series, rich_description for a formatted body the plain
-- text editor would flatten. Every existing row keeps today's behaviour.
ALTER TABLE calendar_events ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0;
ALTER TABLE calendar_events ADD COLUMN recurrence_instance INTEGER NOT NULL DEFAULT 0;
ALTER TABLE calendar_events ADD COLUMN rich_description INTEGER NOT NULL DEFAULT 0;
