# Calendar vertical zoom / smarter default scroll position

The week/day grid renders the full 24-hour day at a fixed 1px-per-minute
scale (1440px tall, see `week_grid.rs`) and on mount scrolls to center the
*current* time in the viewport. For users whose events cluster in the
morning, that fixed scale means only a few hours are visible at once, and
centering on "now" (rather than on where events actually are) doesn't
reliably land on the busiest part of the day. Two independent changes
would help, either alone or together:

1. **Vertical zoom** — let the user compress/expand the grid's
   px-per-minute scale so more (or less) of the day fits in the viewport
   without scrolling.
2. **Smarter default scroll position** — instead of (or in addition to)
   centering on the current time, default the initial scroll to a fixed
   window (e.g. 6–7am) that catches most events by default, since that's
   likely to matter more than "where is now" when the calendar first
   loads.

## Context / Background

- Relevant code: `crates/tidl-web/src/components/calendar/week_grid.rs`
  — `scroll_ref`, the mount-time `Effect` that sets `scroll_top`
  (currently centers `local_now_minutes`), and the CSS driving the
  1440px-tall column body.
- The existing mount effect already has a comment explaining it only
  runs once so it "doesn't fight a scroll position the user chose" —
  any persisted zoom level or scroll preference should respect that
  same constraint.

## Functional Requirements

- Vertical zoom: a control (e.g. +/- buttons, or scroll-wheel +
  modifier key) to adjust the grid's px-per-minute scale within some
  min/max bound, without breaking the drag-to-create / drag-to-resize
  event math that currently assumes 1440px = 24h.
- Default scroll position: on first mount, scroll to a fixed early-morning
  time (e.g. 6am, or 6–7am window) rather than centering "now" — decide
  whether this fully replaces the "center now" behavior or only applies
  when no events are near the current time.
- Whichever preference(s) are added (zoom level, and/or default scroll
  time) should probably persist per-user, similar to how other calendar
  display preferences are stored (see `UserPrefsCtx`).

## Open Questions

- Should zoom level persist across sessions (stored in user prefs / DB)
  or just for the current browser session?
- Does the "center now" behavior get replaced entirely by the fixed
  6–7am default, or only used as a fallback?
- Should zoom apply to both Week and Day views, or Day view only (Week
  view is already visually denser)?

## Resolution

Both halves implemented (`week_grid.rs`, `assets/style.css`).

**Vertical zoom.** The grid's px-per-hour scale is now a single value exposed
to CSS as `--hour-h` on `.week-grid-scroll`, replacing every hardcoded 60px /
1440px assumption: `.week-hour-label` and `.week-hour-slot` heights, the
`calc(24 * var(--hour-h))` body height, each slot's inline `top`, and the
now-line's `top` (`minutes / 60 * --hour-h`). Event blocks already use
percentages, so they scale for free. The scale is adjustable from **24–120
px/hour** on eight discrete steps via **−/+ buttons** in the week nav and
**Ctrl/⌘ + mouse wheel** over the grid; plain wheel keeps scrolling normally.
Zooming holds the time currently at the top of the viewport fixed (a
`pending_scroll` signal is applied by a post-render effect once the new height
is laid out), rather than jumping to a different part of the day.

**Persistence.** The level is stored in **localStorage**
(`tidl-cal-hour-h`), read once after hydration and written on every change.
This deviates from the spec's suggested per-user `UserPrefs`/DB storage: no
migration was needed and the preference is per-browser.

**Drag/click math.** `client_y_to_mins` and the click-to-create handler now
convert pixels to minutes through the live scale (`px / --hour-h * 60`)
instead of assuming 1px = 1 minute, so drag-to-move, drag-to-resize, and
click-empty-slot-to-create stay correct at any zoom.

**Default scroll.** On first mount the grid scrolls so **7:00** sits at the top
of the viewport, replacing the old center-on-now behaviour entirely (not a
fallback), per the spec's primary suggestion. It still only runs once, so it
never fights a user-chosen scroll position; the consumed `--hour-h` is read
untracked so a zoom change never re-triggers it.

Scope: zoom applies to the timed Day / Work-week / Week grid; the Month view is
unaffected (no hour scale). The open questions above were resolved as: persist
per-browser; fixed default replaces center-now; zoom applies to all timed
views.
