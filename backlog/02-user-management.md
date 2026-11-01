# User Management

Introduce the concept of distinct user accounts as the foundational identity layer for tidl. Everything that requires ownership, access control, or sharing builds on this.

## Motivation

Previously tidl had no user model. Multi-calendar support and calendar sharing both require knowing *who* owns what and *who* is allowed to see or edit it. This feature is the prerequisite.

## Core Concepts

- **Email address** is the unique identifier and login name
- **Username slug** is the stable identifier used in CalDAV paths (`/caldav/<username>/`)
- Password-based authentication (email + password)
- Each user has their own isolated space within tidl (calendars, events)

## Functional Requirements

- Login / logout via web UI (email + password)
- CalDAV authentication via Basic Auth (username or email accepted)
- Admin can create accounts via `create-admin` CLI subcommand
- `is_admin` and `is_active` flags for account management
- Basic profile: display name

## CalDAV Principal

Each user maps to CalDAV paths at `/caldav/<username>/`. The username slug is stable and used in all resource URLs.

## Security

- Passwords hashed with **argon2id** (PHC string format)
- Legacy **bcrypt** hashes (`$2b$`, `$2y$`, `$2a$`) accepted during migration window
- Sessions via secure HttpOnly cookie (24-hour TTL)
- No unauthenticated access to any calendar data
- COPY/MOVE restricted to same-user destinations (`require_same_user`)
- LOCK/UNLOCK restricted to the resource owner
- CalDAV POST (free-busy) restricted to calendar owner

## Implementation

- `migrations/20240107000000_users.sql` — `users` table with `username`, `email`, `display_name`, `password_hash`, `is_admin`, `is_active`
- `crates/tidl-server/src/user_store.rs` — `UserStore` owning all user DB operations, mirroring `SessionStore` pattern
- `crates/tidl-server/src/auth.rs` — `BasicAuth` extractor now validates against `UserStore`, returns canonical username slug
- `crates/tidl-server/src/session.rs` — `login_handler` uses `UserStore.find_active_by_email`
- `crates/tidl-web/src/api.rs` — `do_login` server function queries `users` table by email directly
- `crates/tidl-web/src/app.rs` — login form uses `type="email"` input

## Open Questions

- Support OAuth2 / OIDC login (e.g. sign in with Google) as an alternative to local passwords?
- Password reset via email?
- Multi-admin or single-owner model for the self-hosted instance?
- Self-registration via web UI?

## Resolution

Implemented 2026-04-15. Core user model, argon2id hashing with bcrypt migration bridge, CLI bootstrap command, and all CalDAV security vulnerabilities addressed.
