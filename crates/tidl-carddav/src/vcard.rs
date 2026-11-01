//! vCard 3.0 parse/serialize (RFC 6350-compatible subset).
//!
//! Round-trips: UID, FN, N (all 5 components), NICKNAME, ORG, TITLE, BDAY,
//! ANNIVERSARY, ADR (repeatable — see [`VCardAddress`]), EMAIL, TEL, URL,
//! IMPP, CATEGORIES, NOTE. Anything else (VERSION, PRODID, REV, PHOTO,
//! X-SOCIALPROFILE, X-ABRELATEDNAMES, …) is parsed far enough to skip over
//! but otherwise ignored — this is a mapper for tidl's own record shape,
//! not a general-purpose vCard library.
//!
//! **`UID` is optional on parse** (RFC 2426 only mandates VERSION/N/FN;
//! Nimbus Mail's own vCard 3.0 export, confirmed 2026-09-17, omits it on
//! every contact) — nothing downstream reads the parsed value anyway, since
//! contact creation always mints its own storage uid and CardDAV PUT keys
//! off the URL, not the body. A blank `FN` falls back to `ORG` (also seen
//! live in the same export, for organization-only contacts) rather than
//! rejecting the vCard outright.
//!
//! **Apple's item-group + label convention**: real iOS vCards attach a
//! custom label to some properties via `itemN.PROPERTY:value` paired with
//! `itemN.X-ABLabel:_$!<Label>!$_` (confirmed live 2026-09-17 for `URL`:
//! `item2.URL;type=pref:https://…` + `item2.X-ABLabel:_$!<HomePage>!$_`).
//! `URL`/`IMPP` use this to pick a `home`/`work`/`other` kind when no
//! `TYPE=` param is present. `ANNIVERSARY` likely arrives the same way via
//! Apple's `X-ABDATE` (labeled `_$!<Anniversary>!$_`) rather than the plain
//! vCard 4.0 `ANNIVERSARY` property (vCard 3.0, which iOS sends, has no
//! such property at all) — that path is a reasoned bet from the confirmed
//! `URL` mechanism, **not verified against real device data**; retest
//! before relying on it.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VCardError {
    #[error("vCard is missing the BEGIN:VCARD/END:VCARD envelope")]
    MissingEnvelope,
    #[error("vCard is missing the required FN property")]
    MissingFn,
}

/// One `EMAIL`/`TEL`/`URL`/`IMPP` entry. `kind` is lowercase (`"home"`/
/// `"work"`/`"mobile"`/`"other"`) to match the `contact_emails`-shaped
/// tables' `type` CHECK constraint directly — no remapping needed at the
/// storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VCardMethod {
    pub value: String,
    pub kind: String,
    pub is_primary: bool,
}

/// One `ADR` entry. vCard allows repeating `ADR` (home + work addresses are
/// common); `kind` is `"home"`/`"work"`/`"other"` — no `"mobile"`, a postal
/// address doesn't have that concept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VCardAddress {
    pub kind: String,
    pub is_primary: bool,
    pub street: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// A `PHOTO` property's decoded payload. `content_type` is a canonical MIME
/// type (`"image/jpeg"`/`"image/png"`/`"image/gif"`) — vCard's own `TYPE=`
/// token (`JPEG`/`PNG`/`GIF`) is mapped to/from this at the parse/generate
/// boundary so nothing outside this module needs to know the wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VCardPhoto {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Max raw (decoded) `PHOTO` bytes accepted (#86). This is the CardDAV-PUT
/// entry point for photo bytes into tidl; the web-UI upload path enforces
/// the same cap independently (see `upload_contact_photo` in tidl-web) since
/// it never goes through this parser. An oversized `PHOTO` is dropped
/// silently rather than failing the whole vCard — same "skip what we don't
/// handle" behavior this parser already applies to every other unsupported
/// property, and appropriate here since this is untrusted external input.
pub const MAX_PHOTO_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VCardFields {
    pub uid: String,
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    pub nickname: Option<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    /// `yyyy-mm-dd`.
    pub birthday: Option<String>,
    /// `yyyy-mm-dd`.
    pub anniversary: Option<String>,
    pub notes: Option<String>,
    pub categories: Vec<String>,
    pub emails: Vec<VCardMethod>,
    pub phones: Vec<VCardMethod>,
    pub urls: Vec<VCardMethod>,
    pub impps: Vec<VCardMethod>,
    pub addresses: Vec<VCardAddress>,
    pub photo: Option<VCardPhoto>,
}

