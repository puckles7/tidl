# Architecture Cleanup

A collection of code smells, wrong abstractions, and duplication identified in a May 2026 architectural review. Items are grouped by priority and ordered so lower-risk changes can be done first to build confidence before the bigger structural refactors.

## Functional Requirements

## Progress

**H1 — Done (2026-05-27)**
`CalendarView` split into 7 focused files in `calendar/`. The merged `EventFormDialog` also removes the `NewEventDialog`/`EditEventDialog` duplication. Note: the location pre-fill bug fix could not be applied — `Event` struct has no `location` field.

**H2 — Done (2026-05-28)**
`SettingsModal` split into 8 files in `settings/`. Seven panel components extracted: `PreferencesPanel`, `ImportPanel`, `SmtpConfigPanel`, `SharingPanel`, `AppTokensPanel`, `RemoteSourcesPanel`, `MarkdownSourcesPanel`. `SettingsModal` in `mod.rs` reduced to the dialog shell and tab bar. Lazy-loading guards replaced by component-mount loads (`<Show>` conditionally mounts/unmounts panels, making the old `*_loaded` guards redundant).

**M1 — Done (2026-05-28)**
`CalendarsRefresh`, `EventsRefresh`, and `SessionTrigger` replaced with `#[derive(Clone, Copy)]` newtype structs. Each struct has its own `TypeId`, fixing the context-map collision where `provide_context(events_refresh)` was silently overwriting `provide_context(cals_refresh)` (both were `RwSignal<u32>` with the same `TypeId`). Consumer sites use `.map(|r| r.0)` to extract the inner `RwSignal<u32>`; internal component props that accepted `EventsRefresh` now take `RwSignal<u32>` directly.

**M3 — Done (2026-05-28)**
`create_and_set_session` helper extracted in `api.rs`. The three identical blocks (UUID generation, `INSERT INTO sessions`, `Set-Cookie` header) that appeared in `do_login`, `do_verify_email`, and `do_reset_password` are replaced by a single call to the helper. Also fixed pre-existing test compilation errors in `focus.rs` where `include_item` test calls were missing the `now_local` 4th argument.

**H3 — Done (2026-05-29)**
The 4 inline `match tls_mode { "tls" | "starttls" | _ }` transport-build blocks in `api.rs` now call `crate::ssr_types::smtp_send(...)` instead of duplicating the logic. The `Credentials`, `AsyncSmtpTransport`, `Tokio1Executor`, and `AsyncTransport` imports were removed from each call site. `smtp_send` already existed in `ssr_types` (used by `mailer.rs`); H3 completes the consolidation.

**M4 — Done (2026-05-29)**
Private `SmtpConfigRow` struct with `#[cfg(feature = "ssr")]` and `#[derive(sqlx::FromRow)]` defined in `api.rs`. All 5 `try_get` extraction sites replaced: helper functions `send_verification_email_via_smtp` and `send_password_reset_email` now accept `SmtpConfigRow` directly; the 3 inline query sites and the 2 callers of those helpers now use `sqlx::query_as::<_, SmtpConfigRow>(...)` for compile-time column verification. `use sqlx::Row` imports removed from the affected functions.

**M8 — Done (2026-05-29)**
`LockInfo` gained `#[derive(sqlx::FromRow)]`. The three identical field-by-field mapping blocks in `get_lock`, `get_effective_locks`, and `get_locks_for_resource` replaced by `sqlx::query_as!(LockInfo, ...)` — the manual struct literals removed entirely.

**M11 — Done (2026-05-29)**
`SmtpConfigRow` moved from `api.rs` to `ssr_types.rs` (as `pub(crate)`) alongside the `smtp_send` helper where all SMTP behaviour lives. Added `from_addr()` method to eliminate the 5 identical `if from_name.is_empty()` branches in `api.rs`. Added `SMTP_CONFIG_QUERY` const to replace the repeated SQL string. All 5 query sites and all call sites in `api.rs` updated.

**L1 — Done (2026-05-29)**
4 silent `.ok()` calls on cleanup queries in `api.rs` replaced with `if let Err(e) = ... { tracing::warn!(...) }` so failures surface in logs without aborting the request.

**M12 — Done (2026-05-29)**
`code_email_body(display_name, code, code_label, action, footer_clause) -> (String, String)` extracted in `api.rs`. The two identical email body blocks (verification + password reset) now call this helper with different labels and phrasing.

**M6 — Done (2026-05-29)**
`sse_handler` signature changed from `State<AppState>` to `Extension<broadcast::Sender<String>>`. `change_tx` was already registered as a standalone `Extension` in `main`. The handler now declares exactly what it needs: `change_tx` (for subscription) and `pool` (for session auth).

