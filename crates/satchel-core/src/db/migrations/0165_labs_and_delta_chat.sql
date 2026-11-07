-- Labs: experimental features the user turns on per device. A missing row is
-- the flag's default (off); the registry of known flags lives in code
-- (`db::labs::LABS_FLAGS`, `src/labs/flags.ts`).
CREATE TABLE labs_flags (
    id TEXT PRIMARY KEY NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    updated_at TEXT NOT NULL
) WITHOUT ROWID;

-- Delta Chat (chatmail) headers, read at ingest whatever the Labs switch
-- says, so turning Chats on shows the chats that already arrived. Mail synced
-- before this version has no value: the header block is not kept, and
-- reading it again means fetching every message once more.
--   chat_version     `Chat-Version` (spec.md "Outgoing messages")
--   chat_group_id    `Chat-Group-ID`, already checked against the spec's alphabet
--   chat_group_name  `Chat-Group-Name`, decoded
ALTER TABLE messages ADD COLUMN chat_version TEXT;
ALTER TABLE messages ADD COLUMN chat_group_id TEXT;
ALTER TABLE messages ADD COLUMN chat_group_name TEXT;

-- The Chats list is "threads with a chat message"; only chat rows are indexed.
CREATE INDEX idx_messages_chat_thread ON messages(thread_id) WHERE chat_version IS NOT NULL;

-- Autocrypt Level 1 peer state ("Peers", docs.autocrypt.org/level1.html#peers),
-- one row per own account and canonicalised peer address. `public_key` is the
-- peer's armored key from its latest valid `Autocrypt:` header.
CREATE TABLE autocrypt_peers (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    addr TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    autocrypt_timestamp TEXT,
    public_key TEXT,
    fingerprint TEXT,
    prefer_encrypt TEXT NOT NULL DEFAULT 'nopreference'
        CHECK (prefer_encrypt IN ('mutual', 'nopreference')),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, addr)
) WITHOUT ROWID;
