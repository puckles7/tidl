# Contacts — data model + Contacts tab (CRUD)

Add a Contacts tab: a per-user address book with rich contact records (name, org, birthday, address, multiple emails/phones, notes) and a create/read/update/delete UI. This is the foundational piece for #29 (CardDAV protocol sync) and the future @mention-linking feature — both build on the data model and CRUD API introduced here. No external protocol exposure in this item; that's #29.

## Context / Background

Part of reopening #29 (Investigate CardDAV, dropped 2026-06-21). The original spike concluded full CardDAV protocol support wasn't worth it for internal synergies alone (iMIP attendee resolution, wikilink-to-person). This time the ask is explicit: a real Contacts tab plus CardDAV sync, so the work is being split into build-order pieces rather than re-run as a spike. See #29 for the protocol-sync half and #80 for @mention linking.

Design decisions were made collaboratively (including a visual mockup pass) before writing this spec:

- **Layout**: reuses the Notes tab's free-form grid / row-span-sizing logic (`crates/tidl-web/src/components/notes/mod.rs`'s `NOTES_GRID_COLS`/`NOTE_CARD_WIDTH` pattern, #47/#48/#50) — card height grows with how filled-in a contact is, columns adapt responsively to viewport width.
- **No left sidebar** — unlike Notes (whose sidebar lists collections/filters), Contacts has a single flat address book per user, so there's no collection concept to filter by.
- **Card content**: cards show inline content (not just name), matching the "content card" mockup — org/title, all emails (with type label), all phones (with type label), and freetext notes, sized to however much of that is actually filled in. Full detail/edit happens in a modal, following the `NoteEditorModal` create/edit pattern.

## Dependencies

None — this is the foundational item. #29 and #80 depend on this.

## Functional Requirements

### Data model (new migration)

