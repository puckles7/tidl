"""
End-to-end tests for CalDAV sync-token semantics (RFC 6578 sync-collection)
over raw HTTP — the behaviour CalDAV clients like iOS Calendar depend on.

These are deliberately protocol-level (not via the web UI) and read the
server's SQLite DB directly to set up states a client can't produce on its
own (e.g. a stale tombstone), then assert what the server reports back.

Run via: pytest tests/test_caldav_sync.py -v
"""

import sqlite3
import uuid

import pytest
import requests

from fixtures import TidlServer

USER = ("sync_owner", "sync_pass")
CAL = "personal"

SYNC_COLLECTION = (
    "<sync-collection xmlns='DAV:' xmlns:C='urn:ietf:params:xml:ns:caldav'>"
    "{token}<prop><getetag/></prop></sync-collection>"
)


@pytest.fixture(scope="module")
def sync_server(tidl_binary):
    srv = TidlServer(tidl_binary, port=3073, username=USER[0], password=USER[1])
    srv.start()
    yield srv
    srv.stop()


def _db(srv: TidlServer) -> sqlite3.Connection:
    return sqlite3.connect(f"{srv._tmpdir}/tidl.db")


def _cal_url(srv: TidlServer, calendar: str = CAL) -> str:
    return f"{srv.url}/caldav/{USER[0]}/{calendar}/"


def _ensure_calendar(srv: TidlServer, calendar: str = CAL) -> None:
    # GET on the collection auto-creates it (sync_token starts at 1).
    r = requests.get(_cal_url(srv, calendar), auth=USER)
    assert r.status_code == 200, r.text


def _put(srv: TidlServer, uid: str, summary: str, calendar: str = CAL) -> None:
    ical = (
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl tests//EN\r\n"
        f"BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:{summary}\r\n"
        "DTSTART:20261008T150000Z\r\nDTEND:20261008T170000Z\r\n"
        "END:VEVENT\r\nEND:VCALENDAR\r\n"
    )
    r = requests.put(
        f"{_cal_url(srv, calendar)}{uid}.ics",
        auth=USER,
        data=ical.encode(),
        headers={"Content-Type": "text/calendar; charset=utf-8"},
    )
    assert r.status_code in (201, 204), r.text


def _delete(srv: TidlServer, uid: str, calendar: str = CAL) -> None:
    r = requests.delete(f"{_cal_url(srv, calendar)}{uid}.ics", auth=USER)
    assert r.status_code in (204, 404), r.text


def _report(srv: TidlServer, token: str | None, calendar: str = CAL) -> requests.Response:
    token_el = "<sync-token></sync-token>" if token is None else f"<sync-token>{token}</sync-token>"
    return requests.request(
        "REPORT",
        _cal_url(srv, calendar),
        auth=USER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data=SYNC_COLLECTION.format(token=token_el),
    )


def _token(srv: TidlServer, calendar: str = CAL) -> int:
    with _db(srv) as con:
        return con.execute(
            "SELECT sync_token FROM calendars WHERE username = ? AND name = ?",
            (USER[0], calendar),
        ).fetchone()[0]


def _bump_and_log(srv: TidlServer, uid: str, operation: str, calendar: str = CAL) -> int:
    """Force the calendar's token forward by one and log `operation` for `uid`
    at that new token — simulating a log entry the server API wouldn't emit
    for the current DB state."""
    with _db(srv) as con:
        con.execute(
            "UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?",
            (USER[0], calendar),
        )
        token = con.execute(
            "SELECT sync_token FROM calendars WHERE username = ? AND name = ?",
            (USER[0], calendar),
        ).fetchone()[0]
        con.execute(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) VALUES (?, ?, ?, ?, ?)",
            (USER[0], calendar, uid, operation, token),
        )
    return token


# ── full sync ────────────────────────────────────────────────────────────────


