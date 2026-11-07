# Compress web assets to speed up app startup

The SPA ships a multi-megabyte WASM bundle plus CSS/JS, and the server served all
of it uncompressed. On a laptop over Wi-Fi a cold reload spent most of its time
downloading the bundle, while the same page was instant on a fast wired machine;
the Focus → "Show in calendar" button (a full-page navigation at the time) made
that difference very visible.

## Functional Requirements

- Serve `/pkg` and `/assets` responses gzip/brotli-compressed.
- Use precompressed `.gz`/`.br` siblings in release builds, falling back to
  on-the-fly compression for dev builds.
- Never compress SSE (`/api/stream`) or double-compress an already-encoded
  response.

## Resolution

- `tower-http` gained the `compression-gzip` and `compression-br` features; the
  web router wraps its responses in `CompressionLayer` (the default predicate
  skips SSE and already-encoded bodies).
- The `/pkg` `ServeDir` enables
  `precompressed_gzip()`/`precompressed_br()`, and release builds emit the
  siblings: the Nix package builds with
  `cargo leptos build --release --precompress`, and `build-windows --release`
  passes `--precompress` too. `/assets` has no siblings, so the layer compresses
  it (and dev builds' `/pkg`) on the fly. `cache_pkg_assets` adds
  `Vary: accept-encoding` because `ServeDir` sets `Content-Encoding` on a
  precompressed response without varying on the request header.
- Focus' "Show in calendar" also stopped doing a full-page reload and now uses
  client-side navigation (see #157), so the two changes together remove the
  cold-boot cost from that flow.
