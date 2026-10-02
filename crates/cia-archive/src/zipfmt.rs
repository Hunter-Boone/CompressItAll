//! A small, exact ZIP container writer and the size predictor that goes with it.
//!
//! Layout per entry: 30-byte local header + name (+ 20-byte Zip64 extra only
//! when the entry itself is 4 GiB or larger), the data, and later a 46-byte
//! central directory entry + name (+ Zip64 extra holding only the fields that
//! overflow 32 bits). The archive ends with the 22-byte end record, preceded
//! by the Zip64 end record and locator (56 + 20 bytes) when anything
//! overflows. Nothing else is ever written: no data descriptors (sizes and
//! CRC are known before the local header goes out), no extended timestamps,
//! no Unix extra fields, no comments. That is what makes
//! [`zip_predicted_size`] exact for stored entries, which the engine relies
//! on (DESIGN.md 3.9.1, 3.9.2).
//!
//! Names are UTF-8 with general-purpose flag bit 11 set on every entry.
//! Timestamps default to 1980-01-01 00:00 (the earliest DOS time).

use std::io::Write;

use crate::ArchiveError;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const EOCD64_SIG: u32 = 0x0606_4b50;
const EOCD64_LOCATOR_SIG: u32 = 0x0706_4b50;

const FLAG_UTF8: u16 = 1 << 11;
const ZIP64_EXTRA_ID: u16 = 0x0001;
const U32_MAX: u64 = u32::MAX as u64;
/// A size needs Zip64 at exactly 4 GiB (0xFFFFFFFF is the escape value, so
/// anything that large or larger cannot be written in 32 bits).
const ZIP64_THRESHOLD: u64 = 0xFFFF_FFFF;

/// `version made by`: 3 = Unix (so the external attributes carry a mode),
/// 45 = 4.5 (Zip64 capable). Readers only use the high byte.
const VERSION_MADE_BY: u16 = (3 << 8) | 45;
const VERSION_NEEDED: u16 = 20;
const VERSION_NEEDED_ZIP64: u16 = 45;
/// Unix mode 0644 regular file, in the high 16 bits of the external attrs.
const EXTERNAL_ATTRS_FILE: u32 = 0o100644 << 16;
/// Unix mode 0755 directory plus the MS-DOS directory bit.
const EXTERNAL_ATTRS_DIR: u32 = (0o040755 << 16) | 0x10;

/// Compression method numbers (APPNOTE 4.4.5).
pub const METHOD_STORED: u16 = 0;
pub const METHOD_DEFLATE: u16 = 8;

/// Sizes of the fixed parts, public so tests can cross-check the predictor.
pub const LOCAL_HEADER_LEN: u64 = 30;
pub const CENTRAL_HEADER_LEN: u64 = 46;
pub const EOCD_LEN: u64 = 22;
pub const EOCD64_LEN: u64 = 56;
pub const EOCD64_LOCATOR_LEN: u64 = 20;
pub const LOCAL_ZIP64_EXTRA_LEN: u64 = 4 + 16;

/// Predicted size in bytes of a ZIP written by [`ZipWriter`] in which every
/// entry is stored (method 0). Entries are `(name, uncompressed_size)`.
///
/// Below 4 GiB per entry and per archive this equals
/// `sum(sizes) + cia_core::zip_overhead::zip_overhead_bytes(names, largest)`,
/// byte for byte; the test suite proves it against real output. For deflated
/// entries the real archive is smaller by the bytes compression saved, never
/// larger, because stored is chosen whenever deflate does not pay.
///
/// Above 4 GiB the layout needs Zip64 fields whose count depends on each
/// entry's offset, which this function simulates exactly for the stored
/// case; `zip_overhead_bytes` approximates it with a flat 20 bytes per entry
/// and can under-count there.
pub fn zip_predicted_size(entries: &[(&str, u64)]) -> u64 {
    let mut offset = 0u64;
    let mut central = 0u64;
    for (name, size) in entries {
        let n = name.len() as u64;
        let start = offset;
        offset += LOCAL_HEADER_LEN + n + local_extra_len(*size) + size;
        central += CENTRAL_HEADER_LEN + n + central_extra_len(*size, *size, start);
    }
    let cd_offset = offset;
    let mut total = cd_offset + central + EOCD_LEN;
    if needs_eocd64(entries.len() as u64, central, cd_offset) {
        total += EOCD64_LEN + EOCD64_LOCATOR_LEN;
    }
    total
}

fn local_extra_len(size: u64) -> u64 {
    if size >= ZIP64_THRESHOLD {
        LOCAL_ZIP64_EXTRA_LEN
    } else {
        0
    }
}

fn central_extra_len(size: u64, compressed: u64, offset: u64) -> u64 {
    let fields = u64::from(size >= ZIP64_THRESHOLD)
        + u64::from(compressed >= ZIP64_THRESHOLD)
        + u64::from(offset >= ZIP64_THRESHOLD);
    if fields == 0 {
        0
    } else {
        4 + 8 * fields
    }
}

