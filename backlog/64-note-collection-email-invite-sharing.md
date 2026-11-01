# Note-collection sharing via email invite (close username-enumeration hole)

`add_note_collection_member` currently looks up the target username directly
(`SELECT 1 FROM users WHERE username = ?`) and returns `"No such user"` if it
doesn't exist — any collection owner can use this to test whether an
arbitrary username exists on the server. Calendar sharing avoids this
entirely by always sending an email invite regardless of whether an account
exists; note-collection sharing should adopt the same pattern.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — reuses/generalizes its
  `share_invitations` table and `/share/accept|decline/{token}` routes.
- [37 - Notes tab](37-notes-tab.md) — introduced note collections and their
  `shared`/member-list model.
- [44 - Settings → Sharing: manage note-collection sharing](44-settings-sharing-note-collections.md)
  — the revoke-side UI this item's invite flow sits alongside; no changes
  needed there.

## Context / Background

`share_calendar` (`crates/tidl-web/src/api/calendars.rs`) inserts a row into
`share_invitations` (owner, calendar name, invitee email, permission, token,
7-day expiry) and emails a link to `/share/accept/{token}` —
`accept_share_invitation` (`crates/tidl-server/src/storage.rs`) only creates
the `calendar_shares` grant once the *logged-in* accepting user's own email
matches `invitee_email`. This never reveals whether the invitee email
belongs to an existing account.

Note collections instead let the owner type a username directly
(`add_note_collection_member`, `crates/tidl-web/src/api/notes.rs`), checking
`users` for existence first — the enumeration hole this item closes.

The same hole is also reachable through `create_note_collection` and
`update_note_collection`, which accept a client-supplied `members:
Vec<String>` on `NewNoteCollection` and silently
`INSERT OR IGNORE INTO note_collection_members ... SELECT username FROM
users WHERE username = ?` for each entry. The UI never drives this directly
(the editor modal only ever resends the server-confirmed member list it
already has), but the server-fn endpoint itself is still probeable: submit a
guessed username in `members`, save, then check `list_note_collections` to
see whether it stuck. This must be closed too.

## Functional Requirements

- Generalize the `share_invitations` table (SQLite requires a table rebuild
  to change a CHECK constraint, so migrate via the
  create-new/copy/drop/rename pattern already used in
  `migrations/20240119000000_drop_events_calendar_fk.sql`):
  - Add `resource_type TEXT NOT NULL CHECK (resource_type IN ('calendar', 'note_collection'))`,
    defaulting existing rows to `'calendar'`.
  - Add `collection_id INTEGER REFERENCES note_collections(id) ON DELETE CASCADE`,
    make `calendar_name` nullable.
  - Add a table-level CHECK enforcing exactly one of `calendar_name` /
    `collection_id` is set, matching `resource_type`.
  - Widen `permission`'s CHECK to add `'member'` (note collections have no
    read/read-write tiers — a member always gets full access).
- `ShareInvitation` (storage.rs) gains `resource_type`/`collection_id`;
  `calendar_name` becomes `Option<String>`.
- `accept_share_invitation` keeps its existing owner/expiry/email-match
  checks unchanged, then branches on `resource_type` at the grant step:
  `calendar` → existing `calendar_shares` upsert; `note_collection` →
  `INSERT OR IGNORE INTO note_collection_members` + flip
  `note_collections.shared = 1`.
- `/share/accept/{token}` and `/share/decline/{token}` handlers
  (`crates/tidl-server/src/sharing.rs`) need no changes — already
  resource-agnostic (token → generic accept/decline).
- New `share_note_collection(collection_id, invitee_email)` server fn in
  `notes.rs`, mirroring `share_calendar`: owner-only (`is_collection_owner`),
  7-day token expiry, sends the same style of invite email. Extract the
  shared email-building/sending code currently inline in `share_calendar`
  (HTML escaping, the `lettre::Message` builder, SMTP-config lookup) into a
  shared helper so both call sites use it instead of duplicating it.
- Delete `add_note_collection_member` entirely, including its
  username-existence lookup. `remove_note_collection_member` (owner revoking
  an already-known member) is unaffected — no enumeration risk there.
- UI: `NoteCollectionShareDialog`'s "Add by username" input/button
  (`crates/tidl-web/src/components/notes/collection_editor_modal.rs`) is
  replaced with an email input + "Send invite" button calling
  `share_note_collection`, mirroring `CalendarShareDialog`'s invite form but
  without a permission `<select>` (single access tier). Update the stale doc
  comments describing "no invite/accept flow" at the top of that file and on
  `NoteCollectionShareDialog`.
- `create_note_collection`/`update_note_collection` must stop trusting the
  submitted `members` list to *add* anyone. Membership can only grow via an
  accepted invite. On save, intersect the submitted `members` with the
  collection's current confirmed members (empty set for a brand-new
  collection, so `create_note_collection` effectively ignores any submitted
  members) — drop any entry not already a real member instead of doing a
  fresh `users` lookup. This removes the `INSERT OR IGNORE ... SELECT
  username FROM users` pattern from both functions entirely; they only ever
  shrink or leave membership unchanged.

## Testing

- storage.rs: add `note_collection` counterparts to the four existing
  `accept_share_invitation_*` tests — creates membership + flips `shared`,
  case-insensitive email match, rejects the owner accepting their own
  invite, rejects a different user's email.
- No existing notes.rs unit tests reference `add_note_collection_member`, so
  none need removal.

## Open Questions

None — resolved during design (generalize the existing table rather than add
a parallel one; widen `permission` with a `'member'` sentinel rather than
making it nullable; fully replace the username-add path rather than keep it
as a fallback).

## Resolution

Implemented as designed: `share_invitations` generalized with a
`resource_type` discriminator + a `ShareResource` enum in `storage.rs`
(`Calendar { name, permission }` / `NoteCollection { id }`) so the
calendar-vs-collection fields are mutually exclusive at the type level, not
just via a table CHECK. `accept_share_invitation` branches on the resource
at the grant step; `/share/accept|decline/{token}` needed no changes since
they were already resource-agnostic. A new `share_note_collection` server
fn mirrors `share_calendar`, both now sharing one extracted
`send_share_invitation_email` helper in the new `crates/tidl-web/src/api/sharing.rs`
(which also picked up `html_escape`, previously living in `calendars.rs`).
`add_note_collection_member` is deleted; `create_note_collection`/
`update_note_collection` now filter the submitted `members` list against
`existing_note_collection_members` so they can only ever retain
already-confirmed members, never add new ones. The UI's "Add by username"
form in `NoteCollectionShareDialog` was replaced with an "Invite by email"
form calling `share_note_collection`. No deviations from the approved
design.

Not verified with a live browser smoke test — the dev environment's
database (`data/tidl.db`) is shared with the main checkout and may have
real SMTP configured, so creating test collections/invites there risked
writing test data into real personal data or sending a real email. Coverage
instead comes from the full workspace test suite (migration + 4 new
`storage.rs` accept-invitation tests for `note_collection` + 2 new
`existing_note_collection_members` tests), which all pass, plus clean
`cargo clippy`/wasm32 builds.
