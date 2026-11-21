-- Keep thread identity and metadata while a replacement IMAP generation is
-- downloaded. An empty header holds the thread but never matches incoming mail.
CREATE TABLE imap_generation_recovery (
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    message_id_header TEXT NOT NULL,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    PRIMARY KEY (folder_id, message_id_header, thread_id)
);
CREATE INDEX idx_imap_generation_recovery_thread ON imap_generation_recovery(thread_id);
