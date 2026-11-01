-- Surface the last markdown source sync/pull error in the UI instead of only
-- logging it server-side.
ALTER TABLE markdown_sources ADD COLUMN last_error TEXT;
