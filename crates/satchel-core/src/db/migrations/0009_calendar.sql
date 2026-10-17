CREATE TABLE IF NOT EXISTS calendar_invites (
    id TEXT PRIMARY KEY,
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    organizer_email TEXT,
    organizer_name TEXT,
    summary TEXT,
    description TEXT,
    location TEXT,
    starts_at TEXT,
    ends_at TEXT,
    all_day INTEGER NOT NULL DEFAULT 0,
    rsvp_status TEXT NOT NULL DEFAULT 'needs-action',
    raw_ics TEXT NOT NULL,
    sequence INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_invites_account ON calendar_invites(account_id);
CREATE INDEX IF NOT EXISTS idx_invites_message ON calendar_invites(message_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_invites_uid ON calendar_invites(account_id, uid);
