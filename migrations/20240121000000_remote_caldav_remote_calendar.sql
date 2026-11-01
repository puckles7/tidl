-- Add a dedicated remote_calendar column that identifies which calendar
-- collection on the remote server to sync against.  Before this change,
-- local_calendar was used for both the remote URL path and the local
-- storage slug.
--
-- Backward compatibility: populate remote_calendar from local_calendar for
-- all existing rows so nothing breaks for already-configured sources.

ALTER TABLE remote_caldav_sources
    ADD COLUMN remote_calendar TEXT NOT NULL DEFAULT '';

UPDATE remote_caldav_sources
    SET remote_calendar = local_calendar
    WHERE remote_calendar = '';
