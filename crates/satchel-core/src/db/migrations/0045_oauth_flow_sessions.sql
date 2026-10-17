CREATE TABLE oauth_flow_sessions (
  state TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  provider TEXT NOT NULL,
  encrypted_code_verifier TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE INDEX idx_oauth_flow_sessions_expires_at
  ON oauth_flow_sessions(expires_at);
