"""
CalDAV protocol integration tests for tidl.

Requires a running tidl server. Run via:
    test-caldav admin admin          # from inside the nix dev shell
    pytest tests/test_caldav.py -v   # directly, with server on localhost:3060

Flags (passed by test-caldav shell function):
    --server-url   base URL of the server  (default: http://localhost:3060)
    --username     test account username   (default: admin)
    --password     test account password   (default: admin)
"""

import uuid
import pytest
import requests
from datetime import datetime, timezone, timedelta


# ── Fixtures ──────────────────────────────────────────────────────────────────

def _make_calendar(dav_client, server_url, credentials):
    """Create a calendar via raw MKCOL and return a caldav.Calendar object."""
    import caldav
    name = f"tidl-test-{uuid.uuid4().hex[:8]}"
    url = f"{server_url}/caldav/{credentials[0]}/{name}/"
    r = requests.request("MKCOL", url, auth=credentials)
    assert r.status_code in (201, 204), f"MKCOL failed: {r.status_code}"
    return caldav.Calendar(client=dav_client, url=url)


@pytest.fixture
def test_calendar(dav_client, server_url, credentials):
    """Fresh calendar for each test; deleted on teardown."""
    cal = _make_calendar(dav_client, server_url, credentials)
    yield cal
    try:
        cal.delete()
    except Exception:
        pass


# ── ICS helpers ───────────────────────────────────────────────────────────────

def make_vevent(
    uid=None,
    summary="Test Event",
    start=None,
    end=None,
    partstat=None,
):
    """Return (ical_str, uid) for a minimal VEVENT."""
    if uid is None:
        uid = str(uuid.uuid4())
    if start is None:
        start = datetime.now(timezone.utc).replace(microsecond=0)
    if end is None:
        end = start + timedelta(hours=1)

    attendee = ""
    if partstat:
        attendee = f"ATTENDEE;PARTSTAT={partstat}:mailto:testuser@example.com\r\n"

    ical = (
        "BEGIN:VCALENDAR\r\n"
        "VERSION:2.0\r\n"
        "PRODID:-//tidl tests//EN\r\n"
        "BEGIN:VEVENT\r\n"
        f"UID:{uid}\r\n"
        f"SUMMARY:{summary}\r\n"
        f"DTSTART:{start.strftime('%Y%m%dT%H%M%SZ')}\r\n"
        f"DTEND:{end.strftime('%Y%m%dT%H%M%SZ')}\r\n"
        f"{attendee}"
        "END:VEVENT\r\n"
        "END:VCALENDAR\r\n"
    )
    return ical, uid


def raw_put(url, ical, credentials, extra_headers=None):
    headers = {"Content-Type": "text/calendar; charset=utf-8"}
    if extra_headers:
        headers.update(extra_headers)
    return requests.put(url, data=ical, auth=credentials, headers=headers)


def raw_report(url, body, credentials):
    return requests.request(
        "REPORT", url,
        data=body,
        auth=credentials,
        headers={"Content-Type": "application/xml", "Depth": "1"},
    )


def raw_propfind(url, body, credentials, depth="0"):
    return requests.request(
        "PROPFIND", url,
        data=body,
        auth=credentials,
        headers={"Depth": depth},
    )


# ── Authentication ────────────────────────────────────────────────────────────

def test_auth_required(principal_url):
    r = requests.get(principal_url)
    assert r.status_code == 401
    assert "WWW-Authenticate" in r.headers

def test_auth_wrong_password(principal_url, credentials):
    r = requests.get(principal_url, auth=(credentials[0], "definitely-wrong"))
    assert r.status_code == 401

def test_auth_valid(principal_url, credentials):
    r = requests.get(principal_url, auth=credentials)
    assert r.status_code in (200, 207)


# ── Discovery & OPTIONS ───────────────────────────────────────────────────────

def test_options_advertises_caldav(principal_url, credentials):
    r = requests.options(principal_url, auth=credentials)
    assert r.status_code == 200
    assert "calendar-access" in r.headers.get("DAV", "")

def test_principal_reachable(principal):
    assert principal is not None

def test_calendar_home_lists_calendars(principal):
    calendars = principal.calendars()
    assert isinstance(calendars, list)

def test_well_known_redirect(server_url, credentials):
    r = requests.get(
        f"{server_url}/.well-known/caldav",
        auth=credentials,
        allow_redirects=False,
    )
    # Must redirect toward the CalDAV root
    assert r.status_code in (301, 302, 307, 308)


# ── Calendar collection CRUD ──────────────────────────────────────────────────

def test_create_calendar(dav_client, server_url, credentials, principal):
    cal = _make_calendar(dav_client, server_url, credentials)
    name = cal.url.path.rstrip("/").split("/")[-1]
    try:
        urls = [str(c.url) for c in principal.calendars()]
        assert any(name in u for u in urls)
    finally:
        cal.delete()

