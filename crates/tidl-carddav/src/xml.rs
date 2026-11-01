//! CardDAV multistatus (PROPFIND) response generation.
//!
//! Reuses `tidl_caldav::webdav`'s `PropFind`/`Depth`/`Response`/`PropStat`/
//! `Property`/`PropPatchOp` types and its `parse_propfind`/`parse_proppatch`
//! parsers — those are generic WebDAV (RFC 4918), not calendar-specific.
//! Only the multistatus *envelope* differs (CalDAV declares `C:`/`ICAL:`
//! namespaces CardDAV has no use for; CardDAV needs `CARD:
//! urn:ietf:params:xml:ns:carddav` instead), so this module writes its own
//! generator rather than reusing `tidl_caldav::webdav::generate_propfind_response`.
//! Both envelopes declare `CS: http://calendarserver.org/ns/` — `getctag`
//! (Apple's pre-RFC-6578 collection-change marker) is used on both address
//! books and calendars, and every `Property` emitted with `namespace: "CS"`
//! must have that prefix declared here or the document isn't well-formed
//! XML (found the hard way, 2026-09-15: `build_addressbook_propstats`'s
//! `getctag` property silently broke every CardDAV PROPFIND response until
//! this was added — real clients like Thunderbird just abandon the whole
//! sync on unparseable XML, with no visible error).

use tidl_caldav::webdav::{PropStat, Property, Response};

use crate::error::CardDavError;

type Result<T> = std::result::Result<T, CardDavError>;

/// Generate a `207 Multi-Status` PROPFIND response body with the CardDAV
/// namespace declared.
pub fn generate_propfind_response(responses: Vec<Response>) -> Result<String> {
    let response_xml = responses
        .iter()
        .map(format_response)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CARD="urn:ietf:params:xml:ns:carddav" xmlns:CS="http://calendarserver.org/ns/">
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
        tidl_caldav::escape_xml(&response.href),
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

/// Well-known CardDAV/WebDAV property names (unqualified local names) not
/// already covered by `tidl_caldav::webdav::properties`.
pub mod properties {
    pub const ADDRESSBOOK_HOME_SET: &str = "addressbook-home-set";
    pub const ADDRESSBOOK_DESCRIPTION: &str = "addressbook-description";
    pub const SUPPORTED_ADDRESS_DATA: &str = "supported-address-data";
    pub const MAX_RESOURCE_SIZE: &str = "max-resource-size";
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::{events::Event as XmlEvent, Reader};

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

    /// Basic well-formedness (`assert_well_formed`) only checks tag nesting —
    /// it does *not* catch a namespace prefix (e.g. `CS:`) used without a
    /// matching `xmlns:` declaration in scope, which is exactly the bug that
    /// broke every real CardDAV client's PROPFIND parsing until fixed
    /// (2026-09-15, see this module's doc comment). Uses `NsReader` to
    /// resolve every prefixed name and fail on an unbound one.
    fn assert_namespaces_are_declared(xml: &str) {
        use quick_xml::name::ResolveResult;
        use quick_xml::reader::NsReader;

        let mut reader = NsReader::from_str(xml);
        loop {
            match reader.read_resolved_event() {
                Ok((_, XmlEvent::Eof)) => break,
                Ok((ResolveResult::Unknown(prefix), event)) => {
                    panic!(
                        "XML uses undeclared namespace prefix {:?} on {:?}\n---\n{xml}",
                        prefix, event
                    );
                }
                Ok(_) => {}
                Err(e) => panic!("XML is not well-formed: {e}\n---\n{xml}"),
            }
        }
    }

    #[test]
    fn propfind_response_declares_carddav_namespace_and_escapes_href() {
        let responses = vec![Response {
            href: "/carddav/alice/contacts/Bed & Breakfast/".into(),
            propstats: vec![PropStat {
                properties: vec![Property {
                    namespace: "D".into(),
                    name: "displayname".into(),
                    value: Some(tidl_caldav::escape_xml("Bed & Breakfast")),
                }],
                status: "HTTP/1.1 200 OK".into(),
            }],
        }];

        let xml = generate_propfind_response(responses).unwrap();
        assert_well_formed(&xml);
        assert!(xml.contains(r#"xmlns:CARD="urn:ietf:params:xml:ns:carddav""#));
        assert!(xml.contains("Bed &amp; Breakfast"));
        assert!(!xml.contains("Bed & Breakfast"));
    }

    #[test]
    fn propfind_response_declares_every_namespace_it_uses_properties() {
        // Regression: a `getctag` property (namespace "CS", used by
        // carddav_handlers.rs's build_addressbook_propstats) was emitted
        // without `xmlns:CS` ever being declared in the envelope — invalid
        // XML that real clients (Thunderbird) silently abandon parsing,
        // with the sync just going quiet rather than erroring visibly.
        let responses = vec![Response {
            href: "/carddav/alice/contacts/".into(),
            propstats: vec![PropStat {
                properties: vec![
                    Property {
                        namespace: "D".into(),
                        name: "displayname".into(),
                        value: Some("Contacts".into()),
                    },
                    Property {
                        namespace: "CARD".into(),
                        name: "addressbook".into(),
                        value: Some("".into()),
                    },
                    Property {
                        namespace: "CS".into(),
                        name: "getctag".into(),
                        value: Some("\"1\"".into()),
                    },
                ],
                status: "HTTP/1.1 200 OK".into(),
            }],
        }];

        let xml = generate_propfind_response(responses).unwrap();
        assert_well_formed(&xml);
        assert_namespaces_are_declared(&xml);
    }
}
