-- Which attachment-listing rules wrote a row. 0: before a root-signed message
-- listed only the parts of its signed entity, so the row may name a part from
-- outside it. Listings hide those next to a root signature until the
-- `signed_entity_attachments` repair has rebuilt them from the stored bundle.
ALTER TABLE attachments ADD COLUMN listing_rules INTEGER NOT NULL DEFAULT 0;
INSERT OR IGNORE INTO app_settings(key, value, updated_at)
VALUES ('data_repair.signed_entity_attachments', '', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));
