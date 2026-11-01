"""
End-to-end test for backlog #41 — calendar sharing over raw CalDAV.

Two local accounts on one tidl-server instance: an owner shares a calendar
with a grantee, and the grantee accesses it purely over the CalDAV protocol
(no tidl web UI involved), the same way an external client like DAVx5 or
iOS Calendar would.

Run via: pytest tests/test_caldav_sharing.py -v
"""

import subprocess
import sqlite3
import uuid

import pytest
import requests

from fixtures import TidlServer

OWNER = ("share_owner", "owner_pass")
GRANTEE = ("share_grantee", "grantee_pass")


@pytest.fixture(scope="module")
def sharing_server(tidl_binary):
    srv = TidlServer(tidl_binary, port=3071, username=OWNER[0], password=OWNER[1])
    srv.start()
    subprocess.run(
        [
            srv.binary, "create-admin",
            "--email", f"{GRANTEE[0]}@tidl.test",
            "--username", GRANTEE[0],
            "--display-name", GRANTEE[0],
            "--password", GRANTEE[1],
        ],
        env=srv._env(),
        check=True,
        capture_output=True,
    )
    yield srv
    srv.stop()


def _share_calendar_directly(srv: TidlServer, owner_username: str, calendar: str,
                              grantee_username: str, permission: str) -> int:
    """Insert a calendar_shares row directly via sqlite3, bypassing the
    email-invitation flow (which needs SMTP configured) — this test is
    about CalDAV consumption of an accepted grant, not the invite flow
    itself, which is already covered by backlog #13's own scope."""
    import sqlite3
    db_path = f"{srv._tmpdir}/tidl.db"
    con = sqlite3.connect(db_path)
    try:
        con.execute(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) "
            "VALUES (?, ?, ?, ?) "
            "ON CONFLICT(owner_username, calendar_name, grantee_username) "
            "DO UPDATE SET permission = excluded.permission",
            (owner_username, calendar, grantee_username, permission),
        )
        con.commit()
        return con.execute(
            "SELECT id FROM calendar_shares WHERE owner_username=? AND calendar_name=? AND grantee_username=?",
            (owner_username, calendar, grantee_username),
        ).fetchone()[0]
    finally:
        con.close()


def test_grantee_discovers_shared_calendar_via_propfind(sharing_server):
    srv = sharing_server
    # Owner's "personal" calendar is auto-created on first CalDAV touch.
    requests.request(
        "GET", f"{srv.url}/caldav/{OWNER[0]}/personal", auth=OWNER,
    )
    share_id = _share_calendar_directly(srv, OWNER[0], "personal", GRANTEE[0], "read")

    resp = requests.request(
        "PROPFIND", f"{srv.url}/caldav/{GRANTEE[0]}/",
        auth=GRANTEE,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data="<propfind xmlns='DAV:'><allprop/></propfind>",
    )
    assert resp.status_code == 207, resp.text
    assert f"shared-{share_id}" in resp.text, (
        f"Expected shared-{share_id} alias in grantee's home-set PROPFIND response"
    )


