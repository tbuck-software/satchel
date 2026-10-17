ALTER TABLE messages ADD COLUMN original_body_html TEXT;
ALTER TABLE messages ADD COLUMN original_source BLOB;

INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.sanitize_message_html', '', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
