# Consolidate a note to a markdown source (and vice versa)

Notes live in the database; markdown sources are files on disk (optionally
git-backed). Users want to move content across that boundary in both
directions: promote a note into a markdown source file, and import a markdown
file back into a note collection. This is the "consolidate my notes into a
repo" / "pull a file into the app" flow.

**Status:** done. Full design:
[spec](../../docs/superpowers/specs/2026-10-07-note-markdown-consolidation-design.md) ·
[plan](../../docs/superpowers/plans/2026-10-07-note-markdown-consolidation.md).

## Dependencies

- [18 - Markdown source files for todos and pending items](18-markdown-sources.md) — the source + `#task`/`#await` pipeline a note would be written into.
- [34 - Git-backed markdown sources with in-app editor](34-git-backed-markdown-sources.md) — the editor/writer used to create the file.
- [37 - Notes tab](37-notes-tab.md) — the note model being moved.
- [110 - Note image / attachment support](110-note-image-attachments.md) — note images are DB BLOBs a plain markdown file cannot hold.

## Functional Requirements

- From a note: an action that writes title + body + task/await tokens into a
  chosen markdown source as a new file (pick source, file name/slug, folder).
- From a markdown file (preview or finder): an action that imports it as a note
  into a chosen collection.
- Preserve `#task`/`#await` tokens, `[[wikilink]]`s, and `@mention`s across the
  move.
- Define the move semantics explicitly: copy vs. move, and what happens to the
  origin (note deleted? source file left in place? marked/archived?).
- Run twice safely (idempotency or a clear "already exists" path).
- Handle note images/attachments: either refuse, or write them alongside the
  markdown file (needs a decision, see Open Questions).

## Open Questions

- Where do note images go on a note → markdown move (sidecar files next to the
  `.md`, a subfolder, or block the move)?
- Does a note → markdown move delete the note, or leave it and just link them?
- One note ↔ one file, or can a whole collection be exported to a tree?
- Should round-trips be tracked so the two stay in sync, or is it a one-shot
  migration?

## Resolution

Resolved with the user: images go **alongside** the file; the note title becomes
a leading `# Title`; both modals offer a **keep vs. delete the origin** control;
a name collision is a hard error.

- **`crates/tidl-web/src/api/consolidate.rs`** (new): `consolidate_note_to_markdown`
  (writes the `.md` plus `<stem>.assets/` image sidecars, rewrites `@img:<id>`
  to markdown links, git-pushes in one commit, optionally deletes the note),
  `pull_markdown_into_note` (splits a leading `# Title`, imports local relative
  images as note images, optionally deletes the file), and
  `list_source_directories` (a source-scoped, relative-only folder listing).
  Pure transforms (`build_markdown_document`, `split_leading_h1`,
  `find_markdown_image_refs`, `sanitize_filename`, `normalize_rel_folder`, …)
  are unit-tested.
- **`tidl-server`**: `GitCommitPusher` now takes `Vec<String>`;
  `git_sync::commit_and_push_paths` stages writes and deletions in one commit.
- **UI**: a "Consolidate to Markdown" button in the note editor
  (`ConsolidateNoteModal` + `SourceDirPickerModal`), and a "Pull out into Note"
  button in the rendered-markdown preview (`PullOutNoteModal`).
- **Image rendering**: `get_markdown_rendered` rewrites local relative
  `![](…)` destinations to an authenticated
  `GET /api/md-images/{source_id}/{*rel_path}` route (new
  `tidl-server/src/md_image.rs`; session auth, containment, image-extension
  allowlist), so sidecar images actually render in the preview.
- Decision on images/attachments: written as sidecars next to the `.md`
  (`<stem>.assets/`), not refused; the reverse imports them back.
- Decision on origin: modal-controlled keep/delete in both directions.
- "One note ↔ one file" (no collection export) and "one-shot migration" (no
  round-trip tracking) remain explicit non-goals.
