//! Archive inputs (DESIGN.md 3.9.3) and verification (3.11, Archive row).

use std::borrow::Cow;
use std::io::{Cursor, Read};

use crate::{io_to_archive, ArchiveError};

/// Unpacked bytes [`open`] will decode before giving up with
/// [`ArchiveError::TooLarge`]. Applies to the whole decompressed tar/gz
/// payload and to each zip/7z entry.
pub const DEFAULT_UNPACK_LIMIT: u64 = 4 * 1024 * 1024 * 1024;

/// Container detected from magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    /// A gzip stream that is not a tar: one entry, named from the gzip
    /// header when it carries a name.
    Gz,
    /// A bzip2 stream that is not a tar: one entry.
    Bz2,
    /// An xz stream that is not a tar: one entry.
    Xz,
    /// A zstd frame that is not a tar: one entry.
    Zst,
}

impl ArchiveFormat {
    /// Conventional extension without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::SevenZip => "7z",
            ArchiveFormat::Tar => "tar",
            ArchiveFormat::TarGz => "tar.gz",
            ArchiveFormat::TarBz2 => "tar.bz2",
            ArchiveFormat::TarXz => "tar.xz",
            ArchiveFormat::TarZst => "tar.zst",
            ArchiveFormat::Gz => "gz",
            ArchiveFormat::Bz2 => "bz2",
            ArchiveFormat::Xz => "xz",
            ArchiveFormat::Zst => "zst",
        }
    }
}

/// One entry as listed, before extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// Name as stored (`/`-separated; directories end with `/`).
    pub name: String,
    /// Uncompressed size in bytes as declared by the archive.
    pub size: u64,
    pub is_dir: bool,
    /// Entry needs a password. Smidge never asks for one, so this entry
    /// cannot be extracted; the archive as a whole still lists fine.
    pub encrypted: bool,
    /// Compression method name, lowercase: `stored`, `deflate`, `lzma2`,
    /// `bzip2`, `aes`, `unsupported(93)`, the outer codec for tar members.
    pub method: String,
    /// False when this build cannot extract the entry (encrypted, or a
    /// method we do not ship a decoder for). Reported per entry rather than
    /// failing the archive.
    pub supported: bool,
}

/// Outcome of [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub format: ArchiveFormat,
    pub entry_count: usize,
    /// Sum of the bytes read back from every entry.
    pub unpacked_bytes: u64,
}

/// An opened archive. Borrows the input bytes; compressed tars are
/// decompressed once at open time.
pub struct Archive<'a> {
    format: ArchiveFormat,
    entries: Vec<ArchiveEntry>,
    backend: Backend<'a>,
    limit: u64,
}

enum Backend<'a> {
    Zip(zip::ZipArchive<Cursor<&'a [u8]>>),
    SevenZip {
        archive: Box<sevenz_rust2::Archive>,
        source: Cursor<&'a [u8]>,
    },
    Tar {
        data: Cow<'a, [u8]>,
        /// (offset, length) of each entry's data inside `data`.
        spans: Vec<(usize, usize)>,
    },
    Single(Vec<u8>),
}

enum Magic {
    Zip,
    SevenZip,
    Gz,
    Bz2,
    Xz,
    Zst,
    Rar,
    Tar,
}

fn sniff(bytes: &[u8]) -> Option<Magic> {
    if bytes.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Some(Magic::SevenZip);
    }
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        return Some(Magic::Zip);
    }
    if bytes.starts_with(b"Rar!\x1A\x07") {
        return Some(Magic::Rar);
    }
    if bytes.starts_with(&[0x1F, 0x8B]) {
        return Some(Magic::Gz);
    }
    if bytes.starts_with(b"BZh") && bytes.len() > 3 && bytes[3].is_ascii_digit() {
        return Some(Magic::Bz2);
    }
    if bytes.starts_with(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
        return Some(Magic::Xz);
    }
    if bytes.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return Some(Magic::Zst);
    }
    if looks_like_tar(bytes) {
        return Some(Magic::Tar);
    }
    None
}

/// A tar has no magic at offset 0; accept a `ustar` marker at 257 or a
/// first header block whose checksum field is right.
fn looks_like_tar(bytes: &[u8]) -> bool {
    if bytes.len() < 512 {
        return false;
    }
    if &bytes[257..262] == b"ustar" {
        return true;
    }
    let stored = parse_octal(&bytes[148..156]);
    let Some(stored) = stored else {
        return false;
    };
    let mut sum: u64 = 0;
    for (i, b) in bytes[..512].iter().enumerate() {
        sum += if (148..156).contains(&i) {
            b' ' as u64
        } else {
            *b as u64
        };
    }
    sum == stored
}

