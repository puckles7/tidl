# DAVx5 compatibility verification and fixes

DAVx5 is the de facto standard CalDAV/CardDAV sync client for Android. While tidl's CalDAV implementation should work with DAVx5 in principle, it has never been systematically verified. This item covers testing tidl against a real DAVx5 installation and fixing any incompatibilities found.

## Background

iOS has native CalDAV support built in, so Apple devices sync with tidl without any third-party app. Android has no native CalDAV support — users must install DAVx5. DAVx5 is strict about protocol compliance and has historically exposed bugs in CalDAV servers that work fine with iOS.

## Functional Requirements

- Set up a DAVx5 test account against a running tidl instance
- Test the full sync lifecycle: initial discovery, calendar list, event create/update/delete, recurring events, alarms
- Test with multiple calendars (tidl supports multi-calendar)
- Document any failures, protocol errors, or unexpected behaviours
- Fix identified issues in tidl's CalDAV implementation
- Optionally: test against the DAVx5 debug log output to catch non-fatal warnings too

## Stretch Goal

- Once #30 (WebDAV Push) is investigated: verify DAVx5's push subscription flow works end-to-end if implemented

## Open Questions

- Does DAVx5 require any specific well-known URL discovery (`/.well-known/caldav`) that tidl may not expose?
- Are there known DAVx5 quirks around `VTIMEZONE` handling that tidl's parser needs to accommodate?
- Does DAVx5 exercise any CalDAV scheduling extensions (iTIP/iMIP) beyond what tidl already handles?
