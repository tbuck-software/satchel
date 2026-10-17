CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
    ('notifications.enabled', 'true', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('notifications.scope', 'inbox', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('notifications.sound', 'true', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('tray.quit_on_close', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('tray.start_minimized', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('autostart.enabled', 'false', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