fn parse_octal(field: &[u8]) -> Option<u64> {
    let s: Vec<u8> = field
        .iter()
        .copied()
        .skip_while(|b| *b == b' ' || *b == 0)
        .take_while(|b| (b'0'..=b'7').contains(b))
        .collect();
    if s.is_empty() {
        return None;
    }
    let mut v = 0u64;
    for b in s {
        v = v.checked_mul(8)? + u64::from(b - b'0');
    }
    Some(v)
}

/// Read everything from `r` into memory, refusing to go past `limit`.
fn read_limited<R: Read>(mut r: R, limit: u64) -> Result<Vec<u8>, ArchiveError> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(io_to_archive)?;
        if n == 0 {
            break;
        }
        if out.len() as u64 + n as u64 > limit {
            return Err(ArchiveError::TooLarge { limit });
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// Open an archive held in memory, detecting the container from its magic
/// bytes. See [`Archive`]. RAR gives `ArchiveError::Unsupported("rar")`;
/// anything unrecognised gives [`ArchiveError::Unsupported`] too.
pub fn open(bytes: &[u8]) -> Result<Archive<'_>, ArchiveError> {
    open_with_limit(bytes, DEFAULT_UNPACK_LIMIT)
}

/// [`open`] with an explicit cap on unpacked bytes (per entry for zip and
/// 7z, for the whole payload of tar and single-stream inputs).
pub fn open_with_limit(bytes: &[u8], limit: u64) -> Result<Archive<'_>, ArchiveError> {
    match sniff(bytes) {
        Some(Magic::Rar) => Err(ArchiveError::Unsupported("rar".into())),
        None => Err(ArchiveError::Unsupported(
            "unrecognised archive format".into(),
        )),
        Some(Magic::Zip) => open_zip(bytes, limit),
        Some(Magic::SevenZip) => open_7z(bytes, limit),
        Some(Magic::Tar) => open_tar(Cow::Borrowed(bytes), ArchiveFormat::Tar, limit),
        Some(Magic::Gz) => {
            let mut dec = flate2::read::MultiGzDecoder::new(bytes);
            let data = read_limited(&mut dec, limit)?;
            let fname = dec
                .header()
                .and_then(|h| h.filename())
                .and_then(|f| std::str::from_utf8(f).ok())
                .map(str::to_owned);
            open_decoded(data, ArchiveFormat::TarGz, ArchiveFormat::Gz, limit, fname)
        }
        Some(Magic::Bz2) => {
            let data = read_limited(bzip2::read::MultiBzDecoder::new(bytes), limit)?;
            open_decoded(data, ArchiveFormat::TarBz2, ArchiveFormat::Bz2, limit, None)
        }
        Some(Magic::Xz) => {
            let data = xz_decode(bytes, limit)?;
            open_decoded(data, ArchiveFormat::TarXz, ArchiveFormat::Xz, limit, None)
        }
        Some(Magic::Zst) => {
            let mut dec = zstd::stream::read::Decoder::new(bytes)?;
            dec.window_log_max(27)?;
            let data = read_limited(dec, limit)?;
            open_decoded(data, ArchiveFormat::TarZst, ArchiveFormat::Zst, limit, None)
        }
    }
}

#[cfg(feature = "xz")]
fn xz_decode(bytes: &[u8], limit: u64) -> Result<Vec<u8>, ArchiveError> {
    read_limited(liblzma::read::XzDecoder::new_multi_decoder(bytes), limit)
}

#[cfg(not(feature = "xz"))]
fn xz_decode(_bytes: &[u8], _limit: u64) -> Result<Vec<u8>, ArchiveError> {
    Err(ArchiveError::Unsupported(
        "xz (built without the xz feature)".into(),
    ))
}

/// A decompressed single stream is either a tar or one file.
fn open_decoded(
    data: Vec<u8>,
    tar_format: ArchiveFormat,
    single_format: ArchiveFormat,
    limit: u64,
    name_hint: Option<String>,
) -> Result<Archive<'static>, ArchiveError> {
    if looks_like_tar(&data) {
        return open_tar(Cow::Owned(data), tar_format, limit);
    }
    let method = match single_format {
        ArchiveFormat::Gz => "gzip",
        ArchiveFormat::Bz2 => "bzip2",
        ArchiveFormat::Xz => "xz",
        _ => "zstd",
    };
    let name = name_hint
        .filter(|n| crate::check_entry_name(n).is_ok() && !n.ends_with('/'))
        .unwrap_or_else(|| "data".to_owned());
    Ok(Archive {
        format: single_format,
        entries: vec![ArchiveEntry {
            name,
            size: data.len() as u64,
            is_dir: false,
            encrypted: false,
            method: method.to_owned(),
            supported: true,
        }],
        backend: Backend::Single(data),
        limit,
    })
}

