# @mention popup mispositioned when note text wraps to multiple lines

In the note editor's `@mention` typeahead (#80), the popup is meant to anchor at the caret (Slack/GitHub-style, via the mirror-div caret-coordinate technique). When the textarea contains only a single short line, this works. When the textarea already has enough text to wrap onto a second (or later) line and the user types `@` there, the popup appears far from the actual caret — observed floating near the top-right of the textarea instead of next to the `@` on the wrapped line.

## Dependencies

- [80 - @mention contact linking in notes](80-note-contact-mention.md) — introduced the popup and the mirror-div caret positioning this bug is in.

## Context / Background

Reported by the user with a screenshot: a note containing wrapped filler text ("Some textSome textSome text..." wrapping onto a second line) followed by `@` on the second line. The suggestion popup rendered near the top-right of the textarea, nowhere near the `@` character.

The caret position is computed by `caret_client_coords` (`components/notes/note_editor_modal.rs`), which mirrors the textarea's text into an offscreen div (matching font/box CSS) up to the caret, then reads `getBoundingClientRect()` on a marker span at the end. The mismatch only shows up once wrapping is involved, so the likely culprit is the mirror div's width/wrapping not exactly matching the real textarea's rendered width (e.g. missing `white-space: pre-wrap`/`word-wrap`, a box-sizing/padding/border discrepancy, or the mirror div not being attached to the DOM with the same computed width at measurement time).

## Functional Requirements

- The `@mention` popup must anchor at the actual caret position regardless of how many lines of text precede it or whether the current line has wrapped.
- No regression to the single-line/no-wrap case, which already works correctly.

## Open Questions

- Root cause to be confirmed during investigation — is it a CSS mismatch between the real textarea and the mirror div, or a stale measurement (e.g. read before the mirror div's layout reflows)?

## Resolution

Root cause confirmed as a CSS mismatch, specifically in how the style-copying loop talked to the DOM: `caret_client_coords` read/wrote the mirror div's style using camelCase JS property names (`"fontFamily"`, `"whiteSpace"`, `"paddingTop"`, etc.), but `CSSStyleDeclaration.getPropertyValue()` / `.setProperty()` require CSS syntax (hyphen-case) property names — camelCase names simply don't match anything, so `get_property_value` silently returned `""` for all 18 properties in the loop and the subsequent `set_property` calls were no-ops. The mirror div therefore rendered with the browser's default `<div>` styling the whole time, not a copy of the textarea's.

This was masked for single-line notes because font metrics are largely inherited from `body` regardless (mirror and textarea often end up with the same font-family/size by CSS inheritance even without the explicit copy succeeding), so short notes' caret math happened to come out close enough. The default `white-space: normal` is what actually broke things: unlike a `<textarea>`'s `white-space: pre-wrap`, `normal` collapses explicit newlines and reflows text at ordinary word-wrap points — so as soon as a note had a line break or wrapped onto a second line before the `@`, the marker span's computed position no longer had any relationship to where the real caret sat.

**Fix**: changed all 18 property names in the copy loop from camelCase to hyphen-case (`"font-family"`, `"white-space"`, `"padding-top"`, etc.), with a comment on the API contract so this can't silently regress again. No other logic changed — the mirror-div technique itself, the marker span, and the coordinate math were all already correct once the div actually receives the textarea's real styling.

Verified by the user in a real browser against a `gts` test server: typing wrapped/multi-line note text followed by `@` now anchors the popup correctly at the caret.

`cargo test --workspace` (440 passed, 0 failed) and `cargo build --target wasm32-unknown-unknown -p tidl-web` (clean) both pass. No new automated test was added — this is `#[cfg(feature = "hydrate")]`-only DOM/CSS interaction code with no `wasm_bindgen_test` harness set up anywhere in `tidl-web` yet (same gap #80 itself noted), and standing one up was out of scope for a one-line-root-cause bugfix.
