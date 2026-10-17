-- Files dropped directly into a topic ("Können wir da auch selbst Dateien
-- reinwerfen?") — not tied to any mail. Content lives as a BLOB in the DB:
-- these are working documents, they belong in the local-first store and in
-- every backup, and no separate file management is needed.
CREATE TABLE IF NOT EXISTS topic_files (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    size INTEGER NOT NULL DEFAULT 0,
    content BLOB NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_topic_files_topic ON topic_files(topic_id, created_at);