fn needs_eocd64(count: u64, cd_size: u64, cd_offset: u64) -> bool {
    count >= 0xFFFF || cd_size >= ZIP64_THRESHOLD || cd_offset >= ZIP64_THRESHOLD
}

/// DOS date/time pair for a Unix timestamp; clamps to the DOS range
/// (1980-01-01 .. 2107-12-31) and returns 1980-01-01 00:00 for `None`.
pub fn dos_datetime(mtime_unix: Option<u64>) -> (u16, u16) {
    const DOS_EPOCH: u64 = 315_532_800; // 1980-01-01T00:00:00Z
    let Some(t) = mtime_unix else {
        return (0, 0x0021);
    };
    let t = t.max(DOS_EPOCH);
    let days = t / 86_400;
    let secs = t % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    let year = (y - 1980).clamp(0, 127) as u16;
    let date = (year << 9) | ((m as u16) << 5) | d as u16;
    let time =
        ((secs / 3600) as u16) << 11 | (((secs % 3600) / 60) as u16) << 5 | (secs % 60 / 2) as u16;
    (time, date)
}

/// Howard Hinnant's days-to-civil algorithm (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

struct Record {
    name: Vec<u8>,
    method: u16,
    crc: u32,
    size: u64,
    compressed: u64,
    offset: u64,
    is_dir: bool,
}

/// Streaming ZIP container writer. Caller supplies already-compressed data
/// plus its CRC; the writer never seeks, so any [`Write`] works.
pub struct ZipWriter<W: Write> {
    out: W,
    offset: u64,
    records: Vec<Record>,
    dos_time: u16,
    dos_date: u16,
}

impl<W: Write> ZipWriter<W> {
    pub fn new(out: W, mtime_unix: Option<u64>) -> Self {
        let (dos_time, dos_date) = dos_datetime(mtime_unix);
        Self {
            out,
            offset: 0,
            records: Vec::new(),
            dos_time,
            dos_date,
        }
    }

    /// Write one entry. `data` is the on-disk bytes for `method`; `size` and
    /// `crc` describe the uncompressed content.
    pub fn write_entry(
        &mut self,
        name: &str,
        method: u16,
        crc: u32,
        size: u64,
        data: &[u8],
        is_dir: bool,
    ) -> Result<(), ArchiveError> {
        crate::check_entry_name(name)?;
        let compressed = data.len() as u64;
        let zip64 = size >= ZIP64_THRESHOLD || compressed >= ZIP64_THRESHOLD;
        let name_bytes = name.as_bytes();
        let extra_len: u16 = if zip64 {
            LOCAL_ZIP64_EXTRA_LEN as u16
        } else {
            0
        };

        let mut h =
            Vec::with_capacity(LOCAL_HEADER_LEN as usize + name_bytes.len() + extra_len as usize);
        put32(&mut h, LOCAL_SIG);
        put16(
            &mut h,
            if zip64 {
                VERSION_NEEDED_ZIP64
            } else {
                VERSION_NEEDED
            },
        );
        put16(&mut h, FLAG_UTF8);
        put16(&mut h, method);
        put16(&mut h, self.dos_time);
        put16(&mut h, self.dos_date);
        put32(&mut h, crc);
        put32(&mut h, clamp32(compressed));
        put32(&mut h, clamp32(size));
        put16(&mut h, name_bytes.len() as u16);
        put16(&mut h, extra_len);
        h.extend_from_slice(name_bytes);
        if zip64 {
            put16(&mut h, ZIP64_EXTRA_ID);
            put16(&mut h, 16);
            put64(&mut h, size);
            put64(&mut h, compressed);
        }
        self.out.write_all(&h)?;
        self.out.write_all(data)?;

        self.records.push(Record {
            name: name_bytes.to_vec(),
            method,
            crc,
            size,
            compressed,
            offset: self.offset,
            is_dir,
        });
        self.offset += h.len() as u64 + compressed;
        Ok(())
    }

