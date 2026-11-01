-- Contacts tab: per-user address book (backlog #79)
--
-- grid_row/grid_col/grid_row_span mirror the Notes-tab masonry grid
-- (migrations/20260715000000_note_grid_position.sql,
-- 20260715000001_note_grid_row_span.sql) — the client-side grid math is
-- shared between the two features (see crates/tidl-web/src/grid.rs), so the
-- persisted-position model is kept identical.

CREATE TABLE contacts (
    id             INTEGER  PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT     NOT NULL REFERENCES users(username),
    uid            TEXT     NOT NULL UNIQUE,
    display_name   TEXT     NOT NULL,
    given_name     TEXT,
    family_name    TEXT,
    org            TEXT,
    title          TEXT,
    birthday       DATE,
    street         TEXT,
    city           TEXT,
    region         TEXT,
    postal_code    TEXT,
    country        TEXT,
    notes          TEXT,
    etag           TEXT     NOT NULL,
    grid_row       INTEGER  NOT NULL DEFAULT 0,
    grid_col       INTEGER  NOT NULL DEFAULT 0,
    grid_row_span  INTEGER  NOT NULL DEFAULT 1,
    created_at     DATETIME NOT NULL DEFAULT (datetime('now')),
    updated_at     DATETIME NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_contacts_owner ON contacts (owner_username);

CREATE TABLE contact_emails (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    value      TEXT    NOT NULL,
    type       TEXT    NOT NULL DEFAULT 'other' CHECK (type IN ('home', 'work', 'mobile', 'other')),
    is_primary BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX idx_contact_emails_contact ON contact_emails (contact_id);

CREATE TABLE contact_phones (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_id INTEGER NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
    value      TEXT    NOT NULL,
    type       TEXT    NOT NULL DEFAULT 'other' CHECK (type IN ('home', 'work', 'mobile', 'other')),
    is_primary BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX idx_contact_phones_contact ON contact_phones (contact_id);
