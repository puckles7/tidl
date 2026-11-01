-- Remote CalDAV sources: per-user configuration for syncing with a remote
-- tidl (or any CalDAV) server.
--
-- `remote_token_enc` stores the app token encrypted with the server secret
-- using AES-256-GCM in the same "{nonce_b64}:{ct_b64}" format used for
-- the SMTP password.
--
-- `sync_direction`: "pull" | "push" | "both"
-- `last_sync_token`: the RFC 6578 sync-token returned by the remote server
--   after the last successful sync.  NULL means "full sync needed".

CREATE TABLE remote_caldav_sources (
    id                TEXT NOT NULL PRIMARY KEY,      -- UUID
    username          TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    name              TEXT NOT NULL,                  -- user-provided label
    remote_base_url   TEXT NOT NULL,                  -- e.g. https://personal.example.com
    remote_username   TEXT NOT NULL,                  -- username on the remote server
    remote_token_enc  TEXT NOT NULL,                  -- encrypted app token
    local_calendar    TEXT NOT NULL,                  -- calendar slug on this instance
    sync_direction    TEXT NOT NULL DEFAULT 'both',   -- "pull" | "push" | "both"
    last_sync_token   TEXT,                           -- RFC 6578 sync token
    last_push_token   INTEGER,                        -- local sync_log token at last push
    last_synced_at    DATETIME,
    last_error        TEXT,                           -- last sync error message, if any
    enabled           INTEGER NOT NULL DEFAULT 1,
    created_at        DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX remote_caldav_sources_username ON remote_caldav_sources(username);
