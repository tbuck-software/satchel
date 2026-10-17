ALTER TABLE accounts
ADD COLUMN outlook_immutable_ids_migrated INTEGER NOT NULL DEFAULT 0;
