#!/usr/bin/env bash
# Guard: no wall-clock time calls *inside* Suspend::new async blocks in component files.
#
# Using Utc::now() / SystemTime::now() inside a Suspend::new or Resource
# computation closure causes SSR and WASM to produce different values, which
# panics the tachys hydration engine.  The date/time must be captured *before*
# entering the async block, computed once in the synchronous component body.
#
# Safe pattern (capture outside, use inside):
#   let today = Utc::now().date_naive();   // ← in component body (sync)
#   Suspend::new(async move {
#       let today = today;                  // ← capture, no re-call
#       ...
#   })
#
# Dangerous pattern (causes hydration panic):
#   Suspend::new(async move {
#       let today = Utc::now().date_naive(); // ← SSR and WASM diverge!
#   })

set -euo pipefail

COMPONENT_DIR="crates/tidl-web/src/components"

# Multi-line search: now() appearing anywhere inside a Suspend::new(async block.
# We cap the lookahead at ~800 chars (~10-15 lines) to avoid false positives from
# unrelated now() calls further in the file.
if rg -U --multiline-dotall \
       'Suspend::new\(async.{0,800}?now\(\)' \
       "$COMPONENT_DIR" --with-filename --line-number; then
    echo ""
    echo "ERROR: wall-clock now() call found inside a Suspend::new block."
    echo "Capture the value before entering the async block to keep SSR and"
    echo "WASM outputs identical. See focus.rs FocusView for the pattern."
    exit 1
fi

echo "OK: no wall-clock time calls inside Suspend::new blocks."
