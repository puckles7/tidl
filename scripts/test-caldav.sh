#!/usr/bin/env bash
# CalDAV / WebDAV compliance test runner for tidl.
#
# Usage:
#   ./scripts/test-caldav.sh <username> <password> [host] [port]
#
# Examples:
#   ./scripts/test-caldav.sh admin mypassword
#   ./scripts/test-caldav.sh admin mypassword localhost 3060

set -euo pipefail

USER="${1:-}"
PASS="${2:-}"
HOST="${3:-localhost}"
PORT="${4:-3060}"
BASE="http://${HOST}:${PORT}"
CALDAV_URL="${BASE}/caldav/${USER}/"
CALENDAR_URL="${BASE}/caldav/${USER}/personal/"

if [[ -z "$USER" || -z "$PASS" ]]; then
  echo "Usage: $0 <username> <password> [host] [port]"
  exit 1
fi

# ── Wait for server ──────────────────────────────────────────────────────────
echo "Waiting for server at ${BASE}/health ..."
for i in $(seq 1 20); do
  if curl -sf "${BASE}/health" > /dev/null 2>&1; then
    echo "Server is up."
    break
  fi
  if [[ $i -eq 20 ]]; then
    echo "ERROR: Server did not respond after 20 attempts. Is it running?"
    exit 1
  fi
  sleep 1
done

echo ""

# ── litmus: WebDAV RFC 4918 baseline ────────────────────────────────────────
echo "============================================================"
echo " litmus — WebDAV RFC 4918 compliance"
echo " URL: ${CALDAV_URL}"
echo "============================================================"
litmus "${CALDAV_URL}" "${USER}" "${PASS}" || LITMUS_EXIT=$?

echo ""

# ── Quick manual CalDAV smoke tests via curl ─────────────────────────────────
echo "============================================================"
echo " Smoke tests — CalDAV-specific methods"
echo "============================================================"
AUTH="-u ${USER}:${PASS}"

# OPTIONS — check DAV header
echo -n "[OPTIONS] DAV capabilities ... "
DAV_HEADER=$(curl -sf $AUTH -X OPTIONS "${CALDAV_URL}" -I 2>&1 | grep -i "^DAV:" || true)
if echo "$DAV_HEADER" | grep -qi "calendar-access"; then
  echo "PASS (calendar-access advertised)"
else
  echo "FAIL — calendar-access not found in DAV header"
  echo "  Got: ${DAV_HEADER}"
fi

# PROPFIND depth 0 on calendar collection
echo -n "[PROPFIND] depth:0 on calendar collection ... "
STATUS=$(curl -sf $AUTH -o /dev/null -w "%{http_code}" \
  -X PROPFIND "${CALDAV_URL}" \
  -H "Depth: 0" \
  -H "Content-Type: application/xml" \
  -d '<?xml version="1.0"?><propfind xmlns="DAV:"><prop><resourcetype/></prop></propfind>')
if [[ "$STATUS" == "207" ]]; then
  echo "PASS (207 Multi-Status)"
else
  echo "FAIL (expected 207, got ${STATUS})"
fi

# PROPFIND depth 1 on calendar collection
echo -n "[PROPFIND] depth:1 on calendar collection ... "
STATUS=$(curl -sf $AUTH -o /dev/null -w "%{http_code}" \
  -X PROPFIND "${CALDAV_URL}" \
  -H "Depth: 1" \
  -H "Content-Type: application/xml" \
  -d '<?xml version="1.0"?><propfind xmlns="DAV:"><prop><resourcetype/><getetag/></prop></propfind>')
if [[ "$STATUS" == "207" ]]; then
  echo "PASS (207 Multi-Status)"
else
  echo "FAIL (expected 207, got ${STATUS})"
fi

# REPORT calendar-query (must target a calendar collection, not the principal root)
echo -n "[REPORT] calendar-query on personal/ ... "
STATUS=$(curl -sf $AUTH -o /dev/null -w "%{http_code}" \
  -X REPORT "${CALENDAR_URL}" \
  -H "Depth: 1" \
  -H "Content-Type: application/xml" \
  -d '<?xml version="1.0" encoding="utf-8"?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop><D:getetag/><C:calendar-data/></D:prop>
  <C:filter><C:comp-filter name="VCALENDAR"/></C:filter>
</C:calendar-query>')
if [[ "$STATUS" == "207" ]]; then
  echo "PASS (207 Multi-Status)"
else
  echo "FAIL (expected 207, got ${STATUS})"
fi

# PUT a test event
TEST_UID="tidl-test-$(date +%s)"
TEST_ICS="BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//tidl test//EN
BEGIN:VEVENT
UID:${TEST_UID}
DTSTART:20250101T100000Z
DTEND:20250101T110000Z
SUMMARY:Compliance Test Event
END:VEVENT
END:VCALENDAR"

echo -n "[PUT] create test event ... "
PUT_STATUS=$(curl -sf $AUTH -o /dev/null -w "%{http_code}" \
  -X PUT "${CALENDAR_URL}${TEST_UID}.ics" \
  -H "Content-Type: text/calendar; charset=utf-8" \
  --data-binary "$TEST_ICS")
if [[ "$PUT_STATUS" == "201" || "$PUT_STATUS" == "204" ]]; then
  echo "PASS (${PUT_STATUS})"
else
  echo "FAIL (expected 201/204, got ${PUT_STATUS})"
fi

# GET the event back
echo -n "[GET] retrieve test event ... "
GET_STATUS=$(curl -sf $AUTH -o /dev/null -w "%{http_code}" \
  "${CALENDAR_URL}${TEST_UID}.ics")
if [[ "$GET_STATUS" == "200" ]]; then
  echo "PASS (200 OK)"
else
  echo "FAIL (expected 200, got ${GET_STATUS})"
fi

# DELETE the test event
echo -n "[DELETE] remove test event ... "
DEL_STATUS=$(curl -sf $AUTH -o /dev/null -w "%{http_code}" \
  -X DELETE "${CALENDAR_URL}${TEST_UID}.ics")
if [[ "$DEL_STATUS" == "204" ]]; then
  echo "PASS (204 No Content)"
else
  echo "FAIL (expected 204, got ${DEL_STATUS})"
fi

echo ""
echo "============================================================"
echo " CalDAVTester (full RFC 4791 suite)"
echo "============================================================"
if command -v python3 > /dev/null && [[ -f "caldavtester/caldavtester.py" ]]; then
  python3 caldavtester/caldavtester.py \
    --host "$HOST" --port "$PORT" \
    --path "/caldav/${USER}/" \
    --user "$USER" --password "$PASS" \
    --all
else
  echo "CalDAVTester not found. To set it up:"
  echo "  git clone https://github.com/candiddev/caldavtester"
  echo "  pip install -r caldavtester/requirements.txt"
  echo "Then re-run this script."
fi

echo ""
echo "Done. litmus exit code: ${LITMUS_EXIT:-0}"
exit "${LITMUS_EXIT:-0}"
