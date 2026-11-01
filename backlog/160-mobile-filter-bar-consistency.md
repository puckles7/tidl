# Mobile filter bars: consistent collapsed card and top alignment

On mobile the filter bars are inconsistent between tabs:

1. **Collapsed bar size.** Calendar and Focus render each section as a compact
   `.fs-panel` card with the shared `.fs-collapse-btn` header. Notes and
   Contacts render their single "Note collections" / "Contact collections"
   sidebar (`.notes-sidebar`) with the same collapse button, but the desktop
   card padding (1.125rem) remains, making the collapsed bar visibly taller
   and its header text lower than the other tabs.
2. **Top spacing.** Notes, Contacts, and Done start too far down the page
   compared with Calendar and Focus. Done and Contacts were missing the
   negative margin that compensates for the container's mobile-bar top
   padding; Notes/Contacts also kept their desktop 32px side padding because
   their base rules are declared after the shared mobile override. Focus was
   also slightly lower than Calendar because its mobile view padding was 1rem
   vs Calendar's 0.5rem.

## Context / Background

- Calendar/Focus panels are `CollapsiblePanel` → `.fs-panel` +
  `.fs-collapse-btn` + `.fs-content`.
- Notes/Contacts use the same button/content classes but an outer
  `.notes-sidebar` card with desktop padding; they are not `.fs-panel` cards.
- Mobile view offsets live in the `@media (max-width: 767px)` block in
  `assets/style.css`: Calendar/Focus/Notes get `margin-top: -3.85rem`, while
  Contacts/Done do not. Source order lets the later `.notes-view` /
  `.contacts-view` base padding rules override the mobile padding block.

## Functional Requirements

- The collapsed Notes/Contacts filter bar must use the same compact card
  styling and height as Calendar/Focus.
- All five tabs must start their filter bar at the same vertical offset in
  mobile mode, including Focus's 8px offset from Calendar.
- Opening a Notes/Contacts panel must keep its content inset so it doesn't
  touch the card border.

## Resolution

- Notes and Contacts sidebars now carry the shared `fs-panel` class, reusing
  the same mobile card shell as Calendar/Focus. A mobile `.notes-sidebar`
  override removes the desktop padding and matches the 0.5rem radius, while
  `.notes-sidebar .fs-content` restores the open content's inset (Notes' inner
  `.fs-section` is stripped to zero padding by the existing desktop rule).
- A late mobile block (after all base view rules, so it wins the cascade)
  gives Focus, Notes, Contacts, Done (and the Tasks/Tickler views) the same
  `margin-top: -3.85rem` compensation and `0.5rem` top / `0.75rem` side
  padding. Calendar already had the same top offset and padding.
- `.done-view` gained the same base page padding as Focus/Notes/Contacts so
  its desktop alignment matches too.

Verified in headless Chromium at 400px and 767px: all five tabs now place the
first filter bar at the same y, and the collapsed bar is 34px tall everywhere.
Opening Notes/Contacts keeps the content inset. `cargo test --workspace`
passes.
