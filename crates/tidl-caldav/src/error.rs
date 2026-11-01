//! Typed errors for this crate's CalDAV/WebDAV/iCalendar parsing.
//!
//! Replaces the `anyhow::Result` these modules used to return — library
//! crates shouldn't leak `anyhow::Error` into callers (CLAUDE.md §11).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CalDavError {
    #[error("{context} XML parse error: {source}")]
    Xml {
        context: &'static str,
        #[source]
        source: quick_xml::Error,
    },
    /// A request body that's well-formed XML but violates a protocol
    /// requirement (missing/empty required element, wrong structure, …).
    #[error("{0}")]
    InvalidRequest(String),
    #[error("failed to parse iCalendar data: {0}")]
    IcalParse(String),
    #[error("no UID found in iCalendar data")]
    MissingUid,
    #[error("no components found in iCalendar data")]
    MissingComponent,
}
