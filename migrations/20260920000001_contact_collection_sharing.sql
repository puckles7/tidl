-- Generalize share_invitations to support contact-collection invites
-- alongside calendar and note-collection invites (backlog #81). Same
-- rebuild approach as migrations/20260723000000_generalize_share_invitations.sql
-- (SQLite can't ALTER a CHECK constraint).

PRAGMA foreign_keys = OFF;

CREATE TABLE share_invitations_new (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username        TEXT     NOT NULL,
    resource_type         TEXT     NOT NULL CHECK (resource_type IN ('calendar', 'note_collection', 'contact_collection')),
    calendar_name         TEXT,
    collection_id         INTEGER REFERENCES note_collections(id) ON DELETE CASCADE,
    contact_collection_id INTEGER REFERENCES contact_collections(id) ON DELETE CASCADE,
    invitee_email         TEXT     NOT NULL,
    permission            TEXT     NOT NULL CHECK (permission IN ('read', 'read-write', 'member')),
    token                 TEXT     NOT NULL UNIQUE,
    expires_at            DATETIME NOT NULL,
    status                TEXT     NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'declined')),
    created_at            DATETIME NOT NULL DEFAULT (datetime('now')),
    CHECK (
        (resource_type = 'calendar'           AND calendar_name IS NOT NULL AND collection_id IS NULL AND contact_collection_id IS NULL)
        OR
        (resource_type = 'note_collection'    AND collection_id IS NOT NULL AND calendar_name IS NULL AND contact_collection_id IS NULL)
        OR
        (resource_type = 'contact_collection' AND contact_collection_id IS NOT NULL AND calendar_name IS NULL AND collection_id IS NULL)
    )
);

INSERT INTO share_invitations_new
    (id, owner_username, resource_type, calendar_name, collection_id, contact_collection_id,
     invitee_email, permission, token, expires_at, status, created_at)
SELECT id, owner_username, resource_type, calendar_name, collection_id, NULL,
       invitee_email, permission, token, expires_at, status, created_at
FROM share_invitations;

DROP TABLE share_invitations;
ALTER TABLE share_invitations_new RENAME TO share_invitations;

PRAGMA foreign_keys = ON;
