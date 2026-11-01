# TASK/AWAIT kind unification

Introduce an explicit, first-class "kind" discriminator — **TASK** (self-actionable,
priority and/or due date) vs. **AWAIT** (date-gated, waiting on something external,
never prioritized) — replacing the implicit and leaky proxies the app currently uses
(NULL-priority-as-sentinel, `source IN ('markdown','note')` heuristics) to guess
which bucket an item belongs in. Surfaces as renamed tabs ("Tasks", "Await"), a
mutex create-choice in the note editor, and renamed markdown tags.

## Dependencies

- [37 - Notes tab](37-notes-tab.md) — owns the `notes.is_todo`/`notes.tickler_date`
  columns and the `sync_mirror` VTODO-mirroring mechanism this extends.
- [38 - Todos/Focus/Tickler note integration](38-notes-integration.md) — owns the
  `LEFT JOIN notes ON mirrored_event_uid` pattern and the `source IN
  ('markdown','note')` heuristic this replaces with a real `kind` column.
- [18 - Markdown source files](18-markdown-sources.md) — owns `parser.rs`'s
  `#todo`/`#pending` tag regexes, renamed here to `#task`/`#await`.
- [19 - Tasks view](19-tasks-view.md), [20 - Tickler view](20-tickler-view.md),
  [21 - Focus view](21-focus-view.md) — the three views whose queries/labels change.
- [40 - Note to-do completion write-back](40-note-todo-completion-writeback.md) —
  `complete_todo_item`'s `source == "note"` branch must keep working unmodified.

## Context / Background

Investigated after noticing a tickler-flagged note showed up on the Todos page.
Root cause: `get_todos()` (`api/events.rs`) selects `WHERE e.type = 'VTODO'` with no
further discriminator — "todo" is currently defined as "any VTODO row," regardless
of *why* that row exists. A markdown `#pending/<date>/<slug>` item is inserted by
`markdown_sync.rs` with no `priority` column value; `get_todos()` defaults that NULL
to a `1000` sentinel, which `display_priority()` buckets as "Low" — so `#pending`
items leak into Todos today too, not just note-tickler items. Symmetrically,
`get_tickler_items()` approximates "is this an await-kind item" via `source IN
('markdown', 'note')`, which only coincidentally works today and breaks down as
soon as a note can independently be both `is_todo` **and** have a `tickler_date` set
(the editor UI already allows checking both), or once markdown gains a dated-TASK
tag that is markdown-origin but must *not* appear in Await.

Considered modeling AWAIT as a `VEVENT` instead of a `VTODO` (a "waiting for X"
item does behave more like a date-triggered marker than an open-ended prioritized
task) — rejected because `VEVENT`'s `STATUS` (TENTATIVE/CONFIRMED/CANCELLED) has no
NEEDS-ACTION/COMPLETED lifecycle, and Focus/Tasks/Await's ranking and completion
machinery is built entirely around `VTODO` semantics. Splitting types would
duplicate that pipeline rather than reuse it. AWAIT stays a `VTODO`; what's missing
is a real discriminator column, not a different iCal type.

## Terminology (settled)

| Term | Meaning | Has priority? | Has date? |
|---|---|---|---|
| **TASK** | Self-actionable; you do this | Yes, optional | Optional |
| **AWAIT** | Dormant until a date; you're waiting on something/someone external | No (derived from date for sort order only) | Always |

- Tab labels: **"Tasks"** (was "Todos") and **"Await"** (was "Tickler") — singular,
  matching the existing "Focus"/"Calendar" tab-naming precedent rather than
  pluralizing ("Awaits" reads as a verb conjugation, was rejected).
- Note editor: the current two independent checkboxes ("Make this a to-do" / "Set a
  reminder date") become a single **mutex** choice — create a TASK (with optional
  priority slider and/or due date) *or* create an AWAIT (date only) — eliminating
  the both-checked ambiguity at the source instead of resolving it after the fact.
- Markdown tags: `#todo/p<N>/<slug>` → `#task/p<N>/<slug>`; new `#task/<date>/<slug>`
  (dated TASK, priority auto-derived, same mechanism AWAIT already uses); `#pending/
  <date>/<slug>` → `#await/<date>/<slug>`.

## Functional Requirements