def test_delete_calendar(dav_client, server_url, credentials, principal):
    cal = _make_calendar(dav_client, server_url, credentials)
    name = cal.url.path.rstrip("/").split("/")[-1]
    cal.delete()
    urls = [str(c.url) for c in principal.calendars()]
    assert not any(name in u for u in urls)


# ── Event CRUD ────────────────────────────────────────────────────────────────

def test_put_and_get_event(test_calendar):
    ical, uid = make_vevent(summary="Round-trip test")
    test_calendar.save_event(ical)
    fetched = test_calendar.event_by_uid(uid)
    assert "Round-trip test" in str(fetched.data)

def test_update_event(test_calendar):
    ical, uid = make_vevent(summary="Original summary")
    test_calendar.save_event(ical)

    updated, _ = make_vevent(uid=uid, summary="Updated summary")
    test_calendar.save_event(updated)

    fetched = test_calendar.event_by_uid(uid)
    assert "Updated summary" in str(fetched.data)
    assert "Original summary" not in str(fetched.data)

def test_delete_event(test_calendar):
    ical, uid = make_vevent(summary="Delete me")
    event = test_calendar.save_event(ical)
    event.delete()

    with pytest.raises(Exception):
        test_calendar.event_by_uid(uid)

def test_multiple_events_in_calendar(test_calendar):
    uids = []
    for i in range(3):
        ical, uid = make_vevent(summary=f"Event {i}")
        test_calendar.save_event(ical)
        uids.append(uid)

    events = test_calendar.events()
    combined = " ".join(str(e.data) for e in events)
    for uid in uids:
        assert uid in combined

def test_put_invalid_ical(test_calendar, credentials):
    url = str(test_calendar.url).rstrip("/") + f"/{uuid.uuid4()}.ics"
    r = raw_put(url, "this is not valid icalendar data", credentials)
    assert r.status_code >= 400


# ── ETag handling ─────────────────────────────────────────────────────────────

def test_put_returns_etag(test_calendar, credentials):
    ical, uid = make_vevent(summary="ETag check")
    url = str(test_calendar.url).rstrip("/") + f"/{uid}.ics"
    r = raw_put(url, ical, credentials)
    assert r.status_code in (201, 204)
    assert "ETag" in r.headers

def test_get_returns_etag(test_calendar, credentials):
    ical, uid = make_vevent(summary="ETag on GET")
    url = str(test_calendar.url).rstrip("/") + f"/{uid}.ics"
    raw_put(url, ical, credentials)
    r = requests.get(url, auth=credentials)
    assert r.status_code == 200
    assert "ETag" in r.headers

def test_if_match_rejects_stale_etag(test_calendar, credentials):
    ical, uid = make_vevent(summary="If-Match test")
    url = str(test_calendar.url).rstrip("/") + f"/{uid}.ics"
    raw_put(url, ical, credentials)

    updated, _ = make_vevent(uid=uid, summary="Should be blocked")
    r = raw_put(url, updated, credentials, extra_headers={"If-Match": '"stale-etag-value"'})
    assert r.status_code == 412

    # Original must still be intact
    r2 = requests.get(url, auth=credentials)
    assert "If-Match test" in r2.text

def test_if_match_accepts_correct_etag(test_calendar, credentials):
    ical, uid = make_vevent(summary="Correct ETag update")
    url = str(test_calendar.url).rstrip("/") + f"/{uid}.ics"
    r1 = raw_put(url, ical, credentials)
    etag = r1.headers["ETag"]

    updated, _ = make_vevent(uid=uid, summary="ETag matched update")
    r2 = raw_put(url, updated, credentials, extra_headers={"If-Match": etag})
    assert r2.status_code in (201, 204)


# ── PROPFIND ──────────────────────────────────────────────────────────────────

def test_propfind_calendar_collection(test_calendar, credentials):
    r = raw_propfind(
        str(test_calendar.url),
        """<?xml version="1.0"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:displayname/>
    <D:resourcetype/>
    <D:getetag/>
  </D:prop>
</D:propfind>""",
        credentials,
    )
    assert r.status_code == 207
    assert "calendar" in r.text.lower()

def test_propfind_event_resource(test_calendar, credentials):
    ical, uid = make_vevent(summary="Propfind event")
    url = str(test_calendar.url).rstrip("/") + f"/{uid}.ics"
    raw_put(url, ical, credentials)

    r = raw_propfind(
        url,
        """<?xml version="1.0"?>
<D:propfind xmlns:D="DAV:">
  <D:prop><D:getetag/></D:prop>
</D:propfind>""",
        credentials,
    )
    assert r.status_code == 207
    assert "etag" in r.text.lower()

def test_propfind_depth1_lists_events(test_calendar, credentials):
    ical, uid = make_vevent(summary="Depth-1 event")
    test_calendar.save_event(ical)

    r = raw_propfind(
        str(test_calendar.url),
        """<?xml version="1.0"?>
<D:propfind xmlns:D="DAV:">
  <D:prop><D:getetag/></D:prop>
</D:propfind>""",
        credentials,
        depth="1",
    )
    assert r.status_code == 207
    assert uid in r.text


