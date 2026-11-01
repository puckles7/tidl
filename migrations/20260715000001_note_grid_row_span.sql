-- Notes grid row-span units (backlog #48): grid_row now indexes fine-
-- grained row *units* (NOTES_ROW_UNIT_PX, 8px) instead of one row per
-- note, and grid_row_span holds each note's real height in units --
-- lets two short notes stack and together match one tall neighbor's
-- height in an adjacent column (true masonry), while keeping #47's
-- stable, addressable (row, col) positions.

ALTER TABLE notes ADD COLUMN grid_row_span INTEGER NOT NULL DEFAULT 1;

-- Reset positions: existing grid_row values meant "note index" under the
-- old model, meaningless once row is a fine unit. The client's
-- self-correction pass (measures real rendered height on first load,
-- persists the corrected span, and cascades any resulting overlaps
-- further down the same column -- see notes/mod.rs) untangles this
-- automatically, so the simplest valid migration is just parking
-- everyone at row 0 in their existing column.
UPDATE notes SET grid_row = 0;
