# Separate Wire Types from Server Logic in api.rs

`tidl-web/src/api.rs` combines two unrelated concerns in 720 lines:

1. **Shared data types** (`Event`, `Alarm`, `NewEvent`, `Todo`, `TicklerItem`, `AuthUsers`) —
   used by both the WASM frontend and the SSR server.
2. **Server functions** — each containing 30–80 lines of `#[cfg(feature = "ssr")]` database
   access, iCal construction, and ETag generation.

This makes it impossible to import a shared type without also pulling in all the SSR
server-function machinery.

## Fix

Extract all shared types into `tidl-web/src/types.rs`. Keep server functions in `api.rs` and
re-export all types from `api` for backward compatibility with existing imports.

## Resolution

Implemented. `types.rs` contains all shared structs. `api.rs` re-exports them via
`pub use crate::types::*`.
