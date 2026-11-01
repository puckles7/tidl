-- CardDAV multi-address-book exposure (backlog #81): address_books and
-- contact_sync_log move from owner_username-keyed (one address book per
-- user) to collection_id-keyed (one per contact collection, owned or
-- shared) — see the design spec's CardDAV Protocol Layer section.
--
-- Deliberately NOT a data-preserving rebuild: the routing change this
-- accompanies (crates/tidl-server/src/carddav_handlers.rs, same backlog
-- item) already requires every CardDAV client to re-run discovery, so
-- there is no continuity worth migrating — every collection starts its
-- CardDAV sync state fresh at token 0.

DROP TABLE contact_sync_log;
DROP TABLE address_books;

CREATE TABLE address_books (
    collection_id INTEGER NOT NULL PRIMARY KEY REFERENCES contact_collections(id) ON DELETE CASCADE,
    sync_token    INTEGER NOT NULL DEFAULT 0
);

INSERT INTO address_books (collection_id) SELECT id FROM contact_collections;

CREATE TABLE contact_sync_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES contact_collections(id) ON DELETE CASCADE,
    uid           TEXT    NOT NULL,
    operation     TEXT    NOT NULL, -- "modified" or "deleted"
    token         INTEGER NOT NULL
);

CREATE INDEX idx_contact_sync_log_collection_token ON contact_sync_log (collection_id, token);
