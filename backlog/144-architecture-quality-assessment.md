# Architecture quality assessment (2026-10-14 full-project pass)

A full-codebase architecture review across all five crates (~90 kLOC: ~67 kLOC
production, ~23 kLOC test), checking for code smells, redundant
implementations, complexity, dependency-management problems, god objects, and
architecture anti-patterns. This is a **tracking/assessment item** in the
spirit of [141 - Code review findings](141-code-review-findings-2026-10-11.md):
the deliverable is the assessment itself, and the actionable findings below are
candidates to be split into their own backlog items.

Method: static inspection (`rg`/`cargo tree`/line-counting) plus the project's
own architecture contract in `CLAUDE.md` §6–§16 as the rubric. No behaviour is
changed by this item.

Crate sizes and shape:

| Crate          |   LOC | Notes                                                        |
|----------------|------:|--------------------------------------------------------------|
| `tidl-web`     | 61845 | 85 files · 175 `#[server]` fns · ~15 kLOC is server SQL/auth |
| `tidl-server`  | 21426 | 16 files · `storage.rs` alone is 5746 lines                  |
| `tidl-markdown`|  2733 | parser + dead source/watcher modules                         |
| `tidl-caldav`  |  2268 | protocol library                                             |
| `tidl-carddav` |  1538 | protocol library                                             |

## Executive summary

The day-to-day feature code is healthy — the forbidden-pattern discipline that
[141](141-code-review-findings-2026-10-11.md) praised still largely holds
(only 48 production `unwrap`/`expect` in the whole workspace, no `unsafe`
outside tests, no leaking `sqlx` types into library structs). The problems this
pass found are almost all **structural** rather than bug-level, and they cluster
into four themes:

1. **A god object plus god components.** `Storage` (68 inherent + 60 trait
   methods, 5746-line file) and the largest Leptos components (`NotesView` 1633
   lines, `ContactsView` 1468, `NoteEditorModal` 1208, `WeekGrid` 1097) carry
   too much responsibility for one unit.
2. **A split data-access layer with forced duplication.** `tidl-server` has
   `Storage`; `tidl-web/src/api/*` cannot depend on it and therefore re-issues
   its own SQL (365 production runtime queries) and re-implements
   `Storage::bump_*_sync_token` by hand. There is no shared domain/storage
   crate. This is the single most impactful architectural issue.
3. **A duplicated markdown/note-token grammar.** `tidl-markdown` is not
   wasm-safe (it bundles a dead watcher), so `tidl-web` forks the same token
   scanner and renderer into `note_tokens.rs`, `markdown_tokens.rs`, and
   `util::render_note_content` — three token implementations with divergence
   risk.
