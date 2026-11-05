-- Local, learning spam filter (naive Bayes, Robinson-Fisher combining).
--
-- `spam_tokens` holds one row per token hash with how many trained spam and
-- ham messages contained it. `spam_model` is the single row of class totals.
-- `spam_training` is the ledger: which label each message was trained with and
-- the exact tokens, so a relabel or an Undo subtracts what was added even after
-- the message body was evicted. `spam_verdicts` is what classification said
-- about a newly arrived message, with the tokens it scored.
CREATE TABLE spam_tokens (
    token INTEGER PRIMARY KEY,
    spam_count INTEGER NOT NULL DEFAULT 0,
    ham_count INTEGER NOT NULL DEFAULT 0,
    last_seen TEXT NOT NULL
);

CREATE TABLE spam_model (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    spam_messages INTEGER NOT NULL DEFAULT 0,
    ham_messages INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT
);

CREATE TABLE spam_training (
    message_id TEXT PRIMARY KEY,
    label TEXT NOT NULL CHECK (label IN ('spam', 'ham')),
    -- The label this row replaced, restored when `receipt_id` is undone.
    previous_label TEXT CHECK (previous_label IN ('spam', 'ham')),
    receipt_id TEXT,
    tokens BLOB NOT NULL,
    trained_at TEXT NOT NULL
) WITHOUT ROWID;

CREATE INDEX idx_spam_training_receipt ON spam_training(receipt_id)
    WHERE receipt_id IS NOT NULL;

CREATE TABLE spam_verdicts (
    message_id TEXT PRIMARY KEY,
    score REAL NOT NULL,
    verdict TEXT NOT NULL CHECK (verdict IN ('spam', 'unsure', 'ham', 'none')),
    hint_headers TEXT,
    -- The receipt of the automatic move to Spam, while one is outstanding.
    receipt_id TEXT,
    classified_at TEXT NOT NULL
) WITHOUT ROWID;

CREATE TRIGGER spam_rows_after_message_delete AFTER DELETE ON messages BEGIN
    DELETE FROM spam_verdicts WHERE message_id = old.id;
    DELETE FROM spam_training WHERE message_id = old.id;
END;

-- One-time training from the Spam folder and read Inbox mail, in blocks after
-- startup (`spam::bootstrap::SPAM_BOOTSTRAP`).
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.spam_bootstrap', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
