INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
    ('compose.undo_send_seconds', '5', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
