# Contacts — drag-to-reorder, shared with Notes' drag system

Contacts currently has no manual drag-and-drop reordering — cards are auto-placed on create (shortest-column masonry) and self-correct their own row-span, but unlike Notes there's no way to pick up a card and drop it somewhere else. This was a deliberate scope cut in #79 (the spec never asked for it, and porting Notes' drag system looked like more risk than the CRUD tab needed). Follow-up user feedback: since #79 already generalized the grid's *placement* math (`crate::grid`) between Notes and Contacts, the *interaction* layer (pointer pickup/move/drop, live displacement preview) should be generalized the same way instead of Contacts simply doing without.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — this item builds directly on its shared `crate::grid` module and the Contacts tab it introduced.

## Context / Background

Notes' drag system (`crates/tidl-web/src/components/notes/mod.rs`) is currently the only implementation, and it is *not* factored for reuse — it's ~400 lines of state and event-handler logic private to `NotesView`, built around two zones (pinned/non-pinned) sharing one `drag` signal so a note picked up in one zone can be blocked from dropping in the other. Contacts has only one flat zone, so its version of this would actually be *simpler* than Notes', but the goal here is a genuinely shared abstraction, not a second copy.

Investigation done in the conversation that raised this item (session ended before any code was written) sketched the following design — captured here so a future session doesn't have to re-derive it:

**New component**: `crate::components::draggable_grid` (or similar name), holding:
- `DragItemState` — a generalized version of today's `NoteDragState` (`components/notes/mod.rs`), with `pinned: bool` replaced by `zone: usize` (Contacts always uses zone `0`; Notes uses `0`/`1` for pinned/non-pinned). Same fields otherwise: `item_id`, `origin_x/y`, `grab_dx/dy`, `drag_px`, `no_drop`, `last_cell`, `moving_span`, `occupied` (frozen at pickup), `geometry` (`grid::GridGeometry`), `cols`, `col_offsets`.
- A `<DraggableGrid>` Leptos component wrapping one zone's rendering + pointer handling, taking as props:
  - `items: Vec<GridItem>` (id/row/col/span) and a `render_item: Callback<i64, AnyView>` for content (caller looks up its own full model by id)
  - `zone_id: usize`, `card_width: f64`, `reserved_col0_floor: i32`
  - `other_zone_refs: Vec<(usize, NodeRef<Div>)>` — for cross-zone "no-drop" checking; **empty for Contacts** (single zone ⇒ the check trivially never blocks), and `[(1, other_grid_ref)]`/`[(0, pinned_row_ref)]` for Notes' two zones. This is what makes Notes' 2-zone blocking rule and Contacts' no-blocking-at-all fall out of the *same* code path instead of an `if` for Notes-only behavior.
  - The four drag signals (`drag`, `drag_offset`, `drag_size`, `drag_overlay`) passed in from the parent rather than owned internally — Notes needs ONE set shared across its two `<DraggableGrid>` instances (so the zone being hovered *into* can see the in-progress drag that started in the *other* zone); Contacts' `ContactsView` would create one set and pass it to its single instance.
  - `on_move: Callback<Vec<GridPosition>>` (persist — `move_notes`/`move_contacts`) and `on_open: Callback<i64>` (plain click, not a drag — opens the editor).
  - An optional leading extra child slot for Notes' pinned-zone-only "+ Take a note…" add-tile (Contacts passes none).
