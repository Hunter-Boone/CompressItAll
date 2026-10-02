//! Archive inputs (DESIGN.md 3.9.3) and the packaging writer (3.9.2).
use super::*;
use cia_archive::{ArchiveKind, ArchiveOptions, ArchiveWriter};

pub fn kind_for(p: Packaging) -> ArchiveKind {
    match p {
        Packaging::SevenZip => ArchiveKind::SevenZip,
        Packaging::TarZst => ArchiveKind::TarZst,
        Packaging::TarXz => ArchiveKind::TarXz,
        _ => ArchiveKind::Zip,
    }
}

/// Package already-encoded outputs into one archive.
pub fn package(kind: ArchiveKind, entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut w = ArchiveWriter::in_memory(kind, ArchiveOptions::default()).map_err(|e| e.to_string())?;
    for (name, bytes) in entries {
        w.add_entry(name, bytes).map_err(|e| e.to_string())?;
    }
    w.finish().map_err(|e| e.to_string())
}

/// Recompress an archive's contents as a nested per-message sub-job is done by the engine
/// (it needs the other planners); this helper handles the simple case: repack entries we are
/// given (already optimised) in the same format.
pub fn repack(format: &str, entries: &[(String, Vec<u8>)]) -> Result<(Vec<u8>, &'static str), String> {
    let (kind, ext) = match format {
        "7z" => (ArchiveKind::SevenZip, "7z"),
        "tar.zst" | "zst" => (ArchiveKind::TarZst, "tar.zst"),
        "tar.xz" | "xz" => (ArchiveKind::TarXz, "tar.xz"),
        _ => (ArchiveKind::Zip, "zip"),
    };
    package(kind, entries).map(|b| (b, ext))
}

pub fn extract_all(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut a = cia_archive::open(bytes).map_err(|e| e.to_string())?;
    let entries: Vec<(usize, String)> = a.entries().iter().enumerate().filter(|(_, e)| !e.is_dir).map(|(i, e)| (i, e.name.clone())).collect();
    let mut out = Vec::with_capacity(entries.len());
    for (i, name) in entries {
        let e = &a.entries()[i];
        if e.encrypted || !e.supported {
            return Err(if e.encrypted { "encrypted".into() } else { format!("unsupported method {}", e.method) });
        }
        let data = a.extract(i).map_err(|e| e.to_string())?;
        out.push((name, data));
    }
    Ok(out)
}
