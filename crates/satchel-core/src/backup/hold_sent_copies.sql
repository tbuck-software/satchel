-- The payload may hold the only copy of an already delivered message.
-- Keep it available for individual review without automatically appending it.
UPDATE sync_queue
SET status = 'failed', last_error = 'backend.errors.sentCopyHeldAfterRestore'
WHERE action_type = 'remote_mutation'
  AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'append_sent'
  AND status IN ('pending', 'scheduled', 'sending', 'failed')
  AND (status <> 'failed' OR last_error IS NOT 'backend.errors.sentCopyHeldAfterRestore');
