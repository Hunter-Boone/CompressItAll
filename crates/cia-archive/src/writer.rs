//! [`ArchiveWriter`]: one API over the four output containers.

use std::io::{Cursor, Write};

use crate::zipfmt::{ZipWriter, METHOD_DEFLATE, METHOD_STORED};
use crate::{check_entry_name, ArchiveError, ArchiveKind, ArchiveOptions};

/// Deflate must save at least this fraction of the input or the entry is
/// stored (DESIGN.md 3.9.2: "Stored for entries that shrink less than 1%").
const MIN_DEFLATE_SAVING: f64 = 0.01;

/// Builds an archive entry by entry. `W` is the sink; use
/// [`ArchiveWriter::in_memory`] to get a `Vec<u8>` back from
/// [`ArchiveWriter::finish`], or [`ArchiveWriter::new`] to stream into any
/// [`Write`] (a file, a channel, an OPFS handle on the web).
///
/// 7z is the one container that needs the whole archive in memory before the
/// first byte can go to the sink: its header sits at the end but is pointed
/// to from the start, and one solid block needs every entry up front.
pub struct ArchiveWriter<W: Write = Vec<u8>> {
    inner: Inner<W>,
    options: ArchiveOptions,
    count: usize,
}

enum Inner<W: Write> {
    Zip(ZipWriter<W>),
    SevenZip {
        out: W,
        entries: Vec<(sevenz_rust2::ArchiveEntry, Vec<u8>)>,
    },
    TarZst(tar::Builder<zstd::stream::write::Encoder<'static, W>>),
    #[cfg(feature = "xz")]
    TarXz(tar::Builder<liblzma::write::XzEncoder<W>>),
}

impl ArchiveWriter<Vec<u8>> {
    /// Write into memory; [`ArchiveWriter::finish`] returns the bytes.
    pub fn in_memory(kind: ArchiveKind, options: ArchiveOptions) -> Result<Self, ArchiveError> {
        Self::new(kind, options, Vec::new())
    }
}

impl<W: Write> ArchiveWriter<W> {
    /// Stream into `out`. Returns [`ArchiveError::Unsupported`] for
    /// [`ArchiveKind::TarXz`] when the crate is built without `xz`.
    pub fn new(kind: ArchiveKind, options: ArchiveOptions, out: W) -> Result<Self, ArchiveError> {
        let inner = match kind {
            ArchiveKind::Zip => Inner::Zip(ZipWriter::new(out, options.mtime_unix)),
            ArchiveKind::SevenZip => Inner::SevenZip {
                out,
                entries: Vec::new(),
            },
            ArchiveKind::TarZst => {
                let mut enc = zstd::stream::write::Encoder::new(out, options.zstd_level)?;
                enc.long_distance_matching(true)?;
                enc.window_log(options.zstd_window_log)?;
                enc.include_checksum(true)?;
                Inner::TarZst(tar_builder(enc))
            }
            #[cfg(feature = "xz")]
            ArchiveKind::TarXz => {
                let stream = liblzma::stream::Stream::new_easy_encoder(
                    9 | liblzma::stream::PRESET_EXTREME,
                    liblzma::stream::Check::Crc64,
                )
                .map_err(|e| ArchiveError::Io(std::io::Error::other(e)))?;
                Inner::TarXz(tar_builder(liblzma::write::XzEncoder::new_stream(
                    out, stream,
                )))
            }
            #[cfg(not(feature = "xz"))]
            ArchiveKind::TarXz => {
                return Err(ArchiveError::Unsupported(
                    "tar.xz (built without the xz feature)".into(),
                ))
            }
        };
        Ok(Self {
            inner,
            options,
            count: 0,
        })
    }

    /// Number of entries added so far.
    pub fn len(&self) -> usize {
        self.count
    }

    /// True when nothing has been added yet.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Add a regular file. `name` is a `/`-separated UTF-8 path relative to
    /// the archive root; order of calls is the order in the archive.
    pub fn add_entry(&mut self, name: &str, bytes: &[u8]) -> Result<(), ArchiveError> {
        check_entry_name(name)?;
        if name.ends_with('/') {
            return Err(ArchiveError::InvalidName(name.to_owned()));
        }
        let mtime = self.options.mtime_unix.unwrap_or(0);
        match &mut self.inner {
            Inner::Zip(z) => {
                let crc = crc32fast::hash(bytes);
                let (method, data) = zip_compress(bytes, &self.options);
                z.write_entry(name, method, crc, bytes.len() as u64, &data, false)?;
            }
            Inner::SevenZip { entries, .. } => {
                let mut e = sevenz_rust2::ArchiveEntry::new_file(name);
                set_7z_mtime(&mut e, self.options.mtime_unix);
                e.size = bytes.len() as u64;
                entries.push((e, bytes.to_vec()));
            }
            Inner::TarZst(b) => tar_add_file(b, name, bytes, mtime)?,
            #[cfg(feature = "xz")]
            Inner::TarXz(b) => tar_add_file(b, name, bytes, mtime)?,
        }
        self.count += 1;
        Ok(())
    }

