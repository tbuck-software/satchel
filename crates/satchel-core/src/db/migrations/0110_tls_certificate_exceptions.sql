-- Expired server certificates a person chose to accept for a while. One row per
-- account and host; the fingerprint pins the exact certificate, so a different
-- one presented under the same name is rejected as before. `login` scopes the
-- exception to the connections of the account that granted it. `last_used_at`
-- keeps an exception alive while a load balancer still serves the pinned
-- certificate from one node and valid ones from others.
CREATE TABLE IF NOT EXISTS tls_certificate_exceptions (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    login TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    PRIMARY KEY (account_id, host)
);
