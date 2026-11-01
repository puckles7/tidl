# Note card — delete button without opening the note

Add a delete action directly on the note card in the Notes grid, so a note can be removed without first opening the editor modal.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — note cards and grid this button lives on

## Context / Background

`delete_note` already exists as a server fn (`crates/tidl-web/src/api/notes.rs`) and is wired up inside `note_editor_modal.rs`. The card itself (`crates/tidl-web/src/components/notes/note_card.rs`) already has an on-card action button for pinning (`on_pin_click` / `toggle_note_pin`, note_card.rs:120) — the same pattern (icon button in the card chrome, stopping event propagation so it doesn't open the note) can be followed for delete.

## Functional Requirements

- Add a delete icon/button to `NoteCard`, positioned alongside the existing pin button.
- Clicking it must not open the note (stop propagation, same as the pin button).
- Require a confirmation step before deleting (destructive, irreversible) — e.g. a second click/hold or a lightweight inline confirm, not a native `confirm()` dialog per project conventions.
- On confirm, call `delete_note` and remove the card from the grid/refresh the notes resource.

## Open Questions

- Should this respect the same sharing-mismatch / write-permission checks the editor modal's delete path already applies (see #39)?

## Resolution

Landed as a byproduct of implementing #79 (Contacts), which needed the identical pattern for its own cards and — per #79's own spec text ("same pattern as #78, so both land consistently") — built it as a shared component instead of two separate implementations.

Went through three design iterations, driven by real usage feedback rather than picked up front:

1. **Icon-toggle (🗑 → red ✓), two-click, auto-disarm after 3s.** Rejected: a checkmark reads as "already done", not "click to confirm" — a real user clicked once, saw the checkmark, and assumed the delete had already happened. Compounded by a CSS bug (`.card-delete-btn:hover`'s color rule outranked the armed state's own text color by specificity, so the label was invisible while the cursor was still over the button from the first click).
2. **Inline confirm bar at the bottom of the card** ("Delete this item? [Delete] [Cancel]", no timeout, explicit cancel). Rejected: canceling sometimes opened the item's editor instead. Root cause — the Cancel button's own click handler removed the bar's DOM node (via the `armed` signal) *before* the bar's own `stop_propagation` wrapper got to run in the bubble phase, so the click occasionally continued bubbling to the card's "open editor" handler.
3. **A real modal** (final): trash icon opens a `.dialog-backdrop`/`.dialog` popup — same chrome as every other dialog in the app (note/contact editors, admin panels), narrower (`.dialog-small`, new 320px-capped modifier), with "Delete this note?" and standard `.btn-danger`/`.btn-secondary` Delete/Cancel buttons. Every click target inside it (backdrop, dialog box, both buttons) stops propagation itself, so there's no path back to the card's own click handling regardless of DOM-mutation timing — sidesteps iteration 2's bug structurally rather than patching around it.

- `components/delete_confirm_button.rs` now exports two components sharing one `armed: RwSignal<bool>`: `DeleteTriggerButton` (the header-row icon) and `DeleteConfirmModal` (the popup) — separate because the icon and the modal render in different places in each card's layout, not adjacent siblings.
- Wired into `NoteCard` (`components/notes/note_card.rs`) alongside the pin button, both inside a shared `.note-card-actions` wrapper. Confirming calls the existing `delete_note` server fn and bumps the notes-view refresh signal.
- No sharing-mismatch permission dialog — `delete_note`'s existing collection-access check is enforced server-side as it already was via the editor modal's delete path, so an unauthorized delete simply fails silently client-side (matching the editor modal's own behavior here, not a new gap).
- Verified end-to-end in a real browser at each iteration (including the two rejected ones, which is how their bugs were actually caught).
