-- The local HTTP API and its CLI are gone (ADR 0005). These two tables only
-- ever served that server: tokens it authenticated and the decisions it logged.
-- Nothing reads them any more, so they stop travelling with every backup.
DROP TABLE IF EXISTS api_audit_log;
DROP TABLE IF EXISTS api_tokens;

DELETE FROM app_settings WHERE key IN ('local_api.enabled', 'local_api.bind', 'local_api.token');