def test_grantee_can_read_write_shared_event(sharing_server):
    srv = sharing_server
    share_id = _share_calendar_directly(srv, OWNER[0], "personal", GRANTEE[0], "read-write")
    alias = f"shared-{share_id}"
    uid = str(uuid.uuid4())
    ical = (
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//test//EN\r\n"
        f"BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:Shared via grantee\r\n"
        "DTSTART:20260801T100000Z\r\nDTEND:20260801T110000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )

    # Grantee PUTs a new event through the alias.
    put_resp = requests.put(
        f"{srv.url}/caldav/{GRANTEE[0]}/{alias}/{uid}.ics",
        auth=GRANTEE,
        data=ical.encode(),
        headers={"Content-Type": "text/calendar; charset=utf-8"},
    )
    assert put_resp.status_code in (201, 204), put_resp.text

    # Owner sees it under their own real calendar name.
    get_resp = requests.get(
        f"{srv.url}/caldav/{OWNER[0]}/personal/{uid}.ics", auth=OWNER,
    )
    assert get_resp.status_code == 200, get_resp.text
    assert "Shared via grantee" in get_resp.text

    # Grantee deletes it back out through the alias.
    del_resp = requests.delete(
        f"{srv.url}/caldav/{GRANTEE[0]}/{alias}/{uid}.ics", auth=GRANTEE,
    )
    assert del_resp.status_code == 204, del_resp.text

    missing_resp = requests.get(
        f"{srv.url}/caldav/{OWNER[0]}/personal/{uid}.ics", auth=OWNER,
    )
    assert missing_resp.status_code == 404


def test_read_only_grantee_cannot_write(sharing_server):
    srv = sharing_server
    share_id = _share_calendar_directly(srv, OWNER[0], "personal", GRANTEE[0], "read")
    alias = f"shared-{share_id}"
    uid = str(uuid.uuid4())
    ical = (
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//test//EN\r\n"
        f"BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:Should be rejected\r\n"
        "DTSTART:20260801T100000Z\r\nDTEND:20260801T110000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )

    put_resp = requests.put(
        f"{srv.url}/caldav/{GRANTEE[0]}/{alias}/{uid}.ics",
        auth=GRANTEE,
        data=ical.encode(),
        headers={"Content-Type": "text/calendar; charset=utf-8"},
    )
    assert put_resp.status_code == 403, put_resp.text


def test_non_grantee_cannot_use_someone_elses_share_id(sharing_server):
    srv = sharing_server
    share_id = _share_calendar_directly(srv, OWNER[0], "personal", GRANTEE[0], "read")
    alias = f"shared-{share_id}"

    # A third, unrelated user tries the grantee's share-id directly.
    subprocess.run(
        [
            srv.binary, "create-admin",
            "--email", "eve@tidl.test", "--username", "eve",
            "--display-name", "eve", "--password", "eve_pass",
        ],
        env=srv._env(), check=True, capture_output=True,
    )
    resp = requests.request(
        "PROPFIND", f"{srv.url}/caldav/eve/{alias}/",
        auth=("eve", "eve_pass"),
        headers={"Depth": "1"},
    )
    assert resp.status_code == 403, resp.text


def _owner_token(srv: TidlServer, calendar: str) -> int:
    con = sqlite3.connect(f"{srv._tmpdir}/tidl.db")
    try:
        return con.execute(
            "SELECT sync_token FROM calendars WHERE username = ? AND name = ?",
            (OWNER[0], calendar),
        ).fetchone()[0]
    finally:
        con.close()


def _event(uid: str, summary: str) -> bytes:
    return (
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//test//EN\r\n"
        f"BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:{summary}\r\n"
        "DTSTART:20261008T150000Z\r\nDTEND:20261008T170000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    ).encode()


def test_grantee_write_via_alias_bumps_the_owners_token(sharing_server):
    srv = sharing_server
    requests.get(f"{srv.url}/caldav/{OWNER[0]}/personal", auth=OWNER)
    share_id = _share_calendar_directly(srv, OWNER[0], "personal", GRANTEE[0], "read-write")
    alias = f"shared-{share_id}"
    before = _owner_token(srv, "personal")

    uid = str(uuid.uuid4())
    put = requests.put(
        f"{srv.url}/caldav/{GRANTEE[0]}/{alias}/{uid}.ics",
        auth=GRANTEE,
        data=_event(uid, "grantee write bumps owner"),
        headers={"Content-Type": "text/calendar; charset=utf-8"},
    )
    assert put.status_code in (201, 204), put.text

    after = _owner_token(srv, "personal")
    assert after > before, "a grantee write must advance the owner's sync token"

    # The owner's own incremental sync must see the grantee's write.
    resp = requests.request(
        "REPORT",
        f"{srv.url}/caldav/{OWNER[0]}/personal/",
        auth=OWNER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data=(
            "<sync-collection xmlns='DAV:'>"
            f"<sync-token>urn:tidl:sync:{before}</sync-token>"
            "<prop><getetag/></prop></sync-collection>"
        ),
    )
    assert resp.status_code == 207, resp.text
    assert f"/{uid}.ics" in resp.text
    assert "HTTP/1.1 200 OK" in resp.text


def test_grantee_incremental_sync_uses_alias_hrefs(sharing_server):
    srv = sharing_server
    requests.get(f"{srv.url}/caldav/{OWNER[0]}/personal", auth=OWNER)
    share_id = _share_calendar_directly(srv, OWNER[0], "personal", GRANTEE[0], "read-write")
    alias = f"shared-{share_id}"
    alias_url = f"{srv.url}/caldav/{GRANTEE[0]}/{alias}/"

    # Baseline full sync through the alias, to capture the token to sync from.
    base = requests.request(
        "REPORT",
        alias_url,
        auth=GRANTEE,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data="<sync-collection xmlns='DAV:'><sync-token></sync-token>"
        "<prop><getetag/></prop></sync-collection>",
    )
    assert base.status_code == 207, base.text
    start = base.text.index("urn:tidl:sync:") + len("urn:tidl:sync:")
    since = base.text[start:].split("<")[0].strip()

    uid = str(uuid.uuid4())
    put = requests.put(
        f"{alias_url}{uid}.ics",
        auth=GRANTEE,
        data=_event(uid, "alias href check"),
        headers={"Content-Type": "text/calendar; charset=utf-8"},
    )
    assert put.status_code in (201, 204), put.text

    inc = requests.request(
        "REPORT",
        alias_url,
        auth=GRANTEE,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data=(
            "<sync-collection xmlns='DAV:'>"
            f"<sync-token>urn:tidl:sync:{since}</sync-token>"
            "<prop><getetag/></prop></sync-collection>"
        ),
    )
    assert inc.status_code == 207, inc.text
    assert f"/caldav/{GRANTEE[0]}/{alias}/{uid}.ics" in inc.text, (
        "the grantee must see hrefs under its own alias path, not the owner's"
    )
