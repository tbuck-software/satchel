CREATE INDEX IF NOT EXISTS idx_attachments_remote_lookup
ON attachments(remote_uid, remote_part_id);
