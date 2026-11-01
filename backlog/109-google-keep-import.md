# Google Keep import — ingest Keep exports as notes

**Status: done.** Full design:
[spec](../../docs/superpowers/specs/2026-10-01-google-keep-import-design.md) ·
[plan](../../docs/superpowers/plans/2026-10-01-google-keep-import.md). Where the
spec differs from the wish, the spec wins (notably: JSON-only parsing, `.html`
ignored; one note per Keep list with inline `#task`/`#done` tokens; Settings →
Import as the entry point; shared notes grouped per person; re-imports de-duped).

Import a Google Keep export (Google Takeout) so existing Keep notes land as
tidl notes in a chosen collection, preserving title, text/body, list items,
labels, colors, pin state, and timestamps where possible. Motivation: let
people migrate off Keep without retyping, mirroring the existing Contacts
import (#87) and calendar import (#10).

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — notes and note collections are the import target.
- [110 - Note image / attachment support](110-note-image-attachments.md) — required to import Keep image attachments rather than drop them.

## Background

Google Takeout exports Keep as a `.zip` containing, per note, either:

- `Takeout/Keep/<note>.html` — a rendered HTML file (title, body, checkbox
  list items, labels, timestamps), and
- `Takeout/Keep/<note>.json` — the structured form: `title`, `textContent`
  (with checkbox markup), `listContent[]` (`text`, `isChecked`), `labels[]`,
  `color`, `isTrashed`, `isArchived`, `isPinned`, `createdTimestampUsec`,
  `userEditedTimestampUsec`, `attachments[]`.

The JSON is the more faithful source; the HTML is a fallback for exports that
lack it. Keep's color is a small fixed palette (DEFAULT, RED, ORANGE, YELLOW,
GREEN, TEAL, BLUE, PURPLE, PINK, BROWN, GRAY).

**Images**: Takeout exports the actual attachment files into the same
`Takeout/Keep/` folder alongside the JSON/HTML (not embedded in the JSON, and
not merely URL references to Keep's servers). Each JSON `attachments[]` entry
carries only a `filePath` (the filename within that folder) plus `mimetype`, so
the bytes are available locally and can be re-attached as note images (#110).

## Functional Requirements

- Accept an uploaded Keep Takeout `.zip` (and ideally a folder of `.json`/`.html`
  files) from the Notes UI.
- Parse each note into a tidl note: title → note title, text → body; Keep
  list notes → `TASK`/checklist items preserving checked state.
- Map labels to tidl note collections (or tags), with an option to import all
  into one target collection.
- Skip trashed notes by default; offer an option to include archived notes.
- Preserve pinned state and created/edited timestamps.
- Report a summary (imported / skipped / failed) and never partially corrupt
  existing notes on error.

## Open Questions

- Map Keep labels → existing/new collections, or to a tag/label concept tidl
  doesn't yet have?
- How to handle Keep images/attachments referenced by the export (copy into
  note attachments, or drop with a warning)?
  Resolved in principle: the bytes ship with the export, so import them as note
  images — depends on #110.
- Color mapping: Keep palette → tidl note color, or ignore colors?
- Checkbox lists: import as a single note body vs. multiple checklist items —
  which fits the notes/`TASK` model best?
- De-duplication on repeated imports (by Keep note id) to allow re-runs?
  Resolved: no note id exists, so `createdTimestampUsec` is persisted as the
  key; re-imports drop duplicates (see the follow-up below).

## Resolution

Implemented as specified. New `crates/tidl-web/src/api/keep.rs`:

- Parses each `Takeout/Keep/*.json` into a note; `.html` files are ignored.
  Trashed notes are always skipped, archived ones unless opted in.
- Title, `textContent` body, inline `#task`/`#done` tokens for `listContent`
  (see #113), pinned state, and created/edited timestamps are all preserved.
- Shared notes (`sharees[]` non-empty) are grouped into `Shared with <email>`
  collections (created on demand, palette colour); everything else lands in the
  chosen target collection. Labels are **not** mapped (deferred to #114).
- Attachments are re-attached as note images through #110's
  `insert_note_image`, appended as `@img:<id>` tokens, honouring the shared
  content-type allowlist and 5 MiB cap.
- The whole import is one transaction; returns
  `KeepImportResult { imported, skipped, failed, images }`.

UI in Settings → Import ("Google Keep (.zip)"): `.zip` picker, target-collection
select, include-archived toggle, and a result summary. A new `NotesRefresh`
context makes an already-open Notes tab refetch.

Deviations from the original wish / open questions:

- **JSON source only**; the HTML fallback was not implemented (modern Takeout
  always emits JSON) — deferred.
- **One note per Keep list**, checklist state as inline `#task`/`#done` tokens
  (#113), rather than one task note per item (decided with the user).
- **Colors ignored** (tidl has no per-note colour).
- **De-dup** implemented in a follow-up (see below): no note id exists, so the
  stable `createdTimestampUsec` is persisted and re-imports are dropped.
- Added the `zip` crate (`ssr`-only, `deflate` feature) for archive parsing.

Follow-up (2026-10-01, collection selection):

- The "Import into" picker now offers an **"Add new collection…"** entry with a
  name field, and is preselected to it when the user owns no collections yet
  (mirroring the `.ics` "New calendar…" pattern), so an import no longer needs a
  pre-existing collection. The server fn's `target_collection_id` became
  `Option<i64>` plus an optional `new_collection_name`, resolved
  find-or-create *inside* the import transaction so a failed import leaves no
  orphaned collection.
- Import is now enabled once a `.zip` is chosen; an empty or blank target is
  reported inline ("Choose a collection…" / "Enter a name for the new
  collection.") instead of only disabling the button.

Follow-up (2026-10-01, sharing replaces label mapping):

- Dropped the lossy label → collection mapping entirely. Labels are deferred to
  backlog #114 (tidl has no tag concept; the archive carried labels on almost
  none of the user's notes anyway).
- Added sharing support: Takeout's `sharees[]` (`{email, type, isOwner}`) names
  the *other* party on a shared note (the exporter's own address never appears;
  `isOwner` just says which side owns it). A note with a non-empty `sharees[]`
  is imported into a `Shared with <email>` collection, one combined collection
  for a multi-sharee note; private notes go to the chosen target. Grouping is
  automatic (no toggle).
- tidl can't auto-share (membership needs an accepted email invite and the
  archive has no tidl username), so the grouping is a migration aid the user can
  share manually from Settings → Sharing.

Follow-up (2026-10-01, de-duplication):

- Migration `20261001000001_note_import_key.sql` adds nullable
  `notes.import_source` + `notes.import_external_id`; every imported note is
  stamped `('keep', <createdTimestampUsec>)`, the stable per-note key.
- A re-imported note (matched by that key, scoped to the caller's own notes) is
  **dropped**, not duplicated. A miss falls back to adopting a pre-migration
  import by `created_at` + title and stamping it, so already-imported notes
  aren't duplicated on the first re-import.
- Duplicates are never overwritten — a note may sit in a shared collection
  another user has edited. `KeepImportResult.duplicates` surfaces the count in
  the summary ("N already imported").
