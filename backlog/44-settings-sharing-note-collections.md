# Settings → Sharing: manage note-collection sharing

Extend the existing Settings → Sharing panel so that note-collection sharing
can be reviewed and removed from the same screen as calendar sharing, not
just from the Notes tab's collection editor.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — the panel this item
  extends already implements calendar-side revoke; this item mirrors that
  pattern for note collections.
- [37 - Notes tab](37-notes-tab.md) — introduced note collections and their
  `shared`/member-list model.

## Context / Background

`SharingPanel` (`crates/tidl-web/src/components/settings/sharing.rs`) already
lets a user pick one of their calendars, see its "Current access" list, and
click "Revoke" per grantee (`revoke_calendar_share`) — this already works
today and needs no changes.

Note-collection sharing has no equivalent here. The only way to remove a
member from a shared collection, or turn a collection back to private, is to
open that collection's edit modal from the Notes tab
(`CollectionEditorModal`) and either remove a member chip or uncheck
"Shared", then save (`update_note_collection`, which does a wholesale
replace of `note_collection_members`). There is currently no note-collection
section in the Settings → Sharing tab at all.

## Functional Requirements

- Add a second section to `SharingPanel` (or a clearly separated block within
  it), following the same layout pattern as the calendar section: a picker
  for the current user's owned shared collections, then a "Current access"
  list of members with a per-member "Revoke" button.
- Reuse `list_note_collections` (already returns `NoteCollectionInfo` with
  `members`) to populate the picker/list — do not add a new server fn for
  listing.
- Removing a member here should take effect immediately (same
  immediate-effect model as calendar revoke), not require opening the full
  collection editor. This likely means either a new small server fn (e.g.
  `remove_note_collection_member(id, username)`) or reusing
  `update_note_collection` with the member already filtered out — prefer
  whichever avoids a wholesale-replace footgun if members are being edited
  concurrently from two places (collection editor + settings panel).
- Only the collection owner may see/manage its member list here, mirroring
  `is_collection_owner` checks already used in `update_note_collection`.
- Unsharing a collection entirely (equivalent to unchecking "Shared" in the
  editor) should be possible from this screen too, not just member-by-member
  removal.

## Open Questions

- Whether to add a dedicated `remove_note_collection_member` server fn vs.
  reusing `update_note_collection` — resolve during implementation based on
  how easy it is to avoid clobbering concurrent edits from the collection
  editor modal.

## Resolution

Added a "Note collections" section to the bottom of `SharingPanel`
(`crates/tidl-web/src/components/settings/sharing.rs`), mirroring the
existing calendar section's layout: a picker limited to collections the
current user owns *and* has shared, a "Current access" list with a
per-member "Revoke" button, and an "Unshare this collection" action.

Went with two new dedicated server fns in
`crates/tidl-web/src/api/notes.rs`, resolving the open question in favor
of avoiding the wholesale-replace footgun:

- `remove_note_collection_member(collection_id, member_username)` — single-row
  `DELETE`, owner-checked via the existing `is_collection_owner` helper.
- `unshare_note_collection(collection_id)` — transactional `shared = 0` flip
  + member-list clear, also owner-checked.

Both reuse `list_note_collections` for reads, per the spec — no new listing
server fn was added. Verified end-to-end in a real browser (create a shared
collection, revoke a member, unshare the collection) via a throwaway
Playwright script against `target/debug/tidl-server`; no deviations from
the spec.
