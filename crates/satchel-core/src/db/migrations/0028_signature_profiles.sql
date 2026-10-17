ALTER TABLE signatures ADD COLUMN short_body_text TEXT NOT NULL DEFAULT '';
ALTER TABLE signatures ADD COLUMN short_body_html TEXT;

CREATE TABLE IF NOT EXISTS signature_assignments (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    identity_email TEXT NOT NULL,
    signature_id TEXT REFERENCES signatures(id) ON DELETE SET NULL,
    first_contact_mode TEXT NOT NULL DEFAULT 'always_full'
        CHECK (first_contact_mode IN ('always_full', 'full_then_short', 'always_short')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, identity_email)
);

CREATE INDEX IF NOT EXISTS idx_signature_assignments_signature
    ON signature_assignments(signature_id);

CREATE TABLE IF NOT EXISTS signature_assets (
    id TEXT PRIMARY KEY,
    signature_id TEXT NOT NULL REFERENCES signatures(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,
    content_id TEXT NOT NULL,
    content BLOB NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_signature_assets_cid
    ON signature_assets(signature_id, content_id);

CREATE TABLE IF NOT EXISTS signature_suggestions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    identity_email TEXT NOT NULL,
    body_text TEXT NOT NULL,
    body_html TEXT,
    evidence_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'dismissed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_signature_suggestions_candidate
    ON signature_suggestions(account_id, identity_email, body_text);
CREATE INDEX IF NOT EXISTS idx_signature_suggestions_status
    ON signature_suggestions(account_id, status);

-- Historical provenance deliberately has no foreign key: a queued/sent message must remain
-- persistable even when its source signature is deleted during an Undo-Send window.
ALTER TABLE messages ADD COLUMN signature_id TEXT;
ALTER TABLE messages ADD COLUMN signature_variant TEXT;
