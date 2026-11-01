# Declarative SMTP configuration (env vars + NixOS module)

SMTP settings were DB-only, set exclusively through Settings → Email in the
web UI. There was no way to provision them at deploy time — every fresh
instance needed a human to click through the UI once before outbound mail
(invites, notifications) worked. Add an env-var-driven path, mirroring how
OIDC and the encryption key are already provisioned, plus a matching NixOS
module option for `services.tidl`.

## Functional Requirements

- New `TIDL_SMTP_*` env vars (`config.rs`): `TIDL_SMTP_HOST`, `TIDL_SMTP_PORT`
  (default 587), `TIDL_SMTP_TLS_MODE` (default "starttls"),
  `TIDL_SMTP_USERNAME`, `TIDL_SMTP_PASSWORD` / `TIDL_SMTP_PASSWORD_FILE`
  (file takes precedence), `TIDL_SMTP_FROM_ADDRESS`, `TIDL_SMTP_FROM_NAME`.
  Activated only when host + from-address are both present, matching the
  OIDC activation pattern.
- On startup (`main.rs`), if declarative SMTP config is present, encrypt the
  password and upsert it into the existing `smtp_config` DB row — the same
  row `SmtpMailer` already reads on every send and the same row Settings →
  Email edits. No new storage path; env vars just win on every restart.
- `services.tidl.smtp.*` NixOS module options (`flake.nix`): `enable`, `host`,
  `port`, `tlsMode`, `username`, `passwordFile` (sops-nix friendly),
  `fromAddress`, `fromName`. Wired into `systemd.services.tidl.environment`
  and `BindReadOnlyPaths` (for `passwordFile`, under the existing
  `ProtectSystem = "strict"` hardening).

## Open Questions

- None — deliberately DB-upsert-on-boot rather than a separate config path,
  so the mailer's existing "reads from DB on every send" model needs no
  changes and the UI stays the escape hatch for ad-hoc edits.

## Resolution

Built per spec:

- `config.rs`: `SmtpEnvConfig` struct + `Config.smtp: Option<SmtpEnvConfig>`,
  populated in `apply_env()`. Covered by six new tests (defaults, full
  parse, password-file precedence, activation gating, absence).
- `mailer.rs`: added `encrypt_password`, symmetric to the existing
  `decrypt_password` (duplicated from `tidl-web::ssr_types` rather than
  pulling the `ssr` feature into the server's startup path — matches the
  existing duplication of `decrypt_password`). Covered by roundtrip/
  wrong-secret/nondeterminism tests.
- `main.rs`: startup block upserts `config.smtp` into `smtp_config` via
  `storage.save_smtp_config`, alongside the existing
  `TIDL_GIT_SOURCES_FILE` upsert.
- `flake.nix`: `services.tidl.smtp` option set + environment/BindReadOnlyPaths
  wiring, following the `oidc` option block's shape.
- README: NixOS deployment example and Configuration section updated.
