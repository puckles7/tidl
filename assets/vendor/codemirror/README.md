# Vendored CodeMirror 6

These files are a snapshot of the ES modules esm.sh would otherwise serve on
every page load for:

- `@codemirror/view@6` (6.43.8)
- `@codemirror/state@6` (6.7.1)
- `@codemirror/commands@6` (6.10.4)
- `@codemirror/lang-markdown@6` (6.5.2)
- `@codemirror/theme-one-dark@6` (6.1.3)

...and their full transitive dependency graph (lezer parsers, `@codemirror/
language`, `@codemirror/autocomplete`, the `node:process` polyfills lang-
markdown pulls in transitively, etc).

They're vendored — rather than fetched from `esm.sh` at runtime — because a
static top-level `import` of a CDN URL blocks the whole module graph
(including the WASM app's own `init()` call in `assets/index.html`) until
that CDN responds. If esm.sh is slow or briefly unreachable, the page stays
blank forever with no error shown. See tidl issue: "initial page load
sometimes hangs blank, reload fixes it".

## Structure

Filenames mirror esm.sh's own resolved path for each module (`/` → `__`,
everything outside `[A-Za-z0-9._-]` sanitized to `_`), so the dependency
graph's structure — including which packages share a single resolved copy of
a common dependency (e.g. `@codemirror/view`, `@codemirror/commands`, and
`@codemirror/lang-markdown` all resolving to the *same* `@codemirror/state`
module) — is preserved exactly as esm.sh would serve it. This matters:
CodeMirror uses object identity (e.g. `Facet`/`StateField` instances) across
packages, so each shared dependency must end up as a single JS module
instance, not a separate bundled copy per consumer.

Some files are trivial redirect stubs (a couple of side-effect imports plus
`export * from "./<real-file>"`) — that's esm.sh's own resolution shape for
version-range specifiers (e.g. three different stubs for `@codemirror/state`
at `@6`, `^6.0.0`, and `^6.7.0` all pointing at the one real
`@codemirror/state@6.7.1` file). Don't be tempted to "clean these up" by hand
without understanding why — see the identity note above.

## Regenerating / upgrading

There's no Node toolchain in this repo, so these were fetched directly with
a small crawler that:

1. Requests each of the 5 top-level `https://esm.sh/<pkg>@<range>` URLs
   (no `?bundle` — that flag inlines shared deps per-package, which breaks
   the identity sharing described above).
2. Follows redirects, records the final resolved path per request.
3. Parses out every `import`/`export * from` declaration pointing at an
   absolute esm.sh path (handling minified code where statements can abut a
   preceding `}` with no `;`, so ASI applies — a naive split on `;` misses
   these).
4. Recurses into each new absolute path, memoizing by *final resolved path*
   so shared dependencies collapse to one fetched file.
5. Rewrites each file's absolute import paths to relative `./<local-name>`
   paths pointing at its sibling files in this directory.

To bump a version, edit the version pin in the entrypoint URL and rerun the
equivalent crawl, then diff the resulting file set before replacing this
directory wholesale (a partial manual edit will not preserve the identity
graph correctly).
