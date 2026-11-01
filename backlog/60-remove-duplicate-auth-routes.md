# Remove duplicate /auth/login and /auth/logout routes

`tidl-server` exposes hand-written `POST /auth/login` and `POST /auth/logout` Axum
handlers (`session.rs:24-53`) that duplicate the `#[server]` functions `do_login`/
`do_logout` (`tidl-web/src/api/auth.rs:72,132`), which already handle cookie-setting
correctly via `ResponseOptions`. The Leptos frontend only ever calls `do_login`/
`do_logout` (`app.rs:274,496`); the manual routes are reachable only from
`tests/fixtures.py`'s `login_session()`, which predates (or was never migrated to)
the `DISABLE_SERVER_FN_HASH=1`-based JSON-RPC pattern (`api_post_json()`) the rest
of the test suite already uses to call `#[server]` fns directly.

## Context / Background

Discovered while comparing `#[server]`-macro endpoints against manual `.route()`
endpoints in `tidl-server/src/main.rs`. `login_handler`/`logout_handler` aren't
called anywhere except the test fixtures — this is otherwise dead, duplicated logic.

## Functional Requirements

- Rewrite `tests/fixtures.py`'s `login_session()` to authenticate via `do_login`
  through the same `api_post_json`-style call the rest of the suite uses (built with
  `DISABLE_SERVER_FN_HASH=1`), instead of posting to `/auth/login`.
- Update `test_sync.py` / any other direct `/auth/login`-`/auth/logout` callers the
  same way.
- Remove the `/auth/login` and `/auth/logout` `.route()` registrations from
  `tidl-server/src/main.rs`.
- Remove `login_handler`/`logout_handler` from `tidl-server/src/session.rs`.
- Do **not** touch `session_token_from_headers`/`SessionCookieName` — confirmed
  shared with `sharing.rs`, `export.rs`, `oidc.rs`, and `main.rs`.
- `cargo test --workspace` and the Python test suite (`pytest tests/`) must pass
  after the change (per repo convention — see `CLAUDE.md` §3, §16).

## Open Questions

- Whether `do_login`/`do_logout` need an explicit `#[server(endpoint = "...")]`
  pinned, or whether relying on `DISABLE_SERVER_FN_HASH=1` in the test build is
  sufficient long-term (it's already how the rest of the suite calls `#[server]`
  fns, so likely fine as-is).

## Resolution

- `tests/fixtures.py`'s `login_session()` now POSTs `{email, password}` JSON to
  `/api/do_login` (the `do_login` `#[server(input = Json)]` fn) instead of
  `/auth/login`. This is the only place the manual routes were used; `test_sync.py`
  and the other suites reach `login_session()` transitively, so no further test
  changes were needed. `/auth/logout` had no callers at all.
- Removed the `/auth/login` and `/auth/logout` `.route()` registrations from
  `tidl-server/src/main.rs`.
- Removed `login_handler`, `logout_handler`, the `LoginRequest` struct, and the
  `SESSION_TTL_SECS` const from `tidl-server/src/session.rs`; the file now only
  holds `session_token_from_headers` (still shared with `sharing.rs`, `export.rs`,
  `oidc.rs`, `main.rs`) and its two imports.
- No `#[server(endpoint = ...)]` pin was needed — the hash-free test build
  (`DISABLE_SERVER_FN_HASH=1`) gives `/api/do_login` a stable URL, matching how the
  rest of the suite already calls `#[server]` fns.
- Verified: `cargo test --workspace` (341 pass), `cargo clippy --workspace -D
  warnings`, and the non-browser `test_sync.py` cases (which exercise
  `login_session`) all pass. The Playwright browser tests error on this machine
  due to a pre-existing GLIBC/Chromium environment mismatch, unrelated to this
  change (confirmed identical failure on clean `main`).
