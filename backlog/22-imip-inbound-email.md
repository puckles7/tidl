# iMIP inbound email — meeting invitation import

Allow users to import meeting invitations by manually forwarding them to a
personal tidl inbound address. Each user gets one or more unique inbound
addresses scoped to a specific calendar; tidl listens for SMTP, parses the
`text/calendar` MIME part, and upserts or deletes the event via the existing
import pipeline.

## Background

Meeting invitations arrive as standard MIME emails with a `text/calendar`
attachment carrying `METHOD:REQUEST` (RFC 6047 iMIP). This is true even for
providers that use end-to-end encryption between their own users, because
corporate senders (Exchange, Google Workspace) always send plain SMTP.

**Intended usage model:** the user manually forwards individual invites they
want to track. This keeps the calendar clean — only consciously chosen events
appear. Automatic forwarding rules are explicitly not recommended: they would
import every invite including ones the user declines, cluttering the calendar
with irrelevant events.

For heavy corporate calendar use, the Outlook COM bridge (#25) or webcal sync
(#18) are the right tools. iMIP forwarding is for occasional personal use.

## Functional Requirements

- Each user can create one or more inbound addresses scoped to a specific
  calendar, e.g. `tidl-<token>@calendar.example.com`. The token routes the
  email to the correct calendar without exposing the username.
- Users register one or more trusted sender addresses in Settings / Profile.
  These are the addresses they forward from. Multiple addresses are supported
  (e.g. work and personal).
- tidl runs an SMTP listener (configurable port, separate from the HTTP port)
  with **mandatory STARTTLS** — any session that sends `MAIL FROM` before
  completing STARTTLS receives `530 5.7.0 Must issue a STARTTLS command first`.
- On receipt, tidl:
  1. Rejects messages exceeding the configured max size (default 1 MB) during
     DATA — return SMTP 552.
  2. Validates the token in the recipient address against the
     `inbound_email_tokens` table — return 550 if invalid.
  3. Checks `Return-Path:` against the user's registered sender addresses;
     rejects with 550 if none match. (`From:` is not used — it is trivially
     spoofable.)
  4. Verifies SPF for the `Return-Path:` domain via the `viaspf` crate.
     Rejects with 550 on SPF fail or error. DKIM is intentionally not checked —
     email forwarding typically breaks DKIM signatures.
  5. Enforces rate limiting: max 20 messages per token per rolling hour
     (configurable); returns SMTP 452 when exceeded.
  6. Returns `250 OK` to the MTA. Steps 1–5 are synchronous (before 250);
     steps 7–9 run in an async task after 250 to keep the connection short.
  7. Extracts the first `text/calendar` MIME part via `mail-parser`.
  8. Parses `METHOD:REQUEST` → upserts the event in the token's target calendar
     (match on UID; duplicate requests are treated as updates).
     Parses `METHOD:CANCEL` → deletes the matching event by UID.
     Messages with no `text/calendar` part are silently ignored.
  9. On parse or import error: logs the error and sends a bounce notification
     to the `Return-Path` address (if outbound SMTP is configured; falls back
     to log-only). Bounce is plain-text, identifies the inbound address and
     error; does not include raw message content.
- Inbound addresses are managed in Settings / Integrations: create (pick
  calendar + label), copy address, and revoke per token. Setup copy reads:
  "Forward any invite you want to track to this address."
- Trusted sender addresses are managed in Settings / Profile.

## Token Model

Dedicated `inbound_email_tokens` table, separate from `app_tokens`:

```sql
CREATE TABLE inbound_email_tokens (
    id           TEXT    NOT NULL PRIMARY KEY,
    username     TEXT    NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    calendar_id  INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    label        TEXT    NOT NULL,
    token_hash   TEXT    NOT NULL,   -- SHA-256 hex of raw token
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at DATETIME
);
CREATE INDEX inbound_email_tokens_hash ON inbound_email_tokens(token_hash);
```

Token format: `tidl_` + 64 hex chars (32 random bytes), identical to
`app_tokens`. Address format: `tidl-<raw_token>@<domain>`.

Rationale for separation: independent revocation; different write scope (these
tokens carry write permission against one calendar, app tokens are read-only);
cleaner UI presentation.

Trusted sender addresses:

```sql
CREATE TABLE trusted_sender_addresses (
    id         TEXT    NOT NULL PRIMARY KEY,
    username   TEXT    NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    email      TEXT    NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(username, email)
);
CREATE INDEX trusted_sender_addresses_username ON trusted_sender_addresses(username);
```

## Security Considerations

- **Mandatory STARTTLS**: plaintext sessions are rejected at `MAIL FROM` with
  530. No calendar data or validation results are transmitted over plaintext.
- **Return-Path allowlist**: `From:` is not used. Only `Return-Path:` (set by
  the MTA) is checked.
- **SPF verification** via `viaspf`: confirms the delivering server is
  authorised to send for the `Return-Path:` domain. Raises the bar from "know
  the token" to "control a server authorised by the user's mail provider".
- **Rate limiting**: max 20 messages per token per rolling hour, in-memory
  sliding window (`DashMap<TokenId, RateLimiter>`). Resets on restart —
  acceptable for a single-process self-hosted server.
- **Max message size**: rejects oversized payloads during DATA before any
  parsing.
- **Per-calendar scope**: a compromised token affects only one calendar.

## SMTP Session State Machine

```
Greeting → Ehlo → Starttls (mandatory) → Ehlo → MailFrom → RcptTo → Data → Done
```

Pre-TLS EHLO advertises only `STARTTLS`, `SIZE <max>`, `8BITMIME`.
After TLS upgrade (tokio-rustls, same cert as HTTP server), client must
re-issue EHLO (RFC 3207). Security checks (steps 2–5) run synchronously after
DATA terminates, before `250 OK` is sent, so the MTA receives real SMTP error
codes on rejection.

## Known Limitation

Declining an invite in your mail client does not remove the event from tidl
(the client sends `METHOD:REPLY` to the organiser, not back to tidl). With
manual forwarding this is a non-issue: the user simply does not forward invites
they intend to decline.

## Out of Scope

- Sending `METHOD:REPLY` (accept/decline) back to the organiser.
- CalDAV scheduling (RFC 6638).
- Parsing encrypted Nimbus Mail/Proton emails.
- Automatic forwarding rules (intentionally excluded — see Background).

## Implementation Notes

- SMTP listener: hand-rolled async state machine in
  `crates/tidl-server/src/smtp_listener.rs`, spawned as a tokio task from
  `main.rs`, sharing `Arc<AppState>`. TLS via `tokio-rustls`.
- SPF: `viaspf` crate (pure-Rust SPF evaluation including includes/redirects).
- MIME: `mail-parser` crate.
- Rate limiting: `dashmap` crate; state in `AppState`.
- Reuse existing `.ics` import logic from #10.
- New `[inbound_smtp]` section in `config.toml`:
  ```toml
  [inbound_smtp]
  enabled = true
  port = 2525
  max_message_bytes = 1048576
  rate_limit_per_hour = 20
  ```
- The inbound SMTP port must be opened in the NixOS firewall config.

## Resolution

Feature completed and merged. Implementation includes:

### Database
- `inbound_email_tokens` table with SHA-256 token hashing and per-calendar scoping
- `trusted_sender_addresses` table with username and email tracking

### Server Configuration
- New `[inbound_smtp]` config section in `config.toml` supporting `enabled`, `port`, `max_message_bytes`, and `rate_limit_per_hour`

### Backend Services
- `UserStore` CRUD methods for inbound email tokens and trusted sender addresses (8 new async methods)
- SMTP listener state machine in `crates/tidl-server/src/smtp_listener.rs`:
  - Mandatory STARTTLS enforcement
  - SPF verification via `viaspf` crate
  - Rate limiting using `DashMap` sliding window
  - MIME parsing via `mail-parser` crate
  - Synchronous security checks before returning SMTP 250
- Integration with existing iCalendar import pipeline (METHOD:REQUEST upsert, METHOD:CANCEL delete)

### API & Settings
- 6 new server functions in `crates/tidl-web/src/api/settings.rs`:
  - `create_inbound_email_token`
  - `list_inbound_email_tokens`
  - `delete_inbound_email_token`
  - `add_trusted_sender`
  - `list_trusted_senders`
  - `delete_trusted_sender`

### UI
- "Inbound Email" settings tab for managing:
  - Token creation with calendar picker and label
  - Token display (hidden by default, click to reveal)
  - Copy-to-clipboard action
  - Token revocation
  - Trusted sender address management (add/delete)

### Token Format
Address format: `tidl_<64hexchars>@<domain>` (single prefix as specified)

### Key Deviation from Spec
HTTP server uses reverse proxy for TLS (no built-in HTTPS server). SMTP TLS certificate and key are configured separately in the `[inbound_smtp]` config section, allowing self-signed or Let's Encrypt certificates independent of the reverse proxy configuration.