/// Unescape a vCard value (RFC 6350 §3.4): `\\` `\,` `\;` `\n`/`\N`.
fn unescape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Escape a value for output — the inverse of [`unescape_value`].
fn escape_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Split a structured value (e.g. `N`, `ADR`) on unescaped `;`.
fn split_components(value: &str) -> Vec<String> {
    split_on(value, ';')
        .into_iter()
        .map(|c| unescape_value(&c))
        .collect()
}

/// Split a multi-value property (e.g. `CATEGORIES`) on unescaped `,`,
/// trimming and dropping empty entries.
fn split_on_commas(value: &str) -> Vec<String> {
    split_on(value, ',')
        .into_iter()
        .map(|c| unescape_value(&c))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split on `sep`, treating a backslash-escaped `sep` as literal (shared by
/// [`split_components`] and [`split_on_commas`] — the escaping rule is the
/// same regardless of which character is the separator).
fn split_on(value: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            current.push(c);
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else if c == sep {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
}

/// Strip Apple's `_$!<Label>!$_` wrapper around its own known label names
/// (e.g. `_$!<HomePage>!$_`, `_$!<Anniversary>!$_`) — a custom user-typed
/// label has no wrapper and passes through unchanged.
fn strip_apple_label_wrapper(s: &str) -> String {
    let s = s.trim();
    match s.strip_prefix("_$!<").and_then(|r| r.strip_suffix(">!$_")) {
        Some(inner) => inner.to_string(),
        None => s.to_string(),
    }
}

struct ParsedLine {
    /// The `itemN` group prefix, if this line had one (`item2.URL` → `Some("item2")`).
    group: Option<String>,
    name: String,
    /// Raw `TYPE=` param values (comma-split, uppercased), if any.
    types: Vec<String>,
    pref: bool,
    value: String,
}

/// Parse one unfolded `[group.]NAME[;PARAM=VAL...]:VALUE` line.
fn parse_line(line: &str) -> Option<ParsedLine> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let first = parts.next()?.trim();
    let (group, name) = match first.find('.') {
        Some(dot) => (
            Some(first[..dot].to_string()),
            first[dot + 1..].to_ascii_uppercase(),
        ),
        None => (None, first.to_ascii_uppercase()),
    };

    let mut types = Vec::new();
    let mut pref = false;
    for param in parts {
        let mut kv = param.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim().to_ascii_uppercase();
        let val = kv.next().unwrap_or("").trim();
        if key == "TYPE" {
            for t in val.split(',') {
                let t = t.trim().trim_matches('"').to_ascii_uppercase();
                if t == "PREF" {
                    pref = true;
                } else if !t.is_empty() {
                    types.push(t);
                }
            }
        } else if key == "PREF" {
            pref = true;
        }
    }

    Some(ParsedLine {
        group,
        name,
        types,
        pref,
        value: value.to_string(),
    })
}

/// Map vCard `TYPE=` tokens to tidl's `home`/`work`/`mobile`/`other` kind.
/// First recognised token wins; unrecognised/absent → `"other"`.
fn kind_from_types(types: &[String]) -> String {
    for t in types {
        match t.as_str() {
            "HOME" => return "home".to_string(),
            "WORK" => return "work".to_string(),
            "CELL" => return "mobile".to_string(),
            _ => {}
        }
    }
    "other".to_string()
}

/// Like [`kind_from_types`] but for `ADR` — no `"mobile"`.
fn address_kind_from_types(types: &[String]) -> String {
    for t in types {
        match t.as_str() {
            "HOME" => return "home".to_string(),
            "WORK" => return "work".to_string(),
            _ => {}
        }
    }
    "other".to_string()
}

/// Best-effort `home`/`work` guess from an Apple item-group label (e.g.
/// `"HomePage"` → home) — `None` if the label doesn't suggest either.
fn kind_from_label(label: &str) -> Option<&'static str> {
    let l = label.to_ascii_lowercase();
    if l.contains("home") {
        Some("home")
    } else if l.contains("work") {
        Some("work")
    } else {
        None
    }
}

/// Resolve a `URL`/`IMPP` line's kind: prefer an explicit `TYPE=` param;
/// fall back to the paired item-group's `X-ABLabel`, if any; else `"other"`.
fn resolve_labeled_kind(
    parsed: &ParsedLine,
    group_labels: &std::collections::HashMap<String, String>,
) -> String {
    let from_type = kind_from_types(&parsed.types);
    if from_type != "other" {
        return from_type;
    }
    if let Some(label) = parsed.group.as_deref().and_then(|g| group_labels.get(g)) {
        if let Some(k) = kind_from_label(label) {
            return k.to_string();
        }
    }
    "other".to_string()
}

/// Inverse of [`kind_from_types`] — the `TYPE=` token for a stored kind.
fn type_from_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "home" => Some("HOME"),
        "work" => Some("WORK"),
        "mobile" => Some("CELL"),
        _ => None,
    }
}

