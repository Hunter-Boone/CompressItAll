#![forbid(unsafe_code)]
//! Smidge archive writers and readers (DESIGN.md 3.9.2, 3.9.3, 3.11).
//!
//! Bytes in, bytes out. Nothing here knows about jobs, items or presets: the
//! engine hands this crate entry names and byte slices, gets an archive back
//! (or streams it into any [`std::io::Write`]), and can re-open and verify
//! what it wrote. No threads, no `std::fs` in the core API, so the same code
//! runs natively and in the WASM engine. Filesystem helpers live behind the
//! `fs` cargo feature.
//!
//! # Writers
//!
//! [`ArchiveWriter`] produces one of [`ArchiveKind::Zip`], [`ArchiveKind::SevenZip`],
//! [`ArchiveKind::TarZst`] or [`ArchiveKind::TarXz`] (the last one behind the `xz`
//! feature, on by default).
//!
//! The ZIP container is written by this crate's own small writer
//! ([`zipfmt`]) rather than by the `zip` crate. Two reasons: the engine
//! predicts the archive size before encoding and that prediction must be
//! exact for stored entries (so every byte of the container has to be under
//! our control: no extended-timestamp or Unix extra fields, no data
//! descriptors), and the stored-vs-deflate decision needs the compressed
//! bytes before the local header is written, which the `zip` crate's
//! streaming writer cannot do without compressing twice. Deflate comes from
//! `flate2` (pure-Rust zlib-rs backend) and `zopfli`; the `zip` crate is used
//! for reading.
//!
//! # Readers
//!
//! [`open`] detects zip, 7z, tar, tar.gz, tar.bz2, tar.xz, tar.zst and plain
//! gz/bz2/xz/zst by magic bytes, lists entries and extracts them. Encrypted
//! entries and unsupported methods are reported per entry (see
//! [`ArchiveEntry::encrypted`] and [`ArchiveEntry::supported`]); only RAR
//! fails the whole archive, with [`ArchiveError::Unsupported`].
//!
//! [`verify`] implements the Archive row of the verification table in
//! DESIGN.md 3.11: re-open, read every entry fully with CRCs checked, and
//! compare entry count and names in order.

use std::fmt;

pub mod reader;
pub mod writer;
pub mod zipfmt;

#[cfg(feature = "fs")]
pub mod fs;

pub use reader::{
    open, open_with_limit, verify, Archive, ArchiveEntry, ArchiveFormat, VerifyReport,
};
pub use writer::{zip_compress, ArchiveWriter};
pub use zipfmt::zip_predicted_size;

/// Output container formats (DESIGN.md 3.9.2 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveKind {
    /// Deflate (level 9, zopfli under 1 MB) or Stored. Opens everywhere.
    Zip,
    /// LZMA2 preset 9, one solid block.
    SevenZip,
    /// POSIX tar inside a zstd frame (level 19, long mode, window log 27).
    TarZst,
    /// POSIX tar inside an xz stream (preset 9e). Needs the `xz` feature.
    TarXz,
}

impl ArchiveKind {
    /// File extension without the leading dot.
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveKind::Zip => "zip",
            ArchiveKind::SevenZip => "7z",
            ArchiveKind::TarZst => "tar.zst",
            ArchiveKind::TarXz => "tar.xz",
        }
    }
}

impl fmt::Display for ArchiveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// 7z LZMA2 dictionary used on desktop (DESIGN.md 3.9.2).
pub const SEVENZ_DICT_NATIVE: u32 = 64 * 1024 * 1024;
/// 7z LZMA2 dictionary used in the web engine (DESIGN.md 3.9.2).
pub const SEVENZ_DICT_WEB: u32 = 16 * 1024 * 1024;
/// Entries smaller than this are deflated with zopfli instead of zlib-rs.
pub const ZOPFLI_MAX_BYTES: u64 = 1024 * 1024;

/// Tuning knobs for [`ArchiveWriter`]. [`ArchiveOptions::default`] gives the
/// DESIGN.md 3.9.2 settings for the current target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveOptions {
    /// LZMA2 dictionary size in bytes for 7z. Default: 64 MiB natively,
    /// 16 MiB on wasm32.
    pub sevenz_dict_bytes: u32,
    /// Zip entries strictly smaller than this many bytes go through zopfli.
    /// Default 1 MiB. Set to 0 to disable zopfli.
    pub zopfli_max_bytes: u64,
    /// Zopfli iteration count (default 15, the zopfli default).
    pub zopfli_iterations: u64,
    /// zstd compression level for tar.zst. Default 19.
    pub zstd_level: i32,
    /// zstd window log for tar.zst (long-distance matching). Default 27.
    pub zstd_window_log: u32,
    /// Modification time written into entries, seconds since the Unix epoch.
    /// `None` writes the earliest representable time (1980-01-01 for zip,
    /// 1970-01-01 for tar/7z) so output is deterministic and the writer never
    /// needs a clock (there is none on wasm32-unknown-unknown).
    pub mtime_unix: Option<u64>,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            #[cfg(target_arch = "wasm32")]
            sevenz_dict_bytes: SEVENZ_DICT_WEB,
            #[cfg(not(target_arch = "wasm32"))]
            sevenz_dict_bytes: SEVENZ_DICT_NATIVE,
            zopfli_max_bytes: ZOPFLI_MAX_BYTES,
            zopfli_iterations: 15,
            zstd_level: 19,
            zstd_window_log: 27,
            mtime_unix: None,
        }
    }
}

