-- Scan legacy MIME cache entries once, after startup, with a durable cursor.
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.legacy_mime', '', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
