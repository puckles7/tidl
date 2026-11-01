-- Add read-only mirror support for sync agent calendars.
-- read_only: when true, only sync_user may write via CalDAV.
-- sync_user: the username permitted to write to a read-only calendar.
ALTER TABLE calendars ADD COLUMN read_only BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE calendars ADD COLUMN sync_user TEXT;
