# CardDAV/Contacts — photo (PHOTO) support

Add photo support to tidl's contacts: store a photo per contact, show it in the web UI, and round-trip it through CardDAV's `PHOTO` vCard property so real clients (iOS Contacts, Thunderbird, DAVx5) can sync it — the last major vCard property #29 didn't implement (it was explicitly deferred there: "`PHOTO` stays out per #79's own decision, no blob storage infra exists").

## Dependencies

- [29 - CardDAV protocol — read-write contact sync](29-carddav-protocol.md) — the vCard parser/generator and CardDAV routes this extends.
- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — the `contacts` table and web UI this extends.

## Context / Background

Design discussion (2026-09-17, brainstorming skill, architectural path) settled the open question of where photo bytes should live and how they should move through the system. Key decisions, with reasoning:

- **Storage: SQLite `BLOB` column on `contacts`**, not files on disk. Atomic with the contact row (backups/exports/deletes just work), no new `TIDL_DATA_DIR`-relative directory to manage or keep in sync with the DB, no orphan-file cleanup, and it survives the Windows cross-compile build the same way the rest of the DB does. Personal-contact-photo scale (dozens–hundreds of contacts, each up to a few MB) is trivial for SQLite.
- **Upload transport: base64 through the existing `#[server]` fn pattern**, not a new multipart endpoint. Checked `server_fn`'s actual axum integration (`server_fn-0.8.12/src/request/axum.rs`): it reads the whole request body via a raw, unbounded `body.collect().await.map(|c| c.to_bytes())`, bypassing axum's own 2MB `DefaultBodyLimit` (which only applies to axum's own `Bytes`/`Json`/`Form` extractors, confirmed from axum 0.8.9's docs). So there's no hidden framework ceiling either way — the choice is purely "add a new multipart pattern to the codebase" vs. "reuse the existing one with our own explicit cap." At a 5 MiB cap, base64's ~33% wire overhead and extra in-memory copy are immaterial, so multipart isn't worth being the first of its kind here.
- **Size cap: 5 MiB of raw (decoded) photo bytes**, enforced in exactly two places: `tidl_carddav::vcard::parse_vcard` (the CardDAV-PUT entry point) and the new `upload_contact_photo` server fn (the web-UI entry point) — the only two ways photo bytes enter the system.
- **Photo changes are a separate call, decoupled from `NewContact`** (`upload_contact_photo`/`delete_contact_photo`), not folded into the existing full-replace-every-field pattern the rest of `NewContact` uses. Editing an unrelated field (e.g. a phone number) never re-sends up to 5MB of photo bytes; uploading a photo never touches other fields. A deliberate departure from the `NewContact` full-replace convention, made explicitly for this reason.
- **`ContactStore`/`ContactRecord` (tidl-server, CardDAV-facing) carries full photo bytes in both `list_contacts` and `get_contact_by_uid`** — verified `carddav_handlers.rs`'s `addressbook-query` REPORT branch builds full vCards straight from `list_contacts`'s result, so that path needs the bytes there, not just in the single-contact `GET`/multiget path. The **web UI's own contact-list query stays light**: it's a separate SQL implementation in `tidl-web/api/contacts.rs` (already duplicated from `ContactStore` across the compilation boundary, same as the existing `bump_contacts_sync_token` duplication from #29), and only needs a computed `has_photo` boolean, not the bytes — actual bytes are served through a dedicated `GET /api/contacts/{id}/photo` Axum route instead of being embedded in the grid's JSON payload.
- **Wire format: vCard 3.0 `PHOTO;ENCODING=b;TYPE=JPEG:<base64>`** (or `TYPE=PNG`), matching every other property `tidl-carddav::vcard` already emits for iOS/Thunderbird. Also defensively parses the vCard 4 `PHOTO:data:image/jpeg;base64,<data>` URI form, since it costs nothing extra to accept. Not folded at 75 octets on generation — no other property in this file folds either (a pre-existing gap, out of scope here), and real clients have tolerated the unfolded form for every other field so far.

## Functional Requirements

### Schema

- New migration `20260917000001_contact_photos.sql`: `contacts` gains `photo BLOB NULL`, `photo_content_type TEXT NULL`.

### `tidl-carddav`

- Add `base64` as a dependency (already a workspace dependency used elsewhere; just not wired into this crate yet).
- `VCardFields` gains `photo: Option<VCardPhoto>` where `VCardPhoto { bytes: Vec<u8>, content_type: String }` (canonical `"image/jpeg"` / `"image/png"` / `"image/gif"`).
- `parse_vcard`: parse `PHOTO;ENCODING=b;TYPE=JPEG:<base64>` and the `data:` URI form; map `TYPE=JPEG/PNG/GIF` ↔ `image/jpeg`/`image/png`/`image/gif`. Enforce the 5 MiB decoded cap — an oversized or undecodable `PHOTO` is dropped silently (contact still parses) rather than failing the whole vCard, consistent with this parser's existing "skip what we don't handle" behavior for other unsupported properties.
- `generate_vcard`: emit `PHOTO;ENCODING=b;TYPE=JPEG:<base64>` (or `TYPE=PNG`/`TYPE=GIF`) when present.

### `tidl-server`

- `ContactRecord` gains `photo: Option<Vec<u8>>`, `photo_content_type: Option<String>`; `ContactStore::list_contacts`/`get_contact_by_uid` populate both.
- `store_contact_vcard` maps `VCardFields.photo` ↔ `ContactRecord.photo`/`photo_content_type` alongside every other field, so a photo added from a CardDAV client (iOS, Thunderbird) syncs into tidl the same way URL/nickname/etc. already do.
- New `GET /api/contacts/{id}/photo` Axum route (new handler, session-cookie auth mirroring `export.rs`'s pattern exactly): resolves the session, verifies the contact belongs to that user, runs a targeted `SELECT photo, photo_content_type` (not the full row), returns the bytes with the right `Content-Type` and a `Cache-Control` header. 404 if the contact has no photo or isn't the caller's.

### `tidl-web`

- `ContactDto` gains `has_photo: bool`; the web UI's own list-contacts SQL computes it (`photo IS NOT NULL`) without selecting the blob.
- New `#[server]` fns in `api/contacts.rs`: `upload_contact_photo(id: i64, base64_data: String, content_type: String) -> Result<ContactDto, ServerFnError>` and `delete_contact_photo(id: i64) -> Result<ContactDto, ServerFnError>`. Both validate ownership, reject anything over the 5 MiB decoded cap, restrict `content_type` to `image/jpeg`/`image/png`, and bump `address_books.sync_token` + log to `contact_sync_log` (mirroring the existing `bump_contacts_sync_token` helper from #29) so CardDAV clients see the change.
- `web-sys` feature list gains `FileReader` (already has `File`/`FileList`/`HtmlInputElement`/`Blob`).
- `contact_editor_modal.rs`: file input (`accept="image/jpeg,image/png"`) + preview; reads the picked `File` via `FileReader` as a data URL client-side, stages the base64 payload + content type in a signal, and calls `upload_contact_photo`/`delete_contact_photo` after `create_contact`/`update_contact` resolves (using the resulting id for the create case).
- `contact_card.rs`: small avatar — `<img src="/api/contacts/{id}/photo">` when `has_photo`, else an initials-style placeholder (none currently exists; adding one as part of this).

### Testing

- `tidl-carddav`: `PHOTO;ENCODING=b;TYPE=JPEG` parsing, the `data:` URI form, `TYPE=PNG`, an oversized `PHOTO` dropped without failing the vCard, generate→parse round trip.
- `storage.rs`: extend existing contact round-trip/update/upsert tests to cover photo bytes.
- `api/contacts.rs`: `upload_contact_photo` happy path, oversized rejection, non-jpeg/png rejection, `delete_contact_photo`, sync-token bump on both.
- New photo route: 401 with no session, 404 for no-photo/not-owned, 200 with correct bytes + `Content-Type` when authenticated and owned.
- Manual: upload a photo via the Contacts tab, confirm it renders on the card and in the modal; confirm a CardDAV `GET .../contacts/{uid}.vcf` returns a `PHOTO` property.

## Open Questions

- Server-side resizing/compression of uploaded photos — deliberately out of scope (would need a new image-processing dependency for a personal-scale feature where phone contact photos are already small); revisit only if the 5 MiB cap proves too generous in practice.
- 75-octet line folding for `generate_vcard` output generally (not photo-specific) — a pre-existing gap across every property in this file, not addressed here.

## Resolution

Built as specified; every decision in the Context/Background section above was implemented as described (SQLite `BLOB` column, base64-through-`#[server]`-fn upload with a 5 MiB cap enforced independently at both entry points, decoupled `upload_contact_photo`/`delete_contact_photo` calls, light `has_photo`-only web-UI list query with a dedicated `GET /api/contacts/{id}/photo` route for actual bytes, vCard 3.0 `PHOTO;ENCODING=b;TYPE=JPEG/PNG:` wire format with defensive vCard-4 `data:` URI parsing).

- **Schema**: `migrations/20260917000001_contact_photos.sql` adds `contacts.photo`/`contacts.photo_content_type`.
- **`tidl-carddav`**: `VCardPhoto` (bytes + canonical MIME type), `MAX_PHOTO_BYTES` (5 MiB), `parse_photo`/`mime_from_type_token`/`type_token_from_mime` in `vcard.rs`. `base64` added as a dependency (already a workspace dep, just newly wired into this crate).
- **`tidl-server`**: `ContactRecord` carries full `photo`/`photo_content_type` (needed by CardDAV's `addressbook-query` REPORT, which builds vCards straight from `list_contacts`). New `ContactStore::get_contact_photo_by_id` (owner+id scoped, used only by the plain HTTP route). New `crates/tidl-server/src/contact_photo.rs` — a plain Axum `GET /api/contacts/{id}/photo` handler, session-cookie auth mirroring `export.rs`'s existing pattern exactly.
- **`tidl-web`**: `ContactDto.has_photo: bool` (list/get queries compute it via `photo IS NOT NULL`, never select the blob for the grid). New `upload_contact_photo`/`delete_contact_photo` `#[server]` fns in `api/contacts.rs`, with validation split into a standalone `decode_and_validate_photo` helper (testable without the server-fn request-context requirement, mirroring `validate_new_contact`'s existing reasoning for being its own function). Both bump the CardDAV sync token via the existing `bump_contacts_sync_token` helper.
- **UI**: `contact_editor_modal.rs` gets a photo picker — reads the picked `File` via `File::array_buffer()` + `JsFuture` (matching `import.rs`'s existing `file.text()` pattern rather than introducing `FileReader`'s callback-based API) and base64-encodes it client-side (`base64` added to the `hydrate` feature too, alongside its existing `ssr`-only use). Staged client-side, sent via its own call after `create_contact`/`update_contact` resolves. `contact_card.rs` gets a small circular avatar (photo or an initials placeholder — didn't exist before this item).
- **Deviation from the plan**: none of substance — the design as approved was implemented as written.
- **Verified**: `cargo test --workspace` (605 tests, including new coverage in all four touched crates), `cargo clippy --workspace -- -D warnings`, `cargo build --target wasm32-unknown-unknown -p tidl-web` — all clean. New/changed code was checked against `cargo fmt --check`; the repository as a whole already has substantial pre-existing formatting drift unrelated to this item (confirmed by diffing which flagged lines predate this change), which was left alone rather than mass-reformatted.
- **Manual verification**: no browser extension was available in this session, so the golden path was verified over raw HTTP instead of a real browser — a full CardDAV `PUT`/`GET` round trip of a vCard with an inline JPEG `PHOTO` property (byte-exact match confirmed against the source file), the photo persisted correctly as a SQLite `BLOB`, the new `GET /api/contacts/{id}/photo` route returning `401` with no session / `404` for another user's contact or no photo / `200` with the correct `Content-Type` and byte-exact body when authenticated and owned, and a follow-up `PUT` without a `PHOTO` property correctly clearing it. The web UI's own `#[server]` fn call path (`upload_contact_photo`/`delete_contact_photo` invoked from the browser) was not separately exercised live — while investigating, `do_login` and even a trivial no-arg `get_registration_enabled` also failed to route via direct `curl` POSTs in this dev-build environment ("could not find a server function at the route"), which is a pre-existing characteristic of how `cargo leptos build`'s dev profile registers server functions, not something introduced by this item — it affects every `#[server]` fn in the app equally, confirmed by testing an unrelated existing one. The new fns follow the exact same `auth()`/`is_contact_owner`/`sqlx` pattern as the already-proven `update_contact`/`create_contact`, and are covered by unit tests (`decode_and_validate_photo`) and an `sqlx::test` integration test exercising the same SQL and sync-token bump.
