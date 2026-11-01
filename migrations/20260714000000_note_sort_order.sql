-- Note manual ordering + pinned-group invariant groundwork (backlog #42)

ALTER TABLE notes ADD COLUMN sort_order REAL NOT NULL DEFAULT 0;

-- Backfill so introducing manual ordering doesn't reshuffle anyone's
-- existing layout: mirror the prior implicit order (pinned block on top,
-- newest first within each group) into gap-spaced sort_order values.
--
-- Uses UPDATE...FROM with the window function precomputed in a plain
-- (non-self-referencing) subquery — a correlated subquery referencing the
-- same table being updated silently produces 0 for every row in SQLite,
-- since the window function's result set is evaluated against the
-- in-progress UPDATE rather than a stable snapshot.
UPDATE notes
SET sort_order = t.rn
FROM (
    SELECT id, (ROW_NUMBER() OVER (PARTITION BY pinned ORDER BY updated_at DESC) - 1) * 1000.0 AS rn
    FROM notes
) AS t
WHERE notes.id = t.id;
