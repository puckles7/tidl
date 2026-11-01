"""
Two-instance CalDAV sync E2E tests.

Run with:
    pytest tests/test_sync.py -v

Requires:
    DISABLE_SERVER_FN_HASH=1 cargo build -p tidl-server
    pip install caldav icalendar requests playwright
    playwright install chromium
    # optional: export TIDL_BINARY=/path/to/tidl-server
"""

import uuid
import time
import pytest
import caldav
import requests
from datetime import datetime, timezone, timedelta
from icalendar import Calendar as ICal

# Import fixtures so pytest discovers them.
# tidl_binary must be imported explicitly: pytest only auto-discovers transitive
# fixture deps when they live in conftest.py; sync_fixtures.py is a plain module.
from fixtures import (
    tidl_binary,     # must be explicit; fixtures.py is not a conftest
    source_server,
    sink_server,
    source_app_token,
    source_cal,
    sink_cal,
    pull_source,
    push_source,
    both_source,
    make_vevent,
    make_vtodo,
    put_event,
    get_event_data,
    ical_prop,           # noqa: F401 — used in Tasks 4+
    wait_for_uid,
    wait_for_uid_absent,
    sync_browser,
    sink_page,
    trigger_sync,
)


class TestInfrastructure:
    def test_source_is_running(self, source_server):
        r = requests.get(source_server.url)
        assert r.status_code < 500

    def test_sink_is_running(self, sink_server):
        r = requests.get(sink_server.url)
        assert r.status_code < 500


class TestFixtureSetup:
    def test_app_token_is_created(self, source_app_token):
        assert source_app_token.startswith("tidl_")
        assert len(source_app_token) > 10

    def test_calendars_are_created(self, source_cal, sink_cal):
        assert "src-" in source_cal.url.path
        assert "snk-" in sink_cal.url.path

    def test_pull_source_is_registered(self, pull_source):
        assert pull_source  # non-empty UUID string


class TestSyncTrigger:
    def test_pull_single_event(self, source_cal, sink_cal, pull_source, sink_page):
        """Smoke: one VEVENT on source appears on sink after sync."""
        ical, uid = make_vevent(summary="Trigger Smoke")
        put_event(source_cal, ical, uid)

        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)

        data = get_event_data(sink_cal, uid)
        assert "Trigger Smoke" in data


