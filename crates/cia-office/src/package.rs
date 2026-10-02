//! Reading an Office package: the ZIP entries, which kind of document it is
//! (DESIGN.md 3.3) and which entries are media (3.8 step 2).

use std::collections::HashMap;

use cia_archive::{Archive, ArchiveError, ArchiveFormat};

use crate::xml;
use crate::{ImageFormat, MediaKind, OfficeError, OfficeKind};

/// OLE compound file header. Password-protected OOXML is wrapped in one.
pub(crate) const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Files that may legitimately differ between input and output besides
/// media: the document-details parts rewritten when
/// `keep_document_details` is off.
pub(crate) const DETAILS_PARTS: [&str; 2] = ["docProps/core.xml", "meta.xml"];

pub(crate) struct Entry {
    pub name: String,
    pub bytes: Vec<u8>,
    pub is_dir: bool,
}

pub(crate) struct Package {
    pub kind: OfficeKind,
    /// Every entry in archive order, directories included.
    pub entries: Vec<Entry>,
    /// `(entry index, media kind)` for every media entry, in entry order.
    pub media: Vec<(usize, MediaKind)>,
}

impl Package {
    pub fn media_kind(&self, index: usize) -> Option<MediaKind> {
        self.media
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, k)| *k)
    }

    /// Entry indices in the order the output must use: the original order,
    /// except that ODF and EPUB move `mimetype` to the front (3.8 step 4).
    pub fn output_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        if self.kind.has_mimetype() {
            if let Some(pos) = order
                .iter()
                .position(|&i| self.entries[i].name == "mimetype")
            {
                let i = order.remove(pos);
                order.insert(0, i);
            }
        }
        order
    }
}

pub(crate) fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

fn map_archive_error(e: ArchiveError) -> OfficeError {
    match e {
        ArchiveError::Encrypted(_) => OfficeError::Encrypted,
        ArchiveError::Unsupported(m) if m.starts_with("unrecognised") => OfficeError::NotOffice,
        ArchiveError::Unsupported(m) => OfficeError::Unsupported(m),
        ArchiveError::UnsupportedMethod { name, method } => {
            OfficeError::Unsupported(format!("entry {name} uses {method}"))
        }
        ArchiveError::TooLarge { limit } => {
            OfficeError::Unsupported(format!("an entry unpacks to more than {limit} bytes"))
        }
        other => OfficeError::Corrupt(other.to_string()),
    }
}

/// Open the ZIP and classify it from its names and one small entry, without
/// inflating anything else. `Ok(None)` when it is a ZIP but not an Office
/// package.
pub(crate) fn open(bytes: &[u8]) -> Result<Option<(Archive<'_>, OfficeKind)>, OfficeError> {
    if bytes.starts_with(&CFB_MAGIC) {
        return Err(OfficeError::Encrypted);
    }
    if !is_zip(bytes) {
        return Ok(None);
    }
    let mut archive = cia_archive::open(bytes).map_err(map_archive_error)?;
    if archive.format() != ArchiveFormat::Zip {
        return Ok(None);
    }
    for e in archive.entries() {
        if e.encrypted {
            return Err(OfficeError::Encrypted);
        }
    }
    let names: Vec<String> = archive.entries().iter().map(|e| e.name.clone()).collect();
    let kind = if let Some(i) = names.iter().position(|n| n == "[Content_Types].xml") {
        let ct = archive.extract(i).map_err(map_archive_error)?;
        ooxml_kind(&ct, &names)
    } else if let Some(i) = names.iter().position(|n| n == "mimetype") {
        let m = archive.extract(i).map_err(map_archive_error)?;
        mimetype_kind(String::from_utf8_lossy(&m).trim())
    } else {
        None
    };
    Ok(kind.map(|k| (archive, k)))
}

/// Read the whole package into memory and classify its media.
pub(crate) fn read(bytes: &[u8]) -> Result<Package, OfficeError> {
    let (mut archive, kind) = open(bytes)?.ok_or(OfficeError::NotOffice)?;
    for e in archive.entries() {
        if !e.supported {
            return Err(OfficeError::Unsupported(format!(
                "entry {} uses {}",
                e.name, e.method
            )));
        }
    }
    let data = archive.extract_all().map_err(map_archive_error)?;
    let entries: Vec<Entry> = archive
        .entries()
        .iter()
        .zip(data)
        .map(|(e, bytes)| Entry {
            name: e.name.clone(),
            bytes,
            is_dir: e.is_dir,
        })
        .collect();
    let media = classify_all(kind, &entries);
    Ok(Package {
        kind,
        entries,
        media,
    })
}

