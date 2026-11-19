-- The paperclip counted every stored part, signature logos included. The
-- `listed_attachment_flags` repair recomputes it from the parts a reader lists.
INSERT OR IGNORE INTO app_settings(key, value, updated_at)
VALUES ('data_repair.listed_attachment_flags', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
