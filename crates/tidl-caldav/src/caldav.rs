//! CalDAV protocol implementation (RFC 4791)
//!
//! Handles REPORT request parsing (calendar-query and calendar-multiget)
//! and the corresponding multi-status response generation.

use quick_xml::events::BytesRef;
use quick_xml::{events::Event as XmlEvent, Reader};
use serde::{Deserialize, Serialize};

use crate::error::CalDavError;

type Result<T> = std::result::Result<T, CalDavError>;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarQuery {
    pub filter: Option<CalendarFilter>,
    /// Requested properties; empty means "all".
    pub properties: Vec<String>,
    /// Whether to include the raw iCalendar data in the response.
    pub include_calendar_data: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarFilter {
    pub comp_filter: ComponentFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentFilter {
    pub name: String,
    pub time_range: Option<TimeRange>,
    pub sub_filters: Vec<ComponentFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarMultiget {
    pub hrefs: Vec<String>,
    pub properties: Vec<String>,
    /// Whether to include the raw iCalendar data in the response.
    pub include_calendar_data: bool,
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parse a CalDAV `calendar-query` REPORT body (RFC 4791 §7.8).
pub fn parse_calendar_query(xml: &str) -> Result<CalendarQuery> {
    let mut reader = Reader::from_str(xml.trim());
    reader.config_mut().trim_text(true);

    let mut properties: Vec<String> = Vec::new();
    let mut include_calendar_data = false;
    let mut filter: Option<CalendarFilter> = None;
    let mut inside_prop = false;
    let mut comp_filter_stack: Vec<ComponentFilter> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "prop" => inside_prop = true,
                    "calendar-data" => {
                        include_calendar_data = true;
                        if inside_prop {
                            properties.push("calendar-data".into());
                        }
                    }
                    "getetag" | "displayname" | "resourcetype" | "getcontenttype"
                    | "getlastmodified"
                        if inside_prop =>
                    {
                        properties.push(local.to_string());
                    }
                    "comp-filter" => {
                        let name = attr_value(e.attributes(), "name")
                            .unwrap_or_else(|| "VCALENDAR".into());
                        comp_filter_stack.push(ComponentFilter {
                            name,
                            time_range: None,
                            sub_filters: vec![],
                        });
                    }
                    "time-range" => {
                        let start = attr_value(e.attributes(), "start");
                        let end = attr_value(e.attributes(), "end");
                        if let Some(cf) = comp_filter_stack.last_mut() {
                            cf.time_range = Some(TimeRange { start, end });
                        }
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "prop" => inside_prop = false,
                    "comp-filter" => {
                        if let Some(finished) = comp_filter_stack.pop() {
                            if let Some(parent) = comp_filter_stack.last_mut() {
                                parent.sub_filters.push(finished);
                            } else {
                                // This was the root VCALENDAR filter
                                filter = Some(CalendarFilter {
                                    comp_filter: finished,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(source) => {
                return Err(CalDavError::Xml {
                    context: "calendar-query",
                    source,
                })
            }
            _ => {}
        }
    }

    if properties.is_empty() {
        properties = vec!["getetag".into(), "calendar-data".into()];
        include_calendar_data = true;
    }

    Ok(CalendarQuery {
        filter,
        properties,
        include_calendar_data,
    })
}

/// Parse a CalDAV `calendar-multiget` REPORT body (RFC 4791 §7.9).
pub fn parse_calendar_multiget(xml: &str) -> Result<CalendarMultiget> {
    let mut reader = Reader::from_str(xml.trim());

    let mut hrefs: Vec<String> = Vec::new();
    let mut properties: Vec<String> = Vec::new();
    let mut include_calendar_data = false;
    let mut inside_href = false;
    let mut inside_prop = false;
    let mut text_buf = String::new();

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "href" => {
                        inside_href = true;
                        text_buf.clear();
                    }
                    "prop" => inside_prop = true,
                    "calendar-data" => {
                        include_calendar_data = true;
                        if inside_prop {
                            properties.push("calendar-data".into());
                        }
                    }
                    "getetag" | "displayname" | "resourcetype" | "getcontenttype"
                    | "getlastmodified"
                        if inside_prop =>
                    {
                        properties.push(local.to_string());
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                if local == "calendar-data" && inside_prop {
                    include_calendar_data = true;
                    properties.push("calendar-data".into());
                }
            }
            Ok(XmlEvent::Text(ref e)) => {
                if inside_href {
                    text_buf.push_str(&e.xml10_content());
                }
            }
            Ok(XmlEvent::GeneralRef(ref e)) => {
                if inside_href {
                    text_buf.push_str(&entity_ref_text(e));
                }
            }
            Ok(XmlEvent::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "href" => {
                        let href = text_buf.trim().to_string();
                        if !href.is_empty() {
                            hrefs.push(href);
                        }
                        inside_href = false;
                    }
                    "prop" => inside_prop = false,
                    _ => {}
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(source) => {
                return Err(CalDavError::Xml {
                    context: "calendar-multiget",
                    source,
                })
            }
            _ => {}
        }
    }

    if properties.is_empty() {
        properties = vec!["getetag".into(), "calendar-data".into()];
        include_calendar_data = true;
    }

    Ok(CalendarMultiget {
        hrefs,
        properties,
        include_calendar_data,
    })
}

// ── Response generation ───────────────────────────────────────────────────────

/// A single calendar object in a REPORT response.
#[derive(Debug, Clone)]
pub struct CalendarObject {
    pub href: String,
    pub etag: String,
    pub calendar_data: Option<String>,
}

/// Generate a `207 Multi-Status` XML response for a REPORT.
pub fn generate_report_response(results: Vec<CalendarObject>) -> Result<String> {
    let responses = results
        .iter()
        .map(format_calendar_object)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
{}
</D:multistatus>"#,
        responses
    ))
}

fn format_calendar_object(obj: &CalendarObject) -> String {
    let cal_data_xml = obj
        .calendar_data
        .as_ref()
        .map(|d| {
            format!(
                "        <C:calendar-data>{}</C:calendar-data>",
                escape_xml(d)
            )
        })
        .unwrap_or_default();

    format!(
        r#"  <D:response>
    <D:href>{}</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>{}</D:getetag>
{}
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>"#,
        escape_xml(&obj.href),
        obj.etag,
        cal_data_xml
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn local_name(name: &str) -> String {
    name.split(':').next_back().unwrap_or(name).to_lowercase()
}

fn attr_value(attrs: quick_xml::events::attributes::Attributes, key: &str) -> Option<String> {
    for attr in attrs.flatten() {
        let attr_name = local_name(attr.key.as_ref());
        if attr_name == key {
            return Some(attr.value.into_owned());
        }
    }
    None
}

/// Decode a general entity/character reference emitted as its own
/// `Event::GeneralRef` (quick-xml splits `&amp;` and friends out of `Text`
/// events), so hrefs containing e.g. `&` still resolve to their characters.
pub(crate) fn entity_ref_text(reference: &BytesRef) -> String {
    if reference.is_char_ref() {
        match reference.resolve_char_ref() {
            Ok(Some(ch)) => ch.to_string(),
            _ => format!("&{};", reference.as_ref()),
        }
    } else {
        quick_xml::escape::resolve_predefined_entity(reference.as_ref())
            .map(str::to_string)
            .unwrap_or_else(|| format!("&{};", reference.as_ref()))
    }
}

/// Escape text for embedding as XML character data (hrefs, display names, etc.).
///
/// Anything derived from user-controlled strings (calendar names, event UIDs)
/// must go through this before being interpolated into a hand-built XML
/// response, or a bare `&`/`<` makes the whole multistatus document malformed
/// and CalDAV clients silently fail to parse it.
pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_well_formed(xml: &str) {
        let mut reader = quick_xml::Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(XmlEvent::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("XML is not well-formed: {e}\n---\n{xml}"),
            }
        }
    }

    #[test]
    fn report_response_escapes_ampersand_in_href() {
        // Regression: a calendar named "Alice & Bob" produced an href like
        // `/caldav/alice/Alice & Bob/event.ics`. A bare `&` in XML content
        // is a hard parse error, so the whole multistatus doc failed to
        // parse client-side and no events showed up on the device.
        let objects = vec![CalendarObject {
            href: "/caldav/alice/Alice & Bob/event.ics".into(),
            etag: "\"1\"".into(),
            calendar_data: Some("BEGIN:VCALENDAR\nEND:VCALENDAR".into()),
        }];

        let xml = generate_report_response(objects).unwrap();
        assert_well_formed(&xml);
        assert!(xml.contains("Alice &amp; Bob"));
        assert!(!xml.contains("Alice & Bob"));
    }

    #[test]
    fn parse_calendar_multiget_decodes_entities_in_hrefs() {
        // quick-xml ≥ 0.37 emits `&amp;`/`&#NN;` as separate `GeneralRef`
        // events rather than leaving them escaped inside `Text`, so the
        // multiget parser must reassemble them or a calendar whose name
        // contains `&` stops resolving its own hrefs.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop><D:getetag/></D:prop>
  <D:href>/caldav/alice/Alice &amp; Bob/event.ics</D:href>
  <D:href>/caldav/alice/a&#45;b/event.ics</D:href>
</C:calendar-multiget>"#;
        let m = parse_calendar_multiget(xml).unwrap();
        assert_eq!(
            m.hrefs,
            vec![
                "/caldav/alice/Alice & Bob/event.ics".to_string(),
                "/caldav/alice/a-b/event.ics".to_string(),
            ]
        );
    }
}
