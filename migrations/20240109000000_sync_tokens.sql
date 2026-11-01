-- Per-collection sync counter and change log for WebDAV sync (RFC 6578) and getctag.
--
-- sync_token: monotonically increasing counter on the calendars row; bumped on
--   every store_event / delete_event. Used for both DAV:sync-token and CS:getctag.
-- sync_log:   append-only log of every change, keyed by (username, calendar, token).
--   Clients that already know a previous token can receive only the delta.

ALTER TABLE calendars ADD COLUMN sync_token INTEGER NOT NULL DEFAULT 1;

CREATE TABLE IF NOT EXISTS sync_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    username    TEXT    NOT NULL,
    calendar    TEXT    NOT NULL,
    uid         TEXT    NOT NULL,
    operation   TEXT    NOT NULL, -- 'modified' or 'deleted'
    token       INTEGER NOT NULL,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_sync_log ON sync_log(username, calendar, token);
