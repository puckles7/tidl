# Add CalendarStore Trait and Separate SessionStore

## CalendarStore trait

`Storage` is a concrete struct. All CalDAV handlers depend on the concrete type, making
handler logic untestable without a real SQLite database and making it impossible to add a
second storage backend.

Define an async `CalendarStore` trait with all calendar, event, lock, and dead-property
methods. Implement it for `Storage`. Change `AppState.storage` to
`Arc<dyn CalendarStore + Send + Sync>` so the concrete type is hidden behind the trait.

## SessionStore separation

Session management (create/get/delete/expire sessions) has no business being in the same
struct as CalDAV calendar operations. The `Storage` struct currently handles both.

Extract session methods into a new `SessionStore` struct (wrapping `SqlitePool`). Add a
`sessions: SessionStore` field to `AppState`. Update `session.rs` login/logout handlers
to use `state.sessions`.

Note: the `current_user`, `do_login`, `do_logout` Leptos server functions in `tidl-web/api.rs`
inject `SqlitePool` directly as an Axum Extension and perform raw session SQL — they cannot
call `SessionStore` (which is in `tidl-server`). This is a known limitation of the Leptos
server-function architecture; they continue to use raw SQL.

## Resolution

Implemented. `CalendarStore` trait defined in `storage.rs` via `async-trait`. `Storage`
implements it. `AppState.storage: Arc<dyn CalendarStore + Send + Sync>`. `SessionStore` in
`session_store.rs`; `AppState.sessions: SessionStore`.
