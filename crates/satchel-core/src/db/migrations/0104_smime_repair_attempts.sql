-- When opening a message last re-fetched its source to recover a detached
-- S/MIME signature. Without it a message the parser still cannot read was
-- downloaded again on every open.
ALTER TABLE messages ADD COLUMN smime_repair_attempted_at TEXT;
