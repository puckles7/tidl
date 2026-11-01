# Contacts export (vCard/VCF)

Let a user export their contacts as a .vcf file, mirroring the existing vCard import (#87) so contacts can round-trip out of tidl into other address book apps.

## Dependencies

- [79 - Contacts data model + Contacts tab (CRUD)](79-contacts-data-model-tab.md) — export operates on the existing contact model

## Context / Background

#87 added vCard import into tidl's Contacts tab. There's no equivalent export path yet — a user who wants to back up their contacts or move them to another app has no way to get a standard vCard file out of tidl (CardDAV sync via #29 covers live client sync, but not a portable one-off file).

## Functional Requirements

- User can export a single contact, a selection of contacts, or all contacts to a `.vcf` file.
- Output uses the same vCard version/fields tidl already reads on import (#87) — round-tripping export → import should be lossless for supported fields, including photos (#86) where present.
- Exported file downloads through the browser (standard file-save flow), not just written server-side.

## Open Questions

- vCard 3.0 vs 4.0 for the export format — match whatever #87's import prefers for consistency.
- Bundle multiple contacts in one .vcf (concatenated VCARDs) vs. one file per contact — confirm with user before implementing.

## Resolution

Built as specified. Both Open Questions were resolved before implementation: vCard 3.0 (already what `tidl_carddav::vcard::generate_vcard`, built for #29/#86 and reused directly here, emits — same format #87's import round-trips), and the bundling question was put to the user directly, who chose one concatenated `.vcf` (multiple `VCARD`s) over one-file-per-contact, matching how Apple Contacts/Thunderbird/Google export a multi-contact selection and avoiding a zip/multi-download dance in the browser.

- **`crates/tidl-server/src/export.rs`**: new `GET /export/contacts?ids=1,2,3` handler (`export_contacts_vcf`), a plain Axum route mirroring the existing `export_calendar_ics`'s session-cookie auth shape exactly — not a `#[server]` fn, since the download needs to be a normal browser navigation (`<a href download>`), same reasoning as the calendar export it sits next to. `ids` is an optional comma-separated query param; omitted or empty exports every owned contact. A `select_contacts_for_export` helper does the filtering (unmatched/malformed ids silently dropped rather than failing the whole export — e.g. a contact deleted between page load and click). One request handles all three of the spec's cases ("single contact, a selection, or all") since a single id and the full list are just different `ids` values on the same route.
- **Reuse across the compilation boundary**: `record_to_vcard` (private `fn` in `crates/tidl-server/src/carddav_handlers.rs`, already used by the CardDAV GET/REPORT handlers to build a vCard from a `ContactRecord`) and its `VCF_CONTENT_TYPE` constant were widened to `pub(crate)` so `export.rs` could call them directly rather than re-deriving a second `ContactRecord → VCardFields` mapping — the two now share one implementation, keeping CardDAV GET and file export byte-for-byte consistent (including the photo round-trip).
- **Route registration**: `.route("/export/contacts", get(export::export_contacts_vcf))` added in `main.rs` alongside the existing `/export/{slug}` calendar route — axum's router resolves the static `/export/contacts` segment ahead of the `{slug}` parameter regardless of registration order, confirmed by the server starting and routing correctly in manual verification below.
- **UI, final shape** (`components/contacts/mod.rs`, `components/settings/export.rs`): "export all" lives in Settings → Export, as a new "Contacts (.vcf)" section alongside the existing calendar `.ics` export (`ContactsExportSection`, no picker needed — one address book per user) — moved there from an initial Contacts-toolbar "⬇ Export all" button, which was removed after the user tried the feature and asked for it to live in Settings instead, matching where contacts *import* already lives (#87). "A single contact" / "a selection" stayed on the Contacts toolbar: reuses the existing merge-mode checkbox selection (backlog #88) rather than a second, parallel selection UI — an "⬇ Export (N)" link appears whenever merge-mode is active and at least one contact is checked (the existing "Merge (N)" action only appears at N≥2), pointing at `/export/contacts?ids=<selected ids>`. Selecting exactly one contact and clicking Export covers the "single contact" case with no separate per-card action needed.
- **Testing**: 5 new unit tests for `select_contacts_for_export` in `export.rs` (all-when-`None`, all-when-empty-string, filters to named ids, a single id, ignores unmatched/malformed ids) plus the pre-existing `record_to_vcard`/round-trip tests in `carddav_handlers.rs`, which already cover the vCard mapping this reuses. Full `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, and `cargo build --target wasm32-unknown-unknown -p tidl-web` all pass/build clean.
- **Manual verification**: built a `gts` ("generate test server") dev alias (`flake.nix` + `tidl-server seed-demo`, see its own doc comments) specifically to get real browser-testable data for this and future features — a disposable throwaway server, fresh port and data dir each run, seeded with contacts/events/notes. The user exercised both calendar and contacts export against it directly and confirmed contacts export works; that same manual pass also surfaced an unrelated **pre-existing bug**, not introduced by this item: `storage::list_events`/`list_todos` (used by calendar `.ics` export and CalDAV listing) decode `events.due_date` as `chrono::DateTime<Utc>`, but `notes::sync_mirror`'s real production INSERT (backlog #38, predates this item) stores a bare `YYYY-MM-DD` for AWAIT/TASK-with-due-date notes — decoding that as a datetime fails with "invalid datetime", 500ing calendar export for any calendar containing such a mirrored VTODO. Nobody had previously exported a calendar containing a note-mirrored due-dated to-do, so this had never surfaced. Fixed by appending a midnight time before that specific write (`tidl_web::api::notes::sync_mirror`); the `DUE` line inside the stored `ical_data` blob (built separately, before that write) is unaffected. Covered by a new regression test (`sync_mirror_await_due_date_decodes_as_datetime` in `notes.rs`) verified red→green against the actual bug, plus a `list_events` sanity check in `seed_demo`'s own test.
