-- Dead WebDAV properties storage (RFC 4918 §4.2)
CREATE TABLE IF NOT EXISTS properties (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL,
    resource_path TEXT NOT NULL,  -- e.g. /caldav/admin/litmus/prop
    namespace TEXT NOT NULL,      -- XML namespace URI
    name TEXT NOT NULL,           -- local property name
    value TEXT,                   -- XML string value (may be NULL for removal)
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(username, resource_path, namespace, name)
);

CREATE INDEX IF NOT EXISTS idx_properties_resource ON properties(username, resource_path);