class TestPull:
    """Source → Sink sync (pull direction only)."""

    def test_vevent_appears_on_sink(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vevent(summary="Pull VEVENT", location="Room A")
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        assert "Pull VEVENT" in data
        assert "Room A" in data

    def test_vtodo_appears_on_sink(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vtodo(summary="Pull VTODO", priority=3)
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        # VTODOs must be looked up via todo_by_uid, not event_by_uid
        deadline = time.time() + 8
        last_exc = None
        while time.time() < deadline:
            try:
                sink_cal.todo_by_uid(uid)
                break
            except Exception as e:
                last_exc = e
                time.sleep(0.25)
        else:
            pytest.fail(f"VTODO UID={uid!r} not found on sink within 8s. Last: {last_exc}")
        todo = sink_cal.todo_by_uid(uid)
        data = todo.data if isinstance(todo.data, str) else todo.data.decode()
        assert "Pull VTODO" in data

    def test_allday_event(self, source_cal, sink_cal, pull_source, sink_page):
        start = datetime(2026, 6, 15, tzinfo=timezone.utc)
        ical, uid = make_vevent(
            summary="All Day", dtstart=start,
            dtend=start + timedelta(days=1), all_day=True,
        )
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        # DATE value type must be preserved (no time component)
        assert "VALUE=DATE" in data
        assert "20260615" in data

    def test_multiday_event(self, source_cal, sink_cal, pull_source, sink_page):
        start = datetime(2026, 6, 20, tzinfo=timezone.utc)
        ical, uid = make_vevent(
            summary="Multi Day",
            dtstart=start,
            dtend=start + timedelta(days=3),
            all_day=True,
        )
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        # End date is June 23 (exclusive end of 3-day span); check DTEND specifically to
        # avoid false positives from DTSTAMP or LAST-MODIFIED carrying the same date.
        assert any("DTEND" in line and "20260623" in line for line in data.splitlines())

    def test_event_with_valarm_roundtrip(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vevent(summary="Alarm Event", valarm_minutes=15)
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        assert "BEGIN:VALARM" in data
        assert "TRIGGER:-PT15M" in data
        assert "END:VALARM" in data

    def test_rrule_weekly(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vevent(summary="Weekly", rrule="FREQ=WEEKLY;COUNT=4")
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        assert "RRULE" in data
        assert "WEEKLY" in data

    def test_rrule_with_exdate(self, source_cal, sink_cal, pull_source, sink_page):
        start = datetime(2026, 6, 1, 10, 0, tzinfo=timezone.utc)
        ical, uid = make_vevent(
            summary="Recur+Excl",
            dtstart=start,
            dtend=start + timedelta(hours=1),
            rrule="FREQ=WEEKLY;COUNT=6",
            exdate="20260608T100000Z",
        )
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        assert "EXDATE" in data
        assert "20260608" in data

    def test_partstat_preserved(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vevent(summary="Meeting Invite", partstat="DECLINED")
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        assert "PARTSTAT=DECLINED" in data

    def test_special_chars_in_summary(self, source_cal, sink_cal, pull_source, sink_page):
        # Colons, semicolons, and UTF-8 in SUMMARY
        summary = "Réunion: café & résumé — 100%"
        ical, uid = make_vevent(summary=summary)
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)
        data = get_event_data(sink_cal, uid)
        assert "café" in data or "Réunion" in data  # UTF-8 survived

    def test_modify_event_on_source(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vevent(summary="Original Summary")
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)

        # Modify on source
        updated_ical, _ = make_vevent(uid=uid, summary="Updated Summary")
        put_event(source_cal, updated_ical, uid)
        trigger_sync(sink_page, sink_cal)

        # Wait until sink has the updated summary
        deadline = time.time() + 8
        while time.time() < deadline:
            try:
                data = get_event_data(sink_cal, uid)
                if "Updated Summary" in data:
                    return
            except Exception:
                pass
            time.sleep(0.25)
        pytest.fail("Updated summary never appeared on sink")

    def test_delete_event_on_source(self, source_cal, sink_cal, pull_source, sink_page):
        ical, uid = make_vevent(summary="To Be Deleted")
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(sink_cal, uid)

        # Delete from source
        event = source_cal.event_by_uid(uid)
        event.delete()
        trigger_sync(sink_page, sink_cal)
        wait_for_uid_absent(sink_cal, uid)

    def test_pull_only_does_not_push_sink_events(
        self, source_cal, sink_cal, pull_source, sink_page
    ):
        """Events created on sink should NOT appear on source in pull-only mode."""
        ical, uid = make_vevent(summary="Sink Only Event")
        put_event(sink_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        # Poll to confirm event stays absent from source
        deadline = time.time() + 8
        appeared = False
        while time.time() < deadline:
            try:
                source_cal.event_by_uid(uid)
                appeared = True
                break
            except Exception:
                time.sleep(0.25)
        assert not appeared, "Pull-only: sink event should not have been pushed to source"


class TestPush:
    """Sink → Source sync (push direction only)."""

    def test_vevent_appears_on_source(self, source_cal, sink_cal, push_source, sink_page):
        ical, uid = make_vevent(summary="Push VEVENT")
        put_event(sink_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(source_cal, uid)
        data = get_event_data(source_cal, uid)
        assert "Push VEVENT" in data

    def test_modify_event_pushes_update(
        self, source_cal, sink_cal, push_source, sink_page
    ):
        ical, uid = make_vevent(summary="Push Original")
        put_event(sink_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(source_cal, uid)

        # Modify on sink, sync again
        updated_ical, _ = make_vevent(uid=uid, summary="Push Updated")
        put_event(sink_cal, updated_ical, uid)
        trigger_sync(sink_page, sink_cal)

        deadline = time.time() + 8
        while time.time() < deadline:
            try:
                data = get_event_data(source_cal, uid)
                if "Push Updated" in data:
                    return
            except Exception:
                pass
            time.sleep(0.25)
        pytest.fail("Push-updated summary never appeared on source")

    def test_delete_event_removes_from_source(
        self, source_cal, sink_cal, push_source, sink_page
    ):
        ical, uid = make_vevent(summary="Push Delete Me")
        put_event(sink_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        wait_for_uid(source_cal, uid)

        event = sink_cal.event_by_uid(uid)
        event.delete()
        trigger_sync(sink_page, sink_cal)
        wait_for_uid_absent(source_cal, uid)

    def test_push_only_does_not_pull_source_events(
        self, source_cal, sink_cal, push_source, sink_page
    ):
        """Events created on source should NOT appear on sink in push-only mode."""
        ical, uid = make_vevent(summary="Source Only")
        put_event(source_cal, ical, uid)
        trigger_sync(sink_page, sink_cal)
        deadline = time.time() + 8
        appeared = False
        while time.time() < deadline:
            try:
                sink_cal.event_by_uid(uid)
                appeared = True
                break
            except Exception:
                time.sleep(0.25)
        assert not appeared, "Push-only: source event should not have been pulled to sink"
