-- Auto-mode manual reorder (backlog #102, Plan B). NULL means "not yet
-- placed" -- falls back to alphabetical order. No backfill: existing
-- rows staying NULL is the correct, meaningful default (nothing has
-- ever been manually reordered yet). See the design spec's
-- "manual_order" section for the first-reorder-backfills-the-zone
-- semantics this column requires (a naive "null falls back to
-- alphabetical" sort can't support inserting a reordered card between
-- two still-null neighbors).
ALTER TABLE notes ADD COLUMN manual_order REAL;
ALTER TABLE contacts ADD COLUMN manual_order REAL;
