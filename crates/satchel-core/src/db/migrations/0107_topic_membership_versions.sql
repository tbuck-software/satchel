-- Tokens survive removal, so an Undo cannot mistake a later membership for its own.
-- Triggers cover every writer, including rules, projection cleanup and other windows.
CREATE TABLE topic_membership_versions (
    topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    item_type TEXT NOT NULL,
    item_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    added_at TEXT NOT NULL,
    added_by TEXT NOT NULL,
    PRIMARY KEY (topic_id, item_type, item_id)
);

CREATE TRIGGER topic_membership_insert AFTER INSERT ON topic_items BEGIN
    INSERT INTO topic_membership_versions VALUES
        (NEW.topic_id, NEW.item_type, NEW.item_id, lower(hex(randomblob(16))), NEW.added_at, NEW.added_by)
    ON CONFLICT(topic_id, item_type, item_id) DO UPDATE SET
        revision = excluded.revision, added_at = excluded.added_at, added_by = excluded.added_by;
END;

CREATE TRIGGER topic_membership_delete AFTER DELETE ON topic_items
WHEN EXISTS (SELECT 1 FROM topics WHERE id = OLD.topic_id) BEGIN
    INSERT INTO topic_membership_versions VALUES
        (OLD.topic_id, OLD.item_type, OLD.item_id, lower(hex(randomblob(16))), OLD.added_at, OLD.added_by)
    ON CONFLICT(topic_id, item_type, item_id) DO UPDATE SET
        revision = excluded.revision, added_at = excluded.added_at, added_by = excluded.added_by;
END;

CREATE TRIGGER topic_membership_update AFTER UPDATE ON topic_items BEGIN
    DELETE FROM topic_membership_versions
        WHERE topic_id = OLD.topic_id AND item_type = OLD.item_type AND item_id = OLD.item_id;
    INSERT INTO topic_membership_versions VALUES
        (NEW.topic_id, NEW.item_type, NEW.item_id, lower(hex(randomblob(16))), NEW.added_at, NEW.added_by)
    ON CONFLICT(topic_id, item_type, item_id) DO UPDATE SET
        revision = excluded.revision, added_at = excluded.added_at, added_by = excluded.added_by;
END;
