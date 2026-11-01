# Lock the Auto-layout toggle while searching (Contacts & Notes)

Searching already forces Auto-layout on (Contacts and Notes both do this
now). Manually turning Auto-layout back off *while still searching*
produces a broken state: dragging a card stops working properly, because
other cards that aren't currently visible (filtered out by the search) are
still occupying real grid positions and the drag/placement logic collides
with them.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md)
- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md)

## Context / Background

`ContactsView` and `NotesView`
(`crates/tidl-web/src/components/contacts/mod.rs`,
`crates/tidl-web/src/components/notes/mod.rs`) both force `auto_layout_enabled`
on when the search box goes from empty to non-empty, and back off when it's
cleared, so the visible (filtered) cards get a tight, gap-free client-only
repack instead of showing wherever they happened to land in the full,
unfiltered layout.

The "⊞ Auto-layout" toolbar button is still clickable during a search,
though, and toggling it off mid-search leaves the view in a state where
manual dragging breaks: the cards you can see got their tight, ephemeral
positions from the search-time repack, but the drag/placement math still
has to account for every other contact/note's *real* (stored, scattered)
grid position — which, while filtered out of view, still occupy real cells
the drag logic treats as taken, blocking the drop the user is trying to
make.

## Functional Requirements

- While the search box has any text in it, the Auto-layout toggle button
  is disabled (visibly, e.g. greyed out / not clickable) and stays on —
  matching the existing forced-on behavior, just also preventing the user
  from fighting it.
- Once the search box is cleared, the toggle becomes clickable again (and
  auto-layout itself turns back off automatically, per existing behavior).
- Applies to both the Contacts tab and the Notes tab.

## Open Questions

- Should hovering/focusing the disabled button show a tooltip explaining
  why (e.g. "Auto-layout stays on while searching")? Not required, but
  worth considering during implementation for discoverability.

## Resolution

Added an `is_searching` derived signal in both `ContactsView` and `NotesView`, and wired it to the "Auto-layout" button's `disabled` attribute plus its `title`, which now swaps to "Auto-layout stays on while searching" while searching (answering the open question — yes, added the tooltip, since it was cheap and the mechanism was already there for the plain-English case). The `on_auto_layout` click handler also early-returns while searching as defense in depth in case `disabled` doesn't block a stray event. Added a `.notes-pin-filter:disabled` CSS rule (`assets/style.css`) so the locked state reads as visibly non-interactive (`opacity: 0.6`, `cursor: not-allowed`) rather than looking identical to the enabled toggle.
