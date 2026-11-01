# Secret-generation CLI convenience

Deploying tidl requires a one-time 32-byte secret (base64) to key the encryption
of stored SMTP passwords and remote CalDAV tokens. The docs told users to run
`printf '%s' "$(openssl rand -base64 32)" > tidl-secret`, and the server already
has `tidl-server generate-key` for printing an equivalent key — but nothing
wrote the key straight to the `TIDL_SECRET_FILE` the deployment expects. Add a
`generate-secret` subcommand that generates the key and writes it to a file in
one step, so no shell redirection or `openssl` install is needed.

## Functional Requirements

- `tidl-server generate-secret` generates a random 256-bit key and writes its
  base64 form to `./tidl-secret` by default.
- `--output <PATH>` overrides the destination.
- Refuse to overwrite an existing file unless `--force`, since rotating the
  secret invalidates every stored SMTP password / CalDAV token; `--force`'s
  help text says so.
- Keep `generate-key` (stdout, for `sops` pipelines) unchanged.

## Resolution

Added the `GenerateSecret { output, force }` variant to `tidl-server`'s CLI in
`crates/tidl-server/src/main.rs`. Docs (`README.md`, `deploy/README.md`,
`docker-compose.yml`, `deploy/unraid/tidl.xml`, `config.toml.example`) now lead
with the new command, keeping the `openssl` equivalent as a fallback.
