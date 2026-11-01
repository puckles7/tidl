-- Note labels (backlog #114): first-class, per-user labels linked to notes
-- many-to-many. Mirrors Google Keep's model -- a user owns a set of labels and
-- any note may carry any subset -- which the Keep importer's old
-- "first label -> collection" mapping could not represent (it dropped the
-- rest). `owner_username` is the note's *author*, not a viewer: labels belong
-- to the note.

CREATE TABLE note_labels (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT NOT NULL REFERENCES users(username),
    name           TEXT NOT NULL,
    color          TEXT NOT NULL,
    created_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    UNIQUE(owner_username, name)
);

CREATE TABLE note_label_links (
    note_id  INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    label_id INTEGER NOT NULL REFERENCES note_labels(id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, label_id)
);

CREATE INDEX idx_note_label_links_label ON note_label_links (label_id);
