"""
Fixtures for two-instance CalDAV sync E2E tests.

Usage:
    pytest tests/test_sync.py -v

Optional env vars:
    TIDL_BINARY   path to tidl-server binary  (default: target/debug/tidl-server)

Requires a built server binary with stable (hash-free) server function URLs:
    DISABLE_SERVER_FN_HASH=1 cargo build -p tidl-server
"""

import os
import shutil
import subprocess
import tempfile
import time
import uuid
from datetime import datetime, timedelta, timezone

import caldav
import pytest
import requests
from icalendar import Calendar as ICal


# ── Constants ─────────────────────────────────────────────────────────────────

# Server function URL prefix.  Leptos 0.7 uses /api/{snake_case_fn_name}.
# If the server returns 404, change to uppercase PascalCase.
_API = "/api"

# Fixed secret for test servers.
_TEST_SECRET = "tidl-e2e-test-secret"


# ── TidlServer ────────────────────────────────────────────────────────────────

class TidlServer:
    """Manages a single tidl-server subprocess with its own temp data directory."""

    def __init__(self, binary: str, port: int, username: str, password: str):
        self.binary = binary
        self.port = port
        self.username = username
        self.email = f"{username}@tidl.test"
        self.password = password
        self.url = f"http://localhost:{port}"
        self._tmpdir = None
        self._proc = None

    # ── Lifecycle ──────────────────────────────────────────────────────────────

    def start(self):
        self._tmpdir = tempfile.mkdtemp(prefix=f"tidl-{self.port}-")
        env = self._env()

        # Bootstrap: create admin user (also runs DB migrations).
        subprocess.run(
            [
                self.binary, "create-admin",
                "--email", self.email,
                "--username", self.username,
                "--display-name", self.username,
                "--password", self.password,
            ],
            env=env,
            check=True,
            capture_output=True,
        )

        # Start the server process.
        self._proc = subprocess.Popen(
            [self.binary, "serve"],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        self._wait_ready()

    def stop(self):
        if self._proc:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                self._proc.wait()
            self._proc = None
        if self._tmpdir:
            shutil.rmtree(self._tmpdir, ignore_errors=True)
            self._tmpdir = None

    def _env(self):
        return {
            **os.environ,
            "TIDL_DATA_DIR": self._tmpdir,
            "TIDL_PORT": str(self.port),
            "TIDL_SECRET": _TEST_SECRET,
            # These servers are reached over plain HTTP, so the session cookie
            # must not carry the `Secure` attribute: a conforming HTTP client
            # (e.g. `requests`) refuses to store or send a Secure cookie over
            # http://, which turns a successful `do_login` into a session the
            # follow-up call can't use. Mirrors a real plain-HTTP deployment
            # (the documented reason `server.secure_cookie` exists). Browsers
            # aren't affected because they treat localhost as a secure context.
            "TIDL_SECURE_COOKIE": "false",
            # The remote-sync tests point Sink at Source over `localhost`, which
            # the SSRF guard exists to block (backlog #141 finding 3). Opt out
            # for the test servers so two local instances can sync to each
            # other; never set this in production.
            "TIDL_SSRF_ALLOW_INTERNAL": "1",
        }

    def _wait_ready(self, timeout: float = 15.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                r = requests.get(self.url, timeout=1)
                if r.status_code < 500:
                    return
            except requests.exceptions.RequestException:
                pass
            time.sleep(0.1)
        raise RuntimeError(
            f"tidl-server on port {self.port} did not respond within {timeout}s"
        )

    # ── HTTP helpers ──────────────────────────────────────────────────────────

    def login_session(self) -> requests.Session:
        """Return a requests.Session with a verified, usable session cookie."""
        s = requests.Session()
        r = s.post(
            f"{self.url}{_API}/do_login",
            json={"email": self.email, "password": self.password},
            timeout=5,
        )
        assert r.status_code == 200, (
            f"Login to {self.url} failed: HTTP {r.status_code} — {r.text}"
        )
        # `do_login` returns 200 even if the cookie it sets is unusable (e.g. a
        # Secure cookie dropped over plain HTTP), so confirm the session really
        # authenticates before handing it back — otherwise the failure surfaces
        # misleadingly as "Not authenticated" on the next server-fn call.
        info = s.post(f"{self.url}{_API}/get_session_info", data={}, timeout=5)
        assert info.status_code == 200, (
            f"Login to {self.url} returned 200 but get_session_info failed: "
            f"HTTP {info.status_code} — {info.text}"
        )
        session_info = info.json()
        assert session_info, (
            f"Login to {self.url} returned 200 but established no session "
            f"(get_session_info → {session_info!r}); cookies={s.cookies.get_dict()}"
        )
        assert session_info.get("username") == self.username, (
            f"Login to {self.url} authenticated as {session_info.get('username')!r}, "
            f"expected {self.username!r}"
        )
        return s

    def api_post_json(self, fn_name: str, payload: dict) -> dict:
        """POST to a JSON-codec server function. Returns the decoded response body."""
        s = self.login_session()
        r = s.post(
            f"{self.url}{_API}/{fn_name}",
            json=payload,
            timeout=10,
        )
        assert r.status_code == 200, (
            f"Server fn {fn_name} returned HTTP {r.status_code}: {r.text}"
        )
        return r.json()

    def api_post_form(self, fn_name: str, data: dict) -> requests.Response:
        """POST to a form-encoded (PostUrl) server function."""
        s = self.login_session()
        r = s.post(
            f"{self.url}{_API}/{fn_name}",
            data=data,
            timeout=10,
        )
        assert r.status_code == 200, (
            f"Server fn {fn_name} returned HTTP {r.status_code}: {r.text}"
        )
        return r

    # ── Domain helpers ────────────────────────────────────────────────────────

    def create_app_token(self, name: str, read_only: bool = False) -> str:
        """Create an app token. Returns the raw token string."""
        return self.api_post_json("create_app_token", {"name": name, "read_only": read_only})

    def list_remote_sources(self) -> list:
        """Return the list of configured remote CalDAV sources.
        list_remote_sources uses default #[server] (PostUrl codec) with no params.
        """
        r = self.login_session().post(
            f"{self.url}{_API}/list_remote_sources",
            data={},
            timeout=10,
        )
        assert r.status_code == 200, f"list_remote_sources: {r.status_code} {r.text}"
        return r.json()

    def add_remote_source(
        self,
        name: str,
        remote_base_url: str,
        remote_username: str,
        remote_token: str,
        remote_calendar: str,
        local_calendar: str,
        direction: str,
    ) -> str:
        """Add a remote CalDAV source. Returns the source ID."""
        self.api_post_json(
            "add_remote_source",
            {
                "source": {
                    "name": name,
                    "remote_base_url": remote_base_url,
                    "remote_username": remote_username,
                    "remote_token": remote_token,
                    "remote_calendar": remote_calendar,
                    "local_calendar": local_calendar,
                    "sync_direction": direction,
                }
            },
        )
        # The ID is not returned by add; fetch it from the list.
        sources = self.list_remote_sources()
        for s in sources:
            if s["name"] == name and s["local_calendar"] == local_calendar:
                return s["id"]
        raise RuntimeError(f"Remote source '{name}' not found after add")

    def delete_remote_source(self, source_id: str):
        # delete_remote_source uses #[server(input = Json)]
        self.api_post_json("delete_remote_source", {"source_id": source_id})

    def trigger_sync_api(self, source_id: str):
        """Trigger sync via the server function (PostUrl codec, form-encoded)."""
        self.api_post_form("trigger_remote_caldav_sync", {"source_id": source_id})

    def caldav_client(self) -> caldav.DAVClient:
        """Return a caldav.DAVClient for this server."""
        principal_url = f"{self.url}/caldav/{self.username}/"
        return caldav.DAVClient(
            url=principal_url,
            username=self.username,
            password=self.password,
        )


# ── Session-scoped server fixtures ────────────────────────────────────────────

@pytest.fixture(scope="session")
def tidl_binary() -> str:
    binary = os.getenv("TIDL_BINARY", "target/debug/tidl-server")
    assert os.path.isfile(binary), (
        f"tidl-server binary not found at '{binary}'. "
        "Run: DISABLE_SERVER_FN_HASH=1 cargo build -p tidl-server  "
        "(or set TIDL_BINARY=/path/to/tidl-server)"
    )
    return binary


@pytest.fixture(scope="session")
def source_server(tidl_binary) -> TidlServer:
    """Source tidl instance on port 3001."""
    srv = TidlServer(tidl_binary, port=3001, username="source_user", password="source_pass")
    srv.start()
    yield srv
    srv.stop()


@pytest.fixture(scope="session")
def sink_server(tidl_binary) -> TidlServer:
    """Sink tidl instance on port 3002."""
    srv = TidlServer(tidl_binary, port=3002, username="sink_user", password="sink_pass")
    srv.start()
    yield srv
    srv.stop()


# ── App token fixture ─────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def source_app_token(source_server) -> str:
    """Create a read-write app token on Source for Sink to authenticate with (push/both sync requires write access)."""
    return source_server.create_app_token("e2e-sync-token", read_only=False)


# ── Calendar fixtures ─────────────────────────────────────────────────────────

def _make_calendar(server: TidlServer, slug: str):
    """Create a calendar on the given server via MKCOL. Returns a caldav.Calendar."""
    url = f"{server.url}/caldav/{server.username}/{slug}/"
    r = requests.request("MKCOL", url, auth=(server.username, server.password))
    assert r.status_code in (201, 204), f"MKCOL {url} failed: {r.status_code} {r.text}"
    client = server.caldav_client()
    return caldav.Calendar(client=client, url=url)


def _delete_calendar(cal: caldav.Calendar):
    try:
        cal.delete()
    except Exception:
        pass


@pytest.fixture
def source_cal(source_server) -> caldav.Calendar:
    """Fresh source calendar, deleted after each test."""
    slug = f"src-{uuid.uuid4().hex[:8]}"
    cal = _make_calendar(source_server, slug)
    yield cal
    _delete_calendar(cal)


@pytest.fixture
def sink_cal(sink_server) -> caldav.Calendar:
    """Fresh sink calendar, deleted after each test."""
    slug = f"snk-{uuid.uuid4().hex[:8]}"
    cal = _make_calendar(sink_server, slug)
    yield cal
    _delete_calendar(cal)


# ── Remote source fixture ─────────────────────────────────────────────────────

def _calendar_slug(cal: caldav.Calendar) -> str:
    """Extract the slug (last non-empty path component) from a calendar URL."""
    return cal.url.path.rstrip("/").split("/")[-1]


@pytest.fixture
def pull_source(sink_server, source_server, source_app_token, source_cal, sink_cal) -> str:
    """Register a pull-direction remote source on Sink pointing at source_cal.

    Returns the remote source ID. Deletes the source on teardown.
    The add_remote_source call also triggers an immediate (empty) initial sync.
    """
    src_slug = _calendar_slug(source_cal)
    snk_slug = _calendar_slug(sink_cal)
    source_id = sink_server.add_remote_source(
        name=f"e2e-pull-{snk_slug}",
        remote_base_url=source_server.url,
        remote_username=source_server.username,
        remote_token=source_app_token,
        remote_calendar=src_slug,
        local_calendar=snk_slug,
        direction="pull",
    )
    yield source_id
    sink_server.delete_remote_source(source_id)


@pytest.fixture
def push_source(sink_server, source_server, source_app_token, source_cal, sink_cal) -> str:
    """Register a push-direction remote source on Sink pointing at source_cal."""
    src_slug = _calendar_slug(source_cal)
    snk_slug = _calendar_slug(sink_cal)
    source_id = sink_server.add_remote_source(
        name=f"e2e-push-{snk_slug}",
        remote_base_url=source_server.url,
        remote_username=source_server.username,
        remote_token=source_app_token,
        remote_calendar=src_slug,
        local_calendar=snk_slug,
        direction="push",
    )
    yield source_id
    sink_server.delete_remote_source(source_id)


@pytest.fixture
def both_source(sink_server, source_server, source_app_token, source_cal, sink_cal) -> str:
    """Register a both-direction remote source on Sink pointing at source_cal."""
    src_slug = _calendar_slug(source_cal)
    snk_slug = _calendar_slug(sink_cal)
    source_id = sink_server.add_remote_source(
        name=f"e2e-both-{snk_slug}",
        remote_base_url=source_server.url,
        remote_username=source_server.username,
        remote_token=source_app_token,
        remote_calendar=src_slug,
        local_calendar=snk_slug,
        direction="both",
    )
    yield source_id
    sink_server.delete_remote_source(source_id)


# ── ICS helpers ───────────────────────────────────────────────────────────────

def make_vevent(
    uid: str = None,
    summary: str = "E2E Test Event",
    dtstart: datetime = None,
    dtend: datetime = None,
    description: str = None,
    location: str = None,
    partstat: str = None,         # e.g. "ACCEPTED", "TENTATIVE", "DECLINED"
    rrule: str = None,            # e.g. "FREQ=WEEKLY;COUNT=4"
    exdate: str = None,           # e.g. "20260601T100000Z"
    valarm_minutes: int = None,   # minutes before event for VALARM
    all_day: bool = False,
) -> tuple[str, str]:
    """Return (ical_str, uid) for a VEVENT."""
    if uid is None:
        uid = str(uuid.uuid4())
    if dtstart is None:
        dtstart = datetime.now(timezone.utc).replace(microsecond=0)
    if dtend is None:
        dtend = dtstart + timedelta(hours=1)

    if all_day:
        start_str = f"DTSTART;VALUE=DATE:{dtstart.strftime('%Y%m%d')}"
        end_str = f"DTEND;VALUE=DATE:{dtend.strftime('%Y%m%d')}"
    else:
        start_str = f"DTSTART:{dtstart.strftime('%Y%m%dT%H%M%SZ')}"
        end_str = f"DTEND:{dtend.strftime('%Y%m%dT%H%M%SZ')}"

    lines = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//tidl//e2e-test//EN",
        "BEGIN:VEVENT",
        f"UID:{uid}",
        f"SUMMARY:{summary}",
        start_str,
        end_str,
    ]
    if description:
        lines.append(f"DESCRIPTION:{description}")
    if location:
        lines.append(f"LOCATION:{location}")
    if partstat:
        lines.append(
            f"ATTENDEE;PARTSTAT={partstat}:mailto:test@tidl.test"
        )
    if rrule:
        lines.append(f"RRULE:{rrule}")
    if exdate:
        lines.append(f"EXDATE:{exdate}")
    if valarm_minutes is not None:
        lines += [
            "BEGIN:VALARM",
            "ACTION:DISPLAY",
            "DESCRIPTION:Reminder",
            f"TRIGGER:-PT{valarm_minutes}M",
            "END:VALARM",
        ]
    lines += ["END:VEVENT", "END:VCALENDAR"]
    return "\r\n".join(lines) + "\r\n", uid


def make_vtodo(
    uid: str = None,
    summary: str = "E2E Test Todo",
    due: datetime = None,
    priority: int = None,
) -> tuple[str, str]:
    """Return (ical_str, uid) for a VTODO."""
    if uid is None:
        uid = str(uuid.uuid4())
    if due is None:
        due = datetime.now(timezone.utc).replace(microsecond=0) + timedelta(days=1)

    lines = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//tidl//e2e-test//EN",
        "BEGIN:VTODO",
        f"UID:{uid}",
        f"SUMMARY:{summary}",
        f"DUE:{due.strftime('%Y%m%dT%H%M%SZ')}",
    ]
    if priority is not None:
        lines.append(f"PRIORITY:{priority}")
    lines += ["END:VTODO", "END:VCALENDAR"]
    return "\r\n".join(lines) + "\r\n", uid