fn ooxml_kind(content_types: &[u8], names: &[String]) -> Option<OfficeKind> {
    use OfficeKind::*;
    const MAIN_PARTS: [(&str, OfficeKind); 15] = [
        (
            "application/vnd.ms-word.document.macroEnabled.main+xml",
            Docm,
        ),
        (
            "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
            Docm,
        ),
        (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            Docx,
        ),
        (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
            Docx,
        ),
        (
            "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml",
            Pptm,
        ),
        (
            "application/vnd.ms-powerpoint.slideshow.macroEnabled.main+xml",
            Pptm,
        ),
        (
            "application/vnd.ms-powerpoint.template.macroEnabled.main+xml",
            Pptm,
        ),
        (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
            Pptx,
        ),
        (
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml",
            Pptx,
        ),
        (
            "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml",
            Pptx,
        ),
        ("application/vnd.ms-excel.sheet.macroEnabled.main+xml", Xlsm),
        (
            "application/vnd.ms-excel.template.macroEnabled.main+xml",
            Xlsm,
        ),
        ("application/vnd.ms-excel.addin.macroEnabled.main+xml", Xlsm),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            Xlsx,
        ),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            Xlsx,
        ),
    ];
    let text = String::from_utf8_lossy(content_types);
    let from_types = MAIN_PARTS
        .iter()
        .find(|(ct, _)| text.contains(ct))
        .map(|(_, k)| *k);
    let has_dir = |p: &str| names.iter().any(|n| n.starts_with(p));
    let kind = from_types.or_else(|| {
        // Some generators omit the Override; the part layout still tells.
        if has_dir("word/") {
            Some(Docx)
        } else if has_dir("ppt/") {
            Some(Pptx)
        } else if has_dir("xl/") {
            Some(Xlsx)
        } else {
            None
        }
    })?;
    let has_vba = names.iter().any(|n| n.ends_with("vbaProject.bin"));
    Some(match (kind, has_vba) {
        (Docx, true) => Docm,
        (Pptx, true) => Pptm,
        (Xlsx, true) => Xlsm,
        (k, _) => k,
    })
}

fn mimetype_kind(mimetype: &str) -> Option<OfficeKind> {
    const ODF: &str = "application/vnd.oasis.opendocument.";
    if mimetype == "application/epub+zip" {
        return Some(OfficeKind::Epub);
    }
    let rest = mimetype.strip_prefix(ODF)?;
    if rest.starts_with("text") {
        Some(OfficeKind::Odt)
    } else if rest.starts_with("presentation") {
        Some(OfficeKind::Odp)
    } else if rest.starts_with("spreadsheet") {
        Some(OfficeKind::Ods)
    } else {
        None
    }
}

fn classify_all(kind: OfficeKind, entries: &[Entry]) -> Vec<(usize, MediaKind)> {
    let manifest = if kind == OfficeKind::Epub {
        epub_manifest(entries)
    } else {
        HashMap::new()
    };
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_dir)
        .filter_map(|(i, e)| classify(kind, &e.name, &e.bytes, &manifest).map(|k| (i, k)))
        .collect()
}

fn classify(
    kind: OfficeKind,
    name: &str,
    bytes: &[u8],
    manifest: &HashMap<String, String>,
) -> Option<MediaKind> {
    if kind == OfficeKind::Epub {
        return kind_from_media_type(manifest.get(name)?, name, bytes);
    }
    let in_media_dir = if kind.is_ooxml() {
        name.starts_with("word/media/")
            || name.starts_with("ppt/media/")
            || name.starts_with("xl/media/")
    } else {
        name.starts_with("Pictures/")
    };
    in_media_dir.then(|| kind_from_extension(name, bytes))
}

pub(crate) fn extension(name: &str) -> String {
    let file = name.rsplit('/').next().unwrap_or(name);
    match file.rfind('.') {
        Some(i) => file[i + 1..].to_ascii_lowercase(),
        None => String::new(),
    }
}

/// An entry only counts as a JPEG or PNG when the extension and the bytes
/// agree: images keep their format and extension (3.8 step 2), so a `.png`
/// that is really a JPEG is left alone rather than rewritten.
fn image_if_magic(format: ImageFormat, bytes: &[u8]) -> MediaKind {
    if format.matches(bytes) {
        MediaKind::Image(format)
    } else {
        MediaKind::Other
    }
}

fn kind_from_extension(name: &str, bytes: &[u8]) -> MediaKind {
    match extension(name).as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => image_if_magic(ImageFormat::Jpeg, bytes),
        "png" => image_if_magic(ImageFormat::Png, bytes),
        "mp4" | "mov" | "m4v" | "avi" | "wmv" | "mpg" | "mpeg" | "webm" | "mkv" | "asf" => {
            MediaKind::Video
        }
        "mp3" | "m4a" | "wav" | "wma" | "aac" | "ogg" | "oga" | "flac" | "aiff" | "aif"
        | "opus" => MediaKind::Audio,
        _ => MediaKind::Other,
    }
}

