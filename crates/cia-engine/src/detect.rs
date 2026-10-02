//! Type detection from content (DESIGN.md 3.3): the first 64 KiB decide the
//! `Kind`; the extension is only a tiebreaker.

use cia_core::Kind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub kind: Kind,
    /// Lower-case format token: "jpeg", "png", "mp4", "docx", "pdf", "zip"...
    pub format: String,
}

fn ext_of(name: &str) -> String {
    name.rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| e.len() <= 5 && !e.contains('/'))
        .unwrap_or_default()
}

/// Sniff a ZIP's purpose from its central directory names: OOXML, ODF, EPUB or plain archive.
fn zip_flavour(head: &[u8], ext: &str) -> Detected {
    let hay = String::from_utf8_lossy(head);
    let looks = |s: &str| hay.contains(s);
    if looks("[Content_Types].xml") || looks("word/") || looks("ppt/") || looks("xl/") {
        let fmt = match ext {
            "docm" | "pptm" | "xlsm" => ext.to_string(),
            "pptx" => "pptx".into(),
            "xlsx" => "xlsx".into(),
            "docx" => "docx".into(),
            _ if looks("ppt/") => "pptx".into(),
            _ if looks("xl/") => "xlsx".into(),
            _ => "docx".into(),
        };
        return Detected {
            kind: Kind::OfficeDoc,
            format: fmt,
        };
    }
    if looks("mimetypeapplication/epub") {
        return Detected {
            kind: Kind::OfficeDoc,
            format: "epub".into(),
        };
    }
    if looks("mimetypeapplication/vnd.oasis.opendocument") {
        let fmt = if looks("opendocument.presentation") {
            "odp"
        } else if looks("opendocument.spreadsheet") {
            "ods"
        } else {
            "odt"
        };
        return Detected {
            kind: Kind::OfficeDoc,
            format: fmt.into(),
        };
    }
    Detected {
        kind: Kind::Archive,
        format: "zip".into(),
    }
}

