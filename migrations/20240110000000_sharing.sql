-- calendar_shares: accepted share grants between users
CREATE TABLE calendar_shares (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username   TEXT    NOT NULL,
    calendar_name    TEXT    NOT NULL,
    grantee_username TEXT    NOT NULL,
    permission       TEXT    NOT NULL CHECK (permission IN ('read', 'read-write')),
    created_at       DATETIME NOT NULL DEFAULT (datetime('now')),
    UNIQUE(owner_username, calendar_name, grantee_username)
);

CREATE INDEX idx_shares_grantee ON calendar_shares (grantee_username);

-- share_invitations: pending email invitations (token links)
CREATE TABLE share_invitations (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT     NOT NULL,
    calendar_name  TEXT     NOT NULL,
    invitee_email  TEXT     NOT NULL,
    permission     TEXT     NOT NULL CHECK (permission IN ('read', 'read-write')),
    token          TEXT     NOT NULL UNIQUE,
    expires_at     DATETIME NOT NULL,
    status         TEXT     NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'declined')),
    created_at     DATETIME NOT NULL DEFAULT (datetime('now'))
);