fn kind_from_media_type(media_type: &str, name: &str, bytes: &[u8]) -> Option<MediaKind> {
    let mt = media_type.trim().to_ascii_lowercase();
    Some(match mt.as_str() {
        "image/jpeg" | "image/jpg" => image_if_magic(ImageFormat::Jpeg, bytes),
        "image/png" => image_if_magic(ImageFormat::Png, bytes),
        m if m.starts_with("video/") => MediaKind::Video,
        m if m.starts_with("audio/") => MediaKind::Audio,
        m if m.starts_with("image/") => MediaKind::Other,
        // Not a media type we know; fall back to the name for the odd
        // manifest that says application/octet-stream for a picture.
        _ => match kind_from_extension(name, bytes) {
            MediaKind::Other => return None,
            k => k,
        },
    })
}

/// EPUB manifest: entry name to media type, for every `<item>` of every
/// rootfile named in `META-INF/container.xml`. Empty when anything needed
/// is missing or malformed; the package is then treated as having no media.
pub(crate) fn epub_manifest(entries: &[Entry]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for opf_path in epub_rootfiles(entries) {
        let Some(opf) = entries.iter().find(|e| e.name == opf_path) else {
            continue;
        };
        let base = xml::dir_of(&opf_path);
        let _ = xml::for_each_element(&opf.bytes, |el| {
            if el.local_name != "item" {
                return;
            }
            if let (Some(href), Some(mt)) = (el.attr("href"), el.attr("media-type")) {
                if href.contains("://") {
                    return;
                }
                out.insert(xml::resolve(base, href), mt.to_owned());
            }
        });
    }
    out
}

/// OPF paths listed in `META-INF/container.xml`.
pub(crate) fn epub_rootfiles(entries: &[Entry]) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(c) = entries.iter().find(|e| e.name == "META-INF/container.xml") {
        let _ = xml::for_each_element(&c.bytes, |el| {
            if el.local_name == "rootfile" {
                if let Some(p) = el.attr("full-path") {
                    paths.push(xml::resolve("", p));
                }
            }
        });
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mimetypes() {
        assert_eq!(
            mimetype_kind("application/vnd.oasis.opendocument.text"),
            Some(OfficeKind::Odt)
        );
        assert_eq!(
            mimetype_kind("application/vnd.oasis.opendocument.text-template"),
            Some(OfficeKind::Odt)
        );
        assert_eq!(
            mimetype_kind("application/vnd.oasis.opendocument.presentation"),
            Some(OfficeKind::Odp)
        );
        assert_eq!(
            mimetype_kind("application/vnd.oasis.opendocument.spreadsheet"),
            Some(OfficeKind::Ods)
        );
        assert_eq!(
            mimetype_kind("application/epub+zip"),
            Some(OfficeKind::Epub)
        );
        assert_eq!(
            mimetype_kind("application/vnd.oasis.opendocument.graphics"),
            None
        );
        assert_eq!(mimetype_kind("text/plain"), None);
    }

    #[test]
    fn ooxml_macro_variants() {
        let names = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let ct = |s: &str| format!(r#"<Types><Override ContentType="{s}"/></Types>"#);
        assert_eq!(
            ooxml_kind(
                ct("application/vnd.ms-word.document.macroEnabled.main+xml").as_bytes(),
                &names(&["word/document.xml"])
            ),
            Some(OfficeKind::Docm)
        );
        assert_eq!(
            ooxml_kind(
                ct("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml")
                    .as_bytes(),
                &names(&["xl/workbook.xml", "xl/vbaProject.bin"])
            ),
            Some(OfficeKind::Xlsm)
        );
        assert_eq!(
            ooxml_kind(b"<Types/>", &names(&["ppt/presentation.xml"])),
            Some(OfficeKind::Pptx)
        );
        assert_eq!(ooxml_kind(b"<Types/>", &names(&["foo.txt"])), None);
    }

    #[test]
    fn extension_and_kind() {
        assert_eq!(extension("word/media/Image1.JPEG"), "jpeg");
        assert_eq!(extension("mimetype"), "");
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0];
        assert_eq!(
            kind_from_extension("ppt/media/a.jpg", &jpeg),
            MediaKind::Image(ImageFormat::Jpeg)
        );
        // Extension says PNG, bytes say JPEG: left alone.
        assert_eq!(
            kind_from_extension("ppt/media/a.png", &jpeg),
            MediaKind::Other
        );
        assert_eq!(
            kind_from_extension("ppt/media/a.mp4", &[]),
            MediaKind::Video
        );
        assert_eq!(
            kind_from_extension("ppt/media/a.m4a", &[]),
            MediaKind::Audio
        );
        assert_eq!(
            kind_from_extension("ppt/media/a.emf", &[]),
            MediaKind::Other
        );
    }
}
