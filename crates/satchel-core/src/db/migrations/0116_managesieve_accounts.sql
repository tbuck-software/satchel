-- Server-side rules over ManageSieve (RFC 5804), one row per IMAP account.
--
-- Detection (`detected_*`, `availability`, `extensions`) is what the last
-- probe found; the `*_override` columns are what the person typed in and win
-- over detection. `enabled` is the person's wish, `server_active` whether
-- Satchel's script is verifiably the one running. Only while both are set
-- does the local engine skip the rules listed in `pushed_rules`
-- (rule id -> fingerprint and the moment that version went live), and only
-- for mail that arrived after that moment.
CREATE TABLE IF NOT EXISTS managesieve_accounts (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    host_override TEXT,
    port_override INTEGER,
    tls_override TEXT,
    detected_host TEXT,
    detected_port INTEGER,
    detected_tls TEXT,
    detected_source TEXT,
    availability TEXT NOT NULL DEFAULT 'unknown',
    implementation TEXT,
    extensions TEXT NOT NULL DEFAULT '[]',
    checked_at TEXT,
    enabled INTEGER NOT NULL DEFAULT 0,
    server_active INTEGER NOT NULL DEFAULT 0,
    activation_mode TEXT,
    previous_active_script TEXT,
    consent_script TEXT,
    pushed_rules TEXT NOT NULL DEFAULT '{}',
    pushed_vacation INTEGER NOT NULL DEFAULT 0,
    last_push_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL
);