def put_event(calendar: caldav.Calendar, ical_str: str, uid: str):
    """PUT an iCal string into a calendar."""
    url = f"{str(calendar.url).rstrip('/')}/{uid}.ics"
    r = requests.put(
        url,
        data=ical_str.encode(),
        headers={"Content-Type": "text/calendar; charset=utf-8"},
        auth=(
            calendar.client.username,
            calendar.client.password,
        ),
    )
    assert r.status_code in (201, 204), f"PUT event failed: {r.status_code} {r.text}"


def get_event_data(calendar: caldav.Calendar, uid: str) -> str:
    """Fetch the raw iCal data for an event by UID."""
    event = calendar.event_by_uid(uid)
    return event.data if isinstance(event.data, str) else event.data.decode()


def ical_prop(ical_str: str, component: str, prop: str) -> str:
    """Extract a property value from the first matching component."""
    cal = ICal.from_ical(ical_str)
    for comp in cal.walk():
        if comp.name == component:
            val = comp.get(prop)
            if val is not None:
                return str(val)
    return ""


# ── Wait helpers ──────────────────────────────────────────────────────────────

def wait_for_uid(
    calendar: caldav.Calendar,
    uid: str,
    timeout: float = 8.0,
    interval: float = 0.25,
):
    """Poll until an event with the given UID appears in the calendar."""
    deadline = time.time() + timeout
    last_exc = None
    while time.time() < deadline:
        try:
            calendar.event_by_uid(uid)
            return  # Found
        except Exception as e:
            last_exc = e
            time.sleep(interval)
    raise TimeoutError(
        f"Event UID={uid!r} not found in {calendar.url} within {timeout}s. "
        f"Last error: {last_exc}"
    )