    /// Add a regular file that must be stored uncompressed (method 0) in a
    /// zip, whatever deflate would have saved. ODF and EPUB packages need
    /// this for their `mimetype` entry, which readers locate by offset. The
    /// other containers have no per-entry method, so this behaves like
    /// [`ArchiveWriter::add_entry`] for them.
    pub fn add_entry_stored(&mut self, name: &str, bytes: &[u8]) -> Result<(), ArchiveError> {
        check_entry_name(name)?;
        if name.ends_with('/') {
            return Err(ArchiveError::InvalidName(name.to_owned()));
        }
        match &mut self.inner {
            Inner::Zip(z) => {
                let crc = crc32fast::hash(bytes);
                z.write_entry(name, METHOD_STORED, crc, bytes.len() as u64, bytes, false)?;
                self.count += 1;
                Ok(())
            }
            _ => self.add_entry(name, bytes),
        }
    }

    /// Add an explicit directory entry. A trailing `/` is added if missing.
    /// Not needed for files inside directories; use it to keep empty folders
    /// when repacking an archive.
    pub fn add_dir(&mut self, name: &str) -> Result<(), ArchiveError> {
        let trimmed = name.trim_end_matches('/');
        check_entry_name(trimmed)?;
        let with_slash = format!("{trimmed}/");
        let mtime = self.options.mtime_unix.unwrap_or(0);
        match &mut self.inner {
            Inner::Zip(z) => z.write_entry(&with_slash, METHOD_STORED, 0, 0, &[], true)?,
            Inner::SevenZip { entries, .. } => {
                let mut e = sevenz_rust2::ArchiveEntry::new_directory(trimmed);
                set_7z_mtime(&mut e, self.options.mtime_unix);
                entries.push((e, Vec::new()));
            }
            Inner::TarZst(b) => tar_add_dir(b, &with_slash, mtime)?,
            #[cfg(feature = "xz")]
            Inner::TarXz(b) => tar_add_dir(b, &with_slash, mtime)?,
        }
        self.count += 1;
        Ok(())
    }

