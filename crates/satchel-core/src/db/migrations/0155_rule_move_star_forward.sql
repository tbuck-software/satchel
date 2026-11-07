-- Rules can move mail, star it and forward it.
--
-- A rule that moves mail belongs to one account (the folder is that
-- account's); NULL keeps the old meaning, every account. A rule whose
-- account is gone never runs again and shows as broken.
ALTER TABLE rules ADD COLUMN account_id TEXT;
-- Set when the rule forwarded more than allowed within an hour; its forward
-- stays off until the person resumes it.
ALTER TABLE rules ADD COLUMN forward_paused_at TEXT;
-- When the person last resumed it: the hourly count starts again there.
ALTER TABLE rules ADD COLUMN forward_resumed_at TEXT;

-- One row per account, rule and message whose forward Satchel decided:
-- queued, handed to the outbox, left to the server or skipped (and why). A
-- forward goes out at most once per rule and Message-ID: the key survives
-- moves and a second copy of the same mail. Rows go with their account and
-- after 180 days.
CREATE TABLE rule_forwards (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    rule_token TEXT NOT NULL,
    message_key TEXT NOT NULL,
    message_id TEXT NOT NULL,
    outcome TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, rule_token, message_key)
) WITHOUT ROWID;

CREATE INDEX idx_rule_forwards_rule ON rule_forwards(rule_token, message_key);
CREATE INDEX idx_rule_forwards_created ON rule_forwards(created_at);

-- Messages the account's Sieve script marked as forwarded ($SatchelFwd),
-- recorded at sync by Message-ID.
CREATE TABLE rule_server_marks (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_key TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (account_id, message_key)
) WITHOUT ROWID;

CREATE INDEX idx_rule_server_marks_created ON rule_server_marks(created_at);

-- What the server may run besides the verified script (see
-- `ServerRulesState`): since when each pushed rule version runs, versions an
-- unverified push may have written, and the hash of the verified script.
ALTER TABLE managesieve_accounts ADD COLUMN pushed_rules_since TEXT NOT NULL DEFAULT '{}';
ALTER TABLE managesieve_accounts ADD COLUMN possibly_live_rules TEXT NOT NULL DEFAULT '[]';
-- Rule id -> trigger, address and start of the verified version's forward.
ALTER TABLE managesieve_accounts ADD COLUMN pushed_forwards TEXT NOT NULL DEFAULT '{}';
ALTER TABLE managesieve_accounts ADD COLUMN script_hash TEXT;
ALTER TABLE managesieve_accounts ADD COLUMN verified_at TEXT;
