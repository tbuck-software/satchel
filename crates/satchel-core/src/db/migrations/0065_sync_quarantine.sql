-- Messages the ingest could not store. Without this record a deterministically
-- broken message froze its whole account: the cursor stayed in front of its
-- UID, every run fetched it again, and no folder of that account ever received
-- new mail. Attempts are counted so the cursor may step over a message that has
-- failed often enough, while the row keeps it out of any provider-side deletion.
--
-- The primary key is the lookup index (account, folder, remote id); a second
-- index over the same columns would only cost writes. `folder_id` carries no
-- foreign key on purpose — recording a failure must not itself be able to fail
-- because the folder row was reset in the same run.
CREATE TABLE sync_quarantine (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    remote_id TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    first_failed_at TEXT NOT NULL,
    last_failed_at TEXT NOT NULL,
    PRIMARY KEY (account_id, folder_id, remote_id)
);
