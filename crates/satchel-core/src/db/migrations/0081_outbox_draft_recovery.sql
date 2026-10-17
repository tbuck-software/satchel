ALTER TABLE messages ADD COLUMN draft_send_options_json TEXT;
ALTER TABLE sync_queue ADD COLUMN sending_session TEXT;
