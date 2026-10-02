//! Document details (`keep_document_details = false`): blank the author and
//! title style fields of `docProps/core.xml` (OOXML) and `meta.xml` (ODF)
//! by rewriting only those elements' text. The files stay, every other
//! byte of markup is streamed through unchanged, and a file that does not
//! parse is left exactly as it was: this crate never risks the document to
//! remove a name.

use quick_xml::events::Event;
use quick_xml::{Reader, Writer};

use crate::OfficeKind;

/// Local names of the `docProps/core.xml` elements whose text goes.
pub(crate) const CORE_FIELDS: &[&str] = &[
    "creator",
    "lastModifiedBy",
    "title",
    "subject",
    "description",
    "keywords",
    "category",
    "contentStatus",
];

/// Local names of the ODF `meta.xml` elements whose text goes.
pub(crate) const META_FIELDS: &[&str] = &[
    "initial-creator",
    "creator",
    "printed-by",
    "title",
    "subject",
    "description",
    "keyword",
];

/// The rewritten bytes for a details part, or `None` when `name` is not a
/// details part for this kind, it does not parse, or nothing changed.
pub(crate) fn strip_details(kind: OfficeKind, name: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let fields = if kind.is_ooxml() && name == "docProps/core.xml" {
        CORE_FIELDS
    } else if kind.is_odf() && name == "meta.xml" {
        META_FIELDS
    } else {
        return None;
    };
    let out = strip_text(bytes, fields).ok()?;
    (out != bytes).then_some(out)
}

/// Copy `xml` dropping the text and CDATA inside any element whose local
/// name is in `fields` (the element itself stays, empty). Nested elements
/// inside a field keep their markup but lose their text too.
pub(crate) fn strip_text(xml: &[u8], fields: &[&str]) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut depth_inside = 0usize;
    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?;
        match &ev {
            Event::Eof => {
                if depth > 0 {
                    return Err(format!("document ends with {depth} unclosed element(s)"));
                }
                break;
            }
            Event::Start(e) => {
                depth += 1;
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if depth_inside > 0 || fields.contains(&local.as_str()) {
                    depth_inside += 1;
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                depth_inside = depth_inside.saturating_sub(1);
            }
            // Entity references arrive as their own events in quick-xml,
            // so "A &amp; B" is three events and all three must go.
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if depth_inside > 0 => {
                buf.clear();
                continue;
            }
            _ => {}
        }
        writer.write_event(ev).map_err(|e| e.to_string())?;
        buf.clear();
    }
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>Quarterly &amp; annual</dc:title><dc:creator>Jane Doe</dc:creator><cp:lastModifiedBy>Jane Doe</cp:lastModifiedBy><cp:revision>3</cp:revision><dcterms:created xsi:type="dcterms:W3CDTF">2024-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#;

    #[test]
    fn strips_only_listed_fields() {
        let out = strip_text(CORE.as_bytes(), CORE_FIELDS).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("<dc:title></dc:title>"), "{s}");
        assert!(s.contains("<dc:creator></dc:creator>"), "{s}");
        assert!(s.contains("<cp:lastModifiedBy></cp:lastModifiedBy>"), "{s}");
        assert!(s.contains("<cp:revision>3</cp:revision>"), "{s}");
        assert!(s.contains("2024-01-02T03:04:05Z"), "{s}");
        assert!(!s.contains("Jane"), "{s}");
        assert!(s.starts_with(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#));
        crate::xml::parses(s.as_bytes()).unwrap();
    }

    #[test]
    fn untouched_when_nothing_to_strip_or_wrong_part() {
        let plain = br#"<a><b>x</b></a>"#;
        assert_eq!(
            strip_details(OfficeKind::Docx, "docProps/app.xml", plain),
            None
        );
        assert_eq!(
            strip_details(OfficeKind::Docx, "docProps/core.xml", plain),
            None
        );
        assert_eq!(
            strip_details(OfficeKind::Odt, "docProps/core.xml", CORE.as_bytes()),
            None
        );
        assert!(strip_details(OfficeKind::Docx, "docProps/core.xml", CORE.as_bytes()).is_some());
    }

    #[test]
    fn broken_xml_is_left_alone() {
        assert_eq!(
            strip_details(OfficeKind::Docx, "docProps/core.xml", b"<dc:creator>oops"),
            None
        );
    }

    #[test]
    fn odf_meta() {
        let meta = br#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><office:meta><meta:initial-creator>A</meta:initial-creator><dc:creator>B</dc:creator><meta:editing-cycles>4</meta:editing-cycles></office:meta></office:document-meta>"#;
        let out = strip_details(OfficeKind::Odt, "meta.xml", meta).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("<meta:initial-creator></meta:initial-creator>"));
        assert!(s.contains("<dc:creator></dc:creator>"));
        assert!(s.contains("<meta:editing-cycles>4</meta:editing-cycles>"));
    }
}
