# Auto-layout should restore manual placement when toggled back off (Contacts & Notes)

Clicking the "⊞ Auto-layout" toggle immediately persists a repacked layout to the database. Clicking it again to turn it off only stops further repacking on resize — it doesn't undo the repack, because nothing ever captured what the layout looked like beforehand. The expectation (manual placement → auto-layout on → auto-layout off → back to the original manual placement) was never actually built; it just happened to look that way because no one had tested the round trip.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md)
- [37 - Notes tab — collections, ad-hoc notes, to-do/tickler coupling](37-notes-tab.md)
- [82 - Contacts — drag-to-reorder, shared with Notes' drag system](82-contacts-drag-reorder.md)
- [93 - Lock the Auto-layout toggle while searching](93-lock-autolayout-toggle-while-searching.md)

## Context / Background

- `ContactsView`/`NotesView` (`crates/tidl-web/src/components/contacts/mod.rs`, `crates/tidl-web/src/components/notes/mod.rs`) both wire "Auto-layout" as a session-local toggle (introduced in `d08ac97`): turning it on calls `auto_layout_contacts`/`auto_layout_notes`, a `#[server]` fn that immediately persists the new (alphabetical, for Contacts; insertion-order, for Notes) positions to the database via `apply_positions`.
- Turning the toggle off only stops the effect that re-repacks on window resize. It never restores anything, because no prior state was ever captured — the toggle was designed as a one-way persisted action with a resize-following twist, not a reversible preview.
- Reported as a suspected regression, but investigation (git history, backlog #84/#93, the original `d08ac97` commit) found no prior spec or working version where toggling off restored the previous manual layout — this is a gap in the original design, not a regression.

## Functional Requirements

- Clicking "Auto-layout" on captures a snapshot of every visible card's current `(grid_row, grid_col, grid_row_span)` before the repack runs.
- Clicking "Auto-layout" off restores that snapshot (optimistically client-side, then persisted via `move_contacts`/`move_notes`) and clears it.
- A manual drag while auto-layout is on (which already turns the toggle off, per existing behavior) discards the pending snapshot instead of restoring over it — the drag itself is the layout to keep.
- The forced-on/off toggling driven by the search box (backlog #93) is unaffected: it doesn't go through this snapshot/restore path, since it never persists anything server-side to begin with.
- Applies to both the Contacts tab and the Notes tab (Notes' snapshot spans both the pinned and non-pinned zones).

## Open Questions

- None — scope was confirmed with the user (cover both Contacts and Notes) before implementation.

## Resolution

Added a `pre_auto_layout: RwSignal<Option<Vec<GridPosition>>>` in both `ContactsView` and `NotesView`. `on_auto_layout` now branches on the current toggle state instead of blindly flipping it: turning on snapshots every currently-displayed card/note's grid position into `pre_auto_layout` before enabling; turning off restores from that snapshot (optimistic local update, then `move_contacts`/`move_notes` to persist, resyncing via `refresh` on failure) and clears it. The `DragOutcome::Moved` branch in both views' pointer-up handlers now also clears `pre_auto_layout` alongside its existing `auto_layout_enabled.set(false)`, so a manual drag doesn't leave a stale snapshot that a later toggle-off could restore over the drag. Notes' snapshot/restore spans both `pinned_display` and `other_display` since a note's grid position is independent of its pinned zone. The toolbar button's tooltip was updated to describe the round trip ("click again to restore your manual layout, or drag a card to override it directly").
