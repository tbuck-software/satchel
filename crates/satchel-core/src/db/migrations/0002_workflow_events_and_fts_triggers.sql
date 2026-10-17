CREATE TABLE IF NOT EXISTS workflow_events (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id),
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workflow_events_thread ON workflow_events(thread_id);
CREATE INDEX IF NOT EXISTS idx_workflow_events_created ON workflow_events(created_at DESC);

CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address);
END;

CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, body_text, from_name, from_address)
    VALUES ('delete', old.rowid, old.subject, old.body_text, old.from_name, old.from_address);
    INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address)
    VALUES (new.rowid, new.subject, new.body_text, new.from_name, new.from_address);
END;

INSERT INTO messages_fts(rowid, subject, body_text, from_name, from_address)
SELECT rowid, subject, body_text, from_name, from_address
FROM messages
WHERE rowid NOT IN (SELECT rowid FROM messages_fts);