fn open_tar(
    data: Cow<'_, [u8]>,
    format: ArchiveFormat,
    limit: u64,
) -> Result<Archive<'_>, ArchiveError> {
    let method = match format {
        ArchiveFormat::TarGz => "gzip",
        ArchiveFormat::TarBz2 => "bzip2",
        ArchiveFormat::TarXz => "xz",
        ArchiveFormat::TarZst => "zstd",
        _ => "none",
    };
    let mut entries = Vec::new();
    let mut spans = Vec::new();
    {
        let mut ar = tar::Archive::new(Cursor::new(&data[..]));
        for entry in ar.entries().map_err(io_to_archive)? {
            let entry = entry.map_err(io_to_archive)?;
            let header = entry.header();
            let kind = header.entry_type();
            let mut name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
            let is_dir = kind.is_dir();
            if is_dir && !name.ends_with('/') {
                name.push('/');
            }
            let regular = kind.is_file() || kind == tar::EntryType::Continuous;
            let size = if regular { entry.size() } else { 0 };
            let start = entry.raw_file_position() as usize;
            if start.saturating_add(size as usize) > data.len() {
                return Err(ArchiveError::Corrupt(format!(
                    "tar entry {name} runs past the end"
                )));
            }
            let supported = regular || is_dir;
            entries.push(ArchiveEntry {
                name,
                size,
                is_dir,
                encrypted: false,
                method: if supported {
                    method.to_owned()
                } else {
                    format!("{kind:?}").to_lowercase()
                },
                supported,
            });
            spans.push((start, size as usize));
        }
    }
    Ok(Archive {
        format,
        entries,
        backend: Backend::Tar { data, spans },
        limit,
    })
}

fn zip_method_name(m: zip::CompressionMethod) -> (String, bool) {
    use zip::CompressionMethod as M;
    match m {
        M::Stored => ("stored".into(), true),
        M::Deflated => ("deflate".into(), true),
        other => {
            // Every method this build cannot decode comes back as the
            // `Unsupported(n)` variant; name it from the number.
            #[allow(deprecated)]
            let n = other.to_u16();
            let name = match M::name_from_u16(n) {
                "Unknown" => format!("unsupported({n})"),
                known => known.to_lowercase(),
            };
            (name, false)
        }
    }
}

fn open_zip(bytes: &[u8], limit: u64) -> Result<Archive<'_>, ArchiveError> {
    let mut za = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut entries = Vec::with_capacity(za.len());
    for i in 0..za.len() {
        let f = za.by_index_raw(i)?;
        let encrypted = f.encrypted();
        let (method, decodable) = zip_method_name(f.compression());
        let is_dir = f.is_dir();
        let mut name = f.name().to_owned();
        if is_dir && !name.ends_with('/') {
            name.push('/');
        }
        entries.push(ArchiveEntry {
            name,
            size: f.size(),
            is_dir,
            encrypted,
            method,
            supported: !encrypted && (decodable || is_dir),
        });
    }
    Ok(Archive {
        format: ArchiveFormat::Zip,
        entries,
        backend: Backend::Zip(za),
        limit,
    })
}

