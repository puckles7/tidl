-- Notes tab: collections, ad-hoc notes, to-do/tickler coupling (backlog #37)

CREATE TABLE note_collections (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT NOT NULL REFERENCES users(username),
    name           TEXT NOT NULL,
    color          TEXT NOT NULL,
    shared         BOOLEAN NOT NULL DEFAULT 0,
    created_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    updated_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    UNIQUE(owner_username, name)
);

CREATE TABLE note_collection_members (
    collection_id   INTEGER NOT NULL REFERENCES note_collections(id) ON DELETE CASCADE,
    member_username TEXT NOT NULL REFERENCES users(username),
    PRIMARY KEY (collection_id, member_username)
);

CREATE INDEX idx_note_collection_members_username ON note_collection_members (member_username);

CREATE TABLE notes (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id      INTEGER NOT NULL REFERENCES note_collections(id) ON DELETE CASCADE,
    author_username    TEXT NOT NULL REFERENCES users(username),
    text               TEXT NOT NULL,
    pinned             BOOLEAN NOT NULL DEFAULT 0,
    is_todo            BOOLEAN NOT NULL DEFAULT 0,
    priority           INTEGER,
    calendar_name      TEXT,
    tickler_date       DATE,
    mirrored_event_uid TEXT,
    created_at         DATETIME NOT NULL DEFAULT (datetime('now')),
    updated_at         DATETIME NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_notes_collection ON notes (collection_id);
CREATE INDEX idx_notes_mirrored_uid ON notes (mirrored_event_uid);
