ALTER TABLE messages ADD COLUMN category TEXT;
ALTER TABLE threads ADD COLUMN category TEXT;

CREATE INDEX IF NOT EXISTS idx_messages_category ON messages(category) WHERE category IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_threads_category ON threads(category) WHERE category IS NOT NULL;
