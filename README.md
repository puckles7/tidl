# tidl — Personal time management for people who live in their notes

A CalDAV server that bridges your calendar and tasks with your markdown knowledge base. One binary, SQLite storage, built in Rust.

## Features

### Calendar

- **CalDAV Protocol** — Standards-compliant RFC 4791 with sync-collection (RFC 6578)
- **Multiple calendars** — Colour coding and visibility toggles; a muted, theme-aware palette adapts between light and dark mode. Every coloured entity type (calendars, markdown sources, note & contact collections, labels) shares one well-spread base palette, hue-rotated per type, so each list has real internal contrast while a given colour still never appears in two different types
- **Calendar sharing** — Share with other users (read or write) via email invitation; the shared calendar appears as a toggleable sidebar entry and its events in the grantee's own Calendar/Focus views, plus as an additional calendar in their own CalDAV client (iOS Calendar, DAVx⁵, Thunderbird) — no second account or all-or-nothing token needed; a self-share or accepting someone else's invitation is rejected outright; grants, calendar deletions, and event edits within an already-shared calendar all push live over SSE to the owner and every grantee, so an already-open tab — on either side of the share — picks up a new, revoked, or deleted share, or a new/edited/deleted event, without a manual refresh; the Settings → Sharing tab lists every grant you've extended (with one-click revoke) and every calendar shared with you (with one-click remove on your own side, independent of the owner), and also lists owned, shared note collections so members can be revoked or a collection unshared without opening its editor; a shared calendar's sidebar entry has its own edit modal too — the grantee can pick their own display color for it (a private override, invisible to the owner and other grantees) and, if their grant is read-write, clear all of its events (with a warning that this deletes the owner's real events, not just their own view); any calendar or note-collection sidebar row that's actually shared — owned with at least one grantee, or shared with you — also gets an always-visible share icon button that jumps straight to Settings → Sharing, for the owner to see/manage grantees or the grantee to see who shared it and leave
- **Calendar import** — Import `.ics` files into any calendar
- **Webcal subscriptions** — Subscribe to external iCalendar feeds (school portals, sports leagues, public holidays)
- **Remote CalDAV sync** — Bidirectional mirror with any remote CalDAV server using app tokens
- **iMIP inbound email** — Import meeting invitations by forwarding them to a personal tidl address; automatic parsing and upsert with SPF verification
- **All-day & multi-day events** — Full support for date-only and spanning events
- **Reminders** — Full VALARM support; DISPLAY/AUDIO alarms fire client-side in the browser, and EMAIL alarms are dispatched server-side by a background job (durable, restart-safe dedup; handles recurring events and REPEAT/DURATION repeats)
- **Recurring events** — Author and edit RRULE-based recurrences; edit single occurrences or entire series; delete single occurrences with EXDATE
- **Vertical zoom & smart default scroll** — the Day/Work-week/Week time grid's hour scale is adjustable (24–120 px/hour) from −/+ buttons in the week nav or Ctrl/⌘ + mouse wheel, and the time at the top of the viewport is held fixed while zooming; the level is remembered per browser. The grid opens on a fixed 7am window instead of centering the current time, so morning-heavy days start in view

### Tasks & Planning

- **Priority-based tasks** — one canonical 1-100 priority scale (1 = highest) shared by Focus's task-creation slider, notes, and markdown's `#task/p<N>` tags; projected onto RFC 5545 CalDAV's native 1-9 `PRIORITY` range only at the CalDAV wire boundary, so an external client (e.g. Thunderbird) editing a task's priority or completion status stays in sync in both directions — with inline create/complete/delete and markdown write-back
- **Focus view** — Urgency-ranked unified list of events, tasks, and await items with an All/Calendar/Tasks/Await content-type filter, a Today / 3 days / 7 days horizon filter, and permanent per-item suppression; create ad-hoc tasks (`+ Task`, 1-100 priority) and awaits (`+ Await`, date) inline, and reopen any ad-hoc item via its "↗" arrow to edit it, move it to another calendar, mark it done, or delete it in one click; calendar events climb in urgency as they approach and trump task/await priority once imminent or in progress, live re-ranking every minute — unless you give the event a priority (1-100, written as RFC 5545 `PRIORITY` for CalDAV clients), which caps its urgency so an unimportant event can never outrank an urgent task
- **Done view & clean-up** — Every completed item is indexed and listed on a dedicated **Done** tab in the navbar: markdown `#done` tags, note inline `#done` tokens, and completed CalDAV tasks, filterable by origin, source, and time, and searchable by title. Live items can be **Re-open**ed (undoing the completion, restoring the original tag/token/task in place), and **Clean up all matching** strips the `#done` tag/token (or deletes the VTODO) from the source while archiving the item as read-only history — so completed items never grow a file or list again. A **Clean up…** dialog sets per-source retention (keep newest N, or archive older than N days), an archive-retention window (0 keeps no history), and a one-click **Clear archived items**; a per-source age rule is applied automatically by a background sweep (hourly, and immediately on save), so items past their age leave the source without any manual clean-up. Re-opening an item from its source removes it from the index automatically. A task completed or deleted by an external CalDAV client (DAVx⁵, Thunderbird, …) is indexed immediately — it appears (or archives) on the Done tab at once instead of only after the next Done-view load.
- **Configurable priority curves** — Settings → Priority Curves lets you reshape, per curve kind (calendar event time-left, due-date time-left, note/task priority, and a calendar-event priority ceiling), how urgency maps to score via a drag-only graph with URGENT/SOON/UPCOMING/LATER threshold bands; click the line to add a point, drag to reshape, double-click to remove; defaults reproduce the built-in ranking exactly until you customize; applies everywhere urgency is scored — Focus and the Notes-tab priority slider
- **Await filter** — Await items are no longer a separate screen: Focus's content-type pill switches to `Await` to show just the date-based await items, colour-coded (purple = future, red = overdue) with topic and source pills
- **Notes tab** — Ad-hoc notes organised into colour-coded collections (private or shared with other users via email invitation, same accept/decline-link flow as calendar sharing); pin and search across all visible collections; a note can optionally have a short title, shown bold above its content on the card; any number of first-class **labels** — per-user entities (name + colour) reused across notes, unlike the single collection a note belongs to — shown as coloured chips on the card, picked or typed in the note editor (existing labels are offered as suggestions), and filterable from a "Labels" sidebar panel (on both Focus and the Notes tab) that toggles the notes carrying a label in and out of the grid, exactly like the "Note collections" panel; labels can be renamed, recoloured (a colour palette in the label editor), or deleted (deleting one just unlinks it, keeping the notes) from that panel's edit action, and a "+" button beside the "Labels" header creates one directly (name + colour), matching the "Note collections" panel's own "+"; A note can hold any number of inline tasks and awaits (see **Inline tasks & awaits in notes** below); the editor warns if the chosen calendar's shares don't match the note collection's. The note editor's collection picker always offers a "+ New" chip to create a collection inline (with its full color/sharing options) without losing the note being drafted, and remembers the last collection a note was saved to as the default for next time — so a brand-new account with no collections yet is prompted to create one instead of hitting a confusing authorization error. Notes sit in a free-form grid, independently per pinned/unpinned zone — pinned notes always form a contiguous block above unpinned ones (editing a note no longer bumps it to the top; pinning or unpinning places it in the first open slot of its new zone). Each note's card height is its own — two short notes stack directly on top of each other and together can match one tall neighbor's height in the next column over, instead of every note in a row being padded out to the tallest one. Drag a note card to any grid cell, empty or occupied — dropping on an occupied cell pushes the note(s) below it further down the same column instead of reordering everything, so removing a note leaves a gap instead of the rest shifting up; a pinned note can't be dragged into the unpinned zone (or vice versa; a "no-drop" affordance shows at the boundary). An "Auto-layout" button toggles a non-persisting, tightly-packed preview of a zone (closing all its gaps, ordered by any drag-assigned rank first and reading order after); toggling it back off just stops previewing, since nothing was ever written — a "Keep this layout" button appears alongside it while active to commit the current preview as each note's new manual position. Dragging while Auto-layout is on reorders instead of repositioning (persisting a rank between whichever notes it landed between, rather than a grid cell). Searching forces the preview on for the duration (so the filtered matches repack tightly instead of sitting wherever they landed in the full, unfiltered grid) and the toggle itself locks disabled until the search is cleared, since toggling it off mid-search would otherwise break dragging against notes hidden by the filter. The grid's column count adapts live to the available width (4 down to 1) instead of ever scrolling horizontally; resizing reflows manually-placed notes cohesion-aware — cards placed near each other stay visually grouped ("gaggles" that reflow their own shape rather than scattering across whichever real column each one originally landed in), and dragging at any width still resolves to an exact, persisted grid cell; when columns collapse, the gap where one column's cards join the next is preserved only up to roughly a card's height, so separate islands stay distinct without leaving a large empty hole. The collection sidebar shares its visual style (card sizing, bordered "add" button) with Focus's filter sidebar panels, and collapses behind a toggle on narrow screens like those panels do. A "+ New note" toolbar button opens the note editor (replacing the old in-grid "+ Take a note…" tile, so pinned notes can now use the full grid from row one); a "Clear all" toolbar button opens a dialog to bulk-delete either one chosen collection or every note you can see, with a checkbox (applying to whichever scope is picked) to also delete the collection(s) themselves rather than just empty them — disabled for a chosen collection you don't own, since only an owner can delete it
- **Note collections in Focus** — Tasks and awaits written inline in a note (see **Inline tasks & awaits in notes** below) surface alongside CalDAV and markdown items in Focus, tagged with a colour-matched "collection · note" pill and, alongside it, a chip per note **label** (coloured dot + name, same visual as the Notes tab's label chips) so the card shows what the task/await is tagged with, not just which collection it came from; a "Note collections" sidebar panel (alongside Calendars and Markdown Sources) toggles a collection's items in and out of Focus, and toggling a note **label** off in the "Labels" panel hides a note-origin task/await only once every label on its note is toggled off — a note tagged with one hidden and one visible label keeps showing, since the visible label wins (toggling the whole collection off still hides it regardless); completing a note-origin task rewrites its inline token to a `#done/<date>/...` record — reopenable from the note editor — while leaving the note itself visible, and deleting it (from the note editor or the Focus list) removes the token instead of orphaning the note. Each note-origin card's "↗ Go to source" hover action opens that exact note's editor, matching the arrow calendar events and markdown items already have (note-origin tasks in Focus show this arrow instead of a delete button, since deleting there would orphan the note rather than clearing its task/await flag properly); that editor's "Show in collection" button jumps to the Notes tab and briefly pulses the exact inline chip the item came from (in that chip's urgency colour), falling back to pulsing the note's card when the token can no longer be located, so it's easy to spot
- **@mention contact linking in notes** — Typing `@` in the note editor opens a filterable popup of your contacts, anchored right at the caret; arrow keys and Enter to pick one (or click), Escape to dismiss. A resolved `@Name` renders in the note card as a clickable pill; clicking it opens a small popover with that contact's full detail — nickname, org/title, birthday/anniversary, categories, emails, phones, URLs, IM handles, and addresses — without leaving the page, plus a "Go to contact →" link that jumps to the Contacts tab, scrolls the card to the center of the screen, and briefly pulses it (same highlight treatment a note gets when opened via Focus's "↗ Go to source"). An `@` that doesn't match a contact (e.g. inside an email address) stays plain text
- **Note images (inline attachments)** — Attach images (JPEG/PNG/GIF/WebP, up to 5 MiB each) to a note from its editor; each file is inserted at the cursor as a short `@img:<id>` reference that renders as an inline image in the note body on the card, so there's no markdown image syntax to type or read. Multiple images per note, in insertion order; staged picks are uploaded and their temporary references rewritten to real ids in the same transaction that saves the note, so a new note is never left half-created. Image bytes live in the database next to the note (atomic with backup/export/delete) and are streamed back through an authenticated `GET /api/notes/{note_id}/images/{id}` route rather than bloating the notes grid. Removing an image (its inline reference or its chip in the editor) deletes the stored bytes; deleting the note deletes them with it. Hovering an inline image dims it and reveals an expand icon; clicking opens it fullscreen in a lightbox (dismissed by backdrop click, ✕, or Escape). The stored `@img:<id>` token is deliberately resolvable through the note's image table, so a future markdown export can turn it back into a normal image link or attachment file
- **Clickable URLs in notes** — A bare `http://`, `https://`, or `www.` URL in a note's body renders as a real link (opening in a new tab) instead of plain text; trailing sentence punctuation and an unbalanced closing bracket are trimmed, while a balanced one stays, and clicking the link navigates rather than opening the note editor or starting a card drag
- **Inline tasks & awaits in notes** — A note body can carry any number of `#task/p<N>/<slug>` and `#await/<YYYY-MM-DD>/<slug>` tokens, authored inline the way images are: a "+ Task" / "+ Await" button opens a small modal (free-text name, slugified into the token; the shared priority slider, or a date), typing `#` opens a Task…/Await… chooser at the caret, and a full token can also be typed by hand (legacy `#todo`/`#pending` spellings still parse). Each token becomes its own VTODO — surfacing in Focus with the note's collection attribution and over CalDAV, exactly like a markdown tag — and the editor lists them as chips (remove, or reopen a finished one). A task/await chip on the note card carries a checkbox that completes it in place, and a done chip's checkbox re-opens it; every chip is a single line — urgency is a round coloured dot (coloured from your priority curves, matching Focus) beside the label, the label truncates with `…` and shows the full text on hover, and an await whose date has already passed turns its dot and outline red. Completing one rewrites its token to a self-describing `#done/<completed-date>/p<N>/<slug>` (or `.../<await-date>/<slug>`) record that keeps the original priority/date, so re-opening is a lossless rewrite; a `#done` record is inert (no task). The note's calendar is a single per-note choice (always settable in the editor) applied to all of them, so calendar sharing works as before. This replaces the old one-task-per-note model; existing task/await notes are migrated to an inline token at startup. Clicking a chip's body (not its checkbox) opens a dialog to rename it, change a task's priority / an await's date, or mark it done, writing the change straight back to the note's token; a done chip's body does nothing, leaving its checkbox as the reopen control. In the note editor, clicking an existing token in the text (or its chip in the "Tasks & awaits" row) opens that same dialog locally — rewriting the token in the draft rather than the stored note — and the `#` chooser never re-opens over an existing tag
- **Contacts tab** — A per-user address book: full name (given/middle/family plus prefix/suffix), nickname, organisation/title, birthday, anniversary, freetext notes, comma-separated categories/tags, any number of postal addresses (home/work/other, each markable as primary), any number of labelled (home/work/mobile/other) emails, phones, URLs, and IM handles per contact, each markable as primary, and an optional photo (JPEG/PNG, shown as a card avatar). Cards are sized to their own content and laid out in a free-form, masonry-packed grid that reflows its column count live with window width, cohesion-aware (cards placed near each other stay visually grouped as the column count changes, rather than scattering across whichever real column each one originally landed in, with a capped join gap so a collapsed column doesn't leave a large hole), and can be dragged to any position — sharing its underlying grid *and* drag-and-drop machinery with the Notes tab so the two stay visually and behaviorally consistent; a card's URL, if it has one, is a normal clickable link (opens in a new tab) rather than triggering the card's own click-to-edit. An "Auto-layout" button toggles a non-persisting preview of the grid packed tightly in alphabetical order by name (contacts are looked up by name, not recency, unlike Notes' reading-order-fallback repack), with a "Keep this layout" button to commit the preview as each contact's new manual position — same search-locks-the-toggle behavior, and dragging while the preview is active reorders (persisting a rank) rather than repositioning, as the Notes tab's Auto-layout. Search filters by any text a contact carries — the display name and its separate name parts, nickname, organisation, title, notes, categories, and every email, phone, URL, IM handle, and postal-address line. A trash icon on every card opens a small delete-confirmation dialog — the same pattern Notes uses for its own note cards. A toolbar "Clear all" button opens a dialog to bulk-delete either one chosen collection or every contact you can see, with a checkbox (applying to whichever scope is picked) to also delete the collection(s) themselves rather than just empty them — disabled for a chosen collection you don't own, and, when clearing every collection, always leaves your own last collection in place rather than deleting it, since a contact must always have somewhere to live; a "Merge" mode lets you select two or more duplicate contacts and combine them into one — repeatable fields (emails, phones, URLs, addresses, categories) are kept from every contact automatically, while conflicting scalar fields (name, org, birthday, etc.) and which photo to keep are resolved in a picker before merging. A dismissible banner also proactively suggests likely-duplicate groups (matching name plus a shared email/phone, or matching name alone when neither has any contact info on file) so you don't have to go looking for them
- **Contact collections & sharing** — Contacts are organised into named, colour-coded collections (private or shared with other users via the same email accept/decline-link flow as calendar and note-collection sharing) instead of one flat address book; a "Contact collections" sidebar panel on the Contacts tab toggles a collection's contacts in and out of the grid, matching the Notes tab's own collection sidebar. A shared collection's contacts become visible and editable to every member (single 'member' tier — full read-write, no read-only split), mixed into the same masonry grid as your own (each card shows its collection's colour as a stripe plus a small colour-dot label); creating a contact offers a collection picker that remembers the last one used. Settings → Sharing lists a third section for contact collections alongside Calendars and Note collections — owned, shared collections with one-click member revoke or bulk unshare, and collections shared with you with one-click leave. Each collection is also its own CardDAV address book (see CardDAV Protocol below) — owned and shared collections both appear in address-book-home-set discovery
- **Contacts import** — Import a multi-contact `.vcf` file into your address book from Settings → Import, alongside calendar import; a preview shows how many contacts were found and how many look like duplicates of an existing contact (matched by name plus a shared email/phone) before you commit, with a skip/overwrite choice for duplicates and a final imported/skipped/failed count
- **Contacts export** — Download your contacts as a standard `.vcf` file (one concatenated file, however many contacts it holds) from Settings → Export, alongside calendar export, or select one or more contacts on the Contacts tab in "🔗 Merge" mode and use "⬇ Export (N)" for just those — the same vCard format contacts import reads back, so it round-trips (photos included) into another address book app or back into tidl
- **CardDAV Protocol** — Standards-compliant RFC 6352 read-write contact sync, mirroring the CalDAV setup below (same app-token auth, `sync-collection` for incremental sync). Every contact collection — owned or shared — is its own address book, discovered via `addressbook-home-set` at `/carddav/username/`; each is addressed as `/carddav/username/book-{collection_id}/`, the same uniform aliasing scheme for owned and shared collections (no separate "shared" URL shape). **Note:** this address-book-per-collection scheme replaced a single fixed `/carddav/username/contacts` path — existing CardDAV client configurations need to re-discover the server (delete and re-add the account, or re-run auto-discovery) to pick up the new per-collection book paths; create, edit, and delete contacts (including photos) from any CardDAV client (DAVx⁵, Apple Contacts, Thunderbird) and it stays in sync with the Contacts tab

