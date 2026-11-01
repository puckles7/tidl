"""
Browser smoke tests for tidl (pure-CSR build).

Starts a headless Chromium browser via Playwright, navigates to every
app route, and verifies that the Leptos WASM bundle mounts without
panicking and renders the expected content.

Requires a running tidl server. Run via:
    test-hydration admin@localhost admin   # from inside the nix dev shell
    pytest tests/test_hydration.py -v     # directly, server on localhost:3060

Flags (add to pytest invocation or via test-hydration script):
    --server-url   base URL of the server          (default: http://localhost:3060)
    --email        web-UI login email              (default: falls back to --username)
    --password     account password                (default: admin)

The Playwright browser binaries are provided by the Nix dev shell via
PLAYWRIGHT_BROWSERS_PATH — no `playwright install` step is needed.
"""

import pytest
from playwright.sync_api import Page

# Routes to exercise in the authenticated-app tests.
ROUTES = ["/", "/focus", "/notes"]

# How long (ms) to wait after networkidle for the WASM event loop to settle.
HYDRATION_SETTLE_MS = 1500


# ── Session fixtures ──────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def login_email(request, tidl_server):
    """Email used for web-UI login.

    Reads --email if given; otherwise uses the email the auto-started server
    created (tidl_server.email = "<username>@tidl.test").
    """
    email = request.config.getoption("--email")
    return email if email else tidl_server.email



@pytest.fixture(scope="session")
def session_cookies(server_url, login_email, credentials, browser):
    """Log in once and return the resulting cookies for the whole session.

    Leptos server functions use hashed URLs (/api/do_login{hash}) that change
    with each build, and the on:submit handler is only attached after WASM
    hydrates.  We therefore:
      1. Listen for the WASM-emitted console.log that fires at the start of
         hydrate() as a reliable signal that WASM is running.
      2. Use expect_response (matching on '/api/do_login') so we wait for the
         actual server function round-trip before inspecting the result.

    If login fails the fixture raises so all authenticated tests are
    skipped with a clear error rather than failing with an auth error.
    """
    _, password = credentials
    ctx = browser.new_context()
    page = ctx.new_page()
    try:
        # Track WASM hydration via the console message Leptos emits.
        wasm_hydrated: list[bool] = []
        page.on(
            "console",
            lambda m: wasm_hydrated.append(True)
            if "[tidl] WASM hydrate" in m.text
            else None,
        )

        page.goto(f"{server_url}/", wait_until="load")

        # Wait up to 10 s for WASM to hydrate and attach event handlers.
        for _ in range(100):
            if wasm_hydrated:
                break
            page.wait_for_timeout(100)
        else:
            raise RuntimeError(
                "WASM did not emit its hydration console message within 10 s. "
                "Is the server running the client-debug WASM build?"
            )

        # Small buffer: mount_to_body(App) runs synchronously after the log
        # call, but IPC latency means Python may see the event slightly before
        # the JS engine finishes attaching all handlers.
        page.wait_for_timeout(200)

        page.fill("input[type='email']", login_email)
        page.fill("input[type='password']", password)

        # click() + expect_response: wait for the server function round-trip.
        with page.expect_response(
            lambda r: "/api/do_login" in r.url, timeout=10_000
        ) as resp_info:
            page.click("button[type='submit']")

        login_resp = resp_info.value
        if login_resp.status != 200:
            raise RuntimeError(
                f"Login server function returned HTTP {login_resp.status} "
                f"for {login_email!r}."
            )

        page.wait_for_load_state("load")

        # Remaining email input means we are still on the login page.
        if page.locator("input[type='email']").is_visible():
            error_text = (
                page.inner_text(".login-error")
                if page.locator(".login-error").count()
                else ""
            )
            raise RuntimeError(
                f"Login failed for {login_email!r}. "
                f"Server message: {error_text!r}. "
                "Pass --email and --password with valid credentials."
            )

        return ctx.cookies()
    finally:
        page.close()
        ctx.close()


# ── Per-test page fixtures ────────────────────────────────────────────────────

@pytest.fixture()
def anon_page(browser):
    """Fresh unauthenticated browser page with console-error collection."""
    ctx = browser.new_context()
    page = ctx.new_page()
    errors: list[str] = []
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    yield page, errors
    page.close()
    ctx.close()


@pytest.fixture()
def auth_page(browser, session_cookies):
    """Fresh authenticated browser page with console-error collection."""
    ctx = browser.new_context()
    ctx.add_cookies(session_cookies)
    page = ctx.new_page()
    errors: list[str] = []
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    yield page, errors
    page.close()
    ctx.close()


# ── Helpers ───────────────────────────────────────────────────────────────────

