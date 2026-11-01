# Note editor: grow the content box, compress the fields below, hide hints behind info icons

The note editing modal gives the note textarea only a tiny `rows=3` box while
the fields beneath it (collection, tasks & awaits, calendar, sharing) plus
several long `dialog-hint` paragraphs take most of the vertical space. Writing a
note happens in the smallest area of the modal. The textarea should get the room
instead: compress the lower fields and move the long explanatory hints behind
info icons.

**Status:** done. Design: [spec](../docs/superpowers/specs/2026-10-08-note-editor-content-box-design.md) · [plan](../docs/superpowers/plans/2026-10-08-note-editor-content-box.md).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — the note editor this restyles.
- [66 - Note titles](66-note-titles.md) — the title field in the same form.
- [113 - Inline tasks and awaits in notes](113-note-inline-tasks.md) — the tasks & awaits chip section.
- [80 - @mention contact linking](80-note-contact-mention.md) / [91 - markdown editor contact mention](91-markdown-editor-contact-mention.md) — the hints that reference `@`/`#` syntax.

## Functional Requirements

- Give the note textarea the available height (grow to fill the modal, or a much
  larger minimum), rather than a fixed 3 rows.
- Compress the collection / tasks & awaits / calendar / sharing sections into a
  tighter layout (smaller rows, less padding, denser chips).
- Replace the long `dialog-hint` paragraphs with a short inline hint and an ⓘ
  info icon that reveals the full text on hover/click.
- Keep the form usable and non-overflowing on narrow screens.

## Open Questions

- Which hints stay visible (e.g. the sharing-mismatch warning) vs. move behind ⓘ?
- ⓘ revealed by tooltip (hover) or a small popover (click/tap-friendly)?
- Does the modal get taller, or does the textarea flex within the current max?
- Any of the lower sections collapsible instead of compressed?

## Resolution

Decisions taken with the user (2026-10-08): textarea flexes to fill the modal;
ⓘ opens a click/tap popover; lower fields compressed (not collapsible); the
dynamic sharing-mismatch warning stays visible.

- `.dialog--note-editor` is pinned to a definite height (bounded by the backdrop
  and the inherited `90vh` max-height) so `.note-editor-form` has slack; the
  textarea's `.form-group--grow` + `flex:1` absorbs it (min-height `8rem` floor,
  form scrolls on short screens). The textarea dropped its `rows=3` for `rows=6`.
- Lower fields compressed under `.note-editor-form` (smaller gaps, denser chips).
- New reusable `InfoHint` component (`components/info_hint.rs`): an inline ⓘ
  button + optional short hint, opening a fixed, viewport-clamped popover that
  dismisses on an outside tap (transparent click-catcher, same pattern as
  `mention_popover.rs`). Used for the body `@`/`#` syntax (floating over the
  textarea), "Tasks & awaits", and "Calendar".
- Drive-by: repaired a pre-existing stale e2e selector in `tests/test_hydration.py`
  (`.notes-sidebar-add` → `button[title='New note collection']`), unrelated to
  this change but the only red test.

Verified: `cargo test --workspace` (633 passed), WASM hydrate build, full
`pytest tests/` (122 passed), and Playwright screenshots at desktop and 380px
widths.

### Follow-up (same day)

Two issues reported after the first pass — the mouse wheel would not scroll the
note body, and a note with many tasks still shrank it a lot. Both came from the
form being one outer scroll container nested around the scrollable textarea.
Fixed by splitting the form: `.note-editor-form` no longer scrolls, the lower
fields are wrapped in a `.note-editor-details` region (`overflow-y:auto`,
`max-height:45%`) that scrolls instead, and the textarea `min-height` is
`10rem`. The body is now the only scroll area under the pointer and stays
~44% of the modal regardless of task count. Re-verified: `cargo test
--workspace` (633 passed), WASM build, `pytest tests/test_hydration.py` (17
passed), and screenshots with 30 inline tasks.
