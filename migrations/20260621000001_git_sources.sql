ALTER TABLE markdown_sources ADD COLUMN pat_encrypted  TEXT;
ALTER TABLE markdown_sources ADD COLUMN config_managed INTEGER NOT NULL DEFAULT 0;