def wasm_panics(errors: list[str]) -> list[str]:
    return [e for e in errors if "panicked at" in e.lower() or "wasm trap" in e.lower()]


def load_and_settle(page: Page, url: str) -> None:
    """Navigate to *url* and wait for the WASM hydration loop to settle.

    We use wait_until="load" (not "networkidle") because authenticated pages
    open a persistent SSE stream (/api/stream) that keeps the network active
    indefinitely, which would cause networkidle to block forever.
    """
    page.goto(url, wait_until="load")
    page.wait_for_timeout(HYDRATION_SETTLE_MS)


# ── Login page (unauthenticated) ──────────────────────────────────────────────

def test_login_page_no_wasm_panic(server_url, anon_page):
    """The login page must hydrate without a WASM panic."""
    page, errors = anon_page
    load_and_settle(page, f"{server_url}/")
    panics = wasm_panics(errors)
    assert not panics, "WASM panic on login page:\n" + "\n".join(panics)


def test_login_form_interactive_after_hydration(server_url, anon_page):
    """After hydration the email input must accept typed text (not just static HTML)."""
    page, errors = anon_page
    load_and_settle(page, f"{server_url}/")
    page.fill("input[type='email']", "test@example.com")
    assert page.input_value("input[type='email']") == "test@example.com"
    # Filling a field implies event handlers are wired — no panic while doing so.
    panics = wasm_panics(errors)
    assert not panics, "WASM panic while interacting with login form:\n" + "\n".join(panics)


# ── Authenticated app routes ──────────────────────────────────────────────────

@pytest.mark.parametrize("route", ROUTES)
def test_no_wasm_panic(server_url, auth_page, route):
    """No route must produce a WASM panic in the browser console."""
    page, errors = auth_page
    load_and_settle(page, f"{server_url}{route}")
    panics = wasm_panics(errors)
    assert not panics, f"WASM panic on {route}:\n" + "\n".join(panics)


@pytest.mark.parametrize("route", ROUTES)
def test_page_has_content(server_url, auth_page, route):
    """Each route must render non-trivial visible content (guards against blank SSR)."""
    page, errors = auth_page
    load_and_settle(page, f"{server_url}{route}")
    body = page.inner_text("body").strip()
    assert len(body) > 50, f"Route {route} rendered almost no content (got {len(body)} chars)"


@pytest.mark.parametrize("route", ROUTES)
def test_navigation_links_present(server_url, auth_page, route):
    """The nav bar must be rendered on every route after hydration."""
    page, errors = auth_page
    load_and_settle(page, f"{server_url}{route}")
    nav_links = page.locator("nav a, header a").all()
    assert len(nav_links) >= 2, f"Expected nav links on {route}, found {len(nav_links)}"


# ── Calendar performance ──────────────────────────────────────────────────────

@pytest.fixture()
def perf_page(browser, session_cookies):
    """Authenticated page that also collects [perf] console messages emitted by
    the WASM build's instrumentation points."""
    ctx = browser.new_context()
    ctx.add_cookies(session_cookies)
    page = ctx.new_page()
    errors: list[str] = []
    perf_msgs: list[str] = []
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    page.on("console", lambda m: perf_msgs.append(m.text) if m.text.startswith("[perf]") else None)
    yield page, errors, perf_msgs
    page.close()
    ctx.close()


def test_calendar_events_filter_not_quadratic(server_url, perf_page):
    """events-filter Memo must not recompute once per event card on initial load.

    Before the Signal::derive → Memo fix, every event card called events.get()
    which re-ran the full filter closure (O(n) per card → O(n²) total).
    With Memo the filter runs only when cached_events or calendars_ctx changes,
    so the count is a small fixed number regardless of how many events exist.

    Threshold: ≤ 5 fires covers initial load (cached_events set, possibly a
    calendars_ctx change), with headroom for future minor additions.  If this
    fires more than 5 times it almost certainly means the signal was changed
    back to Signal::derive or a new per-card subscription was introduced.
    """
    page, errors, perf_msgs = perf_page
    load_and_settle(page, f"{server_url}/")

    panics = wasm_panics(errors)
    assert not panics, "WASM panic on calendar:\n" + "\n".join(panics)

    filter_fires = [m for m in perf_msgs if m.startswith("[perf] events-filter")]
    assert len(filter_fires) <= 5, (
        f"events-filter fired {len(filter_fires)} times on initial load "
        f"(expected ≤ 5 — likely regression to Signal::derive):\n"
        + "\n".join(filter_fires)
    )


