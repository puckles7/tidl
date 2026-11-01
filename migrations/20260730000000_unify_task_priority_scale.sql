-- Unify every task priority onto one canonical 1-100 scale (1 = highest),
-- shared by notes' slider, the Tasks-view creation slider, and markdown's
-- `#task/p<N>` tag; RFC 5545's native PRIORITY (1-9) is now only ever a
-- lossy wire-format projection applied at the CalDAV boundary — never
-- stored as an intermediate representation (backlog #70 / 2026-07-30
-- investigation). Existing rows predate this and are stored on three
-- different scales depending on `source`, so each needs its own
-- reinterpretation rather than a single blanket clamp:
--
--   * source = 'note'      — the old 0-15 "representative priority"
--                             quantizer (note_priority_to_task_priority),
--                             upscaled via the old inverse formula
--                             (min(p,20)*100/20) so existing note-mirrored
--                             VTODOs land on the same point Focus already
--                             scored them at.
--   * source = 'markdown'  — already documented as 1-100 going forward
--                             (the parser now clamps on parse), but rows
--                             synced before that fix may still hold a
--                             stale out-of-range value if their source
--                             file hasn't changed since — clamp in place.
--   * anything else        — RFC 5545's native 1-9 range (raw CalDAV
--                             client writes, e.g. Thunderbird, and the old
--                             Tasks-view dropdown's 1/5/9), upscaled via
--                             the new caldav_priority_to_source_priority
--                             formula: ((clamp(p,1,9)-1)*99/8)+1.
--
-- `notes.priority` (the note's own slider value, independent of its
-- mirrored VTODO) was already on a 0-100 scale — only its `0` needs
-- clamping to `1`, since the domain's floor moved from 0 to 1.

UPDATE events
SET priority = MAX(1, MIN(100, (MIN(priority, 20) * 100) / 20))
WHERE priority IS NOT NULL AND source = 'note';

UPDATE events
SET priority = MAX(1, MIN(100, priority))
WHERE priority IS NOT NULL AND source = 'markdown';

UPDATE events
SET priority = ((MAX(1, MIN(priority, 9)) - 1) * 99 / 8) + 1
WHERE priority IS NOT NULL AND (source IS NULL OR source NOT IN ('note', 'markdown'));

UPDATE notes
SET priority = 1
WHERE priority = 0;