/// Everything that can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// The container format (or a feature of it) is not supported at all,
    /// e.g. `"rar"`, `"xz (feature disabled)"`.
    #[error("unsupported archive: {0}")]
    Unsupported(String),
    /// The named entry is encrypted; Smidge does not take passwords.
    #[error("entry is encrypted: {0}")]
    Encrypted(String),
    /// The named entry uses a compression method this build cannot decode.
    #[error("entry {name} uses unsupported method {method}")]
    UnsupportedMethod { name: String, method: String },
    /// The bytes are not a valid archive, or a CRC/checksum failed.
    #[error("corrupt archive: {0}")]
    Corrupt(String),
    /// Unpacked data would exceed the configured limit.
    #[error("unpacked size exceeds {limit} bytes")]
    TooLarge { limit: u64 },
    /// Index out of range in [`Archive::extract`].
    #[error("no entry at index {0}")]
    NoSuchEntry(usize),
    /// Entry name rejected by the writer (empty, NUL, absolute, `..`).
    #[error("invalid entry name: {0}")]
    InvalidName(String),
    /// Verification: the archive holds a different number of entries.
    #[error("entry count mismatch: expected {expected}, found {found}")]
    CountMismatch { expected: usize, found: usize },
    /// Verification: entry `index` has a different name than planned.
    #[error("entry {index} is named {found:?}, expected {expected:?}")]
    NameMismatch {
        index: usize,
        expected: String,
        found: String,
    },
    /// Underlying I/O or codec error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<zip::result::ZipError> for ArchiveError {
    fn from(e: zip::result::ZipError) -> Self {
        use zip::result::ZipError as Z;
        match e {
            Z::Io(e) => ArchiveError::Io(e),
            Z::InvalidArchive(m) => ArchiveError::Corrupt(m.into_owned()),
            Z::UnsupportedArchive(m) => ArchiveError::Unsupported(m.to_string()),
            Z::CompressionMethodNotSupported(m) => ArchiveError::Unsupported(format!("method {m}")),
            Z::FileNotFound => ArchiveError::Corrupt("entry not found".into()),
            Z::InvalidPassword => ArchiveError::Encrypted(String::new()),
            other => ArchiveError::Corrupt(other.to_string()),
        }
    }
}

impl From<sevenz_rust2::Error> for ArchiveError {
    fn from(e: sevenz_rust2::Error) -> Self {
        use sevenz_rust2::Error as S;
        match e {
            S::Io(e, _) => io_to_archive(e),
            S::FileOpen(e, _) => ArchiveError::Io(e),
            S::PasswordRequired | S::MaybeBadPassword(_) => ArchiveError::Encrypted(String::new()),
            S::UnsupportedCompressionMethod(m) => ArchiveError::Unsupported(format!("7z {m}")),
            S::Unsupported(m) => ArchiveError::Unsupported(format!("7z {m}")),
            other => ArchiveError::Corrupt(other.to_string()),
        }
    }
}

/// Decoders report checksum failures as `InvalidData` I/O errors; surface
/// them as [`ArchiveError::Corrupt`] so callers do not have to sniff.
pub(crate) fn io_to_archive(e: std::io::Error) -> ArchiveError {
    match e.kind() {
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof => {
            ArchiveError::Corrupt(e.to_string())
        }
        _ => ArchiveError::Io(e),
    }
}

/// Reject names an archive must never carry. Keeps the four writers
/// consistent and keeps extracted paths inside the destination.
pub(crate) fn check_entry_name(name: &str) -> Result<(), ArchiveError> {
    let bad = name.is_empty()
        || name.len() > 0xFFFF
        || name.contains('\0')
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains('\\')
        || name.split('/').any(|seg| seg == "..")
        || (name.len() >= 2 && name.as_bytes()[1] == b':');
    if bad {
        Err(ArchiveError::InvalidName(name.to_owned()))
    } else {
        Ok(())
    }
}
