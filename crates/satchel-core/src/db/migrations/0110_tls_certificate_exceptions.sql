-- Expired server certificates a person chose to accept for a while. One row per
-- account and host; the fingerprint pins the exact certificate, so a different
-- one presented under the same name is rejected as before.
CREATE TABLE IF NOT EXISTS tls_certificate_exceptions (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (account_id, host)
);
