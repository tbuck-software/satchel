-- What is left of an authorization flow once its verifier is gone.
--
-- `oauth_flow_sessions` rows are single-spend and disappear on consume, so a
-- process death after that point left no way to answer "what happened to this
-- flow?". This table outlives the verifier and carries the failure key.
CREATE TABLE oauth_flow_outcomes (
  state TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  provider TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  error_key TEXT
);

CREATE INDEX idx_oauth_flow_outcomes_expires_at
  ON oauth_flow_outcomes(expires_at);
