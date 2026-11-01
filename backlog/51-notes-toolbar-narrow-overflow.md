# Notes toolbar not responsive below ~390px (fixed-width search overflows)

Discovered during manual browser verification of backlog #50 (responsive notes
grid columns). Below roughly 390px of available content width, `.notes-toolbar`
(the "★ Starred" / "⊞ Auto-layout" buttons plus the `.notes-search` input) no
longer fits and overflows `.notes-main` — at the most extreme widths this
produces an actual page-level horizontal scrollbar. Confirmed unrelated to
backlog #50's grid-column work: the notes *grid* itself never overflows at any
width tested (down to 320px); the overflow is entirely attributable to the
toolbar's fixed-width elements not wrapping or shrinking.

## Dependencies

- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md) — the toolbar this affects

## Context / Background

`.notes-search` has a fixed `width: 260px` (`assets/style.css`), and the
toolbar (`.notes-toolbar` in `components/notes/mod.rs`) lays out the pin
filter, auto-layout, and search controls in a single non-wrapping row. At
narrow widths (verified around 390px and below, worse at 320px) this row's
combined width exceeds the available space, causing `.notes-main` — and, at
the extreme, the whole page — to overflow horizontally.

## Functional Requirements

- The Notes toolbar should never cause a horizontal scrollbar at any
  supported viewport width, matching the standard already set for the notes
  grid itself (backlog #50).
- Likely fix: allow the toolbar to wrap onto a second line below ~450px (or
  whatever breakpoint proves necessary), and/or let `.notes-search` shrink
  (e.g. `min-width` + `flex: 1` instead of a fixed `width: 260px`) rather than
  reserving its full desktop width unconditionally.

## Open Questions

- Confirm the exact breakpoint(s) needed by testing against real device
  widths (this was only checked at 750/500/390/320px CSS pixels in a headless
  browser), not just interpolating between them.

## Resolution

Fixed via the shrink approach (not wrapping): `.notes-pin-filter` (the
"★ Starred" / "⊞ Auto-layout" buttons) gained `flex-shrink: 0` +
`white-space: nowrap` so they stay a constant single-line height at every
width — previously the default `flex-shrink: 1` let them get squeezed down
to their text's min-content width, wrapping "Auto-layout" onto two lines and
growing the button taller (the user-reported symptom). `.notes-search`
changed from a fixed `width: 260px` to `flex: 0 1 260px` with `min-width:
60px`, so it's the one element that absorbs all the shrinking, down to a
60px floor. `.notes-toolbar` gained an explicit `gap: 6px` so there's always
a guaranteed minimum spacing between all three children, even with nothing
left for `justify-content: space-between` to distribute.

Verified in a real browser (isolated `TidlServer`, Playwright) at
1400/1000/750/500/420/390/360/320px: button height measured identically
(34.0px) at every width, `.notes-search`'s width shrinks smoothly from
260px down to the 60px floor, and the gap between adjacent elements never
dropped below 6px (comfortably above the requested 1-2px minimum) at any
width. No horizontal scrollbar at any tested width, matching backlog #50's
standard for the grid itself.
