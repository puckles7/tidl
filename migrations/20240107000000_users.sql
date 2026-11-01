-- Database-backed user accounts.
--
-- Replaces the static [auth.users] list in config.toml.
-- - `username`  is the short slug used in CalDAV paths (/caldav/{username}/…)
--               and as the key in the calendars/events tables.
-- - `email`     is the login identifier (case-insensitive, stored lowercase).
-- - `password_hash` is an argon2id PHC string; legacy bcrypt hashes starting
--               with $2b$/$2y$ are accepted until the user changes their
--               password.
-- - `is_admin`  allows managing other users.
-- - `is_active` soft-delete: inactive users cannot authenticate.

CREATE TABLE IF NOT EXISTS users (
    id            INTEGER  PRIMARY KEY AUTOINCREMENT,
    username      TEXT     NOT NULL UNIQUE,
    email         TEXT     NOT NULL UNIQUE,
    display_name  TEXT     NOT NULL DEFAULT '',
    password_hash TEXT     NOT NULL,
    is_admin      BOOLEAN  NOT NULL DEFAULT 0,
    is_active     BOOLEAN  NOT NULL DEFAULT 1,
    created_at    DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at    DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_users_email    ON users (email);
CREATE INDEX IF NOT EXISTS idx_users_username ON users (username);