**Schema** (new migration)
- `events.kind TEXT NOT NULL DEFAULT 'task' CHECK (kind IN ('task', 'await'))`.
- Backfill existing rows: `source = 'markdown'` rows with `due_date IS NOT NULL AND
  priority IS NULL` → `'await'` (matches the two tags being mutually exclusive
  today), else `'task'`. `source = 'note'` rows → derive from `notes.is_todo` via
  the existing `mirrored_event_uid` join (`is_todo = 1` → `'task'`, else `'await'`).
  `source = 'caldav'` (or absent) → always `'task'` (see Non-goals — no CalDAV path
  creates an AWAIT).
- `notes.kind TEXT CHECK (kind IN ('task', 'await'))` — **nullable**, unlike
  `events.kind`. `NULL` = plain, non-actionable note (no mirror); this is the
  common case and must stay representable, so `notes.kind` is a 3-way switch
  (`NULL` / `'task'` / `'await'`), not a 2-way one. `needs_mirror = notes.kind.
  is_some()`, replacing the old `is_todo || tickler_date.is_some()` OR. Backfill:
  `is_todo = 1` → `'task'`, else `tickler_date IS NOT NULL` → `'await'`, else
  `NULL`.
- Rename `notes.tickler_date` → `notes.due_date` (same migration). The note
  editor's mutex TASK/AWAIT choice shares one date field — required for AWAIT,
  optional for TASK — so one column serves both instead of AWAIT keeping the
  old tickler-specific name.
- Drop `notes.is_todo` (folded into `kind`).

**Markdown parser** (`tidl-markdown/src/parser.rs`)
- Rename `TODO_RE`'s tag prefix to `#task/p<N>/<slug>`; add a new branch/regex for
  `#task/<date>/<slug>` (dated TASK, no explicit priority in the tag).
- Rename `PENDING_RE` → `#await/<date>/<slug>`; rename `PendingItem` → `AwaitItem`
  (`parser.rs`, `watcher.rs`).

**`markdown_sync.rs`**
- Set `events.kind` explicitly on insert for both tag families (no more inferring
  it later from column nullness).
- New dated-TASK insert path: identical shape to the existing `#await`/pending
  insert (`due_date` set, `priority` left `NULL`, `kind = 'task'`) — **no new
  date→priority derivation needed**. Corrects an earlier assumption in this spec:
  AWAIT items don't actually use a stored priority for sorting either — Tickler
  sorts by `due_date` directly and Focus scores await-kind items by
  `pending_urgency(due_date)`, not by any priority column. A `NULL`-priority TASK
  already sorts last among tasks via `get_tasks()`'s existing `ORDER BY priority
  ASC NULLS LAST` and defaults to the "Low" urgency tier via the same
  `unwrap_or(1000)` sentinel `get_tasks()` already applies — both mechanisms
  pre-exist and need no changes.

**Notes API** (`api/notes.rs`)
- `sync_mirror` sets `events.kind` directly from the note's own (now single-source)
  `notes.kind` value — no more OR-ing two fields together.
- `NewNote`/note editor UI: replace the two independent checkboxes with a single
  mutex TASK/AWAIT control bound to `kind`. Selecting TASK reveals the priority
  slider and optional due-date field; selecting AWAIT reveals only the date field.

**Query changes** (`api/events.rs`)
- `get_todos()` → rename to `get_tasks()`, add `AND e.kind = 'task'`.
- `get_tickler_items()` → rename to `get_awaits()`, replace `source IN ('markdown',
  'note')` with `AND e.kind = 'await'`.
- `Todo` (`types.rs`) → rename to `Task`, sweeping `todo_list.rs`, `focus.rs`,
  `api/events.rs`. Fully compiler-checked, no external API surface, so this is a
  mechanical rename — leaving it `Todo` would be the one place the retired
  terminology survives.

**Focus view (`focus.rs`) — larger than originally scoped.** Focus does **not**
call `get_tickler_items()` today. It fetches only `get_todos()` and reclassifies
locally via `is_pending_todo(t) = t.source == "markdown" && t.due.is_some()` —
which is *already* wrong for note-origin AWAIT items (a note-AWAIT has
`source == "note"`, so it silently falls through to the `ItemKind::Todo` branch
today and gets scored by `todo_urgency` — priority-based — instead of
`pending_urgency` — date-based. Live bug, predates this item, fixed as a side
effect). Once `get_tasks()` filters to `kind = 'task'` only, Focus must be
updated together with the rename, not left calling the old feed:
- Add a `get_awaits()` `LocalResource` fetch alongside the existing `todos_res`.
- Replace `ItemKind::Todo(Todo)` / `ItemKind::Pending(Todo)` with
  `ItemKind::Task(Task)` / `ItemKind::Await(AwaitItem)` — two distinct payload
  types, not one type wearing two hats.
- Delete `is_pending_todo` entirely — kind is now authoritative from the query,
  no local reclassification needed.
- Every `match` on `ItemKind` (`item_is_done`, `item_summary`,
  `todo_stripe_color`/`stripe_color_for`, `item_pill_style`, `item_meta`,
  `include_item`) gains an `Await(AwaitItem)` arm reading `AwaitItem`'s own
  fields (`description`, `topic`, `source_file`, `collection_*`) instead of
  reusing `Task`'s.

**Frontend**
- `todo_list.rs` / `tickler.rs`: nav labels "Todos"/"Tickler" → "Tasks"/"Await".
  CSS needs no new work — `tickler.rs` already reuses the `tasks-*` CSS classes,
  and both already share `.card-tile` (see the card-shell unification done
  2026-07-16).
- Routes: `/todos` → `/tasks`, `/tickler` → `/await`, matching the new labels
  (single-user self-hosted app, so bookmark breakage is low-stakes — leaving stale
  URLs would just reproduce the tab-label/CSS-class drift that started this).
- `types.rs`: `TicklerItem` → `AwaitItem`.

**Markdown migration** (one-time, not a permanent code path)
- Existing `.md` files using `#todo/p<N>/...` and `#pending/<date>/...` need a
  one-time find/replace to the new tag prefixes once this ships (documented `sed`
  command, not a dual-parsing shim kept in `parser.rs` indefinitely).

