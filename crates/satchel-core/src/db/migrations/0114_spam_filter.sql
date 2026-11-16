-- Local, learning spam filter (naive Bayes, Robinson-Fisher combining).
--
-- `spam_tokens` holds one row per token hash with how many trained spam and
-- ham messages contained it. `spam_model` is the single row of class totals,
-- with separate totals for examples that carried header hints (`X-Spam-*`,
-- `List-Id`, …): those tokens only count once both classes have enough of
-- them. `spam_training` is the ledger: which label each example was trained
-- with, from which signal, and the exact tokens, so a relabel or an Undo
-- subtracts what was added even after the message body was evicted. One
-- example per sender and thread. `spam_verdicts` is what classification said
-- about a newly arrived message, with the spam-relevant headers the message
-- row does not keep, so training later sees the same tokens classification
-- saw. `none` means not scored: the model was not ready, or the sender is
-- someone the user corresponds with.
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
    hint_spam_messages INTEGER NOT NULL DEFAULT 0,
    hint_ham_messages INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT
);

CREATE TABLE spam_training (
    message_id TEXT PRIMARY KEY,
    label TEXT NOT NULL CHECK (label IN ('spam', 'ham')),
    -- `explicit`: Spam, Not spam, a move out of Spam. `weak`: starred or
    -- answered. `bootstrap`: the one-time pass over existing mail.
    source TEXT NOT NULL CHECK (source IN ('explicit', 'weak', 'bootstrap')),
    -- The label and source this row replaced, restored when `receipt_id` is
    -- undone.
    previous_label TEXT CHECK (previous_label IN ('spam', 'ham')),
    previous_source TEXT CHECK (previous_source IN ('explicit', 'weak', 'bootstrap')),
    receipt_id TEXT,
    tokens BLOB NOT NULL,
    -- Whether `tokens` holds header-hint tokens.
    hinted INTEGER NOT NULL DEFAULT 0,
    -- Lowercased From address and thread at training time.
    sender TEXT NOT NULL DEFAULT '',
    thread_id TEXT,
    trained_at TEXT NOT NULL
) WITHOUT ROWID;

CREATE INDEX idx_spam_training_receipt ON spam_training(receipt_id)
    WHERE receipt_id IS NOT NULL;
CREATE INDEX idx_spam_training_thread_sender ON spam_training(thread_id, sender);
-- Senders the user rescued with Not spam are never moved again.
CREATE INDEX idx_spam_training_rescued ON spam_training(sender)
    WHERE label = 'ham' AND source = 'explicit';

CREATE TABLE spam_verdicts (
    message_id TEXT PRIMARY KEY,
    score REAL NOT NULL,
    verdict TEXT NOT NULL CHECK (verdict IN ('spam', 'unsure', 'ham', 'none')),
    hint_headers TEXT,
    -- The receipt of the automatic move to Spam, while one is outstanding.
    receipt_id TEXT,
    -- When the filter moved the message to Spam; cleared by Not spam.
    moved_at TEXT,
    classified_at TEXT NOT NULL
) WITHOUT ROWID;

CREATE TRIGGER spam_rows_after_message_delete AFTER DELETE ON messages BEGIN
    DELETE FROM spam_verdicts WHERE message_id = old.id;
    DELETE FROM spam_training WHERE message_id = old.id;
END;

-- One-time training from mail the user already sorted, in blocks after
-- startup (`spam::bootstrap::SPAM_BOOTSTRAP`). Nothing is judged before it
-- has finished.
INSERT OR IGNORE INTO app_settings (key, value, updated_at)
VALUES ('data_repair.spam_bootstrap', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
