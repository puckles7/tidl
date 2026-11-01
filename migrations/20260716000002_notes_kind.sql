-- notes.kind: nullable discriminator. Unlike events.kind, NULL is a valid
-- (and the most common) value — it means "plain note, never tracked as a
-- to-do or await" and must stay representable. needs_mirror becomes
-- `notes.kind.is_some()`, replacing the old `is_todo || tickler_date.
-- is_some()` OR of two independent optional fields. Backlog #49.
ALTER TABLE notes ADD COLUMN kind TEXT CHECK (kind IN ('task', 'await'));

UPDATE notes SET kind = 'task' WHERE is_todo = 1;
UPDATE notes SET kind = 'await' WHERE is_todo = 0 AND tickler_date IS NOT NULL;
-- Rows with is_todo = 0 AND tickler_date IS NULL keep kind = NULL (plain note).

-- Rename tickler_date -> due_date: the note editor's mutex TASK/AWAIT choice
-- shares one date field (required for AWAIT, optional for TASK), so it no
-- longer makes sense for the column to carry AWAIT-specific naming.
ALTER TABLE notes RENAME COLUMN tickler_date TO due_date;

-- is_todo is fully superseded by kind.
ALTER TABLE notes DROP COLUMN is_todo;
