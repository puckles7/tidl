# Markdown-sourced tasks aren't exposed via CalDAV at all

Note-kind tasks (Notes tab, `kind == "task"/"await"`) mirror into a real
calendar and sync to CalDAV clients like Thunderbird. Markdown-sourced
tasks/awaits (`crates/tidl-server/src/markdown_sync.rs`, from `#task`/`#await`
tags in synced `.md` files) don't: they're stored under a synthetic
`calendar = '__markdown__'` label with no backing `calendars` row and no
CalDAV route (`/caldav/{username}/__markdown__` doesn't exist), so they only
ever show up in tidl's own Focus/Todos web UI. This asymmetry was flagged during a 2026-07-30 investigation into why a
Notes-tab task appeared in Thunderbird but a similar markdown-sourced task
couldn't. That investigation also found and fixed a separate, real bug:
note-mirrored VTODOs weren't bumping `calendars.sync_token`, so ctag-based
clients (Thunderbird) never noticed new/changed/deleted note-tasks even
though they were correctly written to the `events` table
(`crates/tidl-web/src/api/notes.rs`'s `sync_mirror`/`delete_note`/
`delete_note_collection`, now calling the new `bump_calendar_sync_token`
helper). Markdown tasks have no such fix available, since there's no
calendar row for their ctag to live on in the first place.

## Context / Background

`crates/tidl-server/src/markdown_sync.rs` explicitly documents the current
design: `__markdown__` "never appears in the UI" (meaning the calendar
picker) and has no FK to `calendars` (removed in migration 20240119). This
was presumably a deliberate simplification when markdown-source tasks were
first built (#18) — the source `.md` file is the source of truth, and CalDAV
write-back would need to translate an edit (e.g. marking a task done from
Thunderbird) back into a specific line of a specific file, which is more
involved than the Notes-tab mirror (where the note itself, not an external
file, is the source of truth).

## Functional Requirements (to be refined — this is an investigation, not a spec)

- Decide whether markdown tasks should get their own real calendar
  (per-source? one shared "Markdown tasks" calendar per user?) or whether
  users should instead be able to pick a target calendar per markdown source,
  mirroring the Notes-tab model.
- Handle CalDAV-side edits: at minimum, completing a task via CalDAV should
  either write back to the source file (matching `events.rs`'s existing
  note-completion write-back for `source == "markdown"`) or be explicitly
  rejected/ignored with a clear reason.
- Keep the two task sources (`notes`, `markdown`) behaviorally consistent
  from a CalDAV client's point of view, or clearly document why they can't
  be.

## Open Questions

- Is there a reason markdown tasks were deliberately kept CalDAV-invisible
  (e.g. avoiding surprising edits to files outside tidl's control) that
  should stay a hard requirement rather than being designed around?
- Should this apply to `await`-kind markdown items too, or tasks only?

## Why dropped (2026-09-17)

Deprioritized for now, per explicit request — not pursuing at this time.
No design or technical blocker; the investigation findings above still
apply if this is picked back up later.
