-- Align the Done-item lifecycle's terminology on "archive": the UI and the
-- retention settings already say "archive", while the schema (and code) said
-- "remove". SQLite rewrites dependent indexes automatically on RENAME COLUMN.
ALTER TABLE done_items RENAME COLUMN removed_at TO archived_at;
ALTER TABLE done_scopes RENAME COLUMN auto_remove TO auto_archive;
