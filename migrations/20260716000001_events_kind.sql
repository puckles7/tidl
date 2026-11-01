-- events.kind: explicit discriminator between TASK (self-actionable) and
-- AWAIT (date-gated, waiting on something external) VTODOs — backlog #49.
-- Single-column CHECK only (SQLite's ALTER TABLE ADD COLUMN cannot add a
-- constraint referencing other columns without a full table rebuild);
-- cross-field invariants (e.g. an await always having a due_date) are
-- enforced at the API layer instead, matching how this schema already
-- validates elsewhere (e.g. "Note text is required" in Rust, not SQL).
ALTER TABLE events ADD COLUMN kind TEXT NOT NULL DEFAULT 'task' CHECK (kind IN ('task', 'await'));

-- Backfill: a markdown row with a due_date but no priority came from the old
-- #pending tag (the two markdown tags were always mutually exclusive on
-- these two columns) — everything else defaults to 'task' already via the
-- column default above, so only the await case needs an explicit UPDATE.
UPDATE events
SET kind = 'await'
WHERE source = 'markdown' AND due_date IS NOT NULL AND priority IS NULL;

-- Note-origin rows: derive from the owning note's is_todo flag (still
-- present at migration time — the notes.kind migration that drops it runs
-- after this one).
UPDATE events
SET kind = 'await'
WHERE source = 'note'
  AND uid IN (
    SELECT mirrored_event_uid FROM notes
    WHERE mirrored_event_uid IS NOT NULL AND is_todo = 0
  );
