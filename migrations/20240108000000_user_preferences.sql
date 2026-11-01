-- Per-user display preferences.
--
-- `timezone`    IANA timezone identifier (e.g. "America/New_York", "UTC").
-- `time_format` "12h" | "24h" | "zulu"

CREATE TABLE IF NOT EXISTS user_preferences (
    username    TEXT     PRIMARY KEY REFERENCES users(username) ON DELETE CASCADE,
    timezone    TEXT     NOT NULL DEFAULT 'UTC',
    time_format TEXT     NOT NULL DEFAULT '12h',
    updated_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