## Non-goals

- No change to Focus's urgency-scoring algorithm.
- No change to the CalDAV wire format — `kind` is an internal column, not
  projected into `ical_data`/`CATEGORIES`.
- No change to [40](40-note-todo-completion-writeback.md)'s completion
  write-back behavior (clear `kind`/`due_date`/`mirrored_event_uid`, delete the
  mirror row) — only the fields being cleared change name, not the mechanism.
- No dual-support for the old `#todo`/`#pending` markdown tags — hard rename, see
  Resolutions below.
- No CalDAV-origin AWAIT — `source = 'caldav'` rows (including the app's own
  "+ Add task" button) always get `kind = 'task'`; there is no UI/API path to
  create an AWAIT except the note editor and markdown tags.

## Open Questions

All resolved — see Resolutions.

### Resolutions

- **Markdown tag backward compatibility** → originally decided as a **hard
  rename** (one-time `sed` migration of existing files, no dual parsing).
  **Reversed 2026-07-16**: the sed migration was never run, and the sync loop
  turned out to cache files by content hash (`sync_state.file_hash`) — so
  unchanged files synced under the old regex kept their stale DB rows
  forever, silently reintroducing exactly the dual-format problem the hard
  rename was meant to avoid. `#todo`/`#pending` are now permanent parser
  aliases for `#task`/`#await` (`tidl-markdown/src/parser.rs`); existing DB
  rows are left as-is, no forced resync.
- **Route URLs** → **rename** to `/tasks` and `/await`. Leaving them stable while
  every other surface (tab label, markdown tag, type names) renames would
  reproduce the exact drift (nav says one thing, internals say another) that
  motivated this item in the first place.
- **`Todo` struct rename** → **rename to `Task`**, full sweep. Workspace-internal
  type, fully compiler-checked reference set, no external API surface — cheap and
  mechanical. Leaving it `Todo` would be the one surviving spot of retired
  terminology.
- **`notes` table** → **add `notes.kind`**, retiring the bare `is_todo` boolean.
  Two independent optional fields (`is_todo` + `tickler_date`) leave the invalid
  "both meaningfully set" state representable even once the UI stops offering it
  — a stray API call or future code path could still produce it. A `kind` column
  with a `CHECK` constraint makes that state unrepresentable at the schema level
  (type-driven design: make invalid states unrepresentable), and gives the mutex
  editor UI one value to bind instead of reconciling two booleans.
