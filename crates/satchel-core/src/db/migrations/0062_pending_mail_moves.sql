-- Presentation may show a queued destination, but provider identities remain
-- scoped to the confirmed folder until reconciliation installs the new UID.
ALTER TABLE messages ADD COLUMN pending_folder_id TEXT REFERENCES folders(id);
CREATE INDEX idx_messages_pending_folder ON messages(pending_folder_id)
WHERE pending_folder_id IS NOT NULL;
