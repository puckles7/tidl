-- Note image / attachment support (backlog #110)
--
-- Images live in a child table rather than a column on `notes` because a note
-- has many, with an explicit display order. `ON DELETE CASCADE` removes a
-- note's images when the note is deleted (SQLite's foreign_keys pragma is on).

CREATE TABLE note_images (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id      INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    data         BLOB NOT NULL,
    content_type TEXT NOT NULL,
    filename     TEXT,
    position     INTEGER NOT NULL DEFAULT 0,
    created_at   DATETIME NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_note_images_note ON note_images (note_id);
