# Replace Glob Re-exports in tidl-caldav/lib.rs

`tidl-caldav/src/lib.rs` uses `pub use module::*` for all four submodules, flooding the
crate's public API surface and making it impossible to know where a given symbol originates
without tracing imports.

## Fix

Replace glob re-exports with explicit `pub use module::TypeName` statements.

## Resolution

Implemented. Each re-exported symbol is listed explicitly.
