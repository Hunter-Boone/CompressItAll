//! Thin quick-xml helpers shared by detection, the EPUB manifest reader,
//! metadata stripping and verification. Nothing here edits document text.

use quick_xml::events::Event;
use quick_xml::Reader;

/// One start or empty element: local name plus `(qualified name, value)`
/// attribute pairs, values unescaped where quick-xml can.
pub(crate) struct Element {
    pub local_name: String,
    pub attributes: Vec<(String, String)>,
}

impl Element {
    /// Attribute by qualified name (`manifest:full-path`) or, when `name`
    /// has no prefix, by local name (`href` matches `opf:href` too).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == name || (!name.contains(':') && local(k) == name))
            .map(|(_, v)| v.as_str())
    }
}

fn local(qualified: &str) -> &str {
    qualified.rsplit(':').next().unwrap_or(qualified)
}

/// Walk every element of `xml`, calling `f` for each start or empty tag.
/// Returns the parser's error message when the document is not well
/// formed (mismatched tags, bad attribute syntax) or ends with elements
/// still open: quick-xml itself treats a truncated document as a clean
/// EOF, so the depth is tracked here.
pub(crate) fn for_each_element(xml: &[u8], mut f: impl FnMut(&Element)) -> Result<(), String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut saw_element = false;
    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| e.to_string())?;
        match ev {
            Event::Eof => {
                return if depth > 0 {
                    Err(format!("document ends with {depth} unclosed element(s)"))
                } else if !saw_element {
                    Err("no root element".to_owned())
                } else {
                    Ok(())
                };
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Start(ref e) | Event::Empty(ref e) => {
                if matches!(ev, Event::Start(_)) {
                    depth += 1;
                }
                saw_element = true;
                let mut attributes = Vec::new();
                for a in e.attributes() {
                    let a = a.map_err(|e| e.to_string())?;
                    let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    let value = match a.unescape_value() {
                        Ok(v) => v.into_owned(),
                        Err(_) => String::from_utf8_lossy(&a.value).into_owned(),
                    };
                    attributes.push((key, value));
                }
                f(&Element {
                    local_name: String::from_utf8_lossy(e.local_name().as_ref()).into_owned(),
                    attributes,
                });
            }
            _ => {}
        }
        buf.clear();
    }
}

/// True when quick-xml reads `xml` to the end without a syntax error.
pub(crate) fn parses(xml: &[u8]) -> Result<(), String> {
    for_each_element(xml, |_| {})
}

/// Directory part of a `/`-separated package path, with the trailing `/`
/// (`"word/"` for `word/document.xml`, `""` for a root entry).
pub(crate) fn dir_of(name: &str) -> &str {
    match name.rfind('/') {
        Some(i) => &name[..=i],
        None => "",
    }
}

/// Resolve `target` against `base_dir` the way OPC and EPUB readers do:
/// a leading `/` means the package root, `.` and `..` segments collapse,
/// `%xx` escapes decode, and a `#fragment` or `?query` is dropped.
pub(crate) fn resolve(base_dir: &str, target: &str) -> String {
    let target = target
        .split(['#', '?'])
        .next()
        .unwrap_or("")
        .replace('\\', "/");
    let target = percent_decode(&target);
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_owned()
    } else {
        format!("{base_dir}{target}")
    };
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_relative_absolute_and_dotdot() {
        assert_eq!(
            resolve("word/", "media/image1.png"),
            "word/media/image1.png"
        );
        assert_eq!(resolve("word/", "/ppt/x.xml"), "ppt/x.xml");
        assert_eq!(
            resolve("ppt/slides/", "../media/a.jpeg"),
            "ppt/media/a.jpeg"
        );
        assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolve("OEBPS/", "img/a%20b.png#x"), "OEBPS/img/a b.png");
        assert_eq!(resolve("a/", "./b/./c"), "a/b/c");
    }

    #[test]
    fn dir_of_examples() {
        assert_eq!(dir_of("word/_rels/document.xml.rels"), "word/_rels/");
        assert_eq!(dir_of("mimetype"), "");
    }

    #[test]
    fn elements_and_errors() {
        let mut seen = Vec::new();
        for_each_element(
            br#"<?xml version="1.0"?><a x="1"><b:c y="&amp;"/></a>"#,
            |e| seen.push((e.local_name.clone(), e.attr("y").map(str::to_owned))),
        )
        .unwrap();
        assert_eq!(
            seen,
            vec![
                ("a".to_owned(), None),
                ("c".to_owned(), Some("&".to_owned()))
            ]
        );
        assert!(parses(b"<a><b></a>").is_err());
        assert!(parses(b"<a>").is_err());
        assert!(parses(b"<a><b>text</b>").is_err());
        assert!(parses(b"").is_err());
        assert!(parses(b"<a/>").is_ok());
        assert!(parses(b"<a><b/></a>").is_ok());
    }
}
