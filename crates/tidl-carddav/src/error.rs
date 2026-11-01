//! Typed errors for this crate's CardDAV REPORT/PROPFIND XML handling.
//!
//! Replaces the `anyhow::Result` `carddav.rs`/`xml.rs` used to return -
//! library crates shouldn't leak `anyhow::Error` into callers (CLAUDE.md
//! §11). Mirrors `tidl_caldav::error::CalDavError`'s `Xml` variant, kept as
//! a separate type rather than reused because it's this crate's own error
//! domain, not a CalDAV error (`tidl-carddav` depends on `tidl-caldav`, not
//! the other way around).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CardDavError {
    #[error("{context} XML parse error: {source}")]
    Xml {
        context: &'static str,
        #[source]
        source: quick_xml::Error,
    },
}
