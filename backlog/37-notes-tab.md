# Notes tab — collections, ad-hoc notes, to-do/tickler coupling

A new "Notes" tab: Google-Keep-style ad-hoc notes organized into **collections**
that are either private or shared (all members get read/write on shared
collections). Notes can optionally become **to-dos** (0-100 priority slider) or
**ticklers** (a reminder date, with priority auto-calculated from the date —
never user-set). To-dos are coupled to a CalDAV-style calendar, defaulting to
the user's default calendar but overridable per note.

This item covers the Notes tab itself (collections + notes CRUD, search, pin,
note editor) and the underlying data model. Surfacing to-do/tickler notes in
the existing Todos/Focus/Tickler views is a separate follow-up item —
[38 - Todos/Focus/Tickler note integration](38-notes-integration.md).

Design reference: handoff package (`Notes Tab.dc.html`, `NoteCard.dc.html`,
README) — high-fidelity HTML/JS prototypes, not production code. Recreate the
layout/interaction/tokens in tidl's existing Leptos + CSS stack; do not port
the HTML verbatim. See "Design decisions" below for how the prototype's ideas
map onto tidl's existing architecture.

## Dependencies

- [2 - User management](02-user-management.md) — shared-collection membership
  is validated against real user accounts, not free text.
- [3 - Multi-calendar support](03-multi-calendar.md) — to-do notes resolve to
  a real calendar (default or override).

## Design decisions (from brainstorming)

- **Storage model — mirror into `events`, not a parallel pipeline.** This
  codebase already has a precedent for "alternate origins of todos": markdown
  sync (`crates/tidl-server/src/markdown_sync.rs`) parses checkboxes and
  upserts them into the `events` table as `VTODO` rows with `source =
  'markdown'`, which is what lets Todos/Tickler/Focus treat them identically
  to native CalDAV todos with zero extra sorting/bucketing logic. Notes follow
  the same pattern: the `notes` table is the source of truth for
  text/collection/pin, and whenever a note has `is_todo` or `tickler_date`
  set, the note-save path upserts a companion `events` row with `source =
  'note'`, storing the resulting `uid` back onto `notes.mirrored_event_uid`.
  Turning both flags off (or deleting the note) deletes the companion row.
  This means #38 needs almost no new queries in Todos/Tickler/Focus — just
  rendering (a collection pill instead of a filename tag) and a tickler query
  filter change.
- **Priority scale — map tier to a representative CalDAV value on write; no
  shared-code changes.** The note editor's slider is 0-100 (thresholds
  0-33/34-66/67-100); existing CalDAV todos use RFC 5545's 1-9 scale
  collapsed into the same three tiers by `display_priority()` in
  `crates/tidl-web/src/components/todo_list.rs`. Both call sites that consume
  that tier — Todos-view sorting (`priority_sort_key(display_priority(...))`)
  and Focus urgency (`todo_urgency()`) — operate purely on the derived tier,
  never on the raw magnitude, and `NoteCard`'s priority badge reads
  `notes.priority` directly (never through the mirrored `events` row). So
  storing the note's raw 0-100 value in the mirrored `events.priority` would
  buy nothing visible while silently inverting sort order relative to
  CalDAV's scale (low integer = high priority) wherever magnitude is read
  directly (`ORDER BY priority ASC` in `get_todos()`/`get_tickler_items()`).
  Instead, the mirror write maps the note's own tier to a representative
  CalDAV value — High → 2, Medium → 5, Low → 8 — so it lands in the correct
  existing bucket in `display_priority()` with **zero changes** to
  `display_priority()`, `priority_sort_key()`, or `todo_urgency()`. The exact
  0-100 number remains the source of truth in `notes.priority` and is always
  what the Notes tab displays.