fn is_text(head: &[u8]) -> bool {
    if head.is_empty() {
        return false;
    }
    if head.starts_with(&[0xEF, 0xBB, 0xBF])
        || head.starts_with(&[0xFF, 0xFE])
        || head.starts_with(&[0xFE, 0xFF])
    {
        return true;
    }
    let sample = &head[..head.len().min(8192)];
    let nul = sample.iter().filter(|&&b| b == 0).count();
    if nul > 0 {
        return false;
    }
    let printable = sample
        .iter()
        .filter(|&&b| b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    printable * 100 / sample.len() >= 95 && std::str::from_utf8(sample).is_ok()
        || printable * 100 / sample.len() >= 98
}

/// `head` is the first 64 KiB (or the whole file); `name` supplies the extension tiebreaker.
pub fn detect(head: &[u8], name: &str) -> Detected {
    let ext = ext_of(name);
    let b = head;
    // Images (own sniffer handles AVIF/HEIC/JXL too).
    let img = cia_image::decode::sniff(b);
    if img != cia_image::SourceFormat::Unknown {
        let animated = match img {
            cia_image::SourceFormat::Gif => cia_image::inspect(b)
                .map(|i| i.frames > 1)
                .unwrap_or(ext == "gif"),
            cia_image::SourceFormat::Png | cia_image::SourceFormat::Webp => {
                cia_image::inspect(b).map(|i| i.frames > 1).unwrap_or(false)
            }
            _ => false,
        };
        return Detected {
            kind: if animated {
                Kind::AnimatedImage
            } else {
                Kind::Image
            },
            format: img.token().into(),
        };
    }
    if b.starts_with(b"%PDF") {
        return Detected {
            kind: Kind::Pdf,
            format: "pdf".into(),
        };
    }
    if b.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        // OLE compound file: legacy Office or password-protected OOXML.
        return Detected {
            kind: Kind::OfficeDoc,
            format: if ["doc", "ppt", "xls"].contains(&ext.as_str()) {
                ext.clone()
            } else {
                "ole".into()
            },
        };
    }
    if b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06") {
        return zip_flavour(b, &ext);
    }
    if b.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Detected {
            kind: Kind::Archive,
            format: "7z".into(),
        };
    }
    if b.starts_with(b"Rar!\x1A\x07") {
        return Detected {
            kind: Kind::Archive,
            format: "rar".into(),
        };
    }
    if b.starts_with(&[0x1F, 0x8B]) {
        return Detected {
            kind: Kind::Archive,
            format: if ext == "tgz" || name.to_ascii_lowercase().ends_with(".tar.gz") {
                "tar.gz".into()
            } else {
                "gz".into()
            },
        };
    }
    if b.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        return Detected {
            kind: Kind::Archive,
            format: if name.to_ascii_lowercase().ends_with(".tar.xz") || ext == "txz" {
                "tar.xz".into()
            } else {
                "xz".into()
            },
        };
    }
    if b.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Detected {
            kind: Kind::Archive,
            format: if name.to_ascii_lowercase().ends_with(".tar.zst") {
                "tar.zst".into()
            } else {
                "zst".into()
            },
        };
    }
    if b.starts_with(b"BZh") {
        return Detected {
            kind: Kind::Archive,
            format: if name.to_ascii_lowercase().ends_with(".tar.bz2") {
                "tar.bz2".into()
            } else {
                "bz2".into()
            },
        };
    }
    if b.len() > 262 && &b[257..262] == b"ustar" {
        return Detected {
            kind: Kind::Archive,
            format: "tar".into(),
        };
    }
    // Video and audio containers.
    if b.len() > 12 && &b[4..8] == b"ftyp" {
        let brand = String::from_utf8_lossy(&b[8..12]).to_ascii_lowercase();
        let audio_only =
            brand.starts_with("m4a") || brand == "m4b " || ext == "m4a" || ext == "m4b";
        if audio_only {
            return Detected {
                kind: Kind::Audio,
                format: "m4a".into(),
            };
        }
        let fmt = if brand.starts_with("qt") || ext == "mov" {
            "mov"
        } else if ext == "3gp" || brand.starts_with("3gp") {
            "3gp"
        } else {
            "mp4"
        };
        return Detected {
            kind: Kind::Video,
            format: fmt.into(),
        };
    }
    if b.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        let hay = String::from_utf8_lossy(&b[..b.len().min(64)]);
        return Detected {
            kind: Kind::Video,
            format: if hay.contains("webm") || ext == "webm" {
                "webm".into()
            } else {
                "mkv".into()
            },
        };
    }
    if b.starts_with(b"RIFF") && b.len() > 12 {
        match &b[8..12] {
            b"AVI " => {
                return Detected {
                    kind: Kind::Video,
                    format: "avi".into(),
                }
            }
            b"WAVE" => {
                return Detected {
                    kind: Kind::Audio,
                    format: "wav".into(),
                }
            }
            _ => {}
        }
    }
    if b.starts_with(&[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11]) {
        return Detected {
            kind: if ext == "wma" {
                Kind::Audio
            } else {
                Kind::Video
            },
            format: if ext == "wma" {
                "wma".into()
            } else {
                "wmv".into()
            },
        };
    }
    if b.starts_with(b"FLV\x01") {
        return Detected {
            kind: Kind::Video,
            format: "flv".into(),
        };
    }
    if b.len() > 188 && b[0] == 0x47 && b[188] == 0x47 {
        return Detected {
            kind: Kind::Video,
            format: "ts".into(),
        };
    }
    if b.starts_with(&[0x00, 0x00, 0x01, 0xBA]) {
        return Detected {
            kind: Kind::Video,
            format: "mpeg".into(),
        };
    }
    if b.starts_with(b"OggS") {
        let hay = String::from_utf8_lossy(&b[..b.len().min(128)]);
        if hay.contains("theora") {
            return Detected {
                kind: Kind::Video,
                format: "ogv".into(),
            };
        }
        return Detected {
            kind: Kind::Audio,
            format: if hay.contains("OpusHead") {
                "opus".into()
            } else {
                "ogg".into()
            },
        };
    }
    if b.starts_with(b"fLaC") {
        return Detected {
            kind: Kind::Audio,
            format: "flac".into(),
        };
    }
    if b.starts_with(b"ID3")
        || (b.len() > 2 && b[0] == 0xFF && (b[1] & 0xE0) == 0xE0 && ext == "mp3")
    {
        return Detected {
            kind: Kind::Audio,
            format: "mp3".into(),
        };
    }
    if b.starts_with(b"FORM") && b.len() > 12 && (&b[8..12] == b"AIFF" || &b[8..12] == b"AIFC") {
        return Detected {
            kind: Kind::Audio,
            format: "aiff".into(),
        };
    }
    if b.starts_with(b"caff") {
        return Detected {
            kind: Kind::Audio,
            format: "caf".into(),
        };
    }
    if (b.starts_with(&[0xFF, 0xF1]) || b.starts_with(&[0xFF, 0xF9])) && ext == "aac" {
        return Detected {
            kind: Kind::Audio,
            format: "aac".into(),
        };
    }
    if is_text(b) {
        let fmt = match ext.as_str() {
            "csv" | "json" | "xml" | "svg" | "log" | "md" | "txt" | "html" | "htm" => ext.clone(),
            _ => "txt".into(),
        };
        return Detected {
            kind: Kind::Text,
            format: fmt,
        };
    }
    Detected {
        kind: Kind::Other,
        format: if ext.is_empty() { "bin".into() } else { ext },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn magic_wins_over_extension() {
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
        ];
        assert_eq!(
            detect(&jpeg, "photo.png"),
            Detected {
                kind: Kind::Image,
                format: "jpeg".into()
            }
        );
        let mut mp4 = vec![
            0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm',
        ];
        mp4.extend_from_slice(&[0; 16]);
        assert_eq!(detect(&mp4, "clip.txt").kind, Kind::Video);
        let mut m4a = vec![
            0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'M', b'4', b'A', b' ',
        ];
        m4a.extend_from_slice(&[0; 16]);
        assert_eq!(
            detect(&m4a, "song.m4a"),
            Detected {
                kind: Kind::Audio,
                format: "m4a".into()
            }
        );
    }
    #[test]
    fn zip_flavours() {
        let mut docx = b"PK\x03\x04".to_vec();
        docx.extend_from_slice(b"...[Content_Types].xml...word/document.xml");
        assert_eq!(
            detect(&docx, "x.docx"),
            Detected {
                kind: Kind::OfficeDoc,
                format: "docx".into()
            }
        );
        let mut epub = b"PK\x03\x04".to_vec();
        epub.extend_from_slice(b"\x00\x00mimetypeapplication/epub+zip");
        assert_eq!(detect(&epub, "x.epub").format, "epub");
        let plain = b"PK\x03\x04\x14\x00\x00\x00\x08\x00photo.jpg".to_vec();
        assert_eq!(detect(&plain, "x.zip").kind, Kind::Archive);
    }
    #[test]
    fn text_and_other() {
        assert_eq!(detect(b"hello world\nline two\n", "notes").kind, Kind::Text);
        assert_eq!(
            detect(
                &[0, 1, 2, 3, 4, 5, 250, 251, 7, 8, 9, 10, 11, 12, 13],
                "blob"
            )
            .kind,
            Kind::Other
        );
        assert_eq!(detect(b"%PDF-1.4\n", "a.pdf").kind, Kind::Pdf);
        assert_eq!(detect(b"Rar!\x1A\x07\x01\x00", "a.rar").format, "rar");
    }
}