def test_calendar_events_filter_timing(server_url, perf_page):
    """Each events-filter run should complete within a reasonable time.

    16 ms+ per run indicates the filter is running synchronously on the JS main
    thread and blocking a frame.  With a large event set this becomes visible
    jank.  This test catches accidental O(n²) reintroductions that show up as
    slow individual filter runs rather than excessive call counts.
    """
    page, errors, perf_msgs = perf_page
    load_and_settle(page, f"{server_url}/")

    panics = wasm_panics(errors)
    assert not panics, "WASM panic on calendar:\n" + "\n".join(panics)

    filter_fires = [m for m in perf_msgs if m.startswith("[perf] events-filter")]
    if not filter_fires:
        pytest.skip("No [perf] events-filter messages — WASM perf instrumentation not present")

    def parse_ms(msg: str) -> float:
        # "[perf] events-filter: 16.67ms (6418 events)"
        try:
            return float(msg.split(":")[1].strip().split("ms")[0])
        except (IndexError, ValueError):
            return 0.0

    slow = [m for m in filter_fires if parse_ms(m) > 50.0]
    assert not slow, (
        f"{len(slow)} events-filter run(s) exceeded 50 ms:\n" + "\n".join(slow)
    )


# ── Notes tab / Focus integration (backlog #38, updated #49, #115) ───────────

def test_note_collection_pill_and_visibility_toggle(server_url, auth_page):
    """A note's inline `#task` token surfaces in Focus's Tasks filter and its
    inline `#await` token in the Await filter, each excluded from the other.

    Covers the end-to-end path: a note collection whose note-origin VTODO shows
    up in the right Focus filter, tagged with a collection-coloured
    "<Collection> note" pill, and the "Note collections" sidebar panel's
    visibility toggle hides it (`HiddenNoteCollectionsCtx` end-to-end).

    Backlog #115 removed the separate /tasks and /await screens; the test now
    switches Focus's All/Calendar/Tasks/Await content-type pill instead.
    Backlog #113 moved a note's task/await from a single note-level kind to
    inline `#task` / `#await` tokens, so each note is built by adding a token
    through the editor's "+ Task" / "+ Await" buttons.
    """
    page, errors = auth_page
    coll_name = "E2E Notes Collection"

    page.goto(f"{server_url}/notes", wait_until="load")
    page.wait_for_timeout(500)

    # Create a note collection via the Notes tab sidebar.
    page.click("button[title='New note collection']")
    page.wait_for_selector(".dialog input.form-input[type='text']", timeout=5000)
    page.fill(".dialog input.form-input[type='text']", coll_name)
    with page.expect_response(lambda r: "/api/create_note_collection" in r.url, timeout=10_000):
        page.click(".dialog-actions >> text=Create")
    # The Notes sidebar renders collections through the shared `SidebarPill`,
    # whose toggle input carries "Show <name>" as its aria-label.
    coll_toggle = f"input[aria-label='Show {coll_name}']"
    page.wait_for_selector(coll_toggle, timeout=5000)

    def add_note_with_token(token: str):
        """Create a note in `coll_name` whose body carries one inline token."""
        page.click("button.notes-pin-filter:has-text('New note')")
        page.wait_for_selector(".note-editor-textarea", timeout=5000)
        # The editor parses `#task/…` / `#await/…` typed directly into the body.
        page.fill(".note-editor-textarea", f"body {token}")
        page.click(f"button.note-chip:has-text('{coll_name}')")
        with page.expect_response(lambda r: "/api/save_note" in r.url, timeout=10_000):
            page.click(".dialog-actions >> text=Create")
        page.wait_for_timeout(500)

    add_note_with_token("#task/p50/verify-task-pill")
    add_note_with_token("#await/2026-08-01/verify-await-pill")

    def pill_text_with_filter(filter_label: str) -> str:
        page.goto(f"{server_url}/focus", wait_until="load")
        page.wait_for_timeout(1500)
        # Focus's content-type pill replaced the old /tasks and /await routes.
        page.click(f".focus-filter-ctrl button:has-text('{filter_label}')")
        page.wait_for_timeout(500)
        pill = page.locator(f"text={coll_name}").first
        pill.wait_for(timeout=8000)
        return pill.inner_text()

    text = pill_text_with_filter("Tasks")
    assert coll_name in text, f"Tasks filter: expected a pill containing {coll_name!r}, got {text!r}"

    text = pill_text_with_filter("Await")
    assert coll_name in text, f"Await filter: expected a pill containing {coll_name!r}, got {text!r}"

    page.goto(f"{server_url}/focus", wait_until="load")
    page.wait_for_timeout(1000)
    page.click(".focus-filter-ctrl button:has-text('Tasks')")
    page.wait_for_timeout(500)
    assert page.locator("text=Verify Await Pill").count() == 0, (
        "Tasks filter should not show the AWAIT-only note"
    )

    # Hide the collection from the Focus sidebar; the item must disappear.
    page.goto(f"{server_url}/focus", wait_until="load")
    page.wait_for_timeout(1000)
    page.click(".focus-filter-ctrl button:has-text('Tasks')")
    page.wait_for_timeout(500)
    page.wait_for_selector(coll_toggle, timeout=8000)
    # The checkbox itself is visually hidden behind the styled track/thumb;
    # click the label (which toggles it and fires the change handler).
    page.locator(".sidebar-pill").filter(has_text=coll_name).first.locator(
        "label.sidebar-pill-toggle"
    ).click()
    page.wait_for_timeout(500)
    assert page.locator(f".focus-cards :text('{coll_name}')").count() == 0, (
        "Focus still shows an item from a hidden note collection"
    )

    panics = wasm_panics(errors)
    assert not panics, "WASM panic during note-collection integration test:\n" + "\n".join(panics)


