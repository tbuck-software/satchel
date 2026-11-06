-- Server-side rules over ManageSieve (RFC 5804), one row per IMAP account.
--
-- Detection (`detected_*`, `availability`, `extensions`) is what the last
-- probe found; the `*_override` columns are what the person typed in and win
-- over detection. `suggested_*` is a server autoconfig named on another
-- domain than the IMAP host: shown, never logged into without a yes.
-- `enabled` is the person's wish, `server_active` whether Satchel's script was
-- verified active by the last push. `activation_mode` and
-- `previous_active_script` are written before SETACTIVE, so turning off can
-- restore the person's script even after a push that broke off.
-- `pushed_rules` (rule id -> fingerprint) is for display only: the local
-- engine runs every rule regardless.
CREATE TABLE IF NOT EXISTS managesieve_accounts (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    host_override TEXT,
    port_override INTEGER,
    tls_override TEXT,
    detected_host TEXT,
    detected_port INTEGER,
    detected_tls TEXT,
    detected_source TEXT,
    suggested_host TEXT,
    suggested_port INTEGER,
    suggested_tls TEXT,
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
