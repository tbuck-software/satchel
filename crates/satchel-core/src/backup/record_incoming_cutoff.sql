INSERT INTO app_settings (key, value, updated_at)
VALUES ('restore.incoming_effects_cutoff', ?1, ?1)
ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
WHERE julianday(excluded.value) > COALESCE(julianday(app_settings.value), 0);
