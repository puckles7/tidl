# Note collection picker — create-new + remembered default

When a user has no note collections yet (there is no server-created default,
unlike calendars which always get a "personal" calendar), the note editor's
collection chip row falls back to a fake `collection_id: 0`. Saving then
fails with a confusing `Not authorized for this collection` error instead of
guiding the user to create a collection. Separately, the collection picker
always defaults to the alphabetically-first collection rather than
remembering what the user picked last, unlike the calendar picker
(`LastUsedCalendarCtx`).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — collections and `NoteEditorModal` this builds on

## Context / Background

Calendars sidestep this entirely: `ensure_personal_calendar` guarantees every
user has at least a "personal" calendar server-side, so `EventFormDialog`'s
`.unwrap_or_else(|| "personal")` fallback always resolves to something real.
Note collections have no equivalent auto-vivification, so
`NoteEditorModal`'s `collection_val` (`crates/tidl-web/src/components/notes/note_editor_modal.rs`)
can end up as the sentinel `0` — a value that never corresponds to a real
row — whenever `collections` is empty and no `default_collection_id` was
supplied.

`EventFormDialog` also remembers the last calendar an event was saved to via
`LastUsedCalendarCtx` (`RwSignal<String>` newtype, provided in
`CalendarView`, updated only on successful *creation*, read as the initial
selection on the next "New Event"). Note collections have no equivalent.

## Functional Requirements

1. **No more fake `0` collection.** `NoteEditorModal`'s `collection_val`
   becomes `RwSignal<Option<i64>>`. `save` validates it before calling the
   server: if `None`, show the inline error "Please choose or create a
   collection." and return without hitting `create_note`/`update_note`.
2. **Inline "+ New" chip.** The collection chip row always renders a
   trailing "+ New" chip (present whether the row is empty or full — mirrors
   the calendar import panel's ever-present "New calendar…" option).
   Clicking it opens the existing `CollectionEditorModal`
   (`crates/tidl-web/src/components/notes/collection_editor_modal.rs`)
   stacked on top of the note editor dialog (both use `.dialog-backdrop`;
   this layers the same way the recurrence scope dialog stacks inside
   `EventFormDialog`).
   - `NoteEditorModal` holds its own `RwSignal<Vec<NoteCollectionInfo>>`
     seeded from the `collections` prop (instead of treating it as a static
     snapshot), so the chip row can grow after an inline create.
   - `CollectionEditorModal` gains one new optional prop,
     `on_created: Option<Callback<i64>>`, fired only on the successful
     *create* path (not edit) with the new row's id. The existing sidebar
     call site (`NotesView`) doesn't pass it and is unaffected.
   - The note editor's nested usage passes a callback that: refetches
     `list_note_collections()`, replaces its local signal, selects the new
     collection, and closes the nested modal — the outer note editor stays
     open with the new collection already picked.
3. **Remember last-used collection.** New context
   `LastUsedCollectionCtx(pub RwSignal<Option<i64>>)` (newtype per the
   project's context-shadowing rule — see CLAUDE.md §15), provided in
   `NotesView` the same way `LastUsedCalendarCtx` is provided in
   `CalendarView`. Set only on successful note *creation* (not edit) to the
   collection that was actually used.
4. **Default-selection priority** when opening "New note", in order:
   `default_collection_id` prop → last-used ctx (only if that id is still
   present in the fetched collections) → first collection alphabetically →
   `None` if the list is empty.

No server or schema changes — this is entirely `tidl-web` component work.

## Open Questions

- None — resolved during brainstorming. Full `CollectionEditorModal` reuse
  (not a name-only quick-add) was chosen for the inline "+ New" flow so
  color/sharing can be set immediately.

## Resolution

Implemented as specified, entirely in `crates/tidl-web/src/components/notes/`:

- `NoteEditorModal`'s `collection_val` is now `RwSignal<Option<i64>>`; `save`
  validates it before calling the server ("Please choose or create a
  collection." inline error, no request sent) instead of ever falling back
  to the old fake `collection_id: 0`.
- The collection chip row always ends with a "+ New" chip. It opens the
  existing `CollectionEditorModal` stacked on top (both use
  `.dialog-backdrop`, layering the same way the recurrence scope dialog
  stacks inside `EventFormDialog`). `CollectionEditorModal` gained one new
  `#[prop(optional)] on_created: Option<Callback<i64>>`, fired only on a
  successful *create*; the note editor's nested usage refetches
  `list_note_collections()`, updates its own `RwSignal<Vec<NoteCollectionInfo>>`
  (`collections_state`, replacing the old static `StoredValue` snapshot),
  and auto-selects the new collection. The pre-existing sidebar call site
  is unaffected (prop defaults to `None`).
- New `LastUsedCollectionCtx(pub RwSignal<Option<i64>>)` newtype, provided
  in `NotesView` exactly like `LastUsedCalendarCtx` is provided in
  `CalendarView`. Set on successful note *creation* only. Default-selection
  order on open: `default_collection_id` → last-used (if still present in
  the fetched collections) → first collection alphabetically → `None`.

No deviations from the design. `cargo test --workspace`, `cargo clippy
--workspace -- -D warnings`, and `cargo build --target
wasm32-unknown-unknown -p tidl-web` all pass.
