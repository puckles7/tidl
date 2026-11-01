# "Clear all" buttons for Contacts and Notes

Add a way to bulk-delete every item in the Contacts and Notes tabs, rather than deleting cards one at a time.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — the flat contact list this clears
- [37 - Notes tab](37-notes-tab.md) — the collections this clears notes from
- [78 - Note card delete button](78-note-card-delete-button.md) — `DeleteConfirmModal`, reused (and extended) for the Contacts side

## Context / Background

Contacts (#79) is a single flat per-user list, so "clear all" there is an unambiguous bulk delete. Notes (#37) are organised into collections, some shared with other users via `note_collection_members` — a member can already delete individual notes in a shared collection they don't own (`has_collection_access` in `api/notes.rs`), so a bulk "clear all notes I can see" isn't a new permission, but it does need an explicit scope choice rather than a single accidental click.

Separately, the Notes pinned zone reserved a fixed block of grid space (`NOTES_ADD_TILE_ROW_SPAN`, 11 row-units) in column 0 for an in-grid "+ Take a note…" tile — Contacts has no such reservation (single flat zone starting at row 0). Moving note creation to a toolbar button (matching Contacts' "+ New contact") made that reservation dead weight, so it was removed along with the tile.

## Functional Requirements

- Contacts toolbar gets a "🗑 Clear all" button (shown only when there's at least one contact), opening the existing `DeleteConfirmModal` (`components/delete_confirm_button.rs`) with its title pluralized via a new `count` prop ("Delete all N contacts?").
- New `clear_contacts()` server fn (`api/contacts.rs`): deletes every contact owned by the current user in one query, then bumps the CardDAV sync log per deleted uid (same per-uid bump `delete_contact` does) so external CardDAV clients still see each deletion individually on their next sync.
- Notes toolbar gets a "🗑 Clear all" button (shown only when there's at least one note visible), opening a new `ClearNotesModal` (`components/notes/clear_notes_modal.rs`): a `<select>` defaulting to "All collections" plus every collection the viewer owns or is a member of, then "Clear"/"Cancel".
- New `clear_notes(collection_id: Option<i64>)` server fn (`api/notes.rs`): `Some(id)` clears just that collection (after the usual owner-or-member access check); `None` clears every note visible to the viewer (own + shared collections), matching `list_notes`'s own scope. Loops a shared `delete_note_inner` helper (factored out of `delete_note`) per note so each note's mirrored VTODO and its calendar's sync token get cleaned up exactly as an individual delete would.
- Notes toolbar reorganised: "+ New note" (opens `NoteEditorModal` in create mode) replaces the in-grid "+ Take a note…" tile, sitting alongside "⊞ Auto-layout" and "🗑 Clear all"; "★ Starred" and search stay at the end.
- The pinned zone's `NOTES_ADD_TILE_ROW_SPAN` reservation is removed entirely (client `zone_floor`/`reserved_floor_for` and server `place_in_first_gap`/`auto_layout_notes` all floor at 0 now, matching Contacts) — pinned notes can occupy row 1 onward. No DB migration: existing pinned notes keep their current stored position until "Auto-layout" repacks them tightly.

## Open Questions

- None outstanding — scope questions (Notes' shared-collection handling, the toolbar reorg's grid-floor removal) were resolved during design with the user before implementation.

## Resolution

Implemented as designed. `DeleteConfirmModal` gained an `#[prop(default = 1)] count: usize` that switches its title between "Delete this {noun}?" and "Delete all {count} {noun}s?", reused as-is by Contacts' toolbar button. Notes' `ClearNotesModal` is a new sibling component using the same dialog chrome (`.dialog-backdrop`/`.dialog-small`) with a `<select class="form-input">` in place of the pluralized title.

`delete_note`'s body was extracted into `delete_note_inner(pool, username, id)` so `clear_notes` could loop it per note rather than duplicating the mirrored-VTODO/calendar-sync-token cleanup logic.

Removing the pinned-zone floor touched more call sites than the button move itself: `zone_floor` (component) and the `NOTES_ADD_TILE_ROW_SPAN` constant (`types.rs`) were deleted outright; `place_in_first_gap`, `auto_layout_notes`'s `col_floors`, `correct_spans`'s floor arg, `column_offsets`'s floor arg, and `DragController::pick_up`'s floor arg all simplified to a flat `0`, mirroring Contacts' zone exactly. The now-unused `.notes-add-tile` CSS rule was deleted along with a stale doc-comment cross-reference in `.grid-drop-shadow`'s comment and in `grid::reserved_floor`'s doc comment.

Verified: `cargo test --workspace` (383 passed, 0 failed), `cargo clippy --workspace -- -D warnings` (clean), `cargo build --target wasm32-unknown-unknown -p tidl-web` (clean). Manual browser verification was attempted but unavailable in this sandboxed session (no Chrome extension connected; Playwright's chromium fails to launch here on a pre-existing `glibc`/`alsa-lib` ABI mismatch, unrelated to this change) — instead, a scratch `tidl-server` instance was built and booted in an isolated worktree/data-dir, and `strings`/`grep` against the compiled `tidl.wasm` confirmed the new UI text ("Clear all", "New note", "Clear notes", "All collections", the pluralized "Delete all …" title) actually made it into the shipped bundle. Real browser click-through is left for manual verification outside this sandbox.