**M10 — Done (2026-05-29)**
`auth() -> Result<(SqlitePool, String), ServerFnError>` and `auth_admin()` helpers added to `api.rs`. Replace the 3-line boilerplate (`use axum::Extension; use leptos_axum::extract; use sqlx::SqlitePool; let Extension(pool) = ...; let headers = ...; let username = current_user(...)`) that appeared 33 times. After replacement, functions that also extract other extensions (ServerSecret, SyncTrigger) still use `use axum::Extension; use leptos_axum::extract;` locally for those additional extractions.

**M9 — Skipped**
After M5 unified the `change_tx.send()` protocol, the remaining duplication across the 3 sync loops is limited to the error handling — which is materially different in each loop (`set_source_error`, `update_webcal_sync_status`, vs. warn-only). A `SyncOutcome` helper would require a closure parameter for error handling; the added abstraction complexity exceeds the benefit.

**M7 — Done (2026-05-29)**
`use_calendars_refresh()` and `use_events_refresh()` hooks added to `app.rs`, each returning `RwSignal<u32>`. All 6 component files (`filter_sidebar`, `todo_list`, `focus`, `calendar_list`, `calendar/mod`, `tickler`) updated to call the hook instead of the 60-character `use_context::<...>().map(|r| r.0).unwrap_or_else(|| RwSignal::new(0u32))` inline lookup.

**M2 — Done (2026-05-29)**
`event_row_to_web_events(uid, summary, calendar, calendar_color, description, raw_start, raw_end, ical) -> Vec<Event>` extracted in `api.rs`. The alarm-parsing, `is_all_day_ical`, RRULE expansion, and date-trimming logic are now in the named helper; `get_events` reduces to a fetch loop of one-liners. A full `impl From<StorageEvent> for WebEvent` is not possible: `storage::Event` lives in `tidl-server` which `tidl-web` cannot depend on, and the JOIN query adds `calendar_color` not present in `storage::Event`.

**M5 — Done (2026-05-29)**
`ChangeKind` enum (`Cals`, `Events`, `All`) added to `tidl-caldav/src/change.rs` with `to_msg(&username) -> String` and `parse_msg(msg) -> (Self, &str)`. The implicit `"<kind>:<username>"` string protocol is now enforced at type level. Updated: `api.rs` `notify_change` (dropped local private enum, uses `tidl_caldav::change::ChangeKind`), `caldav_handlers.rs` (3 send sites), `remote_sync.rs` (2 send sites, fixed bare-username legacy bug), `webcal_sync.rs` (1 send site), `markdown_sync.rs` (2 send sites), `main.rs` SSE handler (uses `ChangeKind::parse_msg`).

## Resolution

Completed 2026-05-30. All 15 actionable items finished; M9 (sync loop triplication) intentionally skipped after analysis showed the three loops have materially different error-handling logic (`set_source_error`, `update_webcal_sync_status`, warn-only), making a shared helper add complexity without benefit.

Key outcomes:
- **H1/H2**: `CalendarView` (2,738 lines) and `SettingsModal` (1,620 lines) split into focused files under `calendar/` and `settings/` subdirectories respectively.
- **Type safety**: `ChangeKind` enum enforces the SSE wire protocol; newtype refresh signals fix context `TypeId` collisions.
- **Boilerplate elimination**: `auth()` / `auth_admin()` helpers replace 33 copies of the 3-line pool/headers/username extraction pattern.
- **sqlx compile-time checks**: `SmtpConfigRow` and `LockInfo` both switched to `query_as!`; `SMTP_CONFIG_QUERY` const eliminates repeated SQL literals.
- **Observability**: Silent `.ok()` on cleanup queries replaced with `tracing::warn!`.

All 114 tests pass at close.

---

### HIGH — God Components

**H1. Split `CalendarView` (~2,738 lines)**
`crates/tidl-web/src/components/calendar.rs`

Extract into focused units:
- `date_helpers.rs` — pure date/weekday functions (lines 22–141)
- `#[component] MiniCalendar` — the small month navigator
- `#[component] WeekGrid` — the main week layout + event chips
- `#[component] EventForm` — new/edit event dialog (reusable)

**H2. Split `SettingsModal` (~1,620 lines)**
`crates/tidl-web/src/components/settings.rs`

Seven independent panels in one component. Extract each into its own `#[component]`:
- `PreferencesPanel` (timezone, time format, dark mode)
- `ImportPanel` (.ics import)
- `SmtpConfigPanel`
- `RemoteSourcesPanel` (CalDAV)
- `MarkdownSourcesPanel`
- `SharingPanel`
- `AppTokensPanel`

**H3. SMTP Transport Builder — 4× duplication**
`crates/tidl-web/src/api.rs` ~lines 681, 776, 1057, 3200+

The `match tls_mode { "tls" | "starttls" | _ }` block is copy-pasted four times with subtly different error handling. Extract to:
```rust
fn build_smtp_transport(host, port, tls_mode, credentials) -> Result<AsyncSmtpTransport>
```
and move alongside `SmtpConfigRow::build_transport(&self)` in `mailer.rs`.

---

