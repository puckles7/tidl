-- Per-user, per-kind control points for the configurable priority curves
-- (backlog #68). One row per control point; a curve is the set of rows for
-- a given (username, curve_kind), ordered by x.
--
-- No rows exist for a user until they save an edited curve for the first
-- time — `get_priority_curves` returns PriorityCurves::default() when a
-- curve_kind has no rows, so there is nothing to backfill.

CREATE TABLE IF NOT EXISTS priority_curve_points (
    username    TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    curve_kind  TEXT NOT NULL CHECK (curve_kind IN
                     ('calendar_time_left', 'due_date_time_left', 'source_priority')),
    x           REAL NOT NULL,
    y           REAL NOT NULL,
    PRIMARY KEY (username, curve_kind, x)
);