# ── REPORT: calendar-multiget ─────────────────────────────────────────────────

def test_multiget_returns_requested_event(test_calendar, credentials, server_url):
    ical, uid = make_vevent(summary="Multiget target")
    test_calendar.save_event(ical)

    cal_url = str(test_calendar.url)
    # Derive the event href from what the server exposes via PROPFIND
    r_pf = raw_propfind(cal_url, """<?xml version="1.0"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:getetag/></D:prop></D:propfind>""",
        credentials, depth="1")
    # Find the href for our uid in the propfind response
    assert uid in r_pf.text
    event_href = f"{cal_url.rstrip('/')}/{uid}.ics"

    body = f"""<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <D:href>{event_href}</D:href>
</C:calendar-multiget>"""

    r = raw_report(cal_url, body, credentials)
    assert r.status_code == 207
    assert uid in r.text
    assert "Multiget target" in r.text

def test_multiget_returns_calendar_data(test_calendar, credentials):
    ical, uid = make_vevent(summary="Calendar-data check")
    test_calendar.save_event(ical)

    cal_url = str(test_calendar.url)
    event_href = f"{cal_url.rstrip('/')}/{uid}.ics"

    body = f"""<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <D:href>{event_href}</D:href>
</C:calendar-multiget>"""

    r = raw_report(cal_url, body, credentials)
    assert r.status_code == 207
    assert "BEGIN:VCALENDAR" in r.text
    assert "BEGIN:VEVENT" in r.text


# ── REPORT: calendar-query ────────────────────────────────────────────────────

def test_calendar_query_returns_events(test_calendar, credentials):
    ical, uid = make_vevent(summary="Query target")
    test_calendar.save_event(ical)

    body = """<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT"/>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"""

    r = raw_report(str(test_calendar.url), body, credentials)
    assert r.status_code == 207
    assert uid in r.text

def test_calendar_query_with_time_range(test_calendar, credentials):
    now = datetime.now(timezone.utc)
    ical, uid = make_vevent(summary="Time-range target", start=now)
    test_calendar.save_event(ical)

    start_str = (now - timedelta(days=1)).strftime("%Y%m%dT%H%M%SZ")
    end_str   = (now + timedelta(days=1)).strftime("%Y%m%dT%H%M%SZ")

    body = f"""<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT">
        <C:time-range start="{start_str}" end="{end_str}"/>
      </C:comp-filter>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"""

    r = raw_report(str(test_calendar.url), body, credentials)
    assert r.status_code == 207
    assert uid in r.text

def test_calendar_query_empty_calendar(test_calendar, credentials):
    body = """<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop><D:getetag/></D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT"/>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"""

    r = raw_report(str(test_calendar.url), body, credentials)
    assert r.status_code == 207
    # Should return a valid multistatus with no event responses
    assert "multistatus" in r.text.lower()


# ── PARTSTAT / response_status ────────────────────────────────────────────────

def test_partstat_accepted_preserved(test_calendar):
    ical, uid = make_vevent(summary="Accepted meeting", partstat="ACCEPTED")
    test_calendar.save_event(ical)
    fetched = test_calendar.event_by_uid(uid)
    assert "PARTSTAT=ACCEPTED" in str(fetched.data)

def test_partstat_tentative_preserved(test_calendar):
    ical, uid = make_vevent(summary="Tentative meeting", partstat="TENTATIVE")
    test_calendar.save_event(ical)
    fetched = test_calendar.event_by_uid(uid)
    assert "PARTSTAT=TENTATIVE" in str(fetched.data)

def test_partstat_declined_preserved(test_calendar):
    ical, uid = make_vevent(summary="Declined meeting", partstat="DECLINED")
    test_calendar.save_event(ical)
    fetched = test_calendar.event_by_uid(uid)
    assert "PARTSTAT=DECLINED" in str(fetched.data)

def test_partstat_visible_in_multiget(test_calendar, credentials):
    ical, uid = make_vevent(summary="Multiget PARTSTAT", partstat="ACCEPTED")
    test_calendar.save_event(ical)

    cal_url = str(test_calendar.url)
    event_href = f"{cal_url.rstrip('/')}/{uid}.ics"

    body = f"""<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <D:href>{event_href}</D:href>
</C:calendar-multiget>"""

    r = raw_report(cal_url, body, credentials)
    assert r.status_code == 207
    assert "PARTSTAT=ACCEPTED" in r.text

def test_partstat_visible_in_calendar_query(test_calendar, credentials):
    ical, uid = make_vevent(summary="Query PARTSTAT", partstat="TENTATIVE")
    test_calendar.save_event(ical)

    body = """<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag/>
    <C:calendar-data/>
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT"/>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"""

    r = raw_report(str(test_calendar.url), body, credentials)
    assert r.status_code == 207
    assert "PARTSTAT=TENTATIVE" in r.text