fn open_7z(bytes: &[u8], limit: u64) -> Result<Archive<'_>, ArchiveError> {
    use sevenz_rust2::EncoderMethod;
    let mut source = Cursor::new(bytes);
    let archive = sevenz_rust2::Archive::read(&mut source, &sevenz_rust2::Password::empty())
        .map_err(|e| match e {
            sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => {
                ArchiveError::Encrypted("7z header".into())
            }
            other => other.into(),
        })?;
    const DECODABLE: &[&[u8]] = &[
        EncoderMethod::ID_COPY,
        EncoderMethod::ID_LZMA,
        EncoderMethod::ID_LZMA2,
        EncoderMethod::ID_DELTA,
        EncoderMethod::ID_BCJ_X86,
        EncoderMethod::ID_BCJ_ARM,
        EncoderMethod::ID_BCJ_ARM64,
        EncoderMethod::ID_BCJ_ARM_THUMB,
        EncoderMethod::ID_BCJ_PPC,
        EncoderMethod::ID_BCJ_IA64,
        EncoderMethod::ID_BCJ_SPARC,
        EncoderMethod::ID_BCJ_RISCV,
    ];
    let mut entries = Vec::with_capacity(archive.files.len());
    for (i, f) in archive.files.iter().enumerate() {
        let mut name = f.name().replace('\\', "/");
        let is_dir = f.is_directory();
        if is_dir && !name.ends_with('/') {
            name.push('/');
        }
        let block = archive
            .stream_map
            .file_block_index
            .get(i)
            .copied()
            .flatten();
        let (method, encrypted, decodable) = match block.and_then(|b| archive.blocks.get(b)) {
            None => ("stored".to_owned(), false, true),
            Some(block) => {
                let mut names = Vec::new();
                let mut encrypted = false;
                let mut decodable = true;
                for coder in &block.coders {
                    let id = coder.encoder_method_id();
                    if id == EncoderMethod::ID_AES256_SHA256 {
                        encrypted = true;
                    }
                    if !DECODABLE.contains(&id) {
                        decodable = false;
                    }
                    names.push(
                        EncoderMethod::by_id(id)
                            .map(|m| m.name().to_lowercase())
                            .unwrap_or_else(|| format!("unsupported({id:02x?})")),
                    );
                }
                (names.join("+"), encrypted, decodable)
            }
        };
        entries.push(ArchiveEntry {
            name,
            size: f.size(),
            is_dir,
            encrypted,
            method,
            supported: !encrypted && decodable,
        });
    }
    Ok(Archive {
        format: ArchiveFormat::SevenZip,
        entries,
        backend: Backend::SevenZip {
            archive: Box::new(archive),
            source,
        },
        limit,
    })
}

impl std::fmt::Debug for Archive<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("format", &self.format)
            .field("entries", &self.entries)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl Archive<'_> {
    pub fn format(&self) -> ArchiveFormat {
        self.format
    }

    /// Entries in archive order.
    pub fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Checks shared by every backend before touching entry `index`.
    fn gate(&self, index: usize) -> Result<&ArchiveEntry, ArchiveError> {
        let e = self
            .entries
            .get(index)
            .ok_or(ArchiveError::NoSuchEntry(index))?;
        if e.encrypted {
            return Err(ArchiveError::Encrypted(e.name.clone()));
        }
        if !e.supported {
            return Err(ArchiveError::UnsupportedMethod {
                name: e.name.clone(),
                method: e.method.clone(),
            });
        }
        if e.size > self.limit {
            return Err(ArchiveError::TooLarge { limit: self.limit });
        }
        Ok(e)
    }

    /// Extract one entry fully, with its checksum verified where the format
    /// has one (zip CRC-32, 7z CRC-32; tar members rely on the outer
    /// stream's check, applied at open). Directories give an empty vector.
    pub fn extract(&mut self, index: usize) -> Result<Vec<u8>, ArchiveError> {
        let entry = self.gate(index)?.clone();
        if entry.is_dir {
            return Ok(Vec::new());
        }
        let limit = self.limit;
        match &mut self.backend {
            Backend::Zip(za) => {
                let f = za.by_index(index)?;
                let data = read_limited(f.take(limit + 1), limit)?;
                Ok(data)
            }
            Backend::SevenZip { archive, source } => {
                let mut out = None;
                sevenz_read_entries(
                    archive,
                    source,
                    limit,
                    |i, data| {
                        if i == index {
                            out = Some(data);
                            return false;
                        }
                        true
                    },
                    Some(index),
                )?;
                out.ok_or_else(|| ArchiveError::Corrupt(format!("7z entry {} missing", entry.name)))
            }
            Backend::Tar { data, spans } => {
                let (start, len) = spans[index];
                Ok(data[start..start + len].to_vec())
            }
            Backend::Single(data) => Ok(data.clone()),
        }
    }

    /// Visit every entry in order with its full contents (checksums
    /// verified). Directories and empty files visit with an empty slice.
    /// Stops at the first encrypted or unsupported entry with that error.
    pub fn for_each<F>(&mut self, mut f: F) -> Result<(), ArchiveError>
    where
        F: FnMut(usize, &ArchiveEntry, &[u8]) -> Result<(), ArchiveError>,
    {
        for i in 0..self.entries.len() {
            self.gate(i)?;
        }
        let limit = self.limit;
        match &mut self.backend {
            Backend::SevenZip { archive, source } => {
                // One decoding pass per block instead of one per entry.
                let entries = &self.entries;
                let mut pending: Result<(), ArchiveError> = Ok(());
                sevenz_read_entries(
                    archive,
                    source,
                    limit,
                    |i, data| match f(i, &entries[i], &data) {
                        Ok(()) => true,
                        Err(e) => {
                            pending = Err(e);
                            false
                        }
                    },
                    None,
                )?;
                pending
            }
            _ => {
                for i in 0..self.entries.len() {
                    let data = self.extract(i)?;
                    f(i, &self.entries[i], &data)?;
                }
                Ok(())
            }
        }
    }

    /// Extract every entry; the result is indexed like [`Archive::entries`].
    pub fn extract_all(&mut self) -> Result<Vec<Vec<u8>>, ArchiveError> {
        let mut out = vec![Vec::new(); self.entries.len()];
        self.for_each(|i, _, data| {
            out[i] = data.to_vec();
            Ok(())
        })?;
        Ok(out)
    }
}

