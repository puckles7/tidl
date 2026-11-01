# Screen toolbars wrap on narrow windows and stay visible on mobile

The top toolbars on Calendar, Focus, Done, Notes, and Contacts are single-row
flex layouts. As the browser window narrows from a wide desktop, their
contents stop fitting inside the main content column while the layout is still
in desktop mode (sidebars stay beside the content until the 767px breakpoint).
The overflow pushes controls out past the right edge of the screen, and the
page gains a horizontal scrollbar.

- **Calendar** (`.cal-pill-row`): today's date on the left; the Day / Work
  week / Week / Month pills plus the event search box on the right. On mobile
  a `max-width: 767px` rule hid the whole row, so the mode pills and search
  were unavailable.
- **Focus / Done** (`.screen-toolbar`): the left add/clean-up action group and
  a right `.focus-right-controls` group containing two sets of pills plus the
  search box. The outer row already wrapped, but the control group itself did
  not, so it overflowed once the group was wider than the main column.
- **Notes / Contacts** (`.notes-toolbar`): a run of `+ New`, Auto-layout, Keep
  this layout, Clear all, Starred/Merge, and selection actions plus the search
  box. The row never wrapped, so it overflowed even at fairly wide desktop
  widths.

## Context / Background

- Toolbar markup lives in `components/calendar/mod.rs`, `focus.rs`, `done.rs`,
  `notes/mod.rs`, and `contacts/mod.rs`; layout rules are in `assets/style.css`.
- The notes/contacts/Focus/calendar search inputs share `.notes-search`
  (`flex: 0 1 260px; min-width: 60px`), while the surrounding buttons are
  fixed-width (`flex-shrink: 0`, `white-space: nowrap`).
- With the 18rem sidebar and page/view padding, the main column is already
  narrow (~480px) around a 900px viewport — above the mobile breakpoint — so
  the toolbars overflow well before mobile mode.

## Functional Requirements

- No toolbar may overflow its main content column or cause a page-level
  horizontal scrollbar at any supported width.
- When the controls no longer fit on one line, they should wrap onto extra
  lines, growing the toolbar downwards instead of pushing off-screen.
- Wrapping should happen only once the controls genuinely cannot fit: the
  search box should shrink first, then the pills/buttons and search wrap.
- Calendar's controls must remain visible in mobile mode, above the calendar
  and below the filter blocks (Month / Calendars panels).

## Resolution

- **Search inputs** (`.notes-search`) now use `flex: 1 1 60px` with
  `max-width: 260px`: the 60px basis is what flex line-breaking measures, so
  the search absorbs shrinking and only forces a wrap once even its 60px floor
  cannot share the line; it grows back to 260px when space allows.
- **Calendar** (`.cal-pill-row`) has `flex-wrap: wrap` and a tighter row gap;
  its control group is sized with `flex: 1 1 min-content` and right-aligned so
  it stays on one line as long as possible before dropping below the date. On
  the narrowest phones (≤420px) the group itself wraps, dropping the search
  below the pills. The mobile `display: none` rule was removed, so the
  controls stay visible between the filter panels and the week grid.
- **Focus / Done** (`.screen-toolbar > .focus-right-controls`) now wraps its
  own children with an 8px row gap, so the two pill groups and search stack
  when the main column is too narrow, instead of overflowing.
- **Notes / Contacts** (`.notes-toolbar`) now has `flex-wrap: wrap`, so the
  action buttons and search form additional rows rather than running off the
  right edge.

Verified with headless Chromium against the real stylesheet for all five
screens at 2400 down to 320px: no horizontal page overflow at any width, and
each toolbar grows vertically instead. Calendar's controls sit below the
sidebar and above the grid in mobile mode at every tested width.