fn address_type_from_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "home" => Some("HOME"),
        "work" => Some("WORK"),
        _ => None,
    }
}

/// vCard 3.0 `TYPE=` token (`JPEG`/`PNG`/`GIF`, no `image/` prefix) → our
/// canonical MIME type. Unrecognised tokens (and a bare `PHOTO;VALUE=uri:…`
/// reference, which has no such token at all) return `None` — we only
/// support inline base64 photos, not remote URI references.
fn mime_from_type_token(t: &str) -> Option<&'static str> {
    match t {
        "JPEG" | "JPG" => Some("image/jpeg"),
        "PNG" => Some("image/png"),
        "GIF" => Some("image/gif"),
        _ => None,
    }
}

/// Inverse of [`mime_from_type_token`].
fn type_token_from_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "PNG",
        "image/gif" => "GIF",
        _ => "JPEG",
    }
}

/// Parse a `PHOTO` line: vCard 3.0 `PHOTO;ENCODING=b;TYPE=JPEG:<base64>`
/// (what iOS/Thunderbird send for every other field this parser supports)
/// and, defensively, the vCard 4 `PHOTO:data:image/jpeg;base64,<data>` URI
/// form — costs nothing extra to accept. Anything else (a `VALUE=uri`
/// reference to a remote photo, an unrecognised `TYPE=`, malformed base64,
/// or a payload over [`MAX_PHOTO_BYTES`]) is dropped, same as any other
/// unsupported property here.
fn parse_photo(parsed: &ParsedLine) -> Option<VCardPhoto> {
    let raw = parsed.value.trim();
    let (content_type, b64) = if let Some(rest) = raw.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let mime = meta.strip_suffix(";base64")?;
        (mime.to_string(), data)
    } else {
        let mime = parsed
            .types
            .iter()
            .find_map(|t| mime_from_type_token(t))
            .or(if parsed.types.is_empty() {
                Some("image/jpeg")
            } else {
                None
            })?;
        (mime.to_string(), raw)
    };

    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()?;
    if bytes.is_empty() || bytes.len() > MAX_PHOTO_BYTES {
        return None;
    }
    Some(VCardPhoto {
        bytes,
        content_type,
    })
}

