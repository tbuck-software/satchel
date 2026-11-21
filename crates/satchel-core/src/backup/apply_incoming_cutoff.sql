-- Older copied databases carry the activation time until migration 0197 adds
-- the cutoff column. Current copies also use this transaction before opening.
WITH activation AS (
    SELECT value AS cutoff FROM app_settings WHERE key = 'restore.incoming_effects_cutoff'
), prior AS (
    SELECT id, COALESCE(NULLIF(COALESCE(incoming_effects_cutoff, last_sync_at), ''),
                        NULLIF(created_at, '')) AS cutoff
    FROM accounts
)
UPDATE accounts
SET incoming_effects_cutoff = CASE
    WHEN julianday(prior.cutoff) >= julianday(activation.cutoff) THEN prior.cutoff
    ELSE activation.cutoff
END
FROM activation, prior
WHERE accounts.id = prior.id;

DELETE FROM app_settings WHERE key = 'restore.incoming_effects_cutoff';
