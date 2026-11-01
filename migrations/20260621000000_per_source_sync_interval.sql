ALTER TABLE remote_caldav_sources
    ADD COLUMN sync_interval_secs INTEGER NOT NULL DEFAULT 300;
