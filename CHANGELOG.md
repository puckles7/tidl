# Changelog

All notable changes to tidl are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While tidl is pre-1.0 (`0.x`), a breaking change bumps the **minor** version
(`0.MINOR.0`), not the patch.

The version is defined once, in the root `Cargo.toml`; the release procedure is
documented in [`docs/releasing.md`](docs/releasing.md).

## [Unreleased]

### Changed

- Focus' "Show in calendar" now centers the highlighted event vertically in the
  week grid instead of pinning it to the top edge. (#157)
- Focus' "Show in calendar" navigates client-side instead of reloading the whole
  app, so the calendar opens as fast as the Calendar tab link and cached events
  remain on screen. (#157)
- Web assets are now served gzip/brotli-compressed, with precompressed
  `.br`/`.gz` siblings in release builds, so the WASM bundle downloads much
  faster on slow connections. (#165)

## [0.2.0] - 2026-11-06

### Added

- Users can again be provisioned declaratively via an optional `[[users]]` list
  in `config.toml` (`email`, `username`, `display_name`, `password_hash`,
  `is_admin`). Entries are created at startup only when no account with the
  same email or username exists, so UI changes survive restarts. (#02)

### Changed

- "Open in editor" is now configured in one place, Settings → **Editors**
  (formerly "This Device"). Git-backed sources always open from your own clone
  on this device and never from tidl's managed copy; filesystem sources can be
  opened externally only when the browser runs on the tidl host, with a
  disabled action and hover explanation otherwise. The separate "Server editor
  command" setting is gone — each editor supplies a default server-side command,
  with an optional override under Advanced for launchers whose binary name or
  path differs by platform or packaging (e.g. `code.cmd` on Windows, Flatpak or
  Snap wrappers). Existing per-user commands keep working as a fallback, and a
  missing launcher now falls back to the host's default file opener instead of
  failing with "No such file or directory". The preview offers an inline,
  source-scoped "Set up clone…" prompt, clone
  paths must be absolute, SSH/HTTPS spellings of the same remote are
  reconciled, and a `Test` link opens the clone root in your editor. Editors
  with no URL-scheme template show a disabled open action with a
  `Configure editor…` shortcut and keep `Copy path`/`Edit in browser` as
  fallbacks. (#164)
- CalDAV and CardDAV sync is verified bidirectionally with DAVx⁵ (Android
  AppSupport) and natively on Sailfish OS (Jolla Phone 2026). (#31, #85)

### Fixed

- `config.toml.example` no longer documents the obsolete `[auth]` users table
  (accounts live in the database) and now explains creating the first admin
  with `create-admin` and that login is by email, not username. Also documents
  the supported `allow_registration`, `admin_email`, `encryption_key_file`,
  `[inbound_smtp]`, and `[oidc]` options. The unused `server.log_level` key
  was dropped; set `RUST_LOG` instead. (#02)

## [0.1.0] - 2026-11-02

### Added

- The markdown editor's task/await edit dialog has a "Mark as done" checkbox:
  saving with it checked rewrites the tag to the usual rich `#done` record
  (`#done/<today>/p<N>/<slug>`, or the dated-task/await variants), preserving
  its original parameters and using your local date. (#162)
- Clicking a completed `#done` item in the markdown editor now opens the same
  edit dialog with "Mark as done" checked; unchecking it and saving restores
  the original task/await with its parameters, keeping the original completion
  date if it is re-checked. (#132)
- Focus's urgency cluster now explains its score on hover, keyboard focus, or
  tap: a small popover lists every input that fed the score — task priority,
  due-date proximity, event start proximity (including the in-progress floor),
  and an event's priority cap — showing the value your Priority Curves produced
  for each and highlighting the one(s) that actually determined the rank. (#158)
- Calendar events can carry a 1-100 priority (opt-in, written as RFC 5545
  `PRIORITY` and shared with CalDAV clients like any other event property).
  A prioritised event's Focus urgency is capped by a new fourth Priority
  Curves entry, so an unimportant event can never outrank an urgent task;
  recurring occurrences inherit the series priority and a "this occurrence"
  edit keeps its own copy. (#156)
- "Open in editor" now works when the browser is not on the tidl host: Settings →
  This Device stores a per-browser editor link and local clone paths, so a git
  source can open its checkout on the user's machine instead of running an
  editor command on the server. Same-machine setups keep the zero-config
  server-copy path, and "Pull latest" forces an immediate pull from the preview.
  Editing the server copy externally also surfaces a "Commit & push (N)" action
  for the clone's uncommitted changes. (#23)
- `tidl-server generate-secret` writes the deployment secret (base64, 32 bytes)
  straight to a file (`./tidl-secret` by default) for `TIDL_SECRET_FILE`,
  refusing to overwrite unless `--force` — no `openssl` or shell redirection
  needed. (#154)
- Official multi-arch Docker image (`ghcr.io`, `linux/amd64` + `linux/arm64`),
  a `docker-compose.yml` example, a from-source `Dockerfile`, and NAS/rootless
  Podman deployment templates under `deploy/`. (#149, #152)
- Prebuilt Linux (`x86_64`, `aarch64`) and Windows release artifacts, built and
  published to GitHub Releases by CI on every version tag. (#148)
- `tidl-server --version` reports the semantic version plus the git revision and
  build date, and the web UI shows the running version in a footer. (#147)

### Changed

- Clicking a calendar event in Focus now opens the full edit-event dialog
  instead of a read-only preview, so events can be inspected and edited
  without leaving Focus. The dialog gained a "Show in calendar" button that
  jumps to the event's week and briefly pulses it. (#157)
- The product version is now a single source of truth: it lives only in the root
  `Cargo.toml` and is read back by the Nix flake, so the package, the image, and
  the release artifacts cannot drift. (#147)
- The task/await edit dialogs' "Mark as done" action is now a checkbox applied
  by Save (matching the markdown editor's dialog), rather than completing the
  moment it is ticked. Awaits, which have no completion, get a "Remove await
  tag" checkbox instead. (#132)

### Fixed

- The markdown file finder now puts the caret in its search box as soon as it
  opens (via `Ctrl/Cmd+K` or the 🔎 button), so you can start typing to filter
  files immediately without clicking the input first. (#75)
- Saving a task with "Mark as done" ticked in the shared edit modal now updates
  its rendered pill in the markdown preview immediately instead of requiring a
  manual refresh. (#132)

- The markdown viewer no longer swallows task/await tags that follow a
  single-line triple-backtick code span (e.g. `` ```systemctl start cbs``` ``):
  such a line was mistaken for an opening code fence, leaving the rest of the
  file rendered as literal code instead of chips. (#161)
- Calendar, Focus, Done, Notes, and Contacts toolbars now wrap their controls
  onto additional lines instead of pushing off the right edge of the screen on
  narrow desktop windows; the search box shrinks first, then wraps only when
  even its minimum width no longer fits. Calendar's date, view pills, and
  search also stay visible in mobile mode above the calendar and below the
  filter panels instead of being hidden. (#159)
- On mobile, the Notes and Contacts filter bars now use the same compact
  collapsible card as Calendar and Focus, and all five tabs start their filter
  bar at the same vertical offset instead of Notes/Contacts/Done sitting lower
  than the rest. (#160)
- Removing a `#task`/`#await` tag in the in-app markdown editor now drops that
  item from the Focus list as soon as the save completes, instead of leaving it
  there until a manual page refresh. (#34)
- "Open in editor" no longer fails with "invalid source config" for git-backed
  markdown sources: the file path is resolved the same way as every other file
  operation (git sources live under `<data_dir>/repos/<id>/`). (#23, #34)
- `systemctl stop` (SIGTERM) now shuts the server down gracefully: in-flight
  requests drain, SSE streams close, and the SQLite WAL is checkpointed back
  into `tidl.db` so no `-wal`/`-shm` files are left behind. A stopped server's
  database is now self-contained, making a plain copy of `tidl.db` a valid
  backup. (#153)
- Consolidating a note to markdown now preserves its line breaks: a single
  newline becomes a hard break instead of rendering as a space, so a note with
  one word per line no longer collapses into a run-on sentence. Blank lines,
  existing hard breaks, and fenced code blocks are left untouched. (#155)
