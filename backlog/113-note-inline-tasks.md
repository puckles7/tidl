# Inline tasks and awaits in notes

Let a single note hold **arbitrary** tasks and awaits, authored inline the way
images are (`@img`): a modal (name + priority slider / date) or a typed
`#task/p<N>/<slug>` / `#await/<date>/<slug>` token. Each token becomes its own
VTODO surfacing in Tasks / Await / Focus (and CalDAV). Replaces the current
single-task-per-note model (`notes.kind`) and gives the Google Keep import
(#109) a real target instead of literal `[ ]`/`[x]` text.

**Status:** done. Full design:
[spec](../../docs/superpowers/specs/2026-10-01-note-inline-tasks-design.md) ·
[plan](../../docs/superpowers/plans/2026-10-01-note-inline-tasks.md).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — the note body/editor this extends.
- [49 - TASK/AWAIT kind unification](49-task-await-unification.md) — the
  `notes.kind` model this replaces and the markdown tag convention it reuses.
- [18 - Markdown source files](18-markdown-sources.md) — the `#task`/`#await`
  parser and VTODO sync shape this mirrors.
- [110 - Note image / attachment support](110-note-image-attachments.md) — the
  `@img` button/modal + inline-token UX this parallels.

## Functional Requirements

- Inline `#task/p<N>/<slug>` and `#await/<date>/<slug>` tokens in note bodies,
  recognized at word boundaries; plus a `#done/<date>/<original-spec>/<slug>`
  completion record.
- Each token syncs to its own `events` VTODO (`source='note'`, `events.note_id`)
  so it appears in Tasks / Await / Focus with collection attribution and CalDAV
  exposure.
- Editor: "+ Task" / "+ Await" buttons open a modal (free-text name → slug,
  `PrioritySlider` for tasks, date picker for awaits); a `#` typeahead offers the
  same modal or hand-typed tokens; a chip row lists recognized tokens.
- Completing a note task rewrites its token to `#done/...` (retaining the
  original priority/date); re-opening rewrites it back.
- Remove the Plain/Task/Await Type chips and task due-date toggle.
- Migrate existing note-level TASK/AWAIT notes into body tokens.
- Google Keep list items import as `#task` (unchecked) / `#done` (checked).

See the spec for the full grammar, synchronization, migration and non-goals.
