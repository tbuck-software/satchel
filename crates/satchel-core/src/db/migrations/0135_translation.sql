-- Translating mail through an outside engine (DeepL first, ADR 0016).
--
-- Everything here belongs to this device: the key, the consents, the cached
-- translations and the request log stay out of the hub log (ADR 0015). A
-- consent given on the desktop does not unlock the phone.

-- The engine translations go to, and the language they go into. One row.
-- `engine` NULL: translation is off. `target_language` NULL: follow the UI.
CREATE TABLE translation_settings (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    engine TEXT,
    target_language TEXT,
    -- Keep the text that was sent in the request log. Off unless asked for.
    log_sent_text INTEGER NOT NULL DEFAULT 0 CHECK (log_sent_text IN (0, 1)),
    updated_at TEXT NOT NULL
);

-- An engine's API key, encrypted with the profile's master key (`enc:v2:`,
-- bound to this column and the engine). It goes into backups like account
-- credentials do and is never handed back to the webview.
CREATE TABLE translation_engine_keys (
    engine TEXT PRIMARY KEY NOT NULL,
    encrypted_api_key TEXT NOT NULL,
    -- What the key buys, read from it when saved (DeepL: `free` or `pro`),
    -- so listing the settings never has to decrypt the key.
    plan TEXT,
    updated_at TEXT NOT NULL
);

-- Which account's mail may go to which engine. Off until the person says so,
-- per account: a work account can stay out while a private one takes part.
CREATE TABLE translation_consents (
    engine TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    granted_at TEXT NOT NULL,
    PRIMARY KEY (engine, account_id)
);

-- A translated body, so opening the message again costs nothing and works
-- offline. `source_hash` is the body it was made from; a body that changed
-- since (fetched again, repaired) makes the row stale rather than wrong.
CREATE TABLE message_translations (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    target_language TEXT NOT NULL,
    engine TEXT NOT NULL,
    source_language TEXT,
    source_hash TEXT NOT NULL,
    body_html TEXT,
    body_text TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (message_id, target_language)
);

-- What left the device, one row per request: feature, provider, model and
-- how much was billed. The sent text only when the person turned that on.
-- The maintenance worker drops rows after 30 days. Not part of the `v_*`
-- read views (ADR 0005).
CREATE TABLE outbound_request_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    feature TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT,
    -- A removed account takes its entries along.
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    message_id TEXT,
    units INTEGER NOT NULL DEFAULT 0,
    unit TEXT NOT NULL,
    -- `ok`, or the message key of the error the request ended in.
    outcome TEXT NOT NULL,
    sent_text TEXT
);

CREATE INDEX idx_outbound_request_log_created_at ON outbound_request_log(created_at);
CREATE INDEX idx_outbound_request_log_account ON outbound_request_log(account_id);
