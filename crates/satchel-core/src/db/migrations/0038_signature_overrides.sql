-- Per-message signature edits made in the composer. NULL means the stored
-- signature (signature_id + signature_variant) is used untouched at send time.
ALTER TABLE messages ADD COLUMN signature_override_text TEXT;
ALTER TABLE messages ADD COLUMN signature_override_html TEXT;