- **Collection sharing — real usernames, no invitation flow.** The prototype
  uses free-text member names ("no real invite/lookup flow", explicitly
  flagged as a production gap). tidl already has a real sharing model
  (`calendar_shares` / `share_invitations`, #13) but its accept/decline
  invitation flow is more than this feature needs — the design's spec is
  "everyone below can read & write" with no pending state. "Add person"
  validates against existing user accounts (like `calendar_shares
  .grantee_username`) and grants immediate read/write; there is no separate
  invitation/accept step.
- **Collection visibility — per-viewer, not global.** The prototype's
  `Collection.active` is a single flag in its flat client-side state. tidl's
  existing show/hide toggles (`HiddenCalsCtx`, `HiddenMdSourcesCtx` in
  `filter_sidebar.rs`) are per-viewer client-side hidden-sets so that one
  user hiding a calendar doesn't affect other users sharing it. Collections
  follow the same pattern — no `active` column in the schema; visibility is
  a client-side `HiddenNoteCollectionsCtx` (see #38).

## Data model

```sql
CREATE TABLE note_collections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_username TEXT NOT NULL,
    name TEXT NOT NULL,
    color TEXT NOT NULL,
    shared BOOLEAN NOT NULL DEFAULT FALSE,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(owner_username, name)
);

CREATE TABLE note_collection_members (
    collection_id INTEGER NOT NULL REFERENCES note_collections(id) ON DELETE CASCADE,
    member_username TEXT NOT NULL REFERENCES users(username),
    PRIMARY KEY (collection_id, member_username)
);

CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES note_collections(id) ON DELETE CASCADE,
    author_username TEXT NOT NULL,
    text TEXT NOT NULL,
    pinned BOOLEAN NOT NULL DEFAULT FALSE,
    is_todo BOOLEAN NOT NULL DEFAULT FALSE,
    priority INTEGER,           -- 0-100; only meaningful if is_todo && tickler_date IS NULL
    calendar_name TEXT,         -- FK-ish to calendars(username, name); only set if is_todo
    tickler_date DATE,
    mirrored_event_uid TEXT,    -- uid of the companion `events` row, when is_todo or tickler_date set
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

A note can have `is_todo && tickler_date` set simultaneously (priority is then
silently auto-computed elsewhere, e.g. by Focus's ranking, and never shown or
editable on the card) — this is data-model-legal per the design's state shape,
even though the note editor UI hides the priority slider whenever the tickler
toggle is on.

## Functional Requirements

**Collections**
- Sidebar card: "NOTE COLLECTIONS" header + "+" button, helper text, vertical
  list of collection bars.
- Collection bar: colored dot + name (ellipsis-truncated), hover-reveal (real
  CSS `:hover`, not the prototype's 0.35-base-opacity compromise) pencil edit
  icon. Clicking the bar (not the pencil) toggles per-viewer visibility.
  Clicking the pencil (with `stopPropagation`) opens the collection editor.
- Collection editor modal: name input, 6 curated color swatches (`#e35d7a
  #3b82f6 #22c55e #f59e0b #a78bfa #06b6d4`), "Shared" checkbox revealing
  member chips + "Add person by name" (validated against real accounts) +
  Add button. Footer: Delete (red, edit mode only, with a confirm dialog —
  cascades to all of that collection's notes) — Cancel / Save.
- Seed data: "Private" = `#e35d7a` (not shared), "Alice & Bob" = `#3b82f6`
  (shared, members `["alice", "bob"]`) — for local dev/demo only, not a
  migration-level default for all users.

**Notes**
- Main area: "★ Pinned" filter toggle + search input (case-insensitive
  substring match on note text) + result count + 3-column grid of a dashed
  "+ Take a note..." tile followed by note cards. Pinned notes sort first
  within currently-visible collections.
- Note card: text + pin star (top-right, gold when pinned, `stopPropagation`
  so it never opens the editor). Optional badge row (only rendered if there's
  something to show): priority badge (only for to-dos without a tickler date)
  and/or tickler badge (`🕑 yyyy-mm-dd`). Bottom row always present, always in
  the same position: collection dot+name bottom-left, relative time
  bottom-right — badges must never push this row around. No author
  avatar/name on the card face. Clicking the card body (not the star) opens
  the note editor in edit mode.
- Note editor modal (shared between "+ Take a note..." add and click-to-edit,
  `mode: Add | Edit`): textarea (3 rows, no resize), single-select collection
  chip row, "Make this a to-do" checkbox revealing a priority slider
  (0-100, color-tinted per tier) and a calendar chip row (defaulting to the
  user's default calendar), "Set a reminder date" checkbox revealing a date
  input (defaulting to today+7) with the auto-priority caption. The priority
  slider is hidden whenever the tickler checkbox is on. Footer: Delete (red,
  edit mode only) — Cancel / Save. No author field.
- Empty state: "No notes yet" message in the grid when collections are
  toggled off or search yields zero results (not designed in the prototype —
  new requirement).

**Access control**
- A note inherits its collection's access: the owner, or any member of a
  shared collection, may read/write. Enforced in the `#[server]` fn bodies.

## Non-goals (this item)

- Todos/Focus/Tickler sidebar and list integration — see #38.
- CalDAV visibility of notes themselves (only the mirrored to-do/tickler VTODO
  is CalDAV-visible, via the existing events pipeline — the note text/pin/
  collection metadata is not exposed over CalDAV).
- Theme-preference persistence beyond what already exists (the app's existing
  dark/light toggle is reused as-is; no new persistence work here).

## Open Questions

- None outstanding — architecture decisions above were resolved during
  brainstorming (2026-07-10).

## Resolution

Implemented as designed. Notes/collections/members live in three new tables;
to-do/tickler-flagged notes mirror into `events` via delete-then-recreate
(not in-place update) to correctly handle calendar reassignment between
saves. Priority is mapped to a representative CalDAV tier value (2/5/8) on
write rather than stored raw, per the sort-order fix found during the
brainstorming self-review (see the "Priority scale" design decision above).
