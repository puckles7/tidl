-- One-off repair for calendars whose sync_log gained stale `deleted` entries
-- at a higher token than the surviving `modified` entry for the same event.
--
-- Cause: deleting a calendar (api/calendars.rs::delete_calendar_api or
-- Storage::delete_calendar) removed its events and calendars row but left the
-- calendar's sync_log rows behind. Recreating the calendar reset its
-- sync_token to the column default (1), so the orphaned tombstones from the
-- calendar's previous life outranked the newly imported `modified` entries.
-- get_sync_changes keeps the highest-token operation per uid, so CalDAV
-- clients replayed a `deleted` for events that still exist in `events` and
-- hid them, while the web UI (which reads `events` directly) still showed them.
--
-- For every calendar that has at least one event currently present whose
-- latest logged operation is `deleted`, advance that calendar's sync_token
-- past every existing log entry and re-affirm the event as `modified` at the
-- new token. Clients then receive a 200 for it and re-add it.
--
-- CardDAV is unaffected: address_books/contact_sync_log are keyed by
-- contact_collections.id (AUTOINCREMENT) and cascade away with the collection
-- on delete, so they cannot retain stale tombstones across a delete/recreate.

-- 1) Advance each affected calendar's token above ALL its existing log rows.
--    MAX(existing, max_log) + 1 (not just +1) is required: a recreated
--    calendar can start at token 2 while orphaned tombstones sit at a much
--    higher token, which a plain increment would not outrank.
UPDATE calendars
SET sync_token = MAX(
        calendars.sync_token,
        COALESCE((SELECT MAX(s.token) FROM sync_log s
                  WHERE s.username = calendars.username
                    AND s.calendar = calendars.name), 0)
    ) + 1
WHERE EXISTS (
    SELECT 1 FROM events e
    WHERE e.username = calendars.username
      AND e.calendar = calendars.name
      AND (SELECT s.operation FROM sync_log s
           WHERE s.username = e.username AND s.calendar = e.calendar AND s.uid = e.uid
           ORDER BY s.token DESC, s.id DESC LIMIT 1) = 'deleted'
);

-- 2) Re-affirm every present event whose latest logged op is `deleted`, at
--    the calendar's freshly advanced token.
INSERT INTO sync_log (username, calendar, uid, operation, token)
SELECT e.username, e.calendar, e.uid, 'modified', c.sync_token
FROM events e
JOIN calendars c ON c.username = e.username AND c.name = e.calendar
WHERE (SELECT s.operation FROM sync_log s
       WHERE s.username = e.username AND s.calendar = e.calendar AND s.uid = e.uid
       ORDER BY s.token DESC, s.id DESC LIMIT 1) = 'deleted';
