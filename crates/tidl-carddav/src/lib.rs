//! tidl-carddav - CardDAV protocol implementation (RFC 6352)
//!
//! Provides vCard parse/serialize and CardDAV REPORT/PROPFIND XML handling
//! for the tidl server's address-book sync. Mirrors `tidl-caldav`'s shape;
//! see that crate's doc comment and CLAUDE.md §6/§11 for the library-crate
//! rules both follow (no Leptos, no Axum, no sqlx).
//!
//! Generic WebDAV parsing (PROPFIND/PROPPATCH request parsing, the
//! `PropFind`/`Response`/`PropStat`/`Property` types) and ETag machinery are
//! reused directly from `tidl_caldav` rather than duplicated — see
//! `src/xml.rs`'s doc comment for why only the multistatus *response*
//! envelope gets its own implementation here.

pub mod carddav;
pub mod error;
pub mod vcard;
pub mod xml;

pub use error::CardDavError;

pub use carddav::{
    generate_report_response, parse_addressbook_multiget, parse_addressbook_query, AddressObject,
    AddressbookMultiget, AddressbookQuery,
};
pub use vcard::{
    generate_vcard, parse_vcard, VCardAddress, VCardError, VCardFields, VCardMethod, VCardPhoto,
};
pub use xml::generate_propfind_response;

// Re-exported for callers that only depend on `tidl-carddav` — the generic
// WebDAV/ETag pieces this crate builds on.
pub use tidl_caldav::etag::{check_if_match, check_if_none_match, generate_etag};
pub use tidl_caldav::webdav::{
    parse_propfind, parse_proppatch, Depth, PropFind, PropFindType, PropPatchOp, PropStat,
    Property, Response,
};
