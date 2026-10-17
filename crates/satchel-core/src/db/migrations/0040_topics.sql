-- Topics: cross-source collections ("Themen" in the UI). A topic groups email
-- threads, individual messages and standalone notes about one ongoing matter,
-- similar to an issue in a tracker. Links are n:m because a single email often
-- carries information for more than one topic.

CREATE TABLE IF NOT EXISTS topics (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    color TEXT,
    icon TEXT,
    status TEXT NOT NULL DEFAULT 'active',   -- 'active' | 'done' | 'archived'
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Polymorphic n:m links. item_id has no FK on purpose: it points at threads,
-- messages, or (future) calendar_events depending on item_type. Cleanup of
-- orphaned links happens in the delete paths and the maintenance sweep.
CREATE TABLE IF NOT EXISTS topic_items (
    topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL,                 -- 'thread' | 'message' | 'event' (event: schema only)
    item_id TEXT NOT NULL,
    added_at TEXT NOT NULL,
    added_by TEXT NOT NULL DEFAULT 'user',   -- 'user' | 'rule' | 'llm'
    PRIMARY KEY (topic_id, item_type, item_id)
);
CREATE INDEX IF NOT EXISTS idx_topic_items_item ON topic_items(item_type, item_id);

-- Standalone notes, each its own timeline entry with its own timestamp
-- (deliberately NOT one text blob per topic; the topic-level overview lives
-- in topics.description).
CREATE TABLE IF NOT EXISTS topic_notes (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    content TEXT NOT NULL,                   -- markdown
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_topic_notes_topic ON topic_notes(topic_id, created_at);
