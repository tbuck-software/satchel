INSERT INTO restored_queue_failures (queue_id, error)
SELECT id, last_error
FROM sync_queue
WHERE status IN ('pending', 'scheduled', 'sending', 'failed')
  AND last_error <> ''
  AND last_error NOT IN ('backend.errors.outgoingSendHeldAfterRestore',
                         'backend.errors.sentCopyHeldAfterRestore',
                         'backend.errors.messageDeliveryStatusUnknown')
  AND (action_type IN ('send_message', 'rule_forward')
       OR (action_type = 'remote_mutation'
           AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.kind') END = 'append_sent'))
ON CONFLICT(queue_id) DO UPDATE SET error = excluded.error;
