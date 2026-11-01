# Settings dialog tabs

Divide the Settings modal into tabs so that growing settings content (sources, SMTP, notifications, sharing) has room to breathe without the dialog becoming unwieldy.

## Background

The Settings dialog currently holds a flat list of options. Upcoming features (#11 sources, #12 notifications, #13 calendar sharing / SMTP) will each add a meaningful block of settings. Tabs are the natural organising unit and should land before those features add their panels.

## Functional Requirements

- Replace the current flat settings layout with a tabbed interface
- Initial tabs (can be extended by later stories without touching this one):
  - **General** — existing general preferences
  - **Sources** — calendar source configuration (currently a separate dialog, see #11)
  - **Notifications** — notification defaults (#12)
  - **Email / SMTP** — outbound email configuration (#13)
- Active tab is highlighted; switching tabs does not close or reset the dialog
- Tab layout works on both desktop and mobile (horizontal tabs on wide screens, consider a select/dropdown fallback on narrow screens)
- No settings are lost when switching between tabs before saving

## Resolution

Implemented two tabs — **General** (timezone + time format + Save/Cancel) and **Import** (existing .ics import flow). The section separator and heading that previously divided the flat layout are gone; the tab bar replaces them.

Decisions made:
- Per-tab action buttons (Save in General, Import in Import) — they operate on separate concerns so a shared Save would be misleading.
- No URL anchoring for now — can be added later without touching the tab structure.
- Placeholder tabs for Sources/Notifications/Email skipped — they'll be added when the features land (#11, #12, #13).