    /// Write the central directory and end records; returns the sink.
    pub fn finish(mut self) -> Result<W, ArchiveError> {
        let cd_offset = self.offset;
        let mut cd = Vec::new();
        for r in &self.records {
            let zip64_size = r.size >= ZIP64_THRESHOLD;
            let zip64_comp = r.compressed >= ZIP64_THRESHOLD;
            let zip64_off = r.offset >= ZIP64_THRESHOLD;
            let extra_len = central_extra_len(r.size, r.compressed, r.offset) as u16;
            let any64 = extra_len > 0;
            put32(&mut cd, CENTRAL_SIG);
            put16(&mut cd, VERSION_MADE_BY);
            put16(
                &mut cd,
                if any64 {
                    VERSION_NEEDED_ZIP64
                } else {
                    VERSION_NEEDED
                },
            );
            put16(&mut cd, FLAG_UTF8);
            put16(&mut cd, r.method);
            put16(&mut cd, self.dos_time);
            put16(&mut cd, self.dos_date);
            put32(&mut cd, r.crc);
            put32(&mut cd, clamp32(r.compressed));
            put32(&mut cd, clamp32(r.size));
            put16(&mut cd, r.name.len() as u16);
            put16(&mut cd, extra_len);
            put16(&mut cd, 0); // comment length
            put16(&mut cd, 0); // disk number start
            put16(&mut cd, 0); // internal attributes
            put32(
                &mut cd,
                if r.is_dir {
                    EXTERNAL_ATTRS_DIR
                } else {
                    EXTERNAL_ATTRS_FILE
                },
            );
            put32(&mut cd, clamp32(r.offset));
            cd.extend_from_slice(&r.name);
            if any64 {
                put16(&mut cd, ZIP64_EXTRA_ID);
                put16(&mut cd, extra_len - 4);
                if zip64_size {
                    put64(&mut cd, r.size);
                }
                if zip64_comp {
                    put64(&mut cd, r.compressed);
                }
                if zip64_off {
                    put64(&mut cd, r.offset);
                }
            }
        }
        let cd_size = cd.len() as u64;
        let count = self.records.len() as u64;
        self.out.write_all(&cd)?;

        if needs_eocd64(count, cd_size, cd_offset) {
            let eocd64_offset = cd_offset + cd_size;
            let mut e = Vec::with_capacity((EOCD64_LEN + EOCD64_LOCATOR_LEN) as usize);
            put32(&mut e, EOCD64_SIG);
            put64(&mut e, EOCD64_LEN - 12); // size of the rest of this record
            put16(&mut e, VERSION_MADE_BY);
            put16(&mut e, VERSION_NEEDED_ZIP64);
            put32(&mut e, 0); // this disk
            put32(&mut e, 0); // disk with the central directory
            put64(&mut e, count);
            put64(&mut e, count);
            put64(&mut e, cd_size);
            put64(&mut e, cd_offset);
            put32(&mut e, EOCD64_LOCATOR_SIG);
            put32(&mut e, 0);
            put64(&mut e, eocd64_offset);
            put32(&mut e, 1);
            self.out.write_all(&e)?;
        }

        let mut e = Vec::with_capacity(EOCD_LEN as usize);
        put32(&mut e, EOCD_SIG);
        put16(&mut e, 0);
        put16(&mut e, 0);
        put16(&mut e, count.min(0xFFFF) as u16);
        put16(&mut e, count.min(0xFFFF) as u16);
        put32(&mut e, clamp32(cd_size));
        put32(&mut e, clamp32(cd_offset));
        put16(&mut e, 0);
        self.out.write_all(&e)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn clamp32(v: u64) -> u32 {
    v.min(U32_MAX) as u32
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn put64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_epoch_default() {
        assert_eq!(dos_datetime(None), (0, 0x0021));
    }

    #[test]
    fn dos_known_date() {
        // 2024-03-05 13:45:30 UTC
        let (time, date) = dos_datetime(Some(1_709_646_330));
        assert_eq!(date >> 9, 2024 - 1980);
        assert_eq!((date >> 5) & 0xF, 3);
        assert_eq!(date & 0x1F, 5);
        assert_eq!(time >> 11, 13);
        assert_eq!((time >> 5) & 0x3F, 45);
        assert_eq!(time & 0x1F, 15);
    }

    #[test]
    fn predictor_matches_core_formula_below_4gib() {
        let entries = [
            ("a.jpg", 1000u64),
            ("誕生日 🎂.txt", 0),
            ("dir/x.bin", 123_456),
        ];
        let names = entries.iter().map(|e| e.0);
        let data: u64 = entries.iter().map(|e| e.1).sum();
        let largest = entries.iter().map(|e| e.1).max().unwrap();
        assert_eq!(
            zip_predicted_size(&entries),
            data + cia_core::zip_overhead::zip_overhead_bytes(names, largest)
        );
        assert_eq!(zip_predicted_size(&[]), EOCD_LEN);
    }

    #[test]
    fn predictor_zip64_single_huge_entry() {
        let five_gib = 5 * 1024 * 1024 * 1024;
        let n = 1;
        // local header + name + 20 extra + data; central + name + (4 + 8 + 8); eocd64 + locator + eocd
        let expect = LOCAL_HEADER_LEN
            + n
            + LOCAL_ZIP64_EXTRA_LEN
            + five_gib
            + CENTRAL_HEADER_LEN
            + n
            + 20
            + EOCD64_LEN
            + EOCD64_LOCATOR_LEN
            + EOCD_LEN;
        assert_eq!(zip_predicted_size(&[("a", five_gib)]), expect);
    }
}
