# "Show in collection" button on the note-goto modal

When a note-origin task/await item is opened via the "go to source" arrow (backlog #62), the resulting `NoteEditorModal` has no way to jump to where that note actually lives in the Notes tab. Add a button that navigates there, ensures the note's collection is visible, scrolls to it, and highlights it briefly so it's easy to spot.

## Dependencies

- [#62 — Note items need a "go to source" arrow in Focus, Tasks, and Await lists](62-note-goto-source-arrow.md) — this button lives in the modal that item introduced

## Design

Brainstormed 2026-07-23. Two decisions confirmed with the user:
- If the note's collection is currently hidden (via the Notes sidebar's show/hide toggle), auto-unhide it so the note is actually visible after navigating — not left silently invisible.
- The pulsating highlight uses the note's own collection color (same color already used for its stripe/pill elsewhere), not a fixed generic accent color.

**Navigation + targeting:** Full-page navigation via `window.location().set_href("/notes?highlight=<note_id>")` — matches the existing precedent in `todo_list.rs` (`ItemKind::Task if source == "caldav"` jumps to `/tasks` the same way) rather than introducing SPA client-side routing for this one case. `NotesView` reads the `highlight` query param on mount (via `leptos_router`'s query-map hook) into a `RwSignal<Option<i64>>`.

**Auto-unhide + scroll + highlight:** Once notes have loaded and `highlight` is set: if that note's `collection_id` is in `HiddenNoteCollectionsCtx`, remove it from the hidden set. Scroll the matching `.note-card` into view (`scroll_into_view`, hydrate-only). Apply a `note-card--highlight` class to that card.

**The pulse:** a CSS `@keyframes` animation on `.note-card--highlight` pulsing an outline/box-shadow in the note's own collection color, `animation-iteration-count: 3` (~2.4s total, no JS timer needed — it just stops and reverts once the iterations are exhausted).

**The button:** an optional `on_show_in_collection: Option<Callback<()>>` prop on `NoteEditorModal` (default `None`, so its existing in-Notes-tab usage is unaffected). `NoteGotoModal` is the only caller passing `Some(...)`, building the URL from the note's `id`.

## Functional Requirements

- A "Show in collection" (or similar label) button appears in the note-goto modal only (not when editing a note from within the Notes tab itself).
- Clicking it navigates to `/notes`, unhides the note's collection if hidden, scrolls to the note, and pulses its outline in the collection's color a few times before settling back to normal.
- No regression to the existing `NoteEditorModal` usage inside the Notes tab.

## Resolution

Implemented as designed, with one simplification: **the explicit "auto-unhide" step turned out to be unnecessary and was dropped.** `NotesView`'s `hidden_collections` is session-only state (a plain `RwSignal`, never persisted to storage) that always starts empty on a fresh mount — and this feature's navigation is always a fresh full-page load. So by the time `?highlight=` is read, the collection can never actually be hidden; adding "unhide" logic would have been dead code for a state that can't occur. If `hidden_collections` ever gains persistence in the future, this would need revisiting.

- `note_editor_modal.rs`: added `#[prop(optional)] on_show_in_collection: Option<Callback<()>>` to `NoteEditorModal`; renders a "Show in collection" button in the dialog footer only when `is_edit` and the prop is `Some`.
- `focus.rs`: `NoteGotoModal` builds the callback — `window.location().set_href("/notes?highlight={note_id}")` — and passes it through.
- `notes/mod.rs`: reads `?highlight=` via `leptos_router::hooks::use_query_map()` into a one-shot `RwSignal<Option<i64>>`; a new `Effect` (alongside the existing height-self-correction one, following its `document.query_selector`/`data-note-id` pattern) waits for the note to actually appear in `pinned_display`/`other_display`, then scrolls its card into view, sets a `--note-highlight-color` inline style from the note's collection color, and adds `.note-card--highlight` — then clears the signal so a later unrelated re-render can't re-trigger it.
- `style.css`: `.note-card--highlight` / `@keyframes note-highlight-pulse` — a `box-shadow` ring pulsing via `color-mix`, `animation-iteration-count: 3`, so it stops and reverts on its own with no JS cleanup needed.

Verified with a real headless-browser run: the button is absent when editing a note directly from the Notes tab, present in the note-goto modal, and clicking it lands on `/notes?highlight=<id>` with exactly the right card carrying the highlight class and a populated color variable.
