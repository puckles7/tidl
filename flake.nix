{
  description = "tidl - Lightweight Rust CalDAV Server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # crane splits the package build into a cached "compile just the
    # dependencies" derivation (keyed on Cargo.lock) and a fast "compile
    # tidl's own 4 crates" derivation — a plain buildRustPackage recompiles
    # the entire ~300+ crate dependency tree from scratch on every source
    # change (CSS/JS/single-file Rust edits included), since Nix builds are
    # sandboxed with no persistent target/ dir to carry incremental state
    # between derivation invocations.
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Read the pinned toolchain from rust-toolchain.toml so the flake
        # and the project's declared toolchain stay in sync.
        rustToolchain =
          pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Single source of truth for the product version (backlog #147): the
        # root Cargo.toml's `[workspace.package] version`, read back here so the
        # Nix package and the release artifacts never carry a second copy.
        version =
          (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

        craneLib = (crane.mkLib pkgs).overrideToolchain (_: rustToolchain);

        # crane's default filter keeps only Cargo-relevant files (*.rs,
        # Cargo.toml/lock, .cargo/config.toml, ...) so unrelated source
        # changes don't bust the cache. Deliberately kept separate from
        # tidlFullSrc below: this is the *only* source fed to buildDepsOnly,
        # so its hash — and therefore whether the dependency-compile cache
        # gets reused — depends only on Cargo.toml/Cargo.lock/*.rs, not on
        # every CSS/asset/migration-only commit (which is most of them).
        cargoDepsSrc = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = craneLib.filterCargoSources;
        };

        # Full source for the actual package build: adds back assets/
        # (cargo-leptos's assets-dir, also copied into $out by
        # installPhaseCommand below), migrations/ (embedded at compile time
        # by the sqlx::migrate!() macro), and tidl-server's .sqlx/ query
        # cache (needed to compile sqlx::query!() macros without a live
        # database) — none of which buildDepsOnly needs, since it never
        # compiles tidl's own crates, just their dependencies.
        tidlFullSrc = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (pkgs.lib.hasInfix "/assets" path)
            || (pkgs.lib.hasInfix "/migrations" path)
            || (pkgs.lib.hasInfix "/.sqlx" path)
            || (craneLib.filterCargoSources path type);
        };

        # wasm-bindgen-cli built at the exact version used by the project's
        # wasm-bindgen crate. nixpkgs ships a different version and they must
        # match exactly — so we build it ourselves from crates.io.
        wasm-bindgen-cli = pkgs.rustPlatform.buildRustPackage {
          pname = "wasm-bindgen-cli";
          version = "0.2.125";
          src = pkgs.fetchCrate {
            pname = "wasm-bindgen-cli";
            version = "0.2.125";
            hash = "sha256-zRawtjxMOdTMX+mZaiNR3YYfTiZJhf9qj7kXSSeMxrc=";
          };
          cargoHash = "sha256-aZCfgR23Qb0Pn4Mm4ToMtuuRQqSJjXCR9li/VvP5CTM=";
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ];
        };

        tidlCommonArgs = {
          pname = "tidl";
          inherit version;
          src = cargoDepsSrc;
          strictDeps = true;

          # buildDepsOnly's default `cargo check`/`build --profile release`
          # has no --package/--features of its own, so without this it only
          # resolves the workspace's `default-members` (tidl-caldav +
          # tidl-markdown) — never touching tidl-server's own (much larger)
          # dependency tree at all. Match cargo-leptos's own native/server
          # invocation exactly so the compiled artifacts' fingerprints line
          # up and actually get reused instead of silently recompiled.
          cargoExtraArgs = "--locked --package tidl-server --no-default-features --features ssr";

          nativeBuildInputs = with pkgs; [
            pkg-config
            cargo-leptos
            wasm-bindgen-cli
            binaryen
          ];

          buildInputs = with pkgs; [ openssl.dev sqlite ];
        };

        # Compiled once per Cargo.lock, reused across source-only rebuilds
        # (the common case — most commits touch CSS/JS/a few .rs files, not
        # dependency versions). This still only caches the *native* side of
        # the build; cargo-leptos's wasm32 half recompiles its own
        # (smaller) dependency set each time, since crane's stubbed-source
        # deps-only pass builds for the host target by default.
        tidlCargoArtifacts = craneLib.buildDepsOnly tidlCommonArgs;

        tidlPackage = craneLib.buildPackage (tidlCommonArgs // {
          src = tidlFullSrc;
          cargoArtifacts = tidlCargoArtifacts;
          buildPhaseCargoCommand = "cargo leptos build --release";

          # Reproducible builds have no `.git`, so build.rs cannot discover the
          # revision — hand it the flake's own rev (backlog #147). Null when the
          # source is not a clean git checkout, in which case build.rs falls back
          # to `unknown`.
          env.TIDL_GIT_SHA = self.shortRev or "unknown";

          # cargo-leptos doesn't produce the JSON build log crane's default
          # post-build hook wants to auto-detect binaries from; we install
          # explicitly below instead.
          doNotPostBuildInstallCargoBinaries = true;

          # Tests aren't this derivation's job — `cargo test`/pytest already
          # run separately (devShell, CI); gating every NixOS deploy on them
          # here would only add time without adding coverage cargo-leptos's
          # own dual-target build doesn't already exercise via compilation.
          doCheck = false;

          installPhaseCommand = ''
            mkdir -p $out/bin $out/share/tidl/target/site
            cp target/release/tidl-server $out/bin/tidl
            cp -r target/site/pkg $out/share/tidl/target/site/pkg
            cp -r assets $out/share/tidl/assets
          '';
        });

        # OCI image (backlog #149). Built from the same `tidlPackage` as the
        # NixOS module, so the image, the release tarballs (#148) and the Nix
        # package share one closure and cannot drift.
        #
        # The server resolves its web assets (`target/site/pkg`, `assets/`) from
        # the process cwd, so `WorkingDir` is pinned to the package's
        # `share/tidl` (which contains exactly that layout). `TIDL_DATA_DIR` is
        # remapped to `/data` because the Nix store is read-only — that is the
        # one writable path, declared as a volume.
        tidlDockerImage = pkgs.dockerTools.buildLayeredImage {
          name = "tidl";
          # CI re-tags with the release version; `latest` is only for local use.
          tag = "latest";
          contents = [ tidlPackage ];
          config = {
            Cmd = [ "${tidlPackage}/bin/tidl" ];
            WorkingDir = "${tidlPackage}/share/tidl";
            Env = [
              # Put the binary on PATH so `docker exec <c> tidl create-admin …`
              # works (the store bin/ is not on PATH by default).
              "PATH=${tidlPackage}/bin:/bin:/usr/bin"
              "TIDL_HOST=0.0.0.0"
              "TIDL_PORT=3060"
              "TIDL_DATA_DIR=/data"
            ];
            ExposedPorts = { "3060/tcp" = { }; };
            Volumes = { "/data" = { }; };
          };
        };

        # Linux release tarball (backlog #148). This is the single definition of
        # the artifact layout — the `dist-linux` dev command and the release CI
        # both consume this derivation, so the binary + `target/site/pkg` +
        # `assets/` layout is never duplicated. The launcher `cd`s into the
        # unpacked directory so the server's cwd-relative asset paths resolve.
        tidlLinuxDist = pkgs.stdenvNoCC.mkDerivation {
          pname = "tidl-linux-dist";
          version = tidlPackage.version;
          dontUnpack = true;
          nativeBuildInputs = [ pkgs.gnutar pkgs.gzip ];
          buildCommand = ''
            set -euo pipefail
            arch=${if pkgs.stdenv.hostPlatform.isAarch64 then "aarch64" else "x86_64"}
            name="tidl-linux-$arch-${tidlPackage.version}"

            mkdir -p "$name/target/site" "$name/assets" "$out"

            cp ${tidlPackage}/bin/tidl "$name/tidl"
            cp -r ${tidlPackage}/share/tidl/target/site/pkg "$name/target/site/pkg"
            cp -r ${tidlPackage}/share/tidl/assets/. "$name/assets/"
            cp ${./config.toml.example} "$name/"
            cp ${./README.md} "$name/"

            cat > "$name/run.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
exec ./tidl "$@"
SH
            chmod +x "$name/run.sh"

            tar -czf "$out/$name.tar.gz" "$name"
          '';
        };

        # MinGW64 C cross-compiler for targeting x86_64-pc-windows-gnu.
        mingwCC = pkgs.pkgsCross.mingwW64.stdenv.cc;

        # winpthread — provides libpthread.a needed by the Rust std on Windows/GNU.
        mingwPthreads = pkgs.pkgsCross.mingwW64.windows.pthreads;

        testPython = pkgs.python3.withPackages (ps: [
          ps.requests
          ps.lxml
          ps.caldav
          ps.pytest
          ps.playwright
        ]);

        # Shell scripts exposed as PATH entries — work in bash, fish, zsh, etc.
        devScripts = [
          # Regenerate the SQLX offline query cache from a fresh schema database.
          # Run after adding/changing SQL queries or migrations, then commit the
          # updated crates/tidl-server/.sqlx/ files.
          (pkgs.writeShellScriptBin "prepare-sqlx" ''
            set -euo pipefail
            TEMP_DB=$(mktemp /tmp/tidl-sqlx-XXXXXX.db)
            rm -f "$TEMP_DB"
            echo "=== Creating temporary schema database ==="
            DATABASE_URL="sqlite://$TEMP_DB" sqlx database create
            DATABASE_URL="sqlite://$TEMP_DB" sqlx migrate run --source "$PWD/migrations"
            echo "=== Regenerating SQLX query cache ==="
            # --tests: without it, `cargo sqlx prepare` only checks the lib/bin
            # target, so query!/query_scalar! calls inside #[cfg(test)] blocks
            # are dropped from the cache and `cargo test` fails offline
            # (found 2026-09-15 while adding backlog #29's ContactStore queries).
            (cd crates/tidl-server && SQLX_OFFLINE=false DATABASE_URL="sqlite://$TEMP_DB" cargo sqlx prepare -- --tests)
            echo "=== Cleaning up ==="
            rm -f "$TEMP_DB" "''${TEMP_DB}-shm" "''${TEMP_DB}-wal"
            echo "Done. Commit the updated crates/tidl-server/.sqlx/ files."
          '')

          # Generate the deployment secret (base64, 32 bytes) that encrypts
          # stored SMTP passwords and remote CalDAV personal access tokens.
          # Writes ./tidl-secret by default; pass --output <path> to change the
          # destination, or --force to rotate an existing one (which invalidates
          # every stored SMTP password / CalDAV token). Builds the server binary
          # first so the shell alias works even before `cargo leptos build`.
          # Usage: generate-secret [--output <path>] [--force]
          (pkgs.writeShellScriptBin "generate-secret" ''
            set -euo pipefail
            echo "=== Building tidl-server (incremental) ==="
            cargo build --no-default-features --features ssr -p tidl-server
            exec "$PWD/target/debug/tidl-server" generate-secret "$@"
          '')

          # Build everything with cargo leptos build, then run the server
          # at a custom port (bypassing cargo-leptos serve/watch, which
          # hard-codes reload-port = 3061).
          # Usage: run-tidl [port] [data-dir]
          #   port     - TCP port to bind (default: 3060)
          #   data-dir - path to database directory (default: ./data-<port>)
          (pkgs.writeShellScriptBin "run-tidl" ''
            set -euo pipefail
            PORT="''${1:-3060}"
            DATA_DIR="''${2:-$PWD/data-$PORT}"
            mkdir -p "$DATA_DIR"
            echo "=== Building (cargo leptos build) ==="
            cargo leptos build

            # No `tidl_bg.wasm` alias is needed: assets/index.html loads the
            # wasm by absolute path (`init({ module_or_path: '/pkg/tidl.wasm' })`),
            # overriding wasm-bindgen's default `tidl_bg.wasm` URL. Creating the
            # alias is actively harmful under `cargo leptos watch` — cargo-leptos
            # renames `tidl_bg.wasm` onto `tidl.wasm` on every rebuild, which
            # turns `tidl.wasm` into a self-referential symlink (ELOOP → 500).

            echo "=== Starting tidl at http://127.0.0.1:$PORT (data: $DATA_DIR) ==="
            TIDL_PORT="$PORT" \
            TIDL_DATA_DIR="$DATA_DIR" \
            DATABASE_URL="sqlite:$DATA_DIR/tidl.db" \
              exec "$PWD/target/debug/tidl-server"
          '')

          # Spin up a disposable "generate test server" for manual UI exploration:
          # a fresh throwaway data dir, a fixed demo login, and synthetic
          # calendar/contacts/notes data via `tidl-server seed-demo` (see
          # crates/tidl-server/src/seed_demo.rs). Seeds once, then hands off to
          # `cargo leptos watch` so source edits hot-rebuild. Always picks a free
          # port and a brand-new temp data dir so repeated runs never collide
          # with each other, `run-tidl`, or a live `cargo leptos watch`.
          # Logins (fixed — throwaway data only, never point this at anything
          # real):
          #   demo@example.com / demo — seeded with synthetic demo data
          #   admin@localhost / admin         — empty; matches pytest/Playwright
          #                                     defaults so `test-hydration
          #                                     admin@localhost admin` works
          # Usage: gts [port]
          #   port - TCP port to bind (default: a free random port). Pass 3060 to
          #          pin the usual dev port; data is still a throwaway temp dir.
          # `cargo leptos watch` also binds a *second*, separate port for its
          # live-reload websocket. That port comes from `reload-port` in
          # Cargo.toml (hardcoded to 3061) unless overridden via the
          # `LEPTOS_RELOAD_PORT` env var cargo-leptos reads — so we always pick
          # a free one here too. Without this, any explicit `gts 3061`, or a
          # second concurrent `gts` instance (both hitting the same hardcoded
          # 3061), fails with "Reload TCP port ... already in use".
          (pkgs.writeShellScriptBin "gts" ''
            set -euo pipefail
            EMAIL="demo@example.com"
            USERNAME="demo"
            PASSWORD="demo"
            free_port() {
              python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
            }
            PORT="''${1:-$(free_port)}"
            RELOAD_PORT=$(free_port)
            while [ "$RELOAD_PORT" = "$PORT" ]; do
              RELOAD_PORT=$(free_port)
            done
            DATA_DIR=$(mktemp -d /tmp/tidl-gts-XXXXXX)

            echo "=== Building (cargo leptos build) ==="
            cargo leptos build

            # The server refuses to start without a secret (it encrypts stored
            # SMTP passwords and remote CalDAV PATs at rest). Generate a fresh
            # throwaway key in the temp data dir so `gts` always boots.
            #
            # The same base64-encoded 32-byte key also enables ad-hoc git
            # source PAT encryption: `generate-key` emits exactly the format
            # `TIDL_ENCRYPTION_KEY_FILE` expects.
            SECRET_FILE="$DATA_DIR/secret"
            ENC_KEY_FILE="$DATA_DIR/enc-key"
            "$PWD/target/debug/tidl-server" generate-key > "$SECRET_FILE"
            "$PWD/target/debug/tidl-server" generate-key > "$ENC_KEY_FILE"

            # No `tidl_bg.wasm` alias is needed: assets/index.html loads the
            # wasm by absolute path (`init({ module_or_path: '/pkg/tidl.wasm' })`),
            # overriding wasm-bindgen's default `tidl_bg.wasm` URL. Creating the
            # alias is actively harmful under `cargo leptos watch` — cargo-leptos
            # renames `tidl_bg.wasm` onto `tidl.wasm` on every rebuild, which
            # turns `tidl.wasm` into a self-referential symlink (ELOOP → 500).

            echo "=== Creating demo user ($EMAIL) ==="
            TIDL_DATA_DIR="$DATA_DIR" "$PWD/target/debug/tidl-server" create-admin \
              --email "$EMAIL" --username "$USERNAME" --display-name "Demo" --password "$PASSWORD"

            echo "=== Seeding synthetic demo data ==="
            TIDL_DATA_DIR="$DATA_DIR" "$PWD/target/debug/tidl-server" seed-demo --username "$USERNAME"

            # Second, empty admin whose credentials match the pytest/Playwright
            # defaults (--email/--password), so the e2e suites can log in against
            # gts without passing the seeded account's details.
            echo "=== Creating e2e admin (admin@localhost) ==="
            TIDL_DATA_DIR="$DATA_DIR" "$PWD/target/debug/tidl-server" create-admin \
              --email "admin@localhost" --username "admin" --display-name "Admin" --password "admin"

            echo "=== Starting watch server at http://0.0.0.0:$PORT (data: $DATA_DIR) ==="
            echo "    login: $EMAIL / $PASSWORD (seeded demo data)"
            echo "    login: admin@localhost / admin (empty, for e2e)"
            echo "    auto-rebuilds on change — Ctrl-C to stop"
            # `cargo leptos watch` (not a direct exec of the seeded binary), so
            # source edits hot-rebuild. Its own [workspace.metadata.leptos]
            # site-addr is 0.0.0.0:3060, but `TIDL_HOST`/`TIDL_PORT` take
            # precedence in the server's config, so an explicit port still binds
            # here. The demo data seeded above lives in $DATA_DIR and survives
            # every rebuild.
            #
            # `TIDL_SECURE_COOKIE=false`: gts is plain HTTP on 0.0.0.0, so a
            # `Secure` session cookie (the production default) is silently
            # dropped by browsers unless the URL is `localhost`. The login then
            # "succeeds" (a session row is created) but reloads back to the
            # login page. Throwaway dev server, so disable it.
            TIDL_HOST="0.0.0.0" \
            TIDL_PORT="$PORT" \
            TIDL_DATA_DIR="$DATA_DIR" \
            TIDL_SECRET_FILE="$SECRET_FILE" \
            TIDL_ENCRYPTION_KEY_FILE="$ENC_KEY_FILE" \
            TIDL_SECURE_COOKIE="false" \
            DATABASE_URL="sqlite:$DATA_DIR/tidl.db" \
            LEPTOS_RELOAD_PORT="$RELOAD_PORT" \
              exec cargo leptos watch
          '')

          # Cross-compile the server binary for Windows (x86_64-pc-windows-gnu).
          # The WASM frontend (target/site/) is built via cargo-leptos and is
          # platform-independent — it can be reused as-is for a Windows deployment.
          # Usage: build-windows [--release]
          #   Produces: target/x86_64-pc-windows-gnu/{debug,release}/tidl-server.exe
          (pkgs.writeShellScriptBin "build-windows" ''
            set -euo pipefail
            PROFILE_FLAG=""
            PROFILE_DIR="debug"

            if [ "''${1:-}" = "--release" ]; then
              PROFILE_FLAG="--release"
              PROFILE_DIR="release"
            fi

            echo "=== Building WASM frontend (cargo leptos build) ==="
            cargo leptos build $PROFILE_FLAG

            echo "=== Cross-compiling server for Windows ==="
            # Pure CSR: the server no longer renders HTML, so there is no need
            # to align WASM and server build flags (LEPTOS_OUTPUT_NAME,
            # --cfg erase_components). Plain cargo build suffices.
            cargo build --target x86_64-pc-windows-gnu $PROFILE_FLAG \
              --no-default-features --features ssr \
              -p tidl-server

            echo ""
            echo "Done."
            echo "  Windows binary : target/x86_64-pc-windows-gnu/$PROFILE_DIR/tidl-server.exe"
            echo "  Web assets     : target/site/"
          '')

          # Run the Windows .exe via Wine64 on Linux — for local smoke-testing only.
          # On an actual Windows host you would run tidl-server.exe directly.
          # Usage: run-tidl-wine [port] [data-dir]
          #   port     - TCP port to bind (default: 3060)
          #   data-dir - path to database directory (default: ./data-<port>)
          (pkgs.writeShellScriptBin "run-tidl-wine" ''
            set -euo pipefail
            PORT="''${1:-3060}"
            DATA_DIR="''${2:-$PWD/data-$PORT}"
            mkdir -p "$DATA_DIR"

            EXE="$PWD/target/x86_64-pc-windows-gnu/debug/tidl-server.exe"

            echo "=== Building Windows binary (incremental) ==="
            build-windows

            echo "=== Starting tidl (Windows/Wine) at http://127.0.0.1:$PORT (data: $DATA_DIR) ==="
            TIDL_PORT="$PORT" \
            TIDL_DATA_DIR="$DATA_DIR" \
            DATABASE_URL="sqlite:$DATA_DIR/tidl.db" \
              exec wine64 "$EXE"
          '')

          # Run the hydration test suite against the Windows binary under Wine.
          # Starts the .exe in the background, waits for it to listen, runs the
          # tests, then kills the Wine process.
          # Usage: test-hydration-wine <email> <password> [port] [data-dir]
          (pkgs.writeShellScriptBin "test-hydration-wine" ''
            set -euo pipefail
            EMAIL="''${1:-}"
            PASS="''${2:-}"
            PORT="''${3:-3062}"
            DATA_DIR="''${4:-$PWD/data-wine-$PORT}"

            if [ -z "$EMAIL" ] || [ -z "$PASS" ]; then
              echo "Usage: test-hydration-wine <email> <password> [port] [data-dir]"
              echo "Example: test-hydration-wine admin@localhost admin"
              exit 1
            fi

            EXE="$PWD/target/x86_64-pc-windows-gnu/debug/tidl-server.exe"

            echo "=== Building Windows binary (incremental) ==="
            build-windows

            mkdir -p "$DATA_DIR"

            # Derive a username from the email (strip @domain).
            USERNAME="''${EMAIL%%@*}"

            # Bootstrap the database with an admin user so the test login works.
            # The create-admin subcommand is idempotent-ish (fails if user exists,
            # which is fine — we ignore the exit code on the second run).
            echo "=== Bootstrapping admin user '$USERNAME' in fresh database ==="
            TIDL_DATA_DIR="$DATA_DIR" \
            DATABASE_URL="sqlite:$DATA_DIR/tidl.db" \
              wine64 "$EXE" create-admin \
                --email "$EMAIL" \
                --username "$USERNAME" \
                --password "$PASS" \
              || true

            echo "=== Starting tidl.exe under Wine on port $PORT ==="
            TIDL_PORT="$PORT" \
            TIDL_DATA_DIR="$DATA_DIR" \
            DATABASE_URL="sqlite:$DATA_DIR/tidl.db" \
              wine64 "$EXE" &

            WINE_PID=$!

            # Wait for the server to accept connections (up to 30 s).
            echo "=== Waiting for server to be ready ==="
            for i in $(seq 1 30); do
              if curl -sf "http://localhost:$PORT" >/dev/null 2>&1; then
                echo "Server is up."
                break
              fi

              if ! kill -0 "$WINE_PID" 2>/dev/null; then
                echo "Wine process exited unexpectedly."
                exit 1
              fi

              sleep 1
            done

            echo "=== pytest: hydration tests (Wine) ==="
            set +e
            pytest "$PWD/tests/test_hydration.py" -v \
              --tb=short \
              --server-url="http://localhost:$PORT" \
              --email="$EMAIL" \
              --password="$PASS"
            TEST_EXIT=$?
            set -e

            echo "=== Stopping Wine server ==="
            kill "$WINE_PID" 2>/dev/null || true
            wait "$WINE_PID" 2>/dev/null || true

            exit $TEST_EXIT
          '')

          # Package the Windows release binary together with its web assets into a
          # distributable zip. The zip unpacks to tidl-windows-<version>/ and
          # mirrors the server's hardcoded relative paths so the exe finds its
          # files when run from the dist directory:
          #   ./target/site/pkg/  — served at /pkg/
          #   ./assets/           — served at /assets/ and fallback index.html
          # Usage: dist-windows
          #   Produces: dist/tidl-windows-<version>.zip
          (pkgs.writeShellScriptBin "dist-windows" ''
            set -euo pipefail

            echo "=== Building Windows release ==="
            build-windows --release

            VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
            DIST_NAME="tidl-windows-$VERSION"
            DIST_DIR="dist/$DIST_NAME"

            echo "=== Assembling $DIST_NAME ==="
            rm -rf "$DIST_DIR"
            mkdir -p "$DIST_DIR/target/site" "$DIST_DIR/assets"

            cp target/x86_64-pc-windows-gnu/release/tidl-server.exe "$DIST_DIR/"

            # Copy whole directories (not hardcoded file lists) so generated
            # files are picked up automatically — the server serves both dirs
            # wholesale via ServeDir::new(). This matters because wasm-bindgen
            # emits a pkg/snippets/<crate-hash>/ subdir for inline JS snippets
            # that tidl.js statically imports; missing it 404s and aborts the
            # whole module graph before hydrate() ever runs (silent blank
            # screen, no WASM panic since WASM never starts).
            cp -r target/site/pkg "$DIST_DIR/target/site/"
            cp -r assets/. "$DIST_DIR/assets/"
            cp config.toml.example "$DIST_DIR/"
            cp README.md "$DIST_DIR/"

            # Outlook COM bridge (backlog #25) — standalone PowerShell script
            # for the Windows work machine; not built, just carried along.
            mkdir -p "$DIST_DIR/tools/outlook-bridge"
            cp tools/outlook-bridge/bridge.ps1 "$DIST_DIR/tools/outlook-bridge/"
            cp tools/outlook-bridge/bridge.conf.example "$DIST_DIR/tools/outlook-bridge/"
            cp tools/outlook-bridge/install.md "$DIST_DIR/tools/outlook-bridge/"

            # Launcher scripts — cd into the dist dir so the server resolves
            # its hardcoded relative paths (target/site/pkg, assets/) correctly.
            # run.bat — Windows (double-click or cmd.exe)
            # run.sh  — Linux/Wine smoke-testing
            cat > "$DIST_DIR/run.bat" <<'BAT'
@echo off
cd /d "%~dp0"
tidl-server.exe %*
BAT

            cat > "$DIST_DIR/run.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"
exec wine64 ./tidl-server.exe "$@"
SH

            chmod +x "$DIST_DIR/run.sh"

            echo "=== Creating zip ==="
            (cd dist && zip -r "$DIST_NAME.zip" "$DIST_NAME/")

            echo ""
            echo "Done: dist/$DIST_NAME.zip"
            echo "  tidl-server.exe        — Windows: run.bat  |  Linux/Wine: ./run.sh"
            echo "  target/site/pkg/       — WASM and JS bundle (served at /pkg/)"
            echo "  assets/                — index.html + CSS (served at /assets/)"
            echo "  config.toml.example    — copy to config.toml and edit"
            echo "  README.md              — project documentation"
            echo "  tools/outlook-bridge/  — Outlook COM bridge script (see install.md)"
          '')

          # Package the Linux release as a distributable tarball (backlog #148).
          # The layout is defined once in the `tidlLinuxDist` derivation, which
          # this command and the release CI both consume — so there is no second
          # copy of the packaging logic to drift.
          # Usage: dist-linux
          #   Produces: dist/tidl-linux-<arch>-<version>.tar.gz
          (pkgs.writeShellScriptBin "dist-linux" ''
            set -euo pipefail

            echo "=== Building Linux dist (nix build .#linuxDist) ==="
            nix build .#linuxDist

            mkdir -p dist
            cp result/*.tar.gz dist/

            echo ""
            echo "Done: dist/$(basename "$(ls result/*.tar.gz)")"
          '')

          # End-to-end smoke test for the Windows dist package.
          # Builds a fresh release dist, unpacks it to a temp directory, starts
          # the .exe under Wine from the dist dir (so relative paths resolve),
          # checks asset MIME types, then runs the full browser hydration suite.
          # Usage: test-dist-wine <email> <password> [port]
          (pkgs.writeShellScriptBin "test-dist-wine" ''
            set -euo pipefail
            EMAIL="''${1:-}"
            PASS="''${2:-}"
            PORT="''${3:-3066}"

            if [ -z "$EMAIL" ] || [ -z "$PASS" ]; then
              echo "Usage: test-dist-wine <email> <password> [port]"
              echo "Example: test-dist-wine admin@localhost admin"
              exit 1
            fi

            echo "=== Building Windows release dist ==="
            dist-windows

            DIST_ZIP=$(ls -t dist/tidl-windows-*.zip | head -1)
            WORK_DIR=$(mktemp -d /tmp/tidl-dist-test-XXXXXX)
            WINE_PID=""

            trap 'kill "$WINE_PID" 2>/dev/null || true; echo "=== Cleaning up $WORK_DIR ==="; rm -rf "$WORK_DIR"' EXIT

            echo "=== Unpacking $DIST_ZIP to $WORK_DIR ==="
            unzip -q "$DIST_ZIP" -d "$WORK_DIR"
            DIST_DIR=$(ls -d "$WORK_DIR"/tidl-windows-*)

            USERNAME="''${EMAIL%%@*}"

            # Bootstrap an admin user in the fresh database.
            # Both create-admin and the server run from DIST_DIR, so they share
            # the same default ./data directory. No TIDL_DATA_DIR is set here
            # because the Windows binary cannot resolve Unix absolute paths
            # (e.g. /tmp/...) and would silently fall back to ./data anyway.
            echo "=== Bootstrapping admin user '$USERNAME' ==="
            (cd "$DIST_DIR" && \
                wine64 ./tidl-server.exe create-admin \
                  --email "$EMAIL" \
                  --username "$USERNAME" \
                  --password "$PASS") \
              || true

            # Start the release dist binary in the background.
            # Run from DIST_DIR so the server finds target/site/pkg/, assets/,
            # and the ./data directory created by create-admin above.
            echo "=== Starting dist server on port $PORT ==="
            (cd "$DIST_DIR" && \
              TIDL_PORT="$PORT" \
                wine64 ./tidl-server.exe) &

            WINE_PID=$!

            # Wait for the HTTP server to accept connections (up to 30 s).
            echo "=== Waiting for server to be ready ==="
            for i in $(seq 1 30); do
              if curl -sf "http://localhost:$PORT/health" >/dev/null 2>&1; then
                echo "Server is up after $i s."
                break
              fi

              if ! kill -0 "$WINE_PID" 2>/dev/null; then
                echo "Wine process exited unexpectedly."
                exit 1
              fi

              sleep 1
            done

            # Sanity-check asset MIME types — these must be correct for the
            # browser to load the WASM module at all.
            echo "=== Checking asset MIME types ==="
            WASM_CT=$(curl -sf -o /dev/null -w "%{content_type}" \
              "http://localhost:$PORT/pkg/tidl.wasm" || echo "FAILED")

            JS_CT=$(curl -sf -o /dev/null -w "%{content_type}" \
              "http://localhost:$PORT/pkg/tidl.js" || echo "FAILED")

            echo "  tidl.wasm : $WASM_CT"
            echo "  tidl.js   : $JS_CT"

            MIME_OK=1

            echo "$WASM_CT" | grep -qi "wasm" \
              || { echo "FAIL: tidl.wasm has wrong MIME type"; MIME_OK=0; }

            echo "$JS_CT" | grep -qi "javascript" \
              || { echo "FAIL: tidl.js has wrong MIME type"; MIME_OK=0; }

            [ "$MIME_OK" -eq 1 ] \
              && echo "MIME types OK." \
              || { kill "$WINE_PID" 2>/dev/null || true; exit 1; }

            # Run the browser hydration test suite.
            echo "=== pytest: dist hydration tests ==="
            set +e

            pytest "$PWD/tests/test_hydration.py" -v \
              --tb=short \
              --server-url="http://localhost:$PORT" \
              --email="$EMAIL" \
              --password="$PASS"

            TEST_EXIT=$?
            set -e

            echo "=== Stopping dist server ==="
            kill "$WINE_PID" 2>/dev/null || true
            wait "$WINE_PID" 2>/dev/null || true

            exit $TEST_EXIT
          '')

          # Run browser hydration tests against a running local server.
          # Usage: test-hydration <email> <password> [port]
          #   email    - account email used to log into the web UI
          #   password - account password
          #   port     - TCP port the server is listening on (default: 3060)
          (pkgs.writeShellScriptBin "test-hydration" ''
            set -euo pipefail
            EMAIL="''${1:-}"
            PASS="''${2:-}"
            PORT="''${3:-3060}"

            if [ -z "$EMAIL" ] || [ -z "$PASS" ]; then
              echo "Usage: test-hydration <email> <password> [port]"
              echo "Example: test-hydration admin@localhost admin"
              exit 1
            fi

            echo "=== pytest: hydration tests ==="
            pytest "$PWD/tests/test_hydration.py" -v \
              --tb=short \
              --server-url="http://localhost:$PORT" \
              --email="$EMAIL" \
              --password="$PASS"
          '')

          # Run WebDAV/CalDAV compliance tests against a running local server.
          # Usage: test-caldav <username> <password> [port]
          (pkgs.writeShellScriptBin "test-caldav" ''
            set -euo pipefail
            USER="''${1:-}"
            PASS="''${2:-}"
            PORT="''${3:-3060}"

            if [ -z "$USER" ] || [ -z "$PASS" ]; then
              echo "Usage: test-caldav <user> <pass> [port]"
              echo "Example: test-caldav admin mypassword"
              exit 1
            fi

            litmus "http://localhost:$PORT/caldav/$USER/" "$USER" "$PASS"

            echo ""
            echo "=== pytest: CalDAV protocol tests ==="
            pytest "$PWD/tests/test_caldav.py" -v \
              --tb=short -x \
              --server-url="http://localhost:$PORT" \
              --username="$USER" \
              --password="$PASS"
          '')
        ];
      in {
        devShells.default = pkgs.mkShell {
          # nativeBuildInputs: tools needed at build time (compilers, linkers, pkg-config)
          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config
            cmake
            cargo-watch
            cargo-deny
            cargo-leptos
            wasm-bindgen-cli
            binaryen
            sqlx-cli
            cargo-llvm-cov

            # CalDAV/WebDAV compliance testing
            litmus

            testPython

            # Playwright browser binaries for hydration tests
            playwright-driver.browsers

            # Wine64 for running the Windows binary on Linux
            wine64

            # zip for dist-windows packaging
            zip
          ] ++ devScripts;

          # buildInputs: libraries that need to be found at compile / link time
          buildInputs = with pkgs; [
            openssl.dev
          ];

          shellHook = ''
            # RustRover uses a writable copy of the Nix-provided rust-src.
            # The compiler/toolchain stays in the Nix store; RustRover's
            # Standard Library setting points at this writable copy.

            RUST_VERSION="$(${rustToolchain}/bin/rustc --version | awk '{print $2}')"
            RUST_ROVER_ROOT="$HOME/.rust-rover"
            RUST_ROVER_TOOLCHAIN="$RUST_ROVER_ROOT/toolchain"
            RUST_ROVER_SRC="$RUST_ROVER_ROOT/rust-src/$RUST_VERSION"

            mkdir -p "$RUST_ROVER_TOOLCHAIN"

            ln -sfn ${rustToolchain}/bin "$RUST_ROVER_TOOLCHAIN/bin"
            ln -sfn ${rustToolchain}/lib "$RUST_ROVER_TOOLCHAIN/lib"

            if [ ! -d "$RUST_ROVER_SRC" ]; then
              mkdir -p "$RUST_ROVER_SRC"
              cp -r --no-preserve=mode \
                "${rustToolchain}/lib/rustlib/src/rust/library/." \
                "$RUST_ROVER_SRC/"
              chmod -R u+rwX "$RUST_ROVER_SRC"
            fi

            export RUST_SRC_PATH="$RUST_ROVER_SRC"

            # Use the pre-generated .sqlx offline cache so sqlx::query! macros
            # do not require a live database at compile time.
            export SQLX_OFFLINE=true

            # Runtime DATABASE_URL — used by the running server, not the build.
            export DATABASE_URL="sqlite:$PWD/data/tidl.db"

            # Log level — override with e.g. RUST_LOG=trace cargo leptos watch
            export RUST_LOG="''${RUST_LOG:-tidl_server=debug,tidl_web=debug,warn}"

            # Playwright: point the Python package at the Nix-managed browser binaries
            # so `playwright install` is not needed and nothing is downloaded at runtime.
            export PLAYWRIGHT_BROWSERS_PATH="${pkgs.playwright-driver.browsers}"
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true

            # An ambient LD_LIBRARY_PATH (pipewire/alsa from the host session, common
            # under nix-ld) force-loads a newer-glibc libasound.so.2 into Playwright's
            # bundled Chromium, which links glibc 2.40 — it then dies at launch with
            # `version GLIBC_ABI_GNU2_TLS not found`. Nothing in this dev shell needs
            # LD_LIBRARY_PATH (all binaries are Nix-built with their own RPATH), so
            # clear it here to keep the hydration tests working.
            unset LD_LIBRARY_PATH

            # Windows cross-compilation toolchain (x86_64-pc-windows-gnu via MinGW64).
            # mingwCC is NOT in nativeBuildInputs to avoid Nix setup hooks overwriting the
            # host CC variable with the cross-compiler (which would break native builds).
            # Instead, add just the bin/ directory to PATH so the tools are discoverable
            # only when explicitly requested (e.g. by the Windows target linker config).
            export PATH="${mingwCC}/bin:$PATH"
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${mingwCC}/bin/x86_64-w64-mingw32-gcc"
            export CC_x86_64_pc_windows_gnu="${mingwCC}/bin/x86_64-w64-mingw32-gcc"
            export AR_x86_64_pc_windows_gnu="${mingwCC}/bin/x86_64-w64-mingw32-ar"

            # Add winpthread lib + include paths so the Windows Rust std and C deps can find
            # libpthread.a / sched.h. aws-lc-sys bundles jitterentropy which unconditionally
            # includes <sched.h>; that header ships in mingwPthreads, not the base MinGW toolchain.
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L ${mingwPthreads}/lib"
            export CFLAGS_x86_64_pc_windows_gnu="-I${mingwPthreads}/include"

            echo "tidl development environment"
            echo "  cargo leptos watch                        - hot-reload dev server (port 3060)"
            echo "  run-tidl [port] [data]                    - build & run server at custom port (default 3060)"
            echo "  gts [port]                                - disposable test server seeded with synthetic data; hot-reloads on change (optional port, default random)"
            echo "  cargo test                                - run all tests"
            echo "  cargo llvm-cov --open                     - measure test coverage, open HTML report"
            echo "  cargo build                               - build server binary only"
            echo "  prepare-sqlx                              - regenerate .sqlx query cache after schema/query changes"
            echo "  generate-secret [--output p] [--force]    - generate the deployment secret (default ./tidl-secret)"
            echo "  test-caldav <user> <pass>                 - run WebDAV/CalDAV compliance tests"
            echo "  test-hydration <email> <pass>             - run browser hydration tests"
            echo "  build-windows [--release]                 - cross-compile server for Windows (x86_64)"
            echo "  dist-windows                              - build release + package as dist zip"
            echo "  dist-linux                                - build release + package as dist tarball"
            echo "  run-tidl-wine [port] [data]               - run Windows .exe via Wine64 for local testing"
            echo "  test-hydration-wine <email> <pass> [port] - hydration tests against Wine server"
            echo "  test-dist-wine <email> <pass> [port]      - build dist, unpack, full smoke test"
            echo ""
            echo "  RustRover toolchain: $RUST_ROVER_TOOLCHAIN"
            echo "  RustRover stdlib:    $RUST_SRC_PATH"
          '';
        };

        # Minimal shell for building release artifacts (backlog #148) in CI.
        # The default shell drags in Wine and Playwright (multi-GB) that a dist
        # build does not need; this one carries only the toolchain and the
        # Windows cross-compiler, mirroring the relevant env from the default
        # shellHook. Used as `nix develop .#dist -c dist-windows`.
        devShells.dist = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain
            pkg-config
            cmake
            cargo-leptos
            wasm-bindgen-cli
            binaryen
            zip
          ] ++ devScripts;

          buildInputs = [ pkgs.openssl.dev ];

          shellHook = ''
            export SQLX_OFFLINE=true
            export PATH="${mingwCC}/bin:$PATH"
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${mingwCC}/bin/x86_64-w64-mingw32-gcc"
            export CC_x86_64_pc_windows_gnu="${mingwCC}/bin/x86_64-w64-mingw32-gcc"
            export AR_x86_64_pc_windows_gnu="${mingwCC}/bin/x86_64-w64-mingw32-ar"
            export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L ${mingwPthreads}/lib"
            export CFLAGS_x86_64_pc_windows_gnu="-I${mingwPthreads}/include"
          '';
        };

        packages = {
          default = tidlPackage;
        } // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          # OCI image (backlog #149): `nix build .#docker` → a loadable image
          # tarball. CI builds this per-arch and publishes it to ghcr.io.
          docker = tidlDockerImage;

          # Linux release tarball (backlog #148): `nix build .#linuxDist`.
          linuxDist = tidlLinuxDist;
        };

        formatter = pkgs.nixfmt-rfc-style;
      }) // {
        nixosModules.default = { config, lib, pkgs, ... }:
          with lib;
          let
            cfg = config.services.tidl;

            # JSON config file written to the Nix store (no secrets; pat_file paths
            # point to sops-decrypted runtime files that are read by tidl-server).
            gitSourcesJson = pkgs.writeText "tidl-git-sources.json" (builtins.toJSON (
              map (s: {
                username = s.username;
                name = s.name;
                url = s.url;
                branch = s.branch;
                pat_file = toString s.patFile;
              }) cfg.gitSources
            ));
          in {
            options.services.tidl = {
              enable = mkEnableOption "tidl CalDAV server";

              package = mkOption {
                type = types.package;
                default = self.packages.${pkgs.system}.default;
                description = "The tidl package to use";
              };

              port = mkOption {
                type = types.port;
                default = 3060;
                description = "Port to listen on";
              };

              host = mkOption {
                type = types.str;
                default = "127.0.0.1";
                description = "Host to bind to";
              };

              domain = mkOption {
                type = types.str;
                default = "tidl.dev";
                description = "Base domain for the server";
              };

              dataDir = mkOption {
                type = types.path;
                default = "/var/lib/tidl";
                description = "Directory to store calendar data";
              };

              user = mkOption {
                type = types.str;
                default = "tidl";
                description = "User account under which tidl runs";
              };

              group = mkOption {
                type = types.str;
                default = "tidl";
                description = "Group under which tidl runs";
              };

              adminEmail = mkOption {
                type = types.nullOr types.str;
                default = null;
                description = "Notify this address when a new account is created. Null disables notifications.";
              };

              allowRegistration = mkOption {
                type = types.bool;
                default = true;
                description = "Allow new users to self-register via the web UI.";
              };

              secureCookie = mkOption {
                type = types.bool;
                default = true;
                description = ''
                  Whether the session cookie gets the `Secure` attribute (sent
                  only over HTTPS). Leave `true` for the standard deployment
                  behind a TLS-terminating reverse proxy; set `false` only for
                  a deployment deliberately served over plain HTTP (e.g.
                  direct LAN access with no TLS in front).
                '';
              };

              initialAdmin = {
                enable = mkEnableOption "bootstrap an initial admin user on first start";

                email = mkOption {
                  type = types.str;
                  description = "Email address for the initial admin user";
                };

                username = mkOption {
                  type = types.str;
                  default = "admin";
                  description = "Username for the initial admin user";
                };

                displayName = mkOption {
                  type = types.str;
                  default = "";
                  description = "Display name for the initial admin user";
                };

                passwordHashFile = mkOption {
                  type = types.path;
                  description = "Path to a file containing an argon2id password hash (from `tidl hash-password`)";
                };
              };

              secretFile = mkOption {
                type = types.path;
                description = ''
                  Path to a file containing the secret used to derive the AES
                  key that encrypts stored SMTP passwords and remote CalDAV
                  personal access tokens. Required — the server refuses to
                  start without one (there is no insecure default). Generate
                  one with `tidl-server generate-key`, then manage it with
                  sops-nix:
                    sops.secrets."tidl-secret" = {};
                    services.tidl.secretFile = config.sops.secrets."tidl-secret".path;
                '';
              };

              encryptionKeyFile = mkOption {
                type = types.nullOr types.path;
                default = null;
                description = ''
                  Path to a file containing exactly 32 random bytes used as the
                  AES-256-GCM key for encrypting ad-hoc git source PATs.
                  Manage with sops-nix:
                    sops.secrets."tidl-enc-key" = {};
                    services.tidl.encryptionKeyFile =
                      config.sops.secrets."tidl-enc-key".path;
                  If null, ad-hoc git sources with stored credentials are disabled.
                '';
              };

              gitSources = mkOption {
                type = types.listOf (types.submodule {
                  options = {
                    username = mkOption {
                      type = types.str;
                      description = "tidl username this source belongs to";
                    };

                    name = mkOption {
                      type = types.str;
                      description = "Display name for this git source";
                    };

                    url = mkOption {
                      type = types.str;
                      description = "HTTPS git remote URL";
                    };

                    branch = mkOption {
                      type = types.str;
                      default = "main";
                      description = "Branch to track";
                    };

                    patFile = mkOption {
                      type = types.path;
                      description = ''
                        Path to a file containing the PAT (personal access token).
                        Use sops-nix: config.sops.secrets."my-pat".path
                      '';
                    };
                  };
                });

                default = [];
                description = "Declarative git-backed markdown sources managed by NixOS.";
              };

              oidc = {
                enable = mkEnableOption "OIDC authentication";

                issuerUrl = mkOption {
                  type = types.str;
                  description = "OIDC issuer URL (e.g. https://auth.example.com/oauth2/openid/tidl)";
                };

                clientId = mkOption {
                  type = types.str;
                  description = "OIDC client ID";
                };

                clientSecretFile = mkOption {
                  type = types.path;
                  description = "Path to a file containing the OIDC client secret";
                };

                providerName = mkOption {
                  type = types.str;
                  default = "SSO";
                  description = "Display name for the sign-in button";
                };
              };

              smtp = {
                enable = mkEnableOption "declarative SMTP configuration";

                host = mkOption {
                  type = types.str;
                  description = "SMTP server hostname";
                };

                port = mkOption {
                  type = types.port;
                  default = 587;
                  description = "SMTP server port";
                };

                tlsMode = mkOption {
                  type = types.enum [ "none" "starttls" "tls" ];
                  default = "starttls";
                  description = "SMTP transport security";
                };

                username = mkOption {
                  type = types.str;
                  default = "";
                  description = "SMTP auth username. Leave empty for an unauthenticated relay.";
                };

                passwordFile = mkOption {
                  type = types.nullOr types.path;
                  default = null;
                  description = ''
                    Path to a file containing the SMTP auth password.
                    Use sops-nix: config.sops.secrets."tidl-smtp-password".path
                    Leave null for an unauthenticated relay.
                  '';
                };

                fromAddress = mkOption {
                  type = types.str;
                  description = "Envelope/header From address for outgoing mail";
                };

                fromName = mkOption {
                  type = types.str;
                  default = "";
                  description = "Display name for the From header";
                };
              };
            };

            config = mkIf cfg.enable {
              users.users.${cfg.user} = {
                isSystemUser = true;
                group = cfg.group;
                home = cfg.dataDir;
                createHome = true;
              };

              users.groups.${cfg.group} = { };

              systemd.services.tidl = {
                description = "tidl CalDAV Server";
                wantedBy = [ "multi-user.target" ];
                after = [ "network.target" ];

                serviceConfig = {
                  Type = "simple";
                  User = cfg.user;
                  Group = cfg.group;
                  ExecStart = "${cfg.package}/bin/tidl";
                  WorkingDirectory = "${cfg.package}/share/tidl";
                  Restart = "on-failure";
                  RestartSec = 5;

                  # Hardening
                  NoNewPrivileges = true;
                  PrivateTmp = true;
                  ProtectSystem = "strict";
                  ProtectHome = true;
                  ReadWritePaths = [ cfg.dataDir ];

                  # Allow reading sops-decrypted PAT files, the server secret,
                  # the encryption key, the SMTP password, and the OIDC client secret.
                  BindReadOnlyPaths =
                    [ (toString cfg.secretFile) ]
                    ++ (lib.optional (cfg.encryptionKeyFile != null)
                      (toString cfg.encryptionKeyFile))
                    ++ (map (s: toString s.patFile) cfg.gitSources)
                    ++ (lib.optional (cfg.smtp.enable && cfg.smtp.passwordFile != null)
                      (toString cfg.smtp.passwordFile))
                    ++ (lib.optional cfg.oidc.enable
                      (toString cfg.oidc.clientSecretFile));
                } // lib.optionalAttrs cfg.initialAdmin.enable {
                  ExecStartPre = pkgs.writeShellScript "tidl-create-admin" ''
                    ${cfg.package}/bin/tidl create-admin \
                      --email    ${lib.escapeShellArg cfg.initialAdmin.email} \
                      --username ${lib.escapeShellArg cfg.initialAdmin.username} \
                      --display-name ${lib.escapeShellArg cfg.initialAdmin.displayName} \
                      --password-hash "$(cat ${toString cfg.initialAdmin.passwordHashFile})" \
                    || true
                  '';
                };

                environment = {
                  TIDL_HOST               = cfg.host;
                  TIDL_PORT               = toString cfg.port;
                  TIDL_DOMAIN             = cfg.domain;
                  TIDL_DATA_DIR           = cfg.dataDir;
                  TIDL_ALLOW_REGISTRATION = if cfg.allowRegistration then "true" else "false";
                  TIDL_SECURE_COOKIE      = if cfg.secureCookie then "true" else "false";
                  TIDL_SECRET_FILE        = toString cfg.secretFile;
                } // lib.optionalAttrs (cfg.adminEmail != null) {
                  TIDL_ADMIN_EMAIL = cfg.adminEmail;
                } // lib.optionalAttrs (cfg.encryptionKeyFile != null) {
                  TIDL_ENCRYPTION_KEY_FILE = toString cfg.encryptionKeyFile;
                } // lib.optionalAttrs (cfg.gitSources != []) {
                  TIDL_GIT_SOURCES_FILE = "${gitSourcesJson}";
                } // lib.optionalAttrs cfg.oidc.enable {
                  TIDL_OIDC_ISSUER_URL         = cfg.oidc.issuerUrl;
                  TIDL_OIDC_CLIENT_ID          = cfg.oidc.clientId;
                  TIDL_OIDC_CLIENT_SECRET_FILE = toString cfg.oidc.clientSecretFile;
                  TIDL_OIDC_PROVIDER_NAME      = cfg.oidc.providerName;
                } // lib.optionalAttrs cfg.smtp.enable ({
                  TIDL_SMTP_HOST         = cfg.smtp.host;
                  TIDL_SMTP_PORT         = toString cfg.smtp.port;
                  TIDL_SMTP_TLS_MODE     = cfg.smtp.tlsMode;
                  TIDL_SMTP_USERNAME     = cfg.smtp.username;
                  TIDL_SMTP_FROM_ADDRESS = cfg.smtp.fromAddress;
                  TIDL_SMTP_FROM_NAME    = cfg.smtp.fromName;
                } // lib.optionalAttrs (cfg.smtp.passwordFile != null) {
                  TIDL_SMTP_PASSWORD_FILE = toString cfg.smtp.passwordFile;
                });
              };
            };
          };
      };
}
