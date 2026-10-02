//! Writing the package back (DESIGN.md 3.8 step 4) with cia-archive's exact
//! ZIP writer: same entry order, `mimetype` first and stored for ODF/EPUB,
//! deflate level 9 (zopfli under 1 MB) for everything else that shrinks,
//! stored when deflate saves under one percent.

use cia_archive::zipfmt::{ZipWriter, METHOD_STORED};
use cia_archive::{zip_compress, ArchiveOptions};

use crate::package::{Entry, Package};
use crate::OfficeError;

/// One entry ready to be written: method, on-disk bytes, CRC and raw size.
#[derive(Debug, Clone)]
pub(crate) struct Part {
    pub method: u16,
    pub data: Vec<u8>,
    pub crc: u32,
    pub size: u64,
}

impl Part {
    /// Bytes this entry adds to the archive beyond the fixed per-entry
    /// headers.
    pub fn stored_len(&self) -> u64 {
        self.data.len() as u64
    }
}

/// The 3.9.2 settings: zopfli under 1 MiB, zlib-rs level 9 above.
pub(crate) fn exact_options() -> ArchiveOptions {
    ArchiveOptions::default()
}

/// Level 9 only, no zopfli. Used for media entries, where the question is
/// only whether deflate saves one percent (zopfli would spend seconds per
/// megabyte to learn that a JPEG does not deflate), and for
/// [`crate::inspect`]'s estimate, where speed matters more than the last
/// percent.
pub(crate) fn quick_options() -> ArchiveOptions {
    ArchiveOptions {
        zopfli_max_bytes: 0,
        ..ArchiveOptions::default()
    }
}

pub(crate) fn compress(bytes: &[u8], options: &ArchiveOptions) -> Part {
    let (method, data) = zip_compress(bytes, options);
    Part {
        method,
        data,
        crc: crc32(bytes),
        size: bytes.len() as u64,
    }
}

pub(crate) fn stored(bytes: &[u8]) -> Part {
    Part {
        method: METHOD_STORED,
        data: bytes.to_vec(),
        crc: crc32(bytes),
        size: bytes.len() as u64,
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

/// Exact container overhead for these entries (DESIGN.md 3.9.2 formula).
pub(crate) fn overhead_bytes(entries: &[Entry]) -> u64 {
    let largest = entries
        .iter()
        .map(|e| e.bytes.len() as u64)
        .max()
        .unwrap_or(0);
    cia_core::zip_overhead::zip_overhead_bytes(entries.iter().map(|e| e.name.as_str()), largest)
}

/// Write the archive. `parts` is indexed like `pkg.entries`; directory
/// entries need no part. The `mimetype` entry of an ODF/EPUB package is
/// always written first and stored, whatever its part says.
pub(crate) fn write_zip(pkg: &Package, parts: &[Option<Part>]) -> Result<Vec<u8>, OfficeError> {
    let mut zw = ZipWriter::new(Vec::new(), None);
    for i in pkg.output_order() {
        let e = &pkg.entries[i];
        if e.is_dir {
            zw.write_entry(&e.name, METHOD_STORED, 0, 0, &[], true)
                .map_err(|err| OfficeError::Corrupt(err.to_string()))?;
            continue;
        }
        let part = parts[i].as_ref().ok_or_else(|| {
            OfficeError::Internal(format!("no data prepared for entry {}", e.name))
        })?;
        if pkg.kind.has_mimetype() && e.name == "mimetype" {
            zw.write_entry(&e.name, METHOD_STORED, part.crc, part.size, &e.bytes, false)
        } else {
            zw.write_entry(&e.name, part.method, part.crc, part.size, &part.data, false)
        }
        .map_err(|err| OfficeError::Corrupt(err.to_string()))?;
    }
    zw.finish()
        .map_err(|err| OfficeError::Corrupt(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn compress_stores_incompressible() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let noise: Vec<u8> = (0..4096)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect();
        let p = compress(&noise, &exact_options());
        assert_eq!(p.method, METHOD_STORED);
        assert_eq!(p.stored_len(), 4096);
        let text = vec![b'a'; 4096];
        let p = compress(&text, &exact_options());
        assert_ne!(p.method, METHOD_STORED);
        assert!(p.stored_len() < 100);
    }
}