### Markdown & Notes Integration

- **In-app editor** — Browse and edit your markdown notes directly in the web UI, or open them in your local editor of choice: a remote browser can open its own git clone via an editor URL scheme (configured per device in Settings → This Device), while a browser running on the tidl host can open the server's clone with no extra setup. "Pull latest" forces an immediate git pull after you commit and push, and external edits to the server copy can be committed and pushed from the preview. Typing `@` opens the same contact-mention typeahead Notes has (backlog #80); a resolved `@mention` in the rendered preview opens the same contact-detail popover. Typing `#` opens the same Task…/Await… chooser Notes has — picking one inserts a new `#task/p<N>/<slug>` or `#await/<date>/<slug>` tag at the caret (the file is written on Save, as usual) — and clicking an existing tag opens its edit dialog prefilled, rewriting just that tag in place — its "Mark as done" checkbox completes the item to a `#done/...` record without leaving the editor (a `#done` record opens nothing)
- **Task and await extraction** — `#task` and `#await` tags in your notes surface as tasks and await items; each renders in the markdown preview as the same single-line chip used in notes — a round urgency dot coloured from your priority curves, no `High/Medium/Low` text label — with a checkbox. A tag renders as a chip whether it sits on its own line or inline with surrounding prose (e.g. trailing a list item), and several tags on one line each get their own checkbox. Completing one (there, or from Focus's list checkboxes) rewrites the tag to a rich `#done/<completed>/p<N>/<slug>` (or `...#done/<completed>/<due>/<slug>` for an await) that keeps the original priority/due date, and the preview's done checkbox un-checks it back to the original tag — so a mistaken check-off is fully recoverable. Clicking a chip's body (not its checkbox) opens the same task/await dialog used for ad-hoc items, prefilled from the source — rename it, change a task's priority or an await's date, or mark it done, and saving rewrites the markdown line or note text in place (done chips open nothing; their checkbox stays the reopen control)
- **Wikilink navigation** — `[[wikilink]]` in the markdown preview opens linked files in-place with a back stack
- **Consolidate notes ↔ markdown files** — Move content across the database/files boundary in either direction. From a note's editor, **Consolidate to Markdown** writes it as a new file in a chosen source and folder (a source-scoped folder browser, a file-name field defaulting to a slugified title, and a keep-or-delete-the-note choice); the note's title becomes a leading `# Title` and its inline images are written as sidecar files in a sibling `<stem>.assets/` folder, with the `@img:<id>` tokens rewritten to normal markdown image links that render in the preview (served through an authenticated route, since the files live on the server). From the rendered markdown preview, **Pull out into Note** imports the file into a chosen collection (a leading `# Title` becomes the note title, local relative images are re-imported as note images, and you can keep or delete the file). `#task`/`#await`/`#done` tokens, `[[wikilink]]`s, and `@mention`s survive the move (both sides already share the same grammar), git-backed sources are committed and pushed, and an existing file name is reported as an error rather than overwritten
- **Go-to-source pill flash** — opening a markdown task/await's preview from its "↗ Go to source" arrow scrolls to and briefly pulses just that item's rendered pill (not the whole line); a note-origin item's "Show in collection" jump pulses its inline chip the same way (falling back to the note card when the token can't be found)
- **Source backends** — Local filesystem and git-backed repositories; sources are configured from the web UI
- **File finder** — `Ctrl/Cmd+K` or the 🔎 navbar button opens a telescope-style fuzzy finder over every file across your enabled markdown sources; typing filters the list live, and picking a file opens it in the same rendered markdown preview used elsewhere in the app
- **Google Keep import** — Import a Google Takeout Keep `.zip` from Settings → Import (no need to extract it first): each Keep note's title, text, pinned state, and timestamps become a tidl note, landing in a collection you pick; notes that were shared in Keep are grouped automatically into **"Shared with <email>"** collections (the archive names only the other people, so this captures who a note was shared with, in either direction — tidl can't auto-share, but the grouping makes those collections easy to share manually) — unless you untick **"Create separate collections for notes shared in Keep"** (ticked by default), in which case shared notes land in the chosen target collection alongside everything else and no extra collections are created. Keep `labels[]` are imported too — each becomes a first-class note **label** (tidl's own per-user label entities, resolved or created on import), so a multi-label note keeps every label. The "Import into" picker also offers **"Add new collection…"** (with a name field), preselected when you have no collections yet, so an import works straight from a fresh account and reports a missing target inline instead of leaving the button silently disabled. Checklist items become inline note tasks (unchecked → `#task`, checked → a `#done` record), so they surface in Tasks instead of staying flat text — but when a checklist was really just a list (a packing or shopping list), the note editor's **Bulletize** button turns every checklist token back into a plain markdown `- ` bullet in one click (dropping the task state and keeping the item's text), so it stops cluttering Tasks/Focus/CalDAV; the button is a toggle, reading **Taskify** whenever the note instead holds bullets and turning those back into default-priority (`p50`) tasks. Image attachments are re-attached as note images (JPEG/PNG/GIF/WebP, up to 5 MiB each). Trashed notes are always skipped, archived ones optionally; re-importing the same archive drops notes already imported (matched by their stable Keep creation timestamp, never overwritten), and the whole import runs in one transaction and reports an imported/images/labels/skipped/already-imported/failed summary

### Clients & Compatibility

- **Mobile** — Works with any standard CalDAV client (iOS, DAVx⁵, Thunderbird); swipe navigation in the web UI
- **Windows** — Cross-compile to a native `.exe` from Linux/NixOS; all browser hydration tests pass under Wine
- **Outlook COM bridge** — PowerShell script (`tools/outlook-bridge/`) that reads the local Outlook calendar via COM automation and pushes events to a local tidl CalDAV endpoint — no install, no admin rights, scheduled via Task Scheduler

### Infrastructure

- **Modern web UI** — Built with Leptos, served from the same binary; Escape closes the topmost open dialog (nested sub-dialogs dismiss one layer at a time), matching its ✕/Cancel/backdrop-click close
- **App tokens** — Named per-client credentials for CalDAV access; revocable without changing your password
- **User registration** — Self-service sign-up flow with email verification
- **Profile editor** — Click your username to edit display name, username (CalDAV address), email, and password; email changes require your current password and a confirmation code sent to the new address (with a heads-up notice to the old one), and password changes sign you out of every other device
- **Optimistic concurrency** — ETag-based, no complex locking
- **Lightweight** — Single executable, SQLite storage, minimal dependencies
- **Packaged** — Official multi-arch Docker image on `ghcr.io`, plus prebuilt Linux/Windows release artefacts and NAS/Podman deployment templates
- **FOSS** — Free and Open Source Software

## Quick Start

### Using Nix (Recommended)

```bash
# Enter development environment
nix develop

# Run in development mode with hot reload
# (binds 0.0.0.0:3060, so it's reachable from other devices on your network)
cargo leptos watch

# Build for production
nix build
```

### Using Cargo

```bash
cargo install cargo-leptos
cargo leptos watch        # dev server
cargo leptos build --release
```

### Deploy with Docker

The quickest way to run tidl is the published multi-arch image
(`linux/amd64`, `linux/arm64`) on GitHub Container Registry. Everything is
configured with `TIDL_*` environment variables, and the only persistent path is
`/data`.

```bash
# One-time: a secret to protect the SMTP password and stored CalDAV tokens.
# (Any 32-byte base64 key works; `openssl rand -base64 32` is equivalent.)
tidl-server generate-secret

# Start it (edit docker-compose.yml first — set the image org and TIDL_DOMAIN).
docker compose up -d

# Create the first admin, then open http://localhost:3060
docker compose exec tidl tidl create-admin \
  --email you@example.com --username you --display-name You \
  --password 'choose-a-strong-password'
```

- [`docker-compose.yml`](docker-compose.yml) — the Compose example (data volume,
  secret, `restart: unless-stopped`, journald logging).
- [`Dockerfile`](Dockerfile) — build the image from source without Nix.
- [`deploy/`](deploy/) — templates for unRAID, TrueNAS SCALE, and rootless
  Podman/Quadlet.

To build the image from this checkout instead of pulling:

```bash
nix build .#docker && docker load -i result   # via the Nix flake
# or
docker build -t tidl .                        # via the Dockerfile
```

### Windows cross-compile (from Linux/NixOS)

The Nix dev-shell includes a MinGW64 cross-toolchain and Wine so you can build and test the Windows `.exe` without leaving Linux:

```bash
# Cross-compile the server for Windows (produces tidl-server.exe).
# The WASM frontend is built by cargo-leptos in the same step.
build-windows

# Smoke-test the .exe locally via Wine against your existing data:
run-tidl-wine 3060 ./data

# Run the full browser hydration test suite against the Wine server:
test-hydration-wine admin@localhost admin
```

Packaging into a distributable release (binary + web assets + launcher):

```bash
dist-windows   # → dist/tidl-windows-<version>.zip
dist-linux     # → dist/tidl-linux-<arch>-<version>.tar.gz
```

Prebuilt Linux (`x86_64`/`aarch64`) and Windows artefacts are published on
GitHub Releases by CI on each version tag.

## Configuration

```bash
cp config.toml.example config.toml
```

```toml
[server]
host = "127.0.0.1"
port = 3060
domain = "tidl.dev"
# Required — protects the SMTP password and any remote CalDAV personal
# access tokens stored (encrypted) in the database. Generate with:
# tidl-server generate-key. Can also be set via TIDL_SECRET/TIDL_SECRET_FILE.
secret = "..."

[paths]
data_dir = "./data"

[auth]
users = [
    { username = "admin", password_hash = "$2b$12$..." }
]

[sync]
enabled = true
watch_interval_secs = 5
```

## Markdown Integration

tidl extracts tasks and await items from your markdown notes and keeps them in sync with your CalDAV tasks.

### Tag formats

```markdown
# My Project Notes

- #task/p100/implement-caldav       → Priority 100 task
- #task/p0/critical-bug             → Priority 0 (highest)
- #task/2025-06-01/renew-passport   → Dated task, no priority
- #await/2025-01-15/call-dentist    → Await item for Jan 15, 2025
- #done/2025-01-20/p0/critical-bug   → Checked off (priority + completion date kept)
- #done/2025-01-20/2025-01-15/call-dentist → Checked-off await (due + completion date kept)
```

- `#task/p<priority>/<slug>` — Priority 1 (highest) to 100 (lowest); `0` is tolerated and clamped to `1`, anything above 100 clamps to 100 (a warning is logged for either), so `#task/p0/<slug>` keeps behaving like `#task/p1/<slug>`; slug uses hyphens for spaces
- `#task/<YYYY-MM-DD>/<slug>` — Dated task with no explicit priority (sorts after prioritized tasks)
- `#await/<YYYY-MM-DD>/<slug>` — Await item for a specific date (dormant until then, no priority of its own)
- `#done/<completed>/p<N>/<slug>`, `#done/<completed>/t<due>/<slug>`, `#done/<completed>/<due>/<slug>` — a checked-off task/dated-task/await (written automatically on completion). Keeps the original priority or due date plus the completion date; not parsed as a task or await, so it drops out of Focus but stays in the file as a record. Its preview checkbox is live: un-checking rewrites it back to the original tag. Older `#done/<slug>` records still render (checked, disabled) but can't be re-opened — their original parameters were never stored.

A tag can sit anywhere on a line: on its own (rendered as a block-level chip, so consecutive tag lines stay separate), trailing a list item's text, or inline in a sentence — surrounding text is ordinary markdown and renders independently of the chip. Several tags on one line each become their own chip, their own task/await in Focus and over CalDAV, and completing/re-opening targets the exact one you clicked.

### Sources

Markdown sources are configured from the **Settings → Markdown Sources** page in the web UI. Currently supported:

- **Local filesystem** — Watch a local directory (optionally recursive); the path can be typed directly or chosen with the built-in server-side folder browser
- **Git repository** — Clone and watch a remote git repo; authenticate with a personal access token stored encrypted at rest

## Mobile Setup

Connect any CalDAV-compatible app:

- Server: `https://your-domain.com`
- URL: `https://your-domain.com/caldav/username/calendar`
- Username / Password: your tidl credentials

Tested with iOS Calendar & Reminders, Android DAVx⁵, and Thunderbird.

Connect any CardDAV-compatible address book app the same way:

- Server: `https://your-domain.com/carddav/username/` (address-book-home-set discovery lists every collection you own or are a member of — most clients auto-discover from this base URL)
- Username / Password: your tidl credentials, or an app token from Settings → App Tokens

Tested with iOS Contacts and Thunderbird; DAVx⁵ contact sync is expected to work (same protocol, same server) but hasn't been verified yet.

## Web Interface

Access at `http://localhost:3060`:

- **Calendar** — Day / Work week / Week / Month views with event management, drag-to-resize, all-day events, and an adjustable hour scale (zoom via −/+ buttons or Ctrl/⌘ + wheel); a header row with today's date heading on the left, the segmented view-mode pill centred, and a search box on the right (filters the grid to matching events); the event dialog's time fields pair free-text `HH:MM` entry with a custom click-to-pick popup (15-minute marks, scrolls to the current value) across all three time formats, editing Start shifts End by the same delta to preserve duration, and End can't land at or before Start on the same day
- **Focus** — Same header shape as Calendar: `+ Task` / `+ Await` buttons on the left, and the All/Calendar/Tasks/Await pill, the Today / 3 days / 7 days horizon pill, and a search box grouped on the right; a 13px one-line summary sits underneath ("All items · Ranked by urgency · 5 remaining"). Urgency-ranked unified list fusing events, tasks, and await items, including task/await-flagged notes; `+ Task` and `+ Await` create ad-hoc CalDAV items inline (plain VTODOs in the selected calendar, not derived from a markdown file or note), and every ad-hoc card's "↗" arrow reopens the same dialog (with the calendar picker) to edit, move, mark it done, or one-click delete it; permanently suppress items (with a series/occurrence choice for recurring events), always visible in a collapsible "Suppressed" section for easy undo; source preview overlays with "Open in editor" button; a calendar event opens the full edit-event dialog (with a "Show in calendar" button that jumps to the event's week and briefly pulses it); hover, focus, or tap a card's urgency cluster to see why it scored what it did — a popover lists each contributing input (task priority, due-date or event-start proximity, an event's priority cap, the in-progress floor) with the value your Priority Curves produced and highlights the one(s) that determined the rank; imminent/in-progress calendar events outrank task and await priority, with the list re-ranking live every minute
- **Notes** — Ad-hoc notes in colour-coded, optionally shared collections; pin, search, promote a note to a task or await item, attach inline images (stored in the database, rendered in the note body), render markdown `- `/`* `/`+ ` bullet lists (nested by indentation, with inline tasks/awaits, mentions, URLs, and images still working inside an item, and a one-click **Bulletize**/**Taskify** toggle that moves between checklist tokens and bullets), see bare URLs as clickable links, and import a Google Keep Takeout archive (title, text, checklist state, labels, pinned state, timestamps, and image attachments)
- **Done** — Central list of completed items across markdown sources, notes, and calendars, with a live count badge on the navbar link, search, an origin filter, a "Show archived" toggle (archived history), per-item **Re-open**, a per-item **Archive** hover action (strips that one item from its source behind a confirm that names the source), a bulk **Clean up all matching**, and a **Clean up…** dialog for per-source retention (applied automatically by a background sweep), archive-retention, and clearing archived items
- **Contacts** — Per-user address book, organised into colour-coded, optionally shared collections, with rich contact records (full name components, nickname, org/title, birthday, anniversary, categories, multiple addresses, multiple labelled emails/phones/URLs/IM handles, an optional photo) in a responsive, content-sized, drag-to-reorder card grid; search, create, edit, and delete; also syncs over CardDAV to any compatible client
- **Note Editor** — In-app markdown editor for browsing and editing your note sources; wikilink navigation with a back stack
- **Settings** — Calendars, sharing, app tokens, markdown sources, SMTP, user preferences (timezone, time format, dark mode, editor command), priority curves (drag-editable urgency-scoring graphs per curve kind); admin-only Users tab to list, create, promote/demote, activate/deactivate, reset the password of, and permanently delete user accounts

## Versioning & Releases

tidl follows [Semantic Versioning](https://semver.org/) for the product as a
whole, with the version defined once in the root `Cargo.toml`.
`tidl-server --version` — and the footer of the web UI — report it together with
the git revision and build date. Releases are tagged `vX.Y.Z`; each tag publishes
the prebuilt artifacts and the multi-arch Docker image, with release notes taken
from [`CHANGELOG.md`](CHANGELOG.md). The procedure is documented in
[`docs/releasing.md`](docs/releasing.md).

## NixOS Deployment

```nix
{
  inputs.tidl.url = "github:puckles7/tidl";

  outputs = { self, nixpkgs, tidl }: {
    nixosConfigurations.yourhost = nixpkgs.lib.nixosSystem {
      modules = [
        tidl.nixosModules.default
        {
          services.tidl = {
            enable = true;
            host = "0.0.0.0";
            port = 3060;
            domain = "tidl.yourdomain.com";
            dataDir = "/var/lib/tidl";

            # Required — protects the SMTP password and any remote CalDAV
            # personal access tokens stored (encrypted) in the database.
            # Generate with `tidl-server generate-key`.
            secretFile = config.sops.secrets."tidl-secret".path;

            smtp = {
              enable = true;
              host = "smtp.yourprovider.com";
              username = "tidl@yourdomain.com";
              passwordFile = config.sops.secrets."tidl-smtp-password".path;
              fromAddress = "tidl@yourdomain.com";
            };
          };

          services.nginx.virtualHosts."tidl.yourdomain.com" = {
            enableACME = true;
            forceSSL = true;
            locations."/" = {
              proxyPass = "http://127.0.0.1:3060";
            };
          };
        }
      ];
    };
  };
}
```

## Development

### Project Structure

```text
tidl/
├── crates/
│   ├── tidl-server/      # Axum binary — auth, HTTP, database, state
│   ├── tidl-caldav/      # CalDAV/WebDAV protocol library
│   ├── tidl-carddav/     # CardDAV protocol library (vCard + REPORT/PROPFIND XML)
│   ├── tidl-markdown/    # Markdown parsing and source backends
│   └── tidl-web/         # Leptos web UI (compiled to WASM + SSR)
├── migrations/           # SQLite database migrations
├── flake.nix             # Nix development environment
└── Cargo.toml            # Workspace definition
```

### Running Tests

```bash
# Unit and integration tests
cargo test --workspace

# Lint and format checks
cargo clippy --workspace -- -D warnings
cargo fmt --check

# Verify the WASM target builds cleanly (required after any tidl-web or dependency change)
cargo build --target wasm32-unknown-unknown -p tidl-web

# Browser hydration tests (requires Wine or a running server)
test-hydration-wine admin@localhost admin
```

Tests are organised by layer:
- **Library unit tests** — `#[cfg(test)]` blocks inside `tidl-caldav` and `tidl-markdown`
- **Integration tests** — `tokio::test` + `sqlx::test` in `tidl-server/tests/`; spin up a real in-memory database
- **WASM build check** — ensures no server-only types leak into the browser bundle
- **Browser hydration tests** — Playwright suite against a running server; run under Wine for CI on Linux

## Architecture

- **Storage** — SQLite for metadata; iCalendar files on disk
- **Auth** — HTTP Basic Auth (use a reverse proxy with TLS in production); app tokens for CalDAV clients
- **Concurrency** — ETag-based optimistic locking
- **UI** — Server-side rendered Leptos with progressive enhancement; single binary serves both the API and the frontend

## Security Note

This server uses HTTP Basic Auth. Always deploy behind a reverse proxy with TLS (nginx, Caddy, Traefik, etc.).

## License

Licensed under either of Apache License, Version 2.0 or MIT license, at your option.

## Contributing

Contributions welcome. This project aims to be hackable and understandable.
