"""
End-to-end test for backlog #81 — contact-collection sharing over raw
CardDAV.

Two local accounts on one tidl-server instance: an owner's contact
collection is shared with a member, and the member accesses it purely
over the CardDAV protocol (no tidl web UI involved), the same way an
external client like DAVx5 or iOS Contacts would. Mirrors
test_caldav_sharing.py's shape for the CardDAV side of this backlog item.

Run via: pytest tests/test_carddav_sharing.py -v
"""

import sqlite3
import subprocess
import uuid

import pytest
import requests

from fixtures import TidlServer

OWNER = ("book_owner", "owner_pass")
MEMBER = ("book_member", "member_pass")


@pytest.fixture(scope="module")
def carddav_server(tidl_binary):
    srv = TidlServer(tidl_binary, port=3072, username=OWNER[0], password=OWNER[1])
    srv.start()
    subprocess.run(
        [
            srv.binary, "create-admin",
            "--email", f"{MEMBER[0]}@tidl.test",
            "--username", MEMBER[0],
            "--display-name", MEMBER[0],
            "--password", MEMBER[1],
        ],
        env=srv._env(),
        check=True,
        capture_output=True,
    )
    yield srv
    srv.stop()


def _db(srv: TidlServer) -> sqlite3.Connection:
    return sqlite3.connect(f"{srv._tmpdir}/tidl.db")


def _default_collection_id(srv: TidlServer, username: str) -> int:
    """The owner's default "My Contacts" collection id, lazily provisioned
    by carddav_home's Depth:1 PROPFIND (mirrors how test_caldav_sharing.py's
    tests first GET a calendar to auto-create it) — never created directly
    via sqlite3, so this exercises the real lazy-provisioning path."""
    resp = requests.request(
        "PROPFIND", f"{srv.url}/carddav/{username}/",
        auth=(username, OWNER[1] if username == OWNER[0] else MEMBER[1]),
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data="<propfind xmlns='DAV:'><allprop/></propfind>",
    )
    assert resp.status_code == 207, resp.text
    con = _db(srv)
    try:
        row = con.execute(
            "SELECT id FROM contact_collections WHERE owner_username=? AND name='My Contacts'",
            (username,),
        ).fetchone()
        assert row is not None, f"expected a default collection to exist for {username}"
        return row[0]
    finally:
        con.close()


def _grant_membership(srv: TidlServer, collection_id: int, member_username: str) -> None:
    """Insert a contact_collection_members row directly via sqlite3,
    bypassing the email-invitation flow (which needs SMTP configured) —
    this test is about CardDAV consumption of an accepted grant, not the
    invite flow itself, which is already covered by backlog #64/#81's own
    web-UI scope (same rationale as test_caldav_sharing.py's own bypass)."""
    con = _db(srv)
    try:
        con.execute(
            "INSERT OR IGNORE INTO contact_collection_members (collection_id, member_username) VALUES (?, ?)",
            (collection_id, member_username),
        )
        con.execute(
            "UPDATE contact_collections SET shared = 1 WHERE id = ?",
            (collection_id,),
        )
        con.commit()
    finally:
        con.close()


def _vcard(uid: str, fn: str) -> bytes:
    return (
        f"BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nFN:{fn}\r\nEND:VCARD\r\n"
    ).encode()


def test_addressbook_home_set_lists_own_and_shared_books(carddav_server):
    srv = carddav_server
    owner_default = _default_collection_id(srv, OWNER[0])
    _grant_membership(srv, owner_default, MEMBER[0])

    resp = requests.request(
        "PROPFIND", f"{srv.url}/carddav/{MEMBER[0]}/",
        auth=MEMBER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data="<propfind xmlns='DAV:'><allprop/></propfind>",
    )
    assert resp.status_code == 207, resp.text
    assert f"book-{owner_default}" in resp.text, (
        f"Expected book-{owner_default} (the shared collection) in the member's own home-set PROPFIND"
    )

    # The member's own default collection is listed too, under the same
    # book-{id} scheme.
    member_default = _default_collection_id(srv, MEMBER[0])
    assert member_default != owner_default
    resp2 = requests.request(
        "PROPFIND", f"{srv.url}/carddav/{MEMBER[0]}/",
        auth=MEMBER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data="<propfind xmlns='DAV:'><allprop/></propfind>",
    )
    assert f"book-{member_default}" in resp2.text


