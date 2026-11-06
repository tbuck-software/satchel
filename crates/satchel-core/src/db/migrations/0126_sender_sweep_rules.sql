-- Recurring per-sender cleanup ("from now on: archive mail from this sender
-- older than 10 days"). One rule per sender and account; the scheduler runs
-- every rule against the account's Inbox and records the outcome here.
--
-- A move rule's folder can disappear in a sync (deleted or renamed on the
-- server). The rule then keeps its row with an empty folder, shows as
-- needing one in Settings, and does nothing until it gets one again.
CREATE TABLE sender_sweep_rules (
    id TEXT PRIMARY KEY NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    sender_email TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('archive', 'trash', 'move')),
    older_than_days INTEGER CHECK (older_than_days IS NULL OR older_than_days BETWEEN 0 AND 3650),
    keep_latest INTEGER NOT NULL DEFAULT 0 CHECK (keep_latest IN (0, 1)),
    target_folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
    last_run_at TEXT,
    last_swept_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (action = 'move' OR target_folder_id IS NULL),
    -- Trash on arrival is not a rule anybody means.
    CHECK (action <> 'trash' OR COALESCE(older_than_days, 0) >= 1),
    UNIQUE (account_id, sender_email)
);