- `NotesView` gets refactored to instantiate two `<DraggableGrid>`s (replacing its current inline pinned/non-pinned blocks) — this should be closely reviewed against the *current*, working, tested behavior line-by-line, since the two zones' logic (esp. the cross-zone no-drop rect check, and the drop-shadow positioning via `drag_overlay`) is exactly what's being generalized and is the part most likely to regress subtly.
- `ContactsView`/`ContactCard` get wired to a single `<DraggableGrid>` instance instead of the current plain `.map().collect_view()` render with no drag support; `api/contacts.rs::move_contacts` already exists (added in #79 for the height-self-correction pass) and needs no changes.

## Functional Requirements

- Contacts cards can be picked up and dropped anywhere in the grid, same interaction as Notes (pointer down + move ≥5px triggers drag; card follows the pointer via a `position: fixed` overlay; dropping on an occupied cell pushes the note(s)/contact(s) below it down the same column; empty space elsewhere in the grid is a valid drop target).
- Notes' existing behavior (both zones, cross-zone no-drop, add-tile reservation, deep-link highlight, height self-correction) must be unchanged after the refactor — this is a refactor-for-reuse of already-shipped, tested functionality, not a rewrite.
- `api/notes.rs`'s and `api/contacts.rs`'s server-side placement/persistence (`move_notes`/`move_contacts`, `apply_positions`, `auto_layout_notes`/`auto_layout_contacts`) are unaffected — this item is client-side interaction only, building on top of the already-shared `crate::grid` placement math from #79.

## Open Questions

- Whether `<DraggableGrid>` should own the height-self-correction `Effect` internally too (it's currently duplicated per-view — `NotesView` and `ContactsView` each have their own near-identical measure-and-correct pass reading `.note-card`/`.contact-card` via a hardcoded CSS selector) — folding it in would need a generic `data-*-id` attribute convention and a caller-supplied CSS class selector, but is a reasonable next step once the drag layer itself is shared.
- Leptos component prop constraints (`Callback`/`AnyView` bounds, `Children`/slot ergonomics for the add-tile) weren't verified against the actual Leptos 0.8 API in this session — worth a quick spike before committing to the exact prop shapes sketched above.

## Resolution

Built close to the sketched design, with one structural change: not a `#[component]` (a Leptos component can only occupy one place in its caller's DOM tree, but a zone's trigger icon and its drop-shadow/positioning live in different places within the card/grid markup) — instead a plain `DragController` struct (four `Copy` `RwSignal`s) with methods, plus two free functions (`card_style`, `drop_shadow_view`), all in a new `crates/tidl-web/src/components/drag_grid.rs`:

- `DragController::pick_up`/`pointer_move`/`pointer_up`/`cancel` — direct generalizations of `NotesView`'s old inline closures, with `NoteDragState::pinned: bool` replaced by `DragItemState::zone: usize` throughout. `pointer_up` returns a `DragOutcome` enum (`Click`/`Moved`/`Cancelled`) rather than mutating anything itself — applying the optimistic local update and calling `move_notes`/`move_contacts` stays caller-specific, since "which local signal to write" and "which editor to open" differ per view.
- `pointer_move`'s no-drop check takes `zones: &[(usize, NodeRef<Div>)]` (every zone sharing the controller, including the dragged item's own) — Notes passes both its zones so each can block drops into the other; Contacts passes just its own single zone, which the check skips automatically (`zid != ds.zone`), so "no cross-zone blocking" falls out of the shared code with zero Contacts-specific branching.
- `card_style`/`drop_shadow_view` are plain functions (not components) returning `impl Fn() -> ...`, embedded directly as `view!` children/props — same reasoning as the pre-existing `event_child` pattern in `week_grid.rs` (a closure this shaped trips the `view!` macro parser if written inline).
- `NotesView` was refactored in place to call into `DragController` — all four drag signals (`drag`/`drag_offset`/`drag_size`/`drag_overlay`) collapsed into one `DragController` shared by both zone blocks. `NoteCard`'s own `card_style` closure was replaced with a call to the shared `card_style` fn; its 4 separate drag-signal props collapsed to one `controller: DragController` prop.
- `ContactsView`/`ContactCard` gained pointer handlers on `.contacts-main`, a `DragController`, and real pick-up/drop-shadow wiring — `api/contacts.rs::move_contacts` (already present from #79, previously only called by the height self-correction pass) is now also called from actual drag-drop, unchanged. `ContactCard`'s `on_click` prop was removed entirely — opening the editor now happens via `DragOutcome::Click` from the shared pointer-up handler, matching how Notes has always resolved "was that a click or a drag".
- CSS: `.note-card-drop-shadow` → `.grid-drop-shadow` (shared), `.card-tile.note-card--dragging` → `.card-tile.card-tile--dragging` (shared), `.notes-main--dragging` extended to also match `.contacts-main--dragging`. `.notes-main--no-drop` stayed Notes-only (Contacts has nothing to block against).
- Verified end-to-end in a real, throwaway-isolated browser session: scripted pointer drags (mousedown → move past the 5px threshold → move to target → mouseup) for both Contacts and Notes, confirming the dragged card's grid position actually changed, and that the new position survived a full page reload (i.e. really persisted server-side via `move_contacts`/`move_notes`), for both features.
- **Found and filed, but explicitly did not fix, a pre-existing bug**: `move_notes`/`move_contacts` can occasionally 500 with `database is locked` under near-simultaneous writes (the drop's own persist racing the height self-correction pass's persist). Confirmed by stashing this item's entire diff and reproducing the identical failure against the pre-#82 committed code — so it predates this item and isn't a regression from it. Filed as [83](83-drag-move-notes-database-locked.md) rather than expanding this item's scope.
