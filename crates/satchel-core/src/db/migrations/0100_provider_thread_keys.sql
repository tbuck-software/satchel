-- Bind a lossless provider identity to a surviving message, rather than to a
-- thread ID. Thread merges then follow the message without rewriting aliases.
CREATE TABLE provider_thread_keys (
    identity TEXT PRIMARY KEY NOT NULL,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE
);
CREATE INDEX idx_provider_thread_keys_message ON provider_thread_keys(message_id);

-- Keep an adopted thread's provider identities while any of its messages
-- remain. Every deletion path uses this rule, including bulk reconciliation.
CREATE TRIGGER provider_thread_keys_reanchor_before_message_delete
BEFORE DELETE ON messages
BEGIN
    UPDATE provider_thread_keys
    SET message_id = COALESCE(
        (SELECT remaining.id
         FROM messages remaining
         WHERE remaining.id != OLD.id
           AND remaining.account_id = OLD.account_id
           AND remaining.thread_id = OLD.thread_id
         ORDER BY remaining.rowid DESC
         LIMIT 1),
        message_id
    )
    WHERE message_id = OLD.id;
    -- Without a replacement the unchanged reference cascades with OLD.id.
END;
