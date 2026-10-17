-- Restore indexes that shipped in the removed migrations 25 and 26 so new
-- installations retain the same thread-list and mailbox-count performance.
CREATE INDEX IF NOT EXISTS idx_messages_thread_folder
ON messages(thread_id, folder_id);

CREATE INDEX IF NOT EXISTS idx_messages_folder_thread_read
ON messages(folder_id, thread_id, is_read);

CREATE INDEX IF NOT EXISTS idx_messages_account_remote
ON messages(account_id, remote_id);
