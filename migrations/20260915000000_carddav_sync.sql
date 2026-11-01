-- CardDAV protocol sync support (backlog #29): per-user address-book sync
-- token + change log, mirroring calendars.sync_token / sync_log
-- (migrations/20240109000000_sync_tokens.sql) for RFC 6578 sync-collection.

CREATE TABLE address_books (
    owner_username TEXT    NOT NULL PRIMARY KEY REFERENCES users(username),
    sync_token     INTEGER NOT NULL DEFAULT 0
);

-- Backfill one row per existing user so lookups never need a separate
-- "ensure" step before the first vCard write bumps sync_token.
INSERT INTO address_books (owner_username) SELECT username FROM users;

CREATE TABLE contact_sync_log (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT    NOT NULL REFERENCES users(username),
    uid            TEXT    NOT NULL,
    operation      TEXT    NOT NULL, -- "modified" or "deleted"
    token          INTEGER NOT NULL
);

CREATE INDEX idx_contact_sync_log_owner_token ON contact_sync_log (owner_username, token);
