# Note labels (and Google Keep `labels[]` import)

Give notes a **tag/label** concept and use it to import Google Keep's
`labels[]` faithfully. Today labels are parsed by no code path: backlog #109's
original "first label → collection" mapping was lossy (one collection per note,
extra labels dropped) and has been removed, so Keep labels are currently
discarded on import.

A single Keep note can carry several labels; tidl notes belong to exactly one
collection, so representing them needs either a first-class tag concept or a
convention that composes with the existing inline-token model.

**Status:** done.

**Spec:** [2026-10-07-note-labels-design.md](../docs/superpowers/specs/2026-10-07-note-labels-design.md)
**Plan:** [2026-10-07-note-labels.md](../docs/superpowers/plans/2026-10-07-note-labels.md)

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — the notes model this extends.
- [109 - Google Keep import](109-google-keep-import.md) — the importer this
  extends; labels were deliberately dropped there.

## Resolved questions

- **Data model:** first-class per-user `note_labels` + `note_label_links`
  tables (not inline tokens) — supports rename, per-label colour, and Keep's
  entity semantics.
- **Filtering UI:** a "Labels" show/hide panel in the filter sidebar, mirroring
  "Note collections".
- **Markdown:** labels are expressed as YAML frontmatter `tags:`; this pass adds
  a canonical structured parser, filtering markdown items by tag is a follow-up.

## Resolution

Built first-class, per-user note labels (backlog #114):

- **Schema** (`migrations/20261007000000_note_labels.sql`): `note_labels`
  (`owner_username`, `name`, `color`, unique per owner+name) and
  `note_label_links` (many-to-many, cascade on note/label delete).
- **Server** (`api/notes.rs`): `NewNote.labels: Vec<String>`; `insert_note_row`
  / `update_note_row` resolve-or-create each name and rewrite the note's links
  in the same transaction. Name resolution is case-insensitive; the owner is the
  note's *author*, never the viewer (a grantee editing a shared note links the
  author's labels). New fns: `list_note_labels`, `rename_note_label`,
  `delete_note_label`. `NoteInfo.labels: Vec<NoteLabelInfo>`.
- **Keep import** (`api/keep.rs`): `labels[]` parsed (trimmed, case-insensitively
  deduped) and attached on import; `KeepImportResult.labels` counts them.
- **UI**: label chips on note cards; a label picker (selected chips + suggestion
  chips + free-text input with `datalist`) in the note editor; a **Labels**
  panel in the Focus sidebar and a matching one in the Notes tab's own sidebar
  (show/hide, rename, delete) backed by `HiddenNoteLabelsCtx`.
- **Markdown convention** (`tidl-markdown/parser.rs`): the frontmatter renderer
  was refactored onto a structured `parse_frontmatter_entries`, and a public
  `frontmatter_tags()` reads `tags:` in both block-list and inline-array forms.
  Filtering markdown-backed items by tag remains out of scope (future item).

`rename_username` now cascades `note_labels.owner_username` (added to
`USERNAME_COLUMNS`, enforced by the exhaustive profile test).

Deviations from the spec: the sidebar Labels panel lists the caller's *owned*
labels (not a union across visible notes), so a label only appearing on a shared
note is shown as a chip but isn't filterable by a grantee — accepted for v1.

