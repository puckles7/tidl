-- Generalize share_invitations to support note-collection invites alongside
-- calendar invites (backlog #64). SQLite can't alter a CHECK constraint, so
-- rebuild the table (same pattern as
-- migrations/20240119000000_drop_events_calendar_fk.sql).

PRAGMA foreign_keys = OFF;

CREATE TABLE share_invitations_new (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT     NOT NULL,
    resource_type  TEXT     NOT NULL CHECK (resource_type IN ('calendar', 'note_collection')),
    calendar_name  TEXT,
    collection_id  INTEGER REFERENCES note_collections(id) ON DELETE CASCADE,
    invitee_email  TEXT     NOT NULL,
    permission     TEXT     NOT NULL CHECK (permission IN ('read', 'read-write', 'member')),
    token          TEXT     NOT NULL UNIQUE,
    expires_at     DATETIME NOT NULL,
    status         TEXT     NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'declined')),
    created_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    CHECK (
        (resource_type = 'calendar'        AND calendar_name IS NOT NULL AND collection_id IS NULL)
        OR
        (resource_type = 'note_collection' AND collection_id IS NOT NULL AND calendar_name IS NULL)
    )
);

INSERT INTO share_invitations_new
    (id, owner_username, resource_type, calendar_name, collection_id,
     invitee_email, permission, token, expires_at, status, created_at)
SELECT id, owner_username, 'calendar', calendar_name, NULL,
       invitee_email, permission, token, expires_at, status, created_at
FROM share_invitations;

DROP TABLE share_invitations;
ALTER TABLE share_invitations_new RENAME TO share_invitations;

PRAGMA foreign_keys = ON;
