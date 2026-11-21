-- Keep the payload so the Outbox can reopen the message in the composer.
-- Replace legacy transient errors too, because startup can retry those.
UPDATE sync_queue
SET status = CASE
        WHEN action_type = 'send_message' AND submission_started_at IS NOT NULL
             AND status <> 'failed' THEN 'delivery_unknown'
        ELSE 'failed'
    END,
    last_error = CASE
        WHEN action_type = 'send_message' AND submission_started_at IS NOT NULL
             AND status <> 'failed' THEN 'backend.errors.messageDeliveryStatusUnknown'
        ELSE 'backend.errors.outgoingSendHeldAfterRestore'
    END
WHERE action_type IN ('send_message', 'rule_forward')
  AND status IN ('pending', 'scheduled', 'sending', 'failed')
  AND (status <> 'failed' OR last_error IS NOT 'backend.errors.outgoingSendHeldAfterRestore');
