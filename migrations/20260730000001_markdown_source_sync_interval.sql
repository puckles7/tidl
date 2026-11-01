-- 60s matches the poll cadence markdown sources already synced at, so
-- existing rows keep their current behaviour after this migration.
ALTER TABLE markdown_sources ADD COLUMN sync_interval_secs INTEGER NOT NULL DEFAULT 60;
