-- The redirect URI now names a port the OS hands out per flow, so the code
-- exchange can no longer re-derive it: it has to repeat the exact one the
-- authorization request used, or the provider answers redirect_uri_mismatch.
--
-- Nullable on purpose. A session started before this column existed carries
-- nothing here, and the exchange falls back to the configured default — which
-- is what those sessions were authorized with.
ALTER TABLE oauth_flow_sessions ADD COLUMN redirect_uri TEXT;
