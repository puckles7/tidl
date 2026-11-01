//! CardDAV protocol implementation (RFC 6352)
//!
//! Handles REPORT request parsing (`addressbook-query` and
//! `addressbook-multiget`) and the corresponding multi-status response
//! generation. Mirrors `tidl_caldav::caldav`'s `calendar-query`/
//! `calendar-multiget` handling — including that `parse_addressbook_query`'s
//! `filter` is parsed but not applied when listing results (the sibling
//! `caldav_report` handler does the same for `calendar-query`'s time-range
//! filter: real sync clients drive off `sync-collection`/`multiget`, and
//! `addressbook-query`'s `prop-filter`/`text-match` is only ever used for
//! ad-hoc search UI tidl doesn't need to serve narrowed results for).

use quick_xml::events::BytesRef;
use quick_xml::{events::Event as XmlEvent, Reader};

use crate::error::CardDavError;

type Result<T> = std::result::Result<T, CardDavError>;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AddressbookQuery {
    pub properties: Vec<String>,
    pub include_address_data: bool,
}

#[derive(Debug, Clone)]
pub struct AddressbookMultiget {
    pub hrefs: Vec<String>,
    pub properties: Vec<String>,
    pub include_address_data: bool,
}

/// A single vCard resource in a REPORT response.
#[derive(Debug, Clone)]
pub struct AddressObject {
    pub href: String,
    pub etag: String,
    pub address_data: Option<String>,
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Parse a CardDAV `addressbook-query` REPORT body (RFC 6352 §8.6).
pub fn parse_addressbook_query(xml: &str) -> Result<AddressbookQuery> {
    let mut reader = Reader::from_str(xml.trim());
    reader.config_mut().trim_text(true);

    let mut properties: Vec<String> = Vec::new();
    let mut include_address_data = false;
    let mut inside_prop = false;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(ref e)) | Ok(XmlEvent::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "prop" => inside_prop = true,
                    "address-data" => {
                        include_address_data = true;
                        if inside_prop {
                            properties.push("address-data".into());
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
            Ok(XmlEvent::End(ref e)) => {
                if local_name(e.name().as_ref()) == "prop" {
                    inside_prop = false;
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(source) => {
                return Err(CardDavError::Xml {
                    context: "addressbook-query",
                    source,
                })
            }
            _ => {}
        }
    }

    if properties.is_empty() {
        properties = vec!["getetag".into(), "address-data".into()];
        include_address_data = true;
    }

    Ok(AddressbookQuery {
        properties,
        include_address_data,
    })
}

/// Parse a CardDAV `addressbook-multiget` REPORT body (RFC 6352 §8.7).
pub fn parse_addressbook_multiget(xml: &str) -> Result<AddressbookMultiget> {
    let mut reader = Reader::from_str(xml.trim());

    let mut hrefs: Vec<String> = Vec::new();
    let mut properties: Vec<String> = Vec::new();
    let mut include_address_data = false;
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
                    "address-data" => {
                        include_address_data = true;
                        if inside_prop {
                            properties.push("address-data".into());
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
                if local == "address-data" && inside_prop {
                    include_address_data = true;
                    properties.push("address-data".into());
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
                return Err(CardDavError::Xml {
                    context: "addressbook-multiget",
                    source,
                })
            }
            _ => {}
        }
    }

    if properties.is_empty() {
        properties = vec!["getetag".into(), "address-data".into()];
        include_address_data = true;
    }

    Ok(AddressbookMultiget {
        hrefs,
        properties,
        include_address_data,
    })
}

// ── Response generation ───────────────────────────────────────────────────────

/// Generate a `207 Multi-Status` XML response for a REPORT.
pub fn generate_report_response(results: Vec<AddressObject>) -> Result<String> {
    let responses = results
        .iter()
        .map(format_address_object)
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CARD="urn:ietf:params:xml:ns:carddav">
{}
</D:multistatus>"#,
        responses
    ))
}

fn format_address_object(obj: &AddressObject) -> String {
    let data_xml = obj
        .address_data
        .as_ref()
        .map(|d| {
            format!(
                "        <CARD:address-data>{}</CARD:address-data>",
                tidl_caldav::escape_xml(d)
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
        tidl_caldav::escape_xml(&obj.href),
        obj.etag,
        data_xml
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn local_name(name: &str) -> String {
    name.split(':').next_back().unwrap_or(name).to_lowercase()
}

/// Decode a general entity/character reference emitted as its own
/// `Event::GeneralRef` (quick-xml splits `&amp;` and friends out of `Text`
/// events), so hrefs containing e.g. `&` still resolve to their characters.
fn entity_ref_text(reference: &BytesRef) -> String {
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
    fn parse_addressbook_query_defaults_to_etag_and_address_data() {
        let xml = r#"<CARD:addressbook-query xmlns:D="DAV:" xmlns:CARD="urn:ietf:params:xml:ns:carddav">
            <D:prop><D:getetag/><CARD:address-data/></D:prop>
        </CARD:addressbook-query>"#;
        let q = parse_addressbook_query(xml).unwrap();
        assert!(q.include_address_data);
        assert!(q.properties.contains(&"getetag".to_string()));
    }

    #[test]
    fn parse_addressbook_multiget_extracts_hrefs() {
        let xml = r#"<CARD:addressbook-multiget xmlns:D="DAV:" xmlns:CARD="urn:ietf:params:xml:ns:carddav">
            <D:prop><D:getetag/><CARD:address-data/></D:prop>
            <D:href>/carddav/alice/contacts/u1.vcf</D:href>
            <D:href>/carddav/alice/contacts/u2.vcf</D:href>
        </CARD:addressbook-multiget>"#;
        let m = parse_addressbook_multiget(xml).unwrap();
        assert_eq!(
            m.hrefs,
            vec![
                "/carddav/alice/contacts/u1.vcf".to_string(),
                "/carddav/alice/contacts/u2.vcf".to_string(),
            ]
        );
        assert!(m.include_address_data);
    }

    #[test]
    fn parse_addressbook_multiget_decodes_entities_in_hrefs() {
        // quick-xml ≥ 0.37 emits `&amp;` as a separate `GeneralRef` event
        // instead of leaving it escaped inside `Text`; the multiget parser
        // must reassemble it for an address book whose name contains `&`.
        let xml = r#"<CARD:addressbook-multiget xmlns:D="DAV:" xmlns:CARD="urn:ietf:params:xml:ns:carddav">
            <D:prop><D:getetag/></D:prop>
            <D:href>/carddav/alice/Bed &amp; Breakfast/u1.vcf</D:href>
        </CARD:addressbook-multiget>"#;
        let m = parse_addressbook_multiget(xml).unwrap();
        assert_eq!(
            m.hrefs,
            vec!["/carddav/alice/Bed & Breakfast/u1.vcf".to_string()]
        );
    }

    #[test]
    fn report_response_escapes_ampersand_in_href() {
        let objs = vec![AddressObject {
            href: "/carddav/alice/contacts/Bed & Breakfast.vcf".into(),
            etag: "\"abc\"".into(),
            address_data: Some("BEGIN:VCARD\r\nEND:VCARD\r\n".into()),
        }];
        let xml = generate_report_response(objs).unwrap();
        assert_well_formed(&xml);
        assert!(xml.contains("Bed &amp; Breakfast"));
        assert!(!xml.contains("Bed & Breakfast"));
    }
}