# ── Ad-hoc task/await create, edit and delete (backlog #116) ──────────────────

def test_adhoc_task_create_edit_delete(server_url, auth_page):
    """Create an ad-hoc task from Focus, reopen it via the ↗ arrow, and delete it.

    Covers the create dialog's +Task mode, the edit dialog (prefilled title and
    calendar), and one-click Delete.
    """
    page, errors = auth_page
    title = "Ad-hoc E2E task"

    page.goto(f"{server_url}/focus", wait_until="load")
    page.wait_for_timeout(1500)
    page.click(".focus-filter-ctrl button:has-text('Tasks')")
    page.wait_for_timeout(300)

    page.click(".tasks-add-btn:has-text('+ Task')")
    page.wait_for_selector(".add-todo-dialog", timeout=5000)
    page.fill(".add-todo-dialog input.form-input[type='text']", title)
    with page.expect_response(lambda r: "/api/create_todo_item" in r.url, timeout=10_000):
        page.click(".add-todo-dialog button[type='submit']")
    page.wait_for_timeout(500)

    card = page.locator(".focus-scored-card", has_text=title).first
    card.wait_for(timeout=8000)

    # Reopen via the ad-hoc arrow; the edit dialog must be prefilled.
    card.hover()
    card.locator(".fsc-action-btn[title='Edit or delete']").click()
    page.wait_for_selector(".add-todo-dialog", timeout=5000)
    assert page.input_value(".add-todo-dialog input.form-input[type='text']") == title, (
        "Edit dialog did not prefill the task title"
    )

    # One-click delete.
    with page.expect_response(lambda r: "/api/delete_todo_item" in r.url, timeout=10_000):
        page.click(".add-todo-dialog .btn-danger")
    page.wait_for_timeout(500)
    assert page.locator(".focus-scored-card", has_text=title).count() == 0, (
        "Deleted ad-hoc task is still shown in Focus"
    )

    panics = wasm_panics(errors)
    assert not panics, "WASM panic during ad-hoc task test:\n" + "\n".join(panics)


# ── Escape closes the topmost modal (backlog #123) ───────────────────────────

def test_escape_closes_settings_modal(server_url, auth_page):
    """Escape dismisses an open modal, reusing the same close path as ✕."""
    page, errors = auth_page
    load_and_settle(page, f"{server_url}/")

    page.click("button.settings-btn")
    page.wait_for_selector(".settings-dialog", timeout=5000)

    page.keyboard.press("Escape")
    page.wait_for_selector(".settings-dialog", state="detached", timeout=5000)

    panics = wasm_panics(errors)
    assert not panics, "WASM panic during Escape-close test:\n" + "\n".join(panics)


def test_escape_closes_topmost_nested_modal_only(server_url, auth_page):
    """Escape peels one layer at a time: the inline task authoring modal first,
    then the note editor — never both at once (backlog #123)."""
    page, errors = auth_page
    page.goto(f"{server_url}/notes", wait_until="load")
    page.wait_for_timeout(500)

    page.click("button.notes-pin-filter:has-text('New note')")
    page.wait_for_selector(".note-editor-textarea", timeout=5000)

    # Open the inline "+ Task" authoring modal (backlog #113).
    page.click("button.note-chip--new:has-text('+ Task')")
    task_dialog = page.locator(".dialog", has_text="New task")
    task_dialog.wait_for(timeout=5000)

    # First Escape closes only the task modal; the editor stays open.
    page.keyboard.press("Escape")
    task_dialog.wait_for(state="detached", timeout=5000)
    assert page.locator(".note-editor-textarea").is_visible(), (
        "first Escape closed the whole note editor instead of just the task modal"
    )

    # Second Escape closes the editor itself.
    page.keyboard.press("Escape")
    page.wait_for_selector(".note-editor-textarea", state="detached", timeout=5000)

    panics = wasm_panics(errors)
    assert not panics, "WASM panic during nested Escape-close test:\n" + "\n".join(panics)