- `contacts`: `id, user_id, uid (vCard UID, generated), display_name, given_name, family_name, org, title, birthday, street, city, region, postal_code, country, notes, etag, created_at, updated_at`
- `contact_emails`: `id, contact_id, value, type (home/work/mobile/other), is_primary`
- `contact_phones`: `id, contact_id, value, type (home/work/mobile/other), is_primary`
- No photo field in v1 (see Open Questions).
- One implicit address book per user — no separate `addressbooks` table. A per-user sync-token column (needed by #29) can be added when that item lands; not required for CRUD-only v1.

### `#[server]` functions (`tidl-web/src/api/contacts.rs`)

- `list_contacts() -> Vec<ContactDto>`
- `get_contact(id) -> ContactDto`
- `create_contact(NewContact) -> ContactDto`
- `update_contact(id, ContactUpdate) -> ContactDto`
- `delete_contact(id) -> ()`
- `ContactDto`/`NewContact` are plain serde structs (no sqlx types), per CLAUDE.md §9. Multi-value email/phone lists are nested `Vec<ContactMethod { value, kind, is_primary }>` in the DTO.

### UI

- New `/contacts` route + nav entry (`crates/tidl-web/src/app.rs`, mirrors the existing `<Route path=path!("/notes") ...>` + `<A href="/notes">` pattern).
- `ContactsView`: search/filter input, "+ New contact" button, free-form grid of `ContactCard`s (reusing Notes' grid/row-span logic, no sidebar).
- `ContactCard`: name, org/title, email(s), phone(s), notes — sized to content.
- `ContactEditorModal`: create/edit form (mirrors `NoteEditorModal`'s structure) with repeatable email/phone rows (add/remove, type picker).
- Delete: inline confirm on the card (same pattern as #78, so both land consistently) or via the editor modal — decide during implementation, not a design fork worth blocking the spec on.

## Open Questions

- Photo: dropped from v1 (needs upload/blob storage infra tidl doesn't have yet). Revisit if CardDAV clients need it (#29).
- Contact "type"/kind (e.g. distinguishing a person from an organization/group vCard KIND) — out of scope for v1, all contacts are individuals.
- Birthday: stored as a date; whether it should also surface as a Focus/Tickler reminder is out of scope here — a natural follow-up, not blocking this item.

## Resolution

Built as specified, plus one explicit architectural decision made during implementation at the user's direction: **generalize the grid/masonry layout between Notes and Contacts rather than duplicating it.**

- **Migration** (`migrations/20260914000000_contacts.sql`): `contacts` / `contact_emails` / `contact_phones` as specified, with two deviations from the literal spec text — `owner_username TEXT REFERENCES users(username)` instead of a `user_id` FK (matches the existing `notes`/`note_collections` convention), and `grid_row`/`grid_col`/`grid_row_span` columns added to `contacts` (not in the original spec) to support the shared grid system below. `contacts.owner_username` was added to `USERNAME_COLUMNS` in `api/profile.rs` so a username rename cascades to it (caught by that file's own exhaustiveness test).
- **Shared grid math** (`crates/tidl-web/src/grid.rs`, new): the pure, side-effect-free masonry/responsive-column logic previously private to `components/notes/mod.rs` and `api/notes.rs` — column packing, virtual/real column compaction for narrow viewports, drop-target resolution, and content-height self-correction — was extracted into this shared module (constants renamed `NOTES_GRID_COLS`/`NOTES_ROW_UNIT_PX` → `CARD_GRID_COLS`/`CARD_GRID_ROW_UNIT_PX`/`CARD_GRID_GAP`; `NotePosition` → `GridPosition`). `api/notes.rs` and `components/notes/mod.rs` were refactored to call the shared functions instead of their own private copies — all of the original unit tests moved with the code and still pass unchanged. `api/contacts.rs` and `components/contacts/mod.rs` build on the same shared functions.
- **What stayed Notes-specific** (deliberately not shared, per the spec's own scope): the pinned/non-pinned zone split and add-tile reservation (Contacts has one flat zone, no sidebar/collection concept); pointer-drag manual reordering (`NoteDragState` and its pointermove/pointerup wiring) — the #79 spec's functional requirements never asked for manual contact reordering, and porting Notes' drag system was assessed as materially higher risk/scope than the CRUD tab asked for. Contacts still gets masonry auto-placement on create and the same content-height self-correction pass as Notes (so cards still size to their content and repack via "Auto-layout"), just without pick-up-and-drag.
- **Shared UI beyond the grid math**: `components/delete_confirm_button.rs` — a delete-confirm modal used by both `ContactCard` and `NoteCard`, closing backlog **#78** as a side effect (tracked and marked done separately) since the spec explicitly called for landing both consistently. Went through two rejected in-card designs before landing on a real modal — see #78's own Resolution for the detail, since that's the more relevant home for it.
- Server functions (`api/contacts.rs`): `list_contacts`, `get_contact`, `create_contact`, `update_contact`, `delete_contact`, plus `move_contacts`/`auto_layout_contacts` for the grid (mirroring `move_notes`/`auto_layout_notes`). `NewContact` is reused for both create and update input (no separate `ContactUpdate` type), matching the existing `NewNote` convention.
- UI: `/contacts` route + nav entry, `ContactsView` (search, "+ New contact", "⊞ Auto-layout", responsive grid, no sidebar), `ContactCard` (name/org·title/emails/phones/notes, sized to content), `ContactEditorModal` (repeatable email/phone rows with a home/work/mobile/other kind picker and a per-list primary flag).
- Verified end-to-end in a real browser at multiple points (Playwright against a throwaway isolated server instance, plus live iteration against the user's own dev server for the delete-confirm and email-field bugs below). Caught and fixed three real bugs in the process:
  - A freshly-added email/phone row defaulted its `kind` to `""` (not matching any `<select>` option) — passed client-side, failed the server's CHECK-constraint validation with an opaque error. Fixed by seeding a real default (`"other"`).
  - The Emails/Phones `<For>` list's `key` closure included the row's live `value`, so every keystroke changed the key and made Leptos remount the `<input>` — losing focus after each character typed. Fixed by keying on the row's index alone (its only real, stable identity — these rows have no id of their own until saved).
  - See #78's Resolution for the delete-confirm button's own two rejected iterations and their bugs.
