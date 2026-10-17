-- The Inbox repeatedly asks for the latest non-draft message in every thread.
-- Cover its complete ordering so SQLite can stop at the first matching row
-- instead of sorting each thread's messages for every projected field.
CREATE INDEX IF NOT EXISTS idx_messages_thread_latest
ON messages(thread_id, is_draft, date DESC, id DESC);

-- Match the keyset pagination order, including its deterministic id tie-break.
CREATE INDEX IF NOT EXISTS idx_threads_date_id
ON threads(last_message_date DESC, id DESC);
