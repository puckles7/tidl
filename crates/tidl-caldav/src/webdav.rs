//! WebDAV protocol implementation (RFC 4918)
//!
//! Handles PROPFIND request parsing and multi-status response generation.

use quick_xml::XmlVersion;
use quick_xml::{events::Event as XmlEvent, Reader};
use serde::{Deserialize, Serialize};

use crate::error::CalDavError;

type Result<T> = std::result::Result<T, CalDavError>;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropFind {
    pub kind: PropFindType,
    pub depth: Depth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropFindType {
    /// Return all properties and their values.
    AllProp,
    /// Return only property names.
    PropName,
    /// Return a specific set of named properties.
    Prop(Vec<String>),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Depth {
    Zero,
    One,
    Infinity,
}

impl Depth {
    pub fn from_header(header: Option<&str>) -> Self {
        match header {
            Some("0") => Depth::Zero,
            Some("1") => Depth::One,
            Some("infinity") | Some("Infinity") => Depth::Infinity,
            _ => Depth::Zero,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PropStat {
    pub properties: Vec<Property>,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub namespace: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MultiStatusResponse {
    pub responses: Vec<Response>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub href: String,
    pub propstats: Vec<PropStat>,
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parse a `PROPFIND` request body.
///
/// The `Depth` header value is passed separately because it lives in the HTTP
/// headers, not the body.  If the body is empty or blank the function falls
/// back to `allprop` as RFC 4918 §9.1 recommends.
///
/// Returns an error if the body is non-empty but not well-formed XML, or if it
/// does not contain a `propfind` root element (RFC 4918 §9.1).
pub fn parse_propfind(xml: &str, depth_header: Option<&str>) -> Result<PropFind> {
    let depth = Depth::from_header(depth_header);

    let trimmed = xml.trim();
    if trimmed.is_empty() {
        return Ok(PropFind {
            kind: PropFindType::AllProp,
            depth,
        });
    }

    let mut reader = Reader::from_str(trimmed);
    reader.config_mut().trim_text(true);

    let mut kind = PropFindType::AllProp;
    let mut prop_names: Vec<String> = Vec::new();
    let mut inside_prop = false;
    let mut found_propfind = false;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());

                // Validate that empty-string namespace URIs are rejected (RFC 4918).
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref();
                    if key.starts_with("xmlns") {
                        let val = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .unwrap_or_default();
                        if val.is_empty() && key != "xmlns" {
                            return Err(CalDavError::InvalidRequest(
                                "Invalid empty namespace URI in PROPFIND".to_string(),
                            ));
                        }
                    }
                }

                match local.as_str() {
                    "propfind" => found_propfind = true,
                    "allprop" => kind = PropFindType::AllProp,
                    "propname" => kind = PropFindType::PropName,
                    "prop" => {
                        inside_prop = true;
                        kind = PropFindType::Prop(vec![]);
                    }
                    // Preserve the client's original case here — unlike the
                    // keywords above (our own protocol vocabulary, safe to
                    // match case-insensitively), this name is meaningful
                    // data: several DAV:-namespace properties are not
                    // all-lowercase (e.g. `principal-URL`, RFC 3744 §4.2),
                    // and callers must echo back exactly what was asked for
                    // — lowercasing it here silently answers a different
                    // (wrongly-cased) property name than the one requested,
                    // which a case-sensitive XML client never recognises as
                    // a response to its own question (found via a real
                    // iOS CardDAV client, 2026-09-17). Callers that need
                    // case-insensitive lookup must lowercase at comparison
                    // time instead, not rely on this already being lowercase.
                    _ if inside_prop => {
                        prop_names.push(local_name_preserving_case(e.name().as_ref()))
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::End(ref e)) => {
                if local_name(e.name().as_ref()) == "prop" {
                    inside_prop = false;
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(source) => {
                return Err(CalDavError::Xml {
                    context: "PROPFIND",
                    source,
                })
            }
            _ => {}
        }
    }

    if !found_propfind {
        return Err(CalDavError::InvalidRequest(
            "PROPFIND body does not contain a <propfind> element".to_string(),
        ));
    }

    if let PropFindType::Prop(_) = kind {
        kind = PropFindType::Prop(prop_names);
    }

    Ok(PropFind { kind, depth })
}

// ── PROPPATCH parsing ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum PropPatchOp {
    Set {
        namespace: String,
        name: String,
        value: String,
    },
    Remove {
        namespace: String,
        name: String,
    },
}

/// Parse a `PROPPATCH` (`propertyupdate`) request body.
///
/// Returns an ordered list of set/remove operations (RFC 4918 §9.2).
pub fn parse_proppatch(xml: &str) -> Result<Vec<PropPatchOp>> {
    let trimmed = xml.trim();
    if trimmed.is_empty() {
        return Err(CalDavError::InvalidRequest(
            "PROPPATCH body is empty".to_string(),
        ));
    }

    let mut reader = Reader::from_str(trimmed);

    let mut ops: Vec<PropPatchOp> = Vec::new();
    let mut in_set = false;
    let mut in_remove = false;
    let mut in_prop = false;
    // (namespace_uri, local_name) of the current prop element
    let mut cur_ns = String::new();
    let mut cur_name = String::new();
    let mut cur_value = String::new();
    let mut depth_in_prop: u32 = 0;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "set" => {
                        in_set = true;
                        in_remove = false;
                    }
                    "remove" => {
                        in_remove = true;
                        in_set = false;
                    }
                    "prop" if in_set || in_remove => {
                        in_prop = true;
                    }
                    _ if in_prop && depth_in_prop == 0 => {
                        // This is a property element — capture its namespace and name.
                        cur_name = local.clone();
                        cur_ns = namespace_uri(e.name().as_ref(), e);
                        cur_value.clear();
                        depth_in_prop = 1;
                    }
                    _ if in_prop && depth_in_prop > 0 => {
                        // Nested element inside property value — treat as raw XML text.
                        let raw = e.name().into_inner();
                        cur_value.push('<');
                        cur_value.push_str(raw);
                        cur_value.push('>');
                        depth_in_prop += 1;
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                if in_prop && depth_in_prop == 0 {
                    // Self-closing property element with no value.
                    let name = local.clone();
                    let ns = namespace_uri(e.name().as_ref(), e);
                    if in_set {
                        ops.push(PropPatchOp::Set {
                            namespace: ns,
                            name,
                            value: String::new(),
                        });
                    } else if in_remove {
                        ops.push(PropPatchOp::Remove {
                            namespace: ns,
                            name,
                        });
                    }
                }
            }
            Ok(XmlEvent::Text(ref e)) => {
                if in_prop && depth_in_prop > 0 {
                    cur_value.push_str(&e.xml10_content());
                }
            }
            Ok(XmlEvent::GeneralRef(ref e)) => {
                if in_prop && depth_in_prop > 0 {
                    cur_value.push_str(&crate::caldav::entity_ref_text(e));
                }
            }
            Ok(XmlEvent::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "set" => {
                        in_set = false;
                    }
                    "remove" => {
                        in_remove = false;
                    }
                    "prop" if depth_in_prop == 0 => {
                        in_prop = false;
                    }
                    _ if in_prop && depth_in_prop == 1 => {
                        // Closing tag of a property element. The value is
                        // trimmed here (rather than via `trim_text`, which
                        // would also eat the spaces around a `&amp;` split
                        // into its own event) to match the old per-text-event
                        // trimming behavior.
                        if in_set {
                            ops.push(PropPatchOp::Set {
                                namespace: cur_ns.clone(),
                                name: cur_name.clone(),
                                value: cur_value.trim().to_string(),
                            });
                        } else if in_remove {
                            ops.push(PropPatchOp::Remove {
                                namespace: cur_ns.clone(),
                                name: cur_name.clone(),
                            });
                        }
                        depth_in_prop = 0;
                        cur_ns.clear();
                        cur_name.clear();
                        cur_value.clear();
                    }
                    _ if in_prop && depth_in_prop > 1 => {
                        let raw = e.name().into_inner();
                        cur_value.push_str("</");
                        cur_value.push_str(raw);
                        cur_value.push('>');
                        depth_in_prop -= 1;
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(source) => {
                return Err(CalDavError::Xml {
                    context: "PROPPATCH",
                    source,
                })
            }
            _ => {}
        }
    }

    Ok(ops)
}

/// Extract the namespace URI from a qualified element name.
/// For `bar:foo`, look up `xmlns:bar` in the element's attributes.
/// Falls back to the default namespace `xmlns` if no prefix.
fn namespace_uri(qname: &str, e: &quick_xml::events::BytesStart) -> String {
    let prefix = if qname.contains(':') {
        qname.split(':').next().unwrap_or("")
    } else {
        ""
    };

    let attr_key = if prefix.is_empty() {
        "xmlns".to_string()
    } else {
        format!("xmlns:{}", prefix)
    };

    for attr in e.attributes().flatten() {
        let key = attr.key.as_ref();
        if key == attr_key {
            return attr
                .normalized_value(XmlVersion::Implicit1_0)
                .unwrap_or_default()
                .into_owned();
        }
    }
    String::new()
}

/// Generate a `207 Multi-Status` PROPPATCH response body.
///
/// Each operation is acknowledged with 200 OK (we store everything).
/// `props` is a list of `(namespace, name)` pairs that were processed.
pub fn generate_proppatch_response(href: &str, props: &[(String, String)]) -> Result<String> {
    let href = crate::caldav::escape_xml(href);
    let prop_xml = props
        .iter()
        .map(|(ns, name)| {
            if ns.is_empty() {
                format!("          <{name}/>")
            } else {
                format!("          <x:{name} xmlns:x=\"{ns}\"/>")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>{href}</D:href>
    <D:propstat>
      <D:prop>
{prop_xml}
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#
    ))
}

// ── Response generation ───────────────────────────────────────────────────────

/// Generate a `207 Multi-Status` XML response body.
pub fn generate_propfind_response(responses: Vec<Response>) -> Result<String> {
    let response_xml = responses
        .iter()
        .map(format_response)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:CS="http://calendarserver.org/ns/" xmlns:ICAL="http://apple.com/ns/ical/">
{}
</D:multistatus>"#,
        response_xml
    ))
}

fn format_response(response: &Response) -> String {
    let propstats = response
        .propstats
        .iter()
        .map(format_propstat)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "  <D:response>\n    <D:href>{}</D:href>\n{}\n  </D:response>",
        crate::caldav::escape_xml(&response.href),
        propstats
    )
}

fn format_propstat(propstat: &PropStat) -> String {
    let props = propstat
        .properties
        .iter()
        .map(format_property)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "    <D:propstat>\n      <D:prop>\n{}\n      </D:prop>\n      <D:status>{}</D:status>\n    </D:propstat>",
        props, propstat.status
    )
}

fn format_property(prop: &Property) -> String {
    match &prop.value {
        Some(value) => format!(
            "        <{}:{}>{}</{0}:{1}>",
            prop.namespace, prop.name, value
        ),
        None => format!("        <{}:{} />", prop.namespace, prop.name),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Strip the namespace prefix from a qualified XML name (e.g. `D:prop` → `prop`).
fn local_name(name: &str) -> String {
    name.split(':').next_back().unwrap_or(name).to_lowercase()
}

/// Like [`local_name`] but preserves the original case — for extracting a
/// requested property name (meaningful data to echo back verbatim), as
/// opposed to matching one of our own protocol keywords case-insensitively.
fn local_name_preserving_case(name: &str) -> String {
    name.split(':').next_back().unwrap_or(name).to_string()
}

// ── Well-known property name constants ────────────────────────────────────────

/// Common WebDAV/CalDAV property names (unqualified local names).
pub mod properties {
    pub const DISPLAYNAME: &str = "displayname";
    pub const RESOURCETYPE: &str = "resourcetype";
    pub const GETETAG: &str = "getetag";
    pub const GETCONTENTTYPE: &str = "getcontenttype";
    pub const GETCONTENTLENGTH: &str = "getcontentlength";
    pub const CREATIONDATE: &str = "creationdate";
    pub const GETLASTMODIFIED: &str = "getlastmodified";
    pub const CURRENT_USER_PRINCIPAL: &str = "current-user-principal";
    pub const PRINCIPAL_URL: &str = "principal-URL";
    pub const PRINCIPAL_COLLECTION_SET: &str = "principal-collection-set";
    pub const RESOURCE_ID: &str = "resource-id";
    pub const SUPPORTED_REPORT_SET: &str = "supported-report-set";
    pub const CALENDAR_HOME_SET: &str = "calendar-home-set";
    pub const CALENDAR_USER_ADDRESS_SET: &str = "calendar-user-address-set";
    pub const SCHEDULE_INBOX_URL: &str = "schedule-inbox-URL";
    pub const SCHEDULE_OUTBOX_URL: &str = "schedule-outbox-URL";
    pub const CALENDAR_DESCRIPTION: &str = "calendar-description";
    pub const CALENDAR_TIMEZONE: &str = "calendar-timezone";
    pub const SUPPORTED_CALENDAR_COMPONENT_SET: &str = "supported-calendar-component-set";
    /// Apple CalendarServer extension for collection change detection (`CS:` namespace).
    pub const GETCTAG: &str = "getctag";
    pub const SYNC_TOKEN: &str = "sync-token";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_well_formed(xml: &str) {
        let mut reader = Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(XmlEvent::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("XML is not well-formed: {e}\n---\n{xml}"),
            }
        }
    }

    #[test]
    fn propfind_response_escapes_ampersand_in_href_and_displayname() {
        // Regression: a calendar named "Alice & Bob" produced a home-collection
        // PROPFIND response with a bare `&` in both <D:href> and <D:displayname>,
        // which is not well-formed XML and made clients (iOS) fail to parse the
        // whole multistatus document — so no calendars/events were discovered.
        let responses = vec![Response {
            href: "/caldav/alice/Alice & Bob/".into(),
            propstats: vec![PropStat {
                properties: vec![Property {
                    namespace: "D".into(),
                    name: properties::DISPLAYNAME.into(),
                    value: Some(crate::caldav::escape_xml("Alice & Bob")),
                }],
                status: "HTTP/1.1 200 OK".into(),
            }],
        }];

        let xml = generate_propfind_response(responses).unwrap();
        assert_well_formed(&xml);
        assert!(xml.contains("Alice &amp; Bob"));
        assert!(!xml.contains("Alice & Bob"));
    }

    #[test]
    fn parse_propfind_preserves_requested_property_name_case() {
        // Regression (found via a real iOS CardDAV client, 2026-09-17):
        // requested property *names* are meaningful data to be echoed back,
        // not protocol keywords — several DAV: properties are not
        // all-lowercase (e.g. `principal-URL`, RFC 3744 §4.2). Lowercasing
        // them at parse time made the server answer a differently-cased,
        // effectively different property than the one actually requested,
        // which a case-sensitive client never recognises as a response to
        // its own question.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<A:propfind xmlns:A="DAV:">
  <A:prop>
    <A:current-user-principal/>
    <A:principal-URL/>
    <A:resourcetype/>
  </A:prop>
</A:propfind>"#;
        let result = parse_propfind(xml, None).unwrap();
        let PropFindType::Prop(names) = result.kind else {
            panic!("expected Prop variant");
        };
        assert_eq!(
            names,
            vec!["current-user-principal", "principal-URL", "resourcetype"]
        );
    }

    #[test]
    fn parse_propfind_still_matches_structural_keywords_case_insensitively() {
        // The keywords <propfind>/<prop>/<allprop>/<propname> are our own
        // protocol vocabulary (not client-supplied data), so those stay
        // case-insensitive even though requested property names no longer are.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<A:PropFind xmlns:A="DAV:">
  <A:Prop>
    <A:Getetag/>
  </A:Prop>
</A:PropFind>"#;
        let result = parse_propfind(xml, None).unwrap();
        let PropFindType::Prop(names) = result.kind else {
            panic!("expected Prop variant");
        };
        assert_eq!(names, vec!["Getetag"], "keyword matching must stay case-insensitive even though the property name itself now preserves case");
    }

    #[test]
    fn proppatch_response_escapes_ampersand_in_href() {
        let xml = generate_proppatch_response(
            "/caldav/alice/Alice & Bob/",
            &[("".into(), "displayname".into())],
        )
        .unwrap();
        assert_well_formed(&xml);
        assert!(xml.contains("Alice &amp; Bob"));
    }

    #[test]
    fn parse_proppatch_decodes_entities_in_property_value() {
        // quick-xml ≥ 0.37 emits `&amp;`/`&#NN;` as separate `GeneralRef`
        // events, so a PROPPATCH display-name like "Alice & Bob" arrives
        // split across several events and must be reassembled unescaped.
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:set>
    <D:prop>
      <D:displayname>Alice &amp; Bob</D:displayname>
    </D:prop>
  </D:set>
</D:propertyupdate>"#;
        let ops = parse_proppatch(xml).unwrap();
        assert_eq!(ops.len(), 1);
        let PropPatchOp::Set { name, value, .. } = &ops[0] else {
            panic!("expected a Set op");
        };
        assert_eq!(name, "displayname");
        assert_eq!(value, "Alice & Bob");
    }
}
