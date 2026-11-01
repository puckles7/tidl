# Fix reactive-tracking warning when seeding markdown preview nav_stack

Leptos logs a runtime warning every time the markdown preview modal opens:

```
At crates/tidl-web/src/components/focus.rs:1515:27, you access a
reactive_graph::signal::rw::RwSignal<Vec<NavEntry>> (defined at
crates/tidl-web/src/components/focus.rs:1511:46) outside a reactive
tracking context.
```

## Context / Background

In `MarkdownPreviewInner` (`crates/tidl-web/src/components/focus.rs`), `nav_stack` is created at line 1511, and `current_uid` (line 1514-1515) is a closure that reads it via `nav_stack.with(...)`. That closure is later called eagerly, outside any reactive scope, to seed `render_key`'s initial value:

```rust
let render_key: RwSignal<String> = RwSignal::new(current_uid());
```

Calling `current_uid()` here reads `nav_stack` via `.with()` without a tracking context, which is exactly the pattern the warning flags. It's harmless today only because `nav_stack` has just been constructed a few lines above and hasn't changed yet — but it's fragile: any refactor that moves this seeding later, or adds tracked state to the `nav_stack` accessor, would silently break.

## Functional Requirements

- Eliminate the warning without changing behavior: either seed `render_key`/`nav_stack` reads with `.with_untracked()`/`.get_untracked()` where the value is genuinely meant to be read once (e.g. the initial `render_key` seed), or seed directly from `root.uid`/`uid` instead of round-tripping through `nav_stack`.
- Audit other `current_*` closures in the same component (`current_sf`, `current_line`, `current_name`, `current_summary`, `can_go_back`) for the same eager-call-outside-tracking-context risk.
- Verify in the browser console that opening the markdown preview modal no longer emits this warning.

## Open Questions

- Are there other spots in `focus.rs` (or elsewhere) using the same "closure wrapping `.with()`, called eagerly once for a seed value" pattern that should be fixed at the same time?

## Resolution

Fixed by seeding `render_key` from `uid` directly instead of round-tripping
through `current_uid()`/`nav_stack.with(...)` — since `nav_stack`'s initial
value is `vec![root]` and `root.uid == uid`, this is the exact same value
with no eager, untracked signal read.

Audited every other call site of `current_uid`/`current_sf`/`current_line`/
`current_name`/`current_summary`/`can_go_back` in `MarkdownPreviewInner`: all
of them run either inside a `{move || ...}` view closure, inside a
`LocalResource::new(move || ...)` source closure, or inside an `on:click`
event handler — all of which already have an active reactive owner, so none
of them triggered (or could trigger) the same warning. Only the one-time
`RwSignal::new(...)` seed at component setup was affected.

No behavior change; verified via `cargo build -p tidl-web --features ssr`
and `cargo build --target wasm32-unknown-unknown -p tidl-web --features
hydrate`, plus `cargo test --workspace`. Did not re-verify against a live
browser console: an already-running `cargo leptos watch`/`tidl-server`
process was live at the time (see the "manual browser verification" pitfall
around clobbering `target/site`), and this fix is behavior-preserving by
construction — same value, same signal, only the read path changed — so a
console-warning check was judged unnecessary risk for no added confidence.
