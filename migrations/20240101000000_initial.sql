-- Initial schema for tidl

-- Calendars table
CREATE TABLE IF NOT EXISTS calendars (
                                         id INTEGER PRIMARY KEY AUTOINCREMENT,
                                         username TEXT NOT NULL,
                                         name TEXT NOT NULL,
                                         display_name TEXT,
                                         description TEXT,
                                         color TEXT,
                                         created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                                         updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                                         UNIQUE(username, name)
    );

-- Events and todos table
CREATE TABLE IF NOT EXISTS events (
                                      id INTEGER PRIMARY KEY AUTOINCREMENT,
                                      username TEXT NOT NULL,
                                      calendar TEXT NOT NULL,
                                      uid TEXT NOT NULL,
                                      type TEXT NOT NULL, -- 'VEVENT' or 'VTODO'
                                      summary TEXT,
                                      description TEXT,
                                      location TEXT,
                                      start_date DATETIME,
                                      end_date DATETIME,
                                      due_date DATETIME,
                                      priority INTEGER, -- For todos: 0-999999
                                      status TEXT, -- NEEDS-ACTION, IN-PROCESS, COMPLETED, CANCELLED
                                      completed_date DATETIME,
                                      etag TEXT NOT NULL,
                                      ical_data TEXT NOT NULL,
                                      source TEXT, -- 'caldav', 'markdown', etc.
                                      source_file TEXT, -- For markdown-sourced items
                                      created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                                      updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                                      UNIQUE(username, calendar, uid),
    FOREIGN KEY (username, calendar) REFERENCES calendars(username, name)
    );

-- Index for fast queries
CREATE INDEX IF NOT EXISTS idx_events_calendar ON events(username, calendar);
CREATE INDEX IF NOT EXISTS idx_events_type ON events(type);
CREATE INDEX IF NOT EXISTS idx_events_start_date ON events(start_date);
CREATE INDEX IF NOT EXISTS idx_events_due_date ON events(due_date);
CREATE INDEX IF NOT EXISTS idx_events_priority ON events(priority);
CREATE INDEX IF NOT EXISTS idx_events_source ON events(source, source_file);

-- Markdown sources configuration
CREATE TABLE IF NOT EXISTS markdown_sources (
                                                id INTEGER PRIMARY KEY AUTOINCREMENT,
                                                username TEXT NOT NULL,
                                                name TEXT NOT NULL,
                                                type TEXT NOT NULL, -- 'filesystem', 'dropbox', 'onedrive', etc.
                                                config TEXT NOT NULL, -- JSON configuration
                                                enabled BOOLEAN DEFAULT TRUE,
                                                last_scan DATETIME,
                                                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                                                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                                                UNIQUE(username, name)
    );

-- Sync state tracking
CREATE TABLE IF NOT EXISTS sync_state (
                                          id INTEGER PRIMARY KEY AUTOINCREMENT,
                                          source_id INTEGER NOT NULL,
                                          file_path TEXT NOT NULL,
                                          file_hash TEXT NOT NULL,
                                          last_synced DATETIME DEFAULT CURRENT_TIMESTAMP,
                                          UNIQUE(source_id, file_path),
    FOREIGN KEY (source_id) REFERENCES markdown_sources(id) ON DELETE CASCADE
    );

CREATE INDEX IF NOT EXISTS idx_sync_state_source ON sync_state(source_id);