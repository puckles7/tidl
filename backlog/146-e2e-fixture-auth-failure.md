# Python e2e fixture: login returns 200 but follow-up server fns are "Not authenticated"

The Python e2e suite's `TidlServer.login_session()` POSTs to
`/api/do_login` and asserts HTTP 200, but the very next server-function call
made with that same `requests.Session` comes back
`HTTP 500: ServerError|Not authenticated`. Because most `test_sync.py` and
`test_cards_layout.py` fixtures authenticate this way before doing anything
else, those suites fail en masse at fixture setup rather than on any real
assertion.

Surfaced while running `rat` for #142; reproduced on the unmodified base
(`quick-xml` 0.31) as well, so it is a pre-existing fixture/auth problem, not
a regression. The protocol suites (`test_caldav.py`,
`test_caldav_sharing.py`, `test_carddav_sharing.py` — 44 tests) pass, because
they use CalDAV/CardDAV HTTP with credentials rather than the session-cookie
server-fn path.

## Observed

Against a hash-free build (`DISABLE_SERVER_FN_HASH=1 cargo leptos build`,
ports 3060/3001/3002/3061/3062/3070 free):

- `tests/test_sync.py` — 19 errors, all at fixture setup:
  `Server fn create_app_token returned HTTP 500: ServerError|Not authenticated`.
- `tests/test_cards_layout.py` — 32 failures, all at fixture setup:
  `Server fn create_contact_collection returned HTTP 500: ServerError|Not authenticated`.

`login_session()` (tests/fixtures.py:119) returns a fresh `Session` after a
200 from `POST /api/do_login`; `api_post_json()` (tests/fixtures.py:132) then
reuses it for the target fn. The login *response* is 200 either way (the
`do_login` server fn appears to render success even when it establishes no
usable session), so the failure only shows up on the follow-up call.

## Likely causes to check

- Does `POST /api/do_login` actually set the session cookie on its response,
  and is that cookie echoed on the next request? The server's session cookie
  is named `session_<port>` (see AGENTS "Manual verification gotchas"); the
  test env (`TIDL_DATA_DIR`/`TIDL_PORT`/`TIDL_SECRET`) may leave the cookie
  `Secure`/`SameSite` such that `requests` drops it over plain HTTP.
- Is `do_login` returning a 200 HTML/error body on bad credentials rather
  than a 4xx, so the fixture's status assertion never catches a real login
  failure (e.g. admin user not seeded as expected)?
- Whether `create-admin` in `TidlServer.start()` (tests/fixtures.py:60) and
  the `do_login` path agree on the email/username form (`admin@tidl.test` vs
  `admin`) under the current auth code.

## Functional Requirements

- Make the e2e fixtures actually authenticate (fix the cookie handling and/or
  make `do_login` fail loudly on bad credentials), so `test_sync.py` and
  `test_cards_layout.py` reach their assertions.
- If the login helper is at fault, assert on a real authenticated response
  (e.g. a `/me`-style call) in `login_session()` so a silent 200 can't mask a
  broken session again.
- Confirm the full `rat` suite (cargo test + pytest) goes green, or that any
  remaining failures are genuine assertion failures, not setup errors.

## Open Questions

- Is this specific to this sandbox/HTTP setup, or does it also reproduce in
  CI? (Worth checking before assuming the auth code is wrong.)

## Resolution

The login helper was at fault, and the same day's security hardening pass
(`#141`) had broken the sync suite in two independent ways.

**Root cause 1 — Secure cookie dropped over plain HTTP.** Commit `74d94db`
("add Secure attribute to the session cookie") made `server.secure_cookie`
default `true`; the Python fixture runs the server over plain `http://` but
never opted out, so `requests` refused to store/send the `Secure` cookie and
`do_login`'s 200 masked a session the next call couldn't use. (The Playwright
fixture was unaffected — Chromium treats `localhost` as a secure context.)

- `TidlServer._env()` now sets `TIDL_SECURE_COOKIE=false`, matching a real
  plain-HTTP deployment (the documented reason that option exists).
- `login_session()` now verifies the session by calling `get_session_info`
  and asserting the returned username, so a silent 200 can't mask a broken
  session again.

**Root cause 2 — SSRF guard blocks loopback remotes.** Commit `8cdb6b9`
(also `#141`) installs a `SsrfGuardedResolver` that refuses loopback/private
addresses on every server-side fetch (`caldav_client.rs`, `webcal_sync.rs`),
so `test_sync.py`'s Sink → Source sync over `localhost:3001` failed at the
first `sync-collection REPORT` (stored `last_error`).

- Added a test/dev-only escape hatch: `TIDL_SSRF_ALLOW_INTERNAL`
  (`ssrf_guard.rs`) makes the resolver permit internal addresses, logging a
  loud warning when active. `TidlServer._env()` sets it, alongside a unit test
  for the env parsing and a flag check on the resolver. Default remains
  blocking; never set it in production.

**Result:** full `rat` is green — `cargo test --workspace` (all suites) plus
`pytest tests/` (122 passed, including all 22 `test_sync.py` and the
`test_cards_layout.py` suite).

