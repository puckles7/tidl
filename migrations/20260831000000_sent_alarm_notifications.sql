-- Durable dedup state for server-side EMAIL alarm dispatch (backlog #73).
--
-- `occurrence_start` disambiguates recurring-event occurrences (compact
-- "YYYYMMDD"/"YYYYMMDDTHHMMSS" key, matching `Event::occurrence_start`) and
-- also encodes REPEAT/DURATION repeats as a "#<repeat_index>" suffix (0 for
-- the base firing) — see `alarm::dispatch::occurrence_key`.
--
-- `username` is part of the uniqueness key because `event_uid` is only
-- unique per (username, calendar), not globally.
CREATE TABLE IF NOT EXISTS sent_alarm_notifications (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    username         TEXT NOT NULL,
    event_uid        TEXT NOT NULL,
    alarm_index      INTEGER NOT NULL,
    occurrence_start TEXT NOT NULL,
    sent_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(username, event_uid, alarm_index, occurrence_start)
);
