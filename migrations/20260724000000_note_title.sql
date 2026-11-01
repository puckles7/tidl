-- Optional note title (backlog #66): shown bold on the Notes-tab card, and
-- used verbatim as the mirrored VTODO's SUMMARY in Tasks/Focus/Await when
-- set, instead of the auto-truncated note text.
ALTER TABLE notes ADD COLUMN title TEXT;