/// Parse a vCard 3.0/4.0 text body into [`VCardFields`].
///
/// The input is unfolded first (RFC 6350 §3.2 continuation-line folding is
/// byte-identical to iCalendar's, see `tidl_caldav::unfold_ical`).
pub fn parse_vcard(text: &str) -> Result<VCardFields, VCardError> {
    let unfolded = tidl_caldav::unfold_ical(text);
    let lines: Vec<&str> = unfolded.lines().collect();

    let has_begin = lines
        .iter()
        .any(|l| l.trim().eq_ignore_ascii_case("BEGIN:VCARD"));
    let has_end = lines
        .iter()
        .any(|l| l.trim().eq_ignore_ascii_case("END:VCARD"));
    if !has_begin || !has_end {
        return Err(VCardError::MissingEnvelope);
    }

    // First pass: collect itemN group → decoded label, so URL/IMPP/X-ABDATE
    // lines can look theirs up regardless of whether the X-ABLabel line
    // appears before or after the property line it's paired with.
    let mut group_labels: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for raw in &lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Some(parsed) = parse_line(line) else {
            continue;
        };
        if parsed.name == "X-ABLABEL" {
            if let Some(group) = &parsed.group {
                group_labels.insert(
                    group.clone(),
                    strip_apple_label_wrapper(&unescape_value(&parsed.value)),
                );
            }
        }
    }

    let mut fields = VCardFields::default();

    for raw in lines {
        let line = raw.trim();
        if line.is_empty()
            || line.eq_ignore_ascii_case("BEGIN:VCARD")
            || line.eq_ignore_ascii_case("END:VCARD")
        {
            continue;
        }
        let Some(parsed) = parse_line(line) else {
            continue;
        };

        match parsed.name.as_str() {
            "UID" => fields.uid = unescape_value(&parsed.value),
            "FN" => fields.display_name = unescape_value(&parsed.value),
            "N" => {
                let comp = split_components(&parsed.value);
                fields.family_name = comp.first().filter(|s| !s.is_empty()).cloned();
                fields.given_name = comp.get(1).filter(|s| !s.is_empty()).cloned();
                fields.middle_name = comp.get(2).filter(|s| !s.is_empty()).cloned();
                fields.name_prefix = comp.get(3).filter(|s| !s.is_empty()).cloned();
                fields.name_suffix = comp.get(4).filter(|s| !s.is_empty()).cloned();
            }
            "NICKNAME" => {
                fields.nickname = Some(unescape_value(&parsed.value)).filter(|s| !s.is_empty())
            }
            "ORG" => fields.org = Some(unescape_value(&parsed.value)).filter(|s| !s.is_empty()),
            "TITLE" => fields.title = Some(unescape_value(&parsed.value)).filter(|s| !s.is_empty()),
            "BDAY" => {
                let v = unescape_value(&parsed.value);
                fields.birthday = normalize_date(&v);
            }
            "ANNIVERSARY" => {
                let v = unescape_value(&parsed.value);
                fields.anniversary = normalize_date(&v);
            }
            // Apple's labeled custom-date convention — see module doc
            // comment. Only maps to anniversary when the paired label says
            // so; other custom dates (e.g. "Other") are intentionally
            // dropped, same as any other unsupported field.
            "X-ABDATE" => {
                if let Some(label) = parsed.group.as_deref().and_then(|g| group_labels.get(g)) {
                    if label.to_ascii_lowercase().contains("anniversary") {
                        let v = unescape_value(&parsed.value);
                        fields.anniversary = normalize_date(&v);
                    }
                }
            }
            "CATEGORIES" => fields.categories = split_on_commas(&parsed.value),
            "ADR" => {
                let comp = split_components(&parsed.value);
                let street = comp.get(2).filter(|s| !s.is_empty()).cloned();
                let city = comp.get(3).filter(|s| !s.is_empty()).cloned();
                let region = comp.get(4).filter(|s| !s.is_empty()).cloned();
                let postal_code = comp.get(5).filter(|s| !s.is_empty()).cloned();
                let country = comp.get(6).filter(|s| !s.is_empty()).cloned();
                if street.is_some()
                    || city.is_some()
                    || region.is_some()
                    || postal_code.is_some()
                    || country.is_some()
                {
                    fields.addresses.push(VCardAddress {
                        kind: address_kind_from_types(&parsed.types),
                        is_primary: parsed.pref,
                        street,
                        city,
                        region,
                        postal_code,
                        country,
                    });
                }
            }
            "NOTE" => fields.notes = Some(unescape_value(&parsed.value)).filter(|s| !s.is_empty()),
            "EMAIL" => {
                let value = unescape_value(&parsed.value);
                if !value.is_empty() {
                    fields.emails.push(VCardMethod {
                        value,
                        kind: kind_from_types(&parsed.types),
                        is_primary: parsed.pref,
                    });
                }
            }
            "TEL" => {
                let value = unescape_value(&parsed.value);
                if !value.is_empty() {
                    fields.phones.push(VCardMethod {
                        value,
                        kind: kind_from_types(&parsed.types),
                        is_primary: parsed.pref,
                    });
                }
            }
            "URL" => {
                let value = unescape_value(&parsed.value);
                if !value.is_empty() {
                    fields.urls.push(VCardMethod {
                        value,
                        kind: resolve_labeled_kind(&parsed, &group_labels),
                        is_primary: parsed.pref,
                    });
                }
            }
            "IMPP" => {
                let value = unescape_value(&parsed.value);
                if !value.is_empty() {
                    fields.impps.push(VCardMethod {
                        value,
                        kind: resolve_labeled_kind(&parsed, &group_labels),
                        is_primary: parsed.pref,
                    });
                }
            }
            "PHOTO" => fields.photo = parse_photo(&parsed),
            _ => {}
        }
    }

    if fields.display_name.is_empty() {
        // A blank FN with a populated ORG is a real, common shape for
        // organization-only contacts (e.g. Nimbus Mail's export) — fall back
        // to the org name rather than rejecting the contact outright.
        match &fields.org {
            Some(org) if !org.is_empty() => fields.display_name = org.clone(),
            _ => return Err(VCardError::MissingFn),
        }
    }

    Ok(fields)
}

