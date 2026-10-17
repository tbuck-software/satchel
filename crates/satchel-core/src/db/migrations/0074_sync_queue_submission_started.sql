-- Tells "claimed, but nothing was transmitted yet" apart from "transmission
-- started, answer unknown". Only the second one is ambiguous after a crash;
-- the first is still safe to send again.
ALTER TABLE sync_queue ADD COLUMN submission_started_at TEXT;
