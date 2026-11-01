-- App tokens: per-user named credentials for CalDAV client access.
--
-- Tokens are never stored in plaintext. The `token_hash` column holds a
-- SHA-256 hex digest of the raw token string so that the token can be
-- validated in constant time without exposing the original value.
--
-- Token format: "tidl_" + 64 hex chars (32 random bytes).
-- Scope: "read" (GET/PROPFIND/REPORT only) or "read-write" (all methods).

CREATE TABLE app_tokens (
    id           TEXT NOT NULL PRIMARY KEY,           -- UUID
    username     TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    name         TEXT NOT NULL,                       -- user-provided label
    token_hash   TEXT NOT NULL,                       -- SHA-256 hex of the raw token
    read_only    INTEGER NOT NULL DEFAULT 0,          -- 1 = read-only; 0 = read-write
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at DATETIME,
    expires_at   DATETIME                             -- NULL = never expires
);

CREATE INDEX app_tokens_username ON app_tokens(username);
CREATE INDEX app_tokens_hash     ON app_tokens(token_hash);