/// `BDAY`/`ANNIVERSARY` values in the wild show up as `YYYY-MM-DD` or bare
/// `YYYYMMDD`; normalise to the `yyyy-mm-dd` shape tidl's DB columns use.
/// Returns `None` for anything that doesn't look like a plain date (e.g. a
/// `VALUE=text` birthday) rather than guessing.
fn normalize_date(v: &str) -> Option<String> {
    let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 8 {
        Some(format!(
            "{}-{}-{}",
            &digits[0..4],
            &digits[4..6],
            &digits[6..8]
        ))
    } else if v.len() == 10 && v.as_bytes()[4] == b'-' && v.as_bytes()[7] == b'-' {
        Some(v.to_string())
    } else {
        None
    }
}

fn method_line(prop: &str, m: &VCardMethod) -> String {
    let mut params = String::new();
    if let Some(t) = type_from_kind(&m.kind) {
        params.push_str(";TYPE=");
        params.push_str(t);
    }
    if m.is_primary {
        params.push_str(if params.is_empty() {
            ";TYPE=PREF"
        } else {
            ",PREF"
        });
    }
    format!("{prop}{params}:{}", escape_value(&m.value))
}

fn address_line(a: &VCardAddress) -> String {
    let mut params = String::new();
    if let Some(t) = address_type_from_kind(&a.kind) {
        params.push_str(";TYPE=");
        params.push_str(t);
    }
    if a.is_primary {
        params.push_str(if params.is_empty() {
            ";TYPE=PREF"
        } else {
            ",PREF"
        });
    }
    format!(
        "ADR{params}:;;{};{};{};{};{}",
        escape_value(a.street.as_deref().unwrap_or("")),
        escape_value(a.city.as_deref().unwrap_or("")),
        escape_value(a.region.as_deref().unwrap_or("")),
        escape_value(a.postal_code.as_deref().unwrap_or("")),
        escape_value(a.country.as_deref().unwrap_or("")),
    )
}

