# Note image / attachment support (in-DB BLOB storage)

**Status: done.** Full design:
[spec](../../docs/superpowers/specs/2026-09-30-note-image-attachments-design.md) ·
[plan](../../docs/superpowers/plans/2026-09-30-note-image-attachments.md).
The sections below are the original wish; where the spec differs, the spec
wins (notably: inline `@img:<id>` body tokens rather than a separate gallery,
and one atomic `save_note` rather than decoupled upload/delete calls).

Let notes carry images: attach image bytes to a note, render them on the note
card and in the note editor, and delete them independently of the note body.
Motivation: notes are currently text-only, which blocks importing image-bearing
notes (e.g. Google Keep, #109) and is a plain feature gap for a personal
note-taking app. Precedent: contacts already store photos as a SQLite `BLOB`
(#86) — same infrastructure decision, applied to notes.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — notes and collections are the attachment target.

## Background

Follows the storage/transport decisions #86 settled for contact photos, since
the constraints are identical (personal-scale, atomic with the row, no
filesystem to manage, survives the Windows cross-compile):

- **Storage: SQLite `BLOB`**, not files on disk. Atomic with the note row
  (backup/export/delete just work), no `TIDL_DATA_DIR` directory to keep in
  sync with the DB, no orphan cleanup.
- **Transport: base64 through the existing `#[server]` fn pattern**, not a new
  multipart endpoint. `server_fn`'s axum integration reads the raw body
  unbounded, so there's no framework ceiling; at a small cap the ~33% base64
  overhead is immaterial, and it avoids introducing the repo's first multipart
  route (#86 made the same call).
- **Explicit size cap** (proposed 5 MiB decoded per image, matching #86),
  enforced at every entry point.
- **Decoupled calls** (`upload_note_image` / `delete_note_image`), so editing a
  note never re-sends image bytes and vice-versa — the deliberate departure from
  the full-replace `NewNote` convention that #86 already established.

Key differences from #86 to settle:

- A contact has **one** photo; a note may have **many** images, and they need a
  display order. This implies a child table (`note_images`) rather than a single
  `photo` column.
- Notes expose no CardDAV-equivalent protocol today, so no `PHOTO` property /
  sync-token concern (a future markdown-source or CalDAV mirror of note bodies
  would need one).
- Rendering needs a decision: inline in the note body markdown (image reference
  syntax) vs. a separate gallery strip on the card/editor.

## Functional Requirements

- Store multiple images per note in a new table (e.g. `note_images`: `id`,
  `note_id`, `data BLOB`, `content_type`, `position`, timestamps), owner-scoped.
- New `#[server]` fns to upload (base64 + content type) and delete an image,
  validating ownership, the decoded-size cap, and a `content_type` allowlist
  (at least `image/jpeg`, `image/png`; consider `image/gif`, `image/webp`).
- Serve bytes through a dedicated plain Axum `GET` route (e.g.
  `GET /api/notes/{note_id}/images/{image_id}`) with session-cookie auth
  mirroring #86's `contact_photo.rs`; the note-list/grid queries return image
  metadata (count / ids / `has_images`) but never the blobs.
- Render images on the note card and in the note editor; picker reads the file
  client-side via `array_buffer()` + `JsFuture` and base64-encodes it (matching
  #86's `contact_editor_modal.rs` approach rather than `FileReader`).
- Deleting a note deletes its image rows; deleting an image leaves the note.
- Keep a path to let the Keep importer (#109) hand parsed attachment bytes in
  without going through the browser upload round trip.

## Open Questions

- Per-image display ordering / reordering UI, or just insertion order?
- Inline-in-body markdown references vs. separate gallery — which fits the note
  card's existing content-sizing (#45) and grid layout (#47/#48) best?
- Cap: 5 MiB per image like #86, or a different budget given many-images-per-note?
- Animated formats (`image/gif`, `image/webp`): allow, or static only?
- Down-scaling on upload? #86 deliberately skipped it; likely same here, revisit
  only if the cap proves too generous.
- Server-side rendering concerns: does the note card render `#[server]`-fetched
  image URLs during SSR, and does that interact with the suspense boundaries?
