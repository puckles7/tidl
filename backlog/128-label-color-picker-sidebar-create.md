# Label colour picker + create-from-sidebar

Give note labels the same editing affordances note collections already have:
a colour palette in the edit modal, and a "+" button beside the "Labels"
header in the sidebar to create a label directly (name + colour).

## Background

Backlog #114 added first-class labels but deliberately kept them thin: a label
was name-only in the UI, its colour hash-assigned on creation by
`resolve_label`, and the only way to create one was by typing its name in the
note editor. Note collections, by contrast, have a full editor (name, colour
palette, delete) reachable from a "+" button in the panel header.

## Functional Requirements

- The label edit modal gains a colour palette, reusing `LABEL_PALETTE`
  (backlog #127) and the collection modal's `.note-palette` swatch markup.
- A "+" button beside the "Labels" sidebar header opens the same modal in
  create mode, mirroring "Note collections"; the modal saves via a new
  `create_note_label(name, color)`.
- Editing saves name **and** colour via `update_note_label(id, name, color)`
  (renamed from `rename_note_label`); blank colour keeps the existing one.
- Case-insensitive per-owner name uniqueness preserved on both create and
  update; delete remains edit-only.
- Implicit creation from the note editor keeps hash-assigning from
  `LABEL_PALETTE` (factored into `label_color_for_name`).

## Resolution

Done as specified — see `note_collections_panel.rs`,
`notes/note_label_editor_modal.rs`, and `api/notes.rs`
(`create_note_label`, `update_note_label`). README updated.
