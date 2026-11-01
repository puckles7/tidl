-- Done-item lifecycle (backlog #124, #125): a persisted index of completed
-- items across all origins, driving the clean-up engine and the central Done
-- view.
--
-- `done_items` is a *cache*: the origin (markdown file / note body / events row)
-- stays authoritative for live items, and reconciliation regenerates live rows
-- from a scan. Archived rows (removed_at IS NOT NULL) have no origin left, so a
-- scan can never regenerate them -- reconcile must leave them untouched.
--
-- `generation` keeps the unique key collision-free when the same location is
-- completed again after it was archived: the new live row gets the next
-- generation instead of clobbering the tombstone.

CREATE TABLE done_items (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    username     TEXT    NOT NULL,
    origin       TEXT    NOT NULL CHECK (origin IN ('markdown','note','caldav')),
    scope_id     TEXT    NOT NULL,   -- markdown source id | note collection id | calendar name
    origin_key   TEXT    NOT NULL,   -- md:{source}:{path}:{line}:{ordinal} | note:{id}:{line}:{ordinal} | {uid}
    generation   INTEGER NOT NULL DEFAULT 0,
    kind         TEXT    NOT NULL CHECK (kind IN ('task','await')),
    slug         TEXT    NOT NULL,
    title        TEXT    NOT NULL,
    priority     INTEGER,            -- original priority (priority/dated task)
    due_date     DATE,               -- original date (await / dated task)
    completed_at DATE    NOT NULL,   -- original completion date (from the tag)
    removed_at   DATETIME,           -- NULL = live; set = archived history
    source_label TEXT    NOT NULL,   -- file / collection / calendar display name
    source_file  TEXT,               -- markdown relative path
    note_id      INTEGER,
    calendar     TEXT,
    created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
);

-- One live row per origin key. The upsert (reconcile runs concurrently from
-- several resources) must be race-free, so this partial unique index is the
-- conflict target rather than a plain UNIQUE that generation would defeat.
-- Archived history rows are exempt, so re-completions accumulate separately.
-- The DELETE first clears any duplicates the old non-atomic upsert left behind.
DELETE FROM done_items WHERE removed_at IS NULL AND id NOT IN (
    SELECT MIN(id) FROM done_items WHERE removed_at IS NULL
    GROUP BY username, origin, origin_key
);
CREATE UNIQUE INDEX idx_done_items_live_unique
    ON done_items(username, origin, origin_key) WHERE removed_at IS NULL;

CREATE INDEX idx_done_items_live  ON done_items(username, removed_at, completed_at DESC);
CREATE INDEX idx_done_items_scope ON done_items(username, origin, scope_id, removed_at);

-- Per-source clean-up settings plus the reconcile watermark for the one-time
-- backfill (reconciled_at IS NULL -> never scanned).
CREATE TABLE done_scopes (
    username        TEXT    NOT NULL,
    origin          TEXT    NOT NULL CHECK (origin IN ('markdown','note','caldav')),
    scope_id        TEXT    NOT NULL,
    auto_remove     BOOLEAN NOT NULL DEFAULT 0,
    keep_newest     INTEGER,         -- retention: NULL = keep all
    older_than_days INTEGER,         -- retention: NULL = off
    reconciled_at   DATETIME,        -- NULL = never backfilled
    PRIMARY KEY (username, origin, scope_id)
);

-- Per-user archive settings. archive_retention_days NULL = keep archived items
-- indefinitely; otherwise archived rows older than this are pruned.
CREATE TABLE done_settings (
    username               TEXT PRIMARY KEY,
    archive_retention_days INTEGER,
    updated_at             DATETIME NOT NULL DEFAULT (datetime('now'))
);
