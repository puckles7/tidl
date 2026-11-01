# syntax=docker/dockerfile:1
#
# tidl — image built from source (backlog #149).
#
# This is the *non-Nix* alternative to the published image (`nix build .#docker`
# / ghcr.io). It is a heavier, slower build but needs only Docker — no Nix, no
# Rust toolchain on the host.
#
#   docker build -t tidl .
#   docker run -p 3060:3060 -v tidl-data:/data \
#     -e TIDL_SECRET_FILE=/run/secrets/tidl-secret \
#     -v "$PWD/tidl-secret:/run/secrets/tidl-secret:ro" tidl
#
# The image is deliberately "dumb" about configuration: everything is driven by
# TIDL_* environment variables (see README → Configuration), and the only
# persistent path is /data.

# ── Stage 1: build the server binary + WASM frontend ─────────────────────────
FROM rust:1-bookworm AS builder

# wasm-bindgen-cli must match the `wasm-bindgen` crate exactly, or the generated
# JS glue won't line up with the WASM module. Keep in sync with Cargo.lock
# (and the pinned version in flake.nix).
ARG WASM_BINDGEN_VERSION=0.2.125

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev libsqlite3-dev binaryen \
 && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown \
 && cargo install cargo-leptos --locked \
 && cargo install wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}" --locked

WORKDIR /src
COPY . .

# Compile against the committed SQLx offline cache (crates/tidl-server/.sqlx),
# so no live database is needed at build time.
ENV SQLX_OFFLINE=true
RUN cargo leptos build --release

# ── Stage 2: minimal runtime ─────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates libssl3 libsqlite3-0 \
 && rm -rf /var/lib/apt/lists/*

# The server resolves `assets/` and `target/site/pkg` relative to the process
# cwd, so keep everything under /app and run from there.
WORKDIR /app
COPY --from=builder /src/target/release/tidl-server /app/tidl
COPY --from=builder /src/target/site/pkg /app/target/site/pkg
COPY --from=builder /src/assets /app/assets
# On PATH so `docker exec <c> tidl create-admin …` works.
RUN ln -s /app/tidl /usr/local/bin/tidl

ENV TIDL_HOST=0.0.0.0 \
    TIDL_PORT=3060 \
    TIDL_DATA_DIR=/data
VOLUME /data
EXPOSE 3060

ENTRYPOINT ["/app/tidl"]
