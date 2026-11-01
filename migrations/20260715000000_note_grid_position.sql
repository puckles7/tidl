-- Notes free-form grid layout (backlog #47): explicit (grid_row, grid_col)
-- per note, replacing #42/#43's linear sort_order entirely -- allows gaps
-- and drag-to-any-empty-cell instead of auto-reflow.

ALTER TABLE notes ADD COLUMN grid_row INTEGER NOT NULL DEFAULT 0;
ALTER TABLE notes ADD COLUMN grid_col INTEGER NOT NULL DEFAULT 0;

-- Backfill: pack notes into a 4-column grid in their prior order (pinned
-- block first, then sort_order), so migrating doesn't reshuffle anyone's
-- layout -- effectively an initial auto-layout pass. 4 is only a backfill
-- packing width, independent of the app's actual configured column count.
UPDATE notes
SET grid_row = t.rn / 4,
    grid_col = t.rn % 4
FROM (
    SELECT id, (ROW_NUMBER() OVER (PARTITION BY pinned ORDER BY sort_order ASC) - 1) AS rn
    FROM notes
) AS t
WHERE notes.id = t.id;

-- No UNIQUE(pinned, grid_row, grid_col) index: collections are scoped per
-- owner, and two unrelated users' notes -- different owners, no shared
-- collection -- would routinely land on the same coordinate (everyone
-- naturally starts near (0,0)) without ever being displayed together. A
-- table-wide uniqueness constraint would turn that harmless overlap into a
-- hard write failure. Collision-avoidance instead happens in application
-- code (`first_gap`/`resolve_drop`/`move_notes`), scoped to whatever a
-- single viewer can actually see -- the only scope where a collision would
-- ever be visible in the first place.

ALTER TABLE notes DROP COLUMN sort_order;
