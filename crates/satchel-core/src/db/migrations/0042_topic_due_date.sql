-- Topics can carry a follow-up date ("Wiedervorlage"): once it passes, the
-- topic is highlighted as due in the topics folder, mirroring thread snooze.
ALTER TABLE topics ADD COLUMN due_date TEXT;
