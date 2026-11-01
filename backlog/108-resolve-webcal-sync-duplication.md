# Resolve webcal sync duplication

Solve the issue as mentioned in the TODO comment in /path/to/tidl/crates/tidl-web/src/api/calendars.rs on line 412:

TODO: Figure out what went wrong here. event_fields wasn't extracted and the fields
marked below were not being written. Crucially because summary/description and start
and end date were overwritten with NULL.. why?
Second point.. this seems to be largely a copy paste of the function:
sync_calendar_now which is being called when the server starts to sync all calendars
We should be using one code path here!

These event fields where not written:

.bind(&event_fields.summary)        // FIELD ADDED
.bind(&event_fields.description)    // FIELD ADDED
.bind(&event_fields.location)       // FIELD ADDED
.bind(&event_fields.start_date)     // FIELD ADDED
.bind(&event_fields.end_date)       // FIELD ADDED

## Resolution

First half (the NULLed fields) was fixed in commit `3e4efd2` on 2026-09-29:
the raw-SQL `INSERT OR REPLACE` was missing the derived-field binds. That
also fixed the worse latent bug — `INSERT OR REPLACE` resolves a UNIQUE
conflict by *deleting* the old row, silently resetting every app-managed
column it doesn't list (`source`, `source_file`, `source_line`, note-mirror
bookkeeping, …).

Second half (one code path) done 2026-09-30. The manual
`refresh_webcal_calendar_api` no longer carries its own fetch/parse/store
copy. It now:

- loads the feed via a new `webcal_sync::load_webcal_feed(pool, username,
  calendar)`, and
- runs the same `webcal_sync::sync_calendar_now` the background loop uses.

Because `tidl-server` depends on `tidl-web` (not the reverse), the API
can't call into `tidl-server` directly; the call is injected the same way
the existing `GitCommitPusher`/`GitBranchDetector` hooks are — a new
`ssr_types::WebcalSyncer` boxed-closure extension, built in `main.rs` over
the app's `Arc<dyn CalendarStore>` and pool, and registered as an Axum
layer. This means the manual path now also gets `store_event`'s proper
`ON CONFLICT ... DO UPDATE` upsert (which preserves app-managed columns)
and the same 304/error/sync-metadata handling as the background loop.
`split_vcalendar_events_for_api` (the duplicated parser) is deleted.
