INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
    ('maintenance.trash_auto_delete_days', '30', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('maintenance.last_vacuum_at', '', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('maintenance.last_cleanup_at', '', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('compose.send_read_receipts', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
