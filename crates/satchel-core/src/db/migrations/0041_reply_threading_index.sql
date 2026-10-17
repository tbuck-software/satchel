-- Reply threading joins a reply's In-Reply-To against the parent's
-- Message-ID (upsert-time thread resolution and the merge repair in
-- sync::manager). Without this index every resolution is a full scan
-- of the messages table.
CREATE INDEX IF NOT EXISTS idx_messages_account_msgid
    ON messages(account_id, message_id_header);
