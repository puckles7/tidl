-- Contact collections (backlog #81): splits the single flat per-user
-- address book (#79) into named, colorable, shareable collections —
-- structurally identical to note_collections/note_collection_members
-- (migrations/20260710000000_notes.sql).

CREATE TABLE contact_collections (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT NOT NULL REFERENCES users(username),
    name           TEXT NOT NULL,
    color          TEXT NOT NULL,
    shared         BOOLEAN NOT NULL DEFAULT 0,
    created_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    updated_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    UNIQUE(owner_username, name)
);

CREATE TABLE contact_collection_members (
    collection_id   INTEGER NOT NULL REFERENCES contact_collections(id) ON DELETE CASCADE,
    member_username TEXT NOT NULL REFERENCES users(username),
    PRIMARY KEY (collection_id, member_username)
);

CREATE INDEX idx_contact_collection_members_username ON contact_collection_members (member_username);

-- Every existing user gets one default collection, even with zero
-- contacts today (mirrors address_books' own "backfill one row per user"
-- rationale in migrations/20260915000000_carddav_sync.sql) — no user is
-- ever collection-less.
INSERT INTO contact_collections (owner_username, name, color)
SELECT username, 'My Contacts', '#3b82f6' FROM users;

-- contacts.collection_id: nullable at the schema level (SQLite can't add a
-- NOT NULL column with a per-row computed default in one ALTER, and a full
-- table rebuild is riskier than necessary here since contacts carries a
-- photo BLOB column) — backfilled to every existing user's new default
-- collection, and enforced non-null in practice by every INSERT-path this
-- backlog item touches (api/contacts.rs::create_contact,
-- storage.rs::store_contact_vcard, seed_demo.rs).
ALTER TABLE contacts ADD COLUMN collection_id INTEGER REFERENCES contact_collections(id);

UPDATE contacts
SET collection_id = (
    SELECT cc.id FROM contact_collections cc
    WHERE cc.owner_username = contacts.owner_username AND cc.name = 'My Contacts'
);

CREATE INDEX idx_contacts_collection ON contacts (collection_id);
