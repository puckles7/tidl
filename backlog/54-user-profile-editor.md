# User profile editor

Let the signed-in user edit their own account (display name, username, email,
password) from a modal opened by clicking their username in the navbar,
instead of adding a ninth tab to the already-crowded Settings modal.

## Context / Background

The `users` table has `username` (also the CalDAV URL slug, `/caldav/{username}/…`),
`email` (login identifier), `display_name`, and `password_hash`. There was
previously no self-service way to change any of these after signup — only
admin CLI paths existed.

`username` is stored by value (not a surrogate FK) across ~19 columns in
~18 tables (`calendars.username`, `calendars.sync_user`, `events.username`,
`markdown_sources.username`, `properties.username`, `locks.username`,
`sync_log.username`, `calendar_shares.owner_username`/`grantee_username`,
`share_invitations.owner_username`, `user_preferences.username`,
`app_tokens.username`, `remote_caldav_sources.username`,
`inbound_email_tokens.username`, `trusted_sender_addresses.username`,
`note_collections.owner_username`, `note_collection_members.member_username`,
`notes.author_username`, `sessions.username`, `email_verifications.username`).
None of the FK-backed ones declare `ON UPDATE CASCADE`, so renaming requires an
explicit transactional cascade across all of them — see `USERNAME_COLUMNS` in
`crates/tidl-web/src/api/profile.rs`.

Note on architecture: this logic initially landed in
`tidl-server::UserStore`, but `tidl-web` cannot depend on `tidl-server` (only
the reverse — `tidl-server` links `tidl-web` with the `ssr` feature to host
its server-fn bodies and Leptos SSR renderer), and the browser-facing
`#[server]` functions live in `tidl-web`. It was relocated to
`tidl-web/src/api/profile.rs`, matching the existing convention in
`api/auth.rs` (`do_signup`/`do_login`) of talking to the DB directly via
`sqlx` rather than through a shared store type.

## Functional Requirements

- Navbar username becomes a button; clicking it opens `ProfileModal`
  (separate component from `SettingsModal`, no tabs — each section saves
  independently, matching how `PreferencesPanel` etc. already work).
- **Display name**: single text field, saves immediately, no confirmation.
- **Username**: editable, with a persistent inline warning shown whenever the
  field differs from the loaded value ("Renaming changes your CalDAV address;
  connected apps will need reconfiguring"). Rename runs as one transaction
  across `USERNAME_COLUMNS`, `users` last, with `PRAGMA defer_foreign_keys =
  ON` for the transaction (SQLite checks FK constraints per-statement, not
  deferred by default, so neither "children first" nor "parent first" order
  works without this). The existing session survives (it's keyed by an opaque
  token, not username), so no forced logout. `CurrentUsernameCtx` is updated
  client-side on success — no full page reload needed.
- **Email**: two-step, gated behind the *current* password (this is the
  credential that lets someone recover/take over the account, unlike
  username/display name):
  1. User enters new email + current password → server verifies the
     password, checks the new email isn't used by another active account,
     stores a 6-digit code in `email_change_requests` (60-minute expiry, one
     pending request per user, same shape as `email_verifications` /
     `password_resets`), emails the code to the **new** address.
  2. User enters the code → server validates it, updates `users.email`,
     deletes the pending row, and fires a best-effort notice to the **old**
     address (fire-and-forget `tokio::spawn`, same pattern as the admin
     new-user notification in `do_signup`).
  - No "cancel" server call needed — a client-side "Back" button just returns
    to the edit state; a later request overwrites the stale pending row.
- **Password**: current password + new password + confirm, gated behind the
  current password. On success, deletes every `sessions` row for that
  username except the caller's own token (revokes other devices) — needs a
  new `auth_with_token()` helper in `api/mod.rs` alongside the existing
  `auth()`, since `sessions` is keyed by the opaque cookie token.

## Testing

- `rename_username`'s cascade + the `rename_username_column_list_is_exhaustive`
  guard test (scans `sqlite_master`/`pragma_table_info` for any column
  containing "user" not triaged into `USERNAME_COLUMNS` or a documented
  exclusion list — currently `smtp_config.username` and
  `remote_caldav_sources.remote_username`) live in
  `crates/tidl-web/src/api/profile.rs`.
- New coverage for `request_email_change`/`confirm_email_change` (wrong
  password rejected, bad/expired code rejected) and `change_password` (other
  sessions revoked, current session survives) — first `#[sqlx::test]`
  coverage of any `#[server]`-fn-adjacent logic in this codebase, written
  against small testable inner functions taking `&SqlitePool` directly, with
  the `#[server]` functions themselves as thin `extract()`/`auth()` wrappers.

## Open Questions

- None outstanding — username-rename scope, password-gating, session-revoke,
  and old-email-notification behavior were all explicitly decided during
  design.

## Resolution

Built as designed, in `crates/tidl-web/src/api/profile.rs` (server functions
+ the rename cascade + tests) and `crates/tidl-web/src/components/profile_modal.rs`
(UI), wired into the navbar in `app.rs`. New migration
`20260721000000_email_change_requests.sql`.

Verified live with a throwaway admin user against a temporary server
instance (Playwright): profile modal opens on username click; the CalDAV
rename warning appears only while the field differs from the loaded value;
renaming updates the navbar immediately with no page reload; a wrong current
password is rejected on the password section with the expected message; a
correct password change persists (confirmed via a subsequent login with the
new password) and the email section surfaces the "SMTP not configured"
business error cleanly when attempted without SMTP set up. No WASM panics
were observed in the console during the pass.

One deviation from the initial plan: the username-rename cascade and its
guard test were first written in `tidl-server::UserStore`, then relocated to
`tidl-web::api::profile` once it became clear `tidl-web` cannot depend on
`tidl-server` (only the reverse) — the browser-facing `#[server]` functions
that need to call it live in `tidl-web`. `tidl-server::UserStore` was left
unchanged, since nothing in the CLI/admin path uses username rename today.
