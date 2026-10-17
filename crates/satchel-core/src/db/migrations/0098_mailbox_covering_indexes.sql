-- Reconcile reads identifiers and flags without visiting body pages.
CREATE INDEX idx_messages_imap_reconcile
ON messages(account_id, folder_id, id, thread_id, remote_id, is_read, is_starred);

-- The unified Inbox pages by date; sparse folders and contacts start from membership.
CREATE INDEX idx_threads_inbox_date_id
ON threads(last_message_date DESC, id DESC, account_id, workflow_state)
WHERE workflow_state IN ('inbox', 'todo');
