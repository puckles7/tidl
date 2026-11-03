# tidl — Personal time management for people who live in their notes

A CalDAV server that bridges your calendar and tasks with your markdown knowledge base. One binary, SQLite storage, built in Rust.

## Features

### Calendar

- **CalDAV** — RFC 4791 with incremental `sync-collection` (RFC 6578); multiple colour-coded calendars.
- **Sharing** — Share calendars with other users (read or write) via email invitation, with live SSE updates on both sides.
- **Sync & import** — Import `.ics` files, subscribe to webcal feeds, and mirror a remote CalDAV server bidirectionally.
- **Events** — Recurring events, all-day/multi-day events, VALARM reminders (browser and email), and iMIP invitation import.
- **Views** — Day / Work week / Week / Month, with drag-to-resize and adjustable hour zoom.

### Tasks & Planning

- **One priority scale** — 1–100 everywhere, projected onto CalDAV's native 1–9 `PRIORITY` at the wire boundary.
- **Focus** — Urgency-ranked list of events, tasks, and awaits, with content/horizon filters, inline creation, per-item suppression, and an explainable score breakdown.
- **Done** — Completed items from markdown, notes, and CalDAV in one place, with reopen, archive, and per-source retention rules.
- **Priority curves** — Drag-editable graphs that reshape how urgency is scored.
- **Flexible sources** — Tasks work as standalone calendar items or inline in notes and markdown; awaits and pending calendar events are date-driven items that surface together in Focus.

### Notes & Contacts

- **Notes** — Colour-coded, optionally shared collections; pin, search, labels, inline images, clickable URLs, and `@mention` contact linking.
- **Contacts** — Rich vCard records in colour-coded, optionally shared collections, with import/export and CardDAV sync.
- **Google Keep import** — Import a Takeout archive, including labels, checklists, pinned state, and images.

### Markdown

- **Editing** — In-app editor, or open the server's clone in your own editor; filesystem and git-backed sources.
- **Task extraction** — `#task` / `#await` tags become real tasks; completing one rewrites it to a recoverable `#done` record.
- **Navigation** — `[[wikilinks]]` with a back stack and a `Ctrl/⌘+K` fuzzy file finder.
- **Consolidate** — Move content between notes and markdown files in either direction.

### Clients & Infrastructure

- **Clients** — Works with iOS, DAVx⁵, and Thunderbird over CalDAV/CardDAV; a Windows build and an Outlook COM bridge are included.
- **Lightweight** — Single Rust binary, SQLite storage, ETag-based optimistic concurrency.
- **Packaged** — Multi-arch Docker image on `ghcr.io`, prebuilt Linux/Windows releases, and NAS/Podman templates.
- **FOSS** — Free and Open Source Software.

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
# One-time: generate the secret that protects stored SMTP passwords and
# CalDAV tokens. Any 32-byte base64 value works.
openssl rand -base64 32 > tidl-secret
# (or, with the tidl binary installed: tidl-server generate-secret)

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

### Server secret

tidl requires a persistent 32-byte secret to encrypt stored SMTP passwords and
remote CalDAV tokens, and refuses to start without one. Generate it once and
keep it out of version control:

```bash
# Print a key to paste into config.toml (`server.secret`) or TIDL_SECRET
tidl-server generate-key

# Or write it straight to ./tidl-secret for TIDL_SECRET_FILE / Docker
tidl-server generate-secret          # --output <path>, --force to rotate

# Without the tidl binary:
openssl rand -base64 32 > tidl-secret
```

`TIDL_SECRET_FILE` takes precedence over `TIDL_SECRET`. Rotating the secret
invalidates every stored SMTP password and CalDAV token.

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

- **Calendar** — Day / Work week / Week / Month views with event management, drag-to-resize, all-day events, search, and an adjustable hour zoom.
- **Focus** — Urgency-ranked list of events, tasks, and awaits, with content/horizon filters, inline `+ Task` / `+ Await` creation, per-item suppression, and a score explainer.
- **Notes** — Colour-coded, optionally shared collections with pin, search, labels, inline images, and inline tasks/awaits; imports Google Keep archives.
- **Done** — Completed items from markdown, notes, and calendars, with search, origin filtering, reopen, archive, and retention clean-up.
- **Contacts** — Address book in colour-coded, optionally shared collections, with rich contact fields and CardDAV sync.
- **Note Editor** — In-app markdown editor with wikilink navigation and a back stack.
- **Settings** — Calendars, sharing, app tokens, markdown sources, SMTP, preferences, and priority curves; admins also get user management.

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