### MEDIUM — Duplication & Wrong Abstractions

**M1. Primitive obsession for Leptos context TypeIds**
`crates/tidl-web/src/app.rs` lines 87–94

`EventsRefresh` is `RwSignal<u64>` solely so its `TypeId` differs from `RwSignal<u32>`. Replace all three refresh signals with distinct newtype wrappers:
```rust
pub struct CalendarsRefresh(pub RwSignal<u32>);
pub struct EventsRefresh(pub RwSignal<u32>);
pub struct SessionTrigger(pub RwSignal<u32>);
```

**M2. Two `Event` types — duplicated domain concept**
`crates/tidl-server/src/storage.rs` lines 162–179 vs `crates/tidl-web/src/types.rs` lines 34–54

Add an explicit `impl From<StorageEvent> for WebEvent` conversion, moving the RRULE-expansion and alarm-parsing out of the `get_events` server function.

**M3. Session cookie creation — 3× duplication**
`crates/tidl-web/src/api.rs` ~lines 205, 554, 959

`do_login`, `do_verify_email`, `do_reset_password` each repeat UUID generation, `+86400 seconds` INSERT, and `Set-Cookie` header construction. Extract:
```rust
async fn create_and_set_session(username: &str, pool: &SqlitePool, opts: &ResponseOptions) -> Result<(), ServerFnError>
```

**M4. SMTP config row extraction — 3× duplication**
`crates/tidl-web/src/api.rs` ~lines 626, 740, 1003

`try_get("host").unwrap_or_default()` for each field, three times. Switch to `sqlx::query_as!(SmtpConfigRow, ...)` for compile-time column verification and a single extraction path.

**M5. SSE change notification — implicit string protocol**
Server (api.rs ~105), SSE handler (main.rs ~340), client (app.rs ~120) each parse `"<kind>:<username>"` independently. Define an enum:
```rust
enum ChangeKind { Calendars, Events, All }
```
and serialize to JSON for the SSE data field.

**M6. `AppState` over-injection in SSE handler**
`crates/tidl-server/src/main.rs` lines 288–375

Handler receives `State<AppState>` + `Extension<SqlitePool>` but only uses `state.change_tx` and `pool`. Define a focused sub-state type and inject only that.

**M7. Leptos context leakage**
Components directly call `.update(|n| *n += 1)` on raw signals from context. Wrap each in a hook:
```rust
fn use_calendars_refresh() -> impl Fn() { ... }
fn use_events_refresh() -> impl Fn() { ... }
```

**M8. `LockInfo` row mapping — 3× duplication**
`crates/tidl-server/src/storage.rs` ~lines 812, 846, 870

Three methods repeat identical field-by-field `LockInfo` construction. Switch to `sqlx::query_as!(LockInfo, ...)`.

**M9. Sync loop triplication**
`crates/tidl-server/src/remote_sync.rs`, `webcal_sync.rs`, `markdown_sync.rs`

All three loops share the same structure: load sources → iterate → sync → notify `change_tx` → log errors. Define a `SyncOutcome` type and a shared `handle_sync_outcome()` helper.

**M10. Server function auth boilerplate — 30+ occurrences**
Every server function in `crates/tidl-web/src/api.rs` manually extracts pool, headers, and calls `current_user`. Consider a helper or macro to standardise the pattern.

**M11. `SmtpConfigRow` feature envy**
Struct lives in `storage.rs` but all behaviour belongs in `mailer.rs`. Move it there and add `fn build_transport(&self) -> Result<AsyncSmtpTransport>`.

**M12. Email template strings — 2× duplication**
`crates/tidl-web/src/api.rs` ~lines 642, 1018

Verification email and password reset email share header/code-display/footer structure but are copy-pasted `format!()` strings. Extract a shared `fn render_email_template(title, body_html, body_text) -> (String, String)`.

---

### LOW — Minor Issues

**L1. Silent `.ok()` on cleanup queries**
`crates/tidl-web/src/api.rs` ~lines 451, 861, 930

`DELETE FROM email_verifications ... .await.ok()` silently drops errors. At minimum log with `warn!` on failure.

## Suggested Order

Work low-risk first:

1. M1 — newtype wrappers (tiny, removes footgun)
2. M3 — extract `create_and_set_session` (security-relevant, low blast radius)
3. H3 — extract `build_smtp_transport` (pure function, easy to test)
4. M4 — `sqlx::query_as!` for `SmtpConfigRow`
5. M8 — `sqlx::query_as!` for `LockInfo`
6. M11 — move `SmtpConfigRow` to `mailer.rs`
7. M5 — typed SSE notification enum
8. M2 — `From<StorageEvent> for WebEvent`
9. M7 — context hooks
10. M9 — sync loop helper
11. H2 — split `SettingsModal`
12. H1 — split `CalendarView`
13. M10 — server function auth pattern
14. L1 — warn on cleanup errors
15. M12 — email template helper
16. M6 — focused SSE state type
