# Explore rustical OIDC + Argon2 auth — contrast with tidl

Rustical implements authentication via Argon2 password hashing and OpenID Connect (OIDC) federation. Tidl currently has a home-baked auth model. This item is a research spike: read rustical's auth implementation, understand what it does differently, and produce a written assessment of whether tidl should adopt any of its patterns or protocols.

## Background

Rustical's auth stack:
- **Argon2** for local password hashing (industry standard, memory-hard, resist GPU cracking)
- **OIDC** via `openidconnect 4.0` crate — allows delegating auth to an external identity provider (Google, Keycloak, Authentik, etc.)

Tidl's current approach: custom session/token model with inbound email tokens and a trusted-sender list. It is not clear whether passwords are hashed with a modern algorithm or whether federation is possible at all.

## Why OIDC Is Appealing for a Self-Hosted Server

The typical tidl user is probably already running an identity provider — Authentik, Keycloak, Kanidm, or Authelia in front of a reverse proxy. OIDC would let tidl delegate auth entirely to that existing system:

- **No password management in tidl** — no password reset flows, no hashing, no credential storage to secure
- **SSO** — one login for tidl, Nextcloud, Gitea, and everything else in the homelab
- **MFA for free** — if the IdP enforces TOTP/passkeys, tidl inherits it without any extra work
- **App passwords** — OIDC IdPs typically issue app-specific long-lived tokens for clients that can't do interactive OAuth flows

## The CalDAV + OIDC Tension

CalDAV clients like DAVx5 authenticate with HTTP Basic auth on every request. OIDC is an interactive browser flow, so it cannot be used directly for DAV sync. The solution is **app passwords**: the IdP (or tidl's UI backed by the IdP) issues a long-lived token that the user pastes into DAVx5 — this is a standard pattern.

Rustical solves this via Nextcloud-compatible app token generation. Worth studying closely.

## Functional Requirements

- Read and document rustical's auth flow (local login + OIDC flow + app token generation)
- Audit tidl's current password storage and session mechanism
- Produce a written gap analysis: what would tidl need to adopt Argon2 and/or OIDC?
- Identify any security risks in tidl's current model worth fixing regardless of OIDC adoption
- Assess how app passwords / DAV tokens would work under an OIDC model

## Open Questions

- Does tidl even support multiple auth backends, or is it single-user?
- Would adopting Argon2 require a migration of existing password hashes?
- Should OIDC be the only auth method, or an optional alternative to local accounts?
- Which IdPs should be tested against (Authentik, Keycloak, Kanidm)?

## Resolution

This item evolved from a research spike into a full OIDC implementation. Rather than produce a written gap analysis, OIDC SSO was built directly.

**What was built:**
- Optional `[oidc]` config section — OIDC is disabled if absent, so local-only installs are unaffected
- DB migration adding `oidc_subject`/`oidc_issuer` columns to users and a `oidc_pending` table for the in-flight OAuth state
- `OidcState` added to `AppState`, initialised from config at startup with graceful error handling if the IdP is unreachable
- `/auth/oidc/start` and `/auth/oidc/callback` Axum handlers implementing the Authorization Code flow
- `/auth/oidc/link` page (Leptos `OidcLinkPage`) — shown after callback when an existing local user needs to confirm account linking
- Auto-provisioning of new users from OIDC claims with sanitised username derivation
- SSO login button on the login page, hidden when OIDC is not configured
- Upgraded `openidconnect` 3.x → 4.x to fix private CA TLS trust

**Decisions and deviations:**
- Argon2 was not adopted — existing local password model was left in place; OIDC is an additive alternative, not a replacement
- App passwords for CalDAV clients (DAVx5 HTTP Basic) were not implemented in this item — the existing session token mechanism continues to serve that role
- Private CA support was a real-world requirement that drove the `openidconnect` 4.x upgrade