4. **Dependency hygiene drift.** Three crates declare an `anyhow` dependency
   they no longer use, several crate versions are duplicated (three `rand`
   major lines), several deps bypass `workspace.dependencies`, and `quick-xml`
   0.31 is still pinned in two crates (tracked as #142).

None of these is a correctness emergency; all are real maintenance/velocity
costs. Recommended follow-ups are listed at the end.

## 1. God objects & module size

### 1.1 `Storage` — the god object
`crates/tidl-server/src/storage.rs` (5746 lines) defines ~128 operations on one
type: `impl Storage { ... }` spans **lines 658–2788 (2130 lines / 68 methods)**
plus `CalendarStore for Storage` (47 methods), `ContactStore for Storage` (12),
and `NoteStore for Storage` (1). Every storage concern in the whole application
funnels through this one struct. It is a textbook god object: high file size,
high method count, and no cohesive sub-boundaries.

- Severity: **high** (maintainability).
- Recommendation: split along the existing domain seams into
  `storage/{calendar,contact,note,session,share}.rs` (or per-entity sub-structs
  behind `Storage`), and give `CalendarStore` (47 methods) a narrower, cohesive
  interface.

### 1.2 God components
Largest single functions in the workspace (brace-accurate measurement, string
literals excluded):

| Lines | Function | Location |
|------:|----------|----------|
| 1633 | `NotesView` | `crates/tidl-web/src/components/notes/mod.rs:79` |
| 1468 | `ContactsView` | `crates/tidl-web/src/components/contacts/mod.rs:94` |
| 1208 | `NoteEditorModal` | `crates/tidl-web/src/components/notes/note_editor_modal.rs:281` |
| 1097 | `WeekGrid` | `crates/tidl-web/src/components/calendar/week_grid.rs:75` |
|  702 | `main` | `crates/tidl-server/src/main.rs:151` |
|  630 | `RemoteSourcesPanel` | `crates/tidl-web/src/components/settings/remote_sources.rs:12` |
|  551 | `MarkdownPreviewInner` | `crates/tidl-web/src/components/focus.rs:2133` |
|  549 | `SharingPanel` | `crates/tidl-web/src/components/settings/sharing.rs:16` |
|  536 | `ContactEditorModal` | `crates/tidl-web/src/components/contacts/contact_editor_modal.rs:28` |
|  526 | `CalendarEditModal` | `crates/tidl-web/src/components/cal_edit_modal.rs:77` |

These repeat the pre-#27 pattern (H1/H2 split `CalendarView`/`SettingsModal`)
that #27 already fixed once: a `#[component]` fns grows into a 4-figure line
count. `main` (702 lines) is a god startup function mixing CLI dispatch, config
loading, router construction, and background-task spawning.

- Severity: **high** for `NotesView`/`ContactsView`/`WeekGrid`, **medium** for
  the 500–700-line modal components and `main`.
- Recommendation: extract sub-components along the same seams #27 used
  (`NotesView` toolbar/grid/filter/handlers; `ContactsView` likewise);
  decompose `main` into `build_router`/`spawn_background_tasks`/`run_cli`.

### 1.3 Fat API modules
`api/contacts.rs` (4791 lines, 33 `#[server]` fns), `api/events.rs` (3942, 21),
`api/notes.rs` (3308, 24), `api/calendars.rs` (2546, 30) each mix shared DTO
re-exports, server-only SQL, auth boilerplate, and embedded tests. They are god
modules by size even though the per-function responsibility is narrow.

- Severity: **medium**. The `#[server]` bodies scale with the feature, so some
  growth is inherent; the fix is mostly mechanical extraction of the SQL-heavy
  helpers.

## 2. Redundant implementations

### 2.1 The markdown/note-token grammar exists three times
The inline note/markdown token grammar (`#task/pN/slug`, `#await/date/slug`,
`#done`, `@mention`) is implemented independently in:

- `crates/tidl-markdown/src/parser.rs` (server; `MarkdownParser::parse_all`,
  `scan_done_tags`, `mark_tag_done(_at)`, `reopen_tag(_at)`, `edit_tag_at`,
  `remove_tag_at`, `slug_to_title`) — used by `tidl-server/markdown_sync.rs`
  and by `tidl-web/api/*` on the SSR side.
- `crates/tidl-web/src/note_tokens.rs` (`scan_note_tokens`,
  `edit_token_at`, `remove_token_at`, `reopen_token`, `slug_to_title`,
  `slugify`, …) — the wasm/client copy.
- `crates/tidl-web/src/markdown_tokens.rs` — a third, client-side scanner.

`slug_to_title` is literally defined in both
`crates/tidl-web/src/note_tokens.rs:254` and
`crates/tidl-markdown/src/parser.rs:1367`. Divergence between these copies
means a task chip can be rendered/scored differently on the client than on the
server.

Root cause: `tidl-markdown` is not wasm-safe (it depends on
`tokio`/`notify`/`walkdir` and carries a dead watcher — see 2.3), so `tidl-web`
cannot reuse its parser in the browser and forked it.

- Severity: **high** (drift risk on a core feature).
- Recommendation: split `tidl-markdown` into a pure, wasm-safe
  `tidl-markdown-core` (parser + token grammar + renderer, no async/fs deps)
  and the native-only watcher, then have `tidl-web` depend on the core crate
  and delete `note_tokens.rs`/`markdown_tokens.rs` duplicates.

### 2.2 Note/markdown rendering duplicated
`crates/tidl-web/src/util.rs:250 render_note_content` renders inline image
tokens, bare URLs, task/await/done chips, and `@Name` mentions client-side
(used by `notes/note_card.rs:364`). `tidl-markdown/src/parser.rs
render_to_html_with_urgency` renders the same constructs server-side (used by
`api/events.rs:2127`). Two renderers, one grammar.

- Severity: **medium** (same root cause as 2.1; folds into the core-crate fix).

### 2.3 Dead `MarkdownSource`/watcher abstraction
`tidl-markdown/src/{source,sources/,watcher}.rs` define a pluggable
`MarkdownSource` trait and a `notify`-based file watcher. Nothing outside the
crate uses them (`rg MarkdownSource` finds only self-references and
`lib.rs`'s glob re-export); the app instead reads files directly in
`tidl-server/src/markdown_sync.rs` (`tokio::fs::read_to_string`, `walkdir`) and
watches via `tidl-server/src/notify.rs`. This dead abstraction is also what
drags `tokio`/`notify`/`walkdir` into the crate and forces the 2.1
duplication.

Two further violations of `CLAUDE.md` §11 ("no async runtime assumptions — do
not call `tokio::spawn` directly") live in this dead code:
`crates/tidl-markdown/src/watcher.rs:134` and
`crates/tidl-markdown/src/sources/filesystem.rs:117`.

- Severity: **medium**.
- Recommendation: delete `source.rs`/`sources/`/`watcher.rs` (and drop
  `tokio`/`notify`/`walkdir`/`async-trait` from the crate), or if kept,
  document a real consumer. This directly unblocks 2.1.

### 2.4 Two `Event` types + deliberate data-access duplication
`tidl-server::storage::Event` (`storage.rs:532`) and
`tidl-web::types::Event` (`types.rs:40`) are two representations of the same
domain concept; #27 M2 already noted a full `From` is impossible across the
crate boundary and settled for a helper. Bigger: sync-token bookkeeping is
copy-pasted because `tidl-web` cannot call `Storage` —
`api/notes.rs:580` (`bump_calendar_sync_token`, "mirroring what
`tidl-server`'s `Storage::bump_sync_token` does") and `api/contacts.rs:668`
(`bump_address_book_sync_token`, "duplicated rather than shared because
`tidl-web` cannot depend on `tidl-server`"). Both even document that they know
they are duplicates.

- Severity: **high** (see §3).

### 2.5 Duplicated test scaffolding
`test_pool`, `test_storage`, `seed_user`, `create_test_user` are each
redefined in 3–4 files instead of living in one shared test-support module.

- Severity: **low**.

## 3. Data-access architecture (the biggest structural issue)

Two server-side code locations talk to SQLite and cannot share code because of
the crate dependency direction (`tidl-web` must not depend on `tidl-server`,
§6):

- `tidl-server/src/storage.rs` — the `Storage` facade + three store traits.
- `tidl-web/src/api/*.rs` — 175 `#[server]` fns issuing raw SQL directly.

Consequences:

1. **The `Storage` abstraction is only partially used.** `api/contacts.rs`,
   `api/notes.rs`, `api/calendars.rs`, `api/events.rs`, `api/admin.rs`,
   `api/done.rs`, `api/profile.rs` issue SQL themselves; `Storage` is only
   referenced from `seed_demo`, `mailer`, and `caldav_handlers`. The
   `CalendarStore`/`ContactStore`/`NoteStore` traits (#07) are consumed almost
   exclusively by `seed_demo.rs` and `mailer.rs`, so the trait layer is a
   partial abstraction rather than the real data boundary.
2. **Queries are overwhelmingly unchecked.** Of production SQL calls,
   **365 use `sqlx::query()` with a string literal** and only ~121 use the
   compile-time-checked `sqlx::query!`/`query_as!`/`query_scalar!` macros —
   roughly 75 % of production queries bypass compile-time verification, which
   `CLAUDE.md` §15 explicitly forbids ("`sqlx::query(...)` with a string-literal
   SQL body"). This directly contradicts the "discipline holds up" claim in
   #141, which assessed only the `format!` exception. (Test code adds ~210 more
   raw queries; the accepted `format!` exception is used ~16 times.)
3. **The `CalendarStore` trait is a fat interface** — 47 methods — so any new
   calendar operation forces an edit to the trait, every impl, and the dyn
   users.

- Severity: **high**.
- Recommendation: introduce a shared `tidl-store` (or `tidl-domain`) crate that
  both `tidl-server` and `tidl-web` (ssr) depend on, holding the store traits
  and SQL. `tidl-web` already depends on `sqlx` optionally, so this only moves
  code, it does not add a new wasm-unsafe dependency (the crate stays
  ssr-only). Then delete the duplicated sync-token helpers. Separately, decide
  the `query!`-macro policy and migrate deliberately (or amend §15 to record
  that runtime queries are accepted, as #141 did for the `format!` case).

## 4. Complexity & code smells

### 4.1 `#[server]` boilerplate dominating the annotation budget
The workspace has **241 `#[allow(...)]` attributes**, of which **157
`allow(unreachable_code)` and 47 `allow(unused_variables)`** (both entirely in
`tidl-web/src/api/*`) dominate. Each `#[server]` fn wraps its body in
`#[cfg(feature = "ssr")] { … }`, adds `#[allow(unused_variables)]` for
client-unused args, and finishes with a dead `#[allow(unreachable_code)] Ok(())`.
With 175 `#[server]` fns this is ~2 allowances each.

- Severity: **low–medium** (noise; obscures the handful of genuinely unusual
  allowances).
- Recommendation: a small `#[server]` wrapper/helper or a documented convention
  could remove most of these; at minimum centralise the trailing stub.

### 4.2 Bare context type aliases (§15) still present
`CLAUDE.md` §15 forbids `pub type FooCtx = RwSignal<T>;` used with
`provide_context`/`use_context`, and #141 stated none remain. Three do:

- `crates/tidl-web/src/app.rs:153` `AllCalendarsCtx = RwSignal<Vec<(CalendarEntry, bool)>>`
- `crates/tidl-web/src/app.rs:168` `CachedEventsCtx = RwSignal<Vec<Event>>`
- `crates/tidl-web/src/components/settings/mod.rs:48` `UserPrefsCtx = RwSignal<UserPrefs>`

All three are used with `use_context::<…>()`/`provide_context`. They happen not
to collide *today* (distinct inner `T`), but that is exactly the fragility §15
describes and the reason #141's "does not recur anywhere" claim is inaccurate.

- Severity: **medium** (latent silent-shadowing footgun).
- Recommendation: wrap all three in newtypes, as the `SessionTrigger`/
  `CalendarsRefresh` pattern already does.

### 4.3 Error-handling rule drift
`CLAUDE.md` §14 says Axum handlers return `Result<impl IntoResponse, AppError>`;
there is no `AppError` type at all — `caldav_handlers.rs` returns
`Result<impl IntoResponse, StatusCode>` (25 sites). `SessionStore`/`UserStore`
are concrete structs (not traits) while `CalendarStore`/`ContactStore`/
`NoteStore` are traits, so the `*Store` naming is inconsistent about whether it
means a trait or a concrete façade.

- Severity: **low**.
- Recommendation: reconcile §14 with reality (or introduce `AppError`), and
  rename the concrete `*Store` structs to avoid implying a trait.

### 4.4 Lint suppressions and dead code
19 `#[allow(dead_code)]` sites. Some are legitimate cfg-half fields
(`drag_grid.rs`, `grid.rs` — read only on the wasm32 half), but
`tidl-server/src/{crypto,git_sync,mailer,caldav_client}.rs` suppress unused
helpers that appear to have no caller.

- Severity: **low**.
- Recommendation: audit each; delete or note the future consumer.

## 5. Dependency management

- **Dead declared dependencies.** `anyhow` is declared in `tidl-caldav`,
  `tidl-carddav`, and `tidl-markdown` but referenced only in doc comments — a
  leftover from the #141 `thiserror` migration. `tidl-web` also declares
  `log = "0.4.33"` and an optional `anyhow` with no `log::`/`anyhow::` use
  found (verify with `cargo-udeps` before removal; leptos logging is a separate
  path).
- **Un-centralised versions.** `quick-xml`, `hex`, `walkdir`, `sha2`, `clap`,
  `percent-encoding`, and `tracing` (in `tidl-web`) are declared with inline
  versions in crate manifests while equivalent `workspace.dependencies` entries
  exist or are trivially centralisable. This is how version drift starts.
- **Duplicate transitive versions** (`cargo tree -d`): three `rand` lines
  (0.8/0.9/0.10), `thiserror` 1 & 2, `toml` 0.8 & 1.1, `base64` 0.21 & 0.22,
  `bitflags` 1 & 2, `winnow` 0.7 & 1.0, plus `regex`/`syn` appearing via two
  feature sets. `deny.toml` sets `multiple-versions = "warn"`, so this is not
  even surfaced as a failure.
- **Vulnerable pin.** `quick-xml = "0.31"` in `tidl-caldav` and `tidl-carddav`
  carries the DoS advisories already tracked by
  [#142](142-quick-xml-dos-vulnerability.md).
- **Boundary note.** `tidl-server` depends on `tidl-web` (the Leptos SSR
  requirement), so the "server" crate transitively pulls the entire UI crate.
  This is the documented design, not a defect, but it is why the data layer
  cannot be cleanly separated and should be remembered when reasoning about
  layering.

Severity: **medium** overall.

## 6. Architecture-rule conformance

| Rule | Status |
|------|--------|
| §6 dependency direction | compliant (no illegal `use`) |
| §8 wasm-safe `tidl-web` | compliant; `tidl-markdown` duplication is the workaround |
| §11 no `anyhow` in lib crates | compliant (fixed by #141); dead `anyhow` deps remain |
| §11 no `tokio::spawn` in libs | **violated** in dead `tidl-markdown` watcher (2 sites) |
| §14 handler error type | drifted (no `AppError`; `StatusCode` used) |
| §15 no raw `sqlx::query` literal SQL | **violated systemically** (~365 production sites) |
| §15 no bare context aliases | **violated** (3 sites, see 4.2) |

## What holds up well

- Production `unwrap`/`expect` is rare (~48 sites workspace-wide; the bulk of
  the 1300+ raw matches are tests), matching #141's finding.
- No `unsafe` outside `config.rs` tests; no `static mut`/`lazy_static`; no
  `todo!`/`unimplemented!`.
- Library crates keep `sqlx`/Axum/Leptos out of their public types, and the
  `thiserror` migration from #141 is complete in code (only the manifests
  lag).
- Test volume is healthy (~23 kLOC, ~25 %), and the crates with the most logic
  (`tidl-caldav`, `tidl-carddav`, `grid.rs`) have dense unit coverage.
- Workspace hygiene is otherwise good: pinned resolver, `deny.toml` with
  license allow-list, `SQLX_OFFLINE`, reproducible Nix shell.

## Recommended follow-ups (candidate backlog items)

1. **Split the `Storage` god object** (1.1) — mechanical, high value.
2. **Extract a shared `tidl-store`/`tidl-domain` crate** (3.1–3.2), then delete
   the duplicated sync-token helpers (2.4). Highest architectural leverage.
3. **Split `tidl-markdown` into a wasm-safe core + native watcher** and delete
   the `note_tokens.rs`/`markdown_tokens.rs` duplicates (2.1–2.3). Removes the
   largest redundancy cluster.
4. **Migrate to `query!` macros / document a policy** (3.2).
5. **Split the god components and `main`** (1.2), following the #27 playbook.
6. **Dependency cleanup**: drop dead `anyhow`/`log`, centralise versions in
   `workspace.dependencies` (5).
7. **Wrap the three bare context aliases** (4.2) — trivial, prevents the exact
   footgun #15/#141 warn about.

## Resolution

Assessment completed 2026-10-14. The deliverable is this report; no production
code was changed. The seven follow-ups above are the actionable output and are
candidates to be split into their own numbered items (as #141 split out #142).
#142 already covers the `quick-xml` pin.
