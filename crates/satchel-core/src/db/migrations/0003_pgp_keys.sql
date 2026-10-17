CREATE TABLE IF NOT EXISTS pgp_keys (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    key_type TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    armored_key TEXT NOT NULL,
    passphrase TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_pgp_keys_fingerprint_type
    ON pgp_keys(fingerprint, key_type);