def test_put_get_delete_round_trip_as_the_owner(carddav_server):
    srv = carddav_server
    collection_id = _default_collection_id(srv, OWNER[0])
    book = f"book-{collection_id}"
    uid = str(uuid.uuid4())

    put_resp = requests.put(
        f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf",
        auth=OWNER,
        data=_vcard(uid, "Owner Round Trip"),
        headers={"Content-Type": "text/vcard; charset=utf-8"},
    )
    assert put_resp.status_code in (201, 204), put_resp.text

    get_resp = requests.get(f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf", auth=OWNER)
    assert get_resp.status_code == 200, get_resp.text
    assert "Owner Round Trip" in get_resp.text

    del_resp = requests.delete(f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf", auth=OWNER)
    assert del_resp.status_code == 204, del_resp.text

    missing_resp = requests.get(f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf", auth=OWNER)
    assert missing_resp.status_code == 404


def test_put_get_delete_round_trip_as_a_member(carddav_server):
    srv = carddav_server
    collection_id = _default_collection_id(srv, OWNER[0])
    _grant_membership(srv, collection_id, MEMBER[0])
    book = f"book-{collection_id}"
    uid = str(uuid.uuid4())

    # The member gets full read/write on a shared collection — single
    # 'member' permission tier, no read-only split (backlog #81).
    put_resp = requests.put(
        f"{srv.url}/carddav/{MEMBER[0]}/{book}/{uid}.vcf",
        auth=MEMBER,
        data=_vcard(uid, "Member Round Trip"),
        headers={"Content-Type": "text/vcard; charset=utf-8"},
    )
    assert put_resp.status_code in (201, 204), put_resp.text

    # The owner sees the member's contact under their own path for the
    # same collection.
    owner_get = requests.get(f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf", auth=OWNER)
    assert owner_get.status_code == 200, owner_get.text
    assert "Member Round Trip" in owner_get.text

    del_resp = requests.delete(f"{srv.url}/carddav/{MEMBER[0]}/{book}/{uid}.vcf", auth=MEMBER)
    assert del_resp.status_code == 204, del_resp.text

    missing_resp = requests.get(f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf", auth=OWNER)
    assert missing_resp.status_code == 404


def test_a_non_member_is_rejected(carddav_server):
    srv = carddav_server
    collection_id = _default_collection_id(srv, OWNER[0])
    book = f"book-{collection_id}"

    subprocess.run(
        [
            srv.binary, "create-admin",
            "--email", "eve@tidl.test", "--username", "eve",
            "--display-name", "eve", "--password", "eve_pass",
        ],
        env=srv._env(), check=True, capture_output=True,
    )

    propfind_resp = requests.request(
        "PROPFIND", f"{srv.url}/carddav/eve/{book}/",
        auth=("eve", "eve_pass"),
        headers={"Depth": "1"},
    )
    assert propfind_resp.status_code == 403, propfind_resp.text

    uid = str(uuid.uuid4())
    put_resp = requests.put(
        f"{srv.url}/carddav/eve/{book}/{uid}.vcf",
        auth=("eve", "eve_pass"),
        data=_vcard(uid, "Should be rejected"),
        headers={"Content-Type": "text/vcard; charset=utf-8"},
    )
    assert put_resp.status_code == 403, put_resp.text


def test_an_unknown_book_alias_is_404(carddav_server):
    srv = carddav_server
    resp = requests.request(
        "PROPFIND", f"{srv.url}/carddav/{OWNER[0]}/book-999999/",
        auth=OWNER,
        headers={"Depth": "1"},
    )
    assert resp.status_code == 404, resp.text


def test_sync_collection_reports_changes_since_a_token(carddav_server):
    srv = carddav_server
    collection_id = _default_collection_id(srv, OWNER[0])
    book = f"book-{collection_id}"

    def put(uid: str, fn: str):
        r = requests.put(
            f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf",
            auth=OWNER,
            data=_vcard(uid, fn),
            headers={"Content-Type": "text/vcard; charset=utf-8"},
        )
        assert r.status_code in (201, 204), r.text

    first_uid = str(uuid.uuid4())
    second_uid = str(uuid.uuid4())
    put(first_uid, "First")
    put(second_uid, "Second")

    # Establish a baseline sync-token via an initial sync-collection REPORT
    # (no sync-token element = "give me everything plus the current token").
    initial = requests.request(
        "REPORT", f"{srv.url}/carddav/{OWNER[0]}/{book}/",
        auth=OWNER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data=(
            "<sync-collection xmlns='DAV:' xmlns:card='urn:ietf:params:xml:ns:carddav'>"
            "<sync-token></sync-token><prop><getetag/></prop></sync-collection>"
        ),
    )
    assert initial.status_code == 207, initial.text
    start = initial.text.index("urn:tidl:sync:") + len("urn:tidl:sync:")
    since_token = initial.text[start:].split("<")[0].strip()

    third_uid = str(uuid.uuid4())
    put(third_uid, "Third")
    del_resp = requests.delete(f"{srv.url}/carddav/{OWNER[0]}/{book}/{first_uid}.vcf", auth=OWNER)
    assert del_resp.status_code == 204, del_resp.text

    follow_up = requests.request(
        "REPORT", f"{srv.url}/carddav/{OWNER[0]}/{book}/",
        auth=OWNER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data=(
            "<sync-collection xmlns='DAV:' xmlns:card='urn:ietf:params:xml:ns:carddav'>"
            f"<sync-token>urn:tidl:sync:{since_token}</sync-token><prop><getetag/></prop></sync-collection>"
        ),
    )
    assert follow_up.status_code == 207, follow_up.text
    assert f"{third_uid}.vcf" in follow_up.text, "the newly-created contact must be reported"
    assert f"{first_uid}.vcf" in follow_up.text, "the deleted contact must be reported"
    assert f"{second_uid}.vcf" not in follow_up.text, (
        "an unchanged contact from before the sync token must not be reported"
    )


def test_stale_deleted_tombstone_for_present_contact_is_reported_200(carddav_server):
    srv = carddav_server
    collection_id = _default_collection_id(srv, OWNER[0])
    book = f"book-{collection_id}"
    uid = str(uuid.uuid4())

    put = requests.put(
        f"{srv.url}/carddav/{OWNER[0]}/{book}/{uid}.vcf",
        auth=OWNER,
        data=_vcard(uid, "Still Here"),
        headers={"Content-Type": "text/vcard; charset=utf-8"},
    )
    assert put.status_code in (201, 204), put.text

    con = _db(srv)
    try:
        baseline = con.execute(
            "SELECT sync_token FROM address_books WHERE collection_id = ?",
            (collection_id,),
        ).fetchone()[0]
        # Stale tombstone for a contact that still exists, at a higher token.
        con.execute(
            "UPDATE address_books SET sync_token = sync_token + 1 WHERE collection_id = ?",
            (collection_id,),
        )
        stranded = con.execute(
            "SELECT sync_token FROM address_books WHERE collection_id = ?",
            (collection_id,),
        ).fetchone()[0]
        con.execute(
            "INSERT INTO contact_sync_log (collection_id, uid, operation, token) VALUES (?, ?, 'deleted', ?)",
            (collection_id, uid, stranded),
        )
        con.commit()
    finally:
        con.close()

    resp = requests.request(
        "REPORT",
        f"{srv.url}/carddav/{OWNER[0]}/{book}/",
        auth=OWNER,
        headers={"Depth": "1", "Content-Type": "application/xml"},
        data=(
            "<sync-collection xmlns='DAV:' xmlns:card='urn:ietf:params:xml:ns:carddav'>"
            f"<sync-token>urn:tidl:sync:{baseline}</sync-token>"
            "<prop><getetag/></prop></sync-collection>"
        ),
    )
    assert resp.status_code == 207, resp.text
    assert f"{uid}.vcf" in resp.text
    assert "HTTP/1.1 200 OK" in resp.text, (
        "a present contact must be reported 200 even when the log says deleted"
    )
    assert "HTTP/1.1 404 Not Found" not in resp.text
