-- Flag-only searches start from the thread projection, then select a single
-- representative message. Cover the unread selector and the thread ordering
-- so large mailboxes do not scan and rank every message.
CREATE INDEX IF NOT EXISTS idx_messages_thread_read_date
ON messages(thread_id, is_read, date DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_threads_unread_date
ON threads(last_message_date DESC, id DESC)
WHERE unread_count > 0;

CREATE INDEX IF NOT EXISTS idx_threads_attachments_date
ON threads(last_message_date DESC, id DESC)
WHERE has_attachments = 1;

CREATE INDEX IF NOT EXISTS idx_threads_starred_date
ON threads(last_message_date DESC, id DESC)
WHERE is_starred = 1;
