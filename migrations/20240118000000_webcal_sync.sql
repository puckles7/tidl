-- Webcal URL subscriptions (#15)
-- A calendar may optionally have a sync URL attached; when set the server
-- fetches the remote .ics feed periodically and the calendar becomes read-only.

ALTER TABLE calendars ADD COLUMN sync_url          TEXT;
ALTER TABLE calendars ADD COLUMN sync_etag         TEXT;
ALTER TABLE calendars ADD COLUMN sync_last_modified TEXT;
ALTER TABLE calendars ADD COLUMN last_synced_at    DATETIME;
ALTER TABLE calendars ADD COLUMN sync_error        TEXT;
