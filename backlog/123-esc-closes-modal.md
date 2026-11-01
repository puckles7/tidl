# ESC closes an open modal

Most modals can only be dismissed by clicking the ✕, a Cancel button, or the
backdrop. Escape is handled in only a couple of places (the note editor's
mention popup, the markdown file finder), so pressing ESC on any other dialog
does nothing. ESC should close the topmost open modal, consistently.

**Status:** done (2026-10-07).

## Background

Modals share the `.dialog-backdrop` / `.dialog` chrome and are rendered from
many components (`settings`, `focus`, `cal_edit_modal`, note/contact editors,
previews, finder, `profile_modal`, …). There is no shared modal base component,
so each surface currently wires its own close callback. The note editor's
textarea `on:keydown` and `markdown_file_finder.rs` are the only Escape
handlers found.

## Functional Requirements

- Escape closes the topmost open modal, for every `.dialog-backdrop` surface.
- Nested/stacked modals: Escape closes only the top one (e.g. the task chooser
  inside the note editor, the scope dialog inside the event form), leaving the
  parent open.
- A nested popup/chooser that already consumes Escape (mention popup, file
  finder's own handler) keeps priority — the first Escape dismisses it, a
  second closes the modal.
- ESC in a text input should still close the modal (matching Cancel), unless a
  chooser is open.
- Behavior stays consistent with the backdrop click (same close path).

## Open Questions

- One global handler keyed off the top `.dialog-backdrop`, or a small shared
  modal wrapper every surface adopts?
- Should ESC with unsaved edits close silently (like Cancel today) or prompt?
- Interaction order with the app-level Ctrl/Cmd-K handler in `app.rs`.

## Resolution

Built the "one global handler" option: a new `crates/tidl-web/src/modal_esc.rs`
provides a `ModalEsc` registry via context (`provide_modal_esc` in `app.rs`) and
a `use_escape_close(on_close) -> id` helper that each modal calls at setup. The
helper returns an id placed on the modal's backdrop as `data-esc-id`. A single
window-level `keydown` listener in `app.rs` (merged with the existing Ctrl/Cmd-K
handler) sees `Escape`, skips it when `default_prevented()` (an inner chooser
consumed it), otherwise finds the **last visible `.dialog-backdrop`** in the DOM
with a `data-esc-id` and runs that component's close callback — the same path as
its ✕ / Cancel / backdrop click. Because a closed modal's backdrop isn't
rendered, "last visible in DOM order" is exactly the topmost open modal.

- Wired every `.dialog-backdrop` surface (settings, profile, note/contact
  editors, collection/label editors, cal/markdown-source editors, share
  dialogs, clear/delete confirms, dir picker, file finder, Focus ad-hoc /
  suppress / preview / note-goto dialogs, event form).
- Nested layers are handled by the enclosing component's callback: the event
  form closes its recurrence scope dialog first, the note editor closes its
  `@mention` / `#` choosers and inline task modal first, calendar/collection
  editors close their clear/delete confirm first. The `@mention` textarea and
  the file finder keep their own `prevent_default()` Escape handling, so the
  first Escape dismisses them and the second closes the modal.
- Chose "close silently, like Cancel" for unsaved edits (matches existing
  behavior); a prompt wasn't added.

Verified with `cargo test --workspace`, `cargo clippy` (clean), the WASM
hydrate build, the full Python suite (`pytest tests/`, 122 passed) plus two new
Playwright cases in `tests/test_hydration.py`: Escape closes Settings, and a
nested Escape dismisses the note editor's inline task modal before the editor
itself.

