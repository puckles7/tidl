-- Add OIDC identity columns to users.
-- NULL means local-only account; non-null means the user can log in via OIDC.
ALTER TABLE users ADD COLUMN oidc_subject TEXT;
ALTER TABLE users ADD COLUMN oidc_issuer  TEXT;
CREATE UNIQUE INDEX idx_users_oidc ON users (oidc_subject, oidc_issuer)
    WHERE oidc_subject IS NOT NULL;

-- Short-lived table holding state during an in-flight OIDC Authorization Code flow.
-- Rows are deleted on callback completion and swept on each new flow start.
CREATE TABLE oidc_pending (
    state          TEXT     NOT NULL PRIMARY KEY,
    pkce_secret    TEXT     NOT NULL,
    nonce_secret   TEXT     NOT NULL,
    -- Populated only when the callback detected an email collision (linking flow):
    pending_sub    TEXT,
    pending_issuer TEXT,
    pending_email  TEXT,
    pending_name   TEXT,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at     DATETIME NOT NULL
);
