-- Drop the FK constraint from events.calendar so that markdown events
-- (calendar = '__markdown__') no longer require a backing row in the calendars
-- table.  The synthetic '__markdown__' calendar was only ever inserted to
-- satisfy this constraint, but it appeared in the UI as a deletable calendar
-- which wiped all markdown events when removed.
--
-- SQLite does not support DROP CONSTRAINT, so we recreate the table.

PRAGMA foreign_keys = OFF;

CREATE TABLE events_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    username         TEXT NOT NULL,
    calendar         TEXT NOT NULL,
    uid              TEXT NOT NULL,
    type             TEXT NOT NULL,
    summary          TEXT,
    description      TEXT,
    location         TEXT,
    start_date       DATETIME,
    end_date         DATETIME,
    due_date         DATETIME,
    priority         INTEGER,
    status           TEXT,
    completed_date   DATETIME,
    etag             TEXT NOT NULL,
    ical_data        TEXT NOT NULL,
    source           TEXT,
    source_file      TEXT,
    response_status  VARCHAR,
    source_line      INTEGER,
    topic            TEXT,
    created_at       DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at       DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(username, calendar, uid)
    -- FK intentionally omitted: markdown events use calendar = '__markdown__'
    -- which has no backing calendars row; integrity is enforced by the
    -- application layer instead.
);

INSERT INTO events_new SELECT
    id, username, calendar, uid, type, summary, description, location,
    start_date, end_date, due_date, priority, status, completed_date,
    etag, ical_data, source, source_file, response_status, source_line, topic,
    created_at, updated_at
FROM events;

DROP TABLE events;
ALTER TABLE events_new RENAME TO events;

CREATE INDEX IF NOT EXISTS idx_events_calendar   ON events(username, calendar);
CREATE INDEX IF NOT EXISTS idx_events_type       ON events(type);
CREATE INDEX IF NOT EXISTS idx_events_start_date ON events(start_date);
CREATE INDEX IF NOT EXISTS idx_events_due_date   ON events(due_date);
CREATE INDEX IF NOT EXISTS idx_events_priority   ON events(priority);
CREATE INDEX IF NOT EXISTS idx_events_source     ON events(source, source_file);

PRAGMA foreign_keys = ON;

-- Recovery: if any markdown source's sync_state is intact but its events were
-- deleted (e.g. by deleting the __markdown__ calendar from the UI), reset the
-- hashes so the next sync re-processes those files and re-inserts the events.
DELETE FROM sync_state
WHERE source_id IN (
    SELECT id FROM markdown_sources
    WHERE enabled = 1
      AND NOT EXISTS (
          SELECT 1 FROM events
          WHERE source = 'markdown'
            AND events.username = markdown_sources.username
      )
);
