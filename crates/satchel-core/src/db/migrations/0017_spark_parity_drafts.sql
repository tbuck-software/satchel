ALTER TABLE messages ADD COLUMN scheduled_for TEXT;
ALTER TABLE messages ADD COLUMN remote_draft_id TEXT;
ALTER TABLE messages ADD COLUMN draft_attachments_json TEXT;

CREATE INDEX IF NOT EXISTS idx_messages_scheduled_for ON messages(scheduled_for) WHERE scheduled_for IS NOT NULL;

UPDATE threads
SET workflow_state = 'done', board_column = 'done'
WHERE workflow_state = 'archived';
