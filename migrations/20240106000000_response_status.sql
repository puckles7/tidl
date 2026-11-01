-- Add response_status column to store the attendee participation status
-- (e.g. ACCEPTED, TENTATIVE, DECLINED, NOT_RESPONDED, ORGANIZER)
-- This mirrors the PARTSTAT parameter on ATTENDEE lines in RFC 5545 iCalendar data.
ALTER TABLE events ADD COLUMN response_status VARCHAR;
