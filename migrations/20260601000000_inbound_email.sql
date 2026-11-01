CREATE TABLE inbound_email_tokens (
    id           TEXT    NOT NULL PRIMARY KEY,
    username     TEXT    NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    calendar_id  INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    label        TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL,
    address      TEXT    NOT NULL,
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at DATETIME
);
CREATE INDEX inbound_email_tokens_hash ON inbound_email_tokens(token_hash);

CREATE TABLE trusted_sender_addresses (
    id         TEXT    NOT NULL PRIMARY KEY,
    username   TEXT    NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    email      TEXT    NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(username, email)
);
CREATE INDEX trusted_sender_addresses_username ON trusted_sender_addresses(username);
