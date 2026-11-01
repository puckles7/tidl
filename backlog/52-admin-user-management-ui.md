# Admin user management UI

Add an admin-only panel in the Settings dialog for managing user accounts from the web UI, instead of requiring the `tidl-server create-admin` CLI subcommand or direct DB edits.

## Dependencies

- [02 - User management](02-user-management.md) — provides the underlying `users` table, `UserStore`, and `is_admin`/`is_active` model this UI operates on

## Context / Background

Today, creating or managing users has no UI path: the only options are the `create-admin` CLI bootstrap command, self-registration (when `allow_registration` is enabled), or editing the SQLite `users` table directly. There's no way for an admin to see who has an account, promote/demote admins, deactivate a user, or reset a password without shelling into the server.

## Functional Requirements

- New admin-only tab ("Users") in the Settings dialog (alongside the existing "Email" admin-only tab), gated on `IsAdminCtx` like `SmtpConfigPanel`
- List existing users: username, email, display name, `is_admin`, `is_active`
- Create a new user (username, email, display name, password) — same slug/email/password validation as `do_signup`; starts `is_active = true`, `is_admin = false`
- Toggle `is_admin` and `is_active` on existing users
- Admin-initiated password reset (admin sets the new password directly) — revokes **all** of that user's sessions, and fires a best-effort email notice ("an admin reset your password") if SMTP is configured, silently skipped otherwise
- Hard delete a user, cascading to everything they own (see below)
- An admin can never target their own account (delete, deactivate, demote) through this panel — this also means "at least one active admin remains" is guaranteed for free, since the acting admin's own row is never touched

## Design Decisions (resolved during brainstorming, 2026-07-23)

**Password reset notification:** email notice, best-effort, matching the fallback pattern of `send_email_changed_notice` in `profile.rs` (skip silently if SMTP isn't configured).

**Deletion model:** hard delete, cascading to everything the user owns. `tidl-web` cannot depend on `tidl-server`'s `UserStore`, so this is raw `sqlx` in a new `crates/tidl-web/src/api/admin.rs` module (same pattern as `profile.rs`/`settings.rs`), gated by the existing `auth_admin()` helper.

Because SQLite's `PRAGMA foreign_keys` is never turned on in `storage.rs`'s `SqliteConnectOptions`, none of the declared `ON DELETE CASCADE` FKs (e.g. `app_tokens`, `user_preferences`, `remote_caldav_sources`) fire automatically — deletion must explicitly touch every table in one transaction, in this order:

1. `events` (username)
2. `calendars` (username)
3. `sync_log` (username)
4. `properties` (username)
5. `locks` (username)
6. `calendar_shares` (owner_username OR grantee_username)
7. `share_invitations` (owner_username)
8. `notes` + `note_collection_members` for collections the user owns (`note_collections.owner_username`)
9. `note_collection_members` (their own memberships in collections owned by others)
10. `notes` (author_username, notes they wrote in others' collections)
11. `note_collections` (owner_username)
12. `user_preferences`, `app_tokens`, `remote_caldav_sources`, `inbound_email_tokens`, `trusted_sender_addresses`, `sessions`, `email_verifications`, `email_change_requests`, `markdown_sources` (all keyed by username)
13. `users` (the row itself)

Cross-checked against `USERNAME_COLUMNS` in `profile.rs` (the rename cascade) — one extra case that list surfaces: `calendars.sync_user` (the designated write-identity on a *read-only* calendar owned by someone else). That calendar isn't owned by the deleted user, so it isn't deleted — instead `sync_user` is set to `NULL` on any calendar pointing at the deleted username, so the column never dangles.

**Delete confirmation UX:** clicking "Delete" first calls `get_user_deletion_summary(username)` (counts of calendars/notes/collections owned/shares owned/app tokens) and shows them inline, then requires typing the exact username into a text field before the final "Delete permanently" button enables.

## Implementation Notes

- New type `UserAdminInfo { username, email, display_name, is_admin, is_active }` in `types.rs`.
- Server functions: `list_users`, `create_user_admin`, `set_user_admin`, `set_user_active`, `admin_reset_password`, `get_user_deletion_summary`, `admin_delete_user(username, confirm_username)`.
- Guard/cascade logic lives in private `async fn`s taking `(pool, ...)` directly (no `extract()`), same as `rename_username_tx` — makes them unit-testable via `#[sqlx::test(migrations = "../../migrations")]` without a live Axum request. The public `#[server]` wrappers just do `auth_admin()` + self-target rejection + delegate.
- UI: `crates/tidl-web/src/components/settings/users.rs`, wired into `SettingsModal` next to the "Email" tab. Create-user form styled like `SmtpConfigPanel`; user list styled like `AppTokensPanel`'s row list. Self row: admin/active toggles and delete button disabled with a tooltip.
- Tests to add: invalid slug/short password/duplicate email on create; admin/active toggle correctness; reset-password session revocation; the big cascade-delete test (calendar+events, a shared note collection with another member, an app token, a share granted and one received — assert the other member's data survives untouched); confirm-username mismatch rejection.

## Resolution

Implemented as designed: a new admin-only "Users" tab in Settings (`crates/tidl-web/src/components/settings/users.rs`) backed by `crates/tidl-web/src/api/admin.rs`. Covers list, create, admin/active toggles, admin-initiated password reset (with session revocation and a best-effort email notice), and cascading hard delete with a type-the-username confirmation. Self-targeting is blocked entirely, which also guarantees at least one active admin always remains without separate counting logic.