def test_full_sync_returns_all_events_and_a_token(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    uid = str(uuid.uuid4())
    _put(srv, uid, "Full sync event")

    resp = _report(srv, token=None)
    assert resp.status_code == 207, resp.text
    assert f"/{uid}.ics" in resp.text
    assert "HTTP/1.1 200 OK" in resp.text
    assert f"urn:tidl:sync:{_token(srv)}" in resp.text


# ── incremental ──────────────────────────────────────────────────────────────


def test_incremental_sync_reports_created_updated_deleted(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    base = str(uuid.uuid4())
    _put(srv, base, "baseline")
    baseline = _token(srv)

    created = str(uuid.uuid4())
    _put(srv, created, "created")
    _put(srv, base, "baseline edited")
    _delete(srv, created)

    resp = _report(srv, token=f"urn:tidl:sync:{baseline}")
    assert resp.status_code == 207, resp.text
    # Updated event present, 200.
    assert f"/{base}.ics" in resp.text
    assert "HTTP/1.1 200 OK" in resp.text
    # Created-then-deleted event reported as gone.
    assert f"/{created}.ics" in resp.text
    assert "HTTP/1.1 404 Not Found" in resp.text


def test_incremental_sync_excludes_changes_at_or_before_since(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    old = str(uuid.uuid4())
    _put(srv, old, "before baseline")
    baseline = _token(srv)
    new = str(uuid.uuid4())
    _put(srv, new, "after baseline")

    resp = _report(srv, token=f"urn:tidl:sync:{baseline}")
    assert resp.status_code == 207, resp.text
    assert f"/{new}.ics" in resp.text
    assert f"/{old}.ics" not in resp.text


# ── stale tombstones / absent rows (the authoritative-lookup behaviour) ──────


def test_stale_deleted_tombstone_for_present_event_is_reported_200(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    uid = str(uuid.uuid4())
    _put(srv, uid, "still here")
    after_put = _token(srv)

    # The log's latest op for this uid is `deleted`, but the event row exists.
    _bump_and_log(srv, uid, "deleted")

    resp = _report(srv, token=f"urn:tidl:sync:{after_put}")
    assert resp.status_code == 207, resp.text
    assert f"/{uid}.ics" in resp.text
    assert "HTTP/1.1 200 OK" in resp.text, (
        "a present event must be reported 200 even when the log says deleted"
    )
    assert "HTTP/1.1 404 Not Found" not in resp.text


def test_modified_log_entry_for_absent_event_is_reported_404(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    after_create = _token(srv)

    ghost = str(uuid.uuid4())
    _bump_and_log(srv, ghost, "modified")

    resp = _report(srv, token=f"urn:tidl:sync:{after_create}")
    assert resp.status_code == 207, resp.text
    assert f"/{ghost}.ics" in resp.text
    assert "HTTP/1.1 404 Not Found" in resp.text


# ── token parsing / fallback ─────────────────────────────────────────────────


def test_unparseable_sync_token_falls_back_to_full_sync(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    uid = str(uuid.uuid4())
    _put(srv, uid, "fallback")

    resp = _report(srv, token="not-a-token")
    assert resp.status_code == 207, resp.text
    assert f"/{uid}.ics" in resp.text
    assert f"urn:tidl:sync:{_token(srv)}" in resp.text


# ── PROPFIND advertisement ───────────────────────────────────────────────────


def test_propfind_advertises_getctag_and_sync_token(sync_server):
    srv = sync_server
    _ensure_calendar(srv)
    uid = str(uuid.uuid4())
    _put(srv, uid, "ctag subject")
    token = _token(srv)

    body = (
        "<propfind xmlns='DAV:' xmlns:C='urn:ietf:params:xml:ns:caldav' "
        "xmlns:CS='http://calendarserver.org/ns/'>"
        "<prop><D:sync-token xmlns:D='DAV:'/><CS:getctag/></prop></propfind>"
    )
    resp = requests.request(
        "PROPFIND", _cal_url(srv), auth=USER, headers={"Depth": "0"}, data=body
    )
    assert resp.status_code == 207, resp.text
    assert f"<D:sync-token>urn:tidl:sync:{token}</D:sync-token>" in resp.text
    assert f'<CS:getctag>"{token}"</CS:getctag>' in resp.text, resp.text

    # A write must advance both.
    _put(srv, str(uuid.uuid4()), "advance")
    token2 = _token(srv)
    assert token2 > token
    resp2 = requests.request(
        "PROPFIND", _cal_url(srv), auth=USER, headers={"Depth": "0"}, data=body
    )
    assert f"<D:sync-token>urn:tidl:sync:{token2}</D:sync-token>" in resp2.text
