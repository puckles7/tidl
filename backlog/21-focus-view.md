# Focus view — urgency-ranked unified list

A new top-level tab that fuses calendar events, todos (both CalDAV and
markdown), and pending items into a single list sorted by urgency score
(0–100). The user always sees the most pressing thing at the top regardless
of source.

## Layout

Flex column, 28px vertical / 32px horizontal padding, 28px gap. Max content
width 720 px.

**Header row:**
- "Focus" title (28px / weight 300 / `#1f1f1f` / letter-spacing -0.5px).
- Today's date subtitle (13px / `#888`), e.g. "Tuesday, 12 May 2026".
- Horizon segmented control on the right (Today / 3 days / 7 days). Pill
  track `#f1f3f4`, selected pill white with `0 1px 3px rgba(0,0,0,.1)` shadow.

**Section header:** 10px uppercase `#aaa` "All items · ranked by urgency" +
count badge (`{N} remaining`) on the right. 1px bottom border `#eee`.

## Focus card (shared component)

Used across Focus, Tasks, and Tickler:

- White bg, 1px `#e8e8e8` border, 7px radius, 10px 12px padding, 10px
  padding-left (for the 3px left stripe).
- **Left stripe** (3px solid):
  - Calendar event → calendar color (blue/red/green)
  - Todo (either source) → `#cfd2d6`
  - Pending future → `#8b5cf6`
  - Pending overdue → `#d93025`
- Hover: border `#c8d4e8`, shadow `0 2px 8px rgba(0,0,0,.06)`.
- Flex row, gap 10px, center-aligned:
  1. **Rank number** — 11px `#ccc` tabular, 16px wide right-aligned. `·` when done.
  2. **Checkbox circle** — 15px, 1.5px border `#ccc`; hover `#1a73e8`; checked
     filled `#1a73e8` + white tick SVG.
  3. **Body** — title 13px / `#1f1f1f` / line-height 1.4 + meta row (pill + when/source).
  4. **Urgency cluster** (right) — score label (9px / weight 600 / uppercase) +
     40×3px score bar (`#f0f0f0` track, colored fill, transitions 0.3s).
- Done state: opacity 0.45, title strikethrough, score label → "DONE".
- Suppressed state: bg `#fafafa`, opacity 0.55 (0.85 hover), rank and urgency
  cluster hidden.

**Hover actions** — fade in on hover, fade out on leave, right-fade gradient
overlay (transparent → white 18%):
- **↗ Go to source** (always): see source routing below.
- **👁 Suppress** (active): moves item to Suppressed section (session-only).
- **↺ Unsuppress** (suppressed): restores to main list.

## Source routing for "Go to source"

| Item kind | Action |
|-----------|--------|
| Calendar event | Open event detail overlay (Variant A), footer "Open in Calendar →" navigates to Calendar tab |
| CalDAV VTODO | Navigate to Tasks tab, scroll to / highlight the item |
| Markdown todo | Open markdown file preview overlay (Variant B) at the tagged line |
| Pending (markdown) | Open markdown file preview overlay (Variant B) at the tagged line |

## Scoring function (canonical)

All four item kinds are normalised to a 0–100 urgency score.

```
eventUrgency(e):
  d = dayDiff(e.date)
  d < 0  → 0   (past)
  d == 0 → time-based: <30min→99, <1h→93, <2h→85, <4h→75, else 65
  d == 1 → 50
  d <= 3 → 38
  else   → 25

todoUrgency(t):          -- applies to BOTH markdown and CalDAV todos
  p1 / CalDAV high (1-4)    → 82
  p2 / CalDAV medium (5)    → 55
  p3 / CalDAV low (6-9, 0)  → 28

pendingUrgency(p):
  overdue → 88, today → 72, tomorrow → 52, ≤3d → 35, else → 18
```

CalDAV priority normalisation:

| CalDAV value | Urgency tier |
|---|---|
| 1–4 (high)        | p1 → 82 |
| 5 (medium)        | p2 → 55 |
| 6–9, 0 (low/none) | p3 → 28 |

**Urgency bands:**
- ≥ 85 → "URGENT"   / `#d93025`
- ≥ 65 → "SOON"     / `#f09300`
- ≥ 40 → "UPCOMING" / `#1a73e8`
- < 40 → "LATER"    / `#bbb`

## Horizon filtering

Applied before scoring:
- Events: `0 ≤ dayDiff < horizon` AND calendar enabled in sidebar.
- Todos (both sources): `normalisedPrio ≤ maxPrio(horizon)` — p1 for Today,
  ≤p2 for 3 days, all for 7 days.
- Pending: `−7 ≤ dayDiff < horizon` (up to 7 days overdue always shown).

## Suppressed section

Below the main list, only when ≥1 item is suppressed:
- 24px top margin, 1px **dashed** `#e0e0e0` top border, 14px top padding.
- Toggle row: chevron (rotates 90° when open) + "Suppressed for this session (N)"
  + "resets on reload" hint (10px `#bbb`).

## Source preview overlay

**Variant A — calendar event:**
- Header: tinted calendar-color icon box + event title + "Calendar event · {name}".
- Body: key/value rows (When, Calendar, Status).
- Footer: "Close" + "Open in Calendar →".

**Variant B — markdown item (todo or pending):**
- Header: 📄 icon on `#f1f3f4` bg + filename + "Line N · {full tag}".
- Body: monospace code block, line numbers, `#todo/...` tags in `#b06000`,
  `#pending/...` tags in `#6b3fc5`, highlighted line in `#fff8d6`.
- Footer: "Close" + "Open in editor →" (stub for now).

**Variant C — CalDAV VTODO:**
No overlay — "Go to source" navigates directly to the Tasks tab and
highlights the item there.

## State

| Key | Type | Persistence |
|-----|------|-------------|
| `done` | `HashMap<id, bool>` | Persist in user prefs or local storage |
| `suppressed` | `HashMap<id, bool>` | **Session-only** — resets on reload |
| `horizon` | `1 \| 3 \| 7` | Persist in user prefs |
| `show_suppressed` | `bool` | UI-only |
| `preview` | `Option<Item>` | UI-only |

Calendar filter state is already in the existing calendars context.

## Dependencies

- [#18 — Markdown source files](18-markdown-sources.md)
- [#3 — Multi-calendar support](03-multi-calendar.md) ✓

## Resolution

Implemented the full Focus view as a new `/focus` route and "Focus" nav tab.

**Built:**
- Urgency scoring engine (canonical scores for events, todos, pending items per spec)
- Horizon segmented control (Today / 3 days / 7 days) with reactive filtering
- Ranked card list (`FocusScoredCard`): left stripe by type/overdue status, rank number, urgency label + score bar, suppress/unsuppress hover actions, done toggle
- Session-only suppression — items hide below a collapsible "Suppressed" section
- Done state: events tracked in-session; todos/pending call `complete_todo_item` on the server
- "Go to source" routing: Variant A overlay for calendar events (title, when, calendar), Variant B overlay for markdown items (file content with highlighted line), Variant C navigation to `/todos` for CalDAV todos
- New `get_markdown_file_content` server function (secure: resolves path via DB, not from client)
- Added `PartialEq` to `Event` and `Todo` types

**Deviations from spec:**
- Calendar sidebar visibility filter not applied (CalendarsCtx is scoped to `CalendarView`, not shared globally); all events in horizon are shown
- `done` state for events is session-only (no persistence to user prefs) — a future enhancement
- Urgency times compared to UTC (no timezone conversion) since `chrono-tz` not in deps
