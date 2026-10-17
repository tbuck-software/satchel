-- Follow-up reminders for sent mail ("remind me if no reply").
CREATE TABLE IF NOT EXISTS follow_up_reminders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
    due_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    resolved_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_followups_due ON follow_up_reminders(status, due_at);
CREATE INDEX IF NOT EXISTS idx_followups_thread ON follow_up_reminders(thread_id);

-- Snippet parity: templates may inject recipients and attachments, not just body text.
ALTER TABLE templates ADD COLUMN cc_json TEXT;
ALTER TABLE templates ADD COLUMN bcc_json TEXT;
ALTER TABLE templates ADD COLUMN attachments_json TEXT;

-- Local calendar events created inside FlowMail. Invite emails still travel as ICS.
CREATE TABLE IF NOT EXISTS calendar_events (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    uid TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    location TEXT,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    all_day INTEGER NOT NULL DEFAULT 0,
    attendees_json TEXT NOT NULL DEFAULT '[]',
    source TEXT NOT NULL DEFAULT 'local',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_calendar_events_account_start ON calendar_events(account_id, starts_at);

-- Provider-specific metadata surfaced as first-class local objects.
CREATE TABLE IF NOT EXISTS provider_labels (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    name TEXT NOT NULL,
    color TEXT,
    kind TEXT,
    visible INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,
    UNIQUE(account_id, remote_id)
);
CREATE INDEX IF NOT EXISTS idx_provider_labels_account ON provider_labels(account_id);
