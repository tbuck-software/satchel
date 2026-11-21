-- Dispatch leases expire back into pending. Suppress both states from the copy
-- while keeping notification history and allowing newly ingested mail to notify.
DELETE FROM notification_outbox WHERE state IN ('pending', 'dispatching');
