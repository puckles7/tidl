-- Google Keep import de-duplication (backlog #109 follow-up).
--
-- A re-import of the same Takeout archive used to duplicate every note. The
-- archive carries no note id, but `createdTimestampUsec` is unique per note and
-- never changes when a note is edited, so it is the de-dup key. Persist the
-- source and that key on imported rows; hand-written notes (and notes imported
-- before this migration) leave them NULL, and an importer adopts those by their
-- stored created_at + title the first time it sees them again.
ALTER TABLE notes ADD COLUMN import_source TEXT;
ALTER TABLE notes ADD COLUMN import_external_id TEXT;

CREATE INDEX idx_notes_import ON notes (import_source, import_external_id);
