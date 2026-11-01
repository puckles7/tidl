# Refactor caldav_handlers.rs

`caldav_handlers.rs` is 2098 lines. Two specific issues:

## RFC 4918 If: header parser

`check_if_header`, `evaluate_if_group`, and `lock_token_in_if_header` implement ~200 lines of
RFC 4918 §10.4 logic. They don't belong in the handler file — they form a self-contained
module that should live in `src/if_header.rs`.

## Repeated copy/move pattern

The overwrite/destination/status pattern appears verbatim in all four of
`caldav_collection_copy`, `caldav_collection_move`, `caldav_event_copy`, `caldav_event_move`:

```rust
if dst_exists && !overwrite { return Err(StatusCode::PRECONDITION_FAILED); }
if dst_exists { storage.delete_<thing>(...).await?; }
Ok(if dst_exists { StatusCode::NO_CONTENT } else { StatusCode::CREATED })
```

Extract this into a helper.

## Fix

- Move the three `If:` functions to `tidl-server/src/if_header.rs`.
- Add an `overwrite_status(dst_existed: bool) -> StatusCode` helper.

## Resolution

Implemented. `if_header.rs` module added to `tidl-server`. `caldav_handlers.rs` imports from
there. `overwrite_status` helper eliminates the repeated ternary.