/// Serialize [`VCardFields`] to a vCard 3.0 text body (CRLF line endings).
///
/// Emits `TYPE=` params directly for `URL`/`IMPP`/`ADR` rather than
/// reproducing Apple's item-group + `X-ABLabel` dance on output — plain
/// `TYPE=` is spec-legal and every real client reads it, so there's no need
/// to mimic iOS's own encoding just because that's what it happens to send.
pub fn generate_vcard(fields: &VCardFields) -> String {
    let mut lines = vec![
        "BEGIN:VCARD".to_string(),
        "VERSION:3.0".to_string(),
        format!("UID:{}", escape_value(&fields.uid)),
        format!("FN:{}", escape_value(&fields.display_name)),
        format!(
            "N:{};{};{};{};{}",
            escape_value(fields.family_name.as_deref().unwrap_or("")),
            escape_value(fields.given_name.as_deref().unwrap_or("")),
            escape_value(fields.middle_name.as_deref().unwrap_or("")),
            escape_value(fields.name_prefix.as_deref().unwrap_or("")),
            escape_value(fields.name_suffix.as_deref().unwrap_or("")),
        ),
    ];

    if let Some(nickname) = &fields.nickname {
        lines.push(format!("NICKNAME:{}", escape_value(nickname)));
    }
    if let Some(org) = &fields.org {
        lines.push(format!("ORG:{}", escape_value(org)));
    }
    if let Some(title) = &fields.title {
        lines.push(format!("TITLE:{}", escape_value(title)));
    }
    if let Some(bday) = &fields.birthday {
        lines.push(format!("BDAY:{bday}"));
    }
    if let Some(anniv) = &fields.anniversary {
        lines.push(format!("ANNIVERSARY:{anniv}"));
    }
    if let Some(photo) = &fields.photo {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&photo.bytes);
        let type_token = type_token_from_mime(&photo.content_type);
        lines.push(format!("PHOTO;ENCODING=b;TYPE={type_token}:{encoded}"));
    }
    for a in &fields.addresses {
        lines.push(address_line(a));
    }
    for e in &fields.emails {
        lines.push(method_line("EMAIL", e));
    }
    for p in &fields.phones {
        lines.push(method_line("TEL", p));
    }
    for u in &fields.urls {
        lines.push(method_line("URL", u));
    }
    for i in &fields.impps {
        lines.push(method_line("IMPP", i));
    }
    if !fields.categories.is_empty() {
        let joined = fields
            .categories
            .iter()
            .map(|c| escape_value(c))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!("CATEGORIES:{joined}"));
    }
    if let Some(notes) = &fields.notes {
        lines.push(format!("NOTE:{}", escape_value(notes)));
    }
    lines.push("END:VCARD".to_string());

    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VCardFields {
        VCardFields {
            uid: "abc-123".into(),
            display_name: "Ada Lovelace".into(),
            given_name: Some("Ada".into()),
            family_name: Some("Lovelace".into()),
            middle_name: Some("Augusta".into()),
            name_prefix: Some("Countess".into()),
            name_suffix: Some("I".into()),
            nickname: Some("Lady A".into()),
            org: Some("Analytical Engines, Inc".into()),
            title: Some("Mathematician".into()),
            birthday: Some("1815-12-10".into()),
            anniversary: Some("1835-07-08".into()),
            notes: Some("Wrote the first algorithm.\nSee note 42.".into()),
            categories: vec!["Colleague".into(), "VIP".into()],
            emails: vec![
                VCardMethod {
                    value: "ada@example.com".into(),
                    kind: "work".into(),
                    is_primary: true,
                },
                VCardMethod {
                    value: "ada@home.example".into(),
                    kind: "home".into(),
                    is_primary: false,
                },
            ],
            phones: vec![VCardMethod {
                value: "+44 20 1234".into(),
                kind: "mobile".into(),
                is_primary: false,
            }],
            urls: vec![VCardMethod {
                value: "https://ada.example".into(),
                kind: "home".into(),
                is_primary: true,
            }],
            impps: vec![VCardMethod {
                value: "xmpp:ada@example.com".into(),
                kind: "other".into(),
                is_primary: false,
            }],
            addresses: vec![
                VCardAddress {
                    kind: "home".into(),
                    is_primary: true,
                    street: Some("1 Engine Row".into()),
                    city: Some("London".into()),
                    region: None,
                    postal_code: Some("EC1".into()),
                    country: Some("UK".into()),
                },
                VCardAddress {
                    kind: "work".into(),
                    is_primary: false,
                    street: Some("2 Difference St".into()),
                    city: Some("London".into()),
                    region: None,
                    postal_code: Some("EC2".into()),
                    country: Some("UK".into()),
                },
            ],
            photo: Some(VCardPhoto {
                bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
                content_type: "image/jpeg".into(),
            }),
        }
    }

    #[test]
    fn round_trips_through_generate_and_parse() {
        let original = sample();
        let text = generate_vcard(&original);
        let parsed = parse_vcard(&text).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn generated_vcard_has_required_envelope_and_version() {
        let text = generate_vcard(&sample());
        assert!(text.starts_with("BEGIN:VCARD\r\n"));
        assert!(text.contains("VERSION:3.0\r\n"));
        assert!(text.trim_end().ends_with("END:VCARD"));
    }

    #[test]
    fn parse_rejects_missing_envelope() {
        assert_eq!(
            parse_vcard("FN:No Envelope\r\n"),
            Err(VCardError::MissingEnvelope)
        );
    }

    #[test]
    fn parse_accepts_missing_uid() {
        // Some real-world vCard 3.0 exports omit UID entirely. vCard 3.0
        // (RFC 2426) only requires VERSION/N/FN, so this must parse, not
        // fail — a strict UID requirement here silently rejected every
        // contact in an import (0 imported) even though nothing downstream
        // reads the parsed UID (the importer always mints its own storage
        // uid; CardDAV PUT uses the URL's uid, not the body's).
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:No Uid\r\nN:Uid;No;;;\r\nEND:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(parsed.display_name, "No Uid");
        assert_eq!(parsed.uid, "");
    }

    #[test]
    fn parse_rejects_missing_fn() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:no-fn\r\nEND:VCARD\r\n";
        assert_eq!(parse_vcard(text), Err(VCardError::MissingFn));
    }

    #[test]
    fn parse_falls_back_to_org_when_fn_is_blank() {
        // Some real-world vCard 3.0 exports contain organization-only
        // entries with `FN:` present but empty and `N:;;;;` all-empty —
        // the actual name lives in `ORG:` only. Rejecting these outright
        // would silently drop every business-only contact from an import.
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:\r\nN:;;;;\r\n\
                     TEL:+15550100\r\nORG:Acme Motors\r\nEND:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(parsed.display_name, "Acme Motors");
        assert_eq!(parsed.org.as_deref(), Some("Acme Motors"));
    }

    #[test]
    fn parse_handles_folded_lines_and_bare_yyyymmdd_bday() {
        // Fold the NOTE line and use a bare YYYYMMDD BDAY, both of which a
        // real client (e.g. Apple Contacts) may send.
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Jane Example\r\n\
                     BDAY:19900101\r\nNOTE:Line one\r\n continued\r\nEND:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(parsed.birthday.as_deref(), Some("1990-01-01"));
        assert_eq!(parsed.notes.as_deref(), Some("Line onecontinued"));
    }

    #[test]
    fn parse_ignores_unknown_properties() {
        let text =
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\nPHOTO:data:whatever\r\nEND:VCARD\r\n";
        assert!(parse_vcard(text).is_ok());
    }

    #[test]
    fn parse_reads_photo_vcard3_encoding_b_form() {
        // What iOS/Thunderbird actually send — see the module doc comment's
        // confirmed wire formats for every other field this parser supports.
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\n\
                     PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQ\r\nEND:VCARD\r\n";
        let photo = parse_vcard(text).unwrap().photo.unwrap();
        assert_eq!(photo.content_type, "image/jpeg");
        use base64::Engine;
        assert_eq!(
            photo.bytes,
            base64::engine::general_purpose::STANDARD
                .decode("/9j/4AAQ")
                .unwrap()
        );
    }

    #[test]
    fn parse_reads_photo_vcard3_type_png() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\n\
                     PHOTO;ENCODING=b;TYPE=PNG:aGVsbG8=\r\nEND:VCARD\r\n";
        let photo = parse_vcard(text).unwrap().photo.unwrap();
        assert_eq!(photo.content_type, "image/png");
        assert_eq!(photo.bytes, b"hello");
    }

    #[test]
    fn parse_reads_photo_vcard4_data_uri_form() {
        let text = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:u1\r\nFN:X\r\n\
                     PHOTO:data:image/png;base64,aGVsbG8=\r\nEND:VCARD\r\n";
        let photo = parse_vcard(text).unwrap().photo.unwrap();
        assert_eq!(photo.content_type, "image/png");
        assert_eq!(photo.bytes, b"hello");
    }

    #[test]
    fn parse_drops_photo_over_the_size_cap_without_failing_the_vcard() {
        use base64::Engine;
        let huge = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_PHOTO_BYTES + 1]);
        let text = format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\nPHOTO;ENCODING=b;TYPE=JPEG:{huge}\r\nEND:VCARD\r\n"
        );
        let parsed = parse_vcard(&text).unwrap();
        assert!(parsed.photo.is_none());
        assert_eq!(parsed.display_name, "X");
    }

    #[test]
    fn parse_drops_photo_uri_reference_rather_than_treating_it_as_base64() {
        // PHOTO;VALUE=uri:… (a remote reference) isn't supported — only
        // inline base64 is. The value fails to base64-decode and is
        // silently dropped, same as any other unsupported property.
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\n\
                     PHOTO;VALUE=uri:http://example.com/photo.jpg\r\nEND:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert!(parsed.photo.is_none());
    }

    #[test]
    fn generate_photo_emits_encoding_b_and_type_token() {
        let mut f = sample();
        f.photo = Some(VCardPhoto {
            bytes: b"hello".to_vec(),
            content_type: "image/png".into(),
        });
        let text = generate_vcard(&f);
        assert!(text.contains("PHOTO;ENCODING=b;TYPE=PNG:aGVsbG8=\r\n"));
    }

    #[test]
    fn escapes_special_characters_in_values() {
        let mut f = sample();
        f.notes = Some("Semi;colon, comma, back\\slash".into());
        let text = generate_vcard(&f);
        let parsed = parse_vcard(&text).unwrap();
        assert_eq!(
            parsed.notes.as_deref(),
            Some("Semi;colon, comma, back\\slash")
        );
    }

    #[test]
    fn parse_handles_the_exact_ios_url_item_group_wire_format() {
        // The exact bytes a real iPhone sent (2026-09-17) when adding a
        // "HomePage" URL, straight from tidl's own request log.
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Herman De Grote\r\n\
                     N:De Grote;Herman;;;\r\n\
                     item2.URL;type=pref:https://127.0.0.1\r\n\
                     item2.X-ABLabel:_$!<HomePage>!$_\r\n\
                     END:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(parsed.urls.len(), 1);
        assert_eq!(parsed.urls[0].value, "https://127.0.0.1");
        assert!(parsed.urls[0].is_primary);
        // No TYPE= param, so the kind falls back to the decoded "HomePage"
        // label — "home" is the best-effort guess (see kind_from_label).
        assert_eq!(parsed.urls[0].kind, "home");
    }

    #[test]
    fn parse_keeps_every_address_when_a_vcard_has_more_than_one() {
        // Regression: this is the actual data-loss bug that started this
        // work — a second ADR line used to silently overwrite the first
        // instead of both surviving as separate addresses.
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\n\
                     ADR;TYPE=HOME:;;1 Home St;Springfield;;00001;US\r\n\
                     ADR;TYPE=WORK:;;2 Work Ave;Springfield;;00002;US\r\n\
                     END:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(
            parsed.addresses.len(),
            2,
            "both addresses must survive, not just the last one"
        );
        assert_eq!(parsed.addresses[0].kind, "home");
        assert_eq!(parsed.addresses[0].street.as_deref(), Some("1 Home St"));
        assert_eq!(parsed.addresses[1].kind, "work");
        assert_eq!(parsed.addresses[1].street.as_deref(), Some("2 Work Ave"));
    }

    #[test]
    fn parse_maps_x_abdate_to_anniversary_only_when_labeled_as_such() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\n\
                     item1.X-ABDATE:2020-06-01\r\n\
                     item1.X-ABLabel:_$!<Anniversary>!$_\r\n\
                     item2.X-ABDATE:2021-01-01\r\n\
                     item2.X-ABLabel:Some Other Date\r\n\
                     END:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(parsed.anniversary.as_deref(), Some("2020-06-01"));
    }

    #[test]
    fn parse_reads_nickname_categories_and_full_name_components() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:X\r\n\
                     N:Grote;Herman;Jan;Dr.;Jr.\r\n\
                     NICKNAME:Ronnie\r\n\
                     CATEGORIES:Friend,VIP\\, Sort Of\r\n\
                     END:VCARD\r\n";
        let parsed = parse_vcard(text).unwrap();
        assert_eq!(parsed.middle_name.as_deref(), Some("Jan"));
        assert_eq!(parsed.name_prefix.as_deref(), Some("Dr."));
        assert_eq!(parsed.name_suffix.as_deref(), Some("Jr."));
        assert_eq!(parsed.nickname.as_deref(), Some("Ronnie"));
        assert_eq!(parsed.categories, vec!["Friend", "VIP, Sort Of"]);
    }
}
