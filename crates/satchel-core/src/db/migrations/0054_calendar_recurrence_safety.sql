-- Keep a recurring master and its overridden instances independently. An
-- empty recurrence id denotes the master event and makes the uniqueness
-- constraint deterministic (SQLite permits multiple NULL values otherwise).
ALTER TABLE calendar_invites ADD COLUMN recurrence_id TEXT NOT NULL DEFAULT '';
ALTER TABLE calendar_invites ADD COLUMN recurring INTEGER NOT NULL DEFAULT 0;
ALTER TABLE calendar_invites ADD COLUMN recurrence_unbounded INTEGER NOT NULL DEFAULT 0;

DROP INDEX IF EXISTS idx_invites_uid;
CREATE UNIQUE INDEX idx_invites_uid
    ON calendar_invites(account_id, uid, recurrence_id);

-- Existing rows are classified precisely in Rust whenever they are read. This
-- backfill gives older databases a useful value before their next mail sync;
-- starting at VEVENT avoids mistaking VTIMEZONE transition RRULEs for a series.
UPDATE calendar_invites
SET recurring = CASE
        WHEN instr(upper(substr(raw_ics, instr(upper(raw_ics), 'BEGIN:VEVENT'))), 'RRULE:') > 0
          OR instr(upper(substr(raw_ics, instr(upper(raw_ics), 'BEGIN:VEVENT'))), 'RDATE') > 0
          OR instr(upper(substr(raw_ics, instr(upper(raw_ics), 'BEGIN:VEVENT'))), 'RECURRENCE-ID') > 0
        THEN 1 ELSE 0 END,
    recurrence_unbounded = CASE
        WHEN instr(upper(substr(raw_ics, instr(upper(raw_ics), 'BEGIN:VEVENT'))), 'RRULE:') > 0
         AND instr(upper(substr(raw_ics, instr(upper(raw_ics), 'BEGIN:VEVENT'))), 'COUNT=') = 0
         AND instr(upper(substr(raw_ics, instr(upper(raw_ics), 'BEGIN:VEVENT'))), 'UNTIL=') = 0
        THEN 1 ELSE 0 END;