- **CalDAV-origin AWAIT** → **not supported**. No existing CalDAV client
  convention or app UI maps to "external, date-gated, no priority." AWAIT stays
  exclusively creatable via the note editor and markdown tags; `source = 'caldav'`
  always defaults to `kind = 'task'`. Revisit as new scope if ever needed.

## Resolution

Implemented as specced across 17 tasks (see plan
`docs/superpowers/plans/2026-07-16-task-await-unification.md` for the full
diff), with three corrections made during planning/implementation:

- `notes.kind` is **nullable** (not `NOT NULL DEFAULT 'task'` as originally
  drafted) — `NULL` represents a plain, untracked note, which had to stay a
  representable state. `needs_mirror = notes.kind.is_some()`.
- The markdown dated-TASK tag needs no date→priority derivation — AWAIT items
  were never actually scored by a stored priority (Await/Tickler sorts by
  `due_date` directly, Focus scores them via `pending_urgency(due_date)`), so
  a `NULL`-priority TASK correctly falls back to the existing sentinel-based
  "Low" tier handling with no new code.
- Focus's scope was larger than first estimated: it fetched only
  `get_todos()` and reclassified locally via `is_pending_todo()`, which
  turned out to already misclassify note-origin AWAIT items (scored by
  priority instead of date, since that heuristic only checked `source ==
  "markdown"`) — fixed as a side effect of making `kind` authoritative via a
  new dedicated `get_awaits()` fetch and `ItemKind::Await(AwaitItem)` variant.

Other implementation notes:
- `AwaitItem` gained a `source_line: Option<u32>` field (absent from the old
  `TicklerItem`) so Focus's "go to source" markdown preview keeps working for
  AWAIT items exactly as it did for the note-origin items it used to
  misclassify as `Task`.
- The note editor's mutex control is three chips ("Plain note" / "Task" /
  "Await") rather than a `<select>`, matching the existing `.note-chip`
  visual pattern already used for the collection/calendar pickers in the same
  modal. A TASK's due date is a separate on/off toggle ("Add a due date")
  since — unlike AWAIT's, which is always required — a TASK's date is
  optional.
- One pre-existing, unrelated test flake surfaced during verification:
  `tests/test_notes.py::test_notes_full_workflow` occasionally fails with a
  Playwright "navigation interrupted" error caused by an async
  `window.location().reload()` in `app.rs`'s `post_auth()` (unrelated to this
  item — the failure happens on the test's very first, unmodified `page.goto`
  call, before any of backlog #49's code runs). Left as-is since fixing
  shared test fixture timing is out of scope here; retrying reliably passes.
- Full verification: `cargo test --workspace` (202 tests), `cargo clippy
  --workspace -- -D warnings`, `cargo fmt --check` (no new diffs beyond this
  repo's pre-existing ~783-diff baseline, confirmed via a throwaway
  `git worktree` comparison against the pre-#49 commit), the
  `wasm32-unknown-unknown` build, `pytest tests/test_hydration.py` (20/20)
  and `tests/test_notes.py`, and a manual Playwright-driven visual check of
  the mutex note editor, `/tasks`, `/await`, and `/focus` confirming the
  original bug (a tickler/await-only note leaking into Tasks) is fixed.

### Follow-up fixes (2026-07-16, discovered testing the alias reversal above)

- `complete_todo_item`'s tag-stripping write-back (`events.rs`) silently
  downgraded CRLF markdown files to LF as a side effect of completing a
  single tagged line (`content.lines()` discards `\r`, then rejoining with a
  hardcoded `"\n"` rewrote every other line too). Fixed via
  `strip_tag_from_content`, which detects the file's actual line-ending
  style once and rejoins with that.
- `task_urgency` (Focus) only ever scored a TASK by its priority tier,
  ignoring `due` entirely — so a dated `#task/<date>/<slug>` item with no
  priority always sorted/labelled as "Low", even when due tomorrow. Now
  takes the max of its priority score and `pending_urgency(due, today)`,
  matching how urgent AWAIT already reads. The Tasks view's sort and its
  card's urgency label/color/bar now reuse this same scored function instead
  of a separate raw-priority-only tier (`display_priority`/`priority_label`,
  removed) — and Await's card (`TicklerCard`) gained the identical
  `fsc-urgency` label/bar cluster Focus's own card uses, via shared
  `urgency_label`/`urgency_color`/`urgency_bar_fill_px` helpers, so all
  three views now render and rank urgency identically.
