-- WebDAV locks table (RFC 4918 §7)
CREATE TABLE IF NOT EXISTS locks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL UNIQUE,           -- opaquelocktoken UUID
    username TEXT NOT NULL,               -- owner (authenticated user)
    resource_path TEXT NOT NULL,          -- e.g. /caldav/admin/litmus/lockme
    scope TEXT NOT NULL DEFAULT 'exclusive', -- 'exclusive' or 'shared'
    depth TEXT NOT NULL DEFAULT '0',      -- '0' or 'infinity'
    owner_info TEXT,                      -- XML owner info from client
    timeout_secs INTEGER NOT NULL DEFAULT 3600,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_locks_resource ON locks(resource_path);
CREATE INDEX IF NOT EXISTS idx_locks_token ON locks(token);
