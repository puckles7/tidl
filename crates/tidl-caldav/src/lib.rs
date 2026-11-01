//! tidl-caldav - CalDAV and WebDAV protocol implementation
//!
//! This crate provides the core CalDAV/WebDAV protocol functionality
//! for the tidl server.

pub mod caldav;
pub mod change;
pub mod error;
pub mod etag;
pub mod icalendar_utils;
pub mod webdav;

pub use error::CalDavError;

pub use caldav::{
    escape_xml, generate_report_response, parse_calendar_multiget, parse_calendar_query,
    CalendarFilter, CalendarMultiget, CalendarObject, CalendarQuery, ComponentFilter, TimeRange,
};
pub use etag::{
    check_if_match, check_if_none_match, generate_etag, generate_weak_etag, validate_etag,
};
pub use icalendar_utils::{
    create_event, create_todo, extract_component_type, extract_event_fields, extract_partstat,
    extract_uid, ical_dt_to_sqlite, ical_duration_str, inject_partstat, parse_icalendar,
    parse_simple_duration, prop_value, prop_value_with_params, unfold_ical, validate_icalendar,
    EventFields,
};
pub use webdav::{
    generate_propfind_response, generate_proppatch_response, parse_propfind, parse_proppatch,
    properties, Depth, MultiStatusResponse, PropFind, PropFindType, PropPatchOp, PropStat,
    Property, Response,
};
