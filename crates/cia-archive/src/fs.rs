//! Filesystem conveniences (cargo feature `fs`, native only).

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::{Archive, ArchiveError, ArchiveKind, ArchiveOptions, ArchiveWriter};

/// Read a file and open it as an archive. The returned bytes keep the
/// archive alive: `let bytes = read(..)?; let ar = open(&bytes)?;`.
pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>, ArchiveError> {
    Ok(fs::read(path)?)
}

/// Pack `files` (archive name, source path) into `out_path`, streaming to
/// the file. Entries are read into memory one at a time.
pub fn write_files(
    kind: ArchiveKind,
    options: ArchiveOptions,
    files: &[(String, PathBuf)],
    out_path: impl AsRef<Path>,
) -> Result<u64, ArchiveError> {
    let file = fs::File::create(out_path)?;
    let mut w = ArchiveWriter::new(kind, options, std::io::BufWriter::new(file))?;
    for (name, src) in files {
        let bytes = fs::read(src)?;
        w.add_entry(name, &bytes)?;
    }
    let mut out = w.finish()?;
    out.flush()?;
    let file = out
        .into_inner()
        .map_err(|e| ArchiveError::Io(e.into_error()))?;
    Ok(file.metadata()?.len())
}

/// Map an archive entry name onto a path under `dir`, refusing anything that
/// would escape it. Returns `None` for names that are unsafe.
pub fn safe_join(dir: &Path, name: &str) -> Option<PathBuf> {
    let rel = Path::new(name);
    let mut out = dir.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out == dir {
        return None;
    }
    Some(out)
}

/// Extract every supported entry into `dir`, creating directories as
/// needed. Encrypted or unsupported entries and names that would escape
/// `dir` are skipped and returned, so the caller can report them per item
/// (DESIGN.md 3.9.3). Returns `(written_paths, skipped_entry_indices)`.
pub fn extract_to_dir(
    archive: &mut Archive<'_>,
    dir: impl AsRef<Path>,
) -> Result<(Vec<PathBuf>, Vec<usize>), ArchiveError> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    let mut skipped = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.entries()[i].clone();
        if !entry.supported {
            skipped.push(i);
            continue;
        }
        let Some(target) = safe_join(dir, &entry.name) else {
            skipped.push(i);
            continue;
        };
        if entry.is_dir {
            fs::create_dir_all(&target)?;
            written.push(target);
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = archive.extract(i)?;
        fs::write(&target, data)?;
        written.push(target);
    }
    Ok((written, skipped))
}
