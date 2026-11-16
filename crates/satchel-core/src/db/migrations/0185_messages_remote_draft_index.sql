-- Ingest looks up a server draft's local row by its stable draft id inside the
-- write transaction. Without an index that scans the account's messages and
-- holds the writer lock for seconds on large mailboxes.
CREATE INDEX IF NOT EXISTS idx_messages_account_remote_draft
ON messages(account_id, remote_draft_id)
WHERE remote_draft_id IS NOT NULL;
