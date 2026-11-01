-- Add a fourth priority-curve kind: calendar event priority attenuation
-- (backlog #156). The curve's x is an event's canonical 1-100 priority and
-- its y is the maximum urgency that event may reach in Focus.
--
-- SQLite cannot alter a CHECK constraint in place, so rebuild the table
-- (same pattern as the events FK removal in 20240119000000). No rows are
-- lost: every existing (username, curve_kind, x, y) row is copied verbatim.

PRAGMA foreign_keys = OFF;

CREATE TABLE priority_curve_points_new (
    username    TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    curve_kind  TEXT NOT NULL CHECK (curve_kind IN
                     ('calendar_time_left', 'due_date_time_left', 'source_priority',
                      'calendar_event_priority')),
    x           REAL NOT NULL,
    y           REAL NOT NULL,
    PRIMARY KEY (username, curve_kind, x)
);

INSERT INTO priority_curve_points_new (username, curve_kind, x, y)
SELECT username, curve_kind, x, y FROM priority_curve_points;

DROP TABLE priority_curve_points;
ALTER TABLE priority_curve_points_new RENAME TO priority_curve_points;

PRAGMA foreign_keys = ON;