def wait_for_uid_absent(
    calendar: caldav.Calendar,
    uid: str,
    timeout: float = 8.0,
    interval: float = 0.25,
):
    """Poll until an event with the given UID is gone from the calendar."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            calendar.event_by_uid(uid)
            time.sleep(interval)
        except Exception:
            return  # Gone
    raise TimeoutError(
        f"Event UID={uid!r} still present in {calendar.url} after {timeout}s"
    )


# ── Playwright fixtures ───────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def sync_browser(browser):
    """Session-scoped headless Chromium browser for sink server.

    Delegates to the shared conftest browser fixture so only one
    sync_playwright() context runs per process.
    """
    return browser


@pytest.fixture(scope="session")
def sink_page(sync_browser, sink_server):
    """Logged-in Playwright page for the sink server, reused across tests."""
    ctx = sync_browser.new_context()
    page = ctx.new_page()

    # Wait for WASM to hydrate.
    wasm_hydrated: list[bool] = []
    page.on(
        "console",
        lambda m: wasm_hydrated.append(True) if "[tidl] WASM hydrate" in m.text else None,
    )
    page.goto(sink_server.url, wait_until="load")
    for _ in range(100):
        if wasm_hydrated:
            break
        page.wait_for_timeout(100)
    else:
        raise RuntimeError("Sink WASM did not hydrate within 10s")

    page.wait_for_timeout(200)

    # Log in.
    page.fill("input[type='email']", sink_server.email)
    page.fill("input[type='password']", sink_server.password)
    with page.expect_response(lambda r: "/api/do_login" in r.url, timeout=10_000) as ri:
        page.click("button[type='submit']")
    assert ri.value.status == 200, f"Login failed: {ri.value.status}"
    page.wait_for_load_state("load")

    yield page
    ctx.close()


def trigger_sync(page, sink_cal: caldav.Calendar):
    """Click the sync button for the given calendar in the Sink sidebar.

    The sidebar row is the shared `SidebarPill` (`.sidebar-pill`, inside a
    `.fs-list-item`); its sync button is an always-visible icon button titled
    "Sync now" (backlog #115 replaced the old `.cal-pill` / `.cal-list-item`
    markup).  We locate the pill by the calendar slug (also its display name
    until explicitly renamed).
    """
    slug = sink_cal.url.path.rstrip("/").split("/")[-1]

    # Reload the page so the sidebar reflects the newly added remote source.
    # Use "load" (not "networkidle") because the SSE /api/stream connection
    # keeps the page from ever reaching the networkidle state.
    page.reload(wait_until="load")
    # Wait for the sidebar calendar list to be populated (WASM async load).
    page.wait_for_selector(".sidebar-pill", timeout=10_000)
    page.wait_for_timeout(300)

    # Hover over the pill (its icon buttons are always visible, but hovering
    # matches the real user gesture) and click this pill's sync button.
    pill = page.locator(".sidebar-pill").filter(has_text=slug).first
    pill.hover()

    with page.expect_response(
        lambda r: "trigger_remote_caldav_sync" in r.url, timeout=10_000
    ):
        pill.locator("button.sidebar-pill-icon-btn[title='Sync now']").click()

    # The sync runs in the background; give it a moment to start.
    page.wait_for_timeout(200)
