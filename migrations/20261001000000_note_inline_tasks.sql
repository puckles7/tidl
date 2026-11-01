-- Inline tasks and awaits in notes (backlog #113): each `#task`/`#await` token
-- in a note body becomes its own VTODO event, so the note → event link is now
-- one-to-many. `events.note_id` replaces the old 1:1
-- `notes.mirrored_event_uid` join used by get_tasks()/get_awaits().
--
-- Nullable: only `source = 'note'` rows set it; CalDAV/markdown rows leave it
-- NULL. ON DELETE CASCADE mirrors `note_images` — deleting a note removes its
-- task events. The legacy notes.kind/priority/due_date/mirrored_event_uid
-- columns are migrated by a one-time Rust backfill at startup and dropped in a
-- follow-up cleanup item.
ALTER TABLE events ADD COLUMN note_id INTEGER REFERENCES notes(id) ON DELETE CASCADE;

CREATE INDEX idx_events_note_id ON events (note_id);