/// Decode 7z blocks in order, calling `visit(file_index, data)` for each
/// file; `visit` returns false to stop. With `only`, decodes just the block
/// holding that file (and only up to it, for solid blocks).
fn sevenz_read_entries<R: Read + std::io::Seek>(
    archive: &sevenz_rust2::Archive,
    source: &mut R,
    limit: u64,
    mut visit: impl FnMut(usize, Vec<u8>) -> bool,
    only: Option<usize>,
) -> Result<(), ArchiveError> {
    let password = sevenz_rust2::Password::empty();
    let blocks: Vec<usize> = match only {
        Some(i) => match archive
            .stream_map
            .file_block_index
            .get(i)
            .copied()
            .flatten()
        {
            Some(b) => vec![b],
            None => {
                visit(i, Vec::new());
                return Ok(());
            }
        },
        None => (0..archive.blocks.len()).collect(),
    };
    let mut keep_going = true;
    for block_index in blocks {
        if !keep_going {
            break;
        }
        let start = archive.stream_map.block_first_file_index[block_index];
        let mut file_index = start;
        let dec = sevenz_rust2::BlockDecoder::new(1, block_index, archive, &password, source);
        let res = dec.for_each_entries(&mut |_entry, reader| {
            let i = file_index;
            file_index += 1;
            if only.is_some_and(|o| o != i) {
                // Must still consume the bytes so the solid stream stays in sync.
                std::io::copy(reader, &mut std::io::sink())?;
                return Ok(true);
            }
            let data = read_limited(reader.take(limit + 1), limit).map_err(|e| {
                // Tunnel our error through the sevenz error type; unwrapped in sevenz_error.
                sevenz_rust2::Error::Io(std::io::Error::other(e), "".into())
            })?;
            keep_going = visit(i, data);
            Ok(keep_going)
        });
        res.map_err(sevenz_error)?;
    }
    if only.is_none() && keep_going {
        // Files without a stream (empty files, directories) are not in any block.
        for (i, block) in archive.stream_map.file_block_index.iter().enumerate() {
            if block.is_none() && !visit(i, Vec::new()) {
                break;
            }
        }
    }
    Ok(())
}

fn sevenz_error(e: sevenz_rust2::Error) -> ArchiveError {
    match e {
        sevenz_rust2::Error::ChecksumVerificationFailed => {
            ArchiveError::Corrupt("7z checksum mismatch".into())
        }
        sevenz_rust2::Error::Io(e, _) => match e.downcast::<ArchiveError>() {
            Ok(ours) => ours,
            // The checksum reader surfaces its failure inside an io::Error.
            Err(e) if e.to_string().contains("ChecksumVerificationFailed") => {
                ArchiveError::Corrupt("7z checksum mismatch".into())
            }
            Err(e) => io_to_archive(e),
        },
        other => other.into(),
    }
}

/// Verification for the Archive row of DESIGN.md 3.11: re-open `bytes`,
/// read every entry fully (checksums verified by the decoders), and check
/// that the entry count and names match `expected_names` in order.
pub fn verify(bytes: &[u8], expected_names: &[String]) -> Result<VerifyReport, ArchiveError> {
    let mut archive = open(bytes)?;
    if archive.len() != expected_names.len() {
        return Err(ArchiveError::CountMismatch {
            expected: expected_names.len(),
            found: archive.len(),
        });
    }
    for (index, (entry, expected)) in archive.entries().iter().zip(expected_names).enumerate() {
        if &entry.name != expected {
            return Err(ArchiveError::NameMismatch {
                index,
                expected: expected.clone(),
                found: entry.name.clone(),
            });
        }
    }
    let mut unpacked_bytes = 0u64;
    archive.for_each(|_, entry, data| {
        if !entry.is_dir && data.len() as u64 != entry.size {
            return Err(ArchiveError::Corrupt(format!(
                "entry {} is {} bytes, header says {}",
                entry.name,
                data.len(),
                entry.size
            )));
        }
        unpacked_bytes += data.len() as u64;
        Ok(())
    })?;
    Ok(VerifyReport {
        format: archive.format(),
        entry_count: archive.len(),
        unpacked_bytes,
    })
}