    /// Write the trailer, flush, and hand back the sink (the `Vec<u8>` for
    /// [`ArchiveWriter::in_memory`]).
    pub fn finish(self) -> Result<W, ArchiveError> {
        match self.inner {
            Inner::Zip(z) => z.finish(),
            Inner::SevenZip { mut out, entries } => {
                let bytes = write_7z(entries, self.options.sevenz_dict_bytes)?;
                out.write_all(&bytes)?;
                out.flush()?;
                Ok(out)
            }
            Inner::TarZst(b) => {
                let enc = b.into_inner()?;
                let mut out = enc.finish()?;
                out.flush()?;
                Ok(out)
            }
            #[cfg(feature = "xz")]
            Inner::TarXz(b) => {
                let enc = b.into_inner()?;
                let mut out = enc.finish()?;
                out.flush()?;
                Ok(out)
            }
        }
    }
}

/// Pick the on-disk method and bytes for one zip entry: zopfli below the
/// threshold, zlib-rs level 9 above it, stored when deflate saves under 1%
/// (DESIGN.md 3.9.2). Returns `(method, data)` ready for
/// [`crate::zipfmt::ZipWriter::write_entry`]; callers that need to measure
/// an entry before deciding the archive layout (the Office planner's fixed
/// bytes, DESIGN.md 3.8 step 3) can compress once and write the result.
pub fn zip_compress(bytes: &[u8], options: &ArchiveOptions) -> (u16, Vec<u8>) {
    if bytes.is_empty() {
        return (METHOD_STORED, Vec::new());
    }
    let deflated = deflate_bytes(bytes, options);
    let saving = 1.0 - deflated.len() as f64 / bytes.len() as f64;
    if saving < MIN_DEFLATE_SAVING {
        (METHOD_STORED, bytes.to_vec())
    } else {
        (METHOD_DEFLATE, deflated)
    }
}

/// Raw deflate stream for `bytes` using the encoder the size calls for.
pub(crate) fn deflate_bytes(bytes: &[u8], options: &ArchiveOptions) -> Vec<u8> {
    if (bytes.len() as u64) < options.zopfli_max_bytes {
        let mut out = Vec::with_capacity(bytes.len() / 2);
        let opts = zopfli::Options {
            iteration_count: std::num::NonZeroU64::new(options.zopfli_iterations.max(1))
                .expect("non-zero"),
            ..zopfli::Options::default()
        };
        if zopfli::compress(opts, zopfli::Format::Deflate, bytes, &mut out).is_ok() {
            return out;
        }
        // zopfli only fails on sink errors, which a Vec cannot produce; fall through anyway.
    }
    let mut enc = flate2::write::DeflateEncoder::new(
        Vec::with_capacity(bytes.len() / 2),
        flate2::Compression::new(9),
    );
    // Writing to a Vec cannot fail.
    let _ = enc.write_all(bytes);
    enc.finish().unwrap_or_default()
}

fn write_7z(
    entries: Vec<(sevenz_rust2::ArchiveEntry, Vec<u8>)>,
    dict_bytes: u32,
) -> Result<Vec<u8>, ArchiveError> {
    use sevenz_rust2::encoder_options::{EncoderOptions, Lzma2Options};
    use sevenz_rust2::{EncoderConfiguration, EncoderMethod, SourceReader};

    let mut w = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new()))?;
    let mut lzma2 = Lzma2Options::from_level(9);
    lzma2.set_dictionary_size(dict_bytes);
    w.set_content_methods(vec![
        EncoderConfiguration::new(EncoderMethod::LZMA2).with_options(EncoderOptions::Lzma2(lzma2))
    ]);

    // Entries without a stream (directories, empty files) cannot sit inside a
    // solid block, and the writer appends entries in push order. To keep the
    // caller's order exactly, each run of non-empty files becomes one solid
    // block and stream-less entries are pushed between runs.
    let mut run_entries = Vec::new();
    let mut run_readers = Vec::new();
    let flush_run = |w: &mut sevenz_rust2::ArchiveWriter<Cursor<Vec<u8>>>,
                     entries: &mut Vec<sevenz_rust2::ArchiveEntry>,
                     readers: &mut Vec<SourceReader<Cursor<Vec<u8>>>>|
     -> Result<(), ArchiveError> {
        if !entries.is_empty() {
            w.push_archive_entries(std::mem::take(entries), std::mem::take(readers))?;
        }
        Ok(())
    };
    for (entry, data) in entries {
        if entry.is_directory() || data.is_empty() {
            flush_run(&mut w, &mut run_entries, &mut run_readers)?;
            let mut entry = entry;
            entry.has_stream = false;
            entry.size = 0;
            w.push_archive_entry::<Cursor<Vec<u8>>>(entry, None)?;
        } else {
            run_entries.push(entry);
            run_readers.push(SourceReader::new(Cursor::new(data)));
        }
    }
    flush_run(&mut w, &mut run_entries, &mut run_readers)?;
    let cursor = w.finish()?;
    Ok(cursor.into_inner())
}

fn set_7z_mtime(e: &mut sevenz_rust2::ArchiveEntry, mtime_unix: Option<u64>) {
    if let Some(t) = mtime_unix {
        // NtTime counts 100 ns ticks since 1601-01-01.
        const UNIX_TO_NT_SECS: u64 = 11_644_473_600;
        let ticks = (t + UNIX_TO_NT_SECS).saturating_mul(10_000_000);
        e.has_last_modified_date = true;
        e.last_modified_date = sevenz_rust2::NtTime::new(ticks);
    }
}

fn tar_builder<W: Write>(w: W) -> tar::Builder<W> {
    let mut b = tar::Builder::new(w);
    b.mode(tar::HeaderMode::Deterministic);
    b
}

fn tar_add_file<W: Write>(
    b: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
    mtime: u64,
) -> Result<(), ArchiveError> {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Regular);
    h.set_mode(0o644);
    h.set_mtime(mtime);
    h.set_size(bytes.len() as u64);
    b.append_data(&mut h, name, bytes)?;
    Ok(())
}

fn tar_add_dir<W: Write>(
    b: &mut tar::Builder<W>,
    name: &str,
    mtime: u64,
) -> Result<(), ArchiveError> {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Directory);
    h.set_mode(0o755);
    h.set_mtime(mtime);
    h.set_size(0);
    b.append_data(&mut h, name, std::io::empty())?;
    Ok(())
}
