ALTER TABLE threads ADD COLUMN assignee TEXT;
ALTER TABLE threads ADD COLUMN shared_with TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS idx_thread_tags_tag_id ON thread_tags(tag_id, thread_id);
