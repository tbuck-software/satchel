CREATE TABLE IF NOT EXISTS plugin_permissions (
    plugin_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    granted INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, capability)
);

CREATE TABLE IF NOT EXISTS local_webhooks (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    events_json TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    secret TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id TEXT PRIMARY KEY,
    webhook_id TEXT NOT NULL REFERENCES local_webhooks(id) ON DELETE CASCADE,
    event_name TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_due ON webhook_deliveries(status, next_attempt_at);

INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES
('plugin.allow_unsigned', 'true', datetime('now')),
('local_api.enabled', 'true', datetime('now')),
('local_api.bind', '127.0.0.1:8799', datetime('now')),
('local_api.token', lower(hex(randomblob(24))), datetime('now'));
