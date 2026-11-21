-- Pending ingest effects can still enqueue automatic forwards.
UPDATE ingest_effects SET run_rules = 0 WHERE run_rules = 1;
