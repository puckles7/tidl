# Sidebar share icon for calendars & note collections → Settings/Sharing

Today the only sidebar hint that a calendar is shared is a clunky `(from <owner>)`
suffix appended to a shared-with-me pill's name — and the owner side gets
nothing at all: a calendar's own owner has no way to tell, from the sidebar,
that it's shared with anyone. Note-collection sidebars show neither. Add a
hover-reveal share icon to every calendar and note-collection pill (mirroring
the existing edit icon) that opens the Settings modal directly on the Sharing
tab, for both the owner and the grantee. Drop the `(from <owner>)` text now
that the icon (with a descriptive tooltip) carries that information.

## Dependencies

- [13 - Calendar sharing](13-calendar-sharing.md) — this extends the calendar sharing data model
- [41 - Make calendar shares actually usable (web UI + raw CalDAV)](41-caldav-share-exposure.md) — introduced `calendar_list_panel.rs` and the `(from <owner>)` label being replaced
- [44 - Settings → Sharing: manage note-collection sharing](44-settings-sharing-note-collections.md) — the Sharing tab this icon links to

## Context / Background

- `CalendarEntry`/`CalendarInfo` currently have no `shared` flag — an owned
  calendar's sidebar entry can't tell whether it has any grantees at all
  (that's only computed inside the Settings → Sharing panel via a separate
  per-calendar `get_calendar_shares` call). `NoteCollectionInfo` already
  carries `shared`/`is_owner`/`members`/`owner_username` on every row via
  `list_note_collections()`, so no backend change is needed for notes.
- `SettingsModal` (`components/settings/mod.rs`) always opens on its
  `"general"` tab; there's no way today for another component to open it
  pre-selected to a specific tab like `"sharing"`.
- `CalendarPill` (`components/calendar_pill.rs`) already has the hover-reveal
  button pattern (`on_edit`, `on_sync`) this reuses.

## Functional Requirements

- **Backend**: add `shared: bool` to `CalendarInfo`, computed in
  `get_calendars()` (`api/calendars.rs`) via a correlated
  `EXISTS(SELECT 1 FROM calendar_shares cs WHERE cs.owner_username =
  calendars.username AND cs.calendar_name = calendars.name)`. Add `shared:
  bool` to `CalendarEntry` (`types.rs`), populated from `CalendarInfo.shared`
  for owned entries in `merge_calendar_entries` (shared-with-me entries don't
  need it — `shared_permission: Some(...)` already signals their state).
- **Open-to-tab plumbing**: replace `app.rs`'s local `show_settings:
  RwSignal<bool>` with a context newtype `OpenSettingsCtx(pub
  RwSignal<Option<&'static str>>)` — `None` closed, `Some(tab)` open on that
  tab. `SettingsModal` gains `#[prop(default = "general")] initial_tab:
  &'static str`, seeding `active_tab`'s initial value.
- **`CalendarPill`**: new optional props `on_share: Option<Callback<()>>` /
  `share_title: String`, rendered as a fourth always-present, hover-revealed
  button (glyph `⇄`, class `cal-pill-share-btn`) after the edit button —
  matches the existing DOM-always-present-but-invisible pattern so pill
  height never shifts when some entries lack a button.
- **`calendar_list_panel.rs`**: drop the `(from {owner})` suffix — pill name
  is always just `display_name`. The share icon only renders on entries that
  are actually shared: any shared-with-me entry, or an owned entry with
  `shared == true`. `on_share` wires to `OpenSettingsCtx.set(Some("sharing"))`.
  `share_title`:
  - shared-with-me entry → `"Shared by {owner_username}"`
  - owned + `shared == true` → `"Manage sharing"`
- **`notes/mod.rs` + `filter_sidebar.rs`** (both note-collection sidebar
  render sites): same rule and `on_share`/`share_title` wiring using
  `NoteCollectionInfo`'s existing `is_owner`/`shared`/`owner_username`.
- **CSS**: extend the hover-reveal rule block in `assets/style.css`
  (`.cal-pill-edit-btn, .cal-pill-sync-btn { opacity: 0; ... }` and the
  `:hover` companions around line 2711) to include `.cal-pill-share-btn`,
  with its own distinct hover accent color.
- Icon always renders identically wherever it does appear (no
  filled/outline distinction) — only the tooltip text and destination content
  (which section of the Sharing tab is populated) vary. It does *not* appear
  on an owned, not-yet-shared calendar/collection — no shortcut to start
  sharing from the sidebar; that stays behind Edit → Share.
- The existing owner-only "Share" button inside `CalendarEditModal` /
  `CollectionEditorModal` (behind the pencil icon, disabled for grantees) is
  untouched — this new icon is an additional, more discoverable entry point
  for both directions, not a replacement for that flow.

## Open Questions

- None outstanding — icon glyph (`⇄`) and tooltip copy were confirmed during
  brainstorming. Scope (only actually-shared entries, not every pill) was
  corrected post-implementation after a mismatch between the approved design
  wording and actual intent — see Resolution.

## Resolution

Built per spec, with one correction after initial implementation:

- `CalendarInfo`/`CalendarEntry` gained `shared: bool`, populated in
  `get_calendars()` via a correlated `EXISTS` subquery against
  `calendar_shares`; threaded through `merge_calendar_entries` and
  `to_calendar_info()`.
- `app.rs`'s local `show_settings: RwSignal<bool>` was replaced with
  `OpenSettingsCtx(pub RwSignal<Option<&'static str>>)`, provided once in
  `AuthenticatedApp`. `SettingsModal` gained an `initial_tab` prop (default
  `"general"`) seeding `active_tab`.
- `CalendarPill` gained `on_share`/`share_title` props and a fourth
  hover-reveal button (`⇄`, `.cal-pill-share-btn`), styled to match the
  existing edit/sync buttons with its own violet hover accent.
- `calendar_list_panel.rs` dropped the `(from {owner})` label suffix;
  `notes/mod.rs` and `filter_sidebar.rs`'s note-collection sidebars wired the
  same `on_share`/`share_title` using `NoteCollectionInfo`'s existing
  `is_owner`/`shared`/`owner_username` fields (no backend change needed
  there).
- Added `merge_owned_carries_shared_flag` and
  `merge_shared_with_me_entry_has_shared_false` unit tests in `types.rs`.
- **Correction**: the initial pass rendered the share button on every pill
  unconditionally (like edit/sync). The intent, confirmed after the first
  round shipped, was for it to appear only on entries that are actually
  shared — a shared-with-me entry always qualifies; an owned entry only if
  `shared == true`. All three wiring sites now gate `on_share` on
  `!is_owned || shared` instead of always constructing `Some(...)`. The
  `"Share"` (not-yet-shared) tooltip case was removed since that branch no
  longer renders.
