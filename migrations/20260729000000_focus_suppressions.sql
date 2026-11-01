-- Permanent per-user Focus item suppressions (backlog #69). One row per
-- suppressed item_key. For tasks/awaits/non-recurring events, item_key is
-- the item's id (== events.uid, since tasks/awaits/markdown-origin items
-- are all rows in the events table too — see markdown_sync.rs). For a
-- recurring event, item_key is either the bare uid ("suppress the whole
-- series") or "{uid}|{occurrence_start}" ("suppress just this occurrence").
CREATE TABLE focus_suppressions (
    id         TEXT     NOT NULL PRIMARY KEY,   -- UUID
    username   TEXT     NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    item_key   TEXT     NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(username, item_key)
);

CREATE INDEX focus_suppressions_username ON focus_suppressions(username);
