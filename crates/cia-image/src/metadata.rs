//! EXIF and ICC handling (DESIGN.md 3.4.1 steps 2 to 4). Reading uses
//! kamadak-exif; writing kept fields uses little_exif.

use std::io::Cursor;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExifSummary {
    pub orientation: u32,
    pub has_gps: bool,
    pub date_taken: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub exposure: Option<String>,
    pub f_number: Option<String>,
    pub iso: Option<String>,
    pub focal_length: Option<String>,
}

pub fn read_exif(bytes: &[u8]) -> Option<ExifSummary> {
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)).ok()?;
    let get = |tag: exif::Tag| exif.get_field(tag, exif::In::PRIMARY).map(|f| f.display_value().to_string());
    let orientation = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)).filter(|v| (1..=8).contains(v)).unwrap_or(1);
    let has_gps = exif.fields().any(|f| f.tag.context() == exif::Context::Gps);
    Some(ExifSummary {
        orientation,
        has_gps,
        date_taken: get(exif::Tag::DateTimeOriginal).or_else(|| get(exif::Tag::DateTime)),
        make: get(exif::Tag::Make),
        model: get(exif::Tag::Model),
        exposure: get(exif::Tag::ExposureTime),
        f_number: get(exif::Tag::FNumber),
        iso: get(exif::Tag::PhotographicSensitivity),
        focal_length: get(exif::Tag::FocalLength),
    })
}

/// True when the written file still has GPS fields (verification).
pub fn has_gps(bytes: &[u8]) -> bool {
    read_exif(bytes).map(|e| e.has_gps).unwrap_or(false)
}

/// The EXIF orientation of a written file (verification: must be 1 or absent).
pub fn orientation_of(bytes: &[u8]) -> u32 {
    read_exif(bytes).map(|e| e.orientation).unwrap_or(1)
}

/// Copy the kept EXIF fields (date, camera, exposure; GPS only when asked)
/// from the source into a freshly encoded JPEG/PNG/WebP. Returns the input
/// unchanged when there is nothing to copy or writing is not possible.
pub fn write_kept_exif(output: Vec<u8>, source: &[u8], keep_location: bool, format: crate::OutputImageFormat) -> Vec<u8> {
    use little_exif::exif_tag::ExifTag;
    use little_exif::filetype::FileExtension;
    use little_exif::metadata::Metadata;
    let ext = match format {
        crate::OutputImageFormat::Jpeg => FileExtension::JPEG,
        crate::OutputImageFormat::Png => FileExtension::PNG { as_zTXt_chunk: false },
        crate::OutputImageFormat::Webp => FileExtension::WEBP,
        _ => return output,
    };
    let src_ext = match crate::decode::sniff(source) {
        crate::SourceFormat::Jpeg => FileExtension::JPEG,
        crate::SourceFormat::Png => FileExtension::PNG { as_zTXt_chunk: false },
        crate::SourceFormat::Webp => FileExtension::WEBP,
        crate::SourceFormat::Tiff => FileExtension::TIFF,
        _ => return output,
    };
    let Ok(src_meta) = Metadata::new_from_vec(&source.to_vec(), src_ext) else { return output };
    let mut meta = Metadata::new();
    let mut copied = 0;
    for tag in src_meta.into_iter() {
        let keep = match tag {
            ExifTag::DateTimeOriginal(_) | ExifTag::CreateDate(_) | ExifTag::ModifyDate(_) | ExifTag::Make(_) | ExifTag::Model(_) | ExifTag::LensModel(_) | ExifTag::ExposureTime(_) | ExifTag::FNumber(_) | ExifTag::ISO(_) | ExifTag::FocalLength(_) | ExifTag::ExposureProgram(_) | ExifTag::Flash(_) | ExifTag::WhiteBalance(_) | ExifTag::Software(_) | ExifTag::Artist(_) | ExifTag::Copyright(_) | ExifTag::ImageDescription(_) => true,
            ExifTag::GPSLatitude(_) | ExifTag::GPSLatitudeRef(_) | ExifTag::GPSLongitude(_) | ExifTag::GPSLongitudeRef(_) | ExifTag::GPSAltitude(_) | ExifTag::GPSAltitudeRef(_) | ExifTag::GPSTimeStamp(_) | ExifTag::GPSDateStamp(_) | ExifTag::GPSSpeed(_) | ExifTag::GPSSpeedRef(_) | ExifTag::GPSImgDirection(_) | ExifTag::GPSImgDirectionRef(_) | ExifTag::GPSDestBearing(_) | ExifTag::GPSDestBearingRef(_) | ExifTag::GPSHPositioningError(_) | ExifTag::GPSVersionID(_) | ExifTag::GPSMapDatum(_) => keep_location,
            _ => false,
        };
        if keep {
            meta.set_tag(tag.clone());
            copied += 1;
        }
    }
    if copied == 0 {
        return output;
    }
    meta.set_tag(ExifTag::Orientation(vec![1]));
    let mut buf = output.clone();
    match meta.write_to_vec(&mut buf, ext) {
        Ok(()) => buf,
        Err(_) => output,
    }
}

/// APP2 ICC_PROFILE segments concatenated.
pub fn jpeg_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut i = 2;
    let mut chunks: Vec<(u8, Vec<u8>)> = Vec::new();
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            break;
        }
        let seg = &bytes[i + 4..i + 2 + len];
        if marker == 0xE2 && seg.len() > 14 && &seg[0..12] == b"ICC_PROFILE\0" {
            chunks.push((seg[12], seg[14..].to_vec()));
        }
        i += 2 + len;
    }
    if chunks.is_empty() {
        return None;
    }
    chunks.sort_by_key(|c| c.0);
    Some(chunks.into_iter().flat_map(|c| c.1).collect())
}

/// iCCP chunk, inflated.
pub fn png_icc(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut i = 8;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let kind = &bytes[i + 4..i + 8];
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        if kind == b"iCCP" && i + 8 + len <= bytes.len() {
            let data = &bytes[i + 8..i + 8 + len];
            let name_end = data.iter().position(|&b| b == 0)?;
            let comp = data.get(name_end + 2..)?;
            let mut out = Vec::new();
            let mut z = flate2::read::ZlibDecoder::new(comp);
            std::io::Read::read_to_end(&mut z, &mut out).ok()?;
            return Some(out);
        }
        i += 12 + len;
    }
    None
}

/// Heuristic: the profile description says sRGB, or it is the tiny sRGB profile size class.
pub fn icc_is_srgb(icc: &[u8]) -> bool {
    let hay = String::from_utf8_lossy(icc);
    hay.contains("sRGB") || hay.contains("s\0R\0G\0B") || icc.len() < 1024
}
