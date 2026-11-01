# Self-registration / sign-up flow

Allow new users to create their own account via the web UI, without requiring
an admin to bootstrap them via the CLI. This also completes the calendar
sharing story (#13): invitees who don't yet have an account can sign up and
have the pending share automatically accepted.

## Flow

### Standalone sign-up
1. A "Create account" link on the login page opens a registration form.
2. User enters email, username slug, display name, and password.
3. Server creates the account (inactive until email is verified, or immediately
   active if email verification is not implemented yet — see Open Questions).
4. User is logged in and lands on the calendar view.

### Invitation-driven sign-up
1. Invitee receives a share invitation email with a link:
   `/share/accept/<token>`
2. If the invitee is not logged in, the server redirects to:
   `/login?next=/share/accept/<token>`
   (or `/signup?next=/share/accept/<token>` if they don't have an account).
3. The login and signup pages preserve the `next` query parameter through the
   form submission.
4. After a successful login or signup, the server redirects to the `next` URL,
   which completes the share acceptance.
5. The shared calendar appears in the invitee's sidebar.

The `next` parameter must be validated server-side (allow-list or same-origin
check) to prevent open redirect attacks.

## Server-side changes

- New `POST /signup` handler in `session.rs` (mirrors `login_handler`).
- `UserStore::create_user` already exists — just needs to be wired up.
- Login and signup handlers read `next` from the query string and embed it in
  the redirect after success.
- Signup form in `tidl-web` (new component or extended login page).

## Registration policy

Self-registration can be controlled by a server config flag
(`allow_registration = true/false`, default `true` for self-hosted). When
disabled, the "Create account" link is hidden and the `/signup` route returns
403, but invitation-driven signup still works (the invitation token itself
serves as authorisation).

## Dependencies

- [2 - User management](02-user-management.md) ✓ — `UserStore` and account
  model already in place.
- [13 - Calendar sharing](13-calendar-sharing.md) ✓ — invitation tokens and
  `/share/accept/<token>` endpoint already implemented.

## Decisions

- **Email verification**: account is inactive until the user verifies their
  address via a one-time passcode (OTP) sent to the supplied email. The OTP
  flow is part of this item.
- **Admin notification**: an email is sent to the configured admin address
  whenever a new account is created (both self-registration and
  invitation-driven).
- **Password reset**: follow-on item (not in scope here).

## Additional server-side changes (from decisions above)

- Store a pending-verification token/OTP alongside the new user row.
- `GET /verify?token=<otp>` (or `POST /verify`) activates the account and
  redirects to login (or auto-logs the user in).
- Send admin notification email from the signup handler after the user row is
  created (fire-and-forget, same SMTP path used for sharing invitations).

## Resolution

All flows implemented. The standalone and invitation-driven sign-up flows both
work: email OTP verification, `allow_registration` config flag (hides the UI
and returns 403 when disabled), and `?login_next=` preservation through the
signup/verify flow (the URL never changes during client-side view transitions,
so `post_auth` reads the param correctly on verification success).

Admin notification completed this item: `server.admin_email` config field
(optional `String`), `AdminEmail` SSR extension type, injected via axum
`Extension` layer, and `send_admin_new_user_notification` fired as a
`tokio::spawn` (best-effort, warnings logged on failure) in `do_signup` after
the verification email is sent successfully.
